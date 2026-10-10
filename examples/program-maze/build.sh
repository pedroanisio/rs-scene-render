#!/bin/sh
# Builds maze.wasm from maze/ (needs: rustup target add wasm32-unknown-unknown) and prints its SHA-256 for
# program/@sha256 in maze.scene.xml.
set -eu
cd "$(dirname "$0")/maze"
cargo build --release --target wasm32-unknown-unknown
cp target/wasm32-unknown-unknown/release/sr_example_maze.wasm ../maze.wasm
cd ..
sha256sum maze.wasm
