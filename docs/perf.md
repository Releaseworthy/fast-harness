# Performance notes

## Phase 0 spike (2026-11-11)

Throwaway programs, release builds (`lto = "fat"`, `codegen-units = 1`, `strip = true`): a Slint window with a 200-message list and an input line, a ratatui screen with the same text, and a ureq HTTPS client. Machine: Intel Core i9-12900K, 64 GB RAM, RTX 3080 Ti, Windows 11 Pro 10.0.26300. Two other cargo builds were running at the same time, so the times are upper bounds; phase 4 measures again.

| Program | Binary | Start to exit (median of 5) | Idle memory after 3.5 s |
|---|---|---|---|
| Slint 1.18.1, default renderer (OpenGL, femtovg) | 12.3 MB | 506 ms | working set 97.8 MB, private 191.8 MB |
| Slint 1.18.1, software renderer (`SLINT_BACKEND=winit-software`) | same binary | 153 ms | working set 37.0 MB, private 70.0 MB |
| ratatui 0.30.2 + crossterm | 0.35 MB | 27 ms in an existing console | working set 8.6 MB, private 3.1 MB |
| ureq 3.5.1 + rustls, two `GET /v1/models` | 1.9 MB | 369 ms and 345 ms per call (TLS and network) | |

For comparison, DeepSeek Harness (Electron) ran as 9 processes with 1,680 MB working set in total on the same machine.

Consequences:
- The GUI uses Slint's software renderer by default: about a third of the memory and start time of the OpenGL renderer here. `SLINT_BACKEND` still selects another renderer.
- Start time of the TUI in a new console window is dominated by the terminal itself (0.7 to 17 s, Windows Terminal start), not by fast-harness.

## Harness time per step (2026-11-11)

`cargo run --release -p fast-harness-core --example step_bench -- <steps>`: one turn of N steps, each a scripted Claude stream (512-byte chunks) asking for the `read` tool on a small file, then a text answer. It covers SSE parsing, stream assembly, the tool, the session log and the wire capture, with no network. Same machine, other builds running.

| Steps | Total | Per step | Session log | Wire capture |
|---|---|---|---|---|
| 51 | 86.6 ms | 1.7 ms | 65 KB | 1.1 MB |
| 201 | 552.1 ms | 2.7 ms | 245 KB | 12.3 MB |

The per-step time grows with the conversation because every request carries the whole conversation and the wire capture stores every request body: its size grows with the square of the step count. A live step's time to first byte from Claude was 1.4 to 2.0 s (below), so the harness adds about 0.1 to 0.2 % to a step.

## Live call (2026-11-11)

`fast-harness -p "Read README.md with the read tool, then reply with its first line only."` on `claude-opus-5-5` with a Claude subscription account (OAuth), release build:

| Step | TTFB | Total | Input | Output | Cache read | Cache write |
|---|---|---|---|---|---|---|
| 1 (tool call) | 1,992 ms | 2,502 ms | 4 | 70 | 0 | 5,704 |
| 2 (answer) | 1,439 ms | 1,445 ms | 2 | 7 | 5,704 | 115 |

Wall time 4.0 s for the turn. `fast-harness.exe` release: 4.98 MB.

## Phase 4: the finished binaries (2026-11-11, no other build running)

