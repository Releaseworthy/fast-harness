# UI contract (PLAN.md D51)

`fast-harness-gui-live` (cargo feature `ui-files`) builds its window from the `.slint` files in `~/.fast-harness/ui/` instead of the UI compiled into `fast-harness-gui`. The Rust side binds that UI by name, so the files must keep every property, callback and function below with its name, kind and type. Everything else (layouts, widgets, pages, colours, text, new components) is free to change.

## Workflow

1. `fast-harness-gui --export-ui %USERPROFILE%\.fast-harness\ui` writes the built-in `ui/*.slint` files (the same text the binary was built from). In PowerShell, `Start-Process -Wait` shows when it is done: the GUI binary does not hold the console.
2. Edit any file in a text editor.
3. Start `fast-harness-gui-live`, or press Reload in a running one (keys.json action `reload_ui`, `f5` by default, or Settings > UI files > Reload UI files).

`main.slint` must export `MainWindow`. The files refer to `../fonts/FastHarnessIcons.ttf` and `../assets/*.png`: the binary stages the user folder next to its own copies of those, so the relative paths keep working; a user file of the same folder may add its own images.

## Checks

At start and on Reload the binary compiles the user folder with the Slint interpreter and checks this list. When the folder does not compile, `MainWindow` is missing, or an entry below is missing or has another kind or type, the error list goes to the banner of the window and:

- at start, the built-in UI is used;
- on Reload, the current UI stays.

Without a `~/.fast-harness/ui/main.slint` the built-in UI is used and the banner says how to export it.

The list is generated: `cargo test -p fast-harness-gui ui_contract` fails when it differs from the Rust and `.slint` sources (`UPDATE_UI_CONTRACT=1` rewrites it). The default `fast-harness-gui` never links the interpreter.

## Running tools

Both front ends draw a tool call from `view::Item::Tool`. While it runs (`output` is `None`) its `live` tail holds the newest lines of the tool's live output (`AgentEvent::ToolProgress`, PLAN.md D58): at most `view::LIVE_LINES` (8) lines of at most `view::LIVE_LINE_BYTES` (256) bytes, with escape sequences and control characters removed, `\r\n` or `\n` ending a line and a lone `\r` starting it again. The TUI shows the tail under the spinning tool row in the dim role, one row per line clipped to the width; the GUI shows `MsgRow.live` (the tail joined by `\n`, empty otherwise) under the running card's header in `Theme.dim` mono text, one line per line, never wrapped. When the result arrives the tail is gone and the row is the finished tool row, the same as in a replayed session (progress is never in the trace). A user UI without `MsgRow.live` simply does not show the tail.

## Loading

Both front ends list the sessions of a workspace and read a trace log on worker threads, so a screen opens at once and can be empty for a moment (PLAN.md D60). While that read runs, each front end shows one muted waiting text:

- GUI sidebar: an expanded workspace whose sessions have not been listed once gets one row of kind `loading` in `MainWindow.side-rows` where its session rows will be: `title` `Loading sessions…`, empty `id`, `detail` and `run`, not current and not clickable. The row is drawn at 28px with `Theme.font-xs` in `Theme.dim`, indented like a session row. The first list of that workspace replaces it with the session rows. A later list (after a turn, or on Reopen) does not bring it back: the rows shown stay until the new ones arrive.
- GUI Trace page: while the page shows a session whose rows it does not have yet, `MainWindow.trace-message` is `Loading the session log…`, drawn in `Theme.dim`, wrapped and centred in an 80px band above the rows. The rows that arrive replace the text. While a newer read of the same session runs the rows shown stay and no text is set.
- TUI: the Sessions screen draws `Loading sessions...` in the dim style when a read runs and no rows are shown yet, and `No sessions yet.` in the plain text style when a list came back empty. The Trace screen draws `Loading the session log...` in the dim style while its read runs, in place of entries, steps and error.

A user UI that keeps `MainWindow.side-rows` and `MainWindow.trace-message` shows the two texts; one that draws neither kind simply shows the workspace or the page without rows while the read runs.

## Approvals

With `"approvals": "ask"` a call can wait for an answer (PLAN.md D64). Both front ends read the requests of the shown session from `Session::approvals()` and each tool row's word from `view::Transcript::approvals` (`waiting for your answer`, `allowed once`, `allowed for this session`, `declined`, `cancelled`, `paused`, `refused: nobody can answer`, always a word next to the colour).

