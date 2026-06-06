# Export de Transcrições — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Exportar transcrições como Markdown, texto puro e SRT (timestamps reais) via save dialog nativo, a partir da TranscriptionView e da History (issue #2).

**Architecture:** Nova tabela `segments` captura timestamps do whisper no fluxo de finalize (`transcribe_with_callbacks` → `job.rs` → `finish_job`). Módulo `export/` em Rust com formatters puros; um comando Tauri `export_transcription(id, format, path)` gera e grava o arquivo. UI: componente `ExportMenu.svelte` (dropdown) reutilizado na TranscriptionView e na History.

**Tech Stack:** Rust (rusqlite, whisper-rs 0.14, tauri 2, tauri-plugin-dialog), Svelte 5 runes, Vitest (novo devDep para testes JS).

**Spec:** `docs/superpowers/specs/2026-06-05-export-transcriptions-design.md`

**Decisões que refinam o spec (descobertas na pesquisa do código):**

1. Só `transcribe_with_callbacks` precisa extrair timestamps — é o único método usado pelos fluxos que persistem transcrições (`run_finalize_dictation` / `run_finalize_pending_file`). `transcribe` (dead code) e `transcribe_samples` (loop ao vivo do ditado, não persiste) ficam intactos.
2. Segmentos são salvos **apenas quando `job.committed_text` está vazio** (whisper processou o áudio inteiro). No ditado, o finalize roda só na *cauda* após o prefixo do loop ao vivo — timestamps não cobririam a gravação toda. Resultado: gravações e imports (caso de uso real de SRT) sempre ganham segmentos; ditados que usaram o loop ao vivo não (SRT desabilitado, como transcrições antigas).
3. `whisper_rs::SegmentCallbackData` fornece `start_timestamp`/`end_timestamp` em **centissegundos** (i64) — converter para ms multiplicando por 10.

**Convenções deste repo:** commits SEM `Co-Authored-By`. Mensagens em Conventional Commits. `cargo fmt` antes de cada commit Rust.

---

### Task 1: Store — tabela `segments` + `save_segments`/`get_segments`

**Files:**
- Modify: `src-tauri/src/db/store.rs` (schema em `Store::new` ~linha 40-95; novos métodos após `delete` ~linha 270; testes no `mod tests`)

- [ ] **Step 1: Escrever testes que falham**

Adicionar ao final do `mod tests` em `src-tauri/src/db/store.rs`:

```rust
    #[test]
    fn save_segments_and_get_segments_roundtrip() {
        let (store, _temp_file) = create_temp_store();
        let id = store.save("Meeting", "full text", "pt", 10.0).expect("save");

        let segments = vec![
            Segment { start_ms: 0, end_ms: 2500, text: "Olá pessoal".to_string() },
            Segment { start_ms: 2500, end_ms: 5100, text: "vamos começar".to_string() },
        ];
        store.save_segments(id, &segments).expect("save_segments");

        let loaded = store.get_segments(id).expect("get_segments");
        assert_eq!(loaded, segments);
    }

    #[test]
    fn get_segments_returns_empty_for_transcription_without_segments() {
        let (store, _temp_file) = create_temp_store();
        let id = store.save("No segs", "text", "pt", 5.0).expect("save");

        let loaded = store.get_segments(id).expect("get_segments");
        assert!(loaded.is_empty());
    }

    #[test]
    fn deleting_transcription_cascades_to_segments() {
        let (store, _temp_file) = create_temp_store();
        let id = store.save("Meeting", "text", "pt", 10.0).expect("save");
        store
            .save_segments(id, &[Segment { start_ms: 0, end_ms: 1000, text: "hi".to_string() }])
            .expect("save_segments");

        store.delete(id).expect("delete");

        let loaded = store.get_segments(id).expect("get_segments");
        assert!(loaded.is_empty());
    }

    #[test]
    fn save_segments_rejects_nonexistent_transcription() {
        let (store, _temp_file) = create_temp_store();
        let result = store.save_segments(
            999,
            &[Segment { start_ms: 0, end_ms: 1000, text: "hi".to_string() }],
        );
        assert!(result.is_err());
    }

    #[test]
    fn segments_table_is_added_to_existing_database() {
        // Simula um DB criado antes da feature: abre, fecha, reabre.
        let temp_file = NamedTempFile::new().expect("temp file");
        {
            let _store = Store::new(temp_file.path()).expect("first open");
        }
        let store = Store::new(temp_file.path()).expect("reopen");
        let id = store.save("After migration", "text", "pt", 1.0).expect("save");
        store
            .save_segments(id, &[Segment { start_ms: 0, end_ms: 500, text: "ok".to_string() }])
            .expect("save_segments after reopen");
    }
```

- [ ] **Step 2: Rodar e confirmar que falham (não compilam — `Segment` não existe)**

Run: `cargo test --manifest-path src-tauri/Cargo.toml save_segments`
Expected: erro de compilação `cannot find struct Segment`

- [ ] **Step 3: Implementação mínima**

Em `src-tauri/src/db/store.rs`, após o struct `PendingRecording` (~linha 24):

