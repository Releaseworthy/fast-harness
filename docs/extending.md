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
