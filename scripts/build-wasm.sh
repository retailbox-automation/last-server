#!/usr/bin/env bash
# Builds crates/ls-wasm and generates the browser bindings into web/src/wasm/ (not committed).
# Needs: Rust with the wasm32-unknown-unknown target and wasm-bindgen-cli 0.2.130 (must match the crate).
#   cargo install --locked wasm-bindgen-cli --version 0.2.130
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
OUT="$ROOT/web/src/wasm"

# Optional shared-machine wrapper (one heavy job at a time); a plain checkout just runs cargo.
HEAVY="${LS_HEAVY_WRAPPER:-$HOME/.local/share/last-server-toolchain/heavy.sh}"
run() { if [ -x "$HEAVY" ]; then "$HEAVY" "$@"; else "$@"; fi; }

cd "$ROOT"
# strip=true in the workspace profile (for the contract) would hide the target-features section from wasm-bindgen.
CARGO_PROFILE_RELEASE_STRIP=none run cargo build -p ls-wasm --target wasm32-unknown-unknown --release
mkdir -p "$OUT"
wasm-bindgen --target web --remove-name-section --remove-producers-section --out-dir "$OUT" --out-name ls_wasm \
  "$TARGET_DIR/wasm32-unknown-unknown/release/ls_wasm.wasm"
echo "wasm bindings written to $OUT"
