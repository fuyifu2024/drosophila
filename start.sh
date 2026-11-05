#!/bin/sh
# Linux (x86_64/ARM64) and macOS (Intel/Apple Silicon).
set -eu
cd "$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
if [ -x ./drosophila ]; then
  BIN=./drosophila
else
  command -v cargo >/dev/null 2>&1 || { echo 'Install Rust 1.85+ from https://rustup.rs or download a release.' >&2; exit 1; }
  cargo build --release --locked
  BIN=./target/release/drosophila
fi
exec "$BIN" run --seconds 0 --serve 8770 --open "$@"
