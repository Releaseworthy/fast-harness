# Architecture review and improvement plan (phases 22 to 31)

Written 2026-11-12 at 8ad9053 for the owner's request "review kĩ kiến trúc fast-harness, lên plan improve đi, cải tiến được gì nữa không". It follows `docs/efficiency-plan.md` (phases 10 to 21, done) and the release review of the same day. Nothing here is decided: section 4 lists the gates the owner chooses, and a phase starts only after its gate.

How each fact is known: **[run]** a test or a command run for this review, **[measured]** a timing taken for it, **[read]** the cited lines were read, nothing was run. Unix behaviour was only read.

## 1. Verdict

The base is sound and stays: the module layering holds (one upward import found in the core's `src`), the five contracts are small traits, the system prompt and tool list are frozen per session, the log is append-only and written before a front end sees an event, tools load through `find_tools`, executable project files are gated by trust, the TUI loop is throttled (33 ms frame, 5 s idle wait), the GUI idles at 0 ms CPU over 30 s (`docs/perf.md`), secrets print as `<redacted>`.

Seven weaknesses, in the order they matter:

1. **Tool calls of one step race.** Every call of a step starts at once and `edit` and `write` read and write whole files with no lock: edits are lost while every call reports success.
2. **Files under `~/.fast-harness` are shared by several processes with no lock.** Nine separate temp-then-rename writers, whole-file read-modify-write, save errors dropped.
3. **Three turn runners and two event paths.** `service`, the GUI and the TUI each run turns, queue steers and handle the end of a turn their own way; a transcript is built by one function live and another on replay.
4. **Reading sessions costs the size of every log.** Listing parses each log in full; a resume parses one log three times; both run on the UI thread.
5. **The plugin API freezes internals.** `ext::v1` re-exports the core's own types, none can grow without breaking plugins, and the tool contract has no place for effects, progress or non-text output.
6. **Process-wide registries** hold what belongs to one runtime or one session.
7. **Rules held by convention only**: layering, the 50 and 500 line limits, `version` in files, and a plan file every agent must read that is 88,839 bytes.

## 2. Findings

Severity: H data loss or a broken promise, M wrong or slow under real use, L small.

### A. Tools

| # | Finding | Evidence | Sev |
|---|---|---|---|
| A1 | Eight `edit` calls of one step on one file: all eight report `Replaced 1 occurrence`, one edit is in the file. 40 of 40 rounds, with a 120 byte file and with a 320 KB file. | **[run]** `tests/parallel_edits.rs` (ignored until fixed; fails with `left: 1, right: 8`). Cause: `agent/tool_run.rs:128-141` spawns every call; `tools/edit.rs:51-71` reads then writes. | H |
| A2 | The same race orders nothing: `write` then `shell` on that file in one step run at once. | **[read]** `agent/tool_run.rs:128-141` | H |
| A3 | `write` and `edit` truncate in place (`std::fs::write`): a kill in the middle leaves the user's file cut. | **[read]** `tools/write.rs:50`, `tools/edit.rs:70` | M |
| A4 | The `Tool` contract returns one text and nothing else: no effect (reads, writes, runs), no progress while it runs (shell output shows only at the end), no image or structured part. `Block` has no image either. | **[read]** `tools/mod.rs:64-100`, `types.rs:21-51` | M |
| A5 | `shell` has no background form ("do not start background servers here") and file changes have no undo. | **[read]** `tools/shell.rs:44-48`, `tools/write.rs:37-62` | M |

### B. Shared files and processes

| # | Finding | Evidence | Sev |
|---|---|---|---|
| B1 | `accounts.json`: each process keeps its own copy and saves it whole. A refresh in process A spends the token process B still holds; B then gets `Dead` and holds the account. An account added in A is deleted by B's next save. | **[read]** `accounts/store.rs:208-233`, `accounts/pool/auth.rs:70-88,119-125`, `accounts/pool.rs:133` (per-process mutex). Not reproduced. | H |
| B2 | A failed save after a refresh is dropped (`let _ = self.store.save()`): the new token lives only in memory. | **[read]** `accounts/pool/auth.rs:79-81,123` | H |
| B3 | Nine temp-then-rename writers, no two alike; five use a fixed temp name, so two processes write the same temp file. Only one calls `sync_all`. | **[read]** `config.rs:331`, `config/trust.rs:75`, `agent/prompt_store.rs:115`, `tools/mcp/cache.rs:72`, `session/lock.rs:171` (fixed); `accounts/store.rs:241`, `accounts/pool/snapshot.rs:92`, `workspaces.rs:139`, `app/models.rs:52` (pid) | M |
| B4 | `config.json`, `workspaces.json`, `trusted.json`: read, change, write whole, no lock. `memory/` files are rewritten with plain `fs::write`. | **[read]** `config.rs:316-334`, `workspaces.rs:128-146`, `agent/memory.rs` (3 `fs::write`) | M |
| B5 | Session lock: the check and the take are two calls and the take renames over any lock; `Drop` reads then removes. Two processes starting a turn on one session at the same moment both pass. | **[read]** `agent/mod.rs:215-219`, `session/lock.rs:48-56,129-152,178-185` | M |
| B6 | Off Windows: Linux liveness is `/proc/<pid>` with start time 0 (pid reuse unseen), elsewhere every lock reads stale; no secret store; `kill` reaches the direct child only; no file is set to 0600. | **[read]** `platform/unsupported/liveness.rs:8-13` and siblings; no `set_permissions` in `crates/` | M |
| B7 | A `version` is checked for the trace, accounts, workspaces, keys and themes; `config.json`, providers, tools, agents, commands, skills, mcp and panels have none, `hooks.json` skips a key named `version`, `trusted.json` writes one and never checks it. | **[read]** `tools/hooks.rs:173`, `config/trust.rs`, loaders listed in section 2 E | M |

### C. Front ends and events

| # | Finding | Evidence | Sev |
|---|---|---|---|
| C1 | Three turn runners: `service/turn.rs` (`Service::start` with a sink and a done call), `fast-harness-gui/src/turn.rs` + `runs.rs`, `fast-harness/src/tui/worker.rs` + `steer.rs`. The GUI and TUI never import `service`, though its module text says the GUI reuses it. | **[read]** `service/mod.rs:1-5`; no `service::` in the GUI crate or `tui/` | M |
| C2 | Only the GUI guards a panicking turn thread. In the TUI the turn stays "running" for good; in `serve` the session stays busy. | **[read]** `catch_unwind` only at `gui/turn.rs:212` and `agent/tool_run.rs:213`; `tui/worker.rs:43-58` | M |
| C3 | The same event is written twice by hand at every site (a trace call, then an `AgentEvent`), and a transcript has two builders that must agree: `Transcript::apply` (live) and `Transcript::from_trace` (replay). Log readers reach into `data["key"]` by string in at least six files (`trace/detail.rs`, `trace/report.rs`, `session.rs`, `session/unfinished.rs`, `agent/conversation.rs`, `view/replay.rs`). | **[read]** `agent/call.rs:114-146,264-273`, `view/mod.rs:183-235`, `view/replay.rs:39-110` | M |
| C4 | UI thread I/O in the GUI: the sidebar parses every log of every expanded workspace at each turn start and end; opening a session parses its log four times; the Trace page rereads the whole log after each turn. The TUI lists sessions and reads the trace in a key handler. | **[read]** `gui/workspaces.rs:60-95` called from `gui/turn.rs:192,305`; `gui/app.rs:170-184`; `gui/trace_rows.rs:167-195`; `tui/app.rs:353,362` | M |
| C5 | Copies that belong in core: model and effort picker lists, the model fetch thread, steer and queue merge, state words, footer parts, token and countdown formats. | **[read]** `tui/picker.rs:63-110` and `gui/pickers.rs:15-42`; `tui/steer.rs:68-130` and `gui/runs.rs:83-157`; `gui/app.rs:187-209` and `tui/draw.rs:29-43` | L |
| C6 | Every session's events go to the open panel, also those of a parked session. | **[read]** `gui/turn.rs:242` | L |
| C7 | Each GUI row keeps its text three times (`text`, `shown`, styled `body`); a Trace row keeps `body`, `preview` and `copy`. RAM per row was not measured. | **[read]** `gui/rows.rs:23-36`, `gui/trace_rows.rs:119-138` | L |
| C8 | CLI: a new subcommand changes four places (`USAGE`, `Command`, `parse`, `main`); `parse` is 97 lines; every flag is accepted by every command. | **[read]** `fast-harness/src/args.rs:7-79,325-409`, `main.rs:24-78` | L |

### D. Storage and state

| # | Finding | Evidence | Sev |
|---|---|---|---|
| D1 | `session::list` parses every log in full to show a title and a turn count; `latest` (behind `-c`) does the same to pick one id. 6 ms per MB: 50 logs of 300 KB 103 ms, 200 of 300 KB 359 ms, 200 of 1 MB 1,216 ms. | **[measured]** `fast-harness sessions`, release build of HEAD, seeded homes, median of 3, warm file cache. `session.rs:122-145` | M |
| D2 | A resume parses the log three times (`replay`, `read`, `open_append`), the GUI a fourth. | **[read]** `agent/conversation.rs:102,104,116`, `gui/app.rs:174` | L |
| D3 | One damaged line that is not the last makes a session unreadable (known from the release review). | **[read]** `trace/reader.rs:37-41` | M |
| D4 | Process-wide state: custom providers (never removed, ids leaked for `&'static str`), the live model table, the children of every session id (never pruned), the installed plugins. A trusted project's provider stays for every other workspace of the process; two runtimes in one test process share them. | **[read]** `llm/custom.rs:155-175`, `llm/anthropic/live.rs:68`, `agent/children.rs:49-54` (no `remove` in the file), `ext/mod.rs:53` | M |
| D5 | A cancel is seen only when a chunk arrives; a read may wait 600 s. Before the first byte, and on a provider that sends no pings, Stop does nothing until then. | **[read]** `llm/anthropic/mod.rs:98-101`, `net.rs:18,45` | M |

### E. Extension surface

| # | Finding | Evidence | Sev |
|---|---|---|---|
| E1 | `ext::v1` re-exports `Tool`, `ToolCtx`, `ToolOutput`, `Provider`, `ChatRequest`, `Completion`, `Delta`, `AgentEvent` as they are. No type is `#[non_exhaustive]`: a new field or variant breaks every plugin. | **[read]** `ext/v1.rs:6-12`; no `#[non_exhaustive]` in `crates/` | H before a public tag |
| E2 | `AGENTS.md` (root to cwd, and nested) loads without the trust check every other project file gets. It is text, but it steers a session that has `shell` and no approval step. | **[read]** `agent/prompt.rs:121-143`, `agent/nested.rs`; compare `config/layers.rs:87-93` | M |
| E3 | No hook sees or changes a model request: seven events, none before the provider call. | **[read]** `tools/hooks.rs:22-40` | M |
| E4 | Five loaders repeat read, parse, replace, skip; the name rule exists three times. Reporting is shared. | **[read]** `tools/hooks.rs:114-131`, `tools/mcp/config.rs:130-166`, `panels/file.rs:61-105`, `keys/load.rs:111-153`, `llm/custom.rs:121-146`, `config/index.rs:252-263` | L |
| E5 | Only tool files (`disabled`) and MCP servers (`enabled`) have an off switch; `doctor` lists what loaded, but nothing prints it as JSON. | **[read]** `config/index.rs:224`, `tools/mcp/config.rs:186` | L |
| E6 | Hooks run one after another with 60 s each and no total limit. The order is needed (a hook sees the earlier one's rewrite); the missing part is the limit. | **[read]** `agent/hooks.rs:67-76`, `tools/hooks.rs:18` | L |
| E7 | An MCP HTTP server is never seen dead (`alive` returns `true`), so an expired session id is reused. | **[read]** `tools/mcp/http.rs:195-197` | L |

### F. Guard rails and documents

| # | Finding | Evidence | Sev |
|---|---|---|---|
| F1 | No test or CI step checks the layering, the 50 line function limit or the 500 line file limit. One upward import (`tools` uses `trace::now_ms`); `config`, `events`, `keys`, `net` and `theme` are in no layer of `lib.rs`. About ten functions outside tests are over 50 lines; `run_tools` (73) and `execute` (56) were read. | **[run]** import matrix and a line count script; `tools/mcp/mod.rs:83`, `agent/tool_run.rs:42-114,119-174`, `.github/workflows/ci.yml` | L |
| F2 | `PLAN.md` is 88,839 bytes in 199 lines (decision cells of several thousand characters) and `AGENTS.md` tells every agent to read it first. `http.rs:1-2` still says WinHTTP. | **[run]** file sizes; **[read]** `http.rs:1-2`, `net.rs:1` | L |

Checked and left alone: per-delta model work in the GUI (10 to 17 microseconds at 500 items, `docs/perf.md` E5; the paint cost per delta is still unmeasured), the GUI's 200 ms window timer and 1 s usage loop (idle CPU measured at 0 ms), the TUI redraw loop, MCP servers shared by the sessions of one process (a choice, to be written in `docs/extending.md`).

## 3. Rules every phase follows

- The rules of `docs/efficiency-plan.md` section 3 stay: frozen prefix, append-only messages, results shrink before the first send.
- A bug fix starts from a test that fails first (A1 has one already).
- A contract change lands with every user of it in one commit, and before the first public tag (E1).
- A format change bumps its version and updates the format text in the same commit.
- Nothing new blocks a UI thread: a file read that grows with history runs on a worker.
- No new crate; no async runtime; no daemon.

## 4. Gates: the owner chooses

Each gate: the options, what each costs, and a recommendation. The recommendation is not a decision.

**G5. How the tool calls of one step run (A1, A2).**
- a. A lock per path inside `write` and `edit`. Smallest change; edits all land, but `write` then `shell` still race and plugins' tools are not covered.
- b. Each tool declares its effect (`Read`, `Write`, `Run`). Calls with `Read` run in parallel; the others run one at a time in call order, after the reads before them. Costs a `Tool` method (default `Run`, so an unknown tool is safe) and a rewrite of `execute`.
- c. Everything in call order. Simplest and always right; a step with five `read` or `grep` calls takes the sum instead of the longest.
- Recommended: **b**. It fixes both races, keeps parallel reads, and the same effect word is what an approval mode (G11) or a read-only plan mode needs later.

**G6. Files shared by several processes (B1 to B5).**
- a. One `store` module: `write_atomic` (unique temp name, `sync_all`, rename), `read_versioned` with a stated policy per caller, and `locked(path, || ...)` (a lock file made with `create_new`, stale by the process start time) around every read-change-write, which re-reads inside the lock. The nine writers move to it.
- b. As a, plus one file per account secret, so a refresh never rewrites other accounts. A format change (accounts version 4).
- c. One process owns the files and the others ask it. Rejected before: no daemon.
- Recommended: **a** now; b only if a still shows collisions.

**G7. One turn runner for every front end (C1, C2, C4, C5).**
- a. The GUI and the TUI call `service::Service` in process: `open`, `start(sink, done)`, `send`, `cancel`. What they have and it lacks (parked runs, queued texts shown as chips, the panic guard) moves into it. `serve`, `-p --json`, the GUI and the TUI then share one path.
- b. Extract only the thread body (run, guard, leftover) into a small `agent::runner`; each front end keeps its session table.
- c. Leave it; fix the TUI panic alone.
- Recommended: **a**, in two chunks (TUI first, it is smaller). The four front end files that run turns today hold 864 lines (`gui/turn.rs` 292, `gui/runs.rs` 250, `tui/worker.rs` 204, `tui/steer.rs` 118; how much of that goes is not known before the work), and the pickers, the model fetch and the state words get one home (`app`). It touches both UIs, so each chunk ends with a real-window or real-terminal pass.

**G8. One event model (C3).**
- a. Keep both; add one test table that feeds the same scripted turn to `apply` and to `from_trace` and compares the transcripts.
- b. Typed log records: `trace::Record` (an enum with serde, one variant per event type) replaces `data["key"]` readers; the writer takes a `Record`. No format change.
- c. b, then one reducer: the agent sends `Logged(Record)` after writing it plus the stream deltas, and `Transcript` applies records live and on replay with the same function. `from_trace` and half of `apply` go; `service/json.rs` and `acp.rs` map records instead of events. This changes `AgentEvent`, so it must come with E1.
- Recommended: **a now, b next, c only before the public tag** if E1 is taken; c after the tag is a plugin break.

**G9. Plugin API before the first public tag (E1, A4).**
- a. Freeze `v1` as it is.
- b. Now: `#[non_exhaustive]` on every type `v1` re-exports, constructors instead of struct literals, `Tool::effect()` (G5), a progress call in `ToolCtx` (streamed shell output), and `ToolOutput` holding parts (text now, image later) behind the same constructors.
- c. Call the plugin API unstable for every 0.x release in `docs/plugins.md` and check `api = 1` from `plugins.toml` at build time.
- Recommended: **b and c**. The only plugin in this repo is the template; after a public tag the same change breaks every plugin author.

**G10. Session index (D1, D2).**
- a. `sessions/<slug>/index.json`: per log name its size, mtime, title, turns, last time, unfinished state; a log whose size and mtime match is not opened (the scheme `config/index.rs` already uses for tool files). `latest` picks the highest id from the file names.
- b. No index: read the header line and the last 64 KB of each log. No new file; the title needs the first user message, so two reads per log, and the turn count becomes approximate.
- c. Append a `summary` event at each turn end and read the tail. A trace format change.
- Recommended: **a**, with the lists loaded on a worker thread in both UIs, and one pass for a resume (`replay`, header data and append position from the same parsed events).

**G11. Approval of writes and commands.** The efficiency plan left it out ("not asked"); the release review listed it as opt-in.
- a. Stay with `PreToolUse` hooks only.
- b. Opt-in `approvals`: `off` (today), `ask` (a `Write` or `Run` call waits for the front end's yes), with an allow list of command prefixes and paths in settings. Needs G5 b and one inbox message kind; `-p` and `serve` answer through the same channel or refuse.
- Recommended: decide after G5; **b** is small once tools declare effects.

**G12. `AGENTS.md` of a project that is not trusted (E2).**
- a. Keep: it is text, and PLAN.md D12 says the project's files load.
- b. Same rule as every other project file: nothing loads until `/trust`; the note that already names the untrusted project also names the skipped `AGENTS.md` files.
- c. Load, and show the files once as a note.
- Recommended: **b**: one rule, "an untrusted project supplies nothing".

**G13. Unix (B6).**
- a. v0.1 is Windows only; the README says so and the Linux CI job stays as a compile and test check.
- b. A Unix backend: secret store (Keychain, libsecret or a passphrase file), process group kill, liveness with start time, 0600 and 0700.
- Recommended: **a** for the first tag, b as its own phase after it. b needs the owner's OK for whatever the secret store requires.

**G14. PLAN.md (F2).**
- a. Keep one file.
- b. `ARCHITECTURE.md` (current state only: crates, layers, threads, contracts, files on disk, under 12 KB), `docs/decisions/D01.md` to `D53.md` (moved as they are), and `PLAN.md` keeps the status table and a one-line index of decisions. `AGENTS.md` then says "read ARCHITECTURE.md first, a decision file when you touch its area".
- Recommended: **b**. No behaviour changes; the file every agent session reads first shrinks from 88.8 KB to at most 12 KB.

## 5. Phases

One fresh session per phase, each ending green (`fmt`, `clippy`, `test`, and `scripts/live-smoke.ps1` where AGENTS.md asks for it) and committed. Order: defects, then what a public tag would freeze, then the UI stalls, then the rest.

| Phase | Content | Gate | Proof it is done |
|---|---|---|---|
| 22 | Tool run order by effect; `write` and `edit` through temp file and rename in the same folder; `run_tools` and `execute` under 50 lines | G5 | `parallel_edits` passes un-ignored; a new test: `write` then `shell` reading it in one step; live smoke |
| 23 | `store` module (atomic write, versioned read, lock); accounts refresh and save inside the lock with a re-read; save errors reach the user; session lock with `create_new` | G6 | Two-process test: 2 children refresh and add accounts on one home, no token or account lost; two `Held::take` on one log, one wins |
| 24 | Contract hardening: `#[non_exhaustive]`, constructors, `ToolCtx` progress, `ToolOutput` parts, `api` check in `plugins.toml`; shell output streams to the views | G9 | The plugin template builds unchanged except constructors; a test adds a field behind the constructor without touching the template |
| 25 | TUI on `service::Service`, then the GUI; the panic guard in the service; pickers, model fetch and state words in `app` | G7 | The front end turn threads are gone; a panicking scripted tool turn ends `Failed` in TUI, GUI and `serve`; real terminal and real window pass |
| 26 | Session index; lists on a worker; one-pass resume; the reader skips a damaged middle line with a note | G10 | The section 2 D1 timings again: 200 logs of 1 MB under 50 ms warm; a log with a bad middle line opens |
| 27 | Registries become values: a `Catalog` owned by `Runtime` (custom providers, live models), children dropped when their session closes | none | Two runtimes with different providers in one test process do not see each other's |
| 28 | Event model: transcript parity test, then typed `trace::Record` | G8 | One table drives live and replay; no `data["..."]` outside `trace/` |
| 29 | Extension surface: `version` in every extension file (absent reads as 1), one shared loader and name rule, `enabled` for every kind, `fast-harness ext list --json`, a `PreModelRequest` hook, a total hook budget, MCP HTTP session expiry, G12 | G12 | `ext list --json` shows each loaded file with its source; a hook test rewrites one request |
| 30 | Guard rails: a test for the layering (every module in a layer), a test for the size limits with today's exceptions listed and shrinking, CI runs both; the PLAN.md split | G14 | Both tests in `cargo test`; `ARCHITECTURE.md` under 12 KB |
| 31 | New abilities, each its own yes or no: cancel of a silent stream (D5), background shell jobs and an undo of a turn's file changes (A5), image input (A4), approvals (G11), the Unix backend (G13) | per item | per item |

Phases 22 and 23 touch no format and no contract and can start as soon as G5 and G6 are answered. Phase 24 should land before any public tag; phase 28 c only with it.

## 6. Not proposed, with the reason

- An async runtime: threads and blocking I/O serve every case above; D5 needs one helper thread per call at most.
- A database for sessions: the JSONL log is the format other tools read; an index file gives the speed.
- Batching GUI deltas: the model side is measured far below a frame; measure the paint first.
- Splitting `fast-harness-core` into several crates: the layering test of phase 30 gives the same guarantee without the build and version cost.
- Rewriting the CLI parser as a table (C8): worth doing only when the next subcommand is added.
