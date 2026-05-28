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
/** @type {{ id: number|null, percent: number, liveText: string, phase: string, jobLabel: string, startedFromPendingId: number|null }} */
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
        const detail = event.payload?.error ?? "Unknown error";
        finalizeProgress.set({ ...initial });
        window.dispatchEvent(
            new CustomEvent("finalize:error", { detail }),
        );
    });
}

/**
 * Flip the store into the finalizing phase. Call this from the action that
 * invokes `transcribe_pending_recording`, BEFORE awaiting the invoke, so the
 * banner appears immediately.
 * @param {{ id?: number|null, pendingId?: number|null, jobLabel?: string }} opts
 */
export function beginFinalize({ id, pendingId, jobLabel } = {}) {
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
