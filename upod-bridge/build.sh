#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
TARGET_TRIPLE="${1:-}"
HOST_OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
HOST_ARCH="$(uname -m)"

EXEC_TARGET_ARCH="${TARGET_TRIPLE%%-*}"
if [[ -z "${EXEC_TARGET_ARCH}" ]]; then
  EXEC_TARGET_ARCH="${HOST_ARCH}"
fi
case "${EXEC_TARGET_ARCH}" in
  arm64) EXEC_TARGET_ARCH="aarch64" ;;
  amd64) EXEC_TARGET_ARCH="x86_64" ;;
esac
case "${EXEC_TARGET_ARCH}" in
  aarch64) EXEC_DOCKER_ARCH="arm64" ;;
  x86_64) EXEC_DOCKER_ARCH="amd64" ;;
  *)
    echo "error: unsupported exec target arch: ${EXEC_TARGET_ARCH}" >&2
    exit 1
    ;;
esac
EXEC_TARGET_TRIPLE="${EXEC_TARGET_ARCH}-unknown-linux-gnu"

cd "${WORKSPACE_DIR}"

if [[ "${HOST_OS}" == "linux" ]]; then
  cargo build --release --target "${EXEC_TARGET_TRIPLE}" -p upod-bridge
  EXEC_BUILD_OUTPUT_DIR="${WORKSPACE_DIR}/target/${EXEC_TARGET_TRIPLE}/release"
else
  if command -v docker >/dev/null 2>&1; then
    docker run --rm \
      --platform "linux/${EXEC_DOCKER_ARCH}" \
      -v "${WORKSPACE_DIR}:${WORKSPACE_DIR}" \
      -w "${WORKSPACE_DIR}" \
      rust:1 \
      cargo build --release -p upod-bridge
    EXEC_BUILD_OUTPUT_DIR="${WORKSPACE_DIR}/target/release"
  else
    echo "error: docker is required to build Linux upod-bridge on non-linux hosts" >&2
    exit 1
  fi
fi

echo "upod-bridge built: ${EXEC_BUILD_OUTPUT_DIR}/upod-bridge"
