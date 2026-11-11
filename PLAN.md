# Fast Harness plan

Fast Harness (this repo; binaries `fast-harness` and `fast-harness-gui`) is a barebone coding agent harness in native Rust, in the spirit of DeepSeek Harness (DSH) but written from scratch (no pi-ai): fast, small, with a trace of every turn and every step. Windows first; the core is portable to Linux, macOS, Android and iOS.

## Status

| Phase | Content | State |
|---|---|---|
| 0 | Spike: Slint window, ratatui screen, one Claude call; size, start time, idle RAM | done (`docs/perf.md`) |
| 1A | Claude adapter: SSE, request body, streaming assembly, OAuth identity, errors, models | done |
| 1B | Tools: read, write, edit, shell, grep, glob | done |
| 1C | Trace, wire capture, sessions, `fast-harness trace` | done |
| 1D | Accounts core: encrypted store, usage, switch order (the claude-switch import was removed 2026-11-11) | done |
| 2A | Agent loop, subagents, config, CLI print mode | done |
| 2B | Accounts online: OAuth sign-in, refresh, usage polling, account pool | done |
| 2C | `fast-harness accounts`, the pool as default credential, shared `app::Runtime`; live call checked | done |
| 3V | Shared transcript model (`view`) for the TUI and the GUI | done |
| 3A | TUI (ratatui) | done |
| 3B | GUI (Slint) | done |
| 4 | Live checks, performance report (`docs/perf.md`) | done |
| 5 | Model and effort pickers at the chat box (TUI, GUI), readable trace timeline (TUI, GUI, `fast-harness trace`), themes with WCAG and APCA tests, wire capture off by default | done |
| 6 | Fast Harness: live model and effort catalog (pickers refetch on open, `fast-harness models`), short system prompt, claude-switch import removed, frameless GUI with a DSH-style title bar, Chat and Trace tabs, Nerd Font icons (GUI and TUI), lightning-bolt app icon | done |
| 7 | Rename to fast-harness (D22), Google Antigravity accounts and provider (D23 to D26), title bar without the account pill (D27), workspaces in the sidebar (D28), robot-and-bolt app icon (D29) | in progress |

## Decisions

