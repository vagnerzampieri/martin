# Resumo de Transcrições via Claude CLI

## Objetivo

Adicionar a capacidade de gerar resumos de transcrições usando o Claude Code CLI (`claude -p`). O resumo inclui um resumo geral e key points, é persistido no banco e exibido na TranscriptionView.

## Decisões

- **Persistência:** campo `summary TEXT` nullable na tabela `transcriptions`
- **Abordagem:** comando Tauri síncrono (mesmo padrão de subprocessos do projeto)
- **Detecção do CLI:** comando `check_claude_cli()` no startup; botão desabilitado se ausente
- **Localização do botão:** apenas na lista do History, visível só para transcrições sem resumo
- **Indicador:** transcrições com resumo mostram ícone/badge no History
- **Exibição do resumo:** na TranscriptionView, abaixo do texto original

## Banco de dados

Migration:

```sql
ALTER TABLE transcriptions ADD COLUMN summary TEXT;
```

Struct `Transcription` atualizado:

```rust
pub struct Transcription {
    pub id: i64,
    pub title: String,
    pub text: String,
    pub language: String,
    pub duration_secs: f64,
    pub created_at: String,
    pub summary: Option<String>,
}
```

Novo método no `Store`:

- `save_summary(id: i64, summary: &str)` — UPDATE SET summary WHERE id

Queries de `list` e `get` atualizadas para incluir `summary` no SELECT.

## Backend — Comandos Tauri

### `check_claude_cli() -> bool`

- Executa `which claude`
- Retorna `true` se exit code 0, `false` caso contrário

### `summarize_transcription(id: i64) -> Result<String, String>`

1. Busca transcrição por `id` no banco
2. Monta prompt com instrução de resumo + key points
3. Executa `claude -p` passando o texto via stdin (evita limite de argumento)
4. Captura stdout como resumo
5. Salva no banco via `save_summary(id, summary)`
6. Retorna o resumo

Prompt enviado ao Claude:

```
Resuma esta transcrição de reunião. Inclua um resumo geral e os key points principais:

{transcription_text}
```

## Frontend

### History.svelte

- Chama `check_claude_cli()` no mount e guarda resultado em `$state`
- Cada item sem `summary`: botão "Resumir" (desabilitado se CLI ausente)
- Cada item com `summary`: indicador visual (ícone/badge)
- Ao clicar "Resumir": loading state no botão ("Resumindo..."), chama `summarize_transcription(id)`, atualiza lista ao retornar

### TranscriptionView.svelte

- Se `summary` presente: exibe seção "Resumo" abaixo do texto original
- Se `summary` ausente: nada extra

### i18n.js

Novas chaves (pt e en):

- `summarize` — "Resumir" / "Summarize"
- `summarizing` — "Resumindo..." / "Summarizing..."
- `summary` — "Resumo" / "Summary"
- `claudeNotAvailable` — "Claude CLI não disponível" / "Claude CLI not available"

## Fluxo do usuário

1. Abre o app, History carrega lista de transcrições
2. Frontend verifica se `claude` está disponível
3. Transcrições sem resumo mostram botão "Resumir" (habilitado/desabilitado conforme CLI)
4. Usuário clica "Resumir" → botão mostra "Resumindo..." com loading
5. Backend busca texto, chama `claude -p`, salva resumo no banco
6. Botão some, indicador de resumo aparece no item
7. Usuário clica na transcrição → TranscriptionView mostra texto + resumo

## Dependências

- Claude Code CLI instalado e autenticado na máquina do usuário
- Sem novas crates ou packages npm
