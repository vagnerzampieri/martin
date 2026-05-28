<script>
    import { invoke } from "@tauri-apps/api/core";
    import { listen } from "@tauri-apps/api/event";
    import { onMount, onDestroy } from "svelte";
    import { t } from "./i18n.js";

    let { onComplete, onError } = $props();

    let percent = $state(0);
    let downloadedMb = $state(0);
    let totalMb = $state(0);
    let error = $state("");
    let downloading = $state(false);
    /** @type {(() => void) | null} */
    let unlistenProgress = null;
    /** @type {(() => void) | null} */
    let unlistenComplete = null;
    /** @type {(() => void) | null} */
    let unlistenError = null;

    onMount(async () => {
        unlistenProgress = await listen("model://download-progress", (event) => {
            percent = event.payload.percent;
            downloadedMb = event.payload.downloaded_mb;
            totalMb = event.payload.total_mb;
        });
        unlistenComplete = await listen("model://download-complete", () => {
            onComplete?.();
        });
        unlistenError = await listen("model://download-error", (event) => {
            error = event.payload.message;
        });
        startDownload();
    });

    onDestroy(() => {
        if (unlistenProgress) unlistenProgress();
        if (unlistenComplete) unlistenComplete();
        if (unlistenError) unlistenError();
    });

    async function startDownload() {
        try {
            error = "";
            downloading = true;
            await invoke("download_whisper_model");
        } catch (e) {
            error = String(e);
            downloading = false;
            onError?.(String(e));
        }
    }
</script>

<div class="overlay">
    <div class="card">
        <h2>{t("downloadingModel")}</h2>

        {#if error}
            <div class="error">{t("downloadError")}: {error}</div>
            <button class="btn-retry" onclick={startDownload}>
                {t("downloadRetry")}
            </button>
        {:else}
            <div class="progress-bar">
                <div class="progress-fill" style="width: {percent}%"></div>
            </div>
            <div class="progress-text">
                {t("downloadProgress")} {downloadedMb.toFixed(0)} {t("downloadOf")} {totalMb.toFixed(0)} MB ({percent}%)
            </div>
        {/if}
    </div>
</div>

<style>
    .overlay {
        position: fixed;
        inset: 0;
        background: rgba(0, 0, 0, 0.8);
        display: flex;
        align-items: center;
        justify-content: center;
        z-index: 100;
    }

    .card {
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: var(--radius);
        padding: 32px 40px;
        max-width: 420px;
        width: 90%;
        text-align: center;
    }

    h2 {
        font-size: 1.1rem;
        margin-bottom: 20px;
        color: var(--text);
    }

    .progress-bar {
        width: 100%;
        height: 8px;
        background: var(--border);
        border-radius: 4px;
        overflow: hidden;
        margin-bottom: 12px;
    }

    .progress-fill {
        height: 100%;
        background: var(--info);
        border-radius: 4px;
        transition: width 0.3s ease;
    }

    .progress-text {
        font-size: 0.85rem;
        color: var(--text-muted);
    }

    .error {
        color: var(--accent);
        background: rgba(233, 69, 96, 0.1);
        padding: 12px;
        border-radius: var(--radius);
        margin-bottom: 16px;
        font-size: 0.9rem;
    }

    .btn-retry {
        background: var(--info);
        color: white;
        padding: 10px 24px;
    }
</style>
