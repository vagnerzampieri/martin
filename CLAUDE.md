# Martin — Development Guidelines

## Project

Tauri 2 desktop app — Svelte 5 (SvelteKit) frontend, Rust backend. Captures microphone + system audio (via PipeWire's `pw-record`), transcribes locally with Whisper, stores results in SQLite. Privacy-first: no cloud, no internet required.

### Audio Pipeline

Recording uses two sources mixed into one WAV:
- **Microphone** — cpal with ALSA backend
- **System audio** — `pw-record` subprocess targeting default PipeWire sink (resolved via `wpctl inspect`)
- On stop, both WAVs are mixed (`audio/mix.rs`) and the temp files deleted
- Falls back to mic-only if PipeWire is unavailable

### i18n

UI strings live in `src/lib/i18n.js` — a plain JS translations object with `pt` and `en`. Locale detected from `navigator.language`. No library, no manual selector.

## Core Philosophy

Write code that is easy to change, easy to understand, and easy to delete. Favor simplicity over cleverness. Follow Rust idioms and Svelte conventions unless there is a concrete reason not to.

## TDD — Kent Beck

Follow the Red-Green-Refactor cycle for every change:

1. **Red** — Write a failing test that describes the behavior you want.
2. **Green** — Write the simplest code that makes the test pass. No more.
3. **Refactor** — Clean up duplication and improve design while keeping tests green.

Rules:
- Never write production code without a failing test.
- Make each step as small as possible. If a step feels big, break it down.
- When stuck, write a simpler test.
- Tests are first-class code — keep them clean and readable.

## Refactoring — Martin Fowler

- Refactor in small, named steps (Extract Function, Inline Variable, Move Module, Introduce Parameter Object, etc.).
- "Make the change easy, then make the easy change." Preparatory refactoring before adding features.
- Each refactoring step should keep tests passing. If tests break, the step was too big.
- Watch for code smells: Long Function, Large Module, Feature Envy, Data Clump, Primitive Obsession, Shotgun Surgery.
- Refactoring is not rewriting. Preserve behavior while improving structure.

## Ownership & Clarity — The Rust Programming Language

Rust's type system and ownership model are your design tools, not obstacles:

- **Let the compiler guide you.** If the borrow checker rejects your design, rethink the data flow rather than reaching for `clone()`, `Rc`, or `unsafe`.
- **Prefer owned data at boundaries.** Functions that cross module boundaries should take owned types. Borrowing is for internal, short-lived access.
- **Make illegal states unrepresentable.** Use enums with data, newtypes, and the type system to prevent invalid states at compile time instead of runtime checks.
- **Error handling is design.** Use `Result<T, E>` with meaningful error types. Avoid `.unwrap()` in production code — reserve it for cases where failure is truly impossible. Prefer `?` for propagation.
- **`unsafe` is a contract, not a shortcut.** Every `unsafe` block must have a comment explaining why it is sound. Minimize the surface area of `unsafe` code.
- **Zero-cost abstractions are the goal.** Use traits, generics, and iterators freely — they compile away. But don't abstract prematurely; a concrete type is fine until you need polymorphism.

## Clean Code — Robert C. Martin

- **Names reveal intent.** A name should tell you why it exists, what it does, and how it is used. If a name requires a comment, the name is wrong.
- **Functions do one thing.** They should be small, do one thing, and do it well.
- **Single Responsibility Principle.** A module has one reason to change. A function has one level of abstraction.
- **No side effects.** A function named `check_audio_device` should not also initialize a recording session.
- **Don't Repeat Yourself** — but only extract when you see real duplication (three or more occurrences), not structural similarity.
- **Boy Scout Rule.** Leave the code cleaner than you found it — but only in code you are already touching.

Apply these pragmatically. These are guidelines, not laws. If following a principle makes the code harder to understand, reconsider.

## Modular Design — Sandi Metz (adapted for Rust)

Size constraints keep code navigable:

1. **Modules should have a single, clear responsibility.** If you can't describe it in one sentence, split it.
2. **Functions should be under 20 lines.** Rust is more verbose than Ruby — but long functions still signal too many responsibilities.
3. **Pass no more than 4 parameters.** Group related parameters into a struct.
4. **Tauri commands should do one thing** — parse input, delegate to a domain function, return the result.

Principles:
- Depend on behavior, not data. Define traits for capabilities, not for data shapes.
- Favor composition over deep module nesting. Flat is better than nested.
- If you don't know the right abstraction yet, duplication is cheaper than the wrong abstraction.
- "Prefer duplication over the wrong abstraction."

## Pragmatic Architecture — Tauri & Svelte

The framework conventions are the default. Extract abstractions only when complexity demands it:

- **Rust backend** handles system concerns: audio capture, WAV mixing, transcription, file I/O, database. Each concern lives in its own module (`audio/`, `db/`, `transcribe/`).
- **Svelte frontend** handles presentation: components, user interaction, state display. Keep components focused on one view or behavior.
- **Tauri commands are the API boundary.** They are the contract between frontend and backend. Keep them stable, well-named, and documented.
- **State belongs where it is used.** Rust `Mutex<State>` for backend state. Svelte `$state()` for UI state. Don't mirror state across the boundary unnecessarily.
- **Extract when you feel pain**, not before. Signs you need a new module or component:
  - A module has multiple unrelated responsibilities.
  - A component does both data fetching (invoke) and complex rendering.
  - The same logic is duplicated across commands or components.
- **Avoid empty abstractions.** Don't create a wrapper module that just re-exports one function.

## Sustainable Development

- **Follow conventions.** The best Rust looks like the Rust book. The best Svelte looks like the Svelte docs.
- **Boring is good.** Avoid clever macros, exotic patterns, or crates that replace standard library functionality.
- **Database constraints matter.** SQLite schema should enforce what the code expects — NOT NULL, foreign keys, unique constraints.
- **Migrations are permanent code.** Write them to be safe and reversible.
- **Keep dependencies minimal.** Every crate and npm package is a dependency and a liability. Prefer std-lib solutions.
- **Don't optimize for hypothetical scale.** Solve the problem in front of you.
- **Guide decisions with cost of change.** Easy-to-change decisions can be deferred. Hard-to-change decisions (database schema, IPC command signatures, audio pipeline) deserve more thought upfront.

## Testing Approach

- **Rust:** Use built-in `#[cfg(test)]` modules and `cargo test`. Test domain logic in isolation — audio processing, transcription pipeline, database operations.
- **Svelte:** Use Vitest for component and logic tests.
- Test behavior, not implementation. Tests should describe what the code does, not how.
- Unit tests for pure functions and data transformations.
- Integration tests for Tauri command handlers (input → output through the full backend path).
- Avoid mocking unless the external dependency is slow, flaky, or has side effects (audio hardware, Whisper model loading).
- Each test should be independent and repeatable.

## Code Style

### Rust
- Follow `rustfmt` defaults. Run `cargo fmt` before every commit.
- Run `cargo clippy` and fix all warnings — clippy knows more Rust idioms than you remember.
- Use `snake_case` for functions and variables, `CamelCase` for types and traits, `SCREAMING_SNAKE_CASE` for constants.
- No comments that restate the code. Comments explain *why*, never *what*.
- Prefer early returns and `?` operator over deep nesting.
- Group `use` statements: std, external crates, internal modules — separated by blank lines.

### Svelte / JavaScript
- Use Svelte 5 runes (`$state`, `$derived`, `$effect`, `$props`) — no legacy reactive syntax.
- `camelCase` for variables and functions, `PascalCase` for components.
- Scoped CSS in components. CSS variables in `global.css` for theming.
- Prefer `async/await` with try-catch for Tauri `invoke()` calls.
- Keep components small and focused. One component, one responsibility.

## Commands

```bash
# Development
npm run dev              # Svelte dev server
cargo tauri dev          # Full app dev mode with hot reload

# Build
npm run build            # Frontend production build
cargo tauri build        # Full application build

# Quality
npm run check            # Svelte/TypeScript type checking
cargo fmt                # Format Rust code
cargo clippy             # Lint Rust code
cargo test               # Run Rust tests

# Setup
./scripts/download-model.sh small   # Download Whisper model (tiny|base|small|medium)
```
