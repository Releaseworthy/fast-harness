# Extending Fast Harness

Everything here is a plain file: edit it and restart (or start a new session), no rebuild. Decisions: [D41](decisions/D41.md) (settings layers, trust, providers) and [D33](decisions/D33.md) (themes, `docs/themes.md`). UI panels drawn by your own program (`panels/*.json`, JSON lines): `docs/panels.md` (D46).

## Settings layers

Settings resolve key by key; a later layer wins:

1. built-in defaults
2. `~/.fast-harness/config.json` (the user layer; `FAST_HARNESS_HOME` moves the folder)
3. `<project>/.fast-harness/config.json` (only in a trusted project, see below)
4. environment: `FAST_HARNESS_MODEL`, `FAST_HARNESS_EFFORT`, `FAST_HARNESS_THEME`, `FAST_HARNESS_GREYSCALE`, `FAST_HARNESS_ICONS`, `FAST_HARNESS_WIRE`
5. command line: `--model`, `--effort` of `fast-harness -p`

Objects merge key by key: a user `"compaction": {"trigger": "40%"}` and a project `"compaction": {"reserve": 1000}` give both. Every key remembers its layer:

```
fast-harness config path                      the files (and whether the project is trusted)
fast-harness config get compaction.trigger    "40%"  (user)
fast-harness config set maxSteps 100          writes ~/.fast-harness/config.json
fast-harness config set maxSteps 50 --project writes <project>/.fast-harness/config.json
fast-harness doctor                           what loaded, from where, what was skipped and why
```

`VALUE` is read as JSON (`100`, `true`, `"text"`, `{"a":1}`), else taken as a string. A dotted key writes inside an object; a model id that contains a dot cannot be addressed that way (edit the file).

`claudeCodeVersion` (user layer, read at start, D71): `"auto"` (default) presents Claude OAuth calls as the newest Claude Code release, checked on npm in the background each time Fast Harness opens and applied to the next call without a restart; a release such as `"2.1.323"` pins it.

`renderer` and `uiScale` (user layer, read when the GUI starts, D75): `renderer` is `"auto"` (default: OpenGL with vsync, the software renderer when OpenGL cannot start), `"software"` (about 200 MB less memory, may tear when scrolling) or `"femtovg"`; `uiScale` (default `1.0`, 0.5 to 4) is the size of the GUI, independent of the Windows display scale. `SLINT_BACKEND` and `SLINT_SCALE_FACTOR` override them.

A broken user `config.json` stops the start with its error, as before. A broken project `config.json` or provider file is skipped, and `fast-harness doctor` names it with the parser's error.

`chatDetail` (D73): `summary` (default) folds each step's tool activity in the GUI chat into one line that opens on a click; `full` shows every thinking block, tool card and step footer. Any other value reads as `summary`. Settings > Appearance switches it without a restart.

## Project trust

The project is the nearest folder at or above the working directory that holds a `.fast-harness` folder (other than `~/.fast-harness` itself). A project folder can name endpoints and, later, commands, so nothing in it loads until you trust it once:

- `fast-harness trust [PATH]` (default: the current directory), or
- type `/trust` in the TUI or the GUI composer.

The list lives in `~/.fast-harness/trusted.json`:

```json
{"version": 1, "paths": ["F:\\work\\my-project"]}
```

Untrusted, the front ends say so once: a note in the TUI and the GUI with the first message, a `[note]` line on stderr in print mode.

An untrusted project supplies nothing (D62), and that includes its `AGENTS.md` files: the ones from the repository root down to the working folder stay out of the system prompt and nested ones are never attached. The note lists their paths. For `AGENTS.md` the project of a folder is its `.fast-harness` project, else its repository root (the nearest folder with `.git`), else the folder itself; `/trust` trusts that folder. A file above a trusted `.fast-harness` project (between it and the repository root) loads only when the repository root is trusted too. After `/trust` a new session has the files in its system prompt; a running session gets them once as a `[context update]` note at its next message. Your own home files (`system-prompt.md`, `prompts/`) are not project files and always load.

## Approvals: `approvals` and `allow`

Off by default. With `"approvals": "ask"` every `write`, `edit`, `shell`, MCP or plugin call with effect `write` or `run` waits for your yes (D64); reads never ask. The allow list lets some run without a question:

```json
{
  "approvals": "ask",
  "allow": {
    "commands": ["git status", "git diff", "cargo test", "cargo check"],
    "paths": ["src", "F:\\work\\my-project\\docs"],
    "tools": ["mcp__github__list_issues"]
  }
}
```

