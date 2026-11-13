# Remote plan: one host, thin native clients on every OS

Status: a proposal written 2026-11-13. Nothing here is built and no gate is decided. Section 5 lists the choices; the owner picks, then the phases of section 7 start. Each recommendation is advice, not a decision.

Marks used below: **[read]** means read in this repo at 82a4857 on 2026-11-13; **[web]** means a primary web page opened on 2026-11-13 by the author; **[agent]** means reported by a research subagent with the source it named (section 11) and not opened again by the author; **[unverified]** means nobody confirmed it.

## 1. What the owner asked (2026-11-13, his words)

- R1: "pc này của tao run fast harness, pc thứ 2 rất yếu và không có file gì để làm cả, nhưng có thể connect fast harness của pc1 và nhìn thấy toàn bộ session cũng như UI của fast harness pc1, giống với dsh remote hiện tại trên dsh portal".
- R2: "tao muốn pc2 hay 1 đt android, vẫn có app native fast và không ngốn resource của cpu và ram như hiện tại".
- R3: "có cách để bảo mật giữa các máy muốn remote, kiểu như login bằng account google hay facebook".
- R4: "không tốn nhiều băng thông để duy trì kết nối và xem kết quả ở máy client".
- R5: "đảm bảo phần remote này work tốt trên mọi hệ điều hành nha, windows, linux, macos, android, ios".

## 2. Verdict

It can be built, and the core is already shaped for it: every front end drives one `service::Service`, and the transcript both UIs draw is a UI-independent model fed by events. Four things are missing: a way for several viewers to follow one session, a network listener with a wire format the client can turn back into the same events, login and pairing, and platform backends for every OS but Windows.

The design below sends events, never pixels. The client keeps its own native window and applies the same reducer (`view::Transcript`) the local UI uses, so what it shows is equal to the host by construction, and the traffic is text.

One fact decides how careful the login must be: a remote user can make the agent run shell commands on the host. Remote access is remote code execution on PC1, by design.

## 3. What exists today

| Piece | State | Where |
|---|---|---|
| One turn runner for the GUI, the TUI and `serve` | done (D59) | **[read]** `service/mod.rs`, `service/turn.rs` |
| A turn's events go to exactly one `Sink` (`Box<dyn FnMut(AgentEvent) + Send>`) given at `Service::start` | so a second viewer gets nothing | **[read]** `service/turn.rs:45,71` |
| The GUI builds the agent on its UI thread and hands it over through `GuiHost` | a remote start must pass through the UI thread too | **[read]** `fast-harness-gui/src/host.rs`, `turn.rs:196-239` |
| JSON form of every `AgentEvent`, schema 1 | encoder only, no decoder | **[read]** `service/json.rs:55` |
| Typed log records with serde both ways | done (D61) | **[read]** `trace/record.rs` |
| `Transcript::apply` (live) and `Transcript::from_trace` (replay), held equal by a parity table | done (D61) | **[read]** `view/mod.rs:209`, `view/replay.rs:58` |
| JSON-RPC 2.0 server over any writer, one line per message | stdio is the only transport | **[read]** `service/rpc.rs:17-33`, `args.rs:382` |
| ACP v2 methods: list, new, resume with replay, prompt, cancel, steer, follow-up, pause, continue, approve, config | replay is text and tool calls only | **[read]** `docs/service.md` |
| Sessions of another process: lock file, inbox file, log tail every 250 ms | no live deltas, no approvals | **[read]** `docs/service.md` "Session coordination", D49 |
| HTTPS client with a streaming body and an SSE parser | used for model calls | **[read]** `http.rs` `Transport`, `sse.rs`, `net.rs` |
| Browser sign-in with PKCE and a loopback callback | for Claude accounts | **[read]** `accounts/oauth.rs`, `accounts/oauth/callback.rs` |
| Platform backends | `windows/` only; `unsupported/` elsewhere; `protect` refuses off Windows on purpose | **[read]** `platform/mod.rs` |
| GUI idle cost | private working set 18.1 to 18.2 MB, 31.2 ms of CPU in 30 s | **[read]** `docs/perf.md` phase 24C, 25B |
| Crates in `Cargo.lock` that matter here | present: `rustls` 0.23.46, `rustls-platform-verifier` 0.7.1, `webpki-roots` 1.0.12, `ring` 0.17.14, `httparse` 1.10.1, `http` 1.5.0, `flate2` 1.1.11, `miniz_oxide` 0.8.9; absent: `tungstenite`, `tiny_http`, `sha1`, `data-encoding` | **[read]** `Cargo.lock` (688 packages) |
| Cloudflare Tunnel, Access with Google login, a portal with users, roles and machine assignments | live for DSH on pc1 | **[read]** `repos/dsh-remote/docs/design.md` |

