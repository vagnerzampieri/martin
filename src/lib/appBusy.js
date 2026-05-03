import { writable } from "svelte/store";

/**
 * Global flag — when true, navigation is locked because a transcription
 * job is finalizing or cancelling. Components running long jobs set
 * this to true on entry and false on exit.
 */
export const appBusy = writable(false);
