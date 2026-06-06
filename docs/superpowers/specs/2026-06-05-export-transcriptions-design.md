# Export de Transcrições — Design

**Issue:** [#2 — Export transcriptions (Markdown, SRT, plain text, clipboard)](https://github.com/vagnerzampieri/martin/issues/2)
**Data:** 2026-06-05

## Objetivo

Permitir exportar uma transcrição como **Markdown**, **texto puro** e **SRT**, via save dialog nativo, a partir da TranscriptionView e da History. "Copy to clipboard" do transcript completo já existe na TranscriptionView e permanece como está.

## Decisões de escopo

- **SRT exige timestamps reais**, mas hoje `whisper.rs` descarta `t0`/`t1` e o schema não tem segmentos. Decisão: criar tabela `segments` e capturar timestamps **nas novas transcrições**. Transcrições antigas não têm segmentos → opção SRT desabilitada para elas, com tooltip explicativo. Markdown/TXT funcionam para todas.
- **Geração e escrita de arquivo no backend Rust** (formatters puros + comando Tauri), seguindo a divisão de responsabilidades do projeto (file I/O é do backend).
- **UI: botão "Exportar" único com dropdown** (Markdown / Texto / SRT), componente reutilizado na TranscriptionView e em cada item da History.
- Timestamps **estimados** para SRT foram descartados (contrariam o acceptance criteria).

## Parte 1 — Modelo de dados (segmentos)

Nova tabela:

```sql
CREATE TABLE IF NOT EXISTS segments (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    transcription_id INTEGER NOT NULL REFERENCES transcriptions(id) ON DELETE CASCADE,
    start_ms INTEGER NOT NULL,
    end_ms INTEGER NOT NULL,
    text TEXT NOT NULL
);
```

**Captura:** apenas `transcribe_with_callbacks` em `whisper.rs` precisa mudar — é o único método usado pelos fluxos que persistem transcrições. O callback `on_segment` passa a receber também `start_ms`/`end_ms` (whisper devolve centissegundos via `SegmentCallbackData` → converter para ms). `transcribe` (dead code) e `transcribe_samples` (loop ao vivo do ditado, não persiste) ficam intactos.

**Persistência:** `run_finalize_dictation` coleta os segmentos e os devolve em `FinalizeOutcome::Complete`; `finish_job` salva via `store.save_segments(transcription_id, &segments)`. Segmentos só são coletados quando `job.committed_text` está vazio (whisper processou o áudio inteiro) — no ditado com prefixo do loop ao vivo, o finalize roda só na cauda e os timestamps não cobririam a gravação toda. Gravações e imports (caso de uso real de SRT) sempre ganham segmentos. Delete limpa via `ON DELETE CASCADE` — ativar `PRAGMA foreign_keys = ON` na conexão.

**Transcrições antigas e ditados com prefixo ao vivo:** sem linhas em `segments` → SRT indisponível.

## Parte 2 — Módulo de export (Rust)

Novo módulo `src-tauri/src/export/` com formatters como funções puras:

- `markdown.rs` — `fn to_markdown(t: &Transcription) -> String`:

  ```markdown
  # {title}

  **Data:** {created_at} · **Idioma:** {language} · **Duração:** {duration}

  ## Resumo          ← só se summary existir
  {summary}

  ## Transcrição
  {text}
  ```

- `plain_text.rs` — `fn to_plain_text(t: &Transcription) -> String` — apenas `t.text`.
- `srt.rs` — `fn to_srt(segments: &[Segment]) -> String` — formato SRT padrão (`HH:MM:SS,mmm --> HH:MM:SS,mmm`), índice 1-based. Auxiliar `format_srt_timestamp(ms)` testada isoladamente.

**Comando Tauri** (parse → delega → retorna):

```rust
#[tauri::command]
fn export_transcription(id: i64, format: ExportFormat, path: String, state: ...) -> Result<(), String>
```

- `ExportFormat` é enum (`Markdown | PlainText | Srt`) — estado ilegal irrepresentável.
- Busca a transcrição (e segmentos, no caso SRT); SRT sem segmentos → `Err` com mensagem clara.
- Escreve o arquivo com `std::fs::write`.

**Disponibilidade do SRT na UI:** incluir `has_segments: bool` nos records retornados por `list_transcriptions`/`get` (evita roundtrip extra).

**Capability:** adicionar `dialog:allow-save` em `src-tauri/capabilities/default.json`.

## Parte 3 — UI (Svelte)

**Novo componente `ExportMenu.svelte`:**

- Props: `{ transcription }` (usa `id`, `title`, `created_at`, `has_segments`).
- Botão que abre dropdown: Markdown, Texto, SRT (SRT desabilitado com tooltip `t("srtUnavailable")` quando `!has_segments`).
- Fluxo ao escolher formato:
  1. `save()` do `@tauri-apps/plugin-dialog` com filtro de extensão e nome padrão `slug(title)-YYYY-MM-DD.{md|txt|srt}`.
  2. Usuário cancelou → não faz nada.
  3. `invoke("export_transcription", { id, format, path })`.
  4. Feedback inline breve de sucesso/erro no botão (mesmo padrão de `btn-copy-section`).
- Fecha ao clicar fora / `Escape`.

**Integração:**

- `TranscriptionView.svelte` — `<ExportMenu>` no header, ao lado de "Resumir".
- `History.svelte` — ícone `⤓` por item (entre summarize e delete), com `stopPropagation`.

**i18n:** novas chaves pt/en em `i18n.js`: `export`, `exportMarkdown`, `exportText`, `exportSrt`, `srtUnavailable`, `exportSuccess`, `exportError`.

## Tratamento de erros

- SRT sem segmentos: bloqueado na UI (botão desabilitado) e validado no backend (`Err` com mensagem clara).
- Falha de escrita (path inválido, sem permissão): `Err` propagado e exibido via feedback inline.
- Cancelamento do save dialog: no-op silencioso.

## Testes

- **Rust (`cargo test`):**
  - Formatters: markdown com/sem summary; SRT com 1 e N segmentos; `format_srt_timestamp` (0ms, <1s, >1h); texto puro.
  - Store: `save_segments`/`get_segments` roundtrip; cascade delete; `has_segments` em list/get; criação da tabela em DB existente (migração idempotente).
  - Whisper: conversão centissegundos → ms (função pura).
- **Vitest:** `defaultExportFilename(title, date, format)` (slug, acentos, extensão).

## Critérios de aceite (da issue)

- [ ] Botões de export na TranscriptionView e na History
- [ ] Markdown inclui metadata (título, data) e summary quando existir
- [ ] SRT usa timestamps reais dos segmentos
- [ ] Copy to clipboard do transcript completo (já existente — verificar que segue funcionando)
