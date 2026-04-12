<script>
    import { t } from "./i18n.js";

    let { transcription, onBack } = $props();

    let copied = $state(false);

    async function copyToClipboard() {
        try {
            await navigator.clipboard.writeText(transcription.text);
            copied = true;
            setTimeout(() => { copied = false; }, 2000);
        } catch (e) {
            console.error("Failed to copy to clipboard:", e);
        }
    }
</script>

<div class="view">
    <div class="header">
        <button class="btn-back" onclick={onBack}>← {t("back")}</button>
        <button class="btn-copy" onclick={copyToClipboard}>
            {copied ? t("copied") : t("copyText")}
        </button>
    </div>

    <h2>{transcription.title}</h2>
    <p class="meta">{transcription.created_at} · {transcription.language}</p>

    <pre class="transcript">{transcription.text}</pre>

    {#if transcription.summary}
        <h3 class="summary-heading">{t("summary")}</h3>
        <pre class="transcript summary">{transcription.summary}</pre>
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
        margin-bottom: 16px;
    }

    .btn-back {
        background: var(--primary);
        color: white;
        padding: 8px 16px;
    }

    .btn-copy {
        background: var(--surface);
        color: var(--text);
        border: 1px solid var(--border);
        padding: 8px 16px;
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

    .summary-heading {
        margin-top: 24px;
        margin-bottom: 8px;
        color: var(--success);
    }

    .summary {
        border-color: var(--success);
    }
</style>
