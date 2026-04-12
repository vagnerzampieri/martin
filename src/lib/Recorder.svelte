<script>
    import { invoke } from "@tauri-apps/api/core";
    import { t } from "./i18n.js";

    let { onTranscribed } = $props();

    let recording = $state(false);
    let transcribing = $state(false);
    let error = $state("");
    let elapsed = $state(0);
    let timer = null;

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
            await invoke("stop_recording");
            recording = false;
        } catch (e) {
            error = e;
        }
    }

    async function transcribe() {
        try {
            error = "";
            transcribing = true;
            const now = new Date().toLocaleString("pt-BR");
            const result = await invoke("transcribe_recording", {
                title: `${t("meetingTitle")} ${now}`,
                language: "pt",
            });
            onTranscribed?.(result);
        } catch (e) {
            error = e;
        } finally {
            transcribing = false;
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
    {:else if transcribing}
        <div class="status processing">
            {t("transcribing")}
        </div>
    {:else}
        <button class="btn-start" onclick={startRecording}>
            {t("startRecording")}
        </button>
        <button class="btn-transcribe" onclick={transcribe}>
            {t("transcribe")}
        </button>
    {/if}

    {#if error}
        <div class="error">{error}</div>
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

    .btn-transcribe {
        background: var(--success);
        color: #1a1a2e;
        padding: 12px 32px;
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
</style>
