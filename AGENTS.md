# AGENTS.md

Rules for anyone (human or agent) changing this repo. Read `PLAN.md` first: it holds the decisions and the formats.

## Build and checks

- Windows: run commands with PowerShell (`pwsh`).
- Every change ends green: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
- Network tests that call real services are `#[ignore]` and run by hand.

## Code

- Functions and methods: at most 50 lines. Files: under 500 lines; split a module before it gets there.
- Dependencies: only the crates in `[workspace.dependencies]` of the root `Cargo.toml`. A new crate needs the maintainer's OK.
- Layering (`crates/fast-harness-core/src/lib.rs`): a module never imports a higher one. OS specific code only under `platform/`; every `unsafe` block there has a `// SAFETY:` comment.
- The contracts are `http::Transport`, `llm::Provider`, `tools::Tool`, `auth::AuthSource`, `events::AgentEvent` and the `platform` items. Change one only together with every user of it.
- No panics on input from the network, files or the model: return errors. `unwrap` and `expect` only in tests or on invariants with a comment.
- Secrets: never in code, tests, fixtures, logs, traces or wire capture. Redact `authorization`, `x-api-key` and token fields. `Auth` prints as `<redacted>`.
- Formats in `PLAN.md` (trace, accounts) change only with a version bump and a `PLAN.md` update in the same commit.
- Colours come only from `fast_harness_core::theme` roles (TUI `tui/theme.rs` Styles, GUI `Theme` global): no literal colours besides the neutral popup shadow. A palette change must keep `theme_tests.rs` green (WCAG 2.2 and APCA levels for every built-in theme at greyscale 0 to 100, pure grey at 100), and every status colour comes with a mark or a word. Built-in themes are JSON files (`crates/fast-harness-core/src/theme/builtin/`) in the same format users write (`docs/themes.md`, D33): a new or renamed role is a format change.

## Tests

- Model calls are tested with `http::ScriptedTransport`, including bodies split into tiny chunks.
- Filesystem tests use a fresh temporary directory under `std::env::temp_dir()` and remove it.
- Every bug fix comes with a test that failed before it.

## Pitfalls

- Two branches that add crates both touch Cargo.lock: on conflict take one side, run cargo metadata, commit, then build with --locked.
- The ring crate (rustls) needs the MSVC C compiler.
- Wire capture (off by default, `"wire": true` or `FAST_HARNESS_WIRE=1`) stores every request body: its size grows with the square of the step count.
- Slint 1.18.1 StyledText is not selectable: the GUI copies through a hidden TextInput.
- Slint 1.18.1 std-widgets colours are read only (only `Palette.color-scheme` can be set) and follow the OS accent: use the themed widgets in `ui/widgets.slint`, not Button, TextEdit, ComboBox or StandardTableView.
- GUI icons come only from the embedded "Fast Harness Icons" font (a Nerd Font subset, `crates/fast-harness-gui/fonts/`), named in `ui/icons.slint`: a new glyph needs the subset rebuilt with its code point (`fonts/NOTES.md`). Check glyphs in the shots test (`FAST_HARNESS_SHOTS_DIR=<dir> cargo test -p fast-harness-gui shots -- --ignored`).
- TUI Nerd Font glyphs must be one cell wide (the width test in `tui/icons.rs`): U+26A1 is double width.
- `WindowMoveArea` delays the press, so a TouchArea double click inside it never fires: the title bar double click and every window state change go through `src/window_state.rs` (the .slint `maximized` property drifted from the real state).
- Real-window checks: the release exe with `FAST_HARNESS_HOME` set to a fresh temp folder, PrintWindow and posted clicks after SetProcessDPIAware; posted clicks cannot drag or resize the window.
- Opening the TUI creates a session file even if nothing is sent.
- Slint's window `icon` sets only ICON_SMALL on the frameless winit window: the taskbar and Alt-Tab icon comes from `fast_harness_core::platform::set_big_window_icon` (WM_SETICON ICON_BIG from the exe's icon group 1).
- The main card clips its children to the rounded corner and draws its border in an overlay after them: a border on the clipping rectangle itself left a gap along the curve.
- Antigravity's Google OAuth client never enters the repo (D25): it lives encrypted in `~/.fast-harness/google-client.json` or in `FAST_HARNESS_GOOGLE_CLIENT_ID` and `_SECRET`. Before a push, `git grep -nIE "GOCSPX-[A-Za-z0-9_-]{10}"` finds nothing (the bare prefix appears in this line).
- `llm` never imports `accounts`: the Hub User-Agent and hosts live in `llm/antigravity/hub.rs`, and the verification phrase is duplicated in `llm/antigravity/provider/error.rs`, tied to `accounts::google::is_validation` by a test.
- Thinking and thought signatures go back only to the model that made them (`Message.model`, trace version 3): Antigravity sends `skip_thought_signature_validator` for calls of other models, Anthropic drops foreign thinking.
- The folder picker declares the IFileOpenDialog and IShellItem vtables by hand (`platform/windows/folder.rs`): check GUIDs and method order against the `windows` crate source before changing them.
- Live Antigravity check: `tests/antigravity_live.rs` (ignored) with `FAST_HARNESS_LIVE_GOOGLE_REFRESH` and the client variables set in the same process; it prints ids, stop reasons and token counts only.
- Never share one `CARGO_TARGET_DIR` between worktrees of this repo: cargo gives the workspace members the same unit hash in every worktree and judges freshness by file times, so a build silently compiled and tested another worktree's `fast-harness-core` (2026-11-12). Give each worktree its own target, and run `cargo clean -p fast-harness-core -p fast-harness -p fast-harness-gui` before gates in a target another tree used. `Copy-Item` keeps file times, so a copied source can look older than its stale artifact: touch it.
- The software renderer lays text out in i16 physical pixels (`PhysicalLength` in `i-slint-renderer-software` 1.18.1): a Text taller than 32767 px overlaps itself. Every long transcript or trace text goes through `trace_rows::expanded` (300 lines or 12K characters with a note; Copy keeps all).
- A mouse wheel on a ListView fires `scrolled` before its 180 ms animation moves the content, and the std scroll bar never fires `scrolled`: the transcript treats any user scroll as leaving the bottom and follows again only at the bottom or after "Jump to latest" (`ui/transcript.slint`, `src/scroll_tests.rs`). Long lists use the themed bar in `ui/scrollbar.slint` (the ListView's own bar off); its drag moves by the share of the track, since a pixel-offset drag snapped back while the ListView re-measured rows.
- Software renderer limits (`i-slint-renderer-software` 1.18.1 `lib.rs`): gradient fills ignore `border-radius`, clips are square ("TODO: handle radius"), `draw_box_shadow` draws nothing and a gradient window background comes out solid. Rounded gradients are built from solid rounded caps and a square band (`AccentFill` in `ui/parts.slint`), elevation and glow from stacked translucent rounded fills.
- `ui/widgets.slint` is close to the 500-line limit: put a new widget in its own file (as `ui/controls.slint` and `ui/scrollbar.slint`). A Rectangle that stacks Card layers around a layout needs its `preferred-height` from that layout.
- Every tool but `find_tools` is deferred (D30): tests that need all tools loaded build `Registry::new()` and add them, while `tools::builtin()` starts with `find_tools`. On Claude the `cache_control` breakpoint sits on `find_tools`, the last tool that is not deferred.
