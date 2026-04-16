<script>
    import { invoke } from "@tauri-apps/api/core";
    import { onMount, onDestroy } from "svelte";
    import { t, locale } from "./i18n.js";
    import { formatDate, formatDuration } from "./format.js";

    let { onTranscribed } = $props();

    let recording = $state(false);
    let processing = $state(false);
    let error = $state("");
    let elapsed = $state(0);
    let timer = null;

    let pendingRecordings = $state([]);
    let transcribingId = $state(null);

    onMount(loadPending);
    onDestroy(() => { if (timer) clearInterval(timer); });

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
            transcribingId = id;
            const now = new Date().toLocaleString("pt-BR");
            const result = await invoke("transcribe_recording", {
                pendingId: id,
                title: `${t("meetingTitle")} ${now}`,
                language: locale,
            });
            pendingRecordings = pendingRecordings.filter((p) => p.id !== id);
            onTranscribed?.(result);
        } catch (e) {
            error = e;
        } finally {
            transcribingId = null;
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
    {:else}
        <button class="btn-start" onclick={startRecording}>
            {t("startRecording")}
        </button>
    {/if}

    {#if error}
        <div class="error">{error}</div>
    {/if}

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
                        {#if transcribingId === pending.id}
                            <div class="pending-transcribing">
                                {t("transcribing")}
                            </div>
                        {:else}
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
                        {/if}
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

    .pending-transcribing {
        font-size: 0.8rem;
        color: var(--text-muted);
        white-space: nowrap;
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
