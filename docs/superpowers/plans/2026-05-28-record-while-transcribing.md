# Record While Transcribing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the user start a new recording or import an audio file while a pending transcription finalizes, by lifting the finalize state into a global Svelte store and rendering a non-modal banner across all tabs.

**Architecture:** Frontend-only. A new `finalizeProgress` store owns the Tauri `transcription://*` event subscriptions and exposes `beginFinalize`/`requestCancel`. A new `FinalizeBanner` component renders the in-progress finalize as a sticky bar in the page layout. `Recorder.svelte` loses its inline `FinalizingProgress` + `appBusy` calls for the pending-file path and just calls `beginFinalize`. `Dictation.svelte` disables Start Dictation while the banner is active. The dictation finalize path is untouched (still uses `FinalizingProgress` + `appBusy`, since the mic is exclusive anyway).

**Tech Stack:** Svelte 5 (runes), Tauri 2 events, plain JS module for the store. No backend changes; no new dependencies.

**Spec:** `docs/superpowers/specs/2026-05-28-record-while-transcribing-design.md`

---

## File Structure

- Create: `src/lib/finalizeProgress.js` — Svelte writable store + Tauri event subscriptions + `beginFinalize`/`requestCancel` helpers + DOM custom events on terminal transitions.
- Create: `src/lib/FinalizeBanner.svelte` — sticky non-modal banner that subscribes to the store; ring + percent + label + Details toggle + Cancel (with confirmation modal).
- Modify: `src/lib/i18n.js` — 5 new strings (pt + en).
- Modify: `src/routes/+page.svelte` — call `initFinalizeListeners()` once on mount; render `<FinalizeBanner />` at top of layout; listen to the `finalize:complete` window event to navigate to the new transcription.
- Modify: `src/lib/Recorder.svelte` — drop inline finalize state, listeners, `appBusy.set` calls, and the `FinalizingProgress` render; call `beginFinalize` from `transcribePending`; disable Transcribe buttons while the banner is active; listen to `finalize:complete`/`cancelled`/`error` window events to update the pending list.
- Modify: `src/lib/Dictation.svelte` — disable Start Dictation while the banner is active (with tooltip). Do NOT touch the dictation finalize path itself.

No backend (Rust) changes. No test-harness changes (frontend has none in this repo; verification is manual per the spec).

---

## Task 1: i18n strings

**Files:**
- Modify: `src/lib/i18n.js`

- [ ] **Step 1: Add Portuguese strings**

In `src/lib/i18n.js`, inside the `pt` object (after the existing last key, before the closing `}`), add:
```javascript
    transcribeBusy: "Aguarde a transcrição atual",
    dictationBusy: "Aguarde a transcrição atual",
    finalizeBannerLabel: "Transcrevendo em segundo plano",
    showDetails: "Detalhes",
    hideDetails: "Esconder",
```

- [ ] **Step 2: Add English strings**

In the `en` object (after the existing last key), add:
```javascript
    transcribeBusy: "Wait for the current transcription",
    dictationBusy: "Wait for the current transcription",
    finalizeBannerLabel: "Transcribing in background",
    showDetails: "Details",
    hideDetails: "Hide",
```

- [ ] **Step 3: Type-check**

Run: `npm run check`
Expected: error count does NOT increase above the pre-existing baseline (~37 errors in this project). The new keys should not introduce new errors.

- [ ] **Step 4: Commit**

```bash
git add src/lib/i18n.js
git commit -m "feat(i18n): add strings for record-while-transcribing banner"
```
NO `Co-Authored-By` trailer (project preference).

---

## Task 2: finalizeProgress store

**Files:**
- Create: `src/lib/finalizeProgress.js`

- [ ] **Step 1: Create the store file**