| Measure | Result |
|---|---|
| `fast-harness.exe` release size (CLI and TUI) | 5.76 MB |
| `fast-harness-gui.exe` release size (Slint, software and OpenGL renderers compiled in) | 19.9 MB |
| `fast-harness --version`, median of 10 | 13.2 ms |
| `fast-harness accounts` (load the store, read DSH's file, print 7 accounts), median of 5 | 13.2 ms |
| `fast-harness tui` idle in its own console after 3.5 s | working set 16.9 MB, private 6.0 MB |
| `fast-harness-gui` window created (cold, then warm) | 311 ms, then 30 to 31 ms |
| `fast-harness-gui` idle after 4 s with 7 accounts | working set 46.0 to 58.4 MB, private 76 MB |
| DeepSeek Harness on the same machine | 9 processes, 1,680 MB working set |

## Phase 10: request-build time (2026-11-12)

`cargo run --release -p fast-harness-core --example step_bench` (no other build running): the 200-step scripted turn, then the Claude request body (`llm::anthropic::request_body`: build and serialise, built-in tools, mean of 50 runs) for a history of the user prompt and alternating `read` tool calls and results (600-byte results).

| Measure | Result |
|---|---|
| 200-step turn, harness time per step | 517 us (total 103.9 ms, session log 265,553 bytes) |
| Request build, 200 messages | 340 us, body 90,052 bytes |
| Request build, 1000 messages | 2,426 us, body 434,052 bytes |

The request build grows with the history (about 2.4 us per message at 1000), so it is most of the 3 ms per-step budget on a long session.

## Phase 21A: GUI resources before and after panels (2026-11-12)

The GUI budgets are RAM, CPU and GPU use, not exe size (owner, 2026-11-12, PLAN.md D46). `cargo build --release -p fast-harness-gui` at d028d5e (before, its own worktree and target dir) and at the panels merge (after). Each run: a fresh empty `FAST_HARNESS_HOME` (no accounts, no sessions, no panels), the window left alone; after 10 s working set private (`Win32_PerfFormattedData_PerfProc_Process.WorkingSetPrivate`), working set and private bytes; then CPU time (`TotalProcessorTime`) delta over the next 30 s; `\GPU Engine(pid_<pid>_*)\Utilization Percentage` sampled 5 times over 10 s inside that window. Three runs each, before and after alternated (script `fh-measure.ps1`, outside the repo).

| Measure | Before (d028d5e) | After (panels) |
|---|---|---|
| `fast-harness-gui.exe` size | 30,428,672 bytes | 30,755,328 bytes (+326,656) |
| Private working set after 10 s idle | 17.9 to 18.1 MB | 18.1 to 18.2 MB |
| Working set after 10 s idle | 44.4 to 56.9 MB | 44.7 to 56.1 MB |
| Private bytes after 10 s idle | 74.7 to 74.9 MB | 74.9 to 75.0 MB |
| CPU time over 30 s idle | 0 to 15.6 ms (one 15.6 ms timer tick in 2 of 3 runs) | 0 ms in 3 of 3 runs |
| CPU time from start to 40 s | 78 to 94 ms | 62 to 94 ms |
| GPU engine use | no `GPU Engine` instance for the pid (no GPU engine used) | the same |

The renderer is Slint's software renderer (the default, `SLINT_BACKEND` unset); `opengl32.dll` is loaded but no GPU engine instance appears, while other processes' instances do. With one panel open: unmeasured, because a panel opens only from Settings > Configuration "Open" or by typing `/panel <id>` in the composer; there is no command-line or environment hook to open one without input.

## Phase 21B: UI files spike, precompiled vs live (2026-11-12)

Question (a), Slint's `live-preview` feature reading a user folder: not by itself. In 1.18.1 the choice is made at build time (`i-slint-compiler` `generator/rust.rs` switches to `rust_live_preview` when `SLINT_LIVE_PREVIEW` is set in the build script's environment) and the generated `MainWindow::new` passes the build machine's absolute path of `ui/main.slint` as a string literal to `LiveReloadingComponent::new`; there is no run-time variable for it. It works with one change: `build/live.rs` replaces that literal with `crate::live::main_file()`. That keeps the typed Rust API (`set_*`, `on_*`, `global::<T>()`), so the whole crate compiled unchanged against it (one missing `use`), where (b), bindings by name with `slint-interpreter` directly, would rewrite every one of the 147 contract uses. Path (a) with the patch is the one shipped (PLAN.md D51); it links `slint-interpreter` all the same (live preview is built on it).

`cargo build --release -p fast-harness-gui` and `cargo build --release -p fast-harness-gui --features ui-files --bin fast-harness-gui-live` at this commit, same script as phase 21A (`fh-measure-21b.ps1`, outside the repo): fresh empty `FAST_HARNESS_HOME`, start to the visible top-level window titled "Fast Harness" (`EnumWindows` + `IsWindowVisible`, so window shown, not first frame), then 10 s idle, then CPU time over 30 s, then `WM_CLOSE`. Two runs each, alternated; the live variant once without `~/.fast-harness/ui` (built-in files staged and compiled once) and once with the folder from `--export-ui` (checked, then compiled again by the preview).

