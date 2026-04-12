<script>
    let { transcription, onBack } = $props();

    function copyToClipboard() {
        navigator.clipboard.writeText(transcription.text);
    }
</script>

<div class="view">
    <div class="header">
        <button class="btn-back" onclick={onBack}>← Voltar</button>
        <button class="btn-copy" onclick={copyToClipboard}>Copiar texto</button>
    </div>

    <h2>{transcription.title}</h2>
    <p class="meta">{transcription.created_at} · {transcription.language}</p>

    <pre class="transcript">{transcription.text}</pre>
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
</style>