```rust
#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct Segment {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}
```

Em `Store::new`, logo após o `pragma_update` de `synchronous` (~linha 38), habilitar foreign keys (rusqlite não liga por padrão; necessário para o `ON DELETE CASCADE`):

```rust
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(|e| format!("Failed to enable foreign keys: {}", e))?;
```

Ainda em `Store::new`, após o bloco `glossary_terms` (~linha 69):

```rust
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS segments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                transcription_id INTEGER NOT NULL REFERENCES transcriptions(id) ON DELETE CASCADE,
                start_ms INTEGER NOT NULL,
                end_ms INTEGER NOT NULL,
                text TEXT NOT NULL
            );",
        )
        .map_err(|e| format!("Failed to create segments table: {}", e))?;
```

Novos métodos no `impl Store`, após `delete` (~linha 270):

```rust
    pub fn save_segments(&self, transcription_id: i64, segments: &[Segment]) -> Result<(), String> {
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| format!("Failed to start transaction: {}", e))?;
        for seg in segments {
            tx.execute(
                "INSERT INTO segments (transcription_id, start_ms, end_ms, text) VALUES (?1, ?2, ?3, ?4)",
                params![transcription_id, seg.start_ms, seg.end_ms, seg.text],
            )
            .map_err(|e| format!("Failed to save segment: {}", e))?;
        }
        tx.commit()
            .map_err(|e| format!("Failed to commit segments: {}", e))
    }

    pub fn get_segments(&self, transcription_id: i64) -> Result<Vec<Segment>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT start_ms, end_ms, text FROM segments WHERE transcription_id = ?1 ORDER BY id")
            .map_err(|e| format!("Failed to prepare query: {}", e))?;

        let rows = stmt
            .query_map(params![transcription_id], |row| {
                Ok(Segment {
                    start_ms: row.get(0)?,
                    end_ms: row.get(1)?,
                    text: row.get(2)?,
                })
            })
            .map_err(|e| format!("Failed to query segments: {}", e))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to read segment row: {}", e))
    }
```

- [ ] **Step 4: Rodar os testes e confirmar que passam**

Run: `cargo test --manifest-path src-tauri/Cargo.toml store`
Expected: todos PASS (incluindo os 5 novos)

- [ ] **Step 5: Commit**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml
git add src-tauri/src/db/store.rs
git commit -m "feat(db): add segments table with timestamps per transcription"
```

---

### Task 2: Store — `has_segments` no record de `Transcription`

**Files:**
- Modify: `src-tauri/src/db/store.rs` (struct `Transcription` ~linha 5-16; queries em `list` ~linha 197 e `get` ~linha 223; testes)

- [ ] **Step 1: Escrever testes que falham**

Adicionar ao `mod tests`:

```rust
    #[test]
    fn get_reports_has_segments_flag() {
        let (store, _temp_file) = create_temp_store();
        let with_id = store.save("With", "text", "pt", 1.0).expect("save");
        let without_id = store.save("Without", "text", "pt", 1.0).expect("save");
        store
            .save_segments(with_id, &[Segment { start_ms: 0, end_ms: 900, text: "hi".to_string() }])
            .expect("save_segments");

        assert!(store.get(with_id).expect("get").has_segments);
        assert!(!store.get(without_id).expect("get").has_segments);
    }

    #[test]
    fn list_reports_has_segments_flag() {
        let (store, _temp_file) = create_temp_store();
        let with_id = store.save("With", "text", "pt", 1.0).expect("save");
        store.save("Without", "text", "pt", 1.0).expect("save");
        store
            .save_segments(with_id, &[Segment { start_ms: 0, end_ms: 900, text: "hi".to_string() }])
            .expect("save_segments");

        let records = store.list().expect("list");
        let with = records.iter().find(|r| r.title == "With").unwrap();
        let without = records.iter().find(|r| r.title == "Without").unwrap();
        assert!(with.has_segments);
        assert!(!without.has_segments);
    }
```

- [ ] **Step 2: Rodar e confirmar falha de compilação (`has_segments` não existe)**

Run: `cargo test --manifest-path src-tauri/Cargo.toml has_segments`
Expected: erro `no field has_segments`

- [ ] **Step 3: Implementação**

No struct `Transcription`, adicionar o campo ao final:

```rust
    pub has_segments: bool,
```

Em `list` (~linha 200), trocar o SELECT por:

```rust
            .prepare("SELECT id, title, text, language, duration_secs, created_at, summary, status, audio_path, EXISTS(SELECT 1 FROM segments WHERE segments.transcription_id = transcriptions.id) FROM transcriptions ORDER BY created_at DESC")
```

e no `query_map`, adicionar após `audio_path`:

```rust
                    has_segments: row.get(9)?,
```

Em `get` (~linha 226), mesma mudança no SELECT:

```rust
                "SELECT id, title, text, language, duration_secs, created_at, summary, status, audio_path, EXISTS(SELECT 1 FROM segments WHERE segments.transcription_id = transcriptions.id) FROM transcriptions WHERE id = ?1",
```

e no closure:

```rust
                        has_segments: row.get(9)?,
