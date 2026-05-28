<script>
    import { finalizeProgress, requestCancel } from "./finalizeProgress.js";
    import { t } from "./i18n.js";
    import { tick } from "svelte";

    let expanded = $state(false);
    let confirming = $state(false);
    let dialogEl = $state();
    /** @type {HTMLElement | null} */
    let lastFocused = null;

    const SIZE = 36;
    const STROKE = 4;
    const RADIUS = (SIZE - STROKE) / 2;
    const CIRCUMFERENCE = 2 * Math.PI * RADIUS;

    let percent = $derived($finalizeProgress.percent);
    let dashOffset = $derived(
        CIRCUMFERENCE * (1 - Math.max(0, Math.min(100, percent)) / 100),
    );
    let cancelling = $derived($finalizeProgress.phase === "cancelling");

    async function openConfirm() {
        if (cancelling) return;
        lastFocused = /** @type {HTMLElement | null} */ (document.activeElement);
        confirming = true;
        await tick();
        dialogEl?.querySelector(".btn-secondary")?.focus();
    }
    function dismissConfirm() {
        confirming = false;
        lastFocused?.focus?.();
    }
    function confirmCancel() {
        confirming = false;
        lastFocused?.focus?.();
        requestCancel();
    }

    /** @param {KeyboardEvent} e */
    function handleDialogKey(e) {
        if (!confirming) return;
        if (e.key === "Escape") {
            e.preventDefault();
            dismissConfirm();
            return;
        }
        if (e.key === "Tab") {
            const focusables = dialogEl?.querySelectorAll("button");
            if (!focusables || focusables.length === 0) return;
            const first = focusables[0];
            const last = focusables[focusables.length - 1];
            if (e.shiftKey && document.activeElement === first) {
                e.preventDefault();
                last.focus();
            } else if (!e.shiftKey && document.activeElement === last) {
                e.preventDefault();
                first.focus();
            }
        }
    }
</script>

<svelte:window on:keydown={handleDialogKey} />

{#if $finalizeProgress.phase !== "idle"}
    <div class="banner" role="status" aria-live="polite">
        <div class="ring">
            <svg width={SIZE} height={SIZE} viewBox="0 0 {SIZE} {SIZE}">
                <circle
                    cx={SIZE / 2}
                    cy={SIZE / 2}
                    r={RADIUS}
                    fill="none"
                    stroke="var(--border)"
                    stroke-width={STROKE}
                />
                <circle
                    cx={SIZE / 2}
                    cy={SIZE / 2}
                    r={RADIUS}
                    fill="none"
                    stroke="var(--info)"
                    stroke-width={STROKE}
                    stroke-dasharray={CIRCUMFERENCE}
                    stroke-dashoffset={dashOffset}
                    stroke-linecap="round"
                    transform="rotate(-90 {SIZE / 2} {SIZE / 2})"
                    style="transition: stroke-dashoffset 200ms linear;"
                />
            </svg>
            <span class="percent">{Math.round(percent)}%</span>
        </div>
        <span class="label">
            {$finalizeProgress.jobLabel || t("finalizeBannerLabel")}
        </span>
        <button
            class="btn-details"
            onclick={() => (expanded = !expanded)}
        >
            {expanded ? t("hideDetails") : t("showDetails")}
        </button>
        <button
            class="btn-cancel"
            onclick={openConfirm}
            disabled={cancelling}
            aria-disabled={cancelling}
        >
            {t("cancelTranscription")}
        </button>
    </div>
    {#if expanded && $finalizeProgress.liveText}
        <div class="live-text">
            <pre>{$finalizeProgress.liveText}</pre>
        </div>
    {/if}
{/if}

{#if confirming}
    <div class="modal-backdrop">
        <div
            class="modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="cancel-title"
            aria-describedby="cancel-body"
            bind:this={dialogEl}
        >
            <h3 id="cancel-title">{t("cancelConfirmTitle")}</h3>
            <p id="cancel-body">{t("cancelConfirmBody")}</p>
            <div class="modal-actions">
                <button class="btn-secondary" onclick={dismissConfirm}>
                    {t("cancelConfirmNo")}
                </button>
                <button class="btn-danger" onclick={confirmCancel}>
                    {t("cancelConfirmYes")}
                </button>
            </div>
        </div>
    </div>
{/if}

<style>
    .banner {
        position: sticky;
        top: 0;
        z-index: 50;
        display: flex;
        align-items: center;
        gap: 12px;
        padding: 8px 16px;
        background: var(--surface);
        border-bottom: 1px solid var(--border);
    }
    .ring {
        position: relative;
        width: 36px;
        height: 36px;
        flex-shrink: 0;
    }
    .percent {
        position: absolute;
        inset: 0;
        display: flex;
        align-items: center;
        justify-content: center;
        font-size: 0.7rem;
        font-weight: 600;
    }
    .label {
        flex: 1;
        font-size: 0.9rem;
        color: var(--text-muted);
    }
    .btn-details {
        font-size: 0.85rem;
        padding: 4px 10px;
        background: transparent;
        border: 1px solid var(--border);
        color: var(--text-muted);
    }
    .btn-cancel {
        font-size: 0.85rem;
        padding: 4px 10px;
        background: transparent;
        border: 1px solid var(--accent);
        color: var(--accent);
    }
    .btn-cancel[disabled] {
        opacity: 0.5;
        cursor: wait;
    }
    .live-text {
        max-width: 600px;
        margin: 0 auto;
        padding: 8px 16px;
    }
    .live-text pre {
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: var(--radius);
        padding: 12px;
        max-height: 30vh;
        overflow-y: auto;
        white-space: pre-wrap;
        word-wrap: break-word;
        font-family: inherit;
        font-size: 0.9rem;
        line-height: 1.5;
    }
    .modal-backdrop {
        position: fixed;
        inset: 0;
        background: rgba(0, 0, 0, 0.55);
        display: flex;
        align-items: center;
        justify-content: center;
        z-index: 100;
    }
    .modal {
        background: var(--surface);
        border-radius: var(--radius);
        padding: 24px;
        max-width: 360px;
        text-align: center;
    }
    .modal h3 {
        margin-bottom: 8px;
    }
    .modal p {
        color: var(--text-muted);
        margin-bottom: 20px;
    }
    .modal-actions {
        display: flex;
        justify-content: center;
        gap: 12px;
    }
    .btn-secondary {
        background: var(--primary);
        color: white;
        padding: 8px 16px;
    }
    .btn-danger {
        background: var(--accent);
        color: white;
        padding: 8px 16px;
    }
</style>
