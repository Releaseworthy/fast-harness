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
nh trace latest            # one row per step of the newest session
nh-gui                     # the desktop window
```

Accounts switch automatically when one hits its rate limit. Settings live in `~/.nh/config.json` (`NH_HOME` moves the folder); sessions, their traces and the raw request and response of every model call live under `~/.nh/sessions/`.

Using a Claude subscription (OAuth) outside Claude Code works only by sending Claude Code's identity, as other third-party clients do; Anthropic may restrict that. API keys carry no such risk.

## Licence

MIT OR Apache-2.0, at your option. The GUI uses [Slint](https://slint.dev) under its royalty-free licence.