```

- [ ] **Step 4: Rodar testes e confirmar que passam (e que nada quebrou)**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: PASS total. Se outros pontos construírem `Transcription` literal, o compilador aponta — corrigir adicionando `has_segments: false`.

- [ ] **Step 5: Commit**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml
git add src-tauri/src/db/store.rs
git commit -m "feat(db): expose has_segments flag on transcription records"
```

---

### Task 3: Whisper — callback de segmento com timestamps (ms)

**Files:**
- Modify: `src-tauri/src/transcribe/whisper.rs` (`transcribe_with_callbacks` ~linha 129-182)
- Modify: `src-tauri/src/transcribe/job.rs` (closure `on_segment` ~linha 163 — só a assinatura, por ora ignorando os novos args)

- [ ] **Step 1: Escrever teste que falha (conversão centissegundos → ms)**

`whisper_rs::SegmentCallbackData` entrega `start_timestamp`/`end_timestamp` em centissegundos. Adicionar em `src-tauri/src/transcribe/whisper.rs` (criar `mod tests` no fim do arquivo se não existir):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centis_to_ms_converts_whisper_timestamps() {
        assert_eq!(centis_to_ms(0), 0);
        assert_eq!(centis_to_ms(250), 2500); // 2.5s
        assert_eq!(centis_to_ms(360_000), 3_600_000); // 1h
    }
}
```

- [ ] **Step 2: Rodar e confirmar falha de compilação**

Run: `cargo test --manifest-path src-tauri/Cargo.toml centis_to_ms`
Expected: erro `cannot find function centis_to_ms`

- [ ] **Step 3: Implementação**

Em `whisper.rs`, antes do `impl Transcriber`:

```rust
/// whisper.cpp reports segment timestamps in centiseconds.
pub fn centis_to_ms(centis: i64) -> i64 {
    centis * 10
}
```

Mudar a assinatura de `transcribe_with_callbacks` — o bound de `S` (~linha 140):

```rust
        S: FnMut(&str, i64, i64) + Send + 'static,
```

E o registro do callback (~linha 158):

```rust
        params.set_segment_callback_safe_lossy(move |data: whisper_rs::SegmentCallbackData| {
            on_segment(
                &data.text,
                centis_to_ms(data.start_timestamp),
                centis_to_ms(data.end_timestamp),
            );
        });
```

Atualizar o doc comment de `on_segment` (~linha 124):

```rust
    /// - `on_segment` is called with each new segment text plus its start/end
    ///   timestamps in milliseconds as it is produced.
