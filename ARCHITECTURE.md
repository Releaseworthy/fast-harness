# Fast Harness architecture

The current state, read first. Decisions are in [docs/decisions/](docs/decisions/) (index in [PLAN.md](PLAN.md)); the map at the end says which one to open for an area. Formats are in [docs/formats.md](docs/formats.md), rules for changes in `AGENTS.md`.

## Product

A coding agent harness in native Rust: an agent loop with tools, model providers, a trace of every turn and every step, a terminal UI and a desktop GUI. v0.1 supports Windows only; the Linux CI job is a compile and test check.

## Crates

| crate | output | holds |
|---|---|---|
| `fast-harness-core` | library `fast_harness_core` | everything portable: types, HTTP, providers, agent loop, tools, trace and sessions, accounts, settings, themes, keys, panels, service, plugin API, `platform` |
| `fast-harness` | `fast-harness.exe` | CLI print mode (`-p`), TUI, `accounts`, `models`, `prompt`, `trace`, `stats`, `reflect`, `serve --stdio` |
| `fast-harness-gui` | `fast-harness-gui.exe` | Slint GUI; `fast-harness-gui-live` (feature `ui-files`) loads `.slint` files from `~/.fast-harness/ui/` ([docs/ui-contract.md](docs/ui-contract.md)) |
| `fast-harness-plugins` | library | `register_all`, generated from `plugins.toml` ([docs/plugins.md](docs/plugins.md)); sits above the core, only the binaries depend on it |

## Layers of `fast-harness-core`

From `crates/fast-harness-core/src/lib.rs`, a module never imports a higher one:

`types` / `theme` -> `http` / `sse` / `platform` -> `store` / `net` -> `llm` / `tools` / `auth` / `config` -> `trace` / `session` / `events` / `keys` -> `workspaces` / `accounts` -> `agent` -> `view` / `panels` / `stats` -> `reflect` -> `service` -> `ext` -> `app`.

The test `crates/fast-harness-core/tests/guard_rails.rs` holds this list and fails on an import of a higher layer, so the three must say the same. `net` is the real transport (`ureq`, `rustls`, the OS certificate store). OS specific code lives only under `platform/` (`windows/`, and `unsupported/` for every other OS).

## Threads

- UI thread: the GUI runs the Slint event loop; a turn's events reach it through `slint::invoke_from_event_loop`. The TUI gets terminal events from a `fast-harness-input` thread over a channel.
- Agent thread: one per running turn, `turn-<session id>`, started by `service::Service::start` for the TUI, the GUI and `serve`; the service catches a panic on it and ends the turn `Failed` (D59). Only the thread that calls `Agent::run_turn` writes the session log, and every traced event is written before its `AgentEvent` goes to the sink (`FnMut(AgentEvent)`).
- Tool threads, by effect (`agent/tool_run.rs`): a run of consecutive parallel calls (`Effect::Read`, or a tool whose `parallel()` says true, such as the subagent tool) starts together on scoped threads; every other call runs alone, in call order, after everything before it.
- Subagents: each runs its own loop on a `subagent-<id>` thread with its own session log.
- Workers: model list (`fast-harness-models`), usage (`fast-harness-usage`), sign-in (`fast-harness-login`, TUI), `fast-harness-reflect`, session lists and trace reads (`fast-harness-gui-list`, `fast-harness-gui-trace`, `fast-harness-sessions`, `fast-harness-trace`: a log read never runs on a UI thread, `ui_thread::forbid` panics there in debug builds, D60); the shell tool, command hooks, panels and MCP stdio servers start their own helper threads.

## Contracts

Change one only together with every user of it.

- `http::Transport`: `send` streams the response body through `on_chunk` on the calling thread; any status is `Ok` at this layer (the caller reads `head.status`); `Flow::Stop` ends the transfer and still returns `Ok(head)`. Tests use `http::ScriptedTransport`.
- `llm::Provider`: `id()` is a short stable id written to the trace; `stream` runs one streaming call to completion, shows every increment to `on_delta` and returns the assembled `Completion`. On main: Claude (`anthropic`) and the providers of `providers/*.json` (`llm/custom.rs`; api `openai-chat` or `anthropic-messages`, models named `<provider id>/<id>`); `llm::catalog` picks one by model id.
- `tools::Tool`: `run` never panics and reports failure through `ToolOutput::is_error`; invalid arguments give an error that tells the model what to fix; `effect()` is `Run` unless the tool says otherwise, `parallel()` is true for `Effect::Read`.
- `auth::AuthSource`: `lease` gives the credential (may refresh first), `observe` reads response headers, `on_failure` answers an auth or rate-limit error with a `Failover`, `warning` never holds a secret.
- `events::AgentEvent`: owns its data (it crosses threads), `#[non_exhaustive]`; a traced event is in the log before a front end gets it.
- `platform`: every backend exports the same items (`secret`, `random`, `open`, `file`, `process`, `window`, `folder`); an OS without a backend returns `PlatformError::Unsupported`, and `protect` stays unsupported off Windows (no plaintext secrets).
- `ext::v1`: the native plugin API; a plugin imports only this module; the major version is in the path and `ext_api!` turns a version mismatch into a compile error; unstable for every 0.x release; the official binaries ship with an empty plugin list.

