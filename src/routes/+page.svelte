<script>
    import "../styles/global.css";
    import { t } from "../lib/i18n.js";
    import Recorder from "../lib/Recorder.svelte";
    import History from "../lib/History.svelte";
    import TranscriptionView from "../lib/TranscriptionView.svelte";

    let currentView = $state("recorder");
    let selectedTranscription = $state(null);

    function showTranscription(transcription) {
        selectedTranscription = transcription;
        currentView = "view";
    }

    function showRecorder() {
        currentView = "recorder";
        selectedTranscription = null;
    }

    function showHistory() {
        currentView = "history";
        selectedTranscription = null;
    }
</script>

<main>
    <header>
        <h1>martin</h1>
        <nav>
            <button
                class:active={currentView === "recorder"}
                onclick={showRecorder}
            >
                {t("record")}
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
        <Recorder onTranscribed={showTranscription} />
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
</style>
