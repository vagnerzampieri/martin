<script>
    import "../styles/global.css";
    import { t } from "../lib/i18n.js";
    import Recorder from "../lib/Recorder.svelte";
    import Dictation from "../lib/Dictation.svelte";
    import History from "../lib/History.svelte";
    import TranscriptionView from "../lib/TranscriptionView.svelte";
    import { invoke } from "@tauri-apps/api/core";
    import { onMount } from "svelte";
    import ModelDownload from "../lib/ModelDownload.svelte";
    import { appBusy } from "../lib/appBusy.js";
    import FinalizeBanner from "../lib/FinalizeBanner.svelte";
    import { initFinalizeListeners } from "../lib/finalizeProgress.js";
    import { recordingState } from "../lib/recordingState.js";

    let currentView = $state("recorder");
    let selectedTranscription = $state(null);
    let modelReady = $state(true);
    let checkingModel = $state(true);

    onMount(() => {
        (async () => {
            try {
                modelReady = await invoke("check_model_exists");
            } catch {
                modelReady = false;
            }
            checkingModel = false;

            await initFinalizeListeners();
        })().catch(console.error);

        /** @param {Event} e */
        const onComplete = (e) => {
            // Don't yank the user away from an active recording — the backend
            // keeps capturing, but the next mount of <Recorder> would lose the
            // visible "Recording…" UI. The new transcription shows up in
            // History when the user navigates there.
            if ($recordingState.recording) return;
            const detail = /** @type {CustomEvent} */ (e).detail;
            if (detail) showTranscription(detail);
        };
        window.addEventListener("finalize:complete", onComplete);
        return () => {
            window.removeEventListener("finalize:complete", onComplete);
        };
    });

    function onModelDownloaded() {
        modelReady = true;
    }

    function onModelError(/** @type {string} */ _err) {
        // error is displayed inside ModelDownload; nothing to do here
    }

    function showTranscription(transcription) {
        selectedTranscription = transcription;
        currentView = "view";
    }

    function showRecorder() {
        currentView = "recorder";
        selectedTranscription = null;
    }

    function showDictation() {
        currentView = "dictation";
        selectedTranscription = null;
    }

    function showHistory() {
        currentView = "history";
        selectedTranscription = null;
    }
</script>

<main>
    <FinalizeBanner />
    {#if !checkingModel && !modelReady}
        <ModelDownload onComplete={onModelDownloaded} onError={onModelError} />
    {/if}
    <header>
        <h1>martin</h1>
        <nav>
            <button
                class:active={currentView === "recorder"}
                onclick={() => ($appBusy ? null : showRecorder())}
                aria-disabled={$appBusy}
                title={$appBusy ? t("navLockedTooltip") : ""}
            >
                {t("record")}
            </button>
            <button
                class:active={currentView === "dictation"}
                onclick={() => ($appBusy ? null : showDictation())}
                aria-disabled={$appBusy}
                title={$appBusy ? t("navLockedTooltip") : ""}
            >
                {t("dictation")}
            </button>
            <button
                class:active={currentView === "history"}
                onclick={showHistory}
            >
                {t("history")}
            </button>
        </nav>
    </header>

    {#if currentView === "recorder"}
        <Recorder />
    {:else if currentView === "dictation"}
        <Dictation onTranscribed={showTranscription} />
    {:else if currentView === "history"}
        <History onSelect={showTranscription} />
    {:else if currentView === "view" && selectedTranscription}
        <TranscriptionView
            transcription={selectedTranscription}
            onBack={showHistory}
        />
    {/if}
</main>

<style>
    main {
        max-width: 700px;
        margin: 0 auto;
        padding: 20px;
    }

    header {
        text-align: center;
        margin-bottom: 32px;
    }

    h1 {
        font-size: 2rem;
        margin-bottom: 12px;
    }

    nav {
        display: flex;
        justify-content: center;
        gap: 8px;
    }

    nav button {
        background: var(--surface);
        color: var(--text-muted);
        border: 1px solid var(--border);
        padding: 8px 20px;
    }

    nav button.active {
        background: var(--primary);
        color: var(--text);
        border-color: var(--primary);
    }

    nav button[aria-disabled="true"] {
        opacity: 0.4;
        cursor: not-allowed;
    }
</style>