Create `src/lib/finalizeProgress.js` with exactly this content:
```javascript
import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

// Global, single-source-of-truth state for the pending-file finalize that
// the chunked/sequential transcription path produces. The dictation finalize
// keeps its own modal flow.
//
// Shape:
//   id:                   number | null  — transcription row id
//   percent:              number         — 0-100
//   liveText:             string         — accumulated text emitted by whisper
//   phase:                'idle' | 'finalizing' | 'cancelling'
//   jobLabel:             string         — short caption (e.g. "Recorded: 23m 37s")
//   startedFromPendingId: number | null  — the pending row id this came from
const initial = {
    id: null,
    percent: 0,
    liveText: "",
    phase: "idle",
    jobLabel: "",
    startedFromPendingId: null,
};

export const finalizeProgress = writable(initial);

let initialized = false;

/**
 * Register the Tauri event listeners that drive the store. Idempotent — safe
 * to call multiple times; only the first call subscribes.
 */
export async function initFinalizeListeners() {
    if (initialized) return;
    initialized = true;

    await listen("transcription://text", (event) => {
        finalizeProgress.update((s) => {
            if (s.phase === "idle") return s;
            const incoming = event.payload?.text ?? "";
            // Monotonic guard — never replace a longer accumulated text with
            // a shorter one if events arrive out of order.
            if (incoming.length > s.liveText.length) {
                return { ...s, liveText: incoming };
            }
            return s;
        });
    });

    await listen("transcription://progress", (event) => {
        finalizeProgress.update((s) => {
            if (s.phase === "idle") return s;
            return { ...s, percent: event.payload?.percent ?? s.percent };
        });
    });

    await listen("transcription://complete", (event) => {
        const transcription = event.payload?.transcription;
        finalizeProgress.update((s) =>
            s.phase === "idle" ? s : { ...s, percent: 100 },
        );
        // Reset to idle after a brief moment so the bar can show 100% briefly.
        setTimeout(() => finalizeProgress.set({ ...initial }), 250);
        // Broadcast for side effects (remove pending from list, navigate).
        window.dispatchEvent(
            new CustomEvent("finalize:complete", { detail: transcription }),
        );
    });

    await listen("transcription://cancelled", () => {
        finalizeProgress.set({ ...initial });
        window.dispatchEvent(new CustomEvent("finalize:cancelled"));
    });

    await listen("transcription://error", (event) => {
        finalizeProgress.update((s) => ({ ...s, phase: "idle" }));
        window.dispatchEvent(
            new CustomEvent("finalize:error", {
                detail: event.payload?.error ?? "Unknown error",
            }),
        );
    });
}

/**
 * Flip the store into the finalizing phase. Call this from the action that
 * invokes `transcribe_pending_recording`, BEFORE awaiting the invoke, so the
 * banner appears immediately.
 */
export function beginFinalize({ id, pendingId, jobLabel }) {
    finalizeProgress.set({
        id: id ?? null,
        percent: 0,
        liveText: "",
        phase: "finalizing",
        jobLabel: jobLabel ?? "",
        startedFromPendingId: pendingId ?? null,
    });
}

/**
 * Ask the backend to cancel the in-progress finalize. The terminal
 * `cancelled` event will reset the store.
 */
export async function requestCancel() {
    finalizeProgress.update((s) => ({ ...s, phase: "cancelling" }));
    try {
        await invoke("cancel_job");
    } catch (e) {
        console.error("[finalize] cancel_job failed:", e);
    }
}
```

- [ ] **Step 2: Type-check**

Run: `npm run check`
Expected: no new errors beyond the baseline.

- [ ] **Step 3: Commit**

```bash
git add src/lib/finalizeProgress.js
git commit -m "feat(finalize): global store + tauri listeners for pending finalize"
```

---

## Task 3: FinalizeBanner component

**Files:**
- Create: `src/lib/FinalizeBanner.svelte`

- [ ] **Step 1: Create the component**

