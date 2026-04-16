# martin v0.1.0

Privacy-first meeting transcriber and dictation tool for Linux. Records microphone and system audio, transcribes locally with Whisper, and stores results in SQLite. No cloud, no internet required — your audio never leaves your machine.

## Install

Download `martin_0.1.0_amd64.deb` below and run:

```bash
sudo apt install ./martin_0.1.0_amd64.deb
```

On first use, the app automatically downloads the Whisper model (~466MB). Internet is only required for this one-time download.

### Requirements

- Linux with PipeWire (Ubuntu 22.10+, Fedora 36+)
- System dependencies are resolved automatically by apt

## Features

### Recording mode

Record meetings with dual audio capture — microphone and system audio (browser, Zoom, Meet) captured simultaneously via PipeWire.

- Start/stop recording from a simple UI
- Both audio sources are mixed into a single WAV file
- Recordings are saved as "pending" items that survive app restarts
- Transcribe any pending recording when you're ready
- Audio files are deleted after transcription — only the text is kept
- Falls back to mic-only if PipeWire or system audio is unavailable

### Dictation mode

Real-time speech-to-text that shows words as you speak.

- Microphone audio is transcribed live in ~5-second intervals
- Text appears on screen as you talk
- Full transcription is saved to history when you stop
- Useful for hands-free note taking or accessibility

### AI summary

Summarize any transcription into key points using Claude Code CLI (requires Claude Code installed separately).

### General

- **Local transcription** — Whisper small model runs entirely on your machine
- **Auto-download model** — Whisper model downloads automatically on first use with a progress bar
- **Bilingual UI** — Portuguese and English, detected from system locale
- **Transcription history** — browse, view, copy, and delete past transcriptions
- **Privacy first** — no cloud, no telemetry, data stays in local SQLite
- **Non-blocking UI** — long recordings are processed in the background

## Technical details

- Built with Tauri 2 (Rust backend + Svelte 5 frontend)
- Audio capture via cpal (ALSA) + pw-record (PipeWire)
- Transcription via whisper-rs (whisper.cpp bindings)
- Storage in SQLite via rusqlite
- 41 unit tests covering audio mixing, transcription, database, and model management

## Screenshots

| Recorder | History |
|----------|---------|
| ![Recorder](https://raw.githubusercontent.com/vagnerzampieri/martin/main/docs/images/recorder.png) | ![History](https://raw.githubusercontent.com/vagnerzampieri/martin/main/docs/images/history.png) |

| Summary |
|---------|
| ![Summary](https://raw.githubusercontent.com/vagnerzampieri/martin/main/docs/images/summary.png) |