- TUI: the oldest request is a prompt between the queue rows and the input box, in a frame with `warning` borders and the title `Approval`; the whole command (`view::approvals::prompt_text`) wraps in the text style, the key line in the accent style, `N more waiting after this one` in the dim style. The hint line repeats the key line. The tool row shows its word in the warning style after the mark. Nothing shows when no request waits.
- GUI: `MainWindow.approval` (an `ApprovalView`) holds the oldest request of the shown session, an empty `id` when none waits: `title` (`Approval needed: <tool> wants to run a command` or `change files`), `text` (the whole command, `view::approvals::prompt_text`; past 4,000 characters the first and last 1,500 stay around `... N more characters (the end follows) ...`), `agent` (`Asked by subagent <name>` or empty), `more` (`N more waiting after this one` or empty) and the labels of the keys bound to `approve_once`, `approve_session` and `decline`. The built-in UI draws it between the queue chips and the composer, only while `id` is not empty (no row, no timer otherwise), in a `Theme.raised` card with a 2px `Theme.warning` border and the warning mark before the title; the command wraps in `Theme.mono` on `Theme.code` and scrolls inside a 168px box with the themed bar past that. Its buttons: `Decline (<key>)`, `Allow once (<key>)`, `Allow for this session (<key>)`, then the `more` text in `Theme.dim`. Only `Decline` is a Tab stop: the allow buttons set `ActionButton.tab-stop: false`, so no sequence of Tab, Shift+Tab, Enter and Space can approve (a stray key must not say yes); they answer by a click or their key. A button or a key calls `MainWindow.answer-approval(id, action)` with the action name. The window reads the `approval` keys (only chords: Ctrl, Alt or Meta, or a function key) before its own keys and only while `id` is not empty; no button has the focus or a default, so Enter in the composer still sends or steers and Space types. `MsgRow.approval` is the tool row's word, drawn after the status pill with a mark (a watch while it waits, the warning sign after) in `Theme.warning`. A sidebar session row whose session waits has `run` `waiting`, drawn with the watch mark in `Theme.warning`. A user UI without these entries never shows a prompt, and the requests wait until the turn is stopped or another client answers.

## Entries

