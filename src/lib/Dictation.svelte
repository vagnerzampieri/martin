<script>
    import { invoke } from "@tauri-apps/api/core";
    import { listen } from "@tauri-apps/api/event";
    import { onMount, onDestroy } from "svelte";
    import { t, locale } from "./i18n.js";

    let { onTranscribed } = $props();

    let dictating = $state(false);
    let error = $state("");
    let fullText = $state("");
    let elapsed = $state(0);
    let timer = null;
    let unlisten = null;

    onMount(async () => {
        unlisten = await listen("dictation://segment", (event) => {
            fullText = event.payload.fullText;
        });
    });

    onDestroy(() => {
        if (timer) clearInterval(timer);
        if (unlisten) unlisten();
    });

    async function startDictation() {
        try {
            error = "";
            fullText = "";
            await invoke("start_dictation", { language: locale });
            dictating = true;
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
            dictating = false;
            const now = new Date().toLocaleString("pt-BR");
            const result = await invoke("stop_dictation", {
                title: `${t("dictation")} ${now}`,
                fullText: fullText,
                language: locale,
                durationSecs: elapsed,
            });
            onTranscribed?.(result);
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
    {#if dictating}
        <div class="status dictating">
            <span class="dot"></span>
            {t("dictating")} {formatTime(elapsed)}
        </div>
        <button class="btn-stop" onclick={stopDictation}>
            {t("stopDictation")}
        </button>
    {:else}
        <button class="btn-start" onclick={startDictation}>
            {t("startDictation")}
        </button>
    {/if}

    {#if error}
        <div class="error">{error}</div>
    {/if}

    {#if fullText}
        <div class="live-text">
            <pre>{fullText}</pre>
        </div>
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
