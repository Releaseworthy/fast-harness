# Panels: `panels/*.json`

A panel is a view drawn by any program you write, in any language, with no rebuild of Fast Harness (PLAN.md D46, docs/efficiency-plan.md phase 21A). The program speaks JSON lines on stdin and stdout; the GUI draws its view tree with the theme's colours, the TUI draws the same tree as text.

## The file

`panels/<name>.json` in the user folder (`~/.fast-harness`) or a trusted project's `.fast-harness` folder (an untrusted project's panels do not load; `/trust` loads them). A later layer's panel with the same `id` replaces the earlier one.

```json
{"id": "todo", "title": "Todo", "icon": "list-flat", "where": "tab", "command": "bun", "args": ["./todo.ts"]}
```

- `id`: letters, digits, `-` or `_`. `title`: the tab or section name (default: the id).
- `icon`: one of `tools`, `pulse`, `list-flat`, `eye`, `terminal`, `robot`, `file`, `search`, `watch`, `info`, `settings-gear`, `folder` (others show a generic mark).
- `where`: `tab` (the default) or `sidebar`. The GUI has one panel view (one generic row component, see PLAN.md D46): both open as a closable tab like Settings; `where` is shown in the Settings list.
- `command` and `args`: a relative `command` with a `/` or `\` runs from the file's folder; the process starts in the session's working directory with `FAST_HARNESS_PANEL_DIR` set to the file's folder.

Files are only indexed at start: no process runs until the panel is first opened, from GUI Settings > Configuration (a `panels.<id>` row with "Open"), or `/panel <id>` in the GUI or TUI (`/panel` alone lists them; `/help` shows the command). The process is killed with its whole process tree when the app exits. A process that dies shows `process ended (exit code N): <its last stderr line>` in the panel; opening the panel again restarts it.

## Protocol

One JSON object per line, both ways. The host writes first:

```json
{"type":"init","panel":"todo","theme":{"kind":"dark"},"cwd":"F:/work/app","session":"20261112-..."}
```

The process sends:

- `{"type":"view","root":<node>}`: the whole tree (replaces the current one).
- `{"type":"patch","id":"<node id>","node":<node>}`: replaces the node with that `id`.
- `{"type":"subscribe","events":["turn","tool","usage"]}`: session events to receive.
- `{"type":"notify","text":"Saved"}`: shown as the panel's status line.

A line that is not JSON or not one of these shows as a status note (`panel protocol: ...`).

The host sends widget events and the subscribed session events:

- `{"type":"click","id":"<button id>","value":null}`; a list item click has the list's `id` and the item text as `value`.
- `{"type":"submit","id":"<input id>","value":"typed text"}` (Enter in an input).
- `{"type":"change","id":"<id>","value":true}` for a checkbox, `"value":"<option>"` for a select.
- `{"type":"session","event":{...}}`: `turn` gives `{"kind":"turn_start","turn"}` and `{"kind":"turn_end","turn","outcome","durationMs","input","output"}`, `tool` gives `{"kind":"tool_call","id","name"}` and `{"kind":"tool_result","id","isError","durationMs"}`, `usage` gives `{"kind":"usage","input","output","context","window"}` after every step.

## Widgets

Every node is `{"type": ..., "id": ..., ...}`; `id` is needed for events and patches.

| type | props | drawn as |
| --- | --- | --- |
| `text`, `markdown` | `text` | wrapped text (markdown shows as plain text) |
| `button` | `text` or `label` | a button; click sends `click` |
| `input` | `label`, `placeholder`, `value` | a one-line field; Enter sends `submit` |
| `select` | `label`, `options` (strings), `value` | the value; a click (GUI) or Enter, Left, Right (TUI) sends `change` with the next option |
| `checkbox` | `label`, `checked` | `[x]` or `[ ]` and the label; a click sends `change` |
| `progress` | `label`, `value` (0 to 1) | a bar and the percent |
| `list` | `items` (strings) | a row per item; a click sends `click` |
| `table` | `columns`, `rows` (arrays) | a bold header row, a row per line (GUI: the cells joined by tabs) |
| `column` | `children` | children stacked, indented one level (not at the root) |
| `row` | `children` | children side by side in the TUI; stacked at the row's depth in the GUI |

An unknown type shows as a note `! unknown widget <type>`. Views are cut at 2000 rows and texts at 12,000 characters.

The TUI shows the panel in place of the transcript: Tab and Shift+Tab move between buttons, inputs, selects, checkboxes and list items (the focused one has `›`), Enter acts, typing fills an input, Esc returns to the chat.

## Example

`~/.fast-harness/panels/counter.json`:

```json
{"id": "counter", "title": "Counter", "icon": "pulse", "where": "tab", "command": "bun", "args": ["./counter.ts"]}
```

`~/.fast-harness/panels/counter.ts`:

```ts
import { createInterface } from "node:readline";

let count = 0;
let tools = 0;
const send = (msg: unknown) => process.stdout.write(JSON.stringify(msg) + "\n");
const view = () =>
  send({
    type: "view",
    root: {
      type: "column",
      children: [
        { type: "text", id: "title", text: `Count: ${count}` },
        { type: "row", children: [
          { type: "button", id: "add", text: "Add one" },
          { type: "button", id: "reset", text: "Reset" },
        ] },
        { type: "input", id: "note", label: "Note", placeholder: "type and press Enter" },
        { type: "progress", id: "p", label: "Tools this session", value: Math.min(tools / 20, 1) },
      ],
    },
  });

for await (const line of createInterface({ input: process.stdin })) {
  const msg = JSON.parse(line);
  if (msg.type === "init") {
    send({ type: "subscribe", events: ["tool"] });
    view();
  } else if (msg.type === "click") {
    count = msg.id === "add" ? count + 1 : 0;
    send({ type: "patch", id: "title", node: { type: "text", id: "title", text: `Count: ${count}` } });
  } else if (msg.type === "submit") {
    send({ type: "notify", text: `Noted: ${msg.value}` });
  } else if (msg.type === "session" && msg.event.kind === "tool_result") {
    tools += 1;
    view();
  }
}
```

The core keeps no UI types: `fast_harness_core::panels` parses the files, runs the process (threads and channels, one process group), and flattens the tree into rows (`Row { depth, cells }`, `Cell { kind, id, text, value, hint, options, checked, progress, header }`); the GUI (`ui/panel_view.slint`, one generic component) and the TUI (`tui/panel_screen.rs`) draw them.

## Cost

Panels add 328,192 bytes to `fast-harness-gui.exe`, over the first 150 KB budget. The owner accepted it on 2026-11-12: "chỉ 320kb thôi, chấp nhận được, thậm chí 600kb cũng chấp nhận được, size của file exe không quan trọng bằng goal là harness gui chạy nhẹ tài nguyên ram,cpu,gpu" (only 320 KB, acceptable, even 600 KB is acceptable; exe size matters less than the goal that the GUI runs light on RAM, CPU and GPU). The GUI budgets are therefore RAM, CPU and GPU use, idle and with a panel open; the numbers before and after panels are in `docs/perf.md` (phase 21A).
