# native-harness

A barebone coding agent harness in native Rust: an agent loop with tools, Claude as the model, a trace of every turn and every step, a terminal UI and a desktop GUI. Windows first; the core is portable.

Status: first working version on Windows. See [PLAN.md](PLAN.md) for the design and the decisions, [docs/perf.md](docs/perf.md) for measurements.

## Build

```
cargo build --release
cargo test --workspace
```

Binaries: `target/release/nh.exe` (command line and TUI) and `target/release/nh-gui.exe` (desktop window).

## Use

```
nh accounts login          # sign in with a Claude account in the browser
nh accounts import         # or reuse the claude-switch accounts of DeepSeek Harness (read-only)
nh accounts add-key        # or add an Anthropic API key
nh                         # the TUI in the current folder
nh -p "explain this repo"  # one turn, answer on stdout
nh trace latest            # the newest session: what was sent, thinking, answers, tools, then one row per step
nh trace latest --steps    # only the per-step table (--json: the raw events)
nh-gui                     # the desktop window
```

Accounts switch automatically when one hits its rate limit. Settings live in `~/.nh/config.json` (`NH_HOME` moves the folder): `model`, `effort`, `theme` (colour theme) and `wire`, among others; `NH_MODEL`, `NH_EFFORT`, `NH_THEME` and `NH_WIRE` override them. Sessions and their traces live under `~/.nh/sessions/`. Raw wire capture (the redacted request and raw response of every model call, in `<session>.wire/`) is off by default; turn it on with `"wire": true` or `NH_WIRE=1`.

Using a Claude subscription (OAuth) outside Claude Code works only by sending Claude Code's identity, as other third-party clients do; Anthropic may restrict that. API keys carry no such risk.

## Licence

MIT OR Apache-2.0, at your option. The GUI uses [Slint](https://slint.dev) under its royalty-free licence.
