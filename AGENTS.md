# AGENTS.md

Rules for anyone (human or agent) changing this repo. Read `PLAN.md` first: it holds the decisions and the formats.

## Build and checks

- Windows: run commands with PowerShell (`pwsh`).
- Every change ends green: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
- Network tests that call real services are `#[ignore]` and run by hand.

## Code

- Functions and methods: at most 50 lines. Files: under 500 lines; split a module before it gets there.
- Dependencies: only the crates in `[workspace.dependencies]` of the root `Cargo.toml`. A new crate needs the maintainer's OK.
- Layering (`crates/nh-core/src/lib.rs`): a module never imports a higher one. OS specific code only under `platform/`; every `unsafe` block there has a `// SAFETY:` comment.
- The contracts are `http::Transport`, `llm::Provider`, `tools::Tool`, `auth::AuthSource`, `events::AgentEvent` and the `platform` items. Change one only together with every user of it.
- No panics on input from the network, files or the model: return errors. `unwrap` and `expect` only in tests or on invariants with a comment.
- Secrets: never in code, tests, fixtures, logs, traces or wire capture. Redact `authorization`, `x-api-key` and token fields. `Auth` prints as `<redacted>`.
- Formats in `PLAN.md` (trace, accounts) change only with a version bump and a `PLAN.md` update in the same commit.

## Tests

- Model calls are tested with `http::ScriptedTransport`, including bodies split into tiny chunks.
- Filesystem tests use a fresh temporary directory under `std::env::temp_dir()` and remove it.
- Every bug fix comes with a test that failed before it.
