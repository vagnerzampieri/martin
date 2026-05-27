# martin v0.4.0

Import existing audio files and transcribe them through martin's normal flow. Until now martin could only transcribe audio it recorded itself; this release lets you bring in voice memos, meeting recordings, and any audio you already have on disk.

## Highlights

### Import external audio

- **Pick a file, transcribe it** — a new **Import audio** / **Importar áudio** button on the Record tab opens a native file picker filtered to supported formats. The file lands in the pending-recordings list and transcribes exactly like a recording.
- **Common formats** — mp3, m4a, wav, ogg, flac. Decoding is done entirely in-process with [Symphonia](https://github.com/pdeljanov/Symphonia), a pure-Rust decoder — no `ffmpeg`, no external binaries, still fully offline.
- **Your original is never touched** — martin decodes a private mono WAV copy into its data directory and removes that copy after a successful transcription. The file you picked stays exactly where it was.
- **Easy on slow/low-RAM machines** — decoding streams one packet at a time instead of loading the whole file into memory, so importing a multi-hour recording won't exhaust RAM. Resampling to 16 kHz is deferred to transcription time, just like recorded audio.

### Better diagnostics

- **Import is now observable** — the import path logs entry, validation, decode, and save on the backend (`[martin]` prefix in the terminal) and dialog/selection/result on the frontend (`[import]` prefix in the dev console), including the previously silent "dialog dismissed" case. A no-op import now tells you exactly what happened.

## Install

Download `martin_0.4.0_amd64.deb` from the GitHub Releases page and install:

```bash
sudo apt install ./martin_0.4.0_amd64.deb
```

Upgrading from v0.3.x is seamless — the SQLite schema and your existing transcriptions are preserved.

## Requirements

- Linux with PipeWire (Ubuntu 22.10+, Fedora 36+)
- System dependencies are resolved automatically by apt

## Notes

Transcription speed is unchanged by this release — importing a long file gives you the same local Whisper pass as a recording of the same length. Parallel/chunked transcription for faster processing of long files is planned as a follow-up.

## Full commit log

See `git log v0.3.0..v0.4.0` for the complete history.