```

Em `job.rs`, ajustar a closure para compilar (timestamps usados na Task 4):

```rust
    let on_segment = move |seg: &str, _start_ms: i64, _end_ms: i64| {
```

- [ ] **Step 4: Rodar testes e confirmar que passam**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: PASS total

- [ ] **Step 5: Commit**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml
git add src-tauri/src/transcribe/whisper.rs src-tauri/src/transcribe/job.rs
git commit -m "feat(transcribe): surface segment timestamps in whisper callback"
```

---

### Task 4: Job — coletar segmentos e persistir no finalize

**Files:**
- Modify: `src-tauri/src/transcribe/job.rs` (`run_finalize_dictation` ~linha 100-278; `FinalizeOutcome` ~linha 58-65; `finish_job` ~linha 325-380)

Não há teste automatizado prático aqui (exigiria modelo whisper carregado); a cobertura vem dos testes de store (Task 1/2) e da verificação manual (Task 11). Manter as mudanças mínimas e mecânicas.

- [ ] **Step 1: Adicionar `segments` ao `FinalizeOutcome::Complete`**

```rust
pub enum FinalizeOutcome {
    Complete {
        final_text: String,
        duration_secs: f64,
        segments: Vec<crate::db::store::Segment>,
    },
    Cancelled,
    Error(String),
}
```

- [ ] **Step 2: Coletar segmentos em `run_finalize_dictation`**

Só coleta quando não há prefixo do loop ao vivo (whisper cobriu o áudio inteiro — ver decisão 2 no topo). Após `let accumulated = ...` (~linha 145):

```rust
    // Segments only represent the full recording when whisper processed all
    // of it. With a live-loop prefix, finalize ran on the tail only — the
    // timestamps would not cover the whole audio, so we skip collection.
    let collect_segments = committed_prefix.is_empty();
    let segments_acc: Arc<Mutex<Vec<crate::db::store::Segment>>> =
        Arc::new(Mutex::new(Vec::new()));
    let segments_for_callback = segments_acc.clone();
```

Na closure `on_segment` (~linha 163), usar os novos parâmetros e, logo após o `trimmed.is_empty()` check, coletar:

```rust
    let on_segment = move |seg: &str, start_ms: i64, end_ms: i64| {
        let trimmed = seg.trim();
        if trimmed.is_empty() {
            return;
        }
        if collect_segments {
            if let Ok(mut segs) = segments_for_callback.lock() {
                segs.push(crate::db::store::Segment {
                    start_ms,
                    end_ms,
                    text: trimmed.to_string(),
                });
            }
        }
        // ... resto da closure inalterado
```

- [ ] **Step 3: Preencher `segments` nos três pontos de construção de `Complete` em `run_finalize_dictation`**

Caminho prefix-only (~linha 130) — sem whisper, sem segmentos:

```rust
            FinalizeOutcome::Complete {
                final_text: committed_prefix,
                duration_secs,
                segments: Vec::new(),
            }
```

Braço `Ok` (~linha 241) e braço `Err` com fallback (~linha 271) — drenar o acumulador (no fallback os segmentos cobrem a parte transcrita antes da falha, o que é válido para SRT):

```rust
            let segments = segments_acc
                .lock()
                .map(|s| s.clone())
                .unwrap_or_default();
            FinalizeOutcome::Complete {
                final_text,
                duration_secs,
                segments,
            }
```

- [ ] **Step 4: Persistir em `finish_job`**

No braço `Complete` (~linha 333), destructure também `segments` e, logo após `mark_complete`:

```rust
        FinalizeOutcome::Complete {
            final_text,
            duration_secs,
            segments,
        } => {
            if let Ok(s) = store.lock() {
                let _ = s.update_text(id, &final_text, duration_secs);
                let _ = s.mark_complete(id);
                if !segments.is_empty() {
                    let _ = s.save_segments(id, &segments);
                }
                // ... resto inalterado
```

- [ ] **Step 5: Compilar, rodar testes, commit**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: PASS total (os testes existentes de `job.rs` não tocam finalize)

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml
git add src-tauri/src/transcribe/job.rs
git commit -m "feat(transcribe): persist whisper segments on finalize"
```

---

### Task 5: Export — formatter SRT

**Files:**
- Create: `src-tauri/src/export/mod.rs`
- Create: `src-tauri/src/export/srt.rs`
- Modify: `src-tauri/src/lib.rs` (adicionar `mod export;` na lista de mods, ~linha 1-8)

- [ ] **Step 1: Criar o módulo e escrever testes que falham**

`src-tauri/src/export/mod.rs`:

```rust
pub mod srt;
```

Em `src-tauri/src/lib.rs`, adicionar na lista de mods (ordem alfabética):

```rust
mod export;
```

`src-tauri/src/export/srt.rs` (testes primeiro):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::store::Segment;

    #[test]
    fn format_srt_timestamp_handles_zero() {
        assert_eq!(format_srt_timestamp(0), "00:00:00,000");
    }

    #[test]
    fn format_srt_timestamp_handles_sub_second() {
        assert_eq!(format_srt_timestamp(437), "00:00:00,437");
    }

    #[test]
    fn format_srt_timestamp_handles_minutes_and_seconds() {
        assert_eq!(format_srt_timestamp(83_250), "00:01:23,250");
    }

    #[test]
    fn format_srt_timestamp_handles_hours() {
        assert_eq!(format_srt_timestamp(3_600_000 + 61_001), "01:01:01,001");
    }

    #[test]
    fn to_srt_renders_single_segment() {
        let segments = vec![Segment {
            start_ms: 0,
            end_ms: 2500,
            text: "Olá pessoal".to_string(),
        }];
        assert_eq!(to_srt(&segments), "1\n00:00:00,000 --> 00:00:02,500\nOlá pessoal\n");
    }

    #[test]
    fn to_srt_renders_multiple_segments_with_blank_line_between() {
        let segments = vec![
            Segment { start_ms: 0, end_ms: 2500, text: "Olá pessoal".to_string() },
            Segment { start_ms: 2500, end_ms: 5100, text: "vamos começar".to_string() },
        ];
        assert_eq!(
            to_srt(&segments),
            "1\n00:00:00,000 --> 00:00:02,500\nOlá pessoal\n\n2\n00:00:02,500 --> 00:00:05,100\nvamos começar\n"
        );
    }

    #[test]
    fn to_srt_of_empty_slice_is_empty() {
        assert_eq!(to_srt(&[]), "");
    }
}
```

- [ ] **Step 2: Rodar e confirmar falha de compilação**

Run: `cargo test --manifest-path src-tauri/Cargo.toml srt`
Expected: erro `cannot find function to_srt`

- [ ] **Step 3: Implementação no topo de `srt.rs`**

```rust
use crate::db::store::Segment;

/// Renders segments as SubRip (.srt): 1-based index, `HH:MM:SS,mmm` range,
/// text, blank line between cues.
pub fn to_srt(segments: &[Segment]) -> String {
    segments
        .iter()
        .enumerate()
        .map(|(i, seg)| {
            format!(
                "{}\n{} --> {}\n{}\n",
                i + 1,
                format_srt_timestamp(seg.start_ms),
                format_srt_timestamp(seg.end_ms),
                seg.text
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_srt_timestamp(ms: i64) -> String {
    let hours = ms / 3_600_000;
    let minutes = (ms % 3_600_000) / 60_000;
    let seconds = (ms % 60_000) / 1_000;
    let millis = ms % 1_000;
    format!("{:02}:{:02}:{:02},{:03}", hours, minutes, seconds, millis)
}
```

- [ ] **Step 4: Rodar testes e confirmar que passam**

Run: `cargo test --manifest-path src-tauri/Cargo.toml srt`
Expected: 7 PASS

- [ ] **Step 5: Commit**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml
git add src-tauri/src/export/ src-tauri/src/lib.rs
git commit -m "feat(export): SRT formatter from stored segments"
```

---

### Task 6: Export — formatters Markdown e texto puro

**Files:**
- Create: `src-tauri/src/export/markdown.rs`
- Create: `src-tauri/src/export/plain_text.rs`
- Modify: `src-tauri/src/export/mod.rs`

- [ ] **Step 1: Declarar módulos e escrever testes que falham**

`src-tauri/src/export/mod.rs`:

```rust
pub mod markdown;
pub mod plain_text;
pub mod srt;
```

Helper de teste + testes em `src-tauri/src/export/markdown.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::store::Transcription;

    fn fixture(summary: Option<&str>) -> Transcription {
        Transcription {
            id: 1,
            title: "Reunião de planejamento".to_string(),
            text: "Linha um.\nLinha dois.".to_string(),
            language: "pt".to_string(),
            duration_secs: 754.0,
            created_at: "2026-06-05 14:30:00".to_string(),
            summary: summary.map(|s| s.to_string()),
            status: "complete".to_string(),
            audio_path: None,
            has_segments: false,
        }
    }

    #[test]
    fn to_markdown_includes_title_metadata_and_text() {
        let md = to_markdown(&fixture(None));
        assert_eq!(
            md,
            "# Reunião de planejamento\n\n**Data:** 2026-06-05 14:30:00 · **Idioma:** pt · **Duração:** 12min 34s\n\n## Transcrição\n\nLinha um.\nLinha dois.\n"
        );
    }

    #[test]
    fn to_markdown_includes_summary_section_when_present() {
        let md = to_markdown(&fixture(Some("Pontos principais.")));
        assert!(md.contains("## Resumo\n\nPontos principais.\n"));
        // Resumo vem antes da transcrição
        let resumo_pos = md.find("## Resumo").unwrap();
        let trans_pos = md.find("## Transcrição").unwrap();
        assert!(resumo_pos < trans_pos);
    }

    #[test]
    fn to_markdown_omits_summary_section_when_blank() {
        let md = to_markdown(&fixture(Some("   ")));
        assert!(!md.contains("## Resumo"));
    }

    #[test]
    fn format_duration_renders_minutes_and_seconds() {
        assert_eq!(format_duration(0.0), "0min 0s");
        assert_eq!(format_duration(59.6), "0min 60s");
        assert_eq!(format_duration(754.0), "12min 34s");
    }
}
```

Testes em `src-tauri/src/export/plain_text.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::store::Transcription;

    #[test]
    fn to_plain_text_is_just_the_transcript_text() {
        let t = Transcription {
            id: 1,
            title: "T".to_string(),
            text: "Apenas o texto.".to_string(),
            language: "pt".to_string(),
            duration_secs: 1.0,
            created_at: "2026-06-05 14:30:00".to_string(),
            summary: Some("ignorado".to_string()),
            status: "complete".to_string(),
            audio_path: None,
            has_segments: false,
        };
        assert_eq!(to_plain_text(&t), "Apenas o texto.");
    }
}
```

- [ ] **Step 2: Rodar e confirmar falha de compilação**

Run: `cargo test --manifest-path src-tauri/Cargo.toml export`
Expected: erros `cannot find function to_markdown` / `to_plain_text`

- [ ] **Step 3: Implementação**

Topo de `markdown.rs`:

```rust
use crate::db::store::Transcription;