| # | Topic | Chosen |
|---|---|---|
| D1 | Dependencies | Short list only (see `Cargo.toml` `[workspace.dependencies]`): serde, serde_json, ureq, regex, ignore, windows-sys, ratatui, crossterm, pulldown-cmark, slint. All harness logic is our own code. A new crate needs the maintainer's OK. |
| D2 | Concurrency | Threads and blocking I/O, no async runtime. |
| D3 | HTTP and TLS | ureq 3 with rustls (ring) and the OS certificate store (`platform-verifier`). |
| D4 | JSON | serde_json with serde derive. |
| D5 | GUI | Slint (royalty-free licence: the app shows a "made with Slint" credit). |
| D6 | TUI | ratatui and crossterm. |
| D7 | Trace | DSH event vocabulary with our own format version; raw wire capture off by default (owner, 2026-11-11), on with `"wire": true` in `config.json` or `FAST_HARNESS_WIRE=1`. |
| D8 | claude-switch import | Removed 2026-11-11 at the owner's request: no import, no linked accounts, fast-harness never reads DSH's credential file. Linked accounts in older `accounts.json` files are dropped on load. |
| D9 | Token storage | `accounts.json`, secrets encrypted with DPAPI on Windows. |
| D10 | Auto switch | The claude-switch rules (below), usage of idle accounts polled only while the TUI or GUI is open. |
| D11 | Credentials | Anthropic API keys and Claude subscription OAuth signed in by fast-harness itself. |
| D12 | v1 scope | Agent loop; tools read, write, edit, shell, grep, glob, subagent; sessions; the AGENTS.md files of the session's project (from the nearest ancestor of the working directory holding `.git` down to it, only its own file without one) in the system prompt; trace and `fast-harness trace`; Claude provider; accounts; CLI print mode; TUI; GUI. |
| D13 | Default model | `claude-opus-5-5`, effort `high`; model list from `/v1/models`. |
| D14 | Repo | Public, `Releaseworthy/fast-harness` (named `native-harness` until 2026-11-12, D22), licence MIT OR Apache-2.0; binaries `fast-harness` and `fast-harness-gui`. |
| D15 | Themes | `fast_harness_core::theme`: Dark (default), Light, Greyscale dark, Greyscale light, shared by the TUI and the GUI. Tests hold every theme to WCAG 2.2 (7:1 body text, 4.5:1 other text, 3:1 borders and focus) and APCA Bronze (Lc 75 body, 60 other text); greyscale themes are pure greys for text and background; status colours always come with a mark or a word. The pick is saved as `theme` in `config.json`. The GUI draws its own widgets (`ui/widgets.slint`) because the Slint std-widgets palette is read only. |
| D16 | Model and effort at the chat box | GUI: two dropdowns in the composer; TUI: Ctrl+P and Ctrl+E pickers above the input. Effort choices per model (`llm::anthropic::efforts`); a model change refits the effort (`fit_effort`). Picks apply to the next turn and last for the window (not saved). |
| D17 | Live model catalog | Owner, 2026-11-11: fetch Claude's newest models and effort levels each time the user opens the model or effort picker. `GET /v1/models` (all pages, retired models left out) gives per model `capabilities.effort` levels and `capabilities.thinking.types`; a process-wide registry (`llm::anthropic::remember_models`) holds them, filled by every successful list and by `models.json` at start. `efforts` and `fit_effort` read it before the table; the request builder reads it only for models missing from the table (adaptive, budget or no thinking as the API says). Front ends call `Runtime::models()` then `ModelSource::fetch` off the UI thread; `cached()` gives the last known list or the table. A failed list never holds or switches the account. CLI: `fast-harness models [--json]`. |
| D18 | System prompt | Short (owner, 2026-11-11): "You are a coding agent in Fast Harness, working in {cwd} on {os}. Today is {date} (UTC)." plus one line on tools, then the project's AGENTS.md files (D12, 32 KB cap). Nothing from the workspace around the repository. The OAuth identity block "You are Claude Code, Anthropic's official CLI for Claude." stays first for subscription OAuth (Anthropic requires it). The app is Fast Harness (D22). |
| D19 | GUI frame | Owner, 2026-11-11: a custom title bar like DSH's. Frameless window (`no-frame`, 6 px resize border, winit) with a 40 px title bar: sidebar toggle, Settings, About, minimize, maximize or restore, close (the account usage pill was removed 2026-11-12, D27). Slint's `WindowMoveArea` drags the window; a double click on the empty bar maximizes; every window state change goes through `window_state.rs`. The sidebar holds the logo, New session, the sessions and, at the bottom, only an Accounts entry with a person icon. Chat and Trace are tabs of the session; Settings, About and Accounts open as a third, closable tab. |
| D20 | Icons | Nerd Fonts v3.5.1 (owner: "sử dụng nerdfont"). GUI: an embedded 60-glyph subset of Symbols Nerd Font renamed "Fast Harness Icons" (`crates/fast-harness-gui/fonts/`, with the Nerd Fonts licence and notes on the glyph sets: Codicons, Material Design Icons, Octicons); code points live in `ui/icons.slint`. TUI: glyphs from the terminal's Nerd Font, `"icons": "plain"` or `FAST_HARNESS_ICONS=plain` for a terminal without one; every glyph is one cell wide (never U+26A1). |
| D21 | Name and app icon | The app is Fast Harness (owner, 2026-11-11; the names in code followed with D22). The icon (D29 since 2026-11-12) lives in `crates/fast-harness-gui/assets/`: the window icon, and linked into `fast-harness-gui.exe` with version info ("Fast Harness") by `build.rs`, which writes a `.res` file in pure Rust (MSVC only). |
| D22 | Rename | Owner, 2026-11-12: "đổi tên repo và binary thành fast harness hết đi". Repo `Releaseworthy/fast-harness` (GitHub redirects the old URL), crates `fast-harness-core`, `fast-harness`, `fast-harness-gui`, binaries `fast-harness` and `fast-harness-gui`, data folder `~/.fast-harness`, env keys `FAST_HARNESS_*`. The first start renames an existing `~/.nh` to `~/.fast-harness` (`platform::adopt_old_home`; DPAPI is called without entropy, so secrets still decrypt); if the rename fails that run keeps using `~/.nh`. The trace header's `harness` reads `fast-harness <version>`. |
| D23 | Google Antigravity accounts | Owner, 2026-11-12: "accounts thì thêm google antigravity nữa". Google sign-in in the browser with Antigravity's OAuth client (PKCE, loopback callback), refreshed by Fast Harness; project from `loadCodeAssist` (or `onboardUser`); quota from `retrieveUserQuotaSummary` (groups "Gemini Models" and "Claude and GPT models", each with a 5 h and a weekly window); models from `fetchAvailableModels`: every model the account lists (Gemini, Claude, GPT-OSS). Requests follow the live-verified DSH antigravity-switch port: identity text first, Hub User-Agent, daily host then sandbox on 404, 408, 5xx or a network error, cleaned tool schemas, sanitised tool names, thought signatures carried, `VALIDATED` tool mode for Claude, `finishReason: OTHER` after a call read as a stop. Auto switch only among Google accounts (hold until the group's reset, then the account with the most quota left); a provider never switches by itself. Quota is polled only while a front end is open, at most every 10 minutes per account and 30 s or more after a turn. |
| D24 | Two providers in one picker | Owner, 2026-11-12: one model list grouped by provider. Antigravity models are named `antigravity/<id>` in config, pickers, traces and `fast-harness models`; a bare id is an Anthropic model (Antigravity also lists `claude-...` ids). |
| D25 | Google OAuth client | Owner, 2026-11-12: never in the repo. From `FAST_HARNESS_GOOGLE_CLIENT_ID` and `FAST_HARNESS_GOOGLE_CLIENT_SECRET`, else `~/.fast-harness/google-client.json`, written by `fast-harness accounts google-client --id <id>` with the secret read from stdin (never echoed), the secret encrypted like account secrets. Without a client, Google sign-in explains how to set one. |
| D26 | Account store v3 | Owner, 2026-11-12: one `accounts.json`, version 3: each account has `provider` (`anthropic` or `google`); Google secrets hold `refresh`, `access`, `expiresAt` and `projectId`. Version 2 records load as `anthropic`. One pool type per provider, one Accounts list. |
| D27 | Title bar | Owner, 2026-11-12: "remove accounts trên header". The account usage pill leaves the title bar; usage shows on the Accounts page. |
| D28 | Workspaces | Owner, 2026-11-12: "thêm cả workspaces bên trái nữa (tạo hình folder icon nha)". The sidebar lists WORKSPACES: each row a folder icon (an open folder for the current one) and the folder name (the full path on hover); a click selects it and expands its sessions, newest first; New session starts in the selected workspace; `+` adds a folder through the Windows "Select folder" dialog (IFileOpenDialog through windows-sys, on its own thread); a hover `x` removes it from the list only. Kept in `~/.fast-harness/workspaces.json` (version 1), seeded on first run with every folder that already has sessions plus the window's folder. |
| D29 | App icon v2 | Owner, 2026-11-12: a modern logo with a robot and a lightning bolt (a bolt alone did not say AI). Rendered with Krea 2 (1 MP, `out/fast-harness-icon/` in the workspace: job, render command, cut and size script), cut to a rounded tile. |

