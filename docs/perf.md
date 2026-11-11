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
- Start time of the TUI in a new console window is dominated by the terminal itself (0.7 to 17 s, Windows Terminal start), not by nh.

## Harness time per step (2026-11-11)

`cargo run --release -p nh-core --example step_bench -- <steps>`: one turn of N steps, each a scripted Claude stream (512-byte chunks) asking for the `read` tool on a small file, then a text answer. It covers SSE parsing, stream assembly, the tool, the session log and the wire capture, with no network. Same machine, other builds running.

| Steps | Total | Per step | Session log | Wire capture |
|---|---|---|---|---|
| 51 | 86.6 ms | 1.7 ms | 65 KB | 1.1 MB |
| 201 | 552.1 ms | 2.7 ms | 245 KB | 12.3 MB |

The per-step time grows with the conversation because every request carries the whole conversation and the wire capture stores every request body: its size grows with the square of the step count. A live step's time to first byte from Claude was 1.4 to 2.0 s (below), so the harness adds about 0.1 to 0.2 % to a step.

## Live call (2026-11-11)

`nh -p "Read README.md with the read tool, then reply with its first line only."` on `claude-opus-5-5` with a Claude subscription account (OAuth), release build:

| Step | TTFB | Total | Input | Output | Cache read | Cache write |
|---|---|---|---|---|---|---|
| 1 (tool call) | 1,992 ms | 2,502 ms | 4 | 70 | 0 | 5,704 |
| 2 (answer) | 1,439 ms | 1,445 ms | 2 | 7 | 5,704 | 115 |

Wall time 4.0 s for the turn. `nh.exe` release: 4.98 MB.
