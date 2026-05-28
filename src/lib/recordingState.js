import { writable } from "svelte/store";

// Global recording state so the UI survives navigation: when the user (or an
// auto-navigation triggered by `finalize:complete`) leaves the Record tab while
// a capture is in progress, the backend keeps capturing and the next mount of
// <Recorder> needs to pick up the correct state.

const initial = {
    recording: false,
    elapsed: 0,
    /** @type {number | null} */
    startedAt: null,
};

export const recordingState = writable(initial);

/** @type {ReturnType<typeof setInterval> | null} */
let timer = null;

export function beginRecord() {
    const now = Date.now();
    recordingState.set({ recording: true, elapsed: 0, startedAt: now });
    timer = setInterval(() => {
        recordingState.update((s) => ({
            ...s,
            elapsed: Math.floor((Date.now() - now) / 1000),
        }));
    }, 1000);
}

export function endRecord() {
    if (timer) {
        clearInterval(timer);
        timer = null;
    }
    recordingState.set({ ...initial });
}
