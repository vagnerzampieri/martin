# Record While Transcribing — Design

**Date:** 2026-05-28
**Status:** Approved for planning
**Feature:** 1 of 3 in the transcription-throughput set (import ✓ shipped in v0.4.0; parallel chunking ✗ abandoned; **record-while-transcribing** — this spec)

## Problem

When a pending recording is finalizing (Whisper transcription on a long
file can take minutes on slow hardware), the entire UI is locked:
`appBusy` blocks navigation, the `FinalizingProgress` overlay dominates
the Record tab, and the user cannot start a new recording. For a
back-to-back meeting workflow (finish one, immediately start the next),
this forces the user to wait.

## Goals

- While a finalize is running, allow **starting a new recording** and
  **importing an audio file** — the parts that do not need the Whisper
  model.
- Show finalize progress + live text + cancel as a **non-modal banner**
  visible across all tabs, not as a full-screen overlay that monopolizes
  one tab.
- Keep "one transcription at a time" — the Whisper model is a single
  shared resource and the existing `current_job` lock is the right
  guarantee.

## Non-Goals

- Allowing **dictation** to start during a finalize — dictation needs
  the Whisper `Transcriber`, which the finalize is already using.
  Letting both run would require loading the model twice (~466 MB
  extra RAM) or queueing, both out of scope.
- Allowing a **second pending transcription** to start during a
  finalize — `current_job` still serializes whole jobs.
- Auto-queuing pending transcriptions (when finalize A finishes, start
  finalize B automatically) — YAGNI. The user clicks Transcribe when
  ready.

## Context (current code)

- `transcribe_pending_recording` in `src-tauri/src/lib.rs` is invoked
  from `Recorder.svelte`'s `transcribePending`, which sets
  `appBusy.set(true)`, sets `phase = 'finalizing'`, and renders
  `FinalizingProgress` inline in the Record tab. The finalize event
  subscriptions (`transcription://text|progress|complete|cancelled|error`)
  live inside `Recorder.svelte`'s `onMount`.
- `appBusy` (`src/lib/appBusy.js`) is a global Svelte writable used by
  the page/layout to lock navigation while a job runs.
- `start_recording` (lib.rs) only locks `state.capture` — it does **not**
  touch `state.transcriber` or `state.current_job`. So the backend
  already permits recording during a finalize; the constraint is purely
  frontend.
- `import_audio_file` (lib.rs) also does not touch the transcriber or
  current_job (verified in v0.4.0). Safe during a finalize.
- `start_dictation` and `transcribe_pending_recording` both call
  `state.transcriber.lock()?.take()`; while one runs, the other returns
  "Another transcription is in progress" (current_job lock).

The required change is entirely frontend: lift the finalize state from
`Recorder.svelte` to global scope, render it as a non-modal banner, and
drop the `appBusy` lock so recording entry points stay reachable.

## Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Scope | Allow recording + import during finalize; block dictation + second transcribe | Recording/import don't need Transcriber; dictation/transcribe do |
| Finalize UI | Non-modal banner, persistent across tabs | Lets user navigate (Record/Dictation/History) while finalize runs |
| State location | Global Svelte store (`finalizeProgress`) | Cross-tab visibility; single Tauri event subscription |
| Backend changes | None | `start_recording`/`import_audio_file` already permit it; `current_job` lock already enforces serialization |
| `appBusy` | Repurposed/removed for the finalize case | Recording entry must stay reachable; navigation must stay free |
| Disabled controls during finalize | Start Dictation, Transcribe (on pending list), Cancel of the recorder while recording | Reflect the real backend constraint with clear tooltips |
| Auto-queue of pendings | No | YAGNI; explicit user action keeps behavior predictable |

## Architecture

### New global store — `src/lib/finalizeProgress.js`

A Svelte writable store seeded once at app startup with the current
finalize state. Holds:

```
{
    id: number | null,         // transcription row id of the running job, or null
    percent: number,           // 0-100
    liveText: string,
    phase: 'idle' | 'finalizing' | 'cancelling',
    jobLabel: string,          // e.g. "Recorded: 23m 37s"
    startedFromPendingId: number | null,
}
```

On import (called once from `+layout.svelte` or `+page.svelte`), it
registers Tauri listeners for `transcription://progress`,
`transcription://text`, `transcription://complete`,
`transcription://cancelled`, and `transcription://error` that update
the store. The store exposes:

- `beginFinalize({ id, pendingId, jobLabel })` — call from the
  transcribe-pending action to seed `phase = 'finalizing'`.
- `requestCancel()` — calls `invoke('cancel_job')` and sets phase to
  `'cancelling'`.
- The store auto-resets to `idle` on `complete`/`cancelled`/`error` events.

### Banner component — `src/lib/FinalizeBanner.svelte`

A compact, non-modal banner (e.g., fixed bar at the top of the app,
~56 px tall) shown when `phase !== 'idle'`. Contains:

- Job label (e.g., "Recorded: 23m 37s").
- Small ring + percent (same SVG ring as `FinalizingProgress`, scaled
  down to fit the bar).
- A **Show details** toggle that expands a panel below with the live
  text (the existing scrollable `<pre>` with `max-height: 40vh`).
- Cancel button (uses existing confirmation modal flow).

Reuses the existing CSS rules from `FinalizingProgress.svelte` where
sensible (ring math, modal-backdrop, btn-cancel styling).

`FinalizingProgress.svelte` itself is **kept for the Dictation finalize
path** (which still goes through `stop_dictation` → `run_finalize_dictation`
and renders the same modal flow), unless that path also migrates — for
this feature, leave dictation as-is. (Dictation finalize already uses
the global Transcriber lock and blocks navigation, which matches the
ditado modal expectation. If migration is desired later, separate work.)

