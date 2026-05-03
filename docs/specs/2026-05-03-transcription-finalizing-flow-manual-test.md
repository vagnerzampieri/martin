# Manual Test Guide — Transcription Finalizing Flow (v0.2.0)

Roteiro para validar a branch `feat/finalizing-flow` antes do merge.
Inclui isolamento da base de dados real para que os testes não toquem suas transcrições existentes.

---

## Antes de começar

### Por que isolar a base de dados?

A migração da Task 1 adiciona uma coluna `status` à tabela `transcriptions` automaticamente quando o app abre o DB. **Isso é seguro e idempotente** (rodar duas vezes não dá erro, e linhas antigas são preenchidas com `status='complete'`).

Mas, durante o smoke test, você vai gerar várias transcrições de teste, cancelar jobs, force-kill o processo, etc. Para não poluir nem arriscar seu DB de uso real, isole o data dir.

Localização atual do DB:

```
~/.local/share/com.nuuvem.martin/martin.db
```

Modelo Whisper:

```
~/.local/share/com.nuuvem.martin/models/ggml-small.bin
```

### Estratégia de isolamento

Use a variável `XDG_DATA_HOME` para redirecionar TUDO que o app lê e grava (DB + modelo + cache do WebKit). O modelo é grande (~466 MB), então copie ele do diretório real para o sandbox antes de rodar — assim o app não precisa baixar de novo.

**Setup:**

```bash
# 1. Crie um diretório isolado
export TEST_HOME="$HOME/.local/share-martin-test"
mkdir -p "$TEST_HOME/com.nuuvem.martin/models"

# 2. Copie o modelo já baixado para o sandbox (evita re-download de ~466 MB)
cp ~/.local/share/com.nuuvem.martin/models/ggml-small.bin \
   "$TEST_HOME/com.nuuvem.martin/models/"

# 3. Aponte XDG_DATA_HOME para o sandbox APENAS nesta sessão
export XDG_DATA_HOME="$TEST_HOME"

# 4. Confirme que o sandbox está ativo
echo "Test data dir: $XDG_DATA_HOME/com.nuuvem.martin"
ls "$XDG_DATA_HOME/com.nuuvem.martin/"
```

**Importante:** rode todos os comandos `cargo tauri dev` desta seção no MESMO terminal onde `XDG_DATA_HOME` foi exportado. Se abrir um terminal novo, refaça os passos 3 e 4.

**Cleanup quando terminar (opcional):**

```bash
rm -rf "$TEST_HOME"
unset XDG_DATA_HOME
```

Seu DB real em `~/.local/share/com.nuuvem.martin/` permanece intacto.

### Helper para inspecionar o DB de teste

Em outro terminal (sem precisar do `XDG_DATA_HOME`), você pode olhar o DB de teste diretamente:

```bash
sqlite3 "$HOME/.local/share-martin-test/com.nuuvem.martin/martin.db" \
  "SELECT id, status, substr(text,1,40) AS text_preview, duration_secs, title \
   FROM transcriptions ORDER BY id DESC LIMIT 10"
```

E listar arquivos WAV pendentes:

```bash
ls -la "$HOME/.local/share-martin-test/com.nuuvem.martin/"*.wav 2>/dev/null
```

---

## Subir a branch em modo dev

```bash
cd ~/Projects/study/martin
git checkout feat/finalizing-flow

# Garanta que XDG_DATA_HOME está apontado para o sandbox
echo "$XDG_DATA_HOME"

cargo tauri dev
```

Isso compila o backend Rust (primeira vez demora alguns minutos), abre o app com o data dir isolado e libera o frontend Svelte com hot-reload.

---

## Cenários de teste

Para cada cenário, anote: ✅ passou / ❌ falhou + observação. No fim do roteiro tem uma tabela para preencher.

### A — Ditado curto (~10s)

**Objetivo:** validar o fluxo feliz do ditado.

**Passos:**
1. No app, clique em **Ditado**.
2. Clique em **Iniciar Ditado**.
3. Fale por ~10 segundos (qualquer coisa, ex.: "teste um, teste dois, teste três").
4. Clique em **Parar Ditado**.

**Esperado:**
- O `FinalizingProgress` aparece (anel circular).
- O percentual avança (pode começar em "Carregando modelo…" e depois mostrar % real).
- Texto ao vivo aparece embaixo do anel à medida que o whisper processa.
- Ao terminar, o app vai para a tela da transcrição.
- Em **Histórico**, a transcrição aparece SEM badge "Parcial".

**Verificação SQL:**
```sql
-- Última linha deve ter status='complete'
SELECT id, status, duration_secs, substr(text,1,40)
FROM transcriptions ORDER BY id DESC LIMIT 1;
```

---

