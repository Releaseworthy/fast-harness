# Sessions as a service

Other programs drive Fast Harness two ways (PLAN.md D48):

- `fast-harness -p <prompt> --json`: one turn, every event as a JSON line.
- `fast-harness serve --stdio`: a JSON-RPC 2.0 server on stdin and stdout speaking the
  Agent Client Protocol (ACP) v2, with our additions under `_fh/`. Several sessions run at
  once.

Spec pages followed (read 2026-11-12):

- [ACP v2 overview](https://agentclientprotocol.com/protocol/v2/overview)
- [Session setup: new, resume, close](https://agentclientprotocol.com/protocol/v2/session-setup.md)
- [Prompt lifecycle: prompt, updates, states, stop reasons, cancel](https://agentclientprotocol.com/protocol/v2/prompt-lifecycle.md)
- [Tool calls](https://agentclientprotocol.com/protocol/v2/tool-calls.md)
- [Session config options](https://agentclientprotocol.com/protocol/v2/session-config-options.md)
- [Session list](https://agentclientprotocol.com/protocol/v2/session-list.md)
- [JSON-RPC 2.0](https://www.jsonrpc.org/specification)

## `-p --json`

stdout gets one JSON object per line; notes stay on stderr. The first line is
`{"type":"session","schema":1,"session":"<id>","cwd":"<dir>"}`, the last one
`{"type":"result","session","outcome","stopReason","usage","durationMs"}`. Between them, one
line per agent event:

| `type` | Fields |
|---|---|
| `user_input` | `text`, `steer` (a steer or follow-up the turn took) |
| `turn_start` | `turn` |
| `step_start` | `turn`, `step` |
| `request` | `model`, `account`, `attempt` |
| `text` | `text` (a delta of the answer) |
| `thinking` | `text` (a delta of the reasoning) |
| `tool_call_start` | `id`, `name` |
| `tool_args` | `id`, `chunk` |
| `tool_call` | `id`, `name`, `arguments` (raw JSON text) |
| `tool_result` | `id`, `content`, `isError`, `durationMs` |
| `retry` | `attempt`, `delayMs`, `error` |
| `account_switch` | `from`, `to`, `reason` |
| `usage_update` | `account` |
| `subagent` | `session`, `running` |
| `subagent_progress` | `session`, `line` |
| `step_end` | `turn`, `step`, `usage`, `context`, `window`, `timing`, `stop` |
| `compaction` | `summary`, `before`, `after`, `auto` |
| `turn_end` | `turn`, `usage`, `durationMs`, `outcome` |
| `error` | `message` |

`usage` is `{"inputTokens","outputTokens","cacheReadTokens","cacheWriteTokens"}` plus
`cacheWrite1hTokens` when not 0. `outcome` is `done`, `cancelled`, `failed`, `step_limit`,
`truncated`, `context_overflow` or `paused`. A breaking change bumps `schema`.

```text
$ fast-harness -p "say hi" --json
{"type":"session","schema":1,"session":"20261112-094512-1a2b","cwd":"C:\\work"}
{"type":"turn_start","turn":1}
{"type":"step_start","turn":1,"step":1}
{"type":"request","model":"claude-opus-4-6","account":"a1","attempt":1}
{"type":"text","text":"Hi"}
{"type":"step_end","turn":1,"step":1,"usage":{...},"context":4120,"window":200000,...}
{"type":"turn_end","turn":1,"usage":{...},"durationMs":1830,"outcome":"done"}
{"type":"result","session":"20261112-094512-1a2b","outcome":"done","stopReason":"end_turn","usage":{...},"durationMs":1835}
```

(The model and numbers above are illustrative.)

## `serve --stdio`

`fast-harness serve --stdio [--model M] [--effort E]` reads one JSON-RPC message per line on
stdin and writes one per line on stdout; stdout carries nothing else, notes go to stderr. A
request is answered with its `id`, in any order: a turn runs on its own thread, so a second
session's request is answered while the first one works. Input ends: every running turn is
cancelled and the process exits once their last updates are written.

### Methods

| Method | Params | Result |
|---|---|---|
| `initialize` | `protocolVersion` | `{"protocolVersion":2,"capabilities":{"session":{},"_meta":{"fh/methods":[...],"fh/wait":true}},"info":{"name":"fast-harness",...}}` |
| `session/new` | `cwd` (absolute), `mcpServers` (must be empty) | `{"sessionId","configOptions"}` |
| `session/resume` | `sessionId`, `cwd`, `replayFrom` (omitted or `{"type":"start"}`) | `{"configOptions"}`; with replay, the history as updates first, then a `state_update` snapshot |
| `session/list` | `cwd` (optional: else every known directory) | `{"sessions":[{"sessionId","cwd","title","updatedAt","_meta":{"fh/turns","fh/unfinished","fh/open"}}]}`, newest first, one page |
| `session/prompt` | `sessionId`, `prompt` (content blocks), `_meta.fh/wait` | `{"messageId"}`; with `fh/wait` after the turn: `{"messageId","stopReason","_meta":{"fh/outcome","fh/usage","fh/durationMs"}}` |
| `session/cancel` | `sessionId` (a notification) | none |
| `session/close` | `sessionId` | `{}` after the turn, if any, was cancelled and its idle update sent |
| `session/set_config_option` | `sessionId`, `configId`, `type` (`id` or `boolean`), `value` | `{"configOptions"}`, after a `config_option_update` |
| `_fh/steer` | `sessionId`, `text` | `{"queued":true}` while a turn runs (read at the next step boundary); an idle session runs it as a prompt |
| `_fh/follow_up` | `sessionId`, `text` | a new turn after the running one ends `done` (`{"messageId","queued":true}`); idle: a prompt |
| `_fh/pause` | `sessionId` | `{"paused":bool}`: the step's tools finish, the turn ends `_paused` |
| `_fh/continue` | `sessionId`, `_meta.fh/wait` | goes on with a paused, cancelled or cut turn |
| `_fh/compact` | `sessionId`, `instructions`, `_meta.fh/wait` | runs `/compact [instructions]` (D38) |
| `_fh/stats` | `sessionId` | the numbers of `fast-harness stats --json` |

`prompt` blocks: `text`, `resource_link` (sent as a Markdown link) and `resource` with
`text` (sent after its URI); others are refused with -32602. `/help` and the user's
`commands/*.md` work as in print mode: `/help` is answered locally (`agent_message`, then
idle `end_turn`). A `session/prompt` to a session that is running is queued as a follow-up.

Config options: `model` (category `model`, the cached model list plus the current one) and
`effort` (category `thought_level`, `default` plus the model's levels). Any other settings key
of `config.json` (`maxSteps`, `compaction.percent`, `toolOutputBytes`) is accepted too, for
that session only; a string is parsed as JSON when the key does not hold a string. A change
reaches the next turn.

### Updates

Each `session/update` has `params: {"sessionId","update"}` with `update.sessionUpdate`:

| Event | Update |
|---|---|
| prompt accepted | `user_message` (`messageId` `msg_u<index>`), then `state_update` `running` |
| steer or follow-up taken | `user_message` |
| answer delta | `agent_message_chunk` (`messageId` `msg_a<turn>_<step>`, `content` text) |
| reasoning delta | `agent_thought_chunk` (`msg_t<turn>_<step>`) |
| tool call streaming | `tool_call_update` `pending` with `title`, `name`, `kind` |
| tool call runs | `tool_call_update` `in_progress` with `rawInput` |
| tool result | `tool_call_update` `completed` or `failed` with `content` |
| step end | `usage_update` (`used`, `size`) when the window is known |
| compaction | `compaction_update` `completed` with `summary` |
| retry, account switch, error, note | `notice` (`warning`, `info`, `error`) |
| turn end | `state_update` `idle` with `stopReason` (and `error` for `error`) |

Stop reasons: `end_turn`, `cancelled`, `error`, `max_turn_requests` (step limit),
`max_tokens`, and ours `_context_overflow` and `_paused`.

### Errors

-32700 parse error (`id` null), -32600 invalid request (not JSON-RPC 2.0), -32601 unknown
method, -32602 invalid params (relative `cwd`, unknown settings key, client MCP servers, an
unsupported `replayFrom` or content block), -32002 unknown session (ACP), -32010 the session
is running a turn (`_fh/compact`, `_fh/continue`, a replay), -32603 internal.

### Example

```text
> {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":2}}
< {"jsonrpc":"2.0","id":1,"result":{"protocolVersion":2,...}}
> {"jsonrpc":"2.0","id":2,"method":"session/new","params":{"cwd":"C:\\work"}}
< {"jsonrpc":"2.0","id":2,"result":{"sessionId":"20261112-094512-1a2b","configOptions":[...]}}
> {"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{"sessionId":"20261112-094512-1a2b","prompt":[{"type":"text","text":"hi"}]}}
< {"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"...","update":{"sessionUpdate":"user_message","messageId":"msg_u0","content":[{"type":"text","text":"hi"}]}}}
< {"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"...","update":{"sessionUpdate":"state_update","state":"running"}}}
< {"jsonrpc":"2.0","id":3,"result":{"messageId":"msg_u0"}}
< {"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"...","update":{"sessionUpdate":"agent_message_chunk","messageId":"msg_a1_1","content":{"type":"text","text":"Hello"}}}}
< {"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"...","update":{"sessionUpdate":"state_update","state":"idle","stopReason":"end_turn"}}}
```

### Not implemented (differences from ACP v2)

- No `auth/login`; `authMethods` is omitted (accounts come from `fast-harness accounts`).
- No `session/request_permission` (tools run without asking), no terminals, no plans, no
  `session/delete`, no `$/cancel_request` handling, no `systemPrompt` or
  `additionalDirectories`.
- `session/list` returns one page; a `cursor` is refused.
- A prompt sent while a turn runs is answered at once with its queued id; ACP answers when the
  message is inserted. A follow-up the turn did not use (after a cancel or a pause) is reported
  in a `notice` with `_meta.fh/leftover`.
- Replay sends text and tool calls only; assistant ids in a replay (`msg_r<index>`) differ
  from the live ones.

### Library API

`fast_harness_core::service`: `Service::{create, resume, get, close, list, set_config, stats,
start, send_to}` and `Session::{running, send(Input), cancel, wait, with_conv, config}` over a
`Host` (`app::RuntimeHost`).

## Session coordination (no daemon)

Any front end (TUI, GUI, `-p`, `serve`) that runs a turn of a top-level session keeps two
files next to its log `<home>/sessions/<cwd slug>/<id>.jsonl` (PLAN.md D49):

- `<id>.lock`: `{"pid","started","program","id","cwd","since","state","turn","step"}`,
  written when the turns start (a temporary file renamed over it), rewritten at each step
  (`state` is `running`, `pausing` or `stopping`), removed when the turns end. `started` is
  the process creation time, so a reused pid is not taken for the owner. A lock whose process
  is gone is stale: the next reader removes it. A subagent session has no lock.
- `<id>.inbox.jsonl`: one JSON object per line, appended by any process:
  `{"type":"steer","text"}`, `{"type":"follow_up","text"}`, `{"type":"pause"}`,
  `{"type":"stop"}`. The running turn checks the file size once per step boundary, reads the
  new whole lines from its last offset and feeds them to its inbox (D39): a steer joins after
  that step's tool results, a follow-up starts a turn after one that ends `done`, a pause ends
  the turn `paused` after the step's tools, a stop ends it `cancelled` (and stops its
  background subagents) at the boundary. A model call that is streaming is not interrupted.
  When the turns end the file gets a last read (unused texts are the inbox's leftover) and is
  deleted; lines written while no turn runs are read by the next turn.

### Commands

| Command | Does |
|---|---|
| `fast-harness send <ID\|latest> <text> [--steer\|--queue] [--wait]` | A live lock: one inbox line (`steer` by default, `--queue` a follow-up); `--wait` waits for the lock to go and prints the last answer. No live lock: this process runs the turn (answer on stdout; exit 0 when `done`). |
| `fast-harness ps [--json]` | The live locks: id, state, turn, step, pid, program, directory. |
| `fast-harness stop <ID\|latest>` / `pause <ID\|latest>` | A `stop` / `pause` line; exit 1 when the session is not running. |
| `fast-harness wait <ID\|latest>` | Returns when the session has no live lock. |
| `fast-harness tail <ID\|latest> [--json]` | The log as text lines (`-- turn`, `> user`, answers, `[tool]`, `[result]`, `-- turn end`), or the raw events with `--json`, followed every 250 ms while a live lock exists. |
| `fast-harness start [--model M] [--cwd DIR] <prompt>` | Creates a session, prints its id, and runs `fast-harness -p <prompt> --resume <id> --cwd DIR [--model M]` as a detached worker (Windows: `DETACHED_PROCESS` and `CREATE_NEW_PROCESS_GROUP`, outside the caller's Job Object when the job allows breakaway; null stdio). |

`<ID>` is looked up in every directory under `<home>/sessions`; `latest` is the newest session
of `--cwd` (else the current directory).

### Tools for the model

`sessions_list`, `session_start` (`prompt`, `cwd`, `model`: a detached worker, returns the id
at once), `session_send` (`id`, `text`, `mode` `steer` or `queue`; an idle session runs the
text in a worker), `session_read` (`id`: the state and the last answer, cut to the tool output
budget, D37) and `session_stop` (`id`). Each is deferred behind `find_tools` (D30) and comes
with `subagent` in every session below `subagent.maxDepth`. The GUI starts workers with the
`fast-harness` program next to it.

### Limits

- Stop and pause act at step boundaries only; a long model call or tool runs to its end.
- Liveness needs the OS: on Windows `OpenProcess` plus the creation time (a process of another
  user cannot be opened, so its lock looks stale); elsewhere `/proc/<pid>` (Linux), and without
  `/proc` every lock looks stale.
- An idle session that another front end keeps open does not see a turn `send` ran in this
  process until it is reopened.
