# Fast Harness providers

The wire details of the model providers, moved here unchanged from `PLAN.md` on 2026-11-13. The OpenAI-compatible providers of `providers/*.json` are in [extending.md](extending.md).

## Claude provider

- `POST https://api.anthropic.com/v1/messages`, `stream: true`, `anthropic-version: 2023-06-01`.
- API key: `x-api-key`. Subscription OAuth: `Authorization: Bearer`, betas `claude-code-20250219` and `oauth-2025-04-20`, and the system prompt sent as two blocks, first "You are Claude Code, Anthropic's official CLI for Claude." (as pi-ai does; other clients depend on this and Anthropic may restrict it).
- Prompt caching: `cache_control` on the system blocks, on `find_tools` (deferred tools are outside the cached prefix, D30) and on the last message; 5 minutes or 1 hour per `cacheTtl` (D36): the tools and system share one lifetime and come first, so a 1 hour breakpoint never follows a 5 minute one.

## Antigravity provider

> On `main`: this provider, the Google accounts and their files live on branch `antigravity` (a143cfa), not on main; [D53](decisions/D53.md) says how to bring them back. The text below describes that branch.

Ported from DSH's live-verified antigravity-switch (`buildEnvelope`, `fixContents`, `cleanSchema`, `fixThinking`, `pickerIds`) and pi-ai's Gemini conversion (`google-shared.ts`, read 2026-11-12). Code: `llm/antigravity/request/*`, `llm/antigravity/models*`.

- Envelope: `{project (fallback rising-fact-p41fc), model: bare id, request: {contents, systemInstruction, tools, toolConfig, generationConfig, sessionId}, requestType: "agent", userAgent: "antigravity", requestId: "agent-<uuid v4>"}`. `systemInstruction` is `{role: user, parts: [{text}]}`: Antigravity's identity text, a blank line, then the system prompt, in one part.
- Contents: user text and tool results as role `user`, assistant messages as role `model`; consecutive same-role contents merged; a history starting with `model` gets a short user text first; no message is a bad-request error. Tool calls are `functionCall {name: sent name, args: parsed JSON object ({} when not an object), id}`; results `functionResponse {name: sent name of the matching call, response: {output} or {error}, id}`. `id` only for `claude-*`, `gpt-oss-*` and Gemini 3 and later (pi-ai `requiresToolCallId`).
- Provenance: thinking (`thought: true` parts) and thought signatures go back only when the assistant message's `model` equals the request's full id; thinking from another model is dropped. A signature is real at 50 or more characters and not the sentinel. The first `functionCall` of each model turn carries its real signature or `skip_thought_signature_validator`; later parallel calls carry none. Claude thinking without a real signature is dropped.
- Tools: one `{functionDeclarations}` entry; schemas cleaned (`$ref` to a `See:` hint, `const` to `enum`, constraint and enum hints into the description, `allOf` merged, unions and type arrays flattened with `Accepts:`/`nullable` hints, unsupported keywords dropped, `required` kept to real non-nullable properties, empty objects get a required boolean `_placeholder`). Names: `[a-zA-Z0-9_-]`, a letter or `_` first, at most 64 characters, collisions get `_2`, `_3`; the stream maps them back. Claude with tools: `toolConfig.functionCallingConfig.mode = "VALIDATED"`.
- `generationConfig.maxOutputTokens`: the request's max tokens capped by the catalog's `maxOutputTokens`; budget Claude raises it to 64000 when it is at or below the thinking budget.
- Catalog: `fetchAvailableModels {project}`; `gemini-`, `claude-`, `gpt-oss-` ids without `image`, `-disabled`, `-tiered` or `disabled: true`, limited to `agentModelSorts` and `tieredModelIds` when present, in that order, then the rest sorted. Entries are remembered process-wide for the request. Unknown values: context 1,048,576 for Gemini, else 200,000; max output 65,536.

- Routing (`app.rs`, `app/models.rs`, `llm/catalog.rs`): `Runtime::parts` reads `config.model`; `antigravity/<id>` gets the Antigravity provider and the Google accounts (`GoogleAuth`; no Google account is an error naming Accounts > Add Google account and `fast-harness accounts login --google`), any other id keeps Claude (env key or token, then the pool). Subagents copy the parent's parts; the trace's `request/context` provider is `antigravity`. The model catalog lists each provider with a credential (Antigravity through a Google lease) and merges them, Claude first; a provider without a credential or whose list failed keeps its cached part, the fetch fails only when every listed provider failed (the message names each), and a failed list never holds or switches an account. Efforts and effort fitting dispatch by prefix (`llm::efforts`, `llm::fit_effort`). Pickers (GUI dropdown, TUI Ctrl+P and Models screen) show a "Claude" and a "Google Antigravity" title row that keys and clicks skip; `fast-harness models` adds a provider column and `provider` in `--json`.

Effort mapping (our word to Google's):

| Family | Efforts | Wire |
| --- | --- | --- |
| Gemini 3 and later | `minimal` (flash, lite), `low`, `medium`, `high`; `gemini-3-pro*`: `low`, `high` | `thinkingConfig {includeThoughts: true, thinkingLevel: <word>}`; `off` is the lowest level; none: catalog level, else `high` |
| Gemini 2.x | `low`, `medium`, `high`, `off` (not `pro`) | `thinkingBudget` 8192, 16384, 24576; `off` is `{thinkingBudget: 0}`; none: catalog budget, else `includeThoughts` only |
| Claude, adaptive (catalog `supportsAdaptiveThinking`, else Claude 5+ by id) | `low`, `medium`, `high`, `off` | `{includeThoughts: true, thinkingLevel}`; `off` sends none; none: catalog level, else `high` |
| Claude, budget (thinking ids, catalog `supportsThinking`) | `low`, `medium`, `high`, `off` | `{include_thoughts: true, thinking_budget}` 8192, 16384, 32768; none: catalog budget, else 16000 |
| `claude-sonnet-4-6`, GPT-OSS, others | none | no thinking config |

`xhigh`, `max` and unknown words become `high` (else the highest level). An id ending in `-minimal|-low|-medium|-high` thinks at that level and offers no effort. Unverified: the Gemini 2.x budget values and that Gemini 2.x `pro` cannot switch thinking off (from older Gemini docs, not on the pages read 2026-11-12); the per-model level lists come from the Gemini API thinking page (read 2026-11-12), not from Antigravity.