| Measure | `fast-harness-gui` (precompiled) | `fast-harness-gui-live`, no ui folder | `fast-harness-gui-live`, exported ui folder |
|---|---|---|---|
| exe size | 31,363,584 bytes | 29,807,104 bytes | (same exe) |
| start to window shown | 57 to 95 ms | 273 to 688 ms | 601 to 1,189 ms |
| CPU time at window shown | 47 to 78 ms | 219 to 266 ms | 469 to 531 ms |
| private working set after 10 s idle | 18.1 to 18.2 MB | 96.5 to 96.8 MB | 96.7 to 96.8 MB |
| working set after 10 s idle | 44.7 to 44.9 MB | 125.9 to 126.0 MB | 126.0 to 126.3 MB |
| private bytes after 10 s idle | 74.9 MB | 155.9 to 156.0 MB | 156.3 MB |
| CPU time over 30 s idle | 0 ms (2 of 2) | 0 and 15.6 ms | 15.6 and 78.1 ms |

The live variant costs about 79 MB more private memory at idle (the interpreter keeps the compiled document and its type loader) and 3 to 10 times the start time; idle CPU stays near zero (the 78 ms run is unexplained, one run). The precompiled exe now carries the 27 `.slint` sources for `--export-ui` (203,824 bytes of text). `cargo tree -p fast-harness-gui -e normal -i slint-interpreter` without the feature finds no such package: the default binaries never link it. GPU: both use the software renderer; not measured again.
## Phase 24C: live tool output in the GUI (2026-11-13)

Method of phase 21A: `cargo build --release -p fast-harness-gui` at 6aae0c9 (before) and with the live tail (after), copies run alternately twice each on a fresh empty `FAST_HARNESS_HOME`, 10 s idle then CPU time over 30 s. Flood: the real home, Claude Haiku 5.5 at low effort, one turn whose shell runs `1..20000 | ForEach-Object { "line $_ " + ('x' * 100) }` (2,248,908 bytes of output, checked in each spill file), GUI CPU time from Enter over 35 s, sampled by `scripts/window.ps1` (shell child processes not counted).

| Measure | Before (6aae0c9) | After |
|---|---|---|
| Private working set after 10 s idle | 18.1 to 18.2 MB | 18.2 MB |
| Working set after 10 s idle | 43.5 to 56.8 MB | 43.6 to 56.8 MB |
| Private bytes after 10 s idle | 74.4 to 74.5 MB | 74.4 to 74.5 MB |
| CPU time over 30 s idle | 31.2 ms (2 of 2) | 31.2 ms (2 of 2) |
| GUI CPU time, flood turn, 35 s | 422 and 500 ms | 688 and 328 ms |
| Peak working set, flood turn | 54.2 and 56.4 MB | 56.9 and 54.3 MB |

The flood costs about the same (the runs overlap; the shell sends at most 10 chunks a second and the tail keeps 8 lines of 256 bytes, so the work per event is bounded). Before the change the GUI ignored the progress events.

## Phase 25B: the GUI on `service::Service` (2026-11-13)

Method of phase 21A: `cargo build --release -p fast-harness-gui` at e5aa0bb (before) and with the GUI turns in the service (after), copies run alternately twice each on a fresh empty `FAST_HARNESS_HOME`, private working set after 10 s idle, then CPU time over 30 s. Not measured: a finished turn in the transcript, GPU.

| | before | after |
|---|---|---|
| Private working set after 10 s idle | 18.2 and 18.1 MB | 18.1 and 18.1 MB |
| CPU time over 30 s idle | 15.6 and 31.2 ms | 0 and 0 ms |

The service adds no thread or timer at idle: a turn thread exists only while a turn runs, and events reach the UI thread with `slint::invoke_from_event_loop` as before.

## Phase 26A: the session index (2026-11-13)

