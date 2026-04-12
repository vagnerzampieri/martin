<script>
    import "../styles/global.css";
    import Recorder from "../lib/Recorder.svelte";

    let currentView = $state("recorder");
    let lastTranscription = $state(null);

    function onTranscribed(result) {
        lastTranscription = result;
        currentView = "result";
    }

    function backToRecorder() {
        currentView = "recorder";
        lastTranscription = null;
    }
</script>

<main>
    <h1>Martin</h1>
    <p class="subtitle">Transcritor de reunioes</p>

    {#if currentView === "recorder"}
        <Recorder {onTranscribed} />
    {:else if currentView === "result" && lastTranscription}
        <div class="result">
            <h2>{lastTranscription.title}</h2>
            <pre class="transcript">{lastTranscription.text}</pre>
            <button class="btn-back" onclick={backToRecorder}>
                Nova Gravacao
            </button>
        </div>
    {/if}
</main>

<style>
    main {
        max-width: 700px;
        margin: 0 auto;
        padding: 40px 20px;
        text-align: center;
    }

    h1 {
        font-size: 2.5rem;
        margin-bottom: 4px;
    }

    .subtitle {
        color: var(--text-muted);
        margin-bottom: 40px;
    }

    .result {
        text-align: left;
        padding: 20px;
    }

    .result h2 {
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
        margin-bottom: 20px;
    }

    .btn-back {
        background: var(--primary);
        color: white;
    }
</style>