pub fn to_markdown(t: &Transcription) -> String {
    let mut md = format!(
        "# {}\n\n**Data:** {} · **Idioma:** {} · **Duração:** {}\n\n",
        t.title,
        t.created_at,
        t.language,
        format_duration(t.duration_secs)
    );

    if let Some(summary) = t.summary.as_deref().filter(|s| !s.trim().is_empty()) {
        md.push_str(&format!("## Resumo\n\n{}\n\n", summary.trim()));
    }

    md.push_str(&format!("## Transcrição\n\n{}\n", t.text));
    md
}

/// Mirrors the frontend's formatDuration (format.js): "12min 34s".
fn format_duration(secs: f64) -> String {
    let m = (secs / 60.0).floor() as i64;
    let s = (secs % 60.0).round() as i64;
    format!("{}min {}s", m, s)
}
```

Topo de `plain_text.rs`:

```rust
use crate::db::store::Transcription;

pub fn to_plain_text(t: &Transcription) -> String {
    t.text.clone()
}
```

- [ ] **Step 4: Rodar testes e confirmar que passam**

Run: `cargo test --manifest-path src-tauri/Cargo.toml export`
Expected: PASS (markdown + plain_text + srt)

- [ ] **Step 5: Commit**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml
git add src-tauri/src/export/
git commit -m "feat(export): markdown and plain text formatters"
```

---

### Task 7: Comando `export_transcription` + capability