## 4. Target design

```
client: desktop GUI (Windows, Linux, macOS), later TUI, Android, iOS
  remote backend = http::Transport + sse.rs + wire decoder -> Transcript::apply / from_trace
        |  HTTPS: GET /v1/events (text/event-stream)   POST /v1/rpc (JSON-RPC 2.0)
  Cloudflare Access (login, policy per machine) -> Cloudflare Tunnel -> cloudflared on the host
        |  HTTP on 127.0.0.1:<port>, never another interface by default
host: the Fast Harness window, or `fast-harness host` without a window
  remote listener -> device check, capability check, audit -> service::hub -> service::Service -> agent
```

The parts, each tied to a gate of section 5:

- **Hub** (GR1): a new module in the `service` layer. Every event of every session passes through it and gets a sequence number. Subscribers are the local UI (in process) and each remote connection. It keeps the events of the step in flight in a bounded buffer, because everything older is already in the session log; a client that reconnects asks for "everything after sequence N" and gets the buffer, or a "reset" that makes it open the session again. Which events are written to the log and which are live only must be listed first (phase R0): only `ToolProgress` is documented as live only (D58).
- **Wire format** (GR2): live events are the schema 1 objects `service/json.rs` already writes, wrapped as `{"seq","session","event"}`; history is the session's own log records. The client decodes them back to `AgentEvent` and `TraceEvent` and feeds the two existing reducers. A reader skips a `type` it does not know, as `docs/service.md` already rules, so a newer host does not break an older client.
- **What a client subscribes to**: full events only for the session it shows. For every other session it gets state changes only (running, idle, waiting for an approval, title). This is the main bandwidth rule.
- **Coalescing**: the host merges adjacent deltas of the same kind and call for one session and flushes every 50 ms or 4 KB. The transcript is the same; the client wakes and repaints at most 20 times a second, which is what a weak PC2 needs.
- **History in pages**: opening a session returns the newest turns first, sized to one screen and a little more; older turns load when the user scrolls up. The reducer must be shown to start cleanly at a turn boundary (a test in R2).
- **Commands**: the JSON-RPC methods `serve` already has, plus `_fh/open` (history page), `_fh/sessions` (every directory, from the session index), `_fh/workspaces` and `_fh/usage` (names and percentages, never a credential).
- **Commands on the host window**: a remote command is posted to the UI thread and runs the same function as the local button, the way events already arrive there (`turn.rs:227-239`). A turn started remotely appears in the host window, and the reverse.
- **Security at the host**: off by default; a mark in the title bar while on; a list of connected devices with a cut button; a device table of hashed tokens under `<home>/remote/` written through `store::update`; an audit file with one line per connection, pairing and command (method, session, device, never content); five wrong pairing codes void the code. Accounts, tokens, wire capture and settings files are never served.

## 5. Gates: the owner chooses

**GR1. Who owns the sessions on the host.**
- a. The window is the host: remote works only while the Fast Harness window is open on PC1.
- b. A daemon owns every session and each UI, the local one included, is a client of it. Sessions survive a closed window. It reverses "no daemon" (`docs/architecture-plan.md` section 3, D49) and rewrites the local path of both UIs.
- c. The hub lives in the core, so any process that runs a `Service` can host: the window when a setting is on, and `fast-harness host` for a machine without a screen. A session another process runs (a TUI in a terminal, a `-p` worker) is listed with a "limited" mark and followed through the existing lock, inbox and log tail.
- Recommended: **c**. Nobody who leaves remote off pays for it, "no daemon" stays true, and a headless host is the same code.

