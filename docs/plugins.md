# Native plugins and the build workflow

PLAN.md D52, docs/efficiency-plan.md phase 21C. Most extensions are files and need no build (docs/extending.md, docs/panels.md, docs/ui-contract.md). A native plugin is for what files cannot do: a tool, provider, hook or panel written in Rust, compiled into a custom build. The official binaries are built with an empty plugin list and carry no plugin code.

## The API (`fast_harness_core::ext`)

A plugin crate depends on `fast-harness-core` and exposes one function:

```rust
use fast_harness_core::ext::v1::*;

fast_harness_core::ext_api!(1);

pub fn register(ext: &mut Ext) {
    ext.add_tool(Box::new(MyTool));                    // Box<dyn Tool>
    ext.add_provider(Arc::new(MyProvider), file);       // Arc<dyn Provider> + ProviderFile (models)
    ext.add_hook(Box::new(MyHook));                     // in-process hook
    ext.add_panel(Box::new(MyPanel));                   // in-process panel
}
```

- Tools join every session's registry after the built-ins, deferred behind `find_tools` like every tool (D30); a built-in of the same name wins; a `tools/<name>.json` override file applies (description, `disabled`). `Tool::effect` (default `Effect::Run`) decides how its calls run within a step (D54): a tool that only reads returns `Effect::Read` and runs alongside the other read calls; any other call runs alone, in call order.
- Providers join the custom providers (D41): models are `<provider id>/<model id>`, the `ProviderFile` gives the model entries (window, max output, efforts, price) and the credential (`key_env`, `account`, or none). A `providers/<id>.json` file with the same id wins; the ids `anthropic` and `antigravity` are ignored.
- Hooks (`Hook`, the trait `tools::hooks::NativeHook`): the seven events of D44 (`SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `PreCompact`, `Stop`, `SubagentStop`), `matches(event, tool)`, and `run(event, input) -> Result<Answer, String>` with the same input object and answer as a `hooks.json` command. They run after the file entries of their event and are traced as `native:<name>`; an error is ignored (fail open).
- Panels (`Panel`, the trait `panels::NativePanel`): `open`, `event` (`click`, `submit`, `change`) and `session` answer with the messages a panel process prints (`Incoming::View`, `Patch`, `Subscribe`, `Notify`), so the GUI and the TUI draw them like `panels/*.json` panels (D46), `/panel <id>` opens one, and a panel file with the same id wins. Nothing runs before the panel opens.

`fast-harness doctor` prints `native plugins: tools ...; providers ...; hooks ...; panels ...` when a build has any.

### Version

The API's major version is part of the path: `fast_harness_core::ext::v1`, `ext::API_VERSION == 1`. A breaking change adds `ext::v2` and removes `v1`, so a plugin written for another major version names a module that does not exist and does not compile. `ext_api!(1)` states the version in the plugin and turns a mismatch into a compile error with a readable message (doc tests with `compile_fail` in `ext/mod.rs` check both). Additions that break no plugin stay in `v1`.

### Stability

The plugin API is unstable for every 0.x release (owner decision G9, PLAN.md D57): a 0.x release may change `v1`, each change is listed below, and a plugin pins the Fast Harness `rev` it builds against. What keeps most changes from breaking a plugin:

- Every struct with public fields and every enum that `ext::v1` re-exports is `#[non_exhaustive]`, so a field or a variant can be added later. A plugin builds a struct through its constructor (`ToolSpec::new(name, description, schema)`, `Completion::new(message, usage, stop)`, `Answer::default().with_output(text)`, `ProviderFile::new(models)`, `ModelFile::new(id)`, `Usage::new(...)`, `ChatRequest::new(...)`, `Host::new(cwd, session, dark)`), may set public fields on a value it holds, and matches an enum (`AgentEvent`, `HookEvent`, `Block`, `Delta`, `StopReason`, ...) with a wildcard arm. A struct literal or a match without `_` does not compile (doc tests with `compile_fail` in `ext/v1.rs`).
- A method added to `Tool`, `Provider`, `Hook`, `Panel` or `Transport` always has a default body.
- The fields of an enum variant are fixed (variants are not `#[non_exhaustive]`, so a plugin may build and destructure them): new data comes as a new variant, never as a field added to one.
- `ToolOutput` keeps its fields private: a plugin builds it with `ok`, `err`, `new` or `from_parts` and reads it with `text()`, `parts()`, `is_error()`, `bytes()`, `spilled()`, so a later part kind (an image) changes no plugin.
- Each `[[plugin]]` entry may say `api = 1` (below): the build fails when it is not `ext::API_VERSION`.

Changes to `v1`: 2026-11-13 (D57) the types became `#[non_exhaustive]`, with constructors, and `v1` re-exports every type its items reach (`Message`, `Block`, `Role`, `Usage`, `StopReason`, `Timing`, `Cache`, `ErrorKind`, `HttpHead`, `HttpRequest`, `HttpError`, `HttpErrorKind`, `Flow`, `Price`, `CompactionPatch`, `TurnOutcome`, `ToolSession`). 2026-11-13 (D58) tool progress and output parts: `ToolCtx::progress(text)` (with `with_progress`, `reports_progress` and the `Progress` type) sends live text that front ends get as `AgentEvent::ToolProgress { id, chunk }` (never in the trace or a request); `ToolOutput` holds `Part`s (`#[non_exhaustive] enum Part { Text(String) }`, re-exported) behind private fields, read through `text()` (the text parts joined), `into_text()`, `parts()`, `is_error()`, `bytes()`, `spilled()`, built with `ok`, `err`, `new` and the new `from_parts`.

## The plugin list

`plugins.toml` at the workspace root (absent in this repository):

```toml
[[plugin]]
name = "my-plugin"                       # the package name, also the dependency key
git = "https://github.com/me/my-plugin"
rev = "0123abcd"                         # required with git: builds are pinned
api = 1                                  # optional: the plugin API it targets

[[plugin]]
name = "local-thing"
path = "plugins/local-thing"             # relative to the workspace root (or --base)
```

`api` (a bare whole number or a quoted one; `"api": 1` in the JSON form) is optional and reads as 1 when absent. A value other than `fast_harness_core::ext::API_VERSION` fails `build.rs` and `apply-plugins.ts` with a message naming the plugin, its value and the supported version. The number has three copies (the core, `gen::API`, the script); a test in `crates/fast-harness-plugins/src/tests.rs` checks they agree.

The file is a small TOML subset (`[[plugin]]`, `key = "plain string"`, `api = 1`, `#` comments) read by hand in `crates/fast-harness-plugins/src/gen.rs`: the `toml` crate is in `Cargo.lock` only as a dependency of `slint-build`, and naming it would add a crate to `[workspace.dependencies]` (D1).

Cargo cannot add dependencies from a build script, so two steps turn the list into code:

1. `bun scripts/apply-plugins.ts [--from FILE] [--base DIR]` reads the list (TOML, or JSON `[{"name","git","rev"}]`; `--from` also rewrites `plugins.toml`), writes one dependency per plugin between `# BEGIN PLUGINS` and `# END PLUGINS` in `crates/fast-harness-plugins/Cargo.toml`, and, when the list is not empty, a `[patch."https://github.com/Releaseworthy/fast-harness"]` entry in the root `Cargo.toml` so a plugin's git dependency on `fast-harness-core` is this checkout's core (one `Ext` type). A relative `path` becomes absolute against `--base`.
2. `crates/fast-harness-plugins/build.rs` parses `plugins.toml`, fails the build when the dependency keys between the markers differ from the list ("run `bun scripts/apply-plugins.ts`"), and writes `register_all(ext)` calling `::<crate>::register(ext)` per plugin in file order, plus `PLUGINS`. No file or an empty list: `pub fn register_all(_ext: &mut Ext) {}`.

The binaries call `fast_harness_plugins::install()` at start (CLI `main`, GUI `start_runtime`): it runs `register_all` and installs the result with `ext::install` when it is not empty. The core never depends on the plugins crate; `Runtime::apply` reads `ext::installed()` on every settings load.

Restore the official state with an empty list: `bun scripts/apply-plugins.ts --from <empty file>`, then delete `plugins.toml`. The next cargo command (`cargo metadata` is enough) drops the plugin from `Cargo.lock`.

## The workflows

- `.github/workflows/build-custom.yml`, reusable (`workflow_call`) and runnable by hand (`workflow_dispatch`), as documented in [Reuse workflows](https://docs.github.com/en/actions/how-tos/reuse-automations/reuse-workflows). Inputs: `ref` (Fast Harness tag or commit, default `main`), `plugins` (TOML or JSON text; a relative `path` is relative to the caller repository), `ui` (a folder of the caller repository whose files are copied over `crates/fast-harness-gui/ui`; the UI contract test runs with the gates), `targets` (JSON list, default `x86_64-pc-windows-msvc` and `x86_64-unknown-linux-gnu`). Each target runs on its own runner: check out Fast Harness at `ref` and the caller next to it, write the list with the script, copy the UI files, run `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, build `fast-harness` and `fast-harness-gui` in release, upload them (with `plugins.toml` and the Fast Harness commit) as the artifact `fast-harness-<target>`, and, when the caller ran on a tag, attach an archive to the caller's release with `gh release upload` and the caller's `GITHUB_TOKEN` (the caller grants `contents: write`).
- `.github/workflows/ci.yml`: the same gates on Windows and Linux for every push and pull request to `main`.
- `.github/workflows/release.yml`: a `v*` tag calls `build-custom.yml` with an empty plugin list: the official release binaries.

The template repository (`templates/plugin-template/`): one plugin crate with an example tool (`word_count`) and panel (`counter`), a workflow of about ten lines calling `build-custom.yml@main`, and a README.

First run, 2026-11-12 (run 43220560727 of `Releaseworthy/fast-harness-plugin-template`, Fast Harness `main` at a143cfa, the template's `my-plugin` as a path plugin): green on both targets. `windows-latest` has the MSVC toolchain `ring` needs; the Linux package list is enough for Slint; the action versions (`actions/checkout@v7`, `actions/upload-artifact@v7`, `oven-sh/setup-bun@v2`, `dtolnay/rust-toolchain@stable`, `Swatinem/rust-cache@v2`) resolve. Wall time with a cold cache, gates included: Windows 17 min 16 s, Linux 10 min 31 s. Artifacts: Windows 16,543,973 bytes, Linux 17,940,362 bytes (zipped). Its first attempt failed two tests that assumed the official build (fixed in a143cfa).

## Safety

A native plugin runs inside the process with the user's rights. The workflow builds only what the caller's repository lists, and a `git` plugin must be pinned by `rev` (the generator and the script refuse one without it). Standard hosted runners are free for a public repository; a private one uses its owner's included minutes.
