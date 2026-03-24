#!/bin/bash
set -e

echo "Checking Android (aarch64)..."
cargo ndk -t arm64-v8a check

echo "Checking macOS (Apple Silicon)..."
cargo check --target aarch64-apple-darwin

echo "Checking Web (WASM)..."
cargo check --target wasm32-unknown-unknown

echo "All checks passed!"
