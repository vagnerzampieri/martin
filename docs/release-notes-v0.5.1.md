# martin v0.5.1

Maintenance release. **No new features and no user-visible behavior changes** — just an internal cleanup of the type-checker noise that had been accumulating since the project started using plain JavaScript with `svelte-check` in strict mode.

## What changed

- **`npm run check` now reports 0 errors** (down from 32). All implicit-`any` warnings across `format.js`, `i18n.js`, `History.svelte`, `Dictation.svelte`, `Recorder.svelte`, `ModelDownload.svelte`, `FinalizingProgress.svelte`, and `+page.svelte` are gone, fixed via JSDoc annotations.
- **Error formatting in catch blocks is explicit.** A couple of `error = e` assignments were coerced to `error = String(e)`. The rendered text is the same, but the type of the `error` state variable is now consistently `string`.
- **One minor defensive guard:** `clearInterval(timer)` in `Dictation.svelte` now skips when `timer` is null instead of relying on the no-op behavior of `clearInterval(null)` in browsers.

## Should I upgrade?

If you're using v0.5.0 it works fine — there are no bugfixes here. Upgrade only if you're hacking on martin and want a clean type-check loop.

## Install

```bash
sudo apt install ./martin_0.5.1_amd64.deb
```

## Full commit log

See `git log v0.5.0..v0.5.1` for the complete history.
