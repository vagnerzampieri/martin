<script>
    import { invoke } from "@tauri-apps/api/core";
    import { onMount } from "svelte";
    import { t } from "./i18n.js";

    let { onClose } = $props();

    /** @type {string[]} */
    let terms = $state([]);
    let newTerm = $state("");
    let error = $state("");

    onMount(loadTerms);

    async function loadTerms() {
        try {
            terms = await invoke("list_glossary_terms");
            error = "";
        } catch {
            error = t("glossaryLoadError");
        }
    }

    async function addTerm() {
        const term = newTerm.trim();
        if (!term) return;
        try {
            await invoke("add_glossary_term", { term });
            newTerm = "";
            await loadTerms();
        } catch (e) {
            error = String(e);
        }
    }

    /** @param {string} term */
    async function removeTerm(term) {
        try {
            await invoke("remove_glossary_term", { term });
            await loadTerms();
        } catch (e) {
            error = String(e);
        }
    }

    /** @param {KeyboardEvent} e */
    function onInputKeydown(e) {
        if (e.key === "Enter") addTerm();
    }

    /** @param {KeyboardEvent} e */
    function onOverlayKeydown(e) {
        if (e.key === "Escape") onClose?.();
    }
</script>

<svelte:window onkeydown={onOverlayKeydown} />

<div
    class="overlay"
    role="presentation"
    onclick={(e) => e.target === e.currentTarget && onClose?.()}
>
    <div
        class="modal"
        role="dialog"
        aria-modal="true"
        aria-label={t("glossary")}
    >
        <header>
            <h2>{t("glossary")}</h2>
            <span class="count">{terms.length} {t("terms")}</span>
        </header>
        <p class="hint">{t("glossaryHint")}</p>

        <div class="add-row">
            <input
                type="text"
                bind:value={newTerm}
                placeholder={t("glossaryPlaceholder")}
                onkeydown={onInputKeydown}
            />
            <button onclick={addTerm} disabled={!newTerm.trim()}>
                {t("addTerm")}
            </button>
        </div>

        {#if error}
            <p class="error">{error}</p>
        {/if}

        {#if terms.length === 0}
            <p class="empty">{t("glossaryEmpty")}</p>
        {:else}
            <ul>
                {#each terms as term (term)}
                    <li>
                        <span>{term}</span>
                        <button
                            class="remove"
                            onclick={() => removeTerm(term)}
                            title={t("removeTerm")}
                            aria-label={`${t("removeTerm")} ${term}`}
                        >
                            ×
                        </button>
                    </li>
                {/each}
            </ul>
        {/if}

        <footer>
            <button class="close" onclick={() => onClose?.()}>
                {t("close")}
            </button>
        </footer>
    </div>
</div>

<style>
    .overlay {
        position: fixed;
        inset: 0;
        background: rgba(0, 0, 0, 0.5);
        display: flex;
        align-items: center;
        justify-content: center;
        z-index: 100;
    }

    .modal {
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: 8px;
        padding: 20px;
        width: min(440px, 90vw);
        max-height: 80vh;
        display: flex;
        flex-direction: column;
    }

    header {
        display: flex;
        align-items: baseline;
        justify-content: space-between;
    }

    h2 {
        font-size: 1.2rem;
        margin: 0;
    }

    .count {
        color: var(--text-muted);
        font-size: 0.85rem;
    }

    .hint {
        color: var(--text-muted);
        font-size: 0.9rem;
        margin: 8px 0 16px;
    }

    .add-row {
        display: flex;
        gap: 8px;
    }

    .add-row input {
        flex: 1;
    }

    .error {
        color: var(--danger, #c0392b);
        font-size: 0.85rem;
        margin: 8px 0 0;
    }

    .empty {
        color: var(--text-muted);
        text-align: center;
        margin: 24px 0;
    }

    ul {
        list-style: none;
        margin: 16px 0 0;
        padding: 0;
        overflow-y: auto;
    }

    li {
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: 6px 4px;
        border-bottom: 1px solid var(--border);
    }

    .remove {
        background: none;
        border: none;
        color: var(--text-muted);
        font-size: 1.1rem;
        cursor: pointer;
        padding: 0 6px;
    }

    .remove:hover {
        color: var(--danger, #c0392b);
    }

    footer {
        margin-top: 16px;
        text-align: right;
    }
</style>
