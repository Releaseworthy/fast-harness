# native-harness

A barebone coding agent harness in native Rust: an agent loop with tools, Claude as the model, a trace of every turn and every step, a terminal UI and a desktop GUI. Windows first; the core is portable.

Status: in development. See [PLAN.md](PLAN.md) for the design, the decisions and the progress.

## Build

```
cargo build --release
cargo test --workspace
```

## Licence

MIT OR Apache-2.0, at your option. The GUI uses [Slint](https://slint.dev) under its royalty-free licence.