**GR2. Wire format.**
- a. ACP v2 as `serve --stdio` speaks it. It exists, and outside editors may speak it, but its replay is lossy and the client would need a third reducer next to `apply` and `from_trace`.
- b. Our own events and log records in a JSON-RPC envelope, decoded into the two reducers that exist. Costs a decoder and a round trip test over every `AgentEvent` variant.
- c. The host sends drawn rows or transcript diffs. The client is the simplest, but the traffic grows, a phone layout is decided by the host, and it is a new diff protocol.
- Recommended: **b**; ACP stays as it is for editors.

**GR3. Transport.**
- a. WebSocket with `tungstenite` 0.30.0 (2026-07-11): synchronous, one two-way connection, passes Cloudflare on every plan **[agent]**. A new crate on both sides (it also brings `sha1`, `data-encoding` and others not in the lock today); no compression (permessage-deflate is not supported **[agent]**); a bearer token on the upgrade request is not shown in Cloudflare's Managed OAuth documents **[unverified]**.
- b. Plain HTTPS: one long `GET` answered as Server-Sent Events, and one short `POST` per command. The client needs no new crate: it is `http::Transport` plus `sse.rs`, so it is tested with `ScriptedTransport` like a model call. The host needs a small HTTP/1.1 server (parsing with `httparse`, which the lock already holds through `ureq-proto`). Cloudflare Tunnel streams a response as it arrives when its type is `text/event-stream`, and a silent stream must send something within 125 s **[agent]**; the plan sends a comment line every 25 s. A bearer token on plain HTTP is the documented Managed OAuth path **[web]**.
- c. JSON lines over raw TCP. The least code, but it does not pass Cloudflare's HTTP proxy, so it fits a private network only.
- Recommended: **b**, confirmed by the spike of phase R0 (measured delay of one delta through the tunnel); **a** if the spike shows SSE is buffered or cut.

**GR4. Reach and login.**
- a. Cloudflare Tunnel plus Access, as dsh-remote runs today. Login providers: Google is set up; Facebook, GitHub, one-time PIN and passkeys can be added **[web]**. A native app signs in through Access "Managed OAuth": the system browser opens, the user logs in, the app gets a token that refreshes without asking (default 15 minutes, policy checked again at each refresh) **[web]**. Nothing but our app is installed on the client. Limits: Managed OAuth is an open beta **[agent]**; the free Zero Trust plan has 50 seats **[agent]**; Cloudflare ends TLS at its edge, so it can read the session text **[agent]**.
- b. Tailscale: a private WireGuard network, encrypted end to end, login with Google, Apple, GitHub, Microsoft or a passkey, no Facebook, 6 users on the free plan **[agent]**. The host listens on its tailnet address and our app needs no login code. Costs: the Tailscale app runs on every machine and phone (its memory on PC2 is not measured) and takes the phone's VPN slot.
- c. Our own account system and relay with end-to-end encryption. The best privacy with nothing else installed, and by far the most work and the most risk (our own cryptography).
- Recommended: **a** as the default path, and the same binary accepts a plain address for **b** or a LAN ("direct mode": pairing only, documented as safe only inside WireGuard or a trusted LAN). If session text passing through Cloudflare in the clear is not acceptable, choose b.

**GR5. A second lock at the host (pairing).**
- a. None: whoever passes the login is in.
- b. Each device is paired once: the host window shows a short code valid for 5 minutes, the client sends it, and gets a random 256-bit device token the host stores hashed. Every later request needs the login and the token. A device can be revoked.
- c. As b, and every new connection also waits for a click on the host.
- Recommended: **b**. A stolen Google session or a wrong Access policy alone then runs nothing on PC1.