Method: `cargo run --release -p fast-harness-core --example session_bench -- <logs> <kb>`: N logs of S bytes (turns of about 1 KB user messages) in a temporary home, `session::list` with the index removed (cold, every log parsed) and with a warm index, and `session::latest`, median of 3, warm file cache. Before: finding D1 of `docs/architecture-plan.md` (`fast-harness sessions`, release build, seeded homes, median of 3), same parse of every log as the cold column.

| Logs | before (full parse) | after, cold index | after, warm index | `latest` after |
|---|---|---|---|---|
| 50 x 300 KB | 103 ms | 45.4 ms | 0.7 ms | 0.11 ms |
| 200 x 300 KB | 359 ms | 161.9 ms | 2.6 ms | 0.28 ms |
| 200 x 1 MB | 1,216 ms | 505.8 ms | 2.8 ms | 0.30 ms |

Target met: 200 logs of 1 MB list in 2.8 ms with a warm index (target 50 ms). A warm list reads the folder, stats each log and reads `index.json`; a log that changed costs one parse (about 2.5 ms per MB here). The cold column is lower than "before" mostly because the seeded logs differ (D1's logs held tool calls and results); both parse every log. `latest` reads the folder and at most one log. `fast-harness sessions` itself was not timed.

## Phase 26B: lists and trace reads off the UI threads (2026-11-13)

`cargo run --release -p fast-harness-core --example session_bench -- 200 1024` and `-- 1 10240` (median of 3; the "open" line times the calls a front end makes to open the newest log). UI thread blocked time is the time of the calls made on it, not a frame-gap measurement.

| UI thread work | Before (47a1dc0) | After |
|---|---|---|
| Expand 200 x 1 MB, cold index | `session::list` 498.4 ms | none (worker); render from the received list |
| Expand 200 x 1 MB, warm index | 2.4 ms per expanded workspace, again at each turn start and end | none (worker) |
| Open a 10 MB session | 77.5 ms (reopen, then `trace::read` and the transcript) | 48.9 ms (reopen and the transcript from its events) |
| Open a 1 MB session | 6.2 ms | 2.6 ms |
| Trace page after a turn, 10 MB log | a full read on the UI thread | none (worker) |

Idle, by the method of phase 21A (release builds of 47a1dc0 and of ae31d39, a fresh empty `FAST_HARNESS_HOME`, alternated twice, measured by the parent after the chunk; GPU not measured):

| Measure | Before (47a1dc0) | After (ae31d39) |
|---|---|---|
| Private working set after 10 s idle | 18.2 and 18.1 MB | 18.2 and 18.1 MB |
| Working set after 10 s idle | 56.8 and 43.4 MB | 43.7 and 43.7 MB |
| Private bytes after 10 s idle | 74.4 and 74.3 MB | 74.5 and 74.4 MB |
| CPU time over 30 s idle | 15.6 and 15.6 ms | 0.0 and 0.0 ms |

The workers cost nothing at idle: they run when asked and end (15.6 ms is one tick of the process clock).

## Experiments 2026-11-12 (phase 10, E1 to E6)

Note (PLAN.md D53): the Antigravity measurements below were made on code that now lives on branch `antigravity` (a143cfa), not on main.

Machine: Windows, `~/.fast-harness` with one Google Antigravity account and no Claude account. Release build at 340c916 plus the two measurement tests below.

Live calls: 2 requests in total. The first `-p` run (`antigravity/gemini-3.8-flash-low`, `--cwd` a temp project with a 40,070-byte AGENTS.md, session `20261112-114901-645c`) got on both attempts:

`Auth (HTTP 403): PERMISSION_DENIED (TOS_VIOLATION): This service has been disabled in this account for violation of Terms of Service. Please submit an appeal to continue using this product.`

No further live call was made. Side finding: after the second 403 the Google pool holds the only account (`pool_failover.rs` `on_auth`: one forced refresh, then `GOOGLE_BROKEN_HOLD_MS`) and the turn logs `llm/retry` with `delayMs` 3600000, so `fast-harness -p` waits one hour instead of failing; a `TOS_VIOLATION` refusal is permanent and should end the turn with the message.

- E1, Antigravity `cachedContentTokenCount` (8K+ prompt, 4 turns, after a find, Claude and Gemini models, per-conversation `sessionId` of D36): unmeasured, the account is disabled (above). Whether Antigravity caches implicitly stays unverified.
- E2, Anthropic betas on a subscription account: unverified, no Claude account on this machine.
- E3, steps and tokens of 5 equal tasks with `tools.preload: []` vs `["read","grep"]`: unmeasured, account disabled.
- E4, `shell` time for `echo 1`: `crates/fast-harness-core/tests/shell_echo_timing.rs` (ignored; `cargo test --release -p fast-harness-core --test shell_echo_timing -- --ignored --nocapture`), 10 runs each, `pwsh.exe` 7 on PATH:

| Run | min | median | max |
|---|---|---|---|
| `shell` tool `echo 1` (registry `run`) | 307 to 309 ms | 314 ms | 333 to 341 ms |
| bare `pwsh -NoLogo -NoProfile -NonInteractive -Command "echo 1"` | 205 to 215 ms | 219 to 220 ms | 227 to 230 ms |
| bare pwsh with the tool's UTF-8 prelude | 227 ms | 238 ms | 249 ms |

  Conclusion: about 220 ms is PowerShell start, about 20 ms the prelude (`[Console]::OutputEncoding` and friends), and about 75 ms the tool's own supervision: `supervise` polls every 50 ms (`POLL`) and, once both pipes have closed, sleeps another 50 ms before `try_wait`; the job object and `drain` add the rest (not split further). The 513 ms seen once is not reproduced (cold pwsh start or a loaded machine, unverified). Saving: wait on the child (or a pipe-close signal) instead of the fixed sleep, about 50 ms per call; a long-lived pwsh would save the 220 ms start.

  After the fix (same day, `supervise` checks the child at once when the pipes close, then backs off from 1 ms to `POLL`; cancel and timeout unchanged), same command, 2 release runs of 10:

| Run | min | median | max |
|---|---|---|---|
| `shell` tool `echo 1` (registry `run`) | 317 to 334 ms | 348 to 349 ms | 373 to 399 ms |
| bare `pwsh -NoLogo -NoProfile -NonInteractive -Command "echo 1"` | 245 to 247 ms | 269 to 273 ms | 291 to 299 ms |
| bare pwsh with the tool's UTF-8 prelude | 278 to 287 ms | 301 to 307 ms | 317 to 320 ms |

  The machine was slower than in the first run (bare start median 269 to 273 ms against 219 to 220 ms), so compare the tool against bare pwsh with the prelude: the overhead fell from about 76 ms (314 - 238) to 42 to 48 ms (median). The rest is the job object, `drain` and thread start-up (not split further).

  The `TOS_VIOLATION` side finding is fixed too: the Antigravity provider marks the error "account disabled by the provider", the Google pool holds that account as `Disabled` (the TUI and GUI account rows show "disabled by the provider"; the hold lives in the running process, the account is never deleted) and returns `Fail` instead of an hour-long `Wait` when no other account can take over, so the turn ends with the message.
- E5, GUI time per text delta at 500 transcript items: `crates/fast-harness-gui/src/mirror_bench.rs` (ignored; `cargo test --release -p fast-harness-gui delta_time -- --ignored --nocapture`) applies 200 deltas of 24 bytes through `Transcript::apply` and `Mirror::sync` into a `VecModel<MsgRow>` (the work `turn::deliver` does per event, without `refresh_status`, the Slint layout and the paint): 501 items, new turn (`turn_from` at the last user item): mean 10.2 us, median 10.7 us, p95 16.1 us, max 20.0 us; all 501 items in one turn (`from` 0, every item compared): mean 16.7 us, median 16.7 us, p95 23.2 us, max 30.2 us. Conclusion: the model side is far below a frame (16.7 ms) at 500 items; `sync_rows` per event needs no batching. Render cost per delta: unmeasured (needs a window).
- E6, the synthetic `find_tools` pair (compaction D38, subagent preload D40) on Antigravity Claude with thinking: unmeasured, account disabled. Offline tests only.

No PLAN.md fact was added near D36: E1 did not run, so "Antigravity caches nothing" is not shown.