## Rules that bind every change

- The system prompt and the tool list are frozen per session; a change reaches a running session as a context note, never as a new system prompt (`agent/context.rs`). Messages are append-only: request N+1 repeats request N (`tests/prefix_guard.rs`). Compaction (`agent/compact.rs`) replaces the history with a summary.
- Tool results shrink to their budget (`Config::tool_budget`) before the first send, never after; the full text goes to the session's `.out/` folder.
- Every tool but `find_tools` is deferred: a session starts with `find_tools` and loads the others by searching (`llm/find.rs`); on Claude the others are sent with `defer_loading: true`.
- Files under the home folder are written only through `store::write_atomic` or `store::update`; a read-change-write is always `store::update` (it re-reads inside `<file>.lock`).
- Tool calls of one step run by effect (see Threads).
- `AgentEvent::ToolProgress` is live only: never traced, never sent to the model.
- Trust gates project files: nothing in `<project>/.fast-harness/` and no project `AGENTS.md` (root to cwd or nested) is read until the user trusts the project once (`config/trust.rs`, `trusted.json`, `layers::agents_trusted`, D62).
- Thinking goes back only to the model that made it (`Message.model`).
- Colours come only from `theme` roles; built-in themes are JSON in the user format ([docs/themes.md](docs/themes.md)).
- The full list (sizes, panics, secrets, tests) is in `AGENTS.md`.

## Files on disk

Full text: [docs/formats.md](docs/formats.md#files-on-disk). Home folder `~/.fast-harness` (`FAST_HARNESS_HOME` moves it):

- `config.json`: settings (model, effort, cache TTL, theme, greyscale, icons, wire, tool output budgets, compaction); `FAST_HARNESS_*` variables override it.
- `themes/*.json`: user themes. `keys.json`: key bindings. `panels/*.json`: panels. `providers/*.json`: providers and models.
- `trusted.json`: the trusted projects.
- `models.json`: the last model list.
- `accounts.json` (version 3): accounts, secrets encrypted with `platform::protect`.
- `workspaces.json`: the sidebar's workspace folders.
- `system-prompt.md`, `prompts/<cwd slug>.md`: the global and the per-workspace system prompt template.
- `<file>.lock`: exists only while a write of `<file>` runs.
- `sessions/<cwd slug>/<id>.jsonl`: one log per session (transcript and trace); `<id>.out/<callId>.txt`: full tool results; `<id>.wire/`: raw requests and responses, only with wire capture on; `index.json`: the cache of the session list (D60).
- `ui/*.slint`: the UI files of `fast-harness-gui-live`.

Project folder `<project>/.fast-harness/`, read only when trusted: `config.json`, `providers/*.json`, `keys.json`, `panels/*.json`. The `AGENTS.md` files from the repository root down to the working folder join the system prompt, also only when trusted (D62).

## Where the formats are

- [docs/formats.md](docs/formats.md): files on disk, trace format (versions), accounts (sign-in, usage, auto switch), tool output texts.
- [docs/providers.md](docs/providers.md): Claude and Antigravity wire details.
- [docs/extending.md](docs/extending.md): settings layers, trust, providers, tool, agent and command files. [docs/themes.md](docs/themes.md), [docs/panels.md](docs/panels.md), [docs/plugins.md](docs/plugins.md), [docs/ui-contract.md](docs/ui-contract.md), [docs/service.md](docs/service.md) (sessions as a service).
- Measurements: [docs/perf.md](docs/perf.md). GUI pitfalls: [docs/pitfalls-gui.md](docs/pitfalls-gui.md).

## Area to decision files

By decision title; each is `docs/decisions/Dnn.md`.

| area | decisions |
|---|---|
| dependencies, HTTP, JSON, threads | D01 D02 D03 D04 |
| scope, repo, name | D12 D14 D21 D22 D62 |
| trace, sessions, measuring | D07 D35 D48 D49 |
| accounts, credentials | D08 D09 D10 D11 D26 D32 D56 |
| Google Antigravity (branch `antigravity`) | D23 D24 D25 D53 |
| models, effort, catalog | D13 D16 D17 D24 |
| system prompt, context, cache | D18 D31 D36 |
| tool results, compaction | D37 D38 |
| tools | D30 D34 D54 D58 |
| steering, subagents, turn runner | D39 D40 D59 |
| settings layers, extension files | D41 D42 D43 D44 D45 D46 D47 D51 |
| plugins | D52 D57 D58 |
| home files, locks | D55 D56 |
| memory, reflect | D50 |
| GUI, TUI, icons, workspaces | D05 D06 D19 D20 D27 D28 D29 |
| themes | D15 D33 |
