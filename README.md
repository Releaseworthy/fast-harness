# Fast Harness

A barebone coding agent harness in native Rust: an agent loop with tools, Claude as the model, a trace of every turn and every step, a terminal UI and a desktop GUI. Windows first; the core is portable. The repository is `fast-harness`; the binaries are `fast-harness` and `fast-harness-gui`.

Status: working on Windows. See [PLAN.md](PLAN.md) for the design and the decisions, [docs/perf.md](docs/perf.md) for measurements.

## Build

```
cargo build --release
cargo test --workspace
```

Binaries: `target/release/fast-harness.exe` (command line and TUI) and `target/release/fast-harness-gui.exe` (desktop window, with the lightning-bolt icon).

## Use

```
fast-harness accounts login          # sign in with a Claude account in the browser
fast-harness accounts add-key        # or add an Anthropic API key
fast-harness                         # the TUI in the current folder
fast-harness -p "explain this repo"  # one turn, answer on stdout
fast-harness models                  # the models your credential can use now, with their effort levels
fast-harness trace latest            # the newest session: what was sent, thinking, answers, tools, then one row per step
fast-harness trace latest --steps    # only the per-step table (--json: the raw events)
fast-harness-gui                     # the desktop window
```

The desktop window has its own title bar: the sidebar toggle, Settings and About on the left, the active account's 5-hour and 7-day usage, and the window buttons on the right. The sidebar lists the sessions and ends with the Accounts button. A session has a Chat tab and a Trace tab.

In the chat, the GUI composer has a Model and a Thinking effort dropdown; in the TUI, Ctrl+P picks the model and Ctrl+E the effort. Every time a picker opens, Fast Harness asks Claude's `/v1/models` for the newest models and the effort levels each one takes, and keeps the last list in `~/.fast-harness/models.json`. A pick applies to the next turn. The Trace tab (GUI) and F3 (TUI) show each turn as a timeline: what the user sent, every request (model, effort, account), the thinking, the response, tool calls and results, then the numbers; the GUI Steps switch and the TUI `s` key show the per-step table.

The system prompt is two lines (where and when the agent works, how to use the tools) plus the AGENTS.md files of the session's own repository, from its root down to the working folder.

Themes: Dark, Light, Greyscale dark and Greyscale light (text and background in pure greys), all tested against WCAG 2.2 contrast and APCA. Pick one in the GUI Settings page or with F5 in the TUI; the choice is saved in `config.json`. Icons come from Nerd Fonts: the GUI embeds a small subset, the TUI uses your terminal's Nerd Font (set `"icons": "plain"` or `FAST_HARNESS_ICONS=plain` if your terminal has none).

Accounts switch automatically when one hits its rate limit. Settings live in `~/.fast-harness/config.json` (`FAST_HARNESS_HOME` moves the folder): `model`, `effort`, `theme`, `icons` and `wire`, among others; `FAST_HARNESS_MODEL`, `FAST_HARNESS_EFFORT`, `FAST_HARNESS_THEME`, `FAST_HARNESS_ICONS` and `FAST_HARNESS_WIRE` override them. Sessions and their traces live under `~/.fast-harness/sessions/`. Raw wire capture (the redacted request and raw response of every model call, in `<session>.wire/`) is off by default; turn it on with `"wire": true` or `FAST_HARNESS_WIRE=1`.

Using a Claude subscription (OAuth) outside Claude Code works only by sending Claude Code's identity, as other third-party clients do; Anthropic may restrict that. API keys carry no such risk.

## Licence

MIT OR Apache-2.0, at your option. The GUI uses [Slint](https://slint.dev) under its royalty-free licence. The icon font in `crates/fast-harness-gui/fonts/` is a subset of [Nerd Fonts](https://www.nerdfonts.com) (SIL OFL 1.1; glyphs from Codicons CC BY 4.0, Material Design Icons Apache 2.0 and Octicons MIT, see `crates/fast-harness-gui/fonts/NOTES.md`).
