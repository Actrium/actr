# Repository Guidelines

## Project Structure & Module Organization
- `src/` contains the primary Rust library re-exporting `actr_protocol`, `actr_framework`, and feature-gated crates. Treat it as the public API hub.
- `crates/` holds the core components (`protocol`, `framework`, `runtime`, etc.). Each subcrate is self-contained with its own `Cargo.toml`; edit inside these directories for implementation-level changes.
- `examples/` provides runnable reference apps such as `shell-actr-echo`. Mirror their layouts when wiring new services or clients.
- Docs and helper notes live in the repo root (`usage.md`, `explain.md`). Update them whenever behavior changes.

## Build, Test, and Development Commands
- `cargo build` — standard local build; run from the workspace root to compile every crate.
- `cargo check` — fast type/lint verification; run after editing any crate to ensure shared interfaces still compile.
- `cargo test` — executes the full test suite, including per-crate tests in `crates/*`.
- `cd /Users/kaito/Project/ar/actr && actr gen --input=../echo-service/proto --output=../echo-service/src/generated --clean` — regenerates protobuf + actor scaffolding for the `echo-service` sample.

## Coding Style & Naming Conventions
- Follow Rust 2021/2024 idioms: four-space indentation, snake_case for modules/functions, CamelCase for types.
- Run `rustfmt` (same options used by `actr gen`) before committing: `cargo fmt --all`.
- Keep comments concise and purposeful; prefer English for inline docs even when user-facing docs are localized.

## Testing Guidelines
- Unit tests live beside implementation files; integration tests belong in `tests/` when present.
- Use `#[cfg(test)] mod tests` patterns and meaningful test names (`test_actor_registration_flow`).
- Execute targeted tests with `cargo test -p crate_name` when iterating on a specific component; finish with a workspace-wide `cargo test` before merging.

## Commit & Pull Request Guidelines
- Commit messages follow the common imperative style (`Fix runtime tracing guard`). Keep them short but descriptive.
- Each PR should describe the change scope, mention affected crates or directories, and link to any relevant issues or design docs.
- Include reproduction or validation steps (commands, screenshots, or log excerpts) so reviewers can verify behavior quickly.
- Ensure generated files are up to date (`actr gen …`) and that formatters/tests have been run prior to opening a PR.

## Additional Tips
- Regenerating code may fail if `src/generated` files are read-only; run `chmod -R u+w src/generated` beforehand.
- Signaling-related examples expect the signaling server at `ws://localhost:8081/signaling/ws`; document deviations if you change endpoints.