**Files:**
- Modify: `src-tauri/src/export/mod.rs` (enum `ExportFormat` + função `render`)
- Modify: `src-tauri/src/lib.rs` (comando + registro no `invoke_handler` ~linha 766)
- Modify: `src-tauri/capabilities/default.json`

- [ ] **Step 1: Escrever testes que falham (render é a parte testável; o comando só faz parse → delega → escreve)**

Adicionar em `src-tauri/src/export/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::store::{Segment, Transcription};

    fn fixture() -> Transcription {
        Transcription {
            id: 1,
            title: "T".to_string(),
            text: "texto".to_string(),
            language: "pt".to_string(),
            duration_secs: 1.0,
            created_at: "2026-06-05 14:30:00".to_string(),
            summary: None,
            status: "complete".to_string(),
            audio_path: None,
            has_segments: true,
        }
    }

    #[test]
    fn render_markdown_and_plain_text_ignore_segments() {
        let t = fixture();
        assert!(render(&t, &[], ExportFormat::Markdown).unwrap().starts_with("# T"));
        assert_eq!(render(&t, &[], ExportFormat::PlainText).unwrap(), "texto");
    }

    #[test]
    fn render_srt_uses_segments() {
        let t = fixture();
        let segs = vec![Segment { start_ms: 0, end_ms: 1000, text: "oi".to_string() }];
        let srt = render(&t, &segs, ExportFormat::Srt).unwrap();
        assert!(srt.starts_with("1\n00:00:00,000 --> 00:00:01,000\noi"));
    }

    #[test]
    fn render_srt_without_segments_is_an_error() {
        let t = fixture();
        assert!(render(&t, &[], ExportFormat::Srt).is_err());
    }

    #[test]
    fn export_format_deserializes_from_snake_case() {
        let f: ExportFormat = serde_json::from_str("\"plain_text\"").unwrap();
        assert!(matches!(f, ExportFormat::PlainText));
    }
}
```

Nota: `serde_json` já é dependência transitiva do tauri; se `cargo test` reclamar, adicionar `serde_json = "1"` em `[dev-dependencies]` do `src-tauri/Cargo.toml`.

- [ ] **Step 2: Rodar e confirmar falha de compilação**

Run: `cargo test --manifest-path src-tauri/Cargo.toml export::tests`
Expected: erro `cannot find type ExportFormat`

- [ ] **Step 3: Implementação em `export/mod.rs`**

```rust
use crate::db::store::{Segment, Transcription};

pub mod markdown;
pub mod plain_text;
pub mod srt;

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Markdown,
    PlainText,
    Srt,
}

pub fn render(
    t: &Transcription,
    segments: &[Segment],
    format: ExportFormat,
) -> Result<String, String> {
    match format {
        ExportFormat::Markdown => Ok(markdown::to_markdown(t)),
        ExportFormat::PlainText => Ok(plain_text::to_plain_text(t)),
        ExportFormat::Srt => {
            if segments.is_empty() {
                return Err("No segment timestamps available for SRT export".to_string());
            }
            Ok(srt::to_srt(segments))
        }
    }
}
```

- [ ] **Step 4: Rodar testes e confirmar que passam**

Run: `cargo test --manifest-path src-tauri/Cargo.toml export`
Expected: PASS

- [ ] **Step 5: Comando Tauri em `lib.rs`**

Após `delete_transcription` (~linha 324):

```rust
#[tauri::command]
fn export_transcription(
    state: State<'_, AppState>,
    id: i64,
    format: export::ExportFormat,
    path: String,
) -> Result<(), String> {
    let (transcription, segments) = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        (store.get(id)?, store.get_segments(id)?)
    };
    let content = export::render(&transcription, &segments, format)?;
    std::fs::write(&path, content).map_err(|e| format!("Failed to write file: {}", e))
}
```

Registrar no `invoke_handler` (lista ~linha 766), após `delete_transcription`:

```rust
            export_transcription,
```

- [ ] **Step 6: Capability de save dialog**

`src-tauri/capabilities/default.json` — adicionar à lista `permissions`:

```json
    "dialog:allow-save"
```

- [ ] **Step 7: Compilar, testar, commit**

Run: `cargo test --manifest-path src-tauri/Cargo.toml && cargo clippy --manifest-path src-tauri/Cargo.toml`
Expected: PASS, sem warnings novos

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml
git add src-tauri/src/export/mod.rs src-tauri/src/lib.rs src-tauri/capabilities/default.json src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "feat(export): export_transcription command with save capability"
```

---

### Task 8: Frontend — Vitest + helper de nome de arquivo

**Files:**
- Modify: `package.json` (devDep `vitest`, script `test`)
- Create: `src/lib/export.js`
- Create: `src/lib/export.test.js`

- [ ] **Step 1: Instalar Vitest (primeiro teste JS do projeto — CLAUDE.md já prescreve Vitest)**

```bash
npm install -D vitest
```

Em `package.json`, adicionar ao `scripts`:

```json
    "test": "vitest run",
```

- [ ] **Step 2: Escrever teste que falha — `src/lib/export.test.js`**

```js
import { describe, expect, it } from "vitest";
import { defaultExportFilename } from "./export.js";