## Architecture

| crate | output | content |
|---|---|---|
| `fast-harness-core` | library | types, Claude provider, agent loop, tools, trace and sessions, accounts, config, platform layer |
| `fast-harness` | `fast-harness.exe` | CLI print mode, TUI, `fast-harness accounts`, `fast-harness models`, `fast-harness sessions`, `fast-harness trace` |
| `fast-harness-gui` | `fast-harness-gui.exe` | Slint GUI (added in phase 3B) |

`fast-harness-core` modules, lower never imports higher: `types` > `http`, `sse`, `platform` > `llm`, `tools`, `auth` > `trace`, `session` > `accounts` > `agent`. `events` holds what front ends receive.

- Threads: one UI thread, one worker thread per running session; the tool calls of one step run on scoped threads.
- Agent to UI: `AgentEvent` values over a channel. UI to agent: prompt, cancel, switch account, new session, resume.
- Every traced event is written to the session log before a front end receives it.
- Contracts are code: `http::Transport`, `llm::Provider`, `tools::Tool`, `auth::AuthSource`, `events::AgentEvent`, `platform::*`.

## Files on disk

- `~/.fast-harness/config.json`: model, effort, cache TTL, limits, `theme` (colour theme id), `icons` (TUI icon set: `"nerd"` Nerd Font icons, the default, or `"plain"` Unicode marks) and `wire` (raw wire capture, default `false`). `FAST_HARNESS_MODEL`, `FAST_HARNESS_EFFORT`, `FAST_HARNESS_THEME`, `FAST_HARNESS_ICONS` and `FAST_HARNESS_WIRE` override it; `FAST_HARNESS_HOME` moves `~/.fast-harness`.
- `~/.fast-harness/models.json`: the last model list from `/v1/models`, `{"version":1,"fetchedAt":<unix ms>,"models":[{id, displayName, contextWindow, maxOutput, reasoning, efforts, deprecated, live: {effort, adaptive, budget, canDisable} or null}]}`; written after each successful list (temp file, then rename), read at start; a corrupt file or another version is ignored.
- `~/.fast-harness/accounts.json` (version 3, D26): `{"version":3,"active":<Anthropic id>,"activeGoogle":<Google id, omitted when none>,"accounts":[...]}`; each account has `provider` (`anthropic` or `google`): Anthropic own and API-key accounts, Google accounts (`kind: own`, id `g-<Google user id>`, plan = tier name). Secrets encrypted (DPAPI on Windows): `own` {refresh, access, expiresAt}, `apiKey` {key}, `google` {refresh, access, expiresAt, projectId}. Version 2 is version 3 without `provider` and `activeGoogle`: its records load as `anthropic`. Version 1 also held linked accounts: they are dropped on load (an `active` pointing at one falls back to the first remaining account). The next save writes version 3. Readers accept versions 1, 2 and 3 and reject any other.
- `~/.fast-harness/google-client.json` (D25): `{"version":1,"id":"<client id>","secret":"<base64 of the encrypted secret>"}`, written by `fast-harness accounts google-client --id <id>` (secret from stdin); `FAST_HARNESS_GOOGLE_CLIENT_ID` and `FAST_HARNESS_GOOGLE_CLIENT_SECRET` win over it. Readers reject any other version.
- `~/.fast-harness/sessions/<cwd slug>/<session id>.jsonl`: one log per session (transcript and trace).
- `~/.fast-harness/sessions/<cwd slug>/<session id>.wire/`, only with wire capture on: per model call attempt, the request (headers with credentials redacted, body) and the raw response stream.

