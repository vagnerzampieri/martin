# martin v0.3.0

Dictation rewrite focused on long-form, hands-free use (academic writing, journaling, note taking). The dictation flow is now precision-first, gives explicit feedback about what the app is doing, and recovers gracefully when whisper struggles on slow machines.

## Highlights

### Real-time UI feedback

- **Three explicit states** — 🎙 **Ouvindo** / 🧠 **Processando** / ⏸ **Pausa detectada** — with distinct colors and pulse animations. You always know whether the app is listening, working, or waiting.
- **VU meter** — a 16-segment bar reacts to your voice in real time so you can confirm the microphone is picking you up.
- **Provisional vs stable text** — text recently transcribed shows in gray italic (still being refined); confirmed text shows in normal style. Whisper's natural re-corrections happen in the gray zone instead of surprising you mid-paragraph.

### Smarter formatting

- **Automatic paragraphs on long pauses** — pause for ~5 seconds and the next thing you say starts a new paragraph automatically. Natural for dictating thoughts.
- **Portuguese voice commands** — say `novo parágrafo`, `nova linha`, `ponto final`, `vírgula`, `ponto de interrogação`, `ponto de exclamação`, `abre aspas`, `fecha aspas` or the alias `ponto parágrafo` to insert formatting without touching the keyboard. Matching is case-insensitive **and** accent-insensitive, so it still works when whisper drops a diacritic.
- **Smart text normalization** — sentences are capitalized automatically, punctuation spacing is normalized, and stray punctuation that whisper sometimes adds at the start of a chunk (a leading `.` after a long pause) is stripped.

### Reliability

- **Continuous auto-save** — the partial transcript is persisted to SQLite every ~5 seconds during the session. If the app crashes, you don't lose what you said.
- **Raw mic audio preserved** — each dictation session writes a WAV file alongside the transcript so the audio is available for reprocessing later.
- **Tail audio at stop** — the previous version sometimes dropped the last few seconds of dictation when you hit Stop. Stop now runs whisper on the un-transcribed tail and concatenates it with the live transcript.
- **Graceful failure on slow hardware** — when whisper hits memory pressure and returns an error (`-6`), the worker keeps the text it already produced instead of marking the whole dictation as failed.

### Performance for slow machines

- **Cargo release profile** — `lto = "fat"`, `codegen-units = 1`, `strip = true` give whisper a measurable speed boost on CPU-only builds.
- **Whisper thread count** — capped at physical core count (max 8) so the model doesn't over-subscribe weak machines.
- **Silence gate** — whisper is not invoked when the current 500 ms chunk is silent, saving CPU during natural pauses.
- **Tuned silence thresholds** — `SILENCE_THRESHOLD` lowered so soft, thoughtful speech is no longer mis-classified as silence. PAUSED state requires ~2 s of silence, paragraph breaks require ~5 s.

### Internal cleanups

- New pure modules `vad` (RMS-based silence detection) and `postprocess` (text normalization pipeline) with 30+ unit tests.
- Eliminated a duplicate whisper inference that was running at every 120 s buffer rollover.
- DB row for a dictation is now created at `start_dictation` (was at stop), enabling auto-save and proper history reflection during the session.

## Install

Download `martin_0.3.0_amd64.deb` from the GitHub Releases page and install:

```bash
sudo apt install ./martin_0.3.0_amd64.deb
```

If you previously had v0.2.x, the database schema is migrated automatically on first launch — your existing transcriptions are preserved and the new `audio_path` column is added with `NULL` defaults.

## Requirements

- Linux with PipeWire (Ubuntu 22.10+, Fedora 36+)
- System dependencies are resolved automatically by apt

## Notes for slow machines

The release profile and silence gate together deliver a meaningful CPU win, but very weak hardware (older laptops, low-RAM systems) may still see whisper struggle with the default `small` model. The codebase still uses `ggml-small.bin` by default; if performance is a problem, swap to `ggml-base.bin` (~140 MB) by replacing the file in `~/.local/share/com.nuuvem.martin/models/`.

## Full commit log

See `git log v0.2.1..v0.3.0` for the complete history.
