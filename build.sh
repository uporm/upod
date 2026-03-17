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
  if [[ -n "${TARGET_TRIPLE}" ]]; then
    cargo build --release --target "${TARGET_TRIPLE}" -p upod-app
    APP_BUILD_OUTPUT_DIR="${WORKSPACE_DIR}/target/${TARGET_TRIPLE}/release"
  else
    cargo build --release -p upod-app
    APP_BUILD_OUTPUT_DIR="${WORKSPACE_DIR}/target/release"
  fi
}

build_upod_exec_linux() {
  bash "${WORKSPACE_DIR}/upod-bridge/build.sh" "${TARGET_TRIPLE}"
}

resolve_upod_exec_bin() {
  local candidates=(
    "${WORKSPACE_DIR}/target/release/upod-bridge"
    "${WORKSPACE_DIR}/target/x86_64-unknown-linux-gnu/release/upod-bridge"
    "${WORKSPACE_DIR}/target/aarch64-unknown-linux-gnu/release/upod-bridge"
  )
  local candidate
  for candidate in "${candidates[@]}"; do
    if [[ -f "${candidate}" ]]; then
      EXEC_BUILD_OUTPUT_DIR="$(dirname "${candidate}")"
      return 0
    fi
  done
  echo "error: upod-bridge artifact not found after build" >&2
  exit 1
}

package_upod_release() {
  cp "${APP_BUILD_OUTPUT_DIR}/upod" "${BIN_DIR}/upod"
  cp "${EXEC_BUILD_OUTPUT_DIR}/upod-bridge" "${BIN_DIR}/upod-bridge"
  cp "${WORKSPACE_DIR}/upod-app/resources/application.toml" "${CONFIG_DIR}/application.toml"
  cp -R "${WORKSPACE_DIR}/upod-app/resources/locales" "${CONFIG_DIR}/locales"
  chmod +x "${BIN_DIR}/upod" "${BIN_DIR}/upod-bridge"
  ARCHIVE_PATH="${DIST_ROOT}/${RELEASE_NAME}.tar.gz"
  tar -C "${DIST_ROOT}" -czf "${ARCHIVE_PATH}" "${RELEASE_NAME}"
  echo "release package: ${ARCHIVE_PATH}"
}

case "${BUILD_SCOPE}" in
  exec)
    build_upod_exec_linux
    resolve_upod_exec_bin
    echo "upod-bridge built: ${EXEC_BUILD_OUTPUT_DIR}/upod-bridge"
    ;;
  upod|all)
    build_upod_app
    build_upod_exec_linux
    resolve_upod_exec_bin
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
