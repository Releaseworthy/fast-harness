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
- Colours come only from `nh_core::theme` roles (TUI `tui/theme.rs` Styles, GUI `Theme` global): no literal colours besides the neutral popup shadow. A palette change must keep `theme_tests.rs` green (WCAG 2.2 and APCA levels, greyscale themes pure grey), and every status colour comes with a mark or a word.

## Tests

- Model calls are tested with `http::ScriptedTransport`, including bodies split into tiny chunks.
- Filesystem tests use a fresh temporary directory under `std::env::temp_dir()` and remove it.
- Every bug fix comes with a test that failed before it.

## Pitfalls

- Two branches that add crates both touch Cargo.lock: on conflict take one side, run cargo metadata, commit, then build with --locked.
- The ring crate (rustls) needs the MSVC C compiler.
- Wire capture (off by default, `"wire": true` or `NH_WIRE=1`) stores every request body: its size grows with the square of the step count.
- Slint 1.18.1 StyledText is not selectable: the GUI copies through a hidden TextInput.
- Slint 1.18.1 std-widgets colours are read only (only `Palette.color-scheme` can be set) and follow the OS accent: use the themed widgets in `ui/widgets.slint`, not Button, TextEdit, ComboBox or StandardTableView.
- GUI icons come only from the embedded "Fast Harness Icons" font (a Nerd Font subset, `crates/nh-gui/fonts/`), named in `ui/icons.slint`: a new glyph needs the subset rebuilt with its code point (`fonts/NOTES.md`). Check glyphs in the shots test (`NH_SHOTS_DIR=<dir> cargo test -p nh-gui shots -- --ignored`).
- TUI Nerd Font glyphs must be one cell wide (the width test in `tui/icons.rs`): U+26A1 is double width.
- `WindowMoveArea` delays the press, so a TouchArea double click inside it never fires: the title bar double click and every window state change go through `src/window_state.rs` (the .slint `maximized` property drifted from the real state).
- Real-window checks: the release exe with `NH_HOME` set to a fresh temp folder, PrintWindow and posted clicks after SetProcessDPIAware; posted clicks cannot drag or resize the window.
- Opening the TUI creates a session file even if nothing is sent.
