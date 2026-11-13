# Efficiency and extensibility plan (phases 10 to 21)

Written 2026-11-12 from a review of the code at commit `e80be14` and of the provider documents read that day. This file is the brief for the sessions that implement it. `PLAN.md` stays the home of decisions and formats: each phase adds its decision rows and format changes there in the same commit.

Owner's goal (2026-11-12): the smallest, lightest, fastest harness; context and caching first; everything that can load lazily loads lazily; and "tao muốn mọi thứ đều custom và chỉnh sửa được" (users extend it themselves, as in DSH, without rebuilding).

## 1. Verdict on the current code

Note (2026-11-12, PLAN.md D53): Google Antigravity left main for branch `antigravity` (commit a143cfa). The Antigravity rows below describe that branch; E1 needs it.

| Area | State | Evidence |
|---|---|---|
| Harness overhead | Good | 1.7 to 2.7 ms per step (`docs/perf.md`); `fast-harness.exe` release 6,687,744 bytes (built 2026-11-12 09:33) |
| HTTP | Good | one pooled `ureq` agent (`net.rs:22-48`) |
| Tool calls of one step | Good | parallel on scoped threads (`agent/tool_run.rs:94`) |
| Claude cache breakpoints | Good | system blocks, `find_tools`, last user block (`llm/anthropic/request.rs:138-185,266`); one moving breakpoint is enough because a run of `tool_use` or `tool_result` blocks counts as one of the 20 lookback positions |
| Frozen prefix | Broken | the system prompt is rendered again for every turn (`app.rs:149`, `agent/prompt.rs:34`) with `{date}`, the AGENTS.md files and the prompt files; a resume renders it again |
| Context control | Missing | history is append-only until the API answers 400; the context window is known (`models.rs:54`) and never used; no compaction |
| Stop reasons | Partial | `max_tokens` ends the turn as `Done` (`agent/turn.rs:139-143`); `pause_turn` and `model_context_window_exceeded` map to `Other` (`llm/anthropic/stream.rs:50-57`) |
| Tool output size | Wasteful | every result up to 32 KB, head and tail (`tools/mod.rs:62`); `read` 2000 lines, `grep` 200 matches, `glob` 200 paths; no spill file |
| Lazy tools | Works, with a cost | `find_tools` is the only loaded tool (D30); the seven deferred tools total 4,180 characters of JSON (the `request/header` of a session logged 2026-11-11), and each session and each subagent spends one extra model step to load them |
| Subagents | Minimal | blocking, same model and effort as the parent, one level (`agent/subagent.rs:64`), no background, no tool preload; `AgentEvent::Subagent` is defined and never emitted |
| Nested subagents | No | the child registry is `tools::builtin()` without `subagent` |
| Session coordination | No | no control protocol, no session-to-session message; print mode is one turn of text; the GUI runs one turn per window (`fast-harness-gui/src/app.rs:32` `busy`) |
| Stop | Yes | cancel flag reaches the stream, the waits and the shell process tree |
| Resume | Yes, with a flaw | log replay and repair of dangling calls (`agent/conversation.rs:48,99`); the system prompt is not the one that was sent |
| Pause, mid-run steering, queue | No | `run_turn` takes one text (`agent/turn.rs:69`); the GUI refuses a send while busy (`fast-harness-gui/src/turn.rs:26`) |
| MCP, skills, hooks, memory, self-improvement | None | no match for these words in `crates/` |
| User extension | Themes, system prompt, `config.json` only | tools and providers are compiled in; `Anthropic::new(base_url)` exists and no setting reaches it |
| Usage numbers | Partial | input, output, cache read, cache write; no context percent, no cache hit rate, no 5 minute and 1 hour split, no cost |
| Antigravity cache | Unknown | all 12 recorded steps show cache read 0 at prompts of 1,142 to 3,454 tokens; `request.sessionId` is one id per process (`llm/antigravity/provider.rs:45`), shared by every session and subagent |

