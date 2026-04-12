<script>
    import { invoke } from "@tauri-apps/api/core";
    import { onMount } from "svelte";

    let { onSelect } = $props();

    let transcriptions = $state([]);
    let loading = $state(true);
    let error = $state("");

    onMount(async () => {
        try {
            transcriptions = await invoke("list_transcriptions");
        } catch (e) {
            error = `Falha ao carregar transcricoes: ${e}`;
        }
        loading = false;
    });

    async function remove(id) {
        try {
            await invoke("delete_transcription", { id });
            transcriptions = transcriptions.filter((t) => t.id !== id);
        } catch (e) {
            error = `Falha ao excluir transcricao: ${e}`;
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
    <h2>Historico</h2>

    {#if error}
        <div class="error">{error}</div>
    {/if}

    {#if loading}
        <p class="muted">Carregando...</p>
    {:else if transcriptions.length === 0}
        <p class="muted">Nenhuma transcricao ainda.</p>
    {:else}
        <ul>
            {#each transcriptions as t}
                <li>
                    <button class="item" onclick={() => onSelect?.(t)}>
                        <span class="title">{t.title}</span>
                        <span class="meta">
                            {formatDate(t.created_at)} · {formatDuration(t.duration_secs)}
                        </span>
                    </button>
                    <button class="delete" onclick={(e) => { e.stopPropagation(); remove(t.id); }}>
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
