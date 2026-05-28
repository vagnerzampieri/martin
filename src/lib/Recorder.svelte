<script>
    import { invoke } from "@tauri-apps/api/core";
    import { open } from "@tauri-apps/plugin-dialog";
    import { onMount, onDestroy } from "svelte";
    import { t, locale } from "./i18n.js";
    import { formatDate, formatDuration } from "./format.js";
    import { finalizeProgress, beginFinalize } from "./finalizeProgress.js";
    import { recordingState, beginRecord, endRecord } from "./recordingState.js";

    let processing = $state(false);
    let importing = $state(false);
    let error = $state("");

    let pendingRecordings = $state([]);

    function handleFinalizeComplete() {
        const removeId = $finalizeProgress.startedFromPendingId;
        if (removeId !== null) {
            pendingRecordings = pendingRecordings.filter((p) => p.id !== removeId);
        }
    }

    /** @param {Event} e */
    function handleFinalizeError(e) {
        const detail = /** @type {CustomEvent<string>} */ (e).detail;
        error = detail ?? "Transcription error";
    }

    onMount(async () => {
        await loadPending();
        window.addEventListener("finalize:complete", handleFinalizeComplete);
        window.addEventListener("finalize:error", handleFinalizeError);
    });

    onDestroy(() => {
        window.removeEventListener("finalize:complete", handleFinalizeComplete);
        window.removeEventListener("finalize:error", handleFinalizeError);
    });

    async function loadPending() {
        try {
            pendingRecordings = await invoke("list_pending_recordings");
        } catch (e) {
            error = `${t("loadError")}: ${e}`;
        }
    }

    async function startRecording() {
        try {
            error = "";
            await invoke("start_recording");
            beginRecord();
        } catch (e) {
            error = e;
        }
    }

    async function stopRecording() {
        try {
            endRecord();
            processing = true;
            const pending = await invoke("stop_recording");
            pendingRecordings = [pending, ...pendingRecordings];
        } catch (e) {
            error = e;
        } finally {
            processing = false;
        }
    }

    async function importAudio() {
        try {
            error = "";
            console.log("[import] opening file dialog");
            const selected = await open({
                multiple: false,
                filters: [
                    {
                        name: t("audioFiles"),
                        extensions: ["mp3", "m4a", "wav", "ogg", "flac"],
                    },
                ],
            });
            if (typeof selected !== "string") {
                console.log("[import] dialog dismissed (no file selected):", selected);
                return; // user cancelled
            }
            console.log("[import] selected:", selected);
            importing = true;
            const pending = await invoke("import_audio_file", {
                path: selected,
            });
            console.log("[import] pending created:", pending);
            pendingRecordings = [pending, ...pendingRecordings];
        } catch (e) {
            console.error("[import] failed:", e);
            error = `${t("importError")}: ${e}`;
        } finally {
            importing = false;
        }
    }

    /** @param {number} id */
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

    async function deletePending(id) {
        try {
            await invoke("delete_pending_recording", { id });
            pendingRecordings = pendingRecordings.filter((p) => p.id !== id);
        } catch (e) {
            error = e;
        }
    }

    function formatTime(secs) {
        const m = Math.floor(secs / 60).toString().padStart(2, "0");
        const s = (secs % 60).toString().padStart(2, "0");
        return `${m}:${s}`;
    }

</script>

<div class="recorder">
    {#if $recordingState.recording}
        <div class="status recording">
            <span class="dot"></span>
            {t("recording")} {formatTime($recordingState.elapsed)}
        </div>
        <button class="btn-stop" onclick={stopRecording}>
            {t("stopRecording")}
        </button>
    {:else if processing}
        <div class="status processing">
            {t("processingAudio")}
        </div>
    {:else}
        {#if importing}
            <div class="status processing">{t("importing")}</div>
        {:else}
            <button class="btn-start" onclick={startRecording}>
                {t("startRecording")}
            </button>
            <button class="btn-import" onclick={importAudio}>
                {t("importAudio")}
            </button>
        {/if}
    {/if}

    {#if error}
        <div class="error">{error}</div>
    {/if}

    {#if !$recordingState.recording && !processing && pendingRecordings.length > 0}
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
</div>

<style>
    .recorder {
        display: flex;
        flex-direction: column;
        align-items: center;
        gap: 16px;
        padding: 32px;
    }

    .status {
        font-size: 1.2rem;
        display: flex;
        align-items: center;
        gap: 8px;
    }

    .recording .dot {
        width: 12px;
        height: 12px;
        background: var(--accent);
        border-radius: 50%;
        animation: pulse 1s infinite;
    }

    @keyframes pulse {
        0%, 100% { opacity: 1; }
        50% { opacity: 0.3; }
    }

    .btn-start {
        background: var(--accent);
        color: white;
        font-size: 1.3rem;
        padding: 16px 48px;
    }

    .btn-stop {
        background: var(--primary);
        color: white;
        font-size: 1.3rem;
        padding: 16px 48px;
    }

    .btn-import {
        background: transparent;
        color: var(--text-muted);
        border: 1px solid var(--text-muted);
        font-size: 1rem;
        padding: 10px 24px;
    }

    .processing {
        color: var(--text-muted);
    }

    .error {
        color: var(--accent);
        background: rgba(233, 69, 96, 0.1);
        padding: 12px 16px;
        border-radius: var(--radius);
        max-width: 400px;
        text-align: center;
    }

    .pending {
        width: 100%;
        max-width: 500px;
        margin-top: 16px;
    }

    .pending h3 {
        font-size: 0.95rem;
        color: var(--text-muted);
        margin-bottom: 8px;
    }

    .pending ul {
        list-style: none;
    }

    .pending li {
        display: flex;
        align-items: center;
        justify-content: space-between;
        border: 1px solid var(--border);
        border-radius: var(--radius);
        padding: 10px 12px;
        margin-bottom: 6px;
        background: var(--surface);
        gap: 12px;
    }

    .pending-info {
        display: flex;
        flex-direction: column;
        gap: 2px;
        min-width: 0;
    }

    .pending-date {
        font-size: 0.9rem;
        color: var(--text);
    }

    .pending-duration {
        font-size: 0.8rem;
        color: var(--text-muted);
    }

    .pending-actions {
        display: flex;
        gap: 6px;
        flex-shrink: 0;
    }

    .pending-actions .btn-transcribe {
        background: var(--success);
        color: #1a1a2e;
        font-size: 0.8rem;
        padding: 6px 14px;
        white-space: nowrap;
    }

    .pending-actions .btn-delete {
        background: transparent;
        color: var(--accent);
        padding: 6px 10px;
        font-size: 1.1rem;
    }

    .pending-actions .btn-delete:hover {
        background: rgba(233, 69, 96, 0.2);
    }
</style>
