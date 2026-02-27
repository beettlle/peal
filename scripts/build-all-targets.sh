#!/usr/bin/env bash
# Build release binaries for all supported target triples (PRD §13).
# Requires: rustup, and cross-compilation tooling for non-native targets.
# Run from repository root.

set -euo pipefail

TARGETS=(
  x86_64-pc-windows-msvc
  x86_64-apple-darwin
  aarch64-apple-darwin
  x86_64-unknown-linux-gnu
)

for t in "${TARGETS[@]}"; do
  rustup target add "$t"
done

for t in "${TARGETS[@]}"; do
  echo "Building --release --target $t ..."
  cargo build --release --target "$t"
done

echo "Done. Release artifacts under target/<triple>/release/"