Do not change: the breakpoint layout, thinking and signatures sent back unchanged, the `find_tools` wire shape for Claude, the retry and failover loop, the repair on resume.

## 2. Facts from the provider documents (read 2026-11-12)

- Preserved thinking (Opus 5.5, Sonnet 5.5, Haiku 5.5, Fable and Mythos 5.x): a thinking block stays valid only while `system`, `tools` and every earlier message are unchanged. An edit gives a 400 or a dropped block; `drop_block` is called a stopgap with an output-token cost. A resumed session must replay what was sent, not render it again. Changing context goes into the newest turn or a `role: "system"` message. [preserved-thinking](https://platform.claude.com/docs/en/build-with-claude/preserved-thinking)
- "Shorten a tool result before you first send it, not after": clearing old results on the client is a prefix edit. Same page.
- Client compaction that is recommended: summarize the whole session into one user message and send no old thinking ("simple compaction"). Keeping recent turns with their thinking needs the server (`compact-2026-09-04`). [compaction](https://platform.claude.com/docs/en/build-with-claude/compaction), [on demand](https://platform.claude.com/docs/en/build-with-claude/compaction-on-demand)
- The size of the next request is `input_tokens + cache_read_input_tokens + cache_creation_input_tokens + output_tokens` of the last response: no tokenizer is needed. Same page.
- Cache: hierarchy `tools`, `system`, `messages`; at most 4 breakpoints, free; lookback 20 positions; a 1 hour entry must come before 5 minute entries; `tool_choice` changes invalidate the message cache; a 5 minute write costs 1.25 times the input price, a 1 hour write 2 times, a read 0.1 times on most models; the minimum cacheable prefix is 512 tokens on Opus 5 and 5.5, Sonnet 5.5 and Haiku 5.5, and 1,024 on Sonnet 5. [prompt-caching](https://platform.claude.com/docs/en/build-with-claude/prompt-caching)
- Tool search: "Standard tool calling is a better fit when you have fewer than 10 tools"; "Keep your 3 to 5 most frequently used tools non-deferred". [tool-search-tool](https://platform.claude.com/docs/en/agents-and-tools/tool-use/tool-search-tool)
- MCP 2026-07-28: stateless, no `initialize` handshake, `server/discover`, and `tools/list` results carry `ttlMs` and `cacheScope`, so a client may cache the tool list. [changelog](https://github.com/modelcontextprotocol/modelcontextprotocol/blob/main/docs/specification/2026-07-28/changelog.mdx)
- Agent Skills: `name` up to 64 characters and `description` up to 1024 load at start (about 100 tokens each), the `SKILL.md` body loads when used. [specification](https://agentskills.io/specification)
- Claude Code: subagents nest three layers by default; nested and path-scoped instruction files load when a tool touches their folder; in `auto` mode MCP tools defer once their definitions reach 10% of the window. [sub-agents](https://code.claude.com/docs/en/sub-agents), [context-window](https://code.claude.com/docs/en/context-window), [mcp](https://code.claude.com/docs/en/mcp)
- Pi: steering and follow-up queues over an RPC mode; auto compaction at `contextWindow - reserveTokens` (16,384) keeping 20,000 recent tokens. [rpc-commands](https://pi.dev/docs/latest/rpc-commands), [compaction](https://pi.dev/docs/latest/compaction)
- Agent Client Protocol v2: JSON-RPC with `session/new`, `session/resume`, `session/prompt`, `session/cancel`, `session/update`. [overview](https://agentclientprotocol.com/protocol/v2/overview)
- Slint: the `slint-interpreter` crate (1.18.1 on crates.io, the version of `slint` in `Cargo.toml`) loads a `.slint` file at runtime (`Compiler::build_from_path`); the `live-preview` feature reloads `.slint` files in a running application with the Rust logic connected, and is documented as a debugging and development feature. [slint_interpreter](https://docs.rs/slint-interpreter/latest/slint_interpreter), [live preview](https://docs.slint.dev/latest/docs/slint/guide/tooling/live-preview)
- GitHub Actions: standard GitHub-hosted runners are free in public repositories; a workflow can call a reusable workflow of another repository (`uses: owner/repo/.github/workflows/file.yml@ref`). [billing](https://docs.github.com/en/billing/concepts/product-billing/github-actions), [reuse workflows](https://docs.github.com/en/actions/how-tos/reuse-automations/reuse-workflows)

Unverified: every Anthropic beta above on a subscription (OAuth) account (no Claude account on the review machine); whether Antigravity caches implicitly and what its key is; Gemini minimum cache sizes (from an earlier subagent report, not read here).

## 3. Rules every phase follows

1. Frozen prefix: `system` and the tool list are rendered once per session, saved, and replayed. `messages` is append-only. Anything that changes goes to the tail.
2. Shrink before the first send, never after.
3. Lazy by index: a tool, MCP tool, skill, agent or memory note costs at most one name in the context until it is used.
4. Data, not code: an extension is a file under `~/.fast-harness/` or `<project>/.fast-harness/`; code runs as an external process speaking JSON on stdio. No plugin runtime inside the default binary and no new crate (D1); the UI plugin work the owner asked for (phase 21) keeps the default binaries unchanged.
5. One path that works on every provider first; a provider-only shortcut later, behind a capability flag, after it is measured.
6. Every phase ships its counter and its test, and stays inside the budgets: `fast-harness.exe` at most 8 MB, harness time at most 3 ms per step at 200 steps, TUI idle private memory at most 8 MB, zero work at start for an extension kind that has no files. GUI (owner, 2026-11-12: "size của file exe không quan trọng bằng goal là harness gui chạy nhẹ tài nguyên ram,cpu,gpu"): the budgets are RAM, CPU and GPU use, measured idle and in use (`docs/perf.md`) and held from phase to phase; `fast-harness-gui.exe` size is secondary (a few hundred KB for a feature, up to 600 KB, is acceptable).

Each phase is one fresh session of at most 60 steps: read `PLAN.md`, `AGENTS.md` and its section here; end with `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, the `PLAN.md` rows, one commit.

Order: 10, 11, 12, 13, 14, 15, 17A, 19, 17B, 21A, 18, 16, 20, 21B, 21C. Phases 10 to 13 give most of the token saving.

## 4. Phases

### Phase 10: measure

- `Usage` gains `cache_write_1h` (from `cache_creation.ephemeral_1h_input_tokens`); `StepEnd` and the `assistant/message` event gain `context` (the prompt size plus output) and `window` (the model's context window).
- `message_start.input_transformations` is read and traced as `request/transformations`; a `prefix_binding_mismatch` entry is a bug counter.
- `fast-harness stats [ID|latest|--all] [--json]`: steps, prompt tokens, cache hit rate (`cache_read / prompt`), cache rewrites after an idle gap, bytes of tool results per tool, the five largest results, clipped results, steps spent on `find_tools`, compactions, transformations. Prices come from the model entries of phase 17A; without a price no cost is shown.
- TUI and GUI status: `context 41% of 1M`, cache hit rate of the turn.
- Test `prefix_guard`: for scripted sessions on both providers, request N+1 equals request N in `tools`, `system` and the first messages once `cache_control` is removed. It fails today when the date or an AGENTS.md changes between turns; phase 11 makes it pass.
- `examples/step_bench` adds request-build time at 200 and 1000 messages.
- Experiments, results into `docs/perf.md`: E1 Antigravity `cachedContentTokenCount` on a session above 8K prompt tokens, with a static tool list, after a find, and with a `sessionId` per conversation; E2 which betas a subscription account accepts; E3 steps and tokens of ten equal tasks with and without preloaded tools; E4 `shell` time for `echo 1` (513 ms was seen once); E5 GUI time per text delta at 500 transcript items (`sync_rows` runs on every event, `fast-harness-gui/src/turn.rs:118-122`).

### Phase 11: frozen prefix

- `Conversation` holds `system: Option<String>`, set from `prompt::render` when the session is created and from the last `request/header` on resume (`conversation.rs:59-63` already reads it). `Runtime::agent` uses it; prompt files and AGENTS.md are read only for a new session.
- A change after that (the date passed midnight, an AGENTS.md or prompt file edited, the workspace override toggled) is appended once as a text block ahead of the next user text: `[context update] Today is 2026-11-13.` Tracked by comparing a hash kept in the session header events, so a resume does not repeat it. The GUI prompt editor says "applies to new sessions; running sessions get a note".
- The tool list is frozen the same way: the registry of a session is built once from the tool index of phase 17A and its names and schemas are replayed from the header on resume. A tool added later belongs to new sessions.
- Antigravity `sessionId` derives from the conversation id (E1 decides whether it matters; it costs nothing).
- `Managed` models keep `drop_block` until `stats --all` shows zero `prefix_binding_mismatch`; then the tests send `"error"`.
- Stop reasons: `MaxTokens` ends the turn as a new outcome `Truncated` with a visible note, and continues once automatically when the cut message holds a tool call with incomplete JSON; `model_context_window_exceeded` and a 400 or 413 "prompt is too long" go to phase 13's overflow path; `pause_turn` continues the turn.
- Cache TTL: `cacheTtl` is `"5m"`, `"1h"` or `"auto"` (replaces `cacheLong`, still read). `auto`: 1 hour on the system breakpoint always (it is shared by every session of the workspace), 1 hour on the message breakpoint in the TUI and the GUI, 5 minutes in print mode and subagents. Reason: a 1 hour write costs 0.75x more on each new token once; a 5 minute cache that expires during a pause costs 1.15x of the whole history again. Decided by the owner 2026-11-12 (G3).
- Tests: `prefix_guard` passes across a date change, an AGENTS.md edit and a resume after a prompt file change; a Sonnet 5.5 golden resume sends the logged system text.

### Phase 12: output diet

- One budget per result, set before the first send: default 8 KB (`toolOutputBytes`), per tool override. Over budget, the full text goes to `<session>.out/<callId>.txt` and the result ends with `[full output: <path>, <bytes> bytes, <lines> lines; read it with offset and limit or grep it]`.
- `read`: whole lines from `offset` up to the budget, then `[lines a to b of n; continue with offset b+1]`; no head-and-tail cut. An unchanged range that an earlier call of the session already returned (same path, range, size and modified time) answers `[unchanged since call <id>]`.
- `grep`: default 50 matches, grouped by file, then `[n more matches in m files]`. `glob`: default 100 paths.
- `shell`: 2 KB head and 6 KB tail (errors are at the end), spill file for the rest.
- `subagent`: the answer up to the budget plus the stats line.
- `find_tools`: default 5 stays; the model may pass `limit`.
- Trace: `tool/result` gains `bytes` (full size) and `spilled`. `view::tool_detail` shows the spill path.
- Acceptance: on the phase 10 fixture (a 60-step session with ten large results) prompt tokens at step 60 fall by at least 50%; no result in the log exceeds its budget.

### Phase 13: context control and auto compaction per provider and model

- Settings, resolved per request from defaults, then the provider entry, then the model entry (phase 17A layering; until then `config.json`):

```json
"compaction": {
  "auto": true,
  "trigger": "50%",
  "maxTokens": 500000,
  "reserve": 16000,
  "model": null,
  "effort": "low",
  "instructions": null
}
```

  Compaction starts at whichever bound the context reaches first: `trigger` (a percent of the window or a token count such as `"150k"`), `maxTokens`, or the safety bound `window - reserve - max output of the request`. Defaults decided by the owner 2026-11-12 (G2): 50% of the window, capped at 500,000 tokens, so a 1M model compacts at 500,000 and a 200K model at 100,000. Example overrides: `"models": {"claude-haiku-4-5": {"compaction": {"trigger": "70%"}}, "antigravity/*": {"compaction": {"maxTokens": 120000}}}`. Every step reads the whole context again (at a tenth of the input price when cached, at full price when not); `stats` shows the tokens read per step, so the bounds can be tuned per provider and model.
- When: after a step whose tool results are complete and before the next model call, and at turn start, when `context` of the last step is above the threshold. Never between a call and its result.
- How (simple compaction, the same on every provider): one extra request with the unchanged `system`, tools and messages plus one user message asking for the summary (task, decisions, files touched with paths, current state, next action, durable facts; no tool call). The prefix is read from the cache. `tool_choice` is not set (it would invalidate the message cache); a summary request that returns a tool call is retried once with the tool answered by an error. `compaction.model` may name a cheaper model, at the price of no cache hit.
- The new history is: a user message `[summary of the session so far] ...`; a synthetic `find_tools` call and result naming the tools found so far (so Claude gets its `tool_reference` blocks and Antigravity its declarations again); then, mid-turn, a user message `Continue the task.` No old thinking is sent.
- Log (trace version 4): event `compaction {turn, summary, loaded, before, after, usage, auto}`; `session::replay` starts at the last one; the full transcript stays readable in the views with a "compacted" divider.
- Overflow: a context-size error compacts once and retries the step instead of failing the turn.
- Manual: `/compact [instructions]` in the TUI and the GUI, `fast-harness compact <id>`.
- The "durable facts" part of the summary is appended to the session's memory note (phase 20).
- Later, measured first: Anthropic on-demand compaction for models whose `/v1/models` entry lists `capabilities.compaction`, to keep recent turns with their thinking.
- Tests: threshold arithmetic per model; a scripted session compacts at the threshold and its next request has the three-part history; resume after a compaction; overflow retry; found tools survive.

### Phase 14: steering, queue, pause, resume

- `run_turn` takes an `Inbox` (a channel receiver) next to the cancel flag. Inputs: `Steer(text)`, `FollowUp(text)`, `Pause`.
- `Steer`: read at the step boundary; the text is added as a text block after the tool results of that step (Claude: same user message; Antigravity: same `user` content). Running tools are not aborted. Traced as `user/message` with `kind: "steer"`. Not checked in this review: that Claude accepts a text block after `tool_result` blocks in one user message; read the tool-use page and add a golden test first.
- `FollowUp`: when the turn would end, a new turn starts with it.
- `Pause`: the current step finishes its tools, the model is not called again, the outcome is `Paused`. `Agent::resume_turn` continues a history that ends with tool results without adding a user message. A turn without `turn/end` in the log (a crash or a kill) is offered as "Resume".
- Cancel stays as it is. A cancelled turn can also be resumed: the repaired "interrupted" results are already valid history.
- GUI and TUI: Enter while busy steers, Alt+Enter queues, a Pause button next to Stop; queued texts show as chips that can be removed.
- GUI: running turns are kept per session id, so several sessions run at once; the sidebar marks running, paused and interrupted sessions.
- Tests: scripted transport with a steer between steps, a follow-up, pause then resume, crash resume.

### Phase 15: subagents v2

- Arguments: `prompt`, `description`, `agent` (a type from `agents/*.md`, phase 17A), `model`, `effort`, `tools` (names loaded from the start, so the child needs no find step), `background`.
- Defaults from `"subagent": {"model": null, "effort": null, "maxDepth": 1, "maxParallel": 4, "outputBytes": 8192}`; `null` inherits.
- Nesting: the session header gains `depth`; a child gets the `subagent` tool while `depth < maxDepth`. With the same model, system and tool list as its parent, a child's first request reads the parent's cached prefix.
- Background: `background: true` returns `{id}` at once; the child's answer arrives in the parent's inbox as a steer text when it ends. Tools, all deferred: `agent_send(id, text)`, `agent_stop(id)`, `agent_output(id, wait)`.
- The parent's front end receives `AgentEvent::Subagent {session, running}` and one progress line per child (step and current tool), at most every 500 ms.
- Tests: a child with preloaded tools makes no find call; depth limit; two background children answered in order of completion; cancel reaches every descendant.

### Phase 17A: extension files (settings, providers, tools, agents, commands)

- Layers, later wins: built-in defaults, `~/.fast-harness/`, `<project>/.fast-harness/`, environment, command line. A project folder is read only after the user trusts the project once (`trusted.json`), because it can name commands.
- `providers/*.json`: `{"id", "api": "anthropic-messages" | "openai-chat", "baseUrl", "keyEnv" or an account, "headers", "models": [{"id", "contextWindow", "maxOutput", "efforts", "price": {"input", "output", "cacheRead", "cacheWrite"}, "compaction", "cache": "anthropic" | "implicit" | "none"}]}`. Models are named `<provider id>/<model id>`. `openai-chat` is a new adapter for every OpenAI-compatible endpoint (DeepSeek, OpenRouter, a local server). A model entry may also override a built-in model's numbers.
- `tools/*.json`: a command tool `{"name", "description", "schema", "command", "args", "stdin": "json", "timeoutMs", "outputBytes"}`; the arguments go in on stdin, stdout is the result, a non-zero exit is an error. A file named after a built-in tool overrides its `description`, `outputBytes` or sets `"disabled": true`.
- `agents/*.md` and `commands/*.md`: front matter (`name`, `description`, `model`, `effort`, `tools`, `maxDepth`) and a prompt body; commands are `/name args` in the TUI, the GUI and `-p`.
- `"tools": {"preload": []}`: names loaded from the start. Empty by default, which keeps D30; a user may set it (decided by the owner 2026-11-12, G1; E3 gives the numbers).
- Index: at session creation the folders are listed and only names and descriptions are read into the tool index, cached in `index.json` by path, size and modified time. Bodies are read on use. No folder, no work.
- `fast-harness config path|get|set`, `fast-harness doctor` (what loaded, from where, what was skipped and why). GUI: Settings lists every key with its value and source, and "Open config folder".
- Tests: layering order, an untrusted project loads nothing, a command tool round trip, an override of `read`'s budget, a custom provider against the scripted transport.

### Phase 19: skills and nested instructions

- `skills/<name>/SKILL.md` (Agent Skills format), user and project. The system prompt gets one line: `Skills: a, b, c (load one with find_tools)`, names only, at most 1 KB, then `and n more`. `find_tools` matches skills by name and description and returns the body, at most 20 KB, with the folder path for its other files.
- AGENTS.md: from the repository root to the working directory stays in the frozen system prompt, cap `agentsBytes` (default 32 KB as today). A folder below the working directory that has its own AGENTS.md has it appended (at most 6 KB) to the first tool result that touches a path inside it, once per session; the loaded set is rebuilt from the log on resume and listed in the compaction summary.

### Phase 17B: hooks and keys

- `hooks.json`: events `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `PreCompact`, `Stop`, `SubagentStop`; each entry `{"match": "<tool name pattern>", "command", "timeoutMs"}`. The event goes in as JSON on stdin; the answer may be `{"decision": "block", "reason"}`, `{"arguments": {...}}` (PreToolUse), `{"output": "..."}` (PostToolUse, for a user's own output filter), `{"context": "..."}` (added as a context update), `{"continue": "..."}` (Stop). No hook file, no process spawned.
- `keys.json`: every TUI and GUI action by name with its key.

### Phase 18: MCP, lazy

- `mcp.json`: `{"servers": {"<name>": {"command", "args", "env"} or {"url", "headers"}, "enabled", "alwaysLoad": false}}`. Own client over stdio and HTTP, no new crate: protocol 2026-07-28 first (`server/discover`, stateless requests), the `initialize` handshake for older servers.
- No server starts with a session. The tool index comes from `mcp-cache/<server>.json` (the last `tools/list`, kept for its `ttlMs`, refreshed by `fast-harness mcp refresh` or when a tool of the server is used). A server starts on the first call of one of its tools and stops after 10 idle minutes.
- Names `mcp__<server>__<tool>`; descriptions cut to 1 KB; results use the phase 12 budget.
- Wire: up to 64 tools or 64 KB of definitions ride as deferred tools (Claude) or join after a find (others). Past that cap a server's tools are reached through two deferred tools, `mcp_describe(name)` and `mcp_call(name, arguments)`, so the tool list stays the same size on every provider.

### Phase 16: sessions as a service

- `fast-harness -p --json`: the events of the turn as JSON lines.
- `fast-harness serve --stdio`: JSON-RPC 2.0 with the Agent Client Protocol v2 names where they exist: `initialize`, `session/new`, `session/resume`, `session/list`, `session/prompt`, `session/cancel`, `session/close`, `session/set_config_option`, `session/update`; our additions under `_fh/`: `steer`, `follow_up`, `pause`, `compact`, `stats`. Any other program, or another UI, drives Fast Harness through it.
- Coordination without a daemon: each session has `<id>.lock` (the process that runs it) and `<id>.inbox.jsonl`. `fast-harness send <id> <text> [--steer|--queue] [--wait]` appends to the inbox; a running turn reads it at step boundaries (one file size check per step); when nobody runs the session, `send` runs the turn itself. Also `fast-harness ps`, `stop <id>`, `pause <id>`, `wait <id>`, `tail <id>`, `start --model M --cwd D <prompt>` (a detached worker).
- The same for the model, as deferred tools: `sessions_list`, `session_start`, `session_send`, `session_read` (the last answer, within the budget), `session_stop`.

### Phase 20: memory and self-improvement

- `memory/<cwd slug>/MEMORY.md` (an index of at most 2 KB, one line per note) and `notes/*.md`. The index is a second system block after the stable one, with its own breakpoint, so the stable block stays shared across sessions. No memory tool: the system prompt names the folder and the model uses `read`, `write`, `edit` and `grep` on it.
- Free signals, counted from the logs by `stats --all`: results clipped, repeated reads of one file, `not loaded yet` errors, failed edits, non-zero shell exits by command, steps per turn, compactions, cache rewrites.
- `fast-harness reflect [ID]` and `/reflect`: one subagent on the cheapest configured model reads the stats and a digest of the trace (not the transcript) and proposes edits to memory notes, AGENTS.md lines, skills and tool overrides as a diff; `"selfImprove": "ask" | "auto" | "off"` (default `ask`). Nothing runs by itself at turn end.
- The `Stop` and `PreCompact` hooks let a user plug in another memory system.

### Phase 21: UI plugins

Owner, 2026-11-12: "ui plugin thì bắt buộc người dùng phải build lại hả (nếu phải build lại thì có thể tạo github action chuyên build cho người dùng tạo plugin được không)". Three levels. The first two need no build; the third needs one, and a GitHub Action does it.

21A, panels (no build, in the default binaries):
- `panels/*.json`: `{"id", "title", "icon", "where": "tab" | "sidebar", "command", "args"}`. The command starts when its panel is first opened and speaks JSON lines: it sends a view tree made of a fixed widget set (text, markdown, button, input, select, checkbox, list, table, progress, row, column) and receives the events (click, submit, change) and the session events it subscribes to.
- One generic Slint component compiled into the GUI draws the tree with the theme's roles; the TUI draws the same tree as text. Settings lists every panel with an "Open" action.
- Budget: no process before a panel opens; GUI idle RAM, CPU and GPU use held (measured before and after in `docs/perf.md`). Exe size is secondary: the first budget was 150 KB more in `fast-harness-gui.exe`; the shipped +328,192 bytes was accepted by the owner on 2026-11-12 (PLAN.md D46).
- Tests: a scripted panel process; an unknown widget type shows as a note; a process that dies shows its last stderr line.

21B, `.slint` files (no build, a second GUI binary):
- `fast-harness-gui --export-ui <dir>` writes the built-in `ui/*.slint` files. The variant `fast-harness-gui-live` (cargo feature `ui-files`) loads the UI from `~/.fast-harness/ui/` at start and on Reload, so a user changes any layout, widget or page in a text editor.
- `docs/ui-contract.md` lists every property and callback of `MainWindow` the Rust side uses. A file that does not compile, or misses part of the contract, is reported and the built-in UI is used.
- Spike first, numbers into `docs/perf.md`: (1) a build with Slint's `live-preview` feature, which keeps the typed Rust API; check whether it can read a user folder instead of the source path of the build machine, since Slint documents it for development; (2) otherwise `slint-interpreter` with bindings by name. Measure exe size, cold start and idle memory against the precompiled GUI.
- `slint-interpreter` is a new crate: a D1 row with the spike numbers. `fast-harness-gui.exe` never links it.

21C, native plugins built by a GitHub Action (for what files cannot do: a tool, provider or panel written in Rust, `.slint` components compiled in):
- `fast_harness_core::ext`: a plugin crate exposes `pub fn register(ext: &mut Ext)` and adds tools, providers, in-process hooks and panels. `Ext` carries an API version; a plugin written for another major version fails to compile.
- `plugins.toml` lists the plugins (`name`, `git` or `path`, `rev`); the `build.rs` of a small `fast-harness-plugins` crate generates `register_all` from it. An empty list generates nothing, so the official binaries stay as they are.
- `.github/workflows/build-custom.yml` in this repo, a reusable workflow (`workflow_call` and `workflow_dispatch`) with inputs `ref` (a Fast Harness tag), `plugins`, `ui` (a folder of `.slint` overrides) and `targets`: it checks out Fast Harness at `ref`, writes `plugins.toml`, runs the gates, builds the release binaries, uploads them as artifacts and, on a tag, attaches them to the caller's release.
- A template repository `fast-harness-plugin-template`: one plugin crate, a workflow of about ten lines that calls `build-custom.yml`, a README. A user presses "Use this template", pushes, and downloads the binaries from the run or the release, with no Rust toolchain or MSVC on their machine.
- Standard hosted runners are free for a public plugin repository (section 2); a private one uses its owner's included minutes.
- The same workflow builds the official releases; the repo has no workflow today.
- Not checked: that the `windows-latest` runner has the MSVC toolchain `ring` needs, and how many minutes a release build takes there. The first run answers both.
- Safety: a native plugin runs inside the process with the user's rights; the workflow builds only what the caller's repository lists, pinned by `rev`.

## 5. Owner decisions (2026-11-12)

| Gate | Decision | His words |
|---|---|---|
| G1 | `tools.preload` exists as a setting, empty by default (D30 stays the default); subagents get `tools` | "G1 theo plan" |
| G2 | Compact at 50% of the window or at 500,000 tokens, whichever comes first; per provider and model override | "G2 50% windows đi và capped 500000 token (1 trong 2 cái này cái nào tới trước thì compact ở đó được không)" |
| G3 | `cacheTtl: "auto"` (1 hour for interactive sessions, 5 minutes for print mode and subagents) | "G3 theo plan" |
| G4 | UI plugins in three levels (phase 21): panels and `.slint` files without a build, native plugins built by a GitHub Action | see phase 21 |

Still open: whether `slint-interpreter` enters the optional `fast-harness-gui-live` build, after the 21B spike gives its size and start time.

## 6. Not planned, with the reason

- Clearing or shortening old tool results on the client: a prefix edit (section 2); phase 12 and 13 replace it.
- A JavaScript or WebAssembly plugin runtime in the binary: megabytes and start time for what external commands, MCP and hooks already do.
- A background daemon for coordination: files and locks cost nothing while idle.
- A local tokenizer: the last response's usage is exact.
- An approval and sandbox system: not asked; `PreToolUse` hooks give users their own policy.