describe("defaultExportFilename", () => {
  it("slugifies the title and appends date and extension", () => {
    expect(defaultExportFilename("Reunião de Planejamento", "2026-06-05 14:30:00", "md"))
      .toBe("reuniao-de-planejamento-2026-06-05.md");
  });

  it("strips accents and special characters", () => {
    expect(defaultExportFilename("Ação: João & Cia!", "2026-01-02 08:00:00", "srt"))
      .toBe("acao-joao-cia-2026-01-02.srt");
  });

  it("falls back to 'transcricao' when the title has no usable characters", () => {
    expect(defaultExportFilename("???", "2026-06-05 14:30:00", "txt"))
      .toBe("transcricao-2026-06-05.txt");
  });

  it("omits the date part when created_at is missing", () => {
    expect(defaultExportFilename("Notas", "", "txt")).toBe("notas.txt");
  });
});
```

- [ ] **Step 3: Rodar e confirmar que falha**

Run: `npm test`
Expected: FAIL — `Cannot find module './export.js'`

- [ ] **Step 4: Implementação — `src/lib/export.js`**

```js
/**
 * Default filename for an exported transcription: slugified title + date.
 * @param {string} title
 * @param {string} createdAt — "YYYY-MM-DD HH:MM:SS" (from SQLite)
 * @param {string} ext — "md" | "txt" | "srt"
 */
export function defaultExportFilename(title, createdAt, ext) {
  const slug =
    title
      .normalize("NFD")
      .replace(/[\u0300-\u036f]/g, "")
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "") || "transcricao";
  const date = (createdAt || "").slice(0, 10);
  return date ? `${slug}-${date}.${ext}` : `${slug}.${ext}`;
}
```

- [ ] **Step 5: Rodar testes e confirmar que passam**

Run: `npm test`
Expected: 4 PASS

- [ ] **Step 6: Commit**

```bash
git add package.json package-lock.json src/lib/export.js src/lib/export.test.js
git commit -m "feat(ui): export filename helper with vitest setup"
```

---

### Task 9: Frontend — chaves i18n

**Files:**
- Modify: `src/lib/i18n.js` (objetos `pt` e `en`)

- [ ] **Step 1: Adicionar as chaves**

No final do objeto `pt` (após `terms: "termos"`, antes do `}`):

```js
    export: "Exportar",
    exportMarkdown: "Markdown (.md)",
    exportText: "Texto (.txt)",
    exportSrt: "Legenda (.srt)",
    srtUnavailable: "Sem timestamps de segmentos nesta transcrição",
    exportSuccess: "Exportado!",
    exportError: "Falha ao exportar",
```

No final do objeto `en` (após `terms: "terms"`):

```js
    export: "Export",
    exportMarkdown: "Markdown (.md)",
    exportText: "Text (.txt)",
    exportSrt: "Subtitles (.srt)",
    srtUnavailable: "No segment timestamps on this transcription",
    exportSuccess: "Exported!",
    exportError: "Export failed",
```

- [ ] **Step 2: Verificar e commitar**

Run: `npm run check`
Expected: sem erros novos

```bash
git add src/lib/i18n.js
git commit -m "feat(ui): i18n strings for transcription export"
```

---

### Task 10: Frontend — `ExportMenu.svelte` + integração

**Files:**
- Create: `src/lib/ExportMenu.svelte`
- Modify: `src/lib/TranscriptionView.svelte` (header ~linha 65-73)
- Modify: `src/lib/History.svelte` (item da lista ~linha 88-101)

- [ ] **Step 1: Criar `src/lib/ExportMenu.svelte`**

```svelte
<script>
    import { invoke } from "@tauri-apps/api/core";
    import { save } from "@tauri-apps/plugin-dialog";
    import { t } from "./i18n.js";
    import { defaultExportFilename } from "./export.js";

    let { transcription, compact = false } = $props();

    let open = $state(false);
    /** @type {"" | "success" | "error"} */
    let status = $state("");

    const formats = [
        { key: "markdown", ext: "md", labelKey: "exportMarkdown", filterName: "Markdown" },
        { key: "plain_text", ext: "txt", labelKey: "exportText", filterName: "Text" },
        { key: "srt", ext: "srt", labelKey: "exportSrt", filterName: "SubRip" },
    ];

    /** @param {{key: string, ext: string, filterName: string}} format */
    async function exportAs(format) {
        open = false;
        try {
            const path = await save({
                defaultPath: defaultExportFilename(
                    transcription.title,
                    transcription.created_at,
                    format.ext,
                ),
                filters: [{ name: format.filterName, extensions: [format.ext] }],
            });
            if (!path) return; // user cancelled
            await invoke("export_transcription", {
                id: transcription.id,
                format: format.key,
                path,
            });
            flash("success");
        } catch (e) {
            console.error("Export failed:", e);
            flash("error");
        }
    }

    /** @param {"success" | "error"} result */
    function flash(result) {
        status = result;
        setTimeout(() => { status = ""; }, 2000);
    }

    function handleWindowClick() {
        open = false;
    }

    /** @param {KeyboardEvent} e */
    function handleKeydown(e) {
        if (e.key === "Escape") open = false;
    }