Decision: **for this feature only the PENDING-file transcribe path
uses the new banner**. The dictation `stop_dictation` flow continues to
use the existing inline `FinalizingProgress` overlay. Two reasons: (a)
dictation finalize is typically short (tail-only), so a modal overlay
is fine; (b) dictation cannot coexist with recording anyway (same mic),
so unblocking the UI offers no value during dictation finalize.

### Mounting the banner

Render `<FinalizeBanner />` once in `src/routes/+page.svelte` at the
root of the page layout, above the tab content. It self-shows when
`$finalizeProgress.phase !== 'idle'`.

### Changes to `Recorder.svelte`

- Remove the local `phase`/`liveText`/`percent`/`pendingDurationLabel`/
  `startedFromPendingId` state and the inline `FinalizingProgress`
  render.
- Remove the `transcription://*` listeners from `onMount` (the global
  store owns them now).
- Remove `appBusy.set(true)` from `transcribePending`. Replace with
  `finalizeProgress.beginFinalize({ id, pendingId, jobLabel })` (the
  store will flip to `'finalizing'` synchronously and the banner will
  appear).
- The Transcribe button on a pending row becomes `disabled` when
  `$finalizeProgress.phase !== 'idle'`, with a tooltip from
  `t('transcribeBusy')`.
- The Start Recording, Stop Recording, and Import audio buttons stay
  enabled during finalize.

### Changes to `Dictation.svelte`

- Start Dictation button becomes `disabled` when
  `$finalizeProgress.phase !== 'idle'`, with a tooltip from
  `t('dictationBusy')`. (The backend would reject it anyway with
  "Another transcription is in progress"; this surfaces the constraint
  proactively.)

### Changes to `appBusy`

`appBusy` was used to lock navigation during finalize. With the banner
approach, navigation is intentionally free. Drop the
`appBusy.set(true)` calls in `Recorder.svelte`'s `transcribePending`.
Keep `appBusy` in the codebase if the dictation finalize path still
uses it (audit during implementation); otherwise it becomes unused and
can be removed. Treat removal as cleanup if the only remaining caller
goes away.

## Data Flow

```
User clicks "Transcribe" on a pending recording
  → Recorder.svelte: finalizeProgress.beginFinalize({...}) + invoke("transcribe_pending_recording", ...)
  → store flips phase='finalizing' → <FinalizeBanner /> shows at top
  → user navigates freely; clicks Start Recording on Record tab
  → start_recording invoke succeeds (capture lock, no transcriber)
  → user stops recording later → new pending appears in list (its Transcribe button is disabled while banner shows)
  → meanwhile, transcription://progress + text events update the store → banner updates
  → on transcription://complete: store flips to idle, banner hides, transcribe buttons re-enable
  → user clicks Transcribe on the new pending → starts the next finalize
```

## i18n strings (pt/en)

- `transcribeBusy` — "Aguarde a transcrição atual" / "Wait for the current transcription"
- `dictationBusy` — "Aguarde a transcrição atual" / "Wait for the current transcription"
- `finalizeBannerLabel` — "Transcrevendo em segundo plano" / "Transcribing in background"
- `showDetails` — "Detalhes" / "Details"
- `hideDetails` — "Esconder" / "Hide"

## Testing

- **No new automated tests** (no backend changes; frontend has no test
  harness in this project).
- **Manual matrix** (the only validation that matters here):
  1. Start a long pending transcription. Confirm the banner appears at
     the top.
  2. While it runs, click Start Recording, record briefly, click Stop.
     A new pending appears in the list; its Transcribe button is
     disabled with the tooltip.
  3. While the original finalize still runs, navigate to History tab —
     banner stays visible at the top; the previously-saved
     transcriptions are browsable.
  4. Try to click Start Dictation — disabled with tooltip.
  5. Try Import audio — works; new pending appears; its Transcribe
     button is also disabled.
  6. Cancel the original finalize from the banner — banner hides, all
     Transcribe and Start Dictation buttons re-enable, the cancelled
     row is discarded but the pending recording survives (existing
     behavior).
  7. Wait for the original to complete normally → banner hides → click
     Transcribe on a newly-recorded pending → starts a fresh banner.

## File Structure

- Create: `src/lib/finalizeProgress.js` — store + Tauri listeners +
  `beginFinalize`/`requestCancel` helpers.
- Create: `src/lib/FinalizeBanner.svelte` — non-modal banner UI,
  cancel confirmation modal (or reuses existing one).
- Modify: `src/lib/Recorder.svelte` — remove inline finalize state +
  listeners + appBusy call; disable Transcribe button while banner active.
- Modify: `src/lib/Dictation.svelte` — disable Start Dictation while
  banner active.
- Modify: `src/routes/+page.svelte` — mount `<FinalizeBanner />` at the
  top of the layout.
- Modify: `src/lib/i18n.js` — add the 5 strings above (pt + en).
- Possible delete (if no remaining callers after the migration):
  `src/lib/appBusy.js` — audit during implementation.

## Out of Scope / Follow-ups

- Migrating the **dictation** finalize path to the same global banner
  (would require resolving dictation+recording mutual exclusion via the
  mic, not just the model).
- **Auto-queue** of pendings (finalize-complete triggers next pending).
- Concurrent **dictation + transcription** (requires loading the model
  twice or implementing model sharing across threads — large work; the
  Feature 2 investigation showed whisper.cpp concurrency is fragile in
  the app environment).
