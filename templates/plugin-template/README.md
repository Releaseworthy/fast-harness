# Fast Harness plugin template

A native plugin for [Fast Harness](https://github.com/Releaseworthy/fast-harness): Rust code compiled into your own build of `fast-harness` and `fast-harness-gui`. A GitHub Action builds the binaries for you, so you need no Rust toolchain or MSVC on your machine.

Most extensions need no build at all: tools, providers, hooks, panels, themes and keys are files under `~/.fast-harness/` (see `docs/extending.md` and `docs/panels.md` in Fast Harness). Use a native plugin only for what files cannot do.

## Use it

1. Press **Use this template** on GitHub and create your repository.
2. Edit `src/lib.rs`: `register` adds your tools, providers, hooks and panels (`fast_harness_core::ext::v1`). Rename the crate in `Cargo.toml` and in `.github/workflows/build.yml` (`name`) if you like.
3. Push. The `build` workflow calls Fast Harness's reusable workflow `build-custom.yml`, which runs the Fast Harness gates (format, clippy, tests) with your plugin in, then builds the release binaries for Windows x86_64 and Linux x86_64.
4. Download the binaries from the run's artifacts. Push a tag (`git tag v0.1.0 && git push --tags`) and they are attached to that tag's release as well.

`fast-harness doctor` lists the registered tools, providers, hooks and panels of the build.

## Inputs of build-custom.yml

- `ref`: the Fast Harness tag or commit to build (default `main`). Pin a tag and use the same tag for `fast-harness-core` in `Cargo.toml`.
- `plugins`: the plugin list (`[[plugin]]` with `name` and `path`, or `git` and `rev`); a relative `path` is relative to this repository.
- `ui`: a folder of this repository whose `.slint` files replace Fast Harness's `crates/fast-harness-gui/ui` files (the UI contract test must still pass).
- `targets`: a JSON list of Rust targets (default `["x86_64-pc-windows-msvc", "x86_64-unknown-linux-gnu"]`).

## API version

`fast_harness_core::ext_api!(1)` and the `ext::v1` path state the plugin API version your code targets. A Fast Harness with another major version fails to compile your plugin with a clear message instead of misbehaving at run time.

The plugin API is unstable for every 0.x release of Fast Harness (its `docs/plugins.md`, "Stability"): pin the Fast Harness revision you build against. The `v1` types are `#[non_exhaustive]`, so later fields and variants do not break your code as long as you build structs with their constructors (`ToolSpec::new(name, description, schema)`, `Answer::default().with_output(text)`, `ProviderFile::new(models)`), never with struct literals, and give every `match` on a `v1` enum a `_` arm. A plugin list entry may say `api = 1`; a build that supports another version stops with a message.

## Safety

A native plugin runs inside the Fast Harness process with your user account's rights: it can read and write your files and reach the network like any program you run. Build only plugins whose source you have read. The workflow builds only what your repository lists, and a `git` plugin must be pinned by `rev`.

Standard GitHub-hosted runners are free for a public repository; a private repository uses its owner's included minutes.
