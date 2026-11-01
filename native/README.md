# Tomlook native crate

This crate is the whole Tomlook application: a native Windows calendar with
an AI assistant, built with egui/eframe and the official Rust GitHub Copilot
SDK. User documentation starts at the [root README](../README.md).

## Contents

- `src\main.rs` – the `tomlook` executable (also built as
  `tomlook-acceptance` for measurements).
- `src\lib.rs` and modules – storage worker, AI executor, connectors
  (WorkIQ, WebIQ, GitHub, Foundry, MSX), scheduler, reminders, updater.
- `src\app\` – the egui user interface.
- `src\bin\` – diagnostic probes, fixture seeders and the `release-sign` tool.
- `tests\` – integration tests; live checks are `#[ignore]`.
- `connections.example.json`, `decision-router.example.json` – templates for
  the profile's Copilot and Foundry configuration
  ([setup](../docs/setup.md)).

## Build and test

Rust 1.95+ and the Visual Studio x64 C++ toolchain are required; run cargo
inside `vcvars64.bat` (see [development.md](../docs/development.md) for a
helper). Always use the lockfile and the offline cache:

```text
cargo test --locked --offline
cargo clippy --locked --offline --all-targets -- -D warnings
cargo build --release --locked --offline --bin tomlook
```

Try the UI without any account:

```text
target\debug\tomlook.exe --demo --state <new-empty-directory>
```

## Documentation

- [Architecture](../ARCHITECTURE.md) – threads, modules, pipelines
- [Development](../docs/development.md) – tests, demo scenes, probes, releases
- [User guide](../docs/user-guide.md) and [setup](../docs/setup.md)
- [Privacy and safety](../docs/privacy-and-safety.md) – approvals, tokens, AI boundaries
