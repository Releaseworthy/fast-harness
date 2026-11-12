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

## Entries

<!-- contract:start -->
- `AppearanceUi.failing`: property <[ThemeCard]>
- `AppearanceUi.folder`: property <string>
- `AppearanceUi.greyscale`: property <int>
- `AppearanceUi.greyscale-changed`: callback
- `AppearanceUi.notes`: property <string>
- `AppearanceUi.open-folder`: callback
- `AppearanceUi.pick`: callback
- `AppearanceUi.reload`: callback
- `AppearanceUi.themes`: property <[ThemeCard]>
- `ConfigUi.action`: callback
- `ConfigUi.files`: property <string>
- `ConfigUi.folder`: property <string>
- `ConfigUi.open-folder`: callback
- `ConfigUi.rows`: property <[ConfigRow]>
- `MainWindow.accounts`: property <[AccountRow]>
- `MainWindow.accounts-message`: property <string>
- `MainWindow.accounts-provider`: property <string>
- `MainWindow.accounts-working`: property <bool>
- `MainWindow.add-key`: callback
- `MainWindow.add-workspace`: callback
- `MainWindow.adding-key`: property <bool>
- `MainWindow.busy`: property <bool>
- `MainWindow.cancel-sign-in`: callback
- `MainWindow.effort-index`: property <int>
- `MainWindow.effort-picked`: callback
- `MainWindow.efforts`: property <[string]>
- `MainWindow.error-text`: property <string>
- `MainWindow.google-accounts`: property <[GoogleRow]>
- `MainWindow.google-client-error`: property <bool>
- `MainWindow.google-client-id`: property <string>
- `MainWindow.google-client-note`: property <string>
- `MainWindow.google-client-ready`: property <bool>
- `MainWindow.google-remove`: callback
- `MainWindow.google-save-client`: callback
- `MainWindow.google-sign-in`: callback
- `MainWindow.google-use`: callback
- `MainWindow.key-action`: callback
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
- `MainWindow.side-rows`: property <[SideRow]>
- `MainWindow.sidebar-open`: property <bool>
- `MainWindow.sign-in`: callback
- `MainWindow.status-account`: property <string>
- `MainWindow.status-model`: property <string>
- `MainWindow.status-state`: property <string>
- `MainWindow.status-timing`: property <string>
- `MainWindow.status-tokens`: property <string>
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
- `MainWindow.ui-files`: property <bool>
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
<!-- contract:end -->