## Trace format

One JSON object per line. A header line, then events `{type, seq, time, data}`; `time` is Unix milliseconds, `seq` counts from 0. Event names follow the DSH session log.

```
{"type":"session","version":2,"id":"...","createdAt":1794400565123,"cwd":"F:\\work","harness":"fast-harness 0.1.0","parent":null}
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
- `fast-harness trace [ID|latest] [--steps] [--json] [--cwd DIR]` prints the timeline of the session (`trace::detail`): each turn, what the user sent, the request header (system prompt and tools), each request (model, effort, account, attempt), the model's thinking, response text and tool calls with their arguments, tool results, the answer metrics, retries, failed attempts, account switches and the turn totals; then one row per step: turn, step, account, model, TTFB, total time, input, output, cache read and cache write tokens, tools with their durations, retries. `--steps` prints only the step table, `--json` the raw header and events.

## Claude provider

- `POST https://api.anthropic.com/v1/messages`, `stream: true`, `anthropic-version: 2023-06-01`.
- API key: `x-api-key`. Subscription OAuth: `Authorization: Bearer`, betas `claude-code-20250219` and `oauth-2025-04-20`, and the system prompt sent as two blocks, first "You are Claude Code, Anthropic's official CLI for Claude." (as pi-ai does; other clients depend on this and Anthropic may restrict it).
- Prompt caching: `cache_control` on the system blocks, the last tool and the last message; 5 minutes or 1 hour.

## Accounts

- Own accounts: OAuth sign-in in the browser (PKCE, loopback callback), refreshed by fast-harness; or an API key. These are the only kinds: claude-switch links were removed 2026-11-11 (D8).
- Google accounts (D23): `fast-harness accounts login --google`: authorize `accounts.google.com/o/oauth2/v2/auth` (PKCE S256, `access_type=offline`, `prompt=consent select_account`, loopback `/oauth-callback`), form posts to `oauth2.googleapis.com/token` with the client id and secret (a refresh answer without `refresh_token` keeps the old one), userinfo v1, then `loadCodeAssist` (project, tier: paid, else current, else the default allowed) and `onboardUser` without a project (10 polls, 5 s apart; fallback project `rising-fact-p41fc`). Control calls (`llm/antigravity/hub.rs`): Hub User-Agent from the manifest version (cached 6 h, fallback 2.29.0, minimum 2.23.0), daily host then sandbox on 404, 408, 5xx or a network error. Quota: `retrieveUserQuotaSummary` groups with 5 h and weekly windows; a missing `remainingFraction` is 0, an unused bucket (fraction 1) has no reset. The Anthropic pool never leases, polls or fails over to Google accounts.
- Live usage: response headers `anthropic-ratelimit-unified-5h-utilization`, `-7d-utilization` (fractions), `-5h-reset`, `-7d-reset` (Unix seconds), `-status`, `-representative-claim`; idle accounts read `GET https://api.anthropic.com/api/oauth/usage`.
- Auto switch (D10), on a 429 or quota error: hold the current account until its window resets (15 minutes when unknown); skip held, rejected, Free plan and exhausted accounts; order the rest with claude-switch's `switchOrder`: unread accounts last; accounts whose 7-day window resets within 48 hours first, lowest 7-day percent first among them; otherwise the soonest 7-day reset; then the emptier 5-hour window; then the emptier fullest window. Install the first that works and retry at once; with nobody left, wait for the soonest hold and retry every minute.

## Tools

read, write, edit, shell (PowerShell on Windows, timeout, the whole process tree is killed), grep (regex, `.gitignore` aware), glob, subagent (a nested agent with its own session log; one level deep).

## Rules for the code

See `AGENTS.md`.
