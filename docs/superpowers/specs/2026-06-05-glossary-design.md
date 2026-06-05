# Glossary — Custom Vocabulary for Transcription

**Issue:** [#1](https://github.com/vagnerzampieri/martin/issues/1)
**Date:** 2026-06-05
**Status:** Approved

## Problem

Whisper often misrecognizes domain-specific terms: technical jargon, proper
names, acronyms, course-specific vocabulary. Transcription quality is the
foundation of everything Martin does — summaries and future analysis are only
as good as the transcript.

## Solution

A single global glossary of terms, stored in SQLite, injected into Whisper's
`initial_prompt` on every transcription. `initial_prompt` is Whisper's native
mechanism for biasing decoding toward expected vocabulary.

### Why this approach

- **`initial_prompt` (chosen):** improves recognition at the source.
- Post-processing (fuzzy-replace) was rejected as the base mechanism: it can
  only fix near-misses, not gross errors. It may complement later.
- Plain-file storage was rejected: the app stores everything in SQLite.

## Design

### 1. Storage

New table in `db/store.rs`, following the existing migration pattern:

```sql
CREATE TABLE glossary_terms (
    id   INTEGER PRIMARY KEY,
    term TEXT NOT NULL UNIQUE
);
```

Operations: list, add (rejecting duplicates), remove.

### 2. Prompt building

New module `transcribe/glossary.rs` with a pure function:

```rust
pub fn build_initial_prompt(terms: &[String]) -> Option<String>
```

- Joins terms with commas: `"termo1, termo2, ..."` — no prefix, so the prompt
  is language-neutral (works for both pt and en transcriptions).
- Enforces a character cap (~700 chars, a safe margin under Whisper's
  ~224-token prompt limit). Terms beyond the cap are dropped in insertion
  order.
- Empty list → `None` (zero impact on the current flow).

### 3. Whisper integration

The three methods in `transcribe/whisper.rs` (`transcribe`,
`transcribe_samples`, `transcribe_with_callbacks`) gain an
`initial_prompt: Option<&str>` parameter, applied via
`params.set_initial_prompt()` when `Some`.

Call sites (`transcribe/job.rs`, `dictation.rs`) read the glossary terms from
the DB **once at job start** (not per chunk) and pass the built prompt.

Note: dictation uses `set_no_context(true)`, so the prompt applies to every
chunk independently — which is the desired behavior.

### 4. UI

- A "Glossary" button (📖) in the main page header opens a modal.
- Modal: list of terms with a remove action, an input to add (Enter submits),
  and a term counter.
- New component `src/lib/Glossary.svelte`, following the existing modal
  patterns.
- New strings in `src/lib/i18n.js` (pt and en).

### 5. Error handling

- Duplicate term: input feedback, no crash (UNIQUE constraint surfaces as a
  friendly message).
- DB read failure at job start: transcribe without a prompt rather than
  failing the job; log the error.

### 6. Testing

- `build_initial_prompt`: empty → `None`; joining; cap truncation; duplicates
  are not its concern (DB enforces uniqueness).
- Store: CRUD round-trip, uniqueness rejection.
- Whisper: signature compile test (same pattern as
  `transcribe_with_callbacks_signature_compiles`).
- Svelte: Vitest for the component if logic is non-trivial.

## Out of scope (YAGNI)

- Multiple named glossaries
- Term import/export
- Post-processing corrections
- Automatic term suggestion