</script>

<svelte:window
    onclick={open ? handleWindowClick : undefined}
    onkeydown={open ? handleKeydown : undefined}
/>

<div class="export-menu">
    <button
        class="trigger"
        class:compact
        class:success={status === "success"}
        class:failed={status === "error"}
        title={t("export")}
        onclick={(e) => { e.stopPropagation(); open = !open; }}
    >
        {#if compact}
            {status === "success" ? "✓" : status === "error" ? "!" : "⤓"}
        {:else}
            {status === "success" ? t("exportSuccess") : status === "error" ? t("exportError") : `${t("export")} ▾`}
        {/if}
    </button>

    {#if open}
        <div class="dropdown" role="menu">
            {#each formats as format}
                {@const srtBlocked = format.key === "srt" && !transcription.has_segments}
                <button
                    class="option"
                    role="menuitem"
                    disabled={srtBlocked}
                    title={srtBlocked ? t("srtUnavailable") : undefined}
                    onclick={(e) => { e.stopPropagation(); exportAs(format); }}
                >
                    {t(format.labelKey)}
                </button>
            {/each}
        </div>
    {/if}
</div>

<style>
    .export-menu {
        position: relative;
        display: inline-block;
    }

    .trigger {
        background: var(--surface);
        color: var(--text);
        border: 1px solid var(--border);
        padding: 8px 16px;
    }

    .trigger.compact {
        background: transparent;
        border: none;
        color: var(--info);
        padding: 8px 12px;
        font-size: 1rem;
        white-space: nowrap;
    }

    .trigger.success {
        border-color: var(--success);
        color: var(--success);
    }

    .trigger.failed {
        border-color: var(--accent);
        color: var(--accent);
    }

    .dropdown {
        position: absolute;
        right: 0;
        top: calc(100% + 4px);
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: var(--radius);
        min-width: 160px;
        z-index: 10;
        display: flex;
        flex-direction: column;
        overflow: hidden;
    }

    .option {
        background: transparent;
        color: var(--text);
        text-align: left;
        padding: 10px 14px;
        border-radius: 0;
    }

    .option:hover:not(:disabled) {
        background: var(--primary);
        color: white;
    }

    .option:disabled {
        opacity: 0.4;
        cursor: not-allowed;
    }
</style>
```

- [ ] **Step 2: Integrar na `TranscriptionView.svelte`**

No `<script>`, adicionar import:

```js
    import ExportMenu from "./ExportMenu.svelte";
```

No header (~linha 66-73), envolver as ações da direita num grupo para manter o layout `space-between`:

```svelte
    <div class="header">
        <button class="btn-back" onclick={onBack}>← {t("back")}</button>
        <div class="header-actions">
            {#if !summaryText && claudeAvailable}
                <button class="btn-action" onclick={summarize} disabled={summarizing}>
                    {summarizing ? t("summarizing") : t("summarize")}
                </button>
            {/if}
            <ExportMenu {transcription} />
        </div>
    </div>
```

E no `<style>`:

```css
    .header-actions {
        display: flex;
        gap: 8px;
        align-items: center;
    }
```

- [ ] **Step 3: Integrar na `History.svelte`**

Import no `<script>`:

```js
    import ExportMenu from "./ExportMenu.svelte";
```

No item da lista, entre o bloco do summarize (~linha 98) e o botão delete (~linha 99):

```svelte
                    <ExportMenu transcription={item} compact />
```

- [ ] **Step 4: Verificação de tipos**

Run: `npm run check`
Expected: sem erros novos

- [ ] **Step 5: Commit**

```bash
git add src/lib/ExportMenu.svelte src/lib/TranscriptionView.svelte src/lib/History.svelte
git commit -m "feat(ui): export menu in transcription view and history"
```

---

### Task 11: Verificação final

- [ ] **Step 1: Suíte completa**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
npm test
npm run check
```

Expected: tudo verde, sem warnings novos de clippy.

- [ ] **Step 2: Smoke test manual (`cargo tauri dev`)**

1. Gravar um áudio curto (ou importar um WAV) e transcrever → na History, o item novo deve ter SRT habilitado no menu ⤓.
2. Exportar Markdown → conferir título, linha de metadata e seção `## Transcrição` (e `## Resumo` se houver resumo).
3. Exportar SRT → conferir índices, timestamps `HH:MM:SS,mmm` crescentes e textos.
4. Exportar TXT → só o texto.
5. Numa transcrição antiga (sem segmentos): opção SRT desabilitada com tooltip.
6. Cancelar o save dialog → nenhum erro, nenhum arquivo.
7. Copy to clipboard do transcript continua funcionando na TranscriptionView.

- [ ] **Step 3: Atualizar acceptance criteria na issue**

```bash
gh issue comment 2 --body "Implementado na branch feat/2-export-transcriptions: export Markdown/TXT/SRT com save dialog nativo, menu na TranscriptionView e na History, SRT com timestamps reais (tabela segments). Copy to clipboard já existia e segue funcionando."
```
