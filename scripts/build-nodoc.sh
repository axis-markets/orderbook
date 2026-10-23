#!/usr/bin/env bash
# Builds the contract with every doc comment stripped from the wasm spec.
# The sources are copied to target/nodoc, doc comments (/// and //!) are removed
# there and the copy is built in its own target directory (sharing the regular
# one mixes up the cargo fingerprints of the two builds), so the working tree is
# never touched; target/nodoc is removed afterwards. The wasm lands at the
# default path (target/wasm32v1-none/release).
# Usage: scripts/build-nodoc.sh
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
work="$root/target/nodoc"

rm -rf "$work"
mkdir -p "$work"
trap 'rm -rf "$work"' EXIT
cp "$root/Cargo.toml" "$root/Cargo.lock" "$work/"
cp -r "$root/src" "$work/src"
find "$work/src" -name '*.rs' -exec sed -i -E '/^[[:space:]]*\/\/[\/!]/d' {} +

stellar contract build --optimize --manifest-path "$work/Cargo.toml" \
  --out-dir "$root/target/wasm32v1-none/release"
