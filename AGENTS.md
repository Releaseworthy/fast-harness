# AGENTS.md

Rules for anyone (human or agent) changing this repo. Read `ARCHITECTURE.md` first (the current state), then the decision file of the area you touch (`docs/decisions/Dnn.md`, index in `PLAN.md`).

## Build and checks

- Windows: run commands with PowerShell (`pwsh`).
- Every change ends green: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
- Network tests that call real services are `#[ignore]` and run by hand.
- A change to a request body, the find tool, compaction or subagents, and every release tag, also passes `pwsh scripts/live-smoke.ps1` (six small live calls): scripted tests cannot tell that the API accepts a body (the `find_tools` result was an HTTP 400 on Claude until 2026-11-12).

## Code

- Functions and methods: at most 50 lines. Files: under 500 lines; split a module before it gets there.
- Dependencies: only the crates in `[workspace.dependencies]` of the root `Cargo.toml`. A new crate needs the maintainer's OK.
- Layering (`crates/fast-harness-core/src/lib.rs`): a module never imports a higher one. OS specific code only under `platform/`; every `unsafe` block there has a `// SAFETY:` comment.
- Files under the home folder are written only through `store::write_atomic` or `store::update` (never `fs::write`); a read-change-write is always `store::update` (PLAN.md D55).
- The contracts are `http::Transport`, `llm::Provider`, `tools::Tool`, `auth::AuthSource`, `events::AgentEvent` and the `platform` items. Change one only together with every user of it.
- No panics on input from the network, files or the model: return errors. `unwrap` and `expect` only in tests or on invariants with a comment.
- Secrets: never in code, tests, fixtures, logs, traces or wire capture. Redact `authorization`, `x-api-key` and token fields. `Auth` prints as `<redacted>`.
- Formats in `docs/formats.md` (trace, accounts) change only with a version bump and an update there in the same commit. A decision is `docs/decisions/Dnn.md` plus its `PLAN.md` index line; `ARCHITECTURE.md` stays under 12,000 bytes.
- Colours come only from `fast_harness_core::theme` roles (TUI `tui/theme.rs` Styles, GUI `Theme` global): no literal colours besides the neutral popup shadow. A palette change must keep `theme_tests.rs` green (WCAG 2.2 and APCA levels for every built-in theme at greyscale 0 to 100, pure grey at 100), and every status colour comes with a mark or a word. Built-in themes are JSON files (`crates/fast-harness-core/src/theme/builtin/`) in the same format users write (`docs/themes.md`, D33): a new or renamed role is a format change.

## Tests

- Model calls are tested with `http::ScriptedTransport`, including bodies split into tiny chunks.
- Filesystem tests use a fresh temporary directory under `std::env::temp_dir()` and remove it.
- Every bug fix comes with a test that failed before it.

## Pitfalls

- Two branches that add crates both touch Cargo.lock: on conflict take one side, run cargo metadata, commit, then build with --locked.
- The ring crate (rustls) needs the MSVC C compiler.
- Wire capture (off by default, `"wire": true` or `FAST_HARNESS_WIRE=1`) stores every request body: its size grows with the square of the step count.
- TUI Nerd Font glyphs must be one cell wide (the width test in `tui/icons.rs`): U+26A1 is double width.
- GUI, Slint, window and icon pitfalls: docs/pitfalls-gui.md, read before touching crates/fast-harness-gui
- Opening the TUI creates a session file even if nothing is sent.
- Google Antigravity (provider, Google accounts, OAuth client, quota, its pitfalls and live test) lives on branch `antigravity` at a143cfa, not on main: PLAN.md D53 says how to bring it back.
- Thinking and thought signatures go back only to the model that made them (`Message.model`, trace version 3): Anthropic drops foreign thinking.
- Never share one `CARGO_TARGET_DIR` between worktrees of this repo: cargo gives the workspace members the same unit hash in every worktree and judges freshness by file times, so a build silently compiled and tested another worktree's `fast-harness-core` (2026-11-12). Give each worktree its own target, and run `cargo clean -p fast-harness-core -p fast-harness -p fast-harness-gui` before gates in a target another tree used. `Copy-Item` keeps file times, so a copied source can look older than its stale artifact: touch it.
- Every tool but `find_tools` is deferred (D30): tests that need all tools loaded build `Registry::new()` and add them, while `tools::builtin()` starts with `find_tools`. On Claude the `cache_control` breakpoint sits on `find_tools`, the last tool that is not deferred.
- Context, cache, compaction, subagent, steering and extension work follows docs/efficiency-plan.md: the system prompt and tool list are frozen per session, messages are append-only, tool results shrink before the first send (never after).
- Tool calls of one step run by effect (PLAN.md D54): a new tool that only reads declares `Effect::Read` and runs alongside other reads; anything else stays serial, in call order.
- Claude 5.x thinking blocks go back unchanged and bind to the prefix: never drop, shorten or reorder earlier messages on the client.
- Reachability is not struct fields: `ext::v1` also reaches `HttpRequest`, `HttpHead`, `HttpError`, `Flow`, `HttpErrorKind` and `ToolSession` through `Transport` and `ToolCtx`, so walk trait method signatures before listing the types a rule must cover.
- `#[non_exhaustive]` still allows `Default` plus field assignment: add setters only where outside code needs them.
- A cross-crate literal migration: list every site with one regex and fix all before the first `cargo check` (it stops per crate).
- `AgentEvent::ToolProgress` is live only (PLAN.md D58): never in the trace or a message; a view keeps a bounded tail and drops it at the result.
- A CRLF working copy shows as `i/lf w/crlf` in `git ls-files --eol`: the commit is fine and `git checkout` of the file rewrites the tree.