### B — Ditado longo (~60s)

**Objetivo:** validar live-text durante gravação + nav lock + finalize com mais texto.

**Passos:**
1. **Ditado** → **Iniciar Ditado**.
2. Fale continuamente por ~60 segundos.
3. Durante a gravação, observe o texto ao vivo aparecendo no painel.
4. Tente clicar em **Gravar** ou **Ditado** — devem estar visualmente desabilitados (cinza, cursor "not-allowed").
5. Clique em **Histórico** — deve funcionar normalmente (não locked).
6. Volte para **Ditado** clicando lá. Clique em **Parar Ditado**.

**Esperado:**
- Texto ao vivo durante a gravação se atualiza a cada 3-5s.
- Após Parar, vai para tela de finalize com anel.
- Durante finalize, **Gravar** e **Ditado** ficam com `aria-disabled` (visual de cinza); **Histórico** continua clicável.
- Tooltip "Aguarde a transcrição terminar" aparece ao passar o mouse nos botões locked.
- Transcrição final salva com status='complete'.

---

### C — Parar ditado em <1s

**Objetivo:** validar o caminho de quase-sem-áudio.

**Passos:**
1. **Ditado** → **Iniciar Ditado**.
2. Espere menos de 1 segundo. **Parar Ditado** imediatamente.

**Esperado:**
- Sem mensagem de erro do tipo "No text was transcribed" (essa lógica foi removida).
- Pode ficar uma linha com texto vazio ou texto curto, mas com status='complete' eventualmente.
- Ou, se o whisper não produziu nada, a linha fica `partial` com text='' e duration=0 — e na próxima abertura do app é varrida pela `delete_empty_partials`.

**Verificação SQL após o teste:**
```sql
SELECT id, status, text, duration_secs FROM transcriptions ORDER BY id DESC LIMIT 1;
```

---

### D — Cancelar a transcrição no meio do finalize

**Objetivo:** validar o fluxo de cancelamento com modal de confirmação.

**Passos:**
1. **Ditado** → **Iniciar** → fale ~30s → **Parar Ditado**.
2. Enquanto o anel de finalize está rodando, clique no botão **Cancelar** (abaixo do anel).
3. Modal de confirmação aparece: "Cancelar transcrição? O conteúdo será descartado."
4. Verifique que o foco inicial está em **Voltar** (botão azul, à esquerda).
5. Pressione **Tab** — foco vai para **Sim, cancelar** (botão vermelho).
6. Pressione **Esc** — modal fecha, finalize continua rodando.
7. Repita: clique em Cancelar, espere o modal abrir, clique em **Sim, cancelar**.

**Esperado:**
- Modal abre com `role="dialog"`, focus trap, foco inicial no botão seguro (Voltar).
- Esc dismiss o modal.
- Ao confirmar, o evento `transcription://cancelled` chega; UI volta para idle.
- A linha da transcrição é deletada do DB.

**Verificação SQL:**
```sql
-- A transcrição cancelada não deve estar listada
SELECT COUNT(*) FROM transcriptions;
```

---

### E — Gravação pendente → Transcrever

**Objetivo:** validar o fluxo do `transcribe_pending_recording`.

**Passos:**
1. **Gravar** → **Iniciar Gravação** → fale ~15s → **Parar Gravação**.
2. Aparece a lista de "Gravações pendentes" com a linha recém-gravada.
3. Clique em **Transcrever** na linha pendente.

**Esperado:**
- O `FinalizingProgress` aparece imediatamente (sem delay grande para carregamento).
- Na finalização: a linha pendente some da lista; o WAV é deletado do disco; a transcrição aparece em Histórico.

**Verificação:**
```bash
# Pendentes vs WAVs em disco
sqlite3 "$XDG_DATA_HOME/com.nuuvem.martin/martin.db" "SELECT * FROM pending_recordings"
ls "$XDG_DATA_HOME/com.nuuvem.martin/"*.wav 2>/dev/null || echo "sem WAVs"
```

---

### F — Cancelar transcrição de gravação pendente

**Objetivo:** validar a assimetria de cleanup do plano: cancel preserva WAV + linha pendente.

**Passos:**
1. Faça uma gravação pendente nova de ~30s.
2. Clique em **Transcrever**.
3. No finalize, clique em **Cancelar** → **Sim, cancelar**.
4. Verifique a lista de pendentes (clique fora e volte para **Gravar** se preciso).
5. Clique em **Transcrever** novamente na MESMA linha pendente.

**Esperado:**
- Após cancel: a linha pendente AINDA está lá (não foi removida); o WAV AINDA está no disco.
- Clicar em Transcrever de novo finaliza com sucesso.

