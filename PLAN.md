# Fast Harness plan

Fast Harness (this repo; binaries `fast-harness` and `fast-harness-gui`) is a barebone coding agent harness in native Rust, in the spirit of DeepSeek Harness (DSH) but written from scratch (no pi-ai): fast, small, with a trace of every turn and every step. Windows first; the core is portable to Linux, macOS, Android and iOS.

Read [ARCHITECTURE.md](ARCHITECTURE.md) first: it holds the current state. This file keeps the status and the index of decisions; open a decision file when you touch its area.

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
| 7 | Rename to fast-harness (D22), Google Antigravity accounts and provider (D23 to D26), title bar without the account pill (D27), workspaces in the sidebar (D28), robot-and-bolt app icon (D29); live check 2026-11-12 on a real account: Gemini 3.8 Flash and Claude Sonnet 4.6 through Antigravity, text and a tool round trip | done |
| 8 | Chat scroll and long trace text fixes, find tool (D30), system prompt editing (D31), Accounts provider cards (D32), extensible themes with a greyscale setting and a vibrant redesign (D33); live check 2026-11-12: Gemini 3.8 Flash through Antigravity declared only `find_tools`, found grep, then called it; real-window checks with `scripts/window.ps1` | done |
| 9 | Trace rows no longer overlap (workaround for a Slint 1.18.1 layout regression), tool cards with the facts of every tool call in Chat and Trace (D34) | done |

## Decisions

One file per decision in [docs/decisions/](docs/decisions/), moved there unchanged from the table this section held until 2026-11-13. [ARCHITECTURE.md](ARCHITECTURE.md) maps each area to its decisions.

- D1 [Dependencies](docs/decisions/D01.md)
- D2 [Concurrency](docs/decisions/D02.md)
- D3 [HTTP and TLS](docs/decisions/D03.md)
- D4 [JSON](docs/decisions/D04.md)
- D5 [GUI](docs/decisions/D05.md)
- D6 [TUI](docs/decisions/D06.md)
- D7 [Trace](docs/decisions/D07.md)
- D8 [claude-switch import](docs/decisions/D08.md)
- D9 [Token storage](docs/decisions/D09.md)
- D10 [Auto switch](docs/decisions/D10.md)
- D11 [Credentials](docs/decisions/D11.md)
- D12 [v1 scope](docs/decisions/D12.md)
- D13 [Default model](docs/decisions/D13.md)
- D14 [Repo](docs/decisions/D14.md)
- D15 [Themes](docs/decisions/D15.md)
- D16 [Model and effort at the chat box](docs/decisions/D16.md)
- D17 [Live model catalog](docs/decisions/D17.md)
- D18 [System prompt](docs/decisions/D18.md)
- D19 [GUI frame](docs/decisions/D19.md)
- D20 [Icons](docs/decisions/D20.md)
- D21 [Name and app icon](docs/decisions/D21.md)
- D22 [Rename](docs/decisions/D22.md)
- D23 [Google Antigravity accounts](docs/decisions/D23.md)
- D24 [Two providers in one picker](docs/decisions/D24.md)
- D25 [Google OAuth client](docs/decisions/D25.md)
- D26 [Account store v3](docs/decisions/D26.md)
- D27 [Title bar](docs/decisions/D27.md)
- D28 [Workspaces](docs/decisions/D28.md)
- D29 [App icon v2](docs/decisions/D29.md)
- D30 [Find tool](docs/decisions/D30.md)
- D31 [System prompt editing](docs/decisions/D31.md)
- D32 [Accounts as provider cards](docs/decisions/D32.md)
- D33 [Themes as data, greyscale setting, vibrant set](docs/decisions/D33.md)
- D34 [Tool cards with the facts of every call](docs/decisions/D34.md)
- D35 [Measure](docs/decisions/D35.md)
- D36 [Frozen prefix](docs/decisions/D36.md)
- D37 [Output diet](docs/decisions/D37.md)
- D38 [Compaction](docs/decisions/D38.md)
- D39 [Steering, queue, pause, resume](docs/decisions/D39.md)
- D40 [Subagents v2](docs/decisions/D40.md)
- D41 [Settings layers and provider files](docs/decisions/D41.md)
- D42 [Tool, agent and command files](docs/decisions/D42.md)
- D43 [Skills and nested instructions](docs/decisions/D43.md)
- D44 [Hooks](docs/decisions/D44.md)
- D45 [Keys](docs/decisions/D45.md)
- D46 [Panels](docs/decisions/D46.md)
- D47 [MCP, lazy](docs/decisions/D47.md)
- D48 [Sessions as a service (JSON and ACP)](docs/decisions/D48.md)
- D49 [Session coordination](docs/decisions/D49.md)
- D50 [Memory and reflect](docs/decisions/D50.md)
- D51 [UI files](docs/decisions/D51.md)
- D52 [Native plugins and the build workflow](docs/decisions/D52.md)
- D53 [Antigravity moved to branch `antigravity`](docs/decisions/D53.md)
- D54 [Tool effects and run order](docs/decisions/D54.md)
- D55 [Shared files: store module and locks](docs/decisions/D55.md)
- D56 [Accounts under the store lock](docs/decisions/D56.md)
- D57 [Plugin API hardening: types](docs/decisions/D57.md)
- D58 [Tool progress and output parts](docs/decisions/D58.md)
- D59 [One turn runner](docs/decisions/D59.md)
- D60 [Session index](docs/decisions/D60.md)
- D61 [Typed trace records and the transcript parity table](docs/decisions/D61.md)
- D62 [An untrusted project supplies nothing](docs/decisions/D62.md)
- D63 [PreModelRequest hook, hook budget, MCP session expiry](docs/decisions/D63.md)
- D64 [Approvals](docs/decisions/D64.md)
- D65 [Each session keeps its own draft](docs/decisions/D65.md)
- D66 [Cancel of a silent stream](docs/decisions/D66.md)

## Architecture

Now in [ARCHITECTURE.md](ARCHITECTURE.md).

## Files on disk

Now in [docs/formats.md](docs/formats.md#files-on-disk) (short form in [ARCHITECTURE.md](ARCHITECTURE.md#files-on-disk)).

## Trace format

Now in [docs/formats.md](docs/formats.md#trace-format).

## Claude provider

Now in [docs/providers.md](docs/providers.md#claude-provider).

## Antigravity provider

Now in [docs/providers.md](docs/providers.md#antigravity-provider).

## Accounts

Now in [docs/formats.md](docs/formats.md#accounts).

## Tools

Now in [docs/formats.md](docs/formats.md#tools).

## Rules for the code

See `AGENTS.md`.
