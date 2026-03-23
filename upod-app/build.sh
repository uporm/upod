#!/usr/bin/env bash

# Exit immediately if a command exits with a non-zero status
set -e

# Change to the directory where the script is located to ensure paths are correct
cd "$(dirname "$0")"

echo "====================================="
echo "🚀 Building upod-app (Release Mode)  "
echo "====================================="

# Run cargo build with release profile
cargo build --release

# The compiled binary will be in the workspace target directory
WORKSPACE_ROOT=".."
TARGET_DIR="${WORKSPACE_ROOT}/target/release"
BIN_NAME="upod"
BIN_PATH="${TARGET_DIR}/${BIN_NAME}"

if [ -f "$BIN_PATH" ]; then
    echo "====================================="
    echo "✅ Build Successful!"
    echo "📦 Executable located at: ${BIN_PATH}"
    
    # Copy resources directory to target/release
    if [ -d "resources" ]; then
        cp -r "resources" "${TARGET_DIR}/"
        echo "📂 Copied resources directory to ${TARGET_DIR}/"
    else
        echo "⚠️ Warning: resources directory not found!"
    fi
    
    # Optional: Display the size of the binary
    ls -lh "$BIN_PATH" | awk '{print "📏 Binary Size: " $5}'
    echo "====================================="
else
    echo "====================================="
    echo "❌ Build failed or binary not found!"
    echo "====================================="
    exit 1
fi