Create `src/lib/FinalizeBanner.svelte` with exactly this content:
```svelte
<script>
    import { finalizeProgress, requestCancel } from "./finalizeProgress.js";
    import { t } from "./i18n.js";

    let expanded = $state(false);
    let confirming = $state(false);

    const SIZE = 36;
    const STROKE = 4;
    const RADIUS = (SIZE - STROKE) / 2;
    const CIRCUMFERENCE = 2 * Math.PI * RADIUS;

    let percent = $derived($finalizeProgress.percent);
    let dashOffset = $derived(
        CIRCUMFERENCE * (1 - Math.max(0, Math.min(100, percent)) / 100),
    );
    let cancelling = $derived($finalizeProgress.phase === "cancelling");

    function openConfirm() {
        if (!cancelling) confirming = true;
    }
    function dismissConfirm() {
        confirming = false;
    }
    function confirmCancel() {
        confirming = false;
        requestCancel();
    }
</script>

{#if $finalizeProgress.phase !== "idle"}
    <div class="banner" role="status" aria-live="polite">
        <div class="ring">
            <svg width={SIZE} height={SIZE} viewBox="0 0 {SIZE} {SIZE}">
                <circle
                    cx={SIZE / 2}
                    cy={SIZE / 2}
                    r={RADIUS}
                    fill="none"
                    stroke="var(--border)"
                    stroke-width={STROKE}
                />
                <circle
                    cx={SIZE / 2}
                    cy={SIZE / 2}
                    r={RADIUS}
                    fill="none"
                    stroke="var(--info)"
                    stroke-width={STROKE}
                    stroke-dasharray={CIRCUMFERENCE}
                    stroke-dashoffset={dashOffset}
                    stroke-linecap="round"
                    transform="rotate(-90 {SIZE / 2} {SIZE / 2})"
                    style="transition: stroke-dashoffset 200ms linear;"
                />
            </svg>
            <span class="percent">{Math.round(percent)}%</span>
        </div>
        <span class="label">
            {$finalizeProgress.jobLabel || t("finalizeBannerLabel")}
        </span>
        <button
            class="btn-details"
            onclick={() => (expanded = !expanded)}
        >
            {expanded ? t("hideDetails") : t("showDetails")}
        </button>
        <button
            class="btn-cancel"
            onclick={openConfirm}
            disabled={cancelling}
            aria-disabled={cancelling}
        >
            {t("cancelTranscription")}
        </button>
    </div>
    {#if expanded && $finalizeProgress.liveText}
        <div class="live-text">
            <pre>{$finalizeProgress.liveText}</pre>
        </div>
    {/if}
{/if}

{#if confirming}
    <div class="modal-backdrop">
        <div class="modal" role="dialog" aria-modal="true">
            <h3>{t("cancelConfirmTitle")}</h3>
            <p>{t("cancelConfirmBody")}</p>
            <div class="modal-actions">
                <button class="btn-secondary" onclick={dismissConfirm}>
                    {t("cancelConfirmNo")}
                </button>
                <button class="btn-danger" onclick={confirmCancel}>
                    {t("cancelConfirmYes")}
                </button>
            </div>
        </div>
    </div>
{/if}

<style>
    .banner {
        position: sticky;
        top: 0;
        z-index: 50;
        display: flex;
        align-items: center;
        gap: 12px;
        padding: 8px 16px;
        background: var(--surface);
        border-bottom: 1px solid var(--border);
    }
    .ring {
        position: relative;
        width: 36px;
        height: 36px;
        flex-shrink: 0;
    }
    .percent {
        position: absolute;
        inset: 0;
        display: flex;
        align-items: center;
        justify-content: center;
        font-size: 0.7rem;
        font-weight: 600;
    }
    .label {
        flex: 1;
        font-size: 0.9rem;
        color: var(--text-muted);
    }
    .btn-details {
        font-size: 0.85rem;
        padding: 4px 10px;
        background: transparent;
        border: 1px solid var(--border);
        color: var(--text-muted);
    }
    .btn-cancel {
        font-size: 0.85rem;
        padding: 4px 10px;
        background: transparent;
        border: 1px solid var(--accent);
        color: var(--accent);
    }
    .btn-cancel[disabled] {
        opacity: 0.5;
        cursor: wait;
    }
    .live-text {
        max-width: 600px;
        margin: 0 auto;
        padding: 8px 16px;
    }
    .live-text pre {
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: var(--radius);
        padding: 12px;
        max-height: 30vh;
        overflow-y: auto;
        white-space: pre-wrap;
        word-wrap: break-word;
        font-family: inherit;
        font-size: 0.9rem;
        line-height: 1.5;
    }
    .modal-backdrop {
        position: fixed;
        inset: 0;
        background: rgba(0, 0, 0, 0.55);
        display: flex;
        align-items: center;
        justify-content: center;
        z-index: 100;
    }
    .modal {
        background: var(--surface);
        border-radius: var(--radius);
        padding: 24px;
        max-width: 360px;
        text-align: center;
    }
    .modal h3 {
        margin-bottom: 8px;
    }
    .modal p {
        color: var(--text-muted);
        margin-bottom: 20px;
    }
    .modal-actions {
        display: flex;
        justify-content: center;
        gap: 12px;
    }
    .btn-secondary {
        background: var(--primary);
        color: white;
        padding: 8px 16px;
    }
    .btn-danger {
        background: var(--accent);
        color: white;
        padding: 8px 16px;
    }
</style>
```

- [ ] **Step 2: Type-check**

Run: `npm run check`
Expected: no new errors beyond the baseline.

- [ ] **Step 3: Commit**

```bash
git add src/lib/FinalizeBanner.svelte
git commit -m "feat(finalize): non-modal banner for pending-file finalize"
```

---

