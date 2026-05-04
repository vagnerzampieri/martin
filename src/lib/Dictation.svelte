<script>
    import { invoke } from "@tauri-apps/api/core";
    import { listen } from "@tauri-apps/api/event";
    import { onMount, onDestroy } from "svelte";
    import { t, locale } from "./i18n.js";
    import { appBusy } from "./appBusy.js";
    import FinalizingProgress from "./FinalizingProgress.svelte";

    let { onTranscribed } = $props();

    /** @type {"idle" | "recording" | "finalizing" | "cancelling"} */
    let phase = $state("idle");
    let error = $state("");
    let liveText = $state("");
    let elapsed = $state(0);
    let percent = $state(0);
    let recordedDurationLabel = $state("");
    /** @type {ReturnType<typeof setInterval> | null} */
    let timer = null;

    /** @type {Array<() => void>} */
    let unlisteners = [];

    function isFinalizing() {
        return phase === "finalizing" || phase === "cancelling";
    }

    onMount(async () => {
        unlisteners.push(
            await listen("dictation://segment", (event) => {
                if (phase !== "recording") return;
                liveText = event.payload.fullText;
            }),
            await listen("transcription://text", (event) => {
                if (!isFinalizing()) return;
                // Only overwrite if the worker's accumulated text has
                // surpassed the live-loop text we already have. Otherwise
                // we'd flash a regressing liveText (worker starts empty
                // and grows back, which felt like the text disappeared).
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
                percent = 100;
                setTimeout(() => {
                    phase = "idle";
                    appBusy.set(false);
                    liveText = "";
                    percent = 0;
                    recordedDurationLabel = "";
                }, 250);
                onTranscribed?.(event.payload.transcription);
            }),
            await listen("transcription://cancelled", (_event) => {
                if (!isFinalizing()) return;
                phase = "idle";
                appBusy.set(false);
                liveText = "";
                percent = 0;
                recordedDurationLabel = "";
            }),
            await listen("transcription://error", (event) => {
                if (!isFinalizing()) return;
                error = event.payload.error;
                phase = "idle";
                appBusy.set(false);
                recordedDurationLabel = "";
            }),
        );
    });

    onDestroy(() => {
        if (timer) clearInterval(timer);
        for (const u of unlisteners) u();
    });

    async function startDictation() {
        try {
            error = "";
            liveText = "";
            percent = 0;
            await invoke("start_dictation", { language: locale });
            phase = "recording";
            elapsed = 0;
            timer = setInterval(() => { elapsed += 1; }, 1000);
        } catch (e) {
            error = e;
        }
    }

    async function stopDictation() {
        try {
            clearInterval(timer);
            timer = null;
            const now = new Date().toLocaleString(
                locale === "pt" ? "pt-BR" : "en-US",
            );
            recordedDurationLabel = `${t("recordedDuration")} ${formatTime(elapsed)}`;
            phase = "finalizing";
            appBusy.set(true);
            await invoke("stop_dictation", {
                title: `${t("dictation")} ${now}`,
                language: locale,
                durationSecs: elapsed,
            });
        } catch (e) {
            error = e;
            phase = "idle";
            appBusy.set(false);
            recordedDurationLabel = "";
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

    function formatTime(secs) {
        const m = Math.floor(secs / 60).toString().padStart(2, "0");
        const s = (secs % 60).toString().padStart(2, "0");
        return `${m}:${s}`;
    }
</script>

<div class="dictation">
    {#if phase === "recording"}
        <div class="status dictating">
            <span class="dot"></span>
            {t("dictating")} {formatTime(elapsed)}
        </div>
        <button class="btn-stop" onclick={stopDictation}>
            {t("stopDictation")}
        </button>
        {#if liveText}
            <div class="live-text"><pre>{liveText}</pre></div>
        {/if}
    {:else if phase === "finalizing" || phase === "cancelling"}
        <FinalizingProgress
            {percent}
            {liveText}
            cancelling={phase === "cancelling"}
            jobLabel={recordedDurationLabel}
            onCancel={requestCancel}
        />
    {:else}
        <button class="btn-start" onclick={startDictation}>
            {t("startDictation")}
        </button>
    {/if}

    {#if error}
        <div class="error">{error}</div>
    {/if}
</div>

<style>
    .dictation {
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

    .dictating .dot {
        width: 12px;
        height: 12px;
        background: var(--info);
        border-radius: 50%;
        animation: pulse 1s infinite;
    }

    @keyframes pulse {
        0%, 100% { opacity: 1; }
        50% { opacity: 0.3; }
    }

    .btn-start {
        background: var(--info);
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

    .error {
        color: var(--accent);
        background: rgba(233, 69, 96, 0.1);
        padding: 12px 16px;
        border-radius: var(--radius);
        max-width: 400px;
        text-align: center;
    }

    .live-text {
        width: 100%;
        max-width: 600px;
        margin-top: 8px;
    }

    .live-text pre {
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: var(--radius);
        padding: 20px;
        white-space: pre-wrap;
        word-wrap: break-word;
        font-family: inherit;
        font-size: 0.95rem;
        line-height: 1.6;
        max-height: 50vh;
        overflow-y: auto;
    }
</style>