**GR6. What a remote device may do.**
- a. Everything the local window does, settings and accounts included.
- b. Work with sessions only: list, open, new session in a folder the host already knows, prompt, steer, queue, pause, stop, resume, answer approvals, model and effort of a session. Not served: accounts, settings files, `/trust`, the `approvals` and `allow` settings, panels.
- c. As b, plus a "view only" switch per device.
- Recommended: **c**.

**GR7. The desktop client.**
- a. The same `fast-harness-gui` binary gets a backend seam: local (`Service`) or remote (a host). Hosts appear in the sidebar next to the local workspaces. The TUI can take the same seam later.
- b. A separate thin binary without agent and tool code.
- Recommended: **a**, measured against the budget of section 6; b only if remote mode idles above the local window.

**GR8. The phone UI.**
- a. Slint for Android and iOS with one phone layout. One language, and the view code is shared. Facts to weigh: on Android Skia is the only renderer, so the software renderer of the desktop does not carry over and no memory number is published **[agent]**; Android text input has composition, selection handles and the clipboard through a Java helper, with input bugs fixed as late as 2026-11-04 **[agent]**; iOS runs on the winit backend with Skia **[web]** and has open input and scroll issues from October and November 2026 **[agent]**; Vietnamese Telex is documented nowhere **[unverified]**.
- b. Native UI per platform (Jetpack Compose, SwiftUI) over the Rust core through UniFFI **[agent]**. The best keyboard, accessibility and OS integration; two more UI code bases in two more languages.
- c. A web page served by the host, opened in the phone's browser. No install, no store review, login is the browser flow dsh-remote already proves. It is not a native app (R2), and it costs a browser tab.
- Recommended: **a, decided by a spike** on a real Android phone with these pass marks: Telex typing works in the composer, a long transcript scrolls smoothly, idle memory and streaming CPU are written down and accepted by the owner. If it fails, **b** for phones. c stays available as a stopgap for iOS.

**GR9. iOS distribution.**
- a. TestFlight or internal builds only (100 internal testers, builds last 90 days, Apple Developer Program at 99 USD a year **[agent]**).
- b. The public App Store. Guideline 4.2.7 covers remote clients that mirror specific software: (a) the host must be a user-owned personal computer, (e) "Thin clients for cloud-based apps are not appropriate for the App Store" **[web]**; the same rule asks for host and client on a local network **[agent]**. A client that reaches PC1 over the internet may be refused.
- c. No iOS app; GR8 c for iPhones.
- Recommended: **a**.

**GR10. New crates and two standing rules.**
The plan stays inside "no async runtime". It needs the owner's word on:
- new direct dependencies, each named in its phase with its added package count before it lands: `httparse` (host listener, GR3 b), or `tungstenite` (GR3 a); `flate2` if history pages are compressed; `jni` (Android) and `objc2` crates (iOS, macOS) for the platform backends; `webpki-roots` on phones if the platform verifier's Kotlin part is not wanted **[agent]**;
- G13 of `docs/architecture-plan.md` ("v0.1 is Windows only, the Unix backend after the first tag"): R5 needs the Unix backend, so it moves before the phone work;
- "no daemon": untouched unless GR1 b is chosen.
- Recommended: allow per phase with the count shown; Unix backend as phase R6.