- `commands`: a `shell` command runs when its first words equal a prefix (`cargo test -p core` matches `cargo test`, `cargo testx` does not) and it is one simple command: anything with `;`, `&`, `|`, `(`, `)`, `{`, `}`, `<`, `>`, a backtick, a line break or a non-ASCII character asks.
- `paths`: a `write` or `edit` path runs when it lies under a prefix after `..`, links and (on Windows) case are resolved. A relative prefix is relative to the working folder.
- `tools`: these tools run whatever their arguments.

Only `~/.fast-harness/config.json` may set `allow` or turn `approvals` off: a trusted project may only turn it on. An answer "for this session" covers the same command, the same path or the same arguments until the session closes; nothing is written to the settings. Print mode (`-p`) cannot answer: a call that would ask is refused and the model is told so. Approvals do not guard reads, hooks, MCP server processes or plugin code, and an allowed prefix such as `git` can run more than it looks (`git -c core.pager=...`): keep prefixes narrow.

## `version`, `enabled` and `ext list`

Every extension file shares three rules (`config/ext_file.rs`):

- `version`: `"version": 1` at the top of `config.json`, `hooks.json`, `keys.json`, `mcp.json` and every `tools/`, `providers/` and `panels/` file; `version: 1` in the front matter of `agents/*.md`, `commands/*.md` and `SKILL.md`. Absent reads as 1. A higher number is a file from a newer Fast Harness: it is skipped and `doctor` and `ext list` give the reason. `trusted.json` checks its version too: a newer one trusts nothing and is never rewritten.
- `enabled`: `"enabled": false` (front matter `enabled: false`) turns one file off without deleting it. A tool file's older `"disabled": true` still works; when both are set, off wins. A disabled tool file with the name of a built-in tool still turns that tool off.
- Names: tools, agents, commands, skills, panels and MCP servers use one rule, 1 to 64 letters, digits, `_` or `-`.

`fast-harness ext list [--json] [--cwd DIR]` lists every extension file a session in the folder would see, with the same trust answer: `kind`, `name`, `source` (`home` or `project`), `path`, `state` (`loaded`, `disabled`, `skipped` with `reason`, `shadowed` with `by`, the later file of the same name), and `version`. It reads files only: no MCP server starts and no hook runs. Untrusted `AGENTS.md` files show as kind `agents-md`, skipped. Themes are listed by the Settings page, not here.

## Providers and models: `providers/*.json`

The DeepSeek, OpenRouter and local server examples on this page are unverified live: the adapter is tested only against scripted responses, and model ids, windows and prices must be checked in each service's documentation.

Files in `~/.fast-harness/providers/` and, in a trusted project, `<project>/.fast-harness/providers/`, read in file name order, user folder first. A later file with the same `id` replaces the earlier one.

```json
{
  "id": "deepseek",
  "name": "DeepSeek",
  "api": "openai-chat",
  "baseUrl": "https://api.deepseek.com",
  "keyEnv": "DEEPSEEK_API_KEY",
  "headers": {},
  "models": [
    {
      "id": "deepseek-chat",
      "name": "DeepSeek Chat",
      "contextWindow": 128000,
      "maxOutput": 8192,
      "efforts": [],
      "price": {"input": 0.27, "output": 1.1, "cacheRead": 0.07},
      "compaction": {"trigger": "60%"},
      "cache": "implicit"
    }
  ]
}
```

- `api`: `openai-chat` (any OpenAI-compatible Chat Completions endpoint) or `anthropic-messages` (the Claude Messages API at another `baseUrl`; `headers` are not sent for this api yet).
- `baseUrl`: for `openai-chat`, the part before `/chat/completions`; for `anthropic-messages`, the part before `/v1/messages`.
- The key: `keyEnv` names the environment variable that holds it (the key itself never goes in the file); `"account": "claude"` uses the Claude accounts; neither sends no key (a local server).
- Models are named `<provider id>/<model id>` everywhere: the pickers (one group per provider, titled by `name`), `config.json` `model`, traces and `fast-harness models`. The part after the first `/` goes on the wire.
- `efforts`: the effort words the model takes; the request sends `reasoning_effort` only for one of them.
- `maxOutput` caps the request's `max_tokens`; `contextWindow` feeds compaction and `stats`.
- `price`: US dollars per million tokens. `cacheRead` defaults to `input`; a 5 minute cache write costs `cacheWrite` (default `input`); a 1 hour write costs `cacheWrite1h` (default twice `input`). `fast-harness stats` then shows `cost $x (n of m steps priced)`.
- `compaction`: the D38 settings for this model; an exact `models` entry for the same id in `config.json` wins over it.
- `cache`: `anthropic`, `implicit` or `none`, recorded for later use.

