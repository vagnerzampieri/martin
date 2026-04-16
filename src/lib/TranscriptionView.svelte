<script>
    import { invoke } from "@tauri-apps/api/core";
    import { onMount } from "svelte";
    import { t } from "./i18n.js";

    let { transcription, onBack } = $props();

    let copied = $state(false);
    let copyFailed = $state(false);
    let summaryCopied = $state(false);
    let summaryCopyFailed = $state(false);
    let summarizing = $state(false);
    let localSummary = $state("");
    let summaryText = $derived(localSummary || transcription.summary || "");
    let claudeAvailable = $state(false);
    let error = $state("");

    onMount(async () => {
        try {
            claudeAvailable = await invoke("check_claude_cli");
        } catch (e) {
            console.error("Failed to check Claude CLI availability:", e);
            claudeAvailable = false;
        }
    });

    async function copyToClipboard() {
        try {
            await navigator.clipboard.writeText(transcription.text);
            copied = true;
            copyFailed = false;
            setTimeout(() => { copied = false; }, 2000);
        } catch (e) {
            copyFailed = true;
            setTimeout(() => { copyFailed = false; }, 2000);
        }
    }

    async function copySummary() {
        try {
            await navigator.clipboard.writeText(summaryText);
            summaryCopied = true;
            summaryCopyFailed = false;
            setTimeout(() => { summaryCopied = false; }, 2000);
        } catch (e) {
            summaryCopyFailed = true;
            setTimeout(() => { summaryCopyFailed = false; }, 2000);
        }
    }

    async function summarize() {
        try {
            error = "";
            summarizing = true;
            const summary = await invoke("summarize_transcription", { id: transcription.id });
            localSummary = summary;
        } catch (e) {
            error = `${t("summarize")}: ${e}`;
        } finally {
            summarizing = false;
        }
    }
</script>

<div class="view">
    <div class="header">
        <button class="btn-back" onclick={onBack}>← {t("back")}</button>
        {#if !summaryText && claudeAvailable}
            <button class="btn-action" onclick={summarize} disabled={summarizing}>
                {summarizing ? t("summarizing") : t("summarize")}
            </button>
        {/if}
    </div>

    {#if error}
        <div class="error">{error}</div>
    {/if}

    <h2>{transcription.title}</h2>
    <p class="meta">{transcription.created_at} · {transcription.language}</p>

    <div class="section-header">
        <h3 class="transcription-heading">{t("transcription")}</h3>
        <button class="btn-copy-section" class:copy-success={copied} class:copy-failed={copyFailed} onclick={copyToClipboard}>
            {copied ? t("copied") : copyFailed ? t("copyFailed") : t("copyText")}
        </button>
    </div>
    <pre class="transcript">{transcription.text}</pre>

    {#if summaryText}
        <div class="section-header summary-section">
            <h3 class="summary-heading">{t("summary")}</h3>
            <button class="btn-copy-section" class:copy-success={summaryCopied} class:copy-failed={summaryCopyFailed} onclick={copySummary}>
                {summaryCopied ? t("copied") : summaryCopyFailed ? t("copyFailed") : t("copyText")}
            </button>
        </div>
        <pre class="transcript summary">{summaryText}</pre>
    {/if}
</div>

<style>
    .view {
        text-align: left;
        padding: 20px;
    }

    .header {
        display: flex;
        justify-content: space-between;
        align-items: center;
        margin-bottom: 16px;
    }

    .btn-back {
        background: var(--primary);
        color: white;
        padding: 8px 16px;
    }

    .btn-action {
        background: var(--surface);
        color: var(--text);
        border: 1px solid var(--border);
        padding: 8px 16px;
    }

    .btn-action:disabled {
        opacity: 0.5;
        cursor: not-allowed;
    }

    h2 {
        margin-bottom: 4px;
    }

    .meta {
        color: var(--text-muted);
        font-size: 0.85rem;
        margin-bottom: 16px;
    }

    .transcript {
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: var(--radius);
        padding: 20px;
        white-space: pre-wrap;
        word-wrap: break-word;
        font-family: inherit;
        font-size: 0.95rem;
        line-height: 1.6;
        max-height: 60vh;
        overflow-y: auto;
    }

    .section-header {
        display: flex;
        align-items: center;
        justify-content: space-between;
        margin-bottom: 8px;
    }

    .transcription-heading {
        color: var(--info);
        margin: 0;
    }

    .summary-section {
        margin-top: 24px;
    }

    .summary-heading {
        color: var(--success);
        margin: 0;
    }

    .btn-copy-section {
        background: var(--surface);
        color: var(--text);
        border: 1px solid var(--border);
        padding: 4px 12px;
        font-size: 0.8rem;
    }

    .summary {
        border-color: var(--success);
    }

    .copy-success {
        border-color: var(--success);
        color: var(--success);
    }

    .copy-failed {
        border-color: var(--accent);
        color: var(--accent);
    }

    .error {
        color: var(--accent);
        background: rgba(233, 69, 96, 0.1);
        padding: 12px 16px;
        border-radius: var(--radius);
        margin-bottom: 16px;
    }
</style>
