#!/bin/sh
# Copyright 2026 Petri Koistinen. Licensed under the Apache License, Version 2.0.
# Build the WASM requester bundle into extension/wasm/ (web target for
# the module service worker). Requires cargo, the wasm32 target, and a
# wasm-bindgen matching wasm-requester/Cargo.lock.
set -eu
root=$(dirname "$0")/..
cd "$root/wasm-requester"
cargo build --target wasm32-unknown-unknown --release
mkdir -p ../extension/wasm
wasm-bindgen --target web --out-dir ../extension/wasm \
  target/wasm32-unknown-unknown/release/refineid_wasm_requester.wasm
echo "wrote extension/wasm/"