**GR11. Stored secrets off Windows** (the client's refresh token and device token; `protect` refuses off Windows today, D9).
- a. The `keyring` crate family: Windows, macOS, Linux Secret Service, an Android store crate; iOS not confirmed **[agent]**.
- b. Our own backend per OS in `platform/`, as `windows/secret.rs` does: Keychain, Secret Service, Android Keystore through JNI.
- c. Nothing stored off Windows at first: sign in and pair again at each start.
- Recommended: **c** for the first Linux and macOS build, then **a** after its dependency count is known, **b** where a has no backend.

**GR12. When the phone app is not in front.**
Android suspends network in Doze and its documents point to push messages for this **[agent]**; iOS behaviour under suspension is **[unverified]**.
- a. Nothing: on return the client reconnects and resumes from its sequence number.
- b. Push through a small relay (FCM and APNs keys on a Worker): "approval waiting" and "turn ended".
- c. ntfy or UnifiedPush, Android only.
- Recommended: **a** now, **b** as phase R9.

**GR13. Start.** Recommended: R0, then R1 to R4 (Windows to Windows, the owner's first scene), then R5.

## 6. Rules and budgets every phase follows

- The rules of `docs/architecture-plan.md` section 3 and `AGENTS.md` stay: no async runtime, nothing new blocks a UI thread, no panic on input from the network, home files only through `store`, formats change with a version bump.
- The listener binds `127.0.0.1` unless a separate setting names another address.
- Remote off means off: no thread, no socket, no file.
- Implementation chunks run on Opus at high effort, one fresh session per phase, each ending green and committed (owner rule of 2026-11-13).
- Light on RAM, CPU and GPU comes before binary size (owner rule of 2026-11-12). The desktop client keeps the software renderer, so it uses no GPU; the phones run Skia on the GPU **[agent]**, so R0 and R7 also write down GPU use.
- Users extend it without a rebuild (owner rule): every remote switch is a `config.json` key (`remote.enabled`, `remote.port`, `remote.bind`, the coalescing interval, the page size); the wire format is documented in `docs/remote.md` so anyone can write a client; two new hook events, on a device connecting and on a remote command, let a user's hook refuse it or send his own notification (which also gives GR12 a path with no relay); the client's themes are the same JSON files and its screens follow `docs/ui-contract.md`, the phone layout included.
- Remote changes nothing the model sees: the system prompt and the tool list stay frozen per session and the session still starts with `find_tools` only.
- Every screen added here takes its colours from `theme` roles and passes the theme tests (WCAG 2.2, APCA, greyscale).
- Budgets are proposals. A phase measures with the method of `docs/perf.md` and writes the numbers there; a budget that proves wrong is changed here with the measurement next to it.

| Measure | Baseline (`docs/perf.md`) | Budget |
|---|---|---|
| Host window, remote off, idle | private working set 18.2 MB; 31.2 ms CPU in 30 s | unchanged within the noise of two runs |
| Host window, remote on, no client | the same | at most 0.5 MB more; CPU unchanged (the accept thread blocks) |
| Host window, one client watching the flood turn | 328 to 688 ms GUI CPU in 35 s | at most 20% above the baseline runs |
| Desktop client, connected, idle | local window: 18.2 MB, 31.2 ms in 30 s | not above the local window |
| Idle traffic | none exists | one line per 25 s, under 100 bytes of our data |
| Streaming traffic | none exists | our bytes at most 1.3 times the UTF-8 bytes of the text shown |
| Opening a session | logs on this machine: median 17 KB, largest 44 KB (62 logs, short sessions) | first page at most 64 KB on the wire; older turns on demand |
| Phone client | none published **[agent]** | set by the spike of GR8 |

## 7. Phases

Sizes are estimates (S, M, L), not measurements.

| Phase | Content | Gates | Size | Proof it is done |
|---|---|---|---|---|
| R0 | Spikes, throwaway code: (1) a Rust command signs in to a test Access app through Managed OAuth (registration, PKCE, loopback), then reads an SSE stream and sends a POST through the tunnel, with the delay of one delta measured; the same for a WebSocket upgrade; (2) the list of events that are logged and those that are live only; (3) Android hello: a Slint screen with a text input, one HTTPS call to the host, memory and Telex typing on a real phone | GR3, GR4, GR8 | S | A page of numbers in `docs/perf.md` and a yes or no for GR3 b, GR4 a and GR8 a |
| R1 | The hub: sequence numbers, fan-out, the step buffer, subscriptions; the GUI and the TUI take their events from it; a turn started by one subscriber shows for all | GR1 | M | Scripted turns: two subscribers get the same events in the same order; resume after sequence N; idle numbers unchanged |
| R2 | The wire: decoder for schema 1 events, history pages from log records, coalescing, the method table, the capability check; `docs/remote.md` with the format | GR2, GR6 | M | Every `AgentEvent` variant survives encode and decode; a transcript built over the wire equals the local one in the parity table; a page that starts at a turn boundary replays cleanly |
| R3 | The host: listener on loopback, pairing, device table, audit file, view-only switch; the window's Remote page; `fast-harness host` | GR3, GR5 | M | Two real processes over a real socket; malformed and oversized requests end as errors, never a panic; a revoked device is refused at its next request |
| R4 | The desktop client: backend seam in the GUI, hosts in the sidebar, reconnect with backoff and resume, the offline states; token kept with DPAPI | GR7 | L | A host and a client with separate homes on one PC run a scripted turn end to end, approvals included; a real-window pass; the budget table filled in |
| R5 | Reach and login: one Access application and tunnel route per machine, Managed OAuth on, sign-in in the client, a setup script, the direct mode text | GR4 | M | PC2 over the internet: sign in, pair, list, open, prompt, approve; traffic measured against the budget; a machine that is off shows as offline |
| R6 | The Unix backend for Linux and macOS (process group, liveness, interrupt, open, folder, secret per GR11), CI test jobs, release files | GR10, GR11 | L | The R4 test passes on Linux and macOS runners; one real machine of each is used once |
| R7 | The Android client: a phone layout, sign-in through the browser with a custom scheme, token storage, reconnect on return | GR8 | L | On a real phone: list, open, prompt, approve, Telex typing; memory and CPU written down |
| R8 | The iOS client from the same crate: Xcode project, sign-in, Keychain, TestFlight | GR8, GR9 | L | The R7 list on a real iPhone. Needs a Mac and the Apple Developer Program |
| R9 | Notifications, the machine list from the portal, the Trace page over remote, the TUI as a client | GR12 | M | per item |

R1 to R3 change no format on disk except the new files under `<home>/remote/`. R1 touches both UIs, so it ends with a real-window and a real-terminal pass.

## 8. Per OS

| OS | Role | UI and renderer | TLS | Secret store | Sign-in return | Built on | Known risk |
|---|---|---|---|---|---|---|---|
| Windows | host, client | Slint, software renderer (today) | `ureq` + platform verifier (today) | DPAPI (today) | loopback (pattern exists) | this PC | none new |
| Linux | host, client | Slint winit, software renderer; X11 and Wayland **[web]** | platform verifier over the system bundle **[agent]** | GR11 | loopback | CI `ubuntu-latest` (today a compile and test check) | no backend yet; headless machines have no Secret Service |
| macOS | host, client | the same | platform verifier **[agent]** | Keychain | loopback | a Mac or a `macos-latest` runner | Developer ID and notarization for a download **[agent]**; no Mac was ever used for this repo |
| Android | client | Slint android-activity backend, Skia only **[agent]** | platform verifier needs a Kotlin part and a JVM init call, or `webpki-roots` **[agent]** | Android Keystore through JNI **[agent]** | browser plus custom scheme; a small Activity subclass is needed to receive it **[agent]** | this PC with the NDK, `cargo-ndk` and Gradle **[agent]** | memory unmeasured; keyboard bugs; Play needs target API 36, 16 KB pages and an app bundle **[agent]**, a sideloaded APK does not |
| iOS | client | Slint winit backend, Skia **[web]** | platform verifier **[agent]**; `ring` on iOS **[unverified]** | Keychain | browser plus custom scheme, or `ASWebAuthenticationSession` through `objc2` **[agent]** | a Mac with Xcode and xcodegen **[web]** | review rule 4.2.7; open keyboard and scroll issues **[agent]** |

The software renderer the desktop uses draws "western scripts" only **[agent]**; that is today's state on Windows, not a change of this plan, and the phones use Skia.

## 9. Risks and what nobody has confirmed

- Managed OAuth is a beta: it may change or leave the free plan. Fallback: GR4 b, which needs no login code.
- Whether SSE through the tunnel arrives without a visible delay (R0 measures it).
- Whether a Slint phone app is light enough and types Vietnamese well (R0 and GR8).
- `ring` and the `ureq` platform verifier on Android and iOS targets (a build in R0 answers it).
- Which events are live only besides `ToolProgress` (R0 lists them).
- The reducer starting in the middle of a log (R2 tests it).
- iOS behaviour of a suspended socket, and the expiry of a build signed with a free Apple ID.
- The memory of the Tailscale client on a weak PC.
- No Mac and no Android SDK were checked on this workspace; the owner's hardware for R6 to R8 is not known.

## 10. Not proposed, with the reason

- Sending the screen as video (remote desktop): heavy traffic, and a desktop layout on a phone.
- An async runtime, or `iroh`: its whole API is async on tokio **[agent]**.
- A WebView client (Tauri): the memory is the WebView's, against R2.
- Serving accounts or credentials to a client: a client never needs them, the host makes every model call.
- A relay of our own with our own encryption (GR4 c): possible later, wrong as a first step.

## 11. Sources

Read by the author **[web]**: [Cloudflare Managed OAuth](https://developers.cloudflare.com/cloudflare-one/access-controls/applications/http-apps/managed-oauth/) (page updated 2026-11-03), [Access identity providers](https://developers.cloudflare.com/cloudflare-one/integrations/identity-providers/) and [Facebook login](https://developers.cloudflare.com/cloudflare-one/integrations/identity-providers/facebook-login/), [Slint Android guide](https://docs.slint.dev/latest/docs/slint/guide/platforms/mobile/android/), [slint 1.18.1 android module](https://docs.rs/slint/1.18.1/slint/android/index.html), [Slint iOS guide](https://docs.slint.dev/latest/docs/slint/guide/platforms/mobile/ios), [Slint winit backend](https://docs.slint.dev/latest/docs/slint/guide/backends-and-renderers/backend_winit/), [App Review Guidelines 4.2.7](https://developer.apple.com/app-store/review/guidelines/) (search extract).

Reported by research subagents **[agent]**, with the pages they named:
- Transport and login: [tungstenite on crates.io](https://crates.io/crates/tungstenite) and [its issue 2](https://github.com/snapview/tungstenite-rs/issues/2), [ureq Body](https://docs.rs/ureq/latest/ureq/struct.Body.html), [Cloudflare WebSockets](https://developers.cloudflare.com/network/websockets/), [Tunnel troubleshooting (streaming)](https://developers.cloudflare.com/tunnel/troubleshooting/), [connection limits](https://developers.cloudflare.com/fundamentals/reference/connection-limits/), [Managed OAuth announcement](https://blog.cloudflare.com/managed-oauth-for-access/), [validating the Access JWT](https://developers.cloudflare.com/cloudflare-one/access-controls/applications/http-apps/authorization-cookie/validating-json/), [Zero Trust account limits](https://developers.cloudflare.com/cloudflare-one/account-limits/), [Tailscale pricing](https://tailscale.com/pricing) and [encryption](https://tailscale.com/docs/concepts/tailscale-encryption), [iroh](https://docs.rs/iroh/latest), [Google OAuth for native apps](https://developers.google.com/identity/protocols/oauth2/native-app).
- Android: [Slint Android backend sources](https://github.com/slint-ui/slint/tree/master/internal/backends/android-activity), [android-activity README](https://github.com/rust-mobile/android-activity), Slint issues [11810](https://github.com/slint-ui/slint/issues/11810) and [11344](https://github.com/slint-ui/slint/issues/11344), [rustls-platform-verifier](https://docs.rs/rustls-platform-verifier/latest/rustls_platform_verifier/), [Doze and App Standby](https://developer.android.google.cn/training/monitoring-device-state/doze-standby?hl=en), [Play target API](https://support.google.com/googleplay/android-developer/answer/11926878?hl=en), [16 KB pages](https://android-developers.googleblog.com/2025/07/transition-to-16-kb-page-sizes-android-apps-games-android-studio.html), [UniFFI](https://mozilla.github.io/uniffi-rs/latest/).
- Every OS: [Slint backends and renderers](https://docs.slint.dev/latest/docs/slint/guide/backends-and-renderers/backends_and_renderers/), Slint iOS issue 14032 and three more named in the report, [keyring](https://crates.io/crates/keyring), [webbrowser](https://docs.rs/crate/webbrowser/latest), [RFC 8252](https://www.rfc-editor.org/rfc/rfc8252), Apple TestFlight and program pages on developer.apple.com, GitHub runner pricing.

A claim marked **[agent]** is checked again from its page by the phase that depends on it.

## 12. The owner's corrections after the first draft (2026-11-13), not yet folded into sections 4 to 8

His words:
- "app của tao vẫn là harness bình thường, chạy cả trên phone mà, chẳng qua có chức năng remote nữa thôi".
- "Tailscale sao lại chiếm mất slot vpn của điện thoại, không có cách nào để không chiếm slot vpn của điện thoại hả".
- "tao muốn bảo mật nhất, để không bị hack, chứ này production mà bị hack thì tao đền ngập đầu".

What they change, until the sections above are rewritten:

**The phone app is the full harness.** Sections 4, 5 (GR8, GR9) and 7 (R7, R8) describe a remote-only client on phones. That premise was wrong: the phone runs the normal harness with a local and a remote backend, the seam of GR7 a on every OS. A new gate is needed: what the harness runs locally on a phone (model calls and file tools; `shell`, command hooks, stdio MCP servers and native plugins depend on what each OS allows). On iOS guideline 2.5.2 says an app may not "download, install, or execute code which introduces or changes features or functionality of the app" **[web]**. GR9 b said a public listing may be refused under 4.2.7; that rule is written for a remote desktop app that mirrors specific software, and (a) asks for host and client "on a local and LAN-based network" (text from a mirror of the guideline, not the Apple page). How a reviewer treats a remote function inside a full app is not known. TestFlight first stays the advice.

**The VPN slot.** Android runs one VPN connection at a time **[web]**, and Tailscale's own answer is that iOS and Android run only one VPN at a time **[web]**; its phone app is that VPN. Ways that leave the slot free:
- a. No Tailscale on the phone: the app speaks plain HTTPS to a public address (the Cloudflare path, or b).
- b. Tailscale Funnel on the host only: a public `ts.net` HTTPS address, ports 443, 8443 or 10000, TLS only, bandwidth limits that cannot be set **[web]**, and no login in front of it.
- c. A tailnet node inside our app in userspace: `libtailscale` is a C library over Go that works "entirely from userspace" **[web]**, at the cost of a Go runtime in the app (memory not measured); `tailscale-rs` is "preview, experimental" **[web]**.

**Security first, for production.** This replaces the recommendations of GR3 b and the trust GR4 a put in the edge:
- End to end: an inner mutual TLS 1.3 session between the app and the host, with both public keys pinned at pairing. `rustls` 0.23.46 is already in the lock and has client certificates, a server-side client verifier and raw public keys, and `ring` 0.17.14 generates Ed25519 keys **[read]** (crate sources in the cargo registry). The session runs inside the outer connection, so Cloudflare, a relay, Funnel or any network carries ciphertext it can neither read nor change. No cryptography of our own.
- The inner session needs a two-way byte stream, so GR3 moves to a (WebSocket) unless the spike shows two HTTP streams carry it.
- The login (Google, Facebook) stays as the first gate and stops being the root of trust: a stolen login or a taken-over relay alone runs nothing on a host.
- Device keys stay in the OS key store and are not exportable where the OS allows it; a phone asks for its lock (biometric) before remote commands.
- At the host: rights per device, a remote turn can never switch `approvals` off, lockout and rate limits, expiry of devices not seen for a set time, a version floor (the host refuses a client older than a fixed version), the audit file.
- Before production: a written threat model, fuzzing of the listener and the decoder, `cargo audit` in CI, a review by an outside security tester, and a way to ship a security fix fast.
- What no design removes: a paired phone stolen while unlocked, a compromised operating system on either side, and the agent following hostile text it reads (prompt injection). The plan must say how each is limited, not that it cannot happen.

**Open question for the owner.** Is remote for his own machines, or for customers who pair their own PCs and phones? For customers, accounts and a relay that we run become a product of their own, and that is a new gate.