## Task 4: Mount the banner + navigation handler in +page.svelte

**Files:**
- Modify: `src/routes/+page.svelte`

- [ ] **Step 1: Add imports**

In `src/routes/+page.svelte`, add to the `<script>` block (alongside the existing imports near the top):
```javascript
    import FinalizeBanner from "../lib/FinalizeBanner.svelte";
    import { initFinalizeListeners } from "../lib/finalizeProgress.js";
```

- [ ] **Step 2: Initialize listeners on mount and handle `finalize:complete`**

`+page.svelte` already has an `onMount` (used to invoke model-check). Inside the SAME `onMount` body, after the existing logic, add:
```javascript
        await initFinalizeListeners();
        const onComplete = (e) => {
            if (e.detail) {
                showTranscription(e.detail);
            }
        };
        window.addEventListener("finalize:complete", onComplete);
        return () => {
            window.removeEventListener("finalize:complete", onComplete);
        };
```
(`showTranscription` already exists in this file — it's the function bound to `onTranscribed={showTranscription}` on `<Recorder>` and `<Dictation>`.)

- [ ] **Step 3: Render the banner**

In the `<main>` block of `+page.svelte`, find the line `<main>` and add the banner immediately after it (before the tab nav):
```svelte
<main>
    <FinalizeBanner />
    <!-- existing tab nav + content -->
```

- [ ] **Step 4: Type-check + build**

Run: `npm run check`
Expected: no new errors beyond baseline.

Run: `cd src-tauri && cargo build`
Expected: compiles (the frontend changes don't affect Rust, but this confirms nothing accidentally broke).

- [ ] **Step 5: Commit**

```bash
git add src/routes/+page.svelte
git commit -m "feat(finalize): mount banner and navigate on finalize complete"
```

---

## Task 5: Refactor Recorder.svelte (the big one)

**Files:**
- Modify: `src/lib/Recorder.svelte`

This task removes the inline finalize plumbing from `Recorder.svelte`. Read the current file end-to-end before editing.

- [ ] **Step 1: Drop unused imports + state**

In the `<script>` block of `src/lib/Recorder.svelte`:

Remove these import lines (they're no longer used here):
```javascript
    import { appBusy } from "./appBusy.js";
    import FinalizingProgress from "./FinalizingProgress.svelte";
```

Add this import (for state observation + the transition action):
```javascript
    import { finalizeProgress, beginFinalize } from "./finalizeProgress.js";
```

Remove these state declarations (they're moving to the global store / are now derived):
```javascript
    let liveText = $state("");
    let percent = $state(0);
    /** @type {"idle" | "finalizing" | "cancelling"} */
    let phase = $state("idle");
    let pendingDurationLabel = $state("");
    /** @type {number | null} */
    let startedFromPendingId = null;
    /** @type {Array<() => void>} */
    let unlisteners = [];
```

Also remove the helper:
```javascript
    function isFinalizing() {
        return phase === "finalizing" || phase === "cancelling";
    }
```

- [ ] **Step 2: Replace `onMount` / `onDestroy` listener wiring**

The current `onMount` registers `transcription://*` listeners and pushes them onto `unlisteners`; `onDestroy` calls each unlistener and clears `timer`. The transcription listeners move to the global store; only the local DOM listeners (for pending-list side effects) and `timer` stay here.

Replace the current `onMount` block:
```javascript
    onMount(async () => {
        await loadPending();

        unlisteners.push(
            await listen("transcription://text", (event) => { ... }),
            await listen("transcription://progress", (event) => { ... }),
            await listen("transcription://complete", (event) => { ... }),
            await listen("transcription://cancelled", (_event) => { ... }),
            await listen("transcription://error", (event) => { ... }),
        );
    });
```
with:
```javascript
    function handleFinalizeComplete() {
        const removeId = $finalizeProgress.startedFromPendingId;
        if (removeId !== null) {
            pendingRecordings = pendingRecordings.filter((p) => p.id !== removeId);
        }
    }

    function handleFinalizeError(e) {
        error = e.detail ?? "Transcription error";
    }

    onMount(async () => {
        await loadPending();
        window.addEventListener("finalize:complete", handleFinalizeComplete);
        window.addEventListener("finalize:error", handleFinalizeError);
    });
```

Replace the current `onDestroy`:
```javascript
    onDestroy(() => {
        if (timer) clearInterval(timer);
        for (const u of unlisteners) u();
    });
```
with:
```javascript
    onDestroy(() => {
        if (timer) clearInterval(timer);
        window.removeEventListener("finalize:complete", handleFinalizeComplete);
        window.removeEventListener("finalize:error", handleFinalizeError);
    });
```

Remove the now-unused `listen` import line:
```javascript
    import { listen } from "@tauri-apps/api/event";
```

- [ ] **Step 3: Replace `transcribePending`**

The current body calls `appBusy.set(true)`, sets local `phase`/`liveText`/`pendingDurationLabel`, then invokes. Replace the entire function with:
```javascript
    async function transcribePending(id) {
        try {
            error = "";
            const pending = pendingRecordings.find((p) => p.id === id);
            const jobLabel =
                pending && typeof pending.duration_secs === "number"
                    ? `${t("recordedDuration")} ${formatDuration(pending.duration_secs)}`
                    : "";
            const now = new Date().toLocaleString(
                locale === "pt" ? "pt-BR" : "en-US",
            );
            // Optimistically flip the global finalize state so the banner shows
            // immediately, before awaiting the (long) invoke.
            beginFinalize({ id: null, pendingId: id, jobLabel });
            const newId = await invoke("transcribe_pending_recording", {
                pendingId: id,
                title: `${t("meetingTitle")} ${now}`,
                language: locale,
            });
            // Fill in the row id now that the backend returned it.
            finalizeProgress.update((s) => ({ ...s, id: newId }));
        } catch (e) {
            error = e;
            // Roll back the optimistic store flip.
            finalizeProgress.set({
                id: null,
                percent: 0,
                liveText: "",
                phase: "idle",
                jobLabel: "",
                startedFromPendingId: null,
            });
        }
    }
```

- [ ] **Step 4: Remove `requestCancel`**

Cancel is now handled by the banner. Delete the local function:
```javascript
    async function requestCancel() { ... }
```

- [ ] **Step 5: Update the template**

Remove the inline `FinalizingProgress` render block:
```svelte
    {#if phase === "finalizing" || phase === "cancelling"}
        <FinalizingProgress
            {percent}
            {liveText}
            cancelling={phase === "cancelling"}
            jobLabel={pendingDurationLabel}
            onCancel={requestCancel}
        />
    {:else if !recording && !processing && pendingRecordings.length > 0}
        ...
    {/if}
```
Replace with (the pending list now shows unconditionally when there are pendings, and the Transcribe button is disabled while the global banner is active):
```svelte
    {#if !recording && !processing && pendingRecordings.length > 0}
        <div class="pending">
            <h3>{t("pendingRecordings")}</h3>
            <ul>
                {#each pendingRecordings as pending}
                    <li>
                        <div class="pending-info">
                            <span class="pending-date">{formatDate(pending.created_at)}</span>
                            <span class="pending-duration">{formatDuration(pending.duration_secs)}</span>
                        </div>
                        <div class="pending-actions">
                            <button
                                class="btn-transcribe"
                                onclick={() => transcribePending(pending.id)}
                                disabled={$finalizeProgress.phase !== "idle"}
                                aria-disabled={$finalizeProgress.phase !== "idle"}
                                title={$finalizeProgress.phase !== "idle" ? t("transcribeBusy") : ""}
                            >
                                {t("transcribe")}
                            </button>
                            <button
                                class="btn-delete"
                                onclick={() => deletePending(pending.id)}
                            >
                                ×
                            </button>
                        </div>
                    </li>
                {/each}
            </ul>
        </div>
    {/if}
```

- [ ] **Step 6: Type-check + build**

Run: `npm run check`
Expected: no new errors beyond baseline. If any error references one of the removed names (`liveText`, `percent`, `phase`, `pendingDurationLabel`, `startedFromPendingId`, `unlisteners`, `requestCancel`, `isFinalizing`), find and remove that lingering reference.

Run: `cd src-tauri && cargo build`
Expected: compiles.

- [ ] **Step 7: Commit**

```bash
git add src/lib/Recorder.svelte
git commit -m "feat(recorder): use global finalize store; drop inline overlay"
```

---

## Task 6: Disable Start Dictation while banner is active

**Files:**
- Modify: `src/lib/Dictation.svelte`

- [ ] **Step 1: Import the store**

In the `<script>` block of `src/lib/Dictation.svelte`, add (alongside other lib imports):
```javascript
    import { finalizeProgress } from "./finalizeProgress.js";
```

- [ ] **Step 2: Disable the Start Dictation button**

Find the Start Dictation button (around line 196):
```svelte
        <button class="btn-start" onclick={startDictation}>
            {t("startDictation")}
        </button>
```
Replace with:
```svelte
        <button
            class="btn-start"
            onclick={startDictation}
            disabled={$finalizeProgress.phase !== "idle"}
            aria-disabled={$finalizeProgress.phase !== "idle"}
            title={$finalizeProgress.phase !== "idle" ? t("dictationBusy") : ""}
        >
            {t("startDictation")}
        </button>
```

- [ ] **Step 3: Type-check**

Run: `npm run check`
Expected: no new errors beyond baseline.

- [ ] **Step 4: Commit**

```bash
git add src/lib/Dictation.svelte
git commit -m "feat(dictation): disable start while pending finalize is running"
```

---

## Task 7: Manual verification matrix

**Files:** none (verification only). This is the only validation that matters for this feature — there is no frontend test harness, and the behavior is end-to-end UX.

- [ ] **Step 1: Launch the app**

Run: `cargo tauri dev`

- [ ] **Step 2: Banner shows on transcribe**

Have at least one pending recording (record one if needed). Click **Transcribe** on it. Expected:
- A non-modal banner appears at the top of the app with a small ring, percent, label, **Details**, and **Cancel** buttons.
- The Record tab is still navigable (nav links don't show a "locked" cursor).
- No full-screen overlay covers the Record tab.

- [ ] **Step 3: Record during finalize**

With the banner still showing the finalize, click **Start Recording**. Expected: recording starts normally (timer counts). Click **Stop Recording**. Expected: a new pending appears in the list; its **Transcribe** button is **disabled** with a tooltip ("Aguarde a transcrição atual" / "Wait for the current transcription").

- [ ] **Step 4: Import during finalize**

With the banner still showing, click **Import audio** and pick any supported file. Expected: the import completes; a new pending appears; its Transcribe button is also disabled.

- [ ] **Step 5: Navigate while finalize runs**

Click **History** tab. Expected: the banner stays at the top; the history list is browsable. Click **Dictation** tab. Expected: the banner stays; the **Start Dictation** button is **disabled** with a tooltip.

- [ ] **Step 6: Details toggle**

Click **Details** on the banner. Expected: a scrollable text panel expands below the banner showing the live transcript. Click again to hide.

- [ ] **Step 7: Cancel from the banner**

Click **Cancel** on the banner. Expected: the confirmation modal appears. Click "Yes, cancel" / "Sim, cancelar". Expected: the banner disappears; the Transcribe and Start Dictation buttons re-enable; the pending recording survives (existing behavior).

- [ ] **Step 8: Complete normally**

Start a transcription and let it finish. Expected: the banner shows 100% briefly, then disappears. The app auto-navigates to the new transcription view (existing onTranscribed behavior).

- [ ] **Step 9: Dictation finalize unaffected**

Start a brief dictation (Start Dictation → say a sentence → Stop Dictation). Expected: the existing FinalizingProgress modal shows (since dictation finalize wasn't migrated by design). After it completes, the new transcription opens normally.

---

## Self-Review Notes

- **Spec coverage:** finalizeProgress store (Task 2 covers "global Svelte store"); FinalizeBanner non-modal banner (Task 3); mounted at layout (Task 4); Recorder lifts state + appBusy removed + Transcribe disabled (Task 5); Dictation Start disabled (Task 6); i18n strings (Task 1); dictation finalize path untouched (verified Task 7 Step 9); no backend changes (verified across tasks — Rust never edited). All spec items covered.
- **Type / name consistency:** `finalizeProgress.phase` ('idle'/'finalizing'/'cancelling'), `beginFinalize({ id, pendingId, jobLabel })`, `requestCancel()`, `startedFromPendingId`, `transcription://*` event names, `transcribeBusy`/`dictationBusy`/`finalizeBannerLabel`/`showDetails`/`hideDetails` i18n keys — all referenced consistently across tasks.
- **Intentional non-coverage:** automated tests (project has no frontend test infra; matches the import feature's pattern). Banner-during-dictation-finalize is intentionally NOT migrated (dictation cannot coexist with a new recording on the mic anyway). `appBusy` stays in use by `Dictation.svelte`'s finalize path; the only deletions of `appBusy.set` are in `Recorder.svelte` (Task 5 Step 3).
- **Risk:** if any task ordering is wrong (e.g., Task 5 runs before Task 2), the build breaks because the store isn't there yet. Execute tasks IN ORDER (1 → 2 → 3 → 4 → 5 → 6 → 7).
