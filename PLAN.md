# native-harness plan

A barebone coding agent harness in native Rust, in the spirit of DeepSeek Harness (DSH) but written from scratch (no pi-ai): fast, small, with a trace of every turn and every step. Windows first; the core is portable to Linux, macOS, Android and iOS.

## Status

| Phase | Content | State |
|---|---|---|
| 0 | Spike: Slint window, ratatui screen, one Claude call; size, start time, idle RAM | done (`docs/perf.md`) |
| 1A | Claude adapter: SSE, request body, streaming assembly, OAuth identity, errors, models | done |
| 1B | Tools: read, write, edit, shell, grep, glob | done |
| 1C | Trace, wire capture, sessions, `nh trace` | done |
| 1D | Accounts core: encrypted store, usage, switch order (the claude-switch import was removed 2026-11-11) | done |
| 2A | Agent loop, subagents, config, CLI print mode | done |
| 2B | Accounts online: OAuth sign-in, refresh, usage polling, account pool | done |
| 2C | `nh accounts`, the pool as default credential, shared `app::Runtime`; live call checked | done |
| 3V | Shared transcript model (`view`) for the TUI and the GUI | done |
| 3A | TUI (ratatui) | done |
| 3B | GUI (Slint) | done |
| 4 | Live checks, performance report (`docs/perf.md`) | done |
| 5 | Model and effort pickers at the chat box (TUI, GUI), readable trace timeline (TUI, GUI, `nh trace`), themes with WCAG and APCA tests, wire capture off by default | done |

## Decisions

| # | Topic | Chosen |
|---|---|---|
| D1 | Dependencies | Short list only (see `Cargo.toml` `[workspace.dependencies]`): serde, serde_json, ureq, regex, ignore, windows-sys, ratatui, crossterm, pulldown-cmark, slint. All harness logic is our own code. A new crate needs the maintainer's OK. |
| D2 | Concurrency | Threads and blocking I/O, no async runtime. |
| D3 | HTTP and TLS | ureq 3 with rustls (ring) and the OS certificate store (`platform-verifier`). |
| D4 | JSON | serde_json with serde derive. |
| D5 | GUI | Slint (royalty-free licence: the app shows a "made with Slint" credit). |
| D6 | TUI | ratatui and crossterm. |
| D7 | Trace | DSH event vocabulary with our own format version; raw wire capture off by default (owner, 2026-11-11), on with `"wire": true` in `config.json` or `NH_WIRE=1`. |
| D8 | claude-switch import | Removed 2026-11-11 at the owner's request: no import, no linked accounts, nh never reads DSH's credential file. Linked accounts in older `accounts.json` files are dropped on load. |
| D9 | Token storage | `accounts.json`, secrets encrypted with DPAPI on Windows. |
| D10 | Auto switch | The claude-switch rules (below), usage of idle accounts polled only while the TUI or GUI is open. |
| D11 | Credentials | Anthropic API keys and Claude subscription OAuth signed in by nh itself. |
| D12 | v1 scope | Agent loop; tools read, write, edit, shell, grep, glob, subagent; sessions; AGENTS.md in the system prompt; trace and `nh trace`; Claude provider; accounts; CLI print mode; TUI; GUI. |
| D13 | Default model | `claude-opus-5-5`, effort `high`; model list from `/v1/models`. |
| D14 | Repo | Public, `Releaseworthy/native-harness`, licence MIT OR Apache-2.0; binaries `nh` and `nh-gui`. |
| D15 | Themes | `nh_core::theme`: Dark (default), Light, Greyscale dark, Greyscale light, shared by the TUI and the GUI. Tests hold every theme to WCAG 2.2 (7:1 body text, 4.5:1 other text, 3:1 borders and focus) and APCA Bronze (Lc 75 body, 60 other text); greyscale themes are pure greys for text and background; status colours always come with a mark or a word. The pick is saved as `theme` in `config.json`. The GUI draws its own widgets (`ui/widgets.slint`) because the Slint std-widgets palette is read only. |
| D16 | Model and effort at the chat box | GUI: two dropdowns in the composer; TUI: Ctrl+P and Ctrl+E pickers above the input. Effort choices per model (`llm::anthropic::efforts`); a model change refits the effort (`fit_effort`). Picks apply to the next turn and last for the window (not saved). |

## Architecture

| crate | output | content |
|---|---|---|
| `nh-core` | library | types, Claude provider, agent loop, tools, trace and sessions, accounts, config, platform layer |
| `nh` | `nh.exe` | CLI print mode, TUI, `nh accounts`, `nh sessions`, `nh trace` |
| `nh-gui` | `nh-gui.exe` | Slint GUI (added in phase 3B) |

`nh-core` modules, lower never imports higher: `types` > `http`, `sse`, `platform` > `llm`, `tools`, `auth` > `trace`, `session` > `accounts` > `agent`. `events` holds what front ends receive.

- Threads: one UI thread, one worker thread per running session; the tool calls of one step run on scoped threads.
- Agent to UI: `AgentEvent` values over a channel. UI to agent: prompt, cancel, switch account, new session, resume.
- Every traced event is written to the session log before a front end receives it.
- Contracts are code: `http::Transport`, `llm::Provider`, `tools::Tool`, `auth::AuthSource`, `events::AgentEvent`, `platform::*`.

## Files on disk

