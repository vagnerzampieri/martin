# martin v0.5.0

Record while transcribing. Until now, kicking off a transcription locked the entire app — you could not start a new recording or import another file until the previous one finished, even though those operations don't touch the Whisper model. This release lifts that constraint: start your next recording immediately, and watch the previous transcription finish in a small banner at the top of the app.

## Highlights

### Record while transcribing

- **Non-modal banner** — when you click **Transcribe** on a pending recording, finalize progress shows as a slim bar at the top of the app (ring + percent + label) instead of taking over the Record tab. The banner stays visible across **Record**, **Dictation**, and **History** tabs so you always know a job is running in the background.
- **Start a new recording during finalize** — the **Start Recording** and **Import audio** buttons stay enabled while a transcription is finalizing. Only the Whisper-bound operations are gated: the **Transcribe** button on a new pending row, and **Start Dictation**, are disabled until the current finalize completes (with tooltips that explain why).
- **Details + Cancel from the banner** — click **Details** / **Detalhes** to expand a live-text panel with the partial transcript, or **Cancel** to stop the running finalize (with the same confirmation modal as before). Keyboard handling on the cancel dialog matches the existing dictation modal — Esc dismisses, Tab traps focus.
- **Recording state survives navigation** — recording state lives in a global store now, so navigating between tabs (or being auto-navigated when another transcription finishes) no longer loses the visible "Recording…" UI. The mic capture continues on the backend regardless; the UI now stays in sync.
- **No auto-yank while recording** — when a background transcription completes, martin no longer auto-navigates to the new transcription view if you are mid-recording. The transcription waits for you in **History** instead. If you're not recording, the existing auto-open behavior is preserved.

### Better diagnostics

- **Recording is now observable** — `start_recording` and `stop_recording` log `[martin]` entries on the backend with the file path and final duration. The previously silent start path is now traceable end-to-end.

## What didn't change

- **Dictation finalize is unchanged.** Dictation still uses its full-screen modal, since dictation cannot coexist with a new recording on the microphone anyway.
- **Concurrency model is unchanged.** Whisper still runs one transcription at a time. The backend `current_job` lock that has always serialized whole jobs is intact. The new banner just stops the *frontend* from locking the rest of the UI while the backend works.

## Install

Download `martin_0.5.0_amd64.deb` from the GitHub Releases page and install:

```bash
sudo apt install ./martin_0.5.0_amd64.deb
```

Upgrading from v0.4.x is seamless — no schema changes, no migration, no backend changes at all.

## Requirements

- Linux with PipeWire (Ubuntu 22.10+, Fedora 36+)
- System dependencies are resolved automatically by apt

## Full commit log

See `git log v0.4.0..v0.5.0` for the complete history.
