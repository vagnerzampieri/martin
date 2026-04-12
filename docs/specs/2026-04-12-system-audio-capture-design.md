# System Audio Capture — Two streams mixed into one WAV

## Summary

Capture both microphone and system audio (monitor source) simultaneously, mixing them into a single WAV file. This allows recording meetings where the user's voice and other participants' audio are both captured.

## Architecture

On `start()`, open two cpal input streams:
- **Microphone** — `default_input_device()` (current behavior)
- **Monitor** — scan `host.input_devices()` for a device whose name contains `.monitor` (PipeWire/PulseAudio convention)

Both streams write to the same shared `WavWriterHandle`. Samples from both sources are interleaved at the callback level, naturally mixing the audio.

## Device Discovery

```
for device in host.input_devices():
    if device.name().contains(".monitor"):
        use as monitor device
        break
```

## Fallback Behavior

- No monitor found → mic-only recording, no error (log warning to stderr)
- Monitor has different sample rate → use mic's sample rate, convert monitor samples with f32-to-i16 conversion
- Monitor disappears mid-recording → error flag triggers, reported on stop()

## Changes

### `audio/capture.rs`

- `start()` — after setting up mic stream, discover monitor device, build a second stream writing to the same WAV writer with the same error flag
- `streams: Vec<Stream>` already supports multiple streams
- Both streams share the same `WavWriterHandle` and `AtomicBool` error flag

### No other files change

- WAV format unchanged (mono i16, single file)
- Stop/finalize flow unchanged
- Frontend unchanged
- Error handling unchanged

## Edge Cases

| Scenario | Behavior |
|----------|----------|
| No monitor source found | Mic-only recording, no error |
| Monitor different sample rate | Convert to mic's sample rate |
| Monitor disconnects mid-recording | Error flag set, reported on stop() |
| Multiple monitors found | Use first match |
