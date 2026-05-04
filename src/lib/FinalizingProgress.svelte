<script>
    import { t } from "./i18n.js";
    import { tick } from "svelte";

    let {
        percent = 0,
        liveText = "",
        cancelling = false,
        jobLabel = "",
        onCancel,
    } = $props();
    let confirming = $state(false);
    let liveTextEl = $state();
    let dialogEl = $state();
    let lastFocused = null;

    const SIZE = 96;
    const STROKE = 8;
    const RADIUS = (SIZE - STROKE) / 2;
    const CIRCUMFERENCE = 2 * Math.PI * RADIUS;

    let dashOffset = $derived(
        CIRCUMFERENCE * (1 - Math.max(0, Math.min(100, percent)) / 100),
    );

    // Whisper's progress callback only starts firing once inference begins.
    // Before that — model load, audio decode, first segment — `percent`
    // sits at 0. Show an indeterminate label so the user does not read
    // a frozen 0% as "stuck."
    let isIndeterminate = $derived(percent === 0 && !cancelling);

    $effect(() => {
        if (liveText && liveTextEl) {
            liveTextEl.scrollTop = liveTextEl.scrollHeight;
        }
    });

    async function requestCancel() {
        if (cancelling) return;
        lastFocused = document.activeElement;
        confirming = true;
        await tick();
        dialogEl?.querySelector(".btn-secondary")?.focus();
    }

    function dismissCancel() {
        confirming = false;
        lastFocused?.focus?.();
    }

    function confirmCancel() {
        confirming = false;
        lastFocused?.focus?.();
        onCancel?.();
    }

    function handleDialogKey(e) {
        if (!confirming) return;
        if (e.key === "Escape") {
            e.preventDefault();
            dismissCancel();
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

<div class="finalizing">
    {#if jobLabel}
        <span class="job-label">{jobLabel}</span>
    {/if}

    <div class="ring-wrap" class:indeterminate={isIndeterminate}>
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
                stroke-dasharray={isIndeterminate ? "20 200" : CIRCUMFERENCE}
                stroke-dashoffset={isIndeterminate ? 0 : dashOffset}
                stroke-linecap="round"
                transform="rotate(-90 {SIZE / 2} {SIZE / 2})"
                style="transition: stroke-dashoffset 200ms linear;"
            />
        </svg>
        {#if !isIndeterminate}
            <span class="percent">{Math.round(percent)}%</span>
        {/if}
    </div>

    <div class="status">
        <strong>{t("finalizing")}</strong>
        <span class="hint">
            {isIndeterminate ? t("loadingModel") : t("finalizingHint")}
        </span>
    </div>

    {#if liveText}
        <div class="live-text">
            <pre bind:this={liveTextEl}>{liveText}</pre>
        </div>
    {/if}

    <button
        class="btn-cancel"
        onclick={requestCancel}
        disabled={cancelling}
        aria-disabled={cancelling}
    >
        {t("cancelTranscription")}
    </button>
</div>

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
                <button class="btn-secondary" onclick={dismissCancel}>
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
    .finalizing {
        display: flex;
        flex-direction: column;
        align-items: center;
        gap: 16px;
        padding: 32px;
    }

    .job-label {
        font-size: 0.9rem;
        color: var(--text-muted);
    }

    .ring-wrap {
        position: relative;
        width: 96px;
        height: 96px;
    }

    .ring-wrap.indeterminate svg {
        animation: ring-spin 1.4s linear infinite;
    }

    @keyframes ring-spin {
        from { transform: rotate(0deg); }
        to { transform: rotate(360deg); }
    }

    .percent {
        position: absolute;
        inset: 0;
        display: flex;
        align-items: center;
        justify-content: center;
        font-size: 1.1rem;
        font-weight: 600;
    }

    .status {
        display: flex;
        flex-direction: column;
        align-items: center;
        gap: 4px;
        text-align: center;
    }

    .hint {
        font-size: 0.85rem;
        color: var(--text-muted);
    }

    .live-text {
        width: 100%;
        max-width: 600px;
    }

    .live-text pre {
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: var(--radius);
        padding: 16px;
        white-space: pre-wrap;
        word-wrap: break-word;
        font-family: inherit;
        font-size: 0.95rem;
        line-height: 1.6;
        max-height: 40vh;
        overflow-y: auto;
    }

    .btn-cancel {
        background: transparent;
        color: var(--text-muted);
        border: 1px solid var(--border);
        padding: 8px 20px;
        font-size: 0.9rem;
    }

    .btn-cancel:hover:not([disabled]) {
        color: var(--accent);
        border-color: var(--accent);
    }

    .btn-cancel[disabled] {
        opacity: 0.5;
        cursor: wait;
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