<!-- contract:start -->
- `AppearanceUi.chat-detail-changed`: callback
- `AppearanceUi.chat-full`: property <bool>
- `AppearanceUi.failing`: property <[ThemeCard]>
- `AppearanceUi.folder`: property <string>
- `AppearanceUi.greyscale`: property <int>
- `AppearanceUi.greyscale-changed`: callback
- `AppearanceUi.notes`: property <string>
- `AppearanceUi.open-folder`: callback
- `AppearanceUi.pick`: callback
- `AppearanceUi.reload`: callback
- `AppearanceUi.themes`: property <[ThemeCard]>
- `Caret.composer`: property <bool>
- `Caret.resting`: property <bool>
- `ConfigUi.action`: callback
- `ConfigUi.files`: property <string>
- `ConfigUi.folder`: property <string>
- `ConfigUi.open-folder`: callback
- `ConfigUi.rows`: property <[ConfigRow]>
- `DialogUi.open`: property <string>
- `JobsUi.clear`: callback
- `JobsUi.closed`: callback
- `JobsUi.dropped`: property <bool>
- `JobsUi.expand`: callback
- `JobsUi.expanded`: property <string>
- `JobsUi.finished`: property <[JobChip]>
- `JobsUi.kill`: callback
- `JobsUi.open`: property <bool>
- `JobsUi.output`: property <string>
- `JobsUi.running`: property <[JobChip]>
- `LimitsUi.max-depth`: property <int>
- `LimitsUi.max-depth-changed`: callback
- `LimitsUi.max-parallel`: property <int>
- `LimitsUi.max-parallel-changed`: callback
- `LimitsUi.scale`: property <int>
- `LimitsUi.scale-preview`: callback
- `LimitsUi.scale-saved`: callback
- `MainWindow.accounts`: property <[AccountRow]>
- `MainWindow.accounts-message`: property <string>
- `MainWindow.accounts-provider`: property <string>
- `MainWindow.accounts-working`: property <bool>
- `MainWindow.add-key`: callback
- `MainWindow.add-workspace`: callback
- `MainWindow.adding-key`: property <bool>
- `MainWindow.answer-approval`: callback
- `MainWindow.approval`: property <ApprovalView>
- `MainWindow.attach-image`: callback
- `MainWindow.attached`: property <[ImageChip]>
- `MainWindow.busy`: property <bool>
- `MainWindow.cancel-sign-in`: callback
- `MainWindow.draft`: property <string>
- `MainWindow.effort-index`: property <int>
- `MainWindow.effort-picked`: callback
- `MainWindow.efforts`: property <[string]>
- `MainWindow.error-text`: property <string>
- `MainWindow.key-action`: callback
- `MainWindow.loading`: property <bool>
- `MainWindow.login-url`: property <string>
- `MainWindow.messages`: property <[MsgRow]>
- `MainWindow.model-headers`: property <[bool]>
- `MainWindow.model-index`: property <int>
- `MainWindow.model-picked`: callback
- `MainWindow.models`: property <[string]>
- `MainWindow.models-refreshing`: property <bool>
- `MainWindow.new-session`: callback
- `MainWindow.open-page`: callback
- `MainWindow.open-session`: callback
- `MainWindow.open-url`: callback
- `MainWindow.page`: property <string>
- `MainWindow.pause`: callback
- `MainWindow.picker-note`: property <string>
- `MainWindow.pickers-opened`: callback
- `MainWindow.providers`: property <[ProviderCard]>
- `MainWindow.queue`: callback
- `MainWindow.queue-chips`: property <[QueueChip]>
- `MainWindow.queue-key`: property <string>
- `MainWindow.refresh-trace`: callback
- `MainWindow.refresh-usage`: callback
- `MainWindow.reload-ui`: callback
- `MainWindow.remove-account`: callback
- `MainWindow.remove-queued`: callback
- `MainWindow.remove-workspace`: callback
- `MainWindow.resume`: callback
- `MainWindow.resume-word`: property <string>
- `MainWindow.select-workspace`: callback
- `MainWindow.send`: callback
- `MainWindow.send-key`: property <string>
- `MainWindow.set-color-scheme`: function
- `MainWindow.show-draft`: function
- `MainWindow.side-rows`: property <[SideRow]>
- `MainWindow.sidebar-open`: property <bool>
- `MainWindow.sign-in`: callback
- `MainWindow.status-account`: property <string>
- `MainWindow.status-model`: property <string>
- `MainWindow.status-state`: property <string>
- `MainWindow.status-timing`: property <string>
- `MainWindow.status-tokens`: property <string>
- `MainWindow.step-toggle`: callback
- `MainWindow.stop`: callback
- `MainWindow.title-bar-clicked`: callback
- `MainWindow.to-bottom`: function
- `MainWindow.toggle`: callback
- `MainWindow.trace-entries`: property <[TraceRow]>
- `MainWindow.trace-message`: property <string>
- `MainWindow.trace-mode`: property <string>
- `MainWindow.trace-session`: property <string>
- `MainWindow.trace-steps`: property <[[string]]>
- `MainWindow.trace-toggle`: callback
- `MainWindow.transcript-content-height`: property <length>
- `MainWindow.transcript-follow`: property <bool>
- `MainWindow.transcript-thumb-x`: property <length>
- `MainWindow.transcript-thumb-y`: property <length>
- `MainWindow.turn-toggle`: callback
- `MainWindow.ui-files`: property <bool>
- `MainWindow.undo`: property <UndoBar>
- `MainWindow.undo-turn`: callback
- `MainWindow.use-account`: callback
- `MainWindow.version`: property <string>
- `MainWindow.win-maximized`: property <bool>
- `MainWindow.window-minimize`: callback
- `MainWindow.window-toggle-maximize`: callback
- `PanelUi.activate`: callback
- `PanelUi.ended`: property <bool>
- `PanelUi.icon`: property <string>
- `PanelUi.id`: property <string>
- `PanelUi.rows`: property <[PanelRow]>
- `PanelUi.status`: property <string>
- `PanelUi.submit`: callback
- `PanelUi.title`: property <string>
- `PromptUi.dirty`: property <bool>
- `PromptUi.edited`: callback
- `PromptUi.override-on`: property <bool>
- `PromptUi.override-toggled`: callback
- `PromptUi.placeholders`: property <string>
- `PromptUi.preview`: property <string>
- `PromptUi.preview-toggled`: callback
- `PromptUi.reset`: callback
- `PromptUi.save`: callback
- `PromptUi.saved`: property <string>
- `PromptUi.scope`: property <int>
- `PromptUi.scope-picked`: callback
- `PromptUi.show-preview`: property <bool>
- `PromptUi.source`: property <string>
- `PromptUi.status`: property <string>
- `PromptUi.text`: property <string>
- `PromptUi.workspace`: property <string>
- `SubagentsUi.back`: callback
- `SubagentsUi.back-label`: property <string>
- `SubagentsUi.open`: property <bool>
- `SubagentsUi.open-child`: callback
- `SubagentsUi.read-only`: property <bool>
- `SubagentsUi.rows`: property <[SubagentRow]>
- `SubagentsUi.running`: property <int>
- `Theme.accent`: property <color>
- `Theme.accent-surface`: property <color>
- `Theme.accent2`: property <color>
- `Theme.bg`: property <color>
- `Theme.bg-end`: property <color>
- `Theme.border`: property <color>
- `Theme.bubble`: property <color>
- `Theme.code`: property <color>
- `Theme.dark`: property <bool>
- `Theme.dim`: property <color>
- `Theme.error`: property <color>
- `Theme.glow`: property <color>
- `Theme.greyscale`: property <bool>
- `Theme.greyscale-level`: property <int>
- `Theme.hover`: property <color>
- `Theme.info`: property <color>
- `Theme.line`: property <color>
- `Theme.on-accent`: property <color>
- `Theme.raised`: property <color>
- `Theme.selection`: property <color>
- `Theme.shadow`: property <color>
- `Theme.success`: property <color>
- `Theme.surface`: property <color>
- `Theme.text`: property <color>
- `Theme.warning`: property <color>
- `TraceProbe.content-height`: property <length>
- `TurnsUi.column-right`: property <length>
- `TurnsUi.hovered`: property <int>
- `TurnsUi.nav-shown`: property <bool>
- `TurnsUi.nav-step`: property <length>
- `TurnsUi.nav-top`: property <length>
- `TurnsUi.nav-x`: property <length>
- `TurnsUi.top-row`: property <int>
- `TurnsUi.turns`: property <[TurnTick]>
<!-- contract:end -->
