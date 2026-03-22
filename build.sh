#!/usr/bin/env bash
# 开启严格模式：
# -e: 任一命令失败即退出
# -u: 使用未定义变量时报错
# -o pipefail: 管道中任一命令失败则整体失败
set -euo pipefail

# 当前脚本所在目录（绝对路径）
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# 工程根目录（当前脚本放在仓库根目录，所以与 SCRIPT_DIR 相同）
WORKSPACE_DIR="${SCRIPT_DIR}"
# 构建模式：
# - upod: 构建 upod（包含 upod-app 与 upod-bridge）并打包（默认）
# - exec: 仅构建 upod-bridge（兼容旧参数名，始终 Linux 可执行）
# - all : 等同于 upod
BUILD_SCOPE="${1:-upod}"
# 可选参数：upod-app 的 Rust 目标三元组，例如 x86_64-unknown-linux-gnu
TARGET_TRIPLE="${2:-}"
# 兼容旧用法：仅传一个目标三元组时，视为 upod 模式
if [[ "${BUILD_SCOPE}" != "upod" && "${BUILD_SCOPE}" != "exec" && "${BUILD_SCOPE}" != "all" ]]; then
  TARGET_TRIPLE="${BUILD_SCOPE}"
  BUILD_SCOPE="upod"
fi
# 构建时间戳，用于生成唯一发布包名
BUILD_TIME="$(date +%Y%m%d%H%M%S)"
# 当前宿主机操作系统（统一转小写）
HOST_OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
# 当前宿主机 CPU 架构
HOST_ARCH="$(uname -m)"
# 发布目录名示例：upod-darwin-arm64-20260315123045
RELEASE_NAME="upod-${HOST_OS}-${HOST_ARCH}-${BUILD_TIME}"
# 分发根目录
DIST_ROOT="${WORKSPACE_DIR}/dist"
# 当前版本打包目录
PACKAGE_DIR="${DIST_ROOT}/${RELEASE_NAME}"
# 二进制文件输出目录
BIN_DIR="${PACKAGE_DIR}/bin"
# 配置文件输出目录
CONFIG_DIR="${PACKAGE_DIR}/config"

# 创建打包目录结构（目录已存在时不会报错）
mkdir -p "${BIN_DIR}" "${CONFIG_DIR}"

# 切换到工程根目录，确保后续 cargo/cp 使用统一相对基准
cd "${WORKSPACE_DIR}"

build_upod_app() {
  bash "${WORKSPACE_DIR}/upod-app/build.sh" "${TARGET_TRIPLE}" "${PACKAGE_DIR}"
}

build_upod_exec_linux() {
  bash "${WORKSPACE_DIR}/upod-bridge/build.sh" "${TARGET_TRIPLE}" "${PACKAGE_DIR}"
}

package_upod_release() {
  if [[ -d "${BIN_DIR}" ]]; then
    chmod +x "${BIN_DIR}/"* 2>/dev/null || true
  fi
  ARCHIVE_PATH="${DIST_ROOT}/${RELEASE_NAME}.tar.gz"
  tar -C "${DIST_ROOT}" -czf "${ARCHIVE_PATH}" "${RELEASE_NAME}"
  echo "release package: ${ARCHIVE_PATH}"
}

case "${BUILD_SCOPE}" in
  exec)
    build_upod_exec_linux
    ;;
  upod|all)
    build_upod_app
    build_upod_exec_linux
    package_upod_release
    ;;
  *)
    echo "usage: ./build.sh [upod|bridge|all] [target_triple]" >&2
    echo "examples:" >&2
    echo "  ./build.sh" >&2
    echo "  ./build.sh upod aarch64-apple-darwin" >&2
    echo "  ./build.sh bridge" >&2
    exit 1
    ;;
esac
