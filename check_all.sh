#!/bin/bash
set -e

echo "Checking Android (aarch64)..."
cargo check --target aarch64-linux-android

echo "Checking macOS (Apple Silicon)..."
cargo check --target aarch64-apple-darwin

echo "Checking Web (WASM)..."
cargo check --target wasm32-unknown-unknown

echo "All checks passed!"
