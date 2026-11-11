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
