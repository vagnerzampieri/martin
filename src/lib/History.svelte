<script>
    import { invoke } from "@tauri-apps/api/core";
    import { onMount } from "svelte";
    import { t } from "./i18n.js";

    let { onSelect } = $props();

    let transcriptions = $state([]);
    let loading = $state(true);
    let error = $state("");
    let claudeAvailable = $state(false);
    let summarizingId = $state(null);

    onMount(async () => {
        try {
            transcriptions = await invoke("list_transcriptions");
        } catch (e) {
            error = `${t("loadError")}: ${e}`;
        }
        loading = false;

        try {
            claudeAvailable = await invoke("check_claude_cli");
        } catch {
            claudeAvailable = false;
        }
    });

    async function summarize(id) {
        summarizingId = id;
        error = "";
        try {
            const summary = await invoke("summarize_transcription", { id });
            transcriptions = transcriptions.map((item) =>
                item.id === id ? { ...item, summary } : item
            );
        } catch (e) {
            error = `${t("summarize")}: ${e}`;
        }
        summarizingId = null;
    }

    async function remove(id) {
        try {
            await invoke("delete_transcription", { id });
            transcriptions = transcriptions.filter((item) => item.id !== id);
        } catch (e) {
            error = `${t("deleteError")}: ${e}`;
        }
    }

    function formatDate(dateStr) {
        return new Date(dateStr).toLocaleString("pt-BR");
    }

    function formatDuration(secs) {
        const m = Math.floor(secs / 60);
        const s = Math.round(secs % 60);
        return `${m}min ${s}s`;
    }
</script>

<div class="history">
    <h2>{t("history")}</h2>

    {#if error}
        <div class="error">{error}</div>
    {/if}

    {#if loading}
        <p class="muted">{t("loading")}</p>
    {:else if transcriptions.length === 0}
        <p class="muted">{t("noTranscriptions")}</p>
    {:else}
        <ul>
            {#each transcriptions as item}
                <li>
                    <button class="item" onclick={() => onSelect?.(item)}>
                        <span class="title">
                            {item.title}
                            {#if item.summary}
                                <span class="summary-badge" title={t("summary")}>S</span>
                            {/if}
                        </span>
                        <span class="meta">
                            {formatDate(item.created_at)} · {formatDuration(item.duration_secs)}
                        </span>
                    </button>
                    {#if !item.summary}
                        <button
                            class="summarize"
                            disabled={!claudeAvailable || summarizingId === item.id}
                            title={claudeAvailable ? t("summarize") : t("claudeNotAvailable")}
                            onclick={(e) => { e.stopPropagation(); summarize(item.id); }}
                        >
                            {summarizingId === item.id ? t("summarizing") : t("summarize")}
                        </button>
                    {/if}
                    <button class="delete" onclick={(e) => { e.stopPropagation(); remove(item.id); }}>
                        ×
                    </button>
                </li>
            {/each}
        </ul>
    {/if}
</div>

<style>
    .history {
        text-align: left;
        padding: 20px;
    }

    h2 {
        margin-bottom: 16px;
    }

    .muted {
        color: var(--text-muted);
    }

    ul {
        list-style: none;
    }

    li {
        display: flex;
        align-items: center;
        border: 1px solid var(--border);
        border-radius: var(--radius);
        margin-bottom: 8px;
    }

    .item {
        flex: 1;
        background: var(--surface);
        color: var(--text);
        text-align: left;
        padding: 12px 16px;
        border-radius: var(--radius) 0 0 var(--radius);
        display: flex;
        flex-direction: column;
        gap: 4px;
    }

    .item:hover {
        background: var(--primary);
    }

    .title {
        font-weight: 600;
    }

    .meta {
        font-size: 0.85rem;
        color: var(--text-muted);
    }

    .summary-badge {
        display: inline-block;
        background: var(--primary);
        color: white;
        font-size: 0.65rem;
        font-weight: 700;
        width: 18px;
        height: 18px;
        line-height: 18px;
        text-align: center;
        border-radius: 50%;
        margin-left: 6px;
        vertical-align: middle;
    }

    .summarize {
        background: transparent;
        color: var(--success);
        padding: 8px 12px;
        font-size: 0.8rem;
        white-space: nowrap;
    }

    .summarize:hover:not(:disabled) {
        background: rgba(74, 222, 128, 0.15);
    }

    .summarize:disabled {
        opacity: 0.4;
        cursor: not-allowed;
    }

    .delete {
        background: transparent;
        color: var(--accent);
        padding: 12px 16px;
        font-size: 1.2rem;
        border-radius: 0 var(--radius) var(--radius) 0;
    }

    .delete:hover {
        background: rgba(233, 69, 96, 0.2);
    }

    .error {
        color: var(--accent);
        background: rgba(233, 69, 96, 0.1);
        padding: 12px 16px;
        border-radius: var(--radius);
        margin-bottom: 16px;
    }
</style>
