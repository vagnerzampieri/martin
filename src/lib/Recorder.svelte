<script>
    import { invoke } from "@tauri-apps/api/core";
    import { listen } from "@tauri-apps/api/event";
    import { onMount, onDestroy } from "svelte";
    import { t, locale } from "./i18n.js";
    import { formatDate, formatDuration } from "./format.js";
    import { appBusy } from "./appBusy.js";
    import FinalizingProgress from "./FinalizingProgress.svelte";

    let { onTranscribed } = $props();

    let recording = $state(false);
    let processing = $state(false);
    let error = $state("");
    let elapsed = $state(0);
    let timer = null;

    let pendingRecordings = $state([]);

    let liveText = $state("");
    let percent = $state(0);
    /** @type {"idle" | "finalizing" | "cancelling"} */
    let phase = $state("idle");
    let pendingDurationLabel = $state("");
    /** @type {number | null} */
    let startedFromPendingId = null;
    /** @type {Array<() => void>} */
    let unlisteners = [];

    function isFinalizing() {
        return phase === "finalizing" || phase === "cancelling";
    }

    onMount(async () => {
        await loadPending();

        unlisteners.push(
            await listen("transcription://text", (event) => {
                if (!isFinalizing()) return;
                const incoming = event.payload.text ?? "";
                if (incoming.length > liveText.length) {
                    liveText = incoming;
                }
            }),
            await listen("transcription://progress", (event) => {
                if (!isFinalizing()) return;
                percent = event.payload.percent;
            }),
            await listen("transcription://complete", (event) => {
                if (!isFinalizing()) return;
                const transcription = event.payload.transcription;
                if (startedFromPendingId !== null) {
                    pendingRecordings = pendingRecordings.filter(
                        (p) => p.id !== startedFromPendingId,
                    );
                }
                percent = 100;
                setTimeout(() => {
                    phase = "idle";
                    appBusy.set(false);
                    liveText = "";
                    percent = 0;
                    pendingDurationLabel = "";
                    startedFromPendingId = null;
                }, 250);
                onTranscribed?.(transcription);
            }),
            await listen("transcription://cancelled", (_event) => {
                if (!isFinalizing()) return;
                phase = "idle";
                appBusy.set(false);
                liveText = "";
                percent = 0;
                pendingDurationLabel = "";
                startedFromPendingId = null;
            }),
            await listen("transcription://error", (event) => {
                if (!isFinalizing()) return;
                error = event.payload.error;
                phase = "idle";
                appBusy.set(false);
                pendingDurationLabel = "";
                startedFromPendingId = null;
            }),
        );
    });

    onDestroy(() => {
        if (timer) clearInterval(timer);
        for (const u of unlisteners) u();
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
            recording = true;
            elapsed = 0;
            timer = setInterval(() => { elapsed += 1; }, 1000);
        } catch (e) {
            error = e;
        }
    }

    async function stopRecording() {
        try {
            clearInterval(timer);
            timer = null;
            recording = false;
            processing = true;
            const pending = await invoke("stop_recording");
            pendingRecordings = [pending, ...pendingRecordings];
        } catch (e) {
            error = e;
        } finally {
            processing = false;
        }
    }

    async function transcribePending(id) {
        try {
            error = "";
            const pending = pendingRecordings.find((p) => p.id === id);
            startedFromPendingId = id;
            if (pending && typeof pending.duration_secs === "number") {
                pendingDurationLabel = `${t("recordedDuration")} ${formatDuration(pending.duration_secs)}`;
            }
            phase = "finalizing";
            appBusy.set(true);
            liveText = "";
            percent = 0;
            const now = new Date().toLocaleString(
                locale === "pt" ? "pt-BR" : "en-US",
            );
            await invoke("transcribe_pending_recording", {
                pendingId: id,
                title: `${t("meetingTitle")} ${now}`,
                language: locale,
            });
        } catch (e) {
            error = e;
            phase = "idle";
            appBusy.set(false);
            startedFromPendingId = null;
            pendingDurationLabel = "";
        }
    }

    async function requestCancel() {
        try {
            phase = "cancelling";
            await invoke("cancel_job");
        } catch (e) {
            error = e;
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
    {#if recording}
        <div class="status recording">
            <span class="dot"></span>
            {t("recording")} {formatTime(elapsed)}
        </div>
        <button class="btn-stop" onclick={stopRecording}>
            {t("stopRecording")}
        </button>
    {:else if processing}
        <div class="status processing">
            {t("processingAudio")}
        </div>
    {:else if phase === "idle"}
        <button class="btn-start" onclick={startRecording}>
            {t("startRecording")}
        </button>
    {/if}

    {#if error}
        <div class="error">{error}</div>
    {/if}

    {#if phase === "finalizing" || phase === "cancelling"}
        <FinalizingProgress
            {percent}
            {liveText}
            cancelling={phase === "cancelling"}
            jobLabel={pendingDurationLabel}
            onCancel={requestCancel}
        />
    {:else if !recording && !processing && pendingRecordings.length > 0}
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
