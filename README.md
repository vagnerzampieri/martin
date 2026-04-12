# Martin

Meeting transcriber for Linux. Record your meetings, transcribe locally with Whisper, keep your data on your machine.

## Features

- Record system audio and microphone
- Local transcription using Whisper (no internet needed)
- Portuguese and English support
- Transcription history with search
- Audio files deleted after transcription (privacy first)

## Install

### Prerequisites

- Rust 1.70+
- Node.js 18+
- System dependencies:

```bash
# Ubuntu/Debian
sudo apt install -y libwebkit2gtk-4.1-dev libgtk-3-dev libasound2-dev libpulse-dev build-essential libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

### Build

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

1. Open Martin
2. Click **Iniciar Gravacao** before your meeting
3. Have your meeting
4. Click **Parar Gravacao** when done
5. Click **Transcrever** — wait for local transcription
6. Done. Text is saved, audio is deleted.

## Whisper Models

| Model | Size | Quality | Speed |
|-------|------|---------|-------|
| tiny | 75MB | Usable | Fast |
| base | 142MB | Good | Fast |
| small | 466MB | Very good | Moderate |
| medium | 1.5GB | Excellent | Slow |

Download with: `./scripts/download-model.sh <model>`

## License

GPLv3
