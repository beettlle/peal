# peal

Plan-Execute-Address Loop: orchestrator that drives the Cursor CLI in three phases per task.

## Build and run

From the repository root:

```bash
cargo build
cargo run -- --help
```

See [docs/building.md](docs/building.md) for build requirements, release build, and platforms.

## Platforms / Distribution

Supported target triples: `x86_64-pc-windows-msvc`, `x86_64-apple-darwin`, `aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`. Single binary per platform; see [configuration](docs/configuration.md) for full platform details.

## Documentation

- [Building](docs/building.md) — build requirements, platforms, build-all-targets script
- [Configuration](docs/configuration.md) — config file, supported platforms, Cursor CLI contract