The `openai-chat` adapter streams (`stream: true` with `stream_options.include_usage`), sends tools as functions with only `find_tools` loaded at the start (other tools join after a find names them), assembles streamed tool call arguments, shows `reasoning_content` as thinking (never sent back), counts `prompt_tokens_details.cached_tokens` as cache reads, and maps errors to the usual kinds (a context-length error ends the turn as context overflow).

### Overriding a built-in model

A file whose `id` is `anthropic` (or `antigravity`, reserved for the branch of PLAN.md D53) defines no provider: its model entries change the numbers of built-in models (`contextWindow`, `maxOutput`, `price`, `compaction`). Claude models use their bare id; Antigravity models the id after `antigravity/`.

```json
{"id": "anthropic", "models": [{"id": "claude-opus-5-5", "price": {"input": 5, "output": 25, "cacheRead": 0.5, "cacheWrite": 6.25, "cacheWrite1h": 10}}]}
```

### More examples (unverified live)

These have not been run against the live services; check each endpoint's own documentation for current model ids and prices.

OpenRouter:

```json
{
  "id": "openrouter",
  "api": "openai-chat",
  "baseUrl": "https://openrouter.ai/api/v1",
  "keyEnv": "OPENROUTER_API_KEY",
  "headers": {"X-Title": "Fast Harness"},
  "models": [{"id": "qwen/qwen3-coder", "contextWindow": 262144}]
}
```

The model is then `openrouter/qwen/qwen3-coder`: only the first `/` separates the provider.

A local server (Ollama, llama.cpp, LM Studio, vLLM), no key:

```json
{
  "id": "local",
  "api": "openai-chat",
  "baseUrl": "http://127.0.0.1:11434/v1",
  "models": [{"id": "qwen3:8b", "contextWindow": 32768, "price": {"input": 0, "output": 0}}]
}
```

## Tools: `tools/*.json`

Every tool but `find_tools` is deferred (D30): a tool file costs one name in `find_tools`' description until the model finds it.

### Command tools