- `~/.nh/config.json`: model, effort, cache TTL, limits, `theme` (colour theme id) and `wire` (raw wire capture, default `false`). `NH_MODEL`, `NH_EFFORT`, `NH_THEME` and `NH_WIRE` override it; `NH_HOME` moves `~/.nh`.
- `~/.nh/accounts.json` (version 2): own and API-key accounts; tokens and keys encrypted (DPAPI on Windows). Version 1 also held linked accounts: they are dropped on load (an `active` pointing at one falls back to the first remaining account) and the next save writes version 2. Readers accept versions 1 and 2 and reject any other.
- `~/.nh/sessions/<cwd slug>/<session id>.jsonl`: one log per session (transcript and trace).
- `~/.nh/sessions/<cwd slug>/<session id>.wire/`, only with wire capture on: per model call attempt, the request (headers with credentials redacted, body) and the raw response stream.

## Trace format

One JSON object per line. A header line, then events `{type, seq, time, data}`; `time` is Unix milliseconds, `seq` counts from 0. Event names follow the DSH session log.

```
{"type":"session","version":2,"id":"...","createdAt":1794400565123,"cwd":"F:\\work","harness":"nh 0.1.0","parent":null}
{"type":"turn/start","seq":0,"time":...,"data":{"turn":1}}
{"type":"user/message","seq":1,"time":...,"data":{"turn":1,"message":{...}}}
{"type":"step/start","seq":2,"time":...,"data":{"turn":1,"step":1}}
{"type":"request/header","seq":3,"time":...,"data":{"system":"...","tools":[...]}}
{"type":"request/context","seq":4,"time":...,"data":{"turn":1,"step":1,"attempt":1,"provider":"anthropic","model":"claude-opus-5-5","account":"u-1f3c","effort":"high","wire":"t1-s1-a1"}}
{"type":"llm/retry","seq":5,"time":...,"data":{"turn":1,"step":1,"attempt":1,"delayMs":2000,"error":"Overloaded (HTTP 529): ..."}}
{"type":"account/switch","seq":6,"time":...,"data":{"from":"u-1f3c","to":"u-77ab","reason":"RateLimit","holdUntil":...}}
{"type":"assistant/message","seq":7,"time":...,"data":{"turn":1,"step":1,"message":{...},"usage":{"inputTokens":4,"outputTokens":363,"cacheReadTokens":0,"cacheWriteTokens":24114},"stop":"tool_use","timing":{"ttfbMs":812,"firstTokenMs":1190,"totalMs":5100,"chunks":96,"bytes":18233},"account":"u-1f3c"}}
{"type":"tool/call","seq":8,"time":...,"data":{"turn":1,"step":1,"callId":"toolu_...","name":"read","arguments":"{...}"}}
{"type":"tool/result","seq":9,"time":...,"data":{"turn":1,"step":1,"message":{"role":"tool","content":[...]},"durationMs":3}}
{"type":"step/end","seq":10,"time":...,"data":{"turn":1,"step":1,"durationMs":5120}}
{"type":"turn/end","seq":21,"time":...,"data":{"turn":1,"steps":3,"usage":{...},"durationMs":14200,"outcome":"done"}}
```

- Version 2 (2026-11-11): `request/context` has the `wire` key (the attempt's capture name) only when wire capture is on; version 1 always had it. New logs are version 2; a resumed version 1 log keeps its header. Readers accept versions 1 and 2 and reject any other.
- `request/header` is written only when the system prompt or the tool list changes.
- A failed attempt writes `assistant/attempt` (`{turn, step, attempt, error, timing}`) before `llm/retry` or `account/switch`.
- A subagent writes its own session log whose header has `parent` set to the parent session id; the parent's `tool/result` names the child session.
- `nh trace [ID|latest] [--steps] [--json] [--cwd DIR]` prints the timeline of the session (`trace::detail`): each turn, what the user sent, the request header (system prompt and tools), each request (model, effort, account, attempt), the model's thinking, response text and tool calls with their arguments, tool results, the answer metrics, retries, failed attempts, account switches and the turn totals; then one row per step: turn, step, account, model, TTFB, total time, input, output, cache read and cache write tokens, tools with their durations, retries. `--steps` prints only the step table, `--json` the raw header and events.

## Claude provider

- `POST https://api.anthropic.com/v1/messages`, `stream: true`, `anthropic-version: 2023-06-01`.
- API key: `x-api-key`. Subscription OAuth: `Authorization: Bearer`, betas `claude-code-20250219` and `oauth-2025-04-20`, and the system prompt sent as two blocks, first "You are Claude Code, Anthropic's official CLI for Claude." (as pi-ai does; other clients depend on this and Anthropic may restrict it).
- Prompt caching: `cache_control` on the system blocks, the last tool and the last message; 5 minutes or 1 hour.

## Accounts

- Own accounts: OAuth sign-in in the browser (PKCE, loopback callback), refreshed by nh; or an API key. These are the only kinds: claude-switch links were removed 2026-11-11 (D8).
- Live usage: response headers `anthropic-ratelimit-unified-5h-utilization`, `-7d-utilization` (fractions), `-5h-reset`, `-7d-reset` (Unix seconds), `-status`, `-representative-claim`; idle accounts read `GET https://api.anthropic.com/api/oauth/usage`.
- Auto switch (D10), on a 429 or quota error: hold the current account until its window resets (15 minutes when unknown); skip held, rejected, Free plan and exhausted accounts; order the rest with claude-switch's `switchOrder`: unread accounts last; accounts whose 7-day window resets within 48 hours first, lowest 7-day percent first among them; otherwise the soonest 7-day reset; then the emptier 5-hour window; then the emptier fullest window. Install the first that works and retry at once; with nobody left, wait for the soonest hold and retry every minute.

## Tools

read, write, edit, shell (PowerShell on Windows, timeout, the whole process tree is killed), grep (regex, `.gitignore` aware), glob, subagent (a nested agent with its own session log; one level deep).

## Rules for the code

See `AGENTS.md`.