**Verificação SQL após cancel (passo 3):**
```sql
SELECT * FROM pending_recordings;
-- A linha deve continuar aqui
SELECT * FROM transcriptions WHERE status='partial';
-- Não deve haver linha partial órfã (o cancel deletou ela)
```

---

### G — Iniciar segundo job enquanto outro está rodando

**Objetivo:** validar o lock single-job no backend.

**Passos:**
1. **Ditado** → **Iniciar** → fale ~20s → **Parar Ditado**.
2. Durante o finalize, tente clicar em **Gravar** (deve estar visualmente desabilitado).
3. Tente clicar em **Ditado** (mesmo).
4. Clique em **Histórico** — funciona normalmente.

**Esperado:**
- Clicar em Gravar/Ditado durante finalize não navega — o `aria-disabled` + handler condicional evitam.
- Histórico fica navegável.
- Se você invocar `transcribe_pending_recording` ou `stop_dictation` programaticamente via DevTools enquanto há um job ativo, o backend responde com erro `"Another transcription is in progress"`.

**Verificação programática (DevTools console):**
```js
// Durante um finalize ativo
await window.__TAURI__.core.invoke('cancel_job') // OK, sempre permitido
await window.__TAURI__.core.invoke('transcribe_pending_recording', {
  pendingId: 1, title: 'x', language: 'pt'
}) // Deve rejeitar com "Another transcription is in progress"
```

---

### H — Force-kill no meio do finalize

**Objetivo:** validar a recovery de partial rows na próxima abertura.

**Passos:**
1. **Ditado** → **Iniciar** → fale ~30s → **Parar Ditado**.
2. Durante o finalize, em outro terminal, mate o processo:
   ```bash
   pkill -9 martin
   ```
3. Reabra o app: `cargo tauri dev` (se você fechou o dev) — ou refresh do app já reabre.
4. Vá em **Histórico**.

**Esperado:**
- Existe uma transcrição com badge "Parcial" amarelo ao lado do título.
- O texto está preenchido até o último chunk persistido pelo debounce de 1s antes do kill.
- Se você matou ANTES do primeiro segmento ter sido emitido (kill quase imediato após Parar), na reabertura **não há linha fantasma** — `delete_empty_partials` no startup limpa qualquer linha com text='' e duration=0.

**Verificação SQL após reabrir:**
```sql
SELECT id, status, substr(text,1,40), duration_secs, title
FROM transcriptions ORDER BY id DESC LIMIT 5;
```

E olhe os logs do `cargo tauri dev` — deve ter alguma linha como `[startup] swept N empty partial transcription(s)` se aplicável.

---

### I — Erro do worker (modelo ausente / panic)

**Objetivo:** validar que erros de worker emitem `transcription://error` e desbloqueiam a UI.

**Passo (modelo ausente):**
1. Pare `cargo tauri dev`.
2. Renomeie o modelo:
   ```bash
   mv "$XDG_DATA_HOME/com.nuuvem.martin/models/ggml-small.bin" \
      "$XDG_DATA_HOME/com.nuuvem.martin/models/ggml-small.bin.bak"
   ```
3. Reabra `cargo tauri dev`.
4. Tente um ditado.

**Esperado:**
- O app pode oferecer download do modelo de novo (UI de ModelDownload).
- Se você ignorar e tentar Parar Ditado, o worker tenta `Transcriber::new` no caminho ausente, falha, emite `transcription://error`, a UI mostra a mensagem e o nav volta a ficar livre.

**Cleanup deste cenário:**
```bash
mv "$XDG_DATA_HOME/com.nuuvem.martin/models/ggml-small.bin.bak" \
   "$XDG_DATA_HOME/com.nuuvem.martin/models/ggml-small.bin"
```

Para forçar um panic real do worker é mais complexo — não inclua se o cenário acima cobrir o erro-path.

---

## Folha de resultados

Preencha conforme for testando:

| # | Cenário | Resultado | Observações |
|---|---------|-----------|-------------|
| A | Ditado curto |   |   |
| B | Ditado longo + nav lock |   |   |
| C | Stop em <1s |   |   |
| D | Cancel + modal |   |   |
| E | Pendente → Transcrever |   |   |
| F | Pendente cancel + retry |   |   |
| G | Segundo job concorrente |   |   |
| H | Force-kill mid-finalize |   |   |
| I | Erro de modelo ausente |   |   |

Observações livres / bugs encontrados:
- 
- 
- 

---

## Quando terminar

1. Limpe o sandbox: `rm -rf "$TEST_HOME"`.
2. Volte a usar o app normalmente — seu DB real em `~/.local/share/com.nuuvem.martin/` foi preservado.
3. Se tudo passou, peça merge / PR. Se algo falhou, reporte os cenários afetados para que eu possa investigar.
