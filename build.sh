#!/usr/bin/env bash

# 遇到错误立即退出
set -e

# 切换到脚本所在目录（项目根目录）
cd "$(dirname "$0")"

echo "====================================="
echo "🚀 Starting Full Build Process"
echo "====================================="

# 1. 准备 dist 目录
echo "=> Preparing dist directory..."
rm -rf dist
mkdir -p dist

# 2. 构建 upod-bridge
echo "=> Building upod-bridge..."
bash ./upod-bridge/build.sh

# 3. 构建 upod-app
echo "=> Building upod-app..."
bash ./upod-app/build.sh

# 4. 收集产物到 dist 目录
echo "====================================="
echo "📦 Collecting artifacts into dist/..."
echo "====================================="

# 复制 upod
if [ -f "target/release/upod" ]; then
    cp "target/release/upod" dist/
    echo "✅ Copied upod"
else
    echo "❌ Error: upod binary not found!"
    exit 1
fi

# 复制 resources
if [ -d "target/release/resources" ]; then
    cp -r "target/release/resources" dist/
    echo "✅ Copied resources"
else
    echo "❌ Error: resources directory not found!"
    exit 1
fi

# 复制 upod-bridge
# upod-bridge 的构建产物在 target/<target-triple>/release/upod-bridge，因此通过 find 动态获取路径
BRIDGE_BIN=$(find target -type f -path "*/release/upod-bridge" | head -n 1)
if [ -n "$BRIDGE_BIN" ] && [ -f "$BRIDGE_BIN" ]; then
    cp "$BRIDGE_BIN" dist/
    echo "✅ Copied upod-bridge"
else
    echo "❌ Error: upod-bridge binary not found!"
    exit 1
fi

echo "====================================="
echo "🎉 All builds successful! Artifacts are in the dist/ directory:"
ls -lh dist/
echo "====================================="
