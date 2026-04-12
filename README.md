# martin

Privacy-first meeting transcriber for Linux. Records microphone and system audio, transcribes locally with Whisper, stores results in SQLite. No cloud, no internet — your audio never leaves your machine.

## Screenshots

| Recorder | History |
|----------|---------|
| ![Recorder](docs/images/recorder.png) | ![History](docs/images/history.png) |

## Features

- **Dual audio capture** — records microphone + system audio (browser, Zoom, Meet) via PipeWire
- **Local transcription** — Whisper runs on your machine, no internet needed
- **Bilingual UI** — Portuguese and English, follows system locale
- **Transcription history** — browse, view, copy, and delete past transcriptions
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

1. Open martin
2. Click **Start Recording** / **Iniciar Gravação**
3. Have your meeting — both your mic and system audio are captured
4. Click **Stop Recording** / **Parar Gravação**
5. Click **Transcribe** / **Transcrever** — wait for local transcription
6. Done. Text is saved, audio is deleted.

## How audio capture works

martin records two audio sources simultaneously:

- **Microphone** — captured via cpal (ALSA backend)
- **System audio** — captured via `pw-record` targeting the default PipeWire sink

When you stop recording, both WAV files are mixed into a single file and sent to Whisper. If PipeWire is not available, Martin falls back to microphone-only recording.

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
cargo test               # Run Rust tests (23 tests)
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
│   └── store.rs        # SQLite CRUD for transcriptions
└── transcribe/
    └── whisper.rs      # Whisper transcription + WAV loading + resampling

src/
├── lib/
│   ├── i18n.js         # Locale detection + translations (pt/en)
│   ├── Recorder.svelte # Recording controls
│   ├── History.svelte  # Transcription list
│   └── TranscriptionView.svelte
└── routes/
    └── +page.svelte    # Main page
```

## License

GPLv3
