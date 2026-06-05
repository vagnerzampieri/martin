# martin v0.6.0

Feature release: **Glossary**. Teach martin the words it keeps getting wrong.

## What's new

- **Custom vocabulary glossary.** A new "Glossário" button in the header opens
  a modal where you register technical terms, proper names and acronyms —
  things Whisper tends to mangle ("Foucault" → "FUCO", "PPGE" → "PPG").
  Registered terms are fed to Whisper as a recognition hint on **every**
  transcription: recordings, imported audio files and live dictation.
- **Fully local, as always.** Terms live in the same SQLite database as your
  transcriptions. Nothing leaves your machine.

## How it works

- Terms are read once at the start of each transcription job and joined into
  Whisper's `initial_prompt` (capped at ~700 bytes — roughly 60-80 terms, so
  prioritize the words that show up most).
- An empty glossary changes nothing: behavior is identical to v0.5.x.
- If the glossary can't be read for any reason, transcription proceeds
  without it — vocabulary hints are never worth a failed job.

## Tips

- Add the terms Whisper has already gotten wrong in your past transcriptions.
- Foreign author names, institution acronyms and project names benefit most.
- Removing a term takes effect on the next transcription, not one already
  running.

## Install

```bash
sudo apt install ./martin_0.6.0_amd64.deb
```

## Full commit log

See `git log v0.5.1..v0.6.0` for the complete history.
