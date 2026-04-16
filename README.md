# martin

Privacy-first meeting transcriber and dictation tool for Linux. Records microphone and system audio, transcribes locally with Whisper, stores results in SQLite. No cloud, no internet — your audio never leaves your machine.

Two modes: **Record** meetings with dual audio capture (mic + system), or **Dictate** with real-time speech-to-text that shows words as you speak.

## Screenshots

| Recorder | History |
|----------|---------|
| ![Recorder](docs/images/recorder.png) | ![History](docs/images/history.png) |

| Summary |
|---------|
| ![Summary](docs/images/summary.png) |

## Features

### Recording mode
- **Dual audio capture** — records microphone + system audio (browser, Zoom, Meet) via PipeWire
- **Pending recordings** — recordings are tracked in the database, survive app restarts, and can be transcribed or deleted from a list
- **Non-blocking stop** — stopping long recordings runs mixing in the background, UI stays responsive

### Dictation mode
- **Real-time transcription** — speaks and sees text appear live as you talk
- **Mic-only capture** — uses microphone directly, no system audio needed
- **Sliding window** — transcribes accumulated audio every ~5 seconds, re-processing the full buffer for better accuracy
- **Auto-save** — on stop, the full transcription is saved to history with title and duration

### General
- **Local transcription** — Whisper runs on your machine, no internet needed
- **Bilingual UI** — Portuguese and English, follows system locale (also used for transcription language)
- **Transcription history** — browse, view, copy, and delete past transcriptions
- **AI summary** — summarize transcriptions with key points via Claude Code CLI, with copy support
- **Privacy first** — audio files deleted after transcription, data stays in local SQLite

## Requirements

- Linux with PipeWire (Ubuntu 22.10+, Fedora 36+, etc.)
- `pw-record` and `wpctl` (included with PipeWire)
- Rust 1.70+
- Node.js 18+

### System dependencies

```bash
# Ubuntu/Debian
sudo apt install -y libwebkit2gtk-4.1-dev libgtk-3-dev libasound2-dev libpulse-dev \
  build-essential libssl-dev libayatana-appindicator3-dev librsvg2-dev pipewire
```

## Install

```bash
git clone https://github.com/YOUR_USER/martin.git
cd martin
npm install
cargo install tauri-cli --version "^2"
./scripts/download-model.sh small
cargo tauri build
```

The binary will be in `src-tauri/target/release/martin`.

## Usage

### Recording a meeting

1. Open martin and select the **Record** tab
2. Click **Start Recording** / **Iniciar Gravação**
3. Have your meeting — both your mic and system audio are captured
4. Click **Stop Recording** / **Parar Gravação** — recording appears in the pending list
5. Click **Transcribe** / **Transcrever** on the pending item — wait for local transcription
6. Done. Text is saved, audio is deleted.

You can record multiple times before transcribing — each recording is tracked separately. Close and reopen the app; your pending recordings are still there.

### Dictating text

1. Select the **Dictation** / **Ditado** tab
2. Click **Start Dictation** / **Iniciar Ditado**
3. Speak — text appears in real time as you talk
4. Click **Stop Dictation** / **Parar Ditado** — the full text is saved to history

## How audio capture works

### Recording mode

Records two audio sources simultaneously:

- **Microphone** — captured via cpal (ALSA backend)
- **System audio** — captured via `pw-record` targeting the default PipeWire sink

When you stop recording, both WAV files are mixed into a single file. If PipeWire is not available or the system audio is corrupt, martin falls back to microphone-only recording. The mixed file is saved as a pending recording and can be transcribed later.

### Dictation mode

Captures microphone audio via cpal and transcribes in real time:

- Audio streams into a shared buffer at the device's native sample rate
- Every ~5 seconds, the full accumulated buffer is converted to mono 16kHz and sent to Whisper
- Results are emitted as Tauri events (`dictation://segment`) and displayed live in the UI
- When the buffer exceeds 120 seconds, the current segment is committed and a new buffer starts

## Whisper Models

| Model | Size | Quality | Speed |
|-------|------|---------|-------|
| tiny | 75MB | Usable | Fast |
| base | 142MB | Good | Fast |
| small | 466MB | Very good | Moderate |
| medium | 1.5GB | Excellent | Slow |

Download with: `./scripts/download-model.sh <model>`

## Development

```bash
cargo tauri dev          # Full app dev mode with hot reload
cargo test               # Run Rust tests (37 tests)
npm run check            # Svelte/TypeScript type checking
cargo fmt                # Format Rust code
cargo clippy             # Lint Rust code
```

## Architecture

```
src-tauri/src/
├── lib.rs              # Tauri commands, app state
├── audio/
│   ├── capture.rs      # Mic (cpal) + system audio (pw-record)
│   ├── mix.rs          # WAV mixing (mic + system → single file)
│   └── wav_writer.rs   # Thread-safe WAV writer
├── db/
│   └── store.rs        # SQLite CRUD for transcriptions + pending recordings
├── dictation.rs        # Real-time mic capture + transcription loop
├── summarize.rs        # Claude CLI integration for AI summaries
└── transcribe/
    └── whisper.rs      # Whisper transcription + WAV loading + resampling

src/
├── lib/
│   ├── i18n.js         # Locale detection + translations (pt/en)
│   ├── format.js       # Shared date/duration formatting
│   ├── Recorder.svelte # Recording controls + pending recordings list
│   ├── Dictation.svelte # Real-time dictation with live text display
│   ├── History.svelte  # Transcription list
│   └── TranscriptionView.svelte  # View + copy + summarize
└── routes/
    └── +page.svelte    # Main page (three tabs: Record / Dictation / History)
```

## License

GPLv3
