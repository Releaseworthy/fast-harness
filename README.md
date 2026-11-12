# Fast Harness

A barebone coding agent harness in native Rust: an agent loop with tools, Claude as the model, a trace of every turn and every step, a terminal UI and a desktop GUI. Windows first; the core is portable. The repository is `fast-harness`; the binaries are `fast-harness` and `fast-harness-gui`.

Status: working on Windows. The core also builds and is tested on Linux, but stored account secrets are encrypted with DPAPI, so there sign-in accounts and pooled API keys are unavailable: use `ANTHROPIC_API_KEY`, or `providers/*.json` files with `keyEnv` keys (PLAN.md D9, D41). See [PLAN.md](PLAN.md) for the design and the decisions, [docs/perf.md](docs/perf.md) for measurements.

## Build

```
cargo build --release
cargo test --workspace
```

Binaries: `target/release/fast-harness.exe` (command line and TUI) and `target/release/fast-harness-gui.exe` (desktop window, with the robot-and-bolt icon).

## Use

```
fast-harness accounts login          # sign in with a Claude account in the browser
fast-harness accounts add-key        # or add an Anthropic API key
fast-harness                         # the TUI in the current folder
fast-harness -p "explain this repo"  # one turn, answer on stdout
fast-harness models                  # the models your credentials can use now, by provider, with their effort levels
fast-harness prompt [--cwd DIR]      # the system prompt a folder gets, and the file it comes from
fast-harness trace latest            # the newest session: what was sent, thinking, answers, tools, then one row per step
fast-harness trace latest --steps    # only the per-step table (--json: the raw events)
fast-harness-gui                     # the desktop window
```

The desktop window has its own title bar: the sidebar toggle, Settings and About on the left, the window buttons on the right. The sidebar lists the workspaces with their sessions and ends with the Accounts button. A session has a Chat tab and a Trace tab. The chat follows new output while you are at the bottom; scroll up and it stays put, with a "Jump to latest" button to go back down. Very long texts show their first 300 lines or 12,000 characters with a note, and Copy copies all of it.

In the chat, the GUI composer has a Model and a Thinking effort dropdown; in the TUI, Ctrl+P picks the model and Ctrl+E the effort. A pick applies to the next turn. The Trace tab (GUI) and F3 (TUI) show each turn as a timeline: what the user sent, every request (model, effort, account), the thinking, the response, tool calls and results, then the numbers; the GUI Steps switch and the TUI `s` key show the per-step table.

Tools load on demand: a session starts with one tool, `find_tools`, whose description names the others (read, write, edit, shell, grep, glob, subagent). The model finds the tools it needs and they stay loaded for the session; on Claude the other tools are sent deferred, so finding one does not break the prompt cache.

The system prompt is a short template (where and when the agent works, how to use the tools) plus the AGENTS.md files of the session's own repository, from its root down to the working folder. Change it in the GUI under Settings > System prompt, for every workspace (`~/.fast-harness/system-prompt.md`) or for one workspace (`~/.fast-harness/prompts/<folder slug>.md`, which wins); `{cwd}`, `{os}`, `{date}` and `{model}` are filled in, Reset goes back to the default, and a change applies to new sessions; a running or resumed session keeps its prompt and gets a `[context update]` note with the new text ahead of its next message.

Themes: Dark, Light, Aurora, Synthwave, Sunset, Emerald, Sakura and Citrus, each tested against WCAG 2.2 contrast and APCA. A theme is a JSON file: add your own to `~/.fast-harness/themes/`, starting from a built-in one with `"extends"` ([docs/themes.md](docs/themes.md)); one that fails the contrast check still loads and Settings lists the failing colour pairs. Greyscale is a setting for every theme, from 0 to 100: a slider and a number box in Settings > Appearance, F6 and Shift+F6 in the TUI, `greyscale` in `config.json`. Pick a theme in Settings > Appearance or with F5 in the TUI; the choices are saved in `config.json`. Icons come from Nerd Fonts: the GUI embeds a small subset, the TUI uses your terminal's Nerd Font (set `"icons": "plain"` or `FAST_HARNESS_ICONS=plain` if your terminal has none).

The Accounts page shows one card per provider (Claude) with its account count and status; a card opens that provider's accounts. Bare ids stay Claude models.

Accounts switch automatically when one hits its rate limit. Settings live in `~/.fast-harness/config.json` (`FAST_HARNESS_HOME` moves the folder): `model`, `effort`, `theme`, `greyscale`, `icons` and `wire`, among others; `FAST_HARNESS_MODEL`, `FAST_HARNESS_EFFORT`, `FAST_HARNESS_THEME`, `FAST_HARNESS_GREYSCALE`, `FAST_HARNESS_ICONS` and `FAST_HARNESS_WIRE` override them. Sessions and their traces live under `~/.fast-harness/sessions/`. Raw wire capture (the redacted request and raw response of every model call, in `<session>.wire/`) is off by default; turn it on with `"wire": true` or `FAST_HARNESS_WIRE=1`.

Using a Claude subscription (OAuth) outside Claude Code works only by sending Claude Code's identity, as other third-party clients do; Anthropic may restrict that. API keys carry no such risk.

## Safety

There is no approval step and no sandbox: the model's tool calls run at once, with your user's rights. `shell` runs any command, and `read`, `write`, `edit`, `grep` and `glob` take absolute paths, so they reach outside the working folder. Use it on code you trust, under version control, and in a virtual machine or a container for anything else.

- The `AGENTS.md` files of a repository become part of the system prompt, so a repository you did not write can steer the agent.
- A repository's `.fast-harness` folder (settings, tools, hooks, MCP servers, skills, panels) loads only after `fast-harness trust` (or `/trust`).
- A `PreToolUse` hook can refuse or rewrite a tool call before it runs: [docs/extending.md](docs/extending.md) has an example that blocks destructive commands.

## Troubleshooting

- `fast-harness doctor` lists what loaded, from where, and what was skipped and why.
- `fast-harness trace latest` shows every request, tool call and step of the newest session; with `"wire": true` the request and response bodies are kept too.
- A panic in the desktop window is appended to `~/.fast-harness/crash.log`; read it before attaching it to a report.

## Licence

MIT OR Apache-2.0, at your option. The GUI uses [Slint](https://slint.dev) under its royalty-free licence. The icon font in `crates/fast-harness-gui/fonts/` is a subset of [Nerd Fonts](https://www.nerdfonts.com) (SIL OFL 1.1; glyphs from Codicons CC BY 4.0, Material Design Icons Apache 2.0 and Octicons MIT, see `crates/fast-harness-gui/fonts/NOTES.md`).
