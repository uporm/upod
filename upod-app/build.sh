#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(dirname "${SCRIPT_DIR}")"

TARGET_TRIPLE="${1:-}"
PACKAGE_DIR="${2:-}"

cd "${WORKSPACE_DIR}"

if [[ -n "${TARGET_TRIPLE}" ]]; then
  cargo build --release --target "${TARGET_TRIPLE}" -p upod-app
  RELEASE_DIR="${WORKSPACE_DIR}/target/${TARGET_TRIPLE}/release"
else
  cargo build --release -p upod-app
  RELEASE_DIR="${WORKSPACE_DIR}/target/release"
fi

UPOD_OUT_DIR="${RELEASE_DIR}/upod"
mkdir -p "${UPOD_OUT_DIR}"

cp "${RELEASE_DIR}/upod" "${UPOD_OUT_DIR}/upod"
cp -R "${SCRIPT_DIR}/resources" "${UPOD_OUT_DIR}/resources"

if [[ -n "${PACKAGE_DIR}" ]]; then
  mkdir -p "${PACKAGE_DIR}/bin" "${PACKAGE_DIR}/config"
  cp "${UPOD_OUT_DIR}/upod" "${PACKAGE_DIR}/bin/upod"
  cp "${UPOD_OUT_DIR}/resources/application.toml" "${PACKAGE_DIR}/config/application.toml"
  cp -R "${UPOD_OUT_DIR}/resources/locales" "${PACKAGE_DIR}/config/locales"
fi