A file with a `command` is a new tool. The arguments go in on stdin as one JSON object, stdout is the result (cut to the tool's byte budget, the rest spilled to a file like any tool), a non-zero exit is an error that carries the exit code and the last 2,000 bytes of stderr. On timeout or cancel the whole process tree is killed.

`~/.fast-harness/tools/word_count.json`:

```json
{
  "name": "word_count",
  "description": "Count the words of a text.",
  "schema": {"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]},
  "command": "pwsh",
  "args": ["-NoProfile", "-Command", "($input | ConvertFrom-Json).text.Split(' ', [StringSplitOptions]::RemoveEmptyEntries).Count"],
  "stdin": "json",
  "timeoutMs": 30000,
  "outputBytes": 2048
}
```

- `name`: 1 to 64 letters, digits, `_` or `-`; the file name without `.json` when missing.
- `description` (required for a command tool) and `schema` (a JSON Schema object; `{"type":"object"}` when missing).
- `command`: a program on `PATH`, an absolute path, or a path with a `/` or `\` relative to the tool file's folder (`./bin/tool.exe`). `args` follow it as given (no shell).
- `stdin`: `json` (default) or `none`. The process runs in the session's working directory with `FAST_HARNESS_TOOL_DIR` set to the tool file's folder.
- `timeoutMs`: default 120000. `outputBytes`: this tool's result budget (a `toolOutput` entry in `config.json` wins).
- `effect` (PLAN.md D54): `run` (default), `write` or `read`. The calls of one step run by effect: consecutive `read` calls run at once, every other call runs alone, in call order, after the calls before it. Say `read` only for a tool that changes nothing (no files, no processes left behind). Any other word skips the file with a reason (`fast-harness doctor`).

### Overriding a built-in tool

A file without `command`, named after a tool (`read`, `write`, `edit`, `shell`, `grep`, `glob`, `subagent`, `agent_send`, `agent_stop`, `agent_output`), changes that tool:

```json
{"name": "read", "outputBytes": 16384}
```

```json
{"name": "shell", "disabled": true}
```

```json
{"name": "glob", "description": "Find files by glob pattern; prefer this over shell dir listings."}
```

## Agent types: `agents/*.md`

Front matter and a body. The subagent tool's `agent` argument names the type; `find_tools`' description lists the type names (at most about 1 KB, then "and n more").

```markdown
---
name: reviewer
description: Reviews a change for bugs
model: claude-sonnet-5
effort: medium
tools: [read, grep, glob]
maxDepth: 1
---
You review code. Report bugs with file and line; do not edit files.
```

The body opens the child's first user message, followed by the `prompt` of the call; the system prompt stays the parent's, so the child shares the parent's cached prefix. `model`, `effort` and `tools` (preloaded, no find step) apply unless the call passes its own; `maxDepth` can only lower `subagent.maxDepth`.

## Commands: `commands/*.md`

`/name args` in the TUI, the GUI and `fast-harness -p "/name args"` sends the body with `$ARGUMENTS` replaced by `args` (without the placeholder, the arguments follow the body after a blank line). `/help` lists the built-in commands (`/compact`, `/trust`, `/help`, which a file cannot replace) and yours.

```markdown
---
description: Explain a file
---
Read $ARGUMENTS and explain what it does in five lines.
```

## Skills: `skills/<name>/SKILL.md`

A skill is a folder in the [Agent Skills](https://agentskills.io/specification) format: `SKILL.md` with front matter `name` (1 to 64 letters, digits, `-` or `_`; the folder name when missing) and `description` (needed, cut at 1024 characters), then the instructions. Other files of the folder (`scripts/`, `references/`, `assets/`) are read by the model with the normal tools when the instructions point to them. Other front matter keys (`license`, `compatibility`, `metadata`, `allowed-tools`) are ignored.

```markdown
---
name: pdf-tools
description: Extract text from PDF files and fill PDF forms. Use when a task mentions PDFs.
---
Run `pdftotext -layout file.pdf -` and read the result; forms: see references/forms.md.
```

- The system prompt of a new session gets one line, `Skills: a, b, c (load one with find_tools)`: names only, at most 1 KB of names, then `and n more`. The line is part of the frozen system prompt: a skill added or removed later does not change a running session's prompt and sends no context update, but `find_tools` matches it as soon as the settings are reloaded.
- `find_tools` matches skills by exact name in `names` and by `query` (the best match scoring a name hit or two description words). The result keeps its `Loaded: ...` line for tools and adds, per skill (at most 3 per call), a block starting `[skill <name>: <folder>]` with the body after the front matter, cut at 20 KB. On Claude a result that loaded tools holds only `tool_reference` blocks (the API rejects any other content beside them), so the body follows as a text block after the results of that message, opened by `[find_tools result, continued]`. This result is not cut by `toolOutputBytes`.
- Skills loaded so far are rebuilt from the session log (the `[skill ...]` lines), and a compaction summary lists them.
- Untrusted projects load no skills; a project skill replaces a user skill of the same name.

## Nested AGENTS.md

The AGENTS.md files from the repository root down to the working directory are in the system prompt, at most `agentsBytes` (default 32768) together. A folder below the working directory with its own `AGENTS.md` has it appended, at most 6 KB, to the first tool result whose `path` argument (read, write, edit, grep, glob, and any command tool with a `path`) lies inside it, once per session. The shell has no working-directory argument, so its calls load nothing. The text joins the result after the `toolOutputBytes` cut (outside the budget) and before the result is sent or logged, under a line `[nested AGENTS.md <path>]`; the loaded set is rebuilt from the log on resume, and a compaction summary lists those lines so the files are not appended again.

## Hooks: `hooks.json`

`~/.fast-harness/hooks.json` and a trusted project's `.fast-harness/hooks.json` run your own programs at eight points of the agent loop. Both files load and both run, user entries first; an untrusted project's file is not read. Without a file nothing is spawned and nothing else is read. The file is read with the other settings (at start, `/trust`, a settings reload); a change applies to the next agent built from them.

```json
{
  "PreToolUse": [
    {"match": "shell", "command": "python", "args": ["./guard.py"], "timeoutMs": 5000}
  ],
  "PostToolUse": [
    {"match": "shell|mcp__*", "command": "python", "args": ["./filter.py"]}
  ]
}
```

- Events: `SessionStart` (first turn of a new session), `UserPromptSubmit` (each user prompt, before the model sees it), `PreToolUse`, `PostToolUse`, `PreCompact`, `Stop` (the model ended the turn), `SubagentStop` (the same, in a subagent session, instead of `Stop`), `PreModelRequest` (before each step's model call, below).
- Entry: `command`, `args`, `timeoutMs` (default 60000), `match`. `match` is for `PreToolUse` and `PostToolUse` only: the tool name, `|` between alternatives, `*` any run of characters, `?` one character; absent matches every tool. A relative `command` with a slash (`./guard.exe`) resolves in the folder of the hooks file; the process runs in the working directory with `FAST_HARNESS_HOOKS_DIR` set to that folder. A timeout or cancel kills the whole process tree.
- Stdin: one JSON object with `event`, `sessionId`, `cwd` and the event data: `prompt` (UserPromptSubmit), `source` (SessionStart), `toolName` and `arguments` (tool events), `output` and `isError` (PostToolUse), `trigger` (`auto` or `manual`), `instructions` and `summaryRequest` (PreCompact), `answer` and `stopHookActive` (Stop, SubagentStop).
- Stdout: empty does nothing; else a JSON object with any of:
  - `{"decision": "block", "reason": "..."}`: a tool call's result becomes the error `blocked by a hook: <reason>`; a prompt does not run (the turn ends failed and the front end shows the reason); a compaction is skipped; on Stop the turn goes on with the reason as the next user text. Later hooks of that event do not run.
  - `{"arguments": {...}}` (PreToolUse): the tool runs with these arguments; the model's call in the log stays as sent.
  - `{"output": "..."}` (PostToolUse): replaces the tool's output before the `toolOutputBytes` cut and before it is sent or logged.
  - `{"context": "..."}`: added as `[hook context] <text>` at the end of the history before the next model call (after the last tool results or user text); earlier messages never change, so the cached prefix holds. For PreCompact it is added to the summary instructions.
  - `{"continue": "..."}` (Stop, SubagentStop): the turn goes on with this text as a user message. The step limit (`maxSteps`) bounds the loop; `stopHookActive` is true once a Stop hook has continued this turn.
- Fail open: a hook that cannot start, times out, exits non-zero or prints something that is not a JSON object is ignored; the log gets a `hook/error` event and the front end an error note. Every run is logged as `hook` with `event`, `command`, `ms` and `decision`.
- `fast-harness doctor` lists every hook with its event, pattern, layer and timeout.

A block rule (`guard.py`, PreToolUse on `shell`; not run by the tests):

```python
import json, sys
event = json.load(sys.stdin)
command = event["arguments"].get("command", "")
if "rm -rf" in command or "git push --force" in command:
    print(json.dumps({"decision": "block", "reason": "destructive command: ask the user first"}))
```

An output filter (`filter.py`, PostToolUse; not run by the tests) that drops progress lines before the result reaches the model:

```python
import json, sys
event = json.load(sys.stdin)
lines = [l for l in event["output"].splitlines() if not l.startswith(("Downloading", "Compiling"))]
print(json.dumps({"output": "\n".join(lines)}))
```

### `PreModelRequest` (D63)

Runs once per step, before the step's model call, in the main session and in subagent sessions; retries, account failovers and the call after an overflow compaction resend the same request without running it again, and a compaction's summary request does not run it. Stdin adds `turn`, `step`, `depth`, `model`, `effort`, `maxTokens`, `messages` and `tools` (each `{"count", "bytes"}`); never a key, token, account or header. The answer may hold:

- `{"decision": "block", "reason": "..."}`: no call; the turn ends failed and the front end shows `model request blocked by a hook: <reason>`.
- `{"request": {"effort": "low", "maxTokens": 4000}}`: `effort` is one of `minimal`, `low`, `medium`, `high`, `xhigh`, `max` (thinking stays on; on Claude a new effort can cost the cache of the messages, not of the system prompt and tools); `maxTokens` may only lower the request's own value. Any other key under `request` (`system`, `tools`, `messages`, `model`, `headers`, ...) makes the whole answer a `hook/error` and the request goes out unchanged: those parts are frozen for the session.
- `{"context": "..."}`: one `[hook context]` note appended to the newest user message, which the model has not seen yet, so the earlier request still repeats byte for byte.

A per-step budget guard (`budget.py`; not run by the tests) that keeps long sessions cheap:

```python
import json, sys
event = json.load(sys.stdin)
if event["messages"]["bytes"] > 400_000:
    print(json.dumps({"request": {"effort": "low", "maxTokens": 8000},
                      "context": "The history is long: answer briefly."}))
```

```json
{"PreModelRequest": [{"command": "python", "args": ["./budget.py"], "timeoutMs": 5000}]}
```

### The hook budget

Besides each entry's `timeoutMs`, all hooks of one event occurrence share one budget: `"budgetMs"` at the top of `hooks.json` (default 120000; with a user and a project file the smaller value counts). A hook starts with at most the time left; once it is spent the remaining hooks of that occurrence are skipped, each logged as `hook/error` (`skipped: the hook budget of N ms was spent`) with one note naming them, and the turn goes on as if they had answered nothing. The budget fails open, like a failed hook: a skipped `PreToolUse` block does not block.

## Memory

Each workspace has a memory folder, `~/.fast-harness/memory/<cwd slug>/` (the slug of `sessions/`): `MEMORY.md`, an index of at most 2 KB with one line per note (`- notes/build.md: how to build on Windows`), and the notes in `notes/*.md`. Edit them by hand or let the model do it: there is no memory tool, the system prompt names the folder and the model uses `read`, `write`, `edit` and `grep` on it.

- A new session puts the index in a second system block after the stable prompt. On Claude it has its own cache breakpoint, so the stable block stays byte-identical (and cached) across the sessions of the workspace whatever the memory holds; the OAuth identity block then has none, keeping 4 breakpoints in all. Antigravity and OpenAI-style providers get the index appended to the system text.
- The block is frozen for the session like the system prompt: a change of `MEMORY.md` reaches a running session as a `[context update]` note on its next message.
- Compaction appends the summary's `Durable facts` to `notes/compaction-facts.md` under a dated heading and keeps its line in `MEMORY.md`. When the index passes 2 KB its oldest lines move to `notes/index-archive.md`.
- `"memory": false` in `config.json` turns the block and the facts off.

### Reflect

`fast-harness reflect [ID] [--apply]` (or `/reflect` in the TUI and the GUI) runs one subagent on the cheapest model: `"reflect": {"model": "..."}` if set, else the model with the lowest `price` (input plus output) in `providers/*.json`, else the session model. It reads the numbers of `fast-harness stats` (including repeated reads, `not loaded yet` errors, failed edits, failed shell commands by program, steps per turn and cache rewrites) and a digest of the trace (tool names, errors, sizes and counts, never the conversation) and proposes a unified diff over memory notes (`memory/...`), `AGENTS.md` lines, skills and tool overrides (paths relative to the workspace).

`"selfImprove"` decides what happens next: `"ask"` (default) shows the diff and applies nothing until `/reflect apply` or `--apply`; `"auto"` applies the memory edits at once and leaves the rest for `/reflect apply`; `"off"` refuses `/reflect`. Nothing runs by itself at turn end.

### Another memory system

The hooks are the plug: a `Stop` hook receives the end of every turn and a `PreCompact` hook runs before a compaction (and may add instructions or block it), so a command can export the session to an outside store, or inject what it recalls through the compaction instructions. Set `"memory": false` to rely on it alone. Example `hooks.json`:

```json
{"Stop": [{"command": "my-memory save"}], "PreCompact": [{"command": "my-memory recall"}]}
```

## MCP servers: `mcp.json`

`mcp.json` in `~/.fast-harness/` and in a trusted project's `.fast-harness/` names MCP servers (PLAN.md D47). A project server with the same name replaces the user one; an untrusted project's file is not read. The servers below are examples, not tested live.

```json
{
  "servers": {
    "github": {
      "url": "https://api.githubcopilot.com/mcp/",
      "headers": {"Authorization": "Bearer ${GITHUB_TOKEN}"}
    },
    "files": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "F:/work"],
      "env": {"LOG_LEVEL": "warn"},
      "alwaysLoad": true
    },
    "old-one": {"command": "./bin/server.exe", "enabled": false, "timeoutMs": 30000}
  }
}
```

- `command`, `args`, `env`: a local server on stdio. A relative command with a folder (`./bin/server.exe`) is joined to the folder of the `mcp.json`.
- `url`, `headers`: a Streamable HTTP server.
- `${VAR}` in `env` and `headers` values is read from the environment when the server starts. Keep secrets in variables: `doctor` and `mcp list` print header and env names, never values.
- `enabled` (default true), `alwaysLoad` (default false: preload the server's tools like `tools.preload`), `timeoutMs` (default 120000, per request).

Nothing starts with a session. The tools come from `~/.fast-harness/mcp-cache/<server>.json`, the server's last tool list, so fill it once:

```
fast-harness mcp refresh            # every enabled server
fast-harness mcp refresh github     # one server
fast-harness mcp list               # servers, cache age, tool counts
```

A server starts on the first call of one of its tools (refreshing its list first when it is older than the server's `ttlMs`, one hour for servers without one) and stops after 10 idle minutes. The client speaks MCP 2026-07-28 first (`server/discover`, stateless requests) and falls back to the `initialize` handshake for older servers. An older HTTP server that answers HTTP 404 to a request with its `Mcp-Session-Id` has ended the session: the client initialises once more and retries the request once; a second failure is the tool's error and the next call connects from scratch (D63).

The model sees `mcp__<server>__<tool>` (other characters become `_`, at most 64 characters) with the description cut to 1 KB, found through `find_tools` like any tool. Up to 64 MCP tools or 64 KB of definitions join the tool list, whole servers in name order; past that, the remaining servers' tools are reached through `mcp_describe` and `mcp_call`, which `find_tools` loads when a search matches one of them. Results follow the tool output budget (`toolOutput`); images and binary resources become a one-line note. A hook can match `mcp__github__*`. A tool file named `mcp__<server>__<tool>.json` without `command` overrides the description or disables the tool.

## The index and preloading

At session creation the `tools`, `agents`, `commands` and `skills` folders of the user folder and of a trusted project are listed; only names and descriptions are read, and kept in `~/.fast-harness/index.json` by path, size and modified time, so an unchanged file is not read again. Bodies are read on use: a command tool's file when the session's tool list is first rendered (the list is then frozen in the session log), an agent or command body when it is called. Without any of these folders nothing is read. A project file with the same name as a user file replaces it.

`"tools": {"preload": ["read", "grep"]}` in `config.json` loads tools from the start of a top-level session (a synthetic `find_tools` call before the first step), so the first call needs no find. Empty by default.

`fast-harness doctor` lists the tool, agent, command and skill files that loaded, their layer, and the files skipped with the reason. The GUI's Settings page lists every key with its value and source, and opens the config folder.

## Keys: `keys.json`

Every key that starts an action in the TUI or the GUI is bound by action name, and `keys.json` in the user folder and in a trusted project folder rebinds it (no rebuild; the TUI and GUI read it at start). Later layer wins per action; `null` or `[]` unbinds.

```json
{
  "version": 1,
  "tui": {"pick_model": ["ctrl+k", "f9"], "pause": null},
  "gui": {"new_session": "ctrl+shift+n"}
}
```

Key strings: modifiers `ctrl`, `alt`, `shift`, `meta` joined with `+` before one key: a printable character (`k`, `,`, `/`; `ctrl++` is Ctrl and plus), a named key (`enter`, `esc`, `tab`, `backspace`, `delete`, `insert`, `space`, `up`, `down`, `left`, `right`, `home`, `end`, `pageup`, `pagedown`) or `f1` to `f24`. Case does not matter; `return`, `escape`, `pgup`, `pgdn`, `del`, `comma`, `control`, `cmd` are accepted and printed in the canonical form. An upper case letter is `shift+<letter>`.

An unknown action, a bad key, an unknown section or two actions that one key would start in the same place and state are reported by `fast-harness doctor` and as a note in the first session; the bad entry is skipped, the rest of the file applies. A file that is not JSON or has another `version` is skipped. An action for one state (idle, while a turn runs) wins over an `always` action on the same key: that is how Enter sends when idle and steers while running, and Alt+Enter queues while running and adds a new line when idle.

`fast-harness keys [--json]` prints the effective map with the layer of every action; the GUI's Settings > Configuration lists the same rows as `keys.tui.<action>` and `keys.gui.<action>`. The TUI hint line shows the keys as bound.

Not actions (fixed): list navigation (arrows, PgUp/PgDn, Home/End, Enter to pick, Esc to close a picker or dialog), Enter on a list row, `y` to confirm a removal, and text editing keys in the input. In the GUI, window actions take only keys with Ctrl, Alt or Meta, or function keys, so typing in a field is never taken; Shift+Enter in the message box stays a native new line.

### TUI actions

Scopes: `global` on every screen, `chat` on the chat, `screens` on every screen but the chat, `accounts`, `trace`, `models` on that screen, `approval` while an approval prompt shows.

| Action | Default keys | Where | When | Does |
|---|---|---|---|---|
| `interrupt` | `ctrl+c` | global | always | cancel the turn; twice when idle quits |
| `cycle_theme` | `f5` | global | always | next theme |
| `greyscale_up` | `f6` | global | always | greyscale +10 |
| `greyscale_down` | `shift+f6` | global | always | greyscale -10 |
| `open_accounts` | `f2`, `ctrl+a` | global | always | accounts screen |
| `toggle_trace` | `f3`, `ctrl+r` | global | always | trace screen and back |
| `open_models` | `f4` | global | always | models screen |
| `open_sessions` | `ctrl+o` | global | always | sessions screen |
| `send` | `enter` | chat | idle | send the input |
| `steer` | `enter` | chat | while a turn runs | steer the running turn |
| `queue` | `alt+enter` | chat | while a turn runs | queue a follow-up |
| `newline` | `alt+enter`, `ctrl+enter`, `shift+enter`, `ctrl+j` | chat | always | new line in the input |
| `stop` | `esc` | chat | while a turn runs | cancel the turn |
| `pause` | `ctrl+g` | chat | while a turn runs | pause after this step's tools |
| `remove_queued` | `ctrl+backspace` | chat | always | drop the last queued follow-up |
| `quit` | `ctrl+d` | chat | idle | quit (empty input) |
| `toggle_thinking` | `ctrl+t` | chat | always | show or hide thinking |
| `new_session` | `ctrl+n` | chat | always | new session |
| `pick_model` | `ctrl+p` | chat | always | model picker |
| `pick_effort` | `ctrl+e` | chat | always | effort picker |
| `scroll_page_up` | `pageup` | chat | always | scroll up a page |
| `scroll_page_down` | `pagedown` | chat | always | scroll down a page |
| `scroll_top` | `ctrl+home` | chat | always | scroll to the top |
| `scroll_bottom` | `ctrl+end` | chat | always | follow the bottom |
| `back` | `esc`, `q` | screens | always | back (to the chat) |
| `trace_full` | `f` | trace | always | full or cut bodies |
| `trace_steps` | `s` | trace | always | steps table or timeline |
| `account_sign_in` | `l` | accounts | always | Claude sign-in |
| `account_add_key` | `k` | accounts | always | add an API key |
| `account_remove` | `d` | accounts | always | remove the account |
| `account_usage` | `r` | accounts | always | read usage |
| `cycle_effort` | `e`, `tab`, `right`, `left` | models | always | next effort |
| `approve_once` | `alt+y` | approval | always | approval prompt: allow this call once |
| `approve_session` | `alt+a` | approval | always | approval prompt: allow it for this session |
| `decline` | `alt+n` | approval | always | approval prompt: decline the call |

The `approval` scope is live only while an approval prompt shows on the chat (`"approvals": "ask"`, D64) and comes before the chat keys, so `doctor` reports a key bound both there and in `chat`. No key answers by default: Enter still steers, Space types, Esc (`stop`) cancels the turn and with it the request. Rebind like any action, for example `{"version": 1, "tui": {"approve_once": "ctrl+y", "decline": ["alt+n", "ctrl+shift+n"]}}`; an unbound answer shows as `(unbound)` in the prompt.

### GUI actions

Scopes: `window` anywhere in the window, `composer` while the message box has the focus, `approval` while an approval prompt shows.

| Action | Default keys | Where | When | Does |
|---|---|---|---|---|
| `send` | `enter` | composer | idle | send the message |
| `steer` | `enter` | composer | while a turn runs | steer the running turn |
| `queue` | `alt+enter` | composer | while a turn runs | queue a follow-up |
| `stop` | `esc` | composer | while a turn runs | stop the turn |
| `pause` | `ctrl+g` | window | while a turn runs | pause after this step's tools |
| `resume` | `ctrl+shift+r` | window | idle | resume the last turn |
| `new_session` | `ctrl+n` | window | always | new session |
| `toggle_sidebar` | `ctrl+b` | window | always | show or hide the sidebar |
| `open_chat` | `ctrl+1` | window | always | chat page |
| `open_accounts` | `f2` | window | always | accounts page |
| `open_trace` | `f3` | window | always | trace page |
| `reload_ui` | `f5` | window | always | reload ~/.fast-harness/ui (fast-harness-gui-live, `docs/ui-contract.md`) |
| `open_settings` | `ctrl+,` | window | always | Settings dialog |
| `focus_composer` | `ctrl+l` | window | always | focus the message box |
| `approve_once` | `alt+y` | approval | always | approval prompt: allow this call once |
| `approve_session` | `alt+a` | approval | always | approval prompt: allow it for this session |
| `decline` | `alt+n` | approval | always | approval prompt: decline the call |

The GUI `approval` scope is live only while the approval prompt shows above the message box (D64) and is read anywhere in the window before the `window` keys. Like `window` keys, only keys with Ctrl, Alt or Meta, or function keys, answer, so typing never does: a binding to a bare letter does nothing there. Enter and Space never answer from the message box; Tab reaches only the Decline button (the allow buttons are not focus stops); Esc (`stop`) cancels the turn and with it the request.
