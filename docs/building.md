# Building peal

## Build requirements

- **Rust:** Minimum supported Rust version (MSRV) is **1.93** or newer. The 2024 edition is required; it was stabilized in Rust 1.85. The canonical source of truth is the `rust-version` field in [Cargo.toml](../Cargo.toml) at the repo root.

- **Toolchain (optional):** A [rust-toolchain.toml](../rust-toolchain.toml) at the repo root pins the expected Rust version for this repo so `rustup` uses it by default when working in this directory.

## Build and test

From the repository root:

```bash
cargo build
cargo test
```

Release build:

```bash
cargo build --release
```

## Platforms / Distribution

Supported Rust target triples: `x86_64-pc-windows-msvc`, `x86_64-apple-darwin`, `aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`. Single binary per platform; no Python/Node at runtime. Full details (OS summary, distribution, Cursor CLI caveats): [Supported platforms and targets](configuration.md#supported-platforms-and-targets).

To build for all supported targets locally, run `./scripts/build-all-targets.sh` (requires rustup and any cross-compilation tooling for non-native targets).

**Windows:** If the Cursor CLI (`agent`) is not found on PATH, set `agent_cmd` to the full executable name (e.g. `agent.exe`) or an absolute path to the executable.
