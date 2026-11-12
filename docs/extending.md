# Extending Fast Harness

Everything here is a plain file: edit it and restart (or start a new session), no rebuild. Decisions: `PLAN.md` D41 (settings layers, trust, providers) and D33 (themes, `docs/themes.md`).

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

A broken user `config.json` stops the start with its error, as before. A broken project `config.json` or provider file is skipped, and `fast-harness doctor` names it with the parser's error.

## Project trust

The project is the nearest folder at or above the working directory that holds a `.fast-harness` folder (other than `~/.fast-harness` itself). A project folder can name endpoints and, later, commands, so nothing in it loads until you trust it once:

- `fast-harness trust [PATH]` (default: the current directory), or
- type `/trust` in the TUI or the GUI composer.

The list lives in `~/.fast-harness/trusted.json`:

```json
{"version": 1, "paths": ["F:\\work\\my-project"]}
```

Untrusted, the front ends say so once: a note in the TUI and the GUI with the first message, a `[note]` line on stderr in print mode.

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

A file whose `id` is `anthropic` or `antigravity` defines no provider: its model entries change the numbers of built-in models (`contextWindow`, `maxOutput`, `price`, `compaction`). Claude models use their bare id; Antigravity models the id after `antigravity/`.

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
- `find_tools` matches skills by exact name in `names` and by `query` (the best match scoring a name hit or two description words). The result keeps its `Loaded: ...` line for tools and adds, per skill (at most 3 per call), a block starting `[skill <name>: <folder>]` with the body after the front matter, cut at 20 KB. On Claude the body is text next to the `tool_reference` blocks. This result is not cut by `toolOutputBytes`.
- Skills loaded so far are rebuilt from the session log (the `[skill ...]` lines), and a compaction summary lists them.
- Untrusted projects load no skills; a project skill replaces a user skill of the same name.

## Nested AGENTS.md

The AGENTS.md files from the repository root down to the working directory are in the system prompt, at most `agentsBytes` (default 32768) together. A folder below the working directory with its own `AGENTS.md` has it appended, at most 6 KB, to the first tool result whose `path` argument (read, write, edit, grep, glob, and any command tool with a `path`) lies inside it, once per session. The shell has no working-directory argument, so its calls load nothing. The text joins the result after the `toolOutputBytes` cut (outside the budget) and before the result is sent or logged, under a line `[nested AGENTS.md <path>]`; the loaded set is rebuilt from the log on resume, and a compaction summary lists those lines so the files are not appended again.

## Hooks: `hooks.json`

`~/.fast-harness/hooks.json` and a trusted project's `.fast-harness/hooks.json` run your own programs at seven points of the agent loop. Both files load and both run, user entries first; an untrusted project's file is not read. Without a file nothing is spawned and nothing else is read. The file is read with the other settings (at start, `/trust`, a settings reload); a change applies to the next agent built from them.

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

- Events: `SessionStart` (first turn of a new session), `UserPromptSubmit` (each user prompt, before the model sees it), `PreToolUse`, `PostToolUse`, `PreCompact`, `Stop` (the model ended the turn), `SubagentStop` (the same, in a subagent session, instead of `Stop`).
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

## The index and preloading

At session creation the `tools`, `agents`, `commands` and `skills` folders of the user folder and of a trusted project are listed; only names and descriptions are read, and kept in `~/.fast-harness/index.json` by path, size and modified time, so an unchanged file is not read again. Bodies are read on use: a command tool's file when the session's tool list is first rendered (the list is then frozen in the session log), an agent or command body when it is called. Without any of these folders nothing is read. A project file with the same name as a user file replaces it.

`"tools": {"preload": ["read", "grep"]}` in `config.json` loads tools from the start of a top-level session (a synthetic `find_tools` call before the first step), so the first call needs no find. Empty by default.

`fast-harness doctor` lists the tool, agent, command and skill files that loaded, their layer, and the files skipped with the reason. The GUI's Settings page lists every key with its value and source, and opens the config folder.
