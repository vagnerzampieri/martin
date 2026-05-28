<script>
    import { invoke } from "@tauri-apps/api/core";
    import { listen } from "@tauri-apps/api/event";
    import { onMount, onDestroy } from "svelte";
    import { t, locale } from "./i18n.js";
    import { appBusy } from "./appBusy.js";
    import { finalizeProgress } from "./finalizeProgress.js";
    import FinalizingProgress from "./FinalizingProgress.svelte";
    import VuMeter from "./VuMeter.svelte";

    let { onTranscribed } = $props();

    /** @type {"idle" | "recording" | "finalizing" | "cancelling"} */
    let phase = $state("idle");
    let error = $state("");
    let stableText = $state("");
    let provisionalText = $state("");
    let liveText = $state(""); // legacy single-string view used during finalize
    let elapsed = $state(0);
    let percent = $state(0);
    let recordedDurationLabel = $state("");
    let peak = $state(0);
    /** @type {"listening" | "processing" | "paused"} */
    let dictationState = $state("listening");
    let partialId = $state(null);
    /** @type {ReturnType<typeof setInterval> | null} */
    /** @type {number | null} */
    let timer = null;

    /** @type {Array<() => void>} */
    let unlisteners = [];

    function isFinalizing() {
        return phase === "finalizing" || phase === "cancelling";
    }

    function stateClass(/** @type {"listening" | "processing" | "paused"} */ s) {
        if (s === "processing") return "state-processing";
        if (s === "paused") return "state-paused";
        return "state-listening";
    }

    function stateLabel(/** @type {"listening" | "processing" | "paused"} */ s) {
        if (s === "processing") return t("stateProcessing");
        if (s === "paused") return t("statePaused");
        return t("stateListening");
    }

    onMount(async () => {
        unlisteners.push(
            await listen("dictation://segment", (event) => {
                if (phase !== "recording") return;
                stableText = event.payload.stableText ?? "";
                provisionalText = event.payload.provisionalText ?? "";
                liveText = event.payload.fullText ?? "";
            }),
            await listen("dictation://level", (event) => {
                if (phase !== "recording") return;
                peak = event.payload.peak ?? 0;
            }),
            await listen("dictation://state", (event) => {
                if (phase !== "recording") return;
                dictationState = event.payload.state ?? "listening";
            }),
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
                percent = 100;
                setTimeout(() => {
                    phase = "idle";
                    appBusy.set(false);
                    stableText = "";
                    provisionalText = "";
                    liveText = "";
                    percent = 0;
                    peak = 0;
                    dictationState = "listening";
                    recordedDurationLabel = "";
                }, 250);
                onTranscribed?.(event.payload.transcription);
            }),
            await listen("transcription://cancelled", (_event) => {
                if (!isFinalizing()) return;
                phase = "idle";
                appBusy.set(false);
                stableText = "";
                provisionalText = "";
                liveText = "";
                percent = 0;
                peak = 0;
                dictationState = "listening";
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
            partialId = await invoke("start_dictation", { language: locale });
            phase = "recording";
            elapsed = 0;
            timer = setInterval(() => { elapsed += 1; }, 1000);
        } catch (e) {
            error = String(e);
        }
    }

    async function stopDictation() {
        try {
            if (timer) clearInterval(timer);
            timer = null;
            const now = new Date().toLocaleString(
                locale === "pt" ? "pt-BR" : "en-US",
            );
            recordedDurationLabel = `${t("recordedDuration")} ${formatTime(elapsed)}`;
            phase = "finalizing";
            appBusy.set(true);
            await invoke("stop_dictation", {
                partialId: partialId ?? 0,
                title: `${t("dictation")} ${now}`,
                language: locale,
                durationSecs: elapsed,
            });
        } catch (e) {
            error = String(e);
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
            error = String(e);
        }
    }

    /** @param {number} secs */
    function formatTime(secs) {
        const m = Math.floor(secs / 60).toString().padStart(2, "0");
        const s = (secs % 60).toString().padStart(2, "0");
        return `${m}:${s}`;
    }
</script>

<div class="dictation">
    {#if phase === "recording"}
        <div class="status {stateClass(dictationState)}">
            <span class="dot"></span>
            {stateLabel(dictationState)} · {formatTime(elapsed)}
        </div>
        <VuMeter {peak} />
        <button class="btn-stop" onclick={stopDictation}>
            {t("stopDictation")}
        </button>
        {#if stableText || provisionalText}
            <div class="live-text">
                <pre><span class="stable">{stableText}</span>{#if stableText && provisionalText}{" "}{/if}<span class="provisional">{provisionalText}</span></pre>
                <small class="hint">{t("provisionalHint")}</small>
            </div>
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
        <button
            class="btn-start"
            onclick={startDictation}
            disabled={$finalizeProgress.phase !== "idle"}
            aria-disabled={$finalizeProgress.phase !== "idle"}
            title={$finalizeProgress.phase !== "idle" ? t("dictationBusy") : ""}
        >
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

    .status .dot {
        width: 12px;
        height: 12px;
        border-radius: 50%;
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

    .state-listening .dot {
        background: var(--info);
        animation: pulse 1.4s infinite;
    }
    .state-processing .dot {
        background: var(--accent);
        animation: pulse 0.7s infinite;
    }
    .state-paused .dot {
        background: var(--border);
        animation: none;
        opacity: 0.6;
    }

    .live-text pre .stable {
        color: var(--text);
    }
    .live-text pre .provisional {
        color: var(--muted, #888);
        font-style: italic;
    }
    .live-text .hint {
        display: block;
        margin-top: 8px;
        font-size: 0.8rem;
        color: var(--muted, #888);
    }
</style>
