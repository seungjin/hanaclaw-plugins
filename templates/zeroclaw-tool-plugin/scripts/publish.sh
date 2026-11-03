#!/usr/bin/env bash
# Publish a standalone ZeroClaw tool plugin to a GitHub Release.
# Usage: ./scripts/publish.sh [RELEASE_TAG]
# Default RELEASE_TAG is "plugins".
set -euo pipefail

RELEASE_TAG="${1:-plugins}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${PROJECT_DIR}"

# --- read manifest ---
if [[ ! -f manifest.toml ]]; then
    echo "Error: manifest.toml not found in ${PROJECT_DIR}"
    exit 1
fi

NAME="$(sed -n 's/^name *= *"\(.*\)"/\1/p' manifest.toml | head -1)"
VERSION="$(sed -n 's/^version *= *"\(.*\)"/\1/p' manifest.toml | head -1)"
WASM_PATH="$(sed -n 's/^wasm_path *= *"\(.*\)"/\1/p' manifest.toml | head -1)"

if [[ -z "${NAME}" || -z "${VERSION}" || -z "${WASM_PATH}" ]]; then
    echo "Error: could not parse name, version, or wasm_path from manifest.toml"
    exit 1
fi

ZIP_NAME="${NAME}-${VERSION}.zip"
ZIP_PATH="dist/${ZIP_NAME}"
mkdir -p dist

# --- build wasm ---
echo "==> Building ${NAME} v${VERSION} ..."
cargo build --target wasm32-wasip2 --release

WASM_SRC="target/wasm32-wasip2/release/${WASM_PATH}"
if [[ ! -f "${WASM_SRC}" ]]; then
    echo "Error: expected wasm at ${WASM_SRC}"
    exit 1
fi

# --- package zip (deterministic layout, fixed timestamp) ---
# Layout: <name>/manifest.toml + <name>/<wasm_path>
echo "==> Packaging ${ZIP_PATH} ..."
rm -f "${ZIP_PATH}"
python3 - "${NAME}" "${ZIP_PATH}" manifest.toml "${WASM_SRC}" "${WASM_PATH}" <<'PY'
import os, sys, zipfile

name, zip_path, manifest_src, wasm_src, wasm_name = sys.argv[1:]
# Fixed timestamp: 1980-01-01 00:00 UTC, same as upstream reproducible zips.
FIXED_TIME = (1980, 1, 1, 0, 0, 0)

with zipfile.ZipFile(zip_path, 'w', compression=zipfile.ZIP_DEFLATED) as zf:
    for src, arcname in [(manifest_src, f"{name}/manifest.toml"),
                         (wasm_src, f"{name}/{wasm_name}")]:
        zf.write(src, arcname)
        # Force deterministic date/time.
        info = zf.getinfo(arcname)
        info.date_time = FIXED_TIME
PY

SHA256="$(sha256sum "${ZIP_PATH}" | awk '{print $1}')"
echo "==> sha256: ${SHA256}"

# --- upload to GitHub release ---
if command -v gh >/dev/null 2>&1 && gh auth status >/dev/null 2>&1; then
    echo "==> Ensuring GitHub release '${RELEASE_TAG}' exists ..."
    gh release view "${RELEASE_TAG}" >/dev/null 2>&1 || \
        gh release create "${RELEASE_TAG}" --title "Plugin artifacts" --notes "ZeroClaw plugin artifacts"

    echo "==> Uploading ${ZIP_NAME} to release '${RELEASE_TAG}' ..."
    gh release upload "${RELEASE_TAG}" "${ZIP_PATH}" --clobber

    REMOTE_URL="$(git remote get-url origin 2>/dev/null || true)"
    if [[ -n "${REMOTE_URL}" ]]; then
        # Convert git@host:owner/repo.git to https://host/owner/repo
        REPO_URL="$(echo "${REMOTE_URL}" | sed -E 's|^git@([^:]+):|https://\1/|; s|\.git$||')"
        ASSET_URL="${REPO_URL}/releases/download/${RELEASE_TAG}/${ZIP_NAME}"
        echo ""
        echo "==> Published asset:"
        echo "    ${ASSET_URL}"
        echo ""
        echo "==> Registry entry snippet:"
        cat <<JSON
{
  "name": "${NAME}",
  "version": "${VERSION}",
  "description": "UPDATE_ME",
  "author": "UPDATE_ME",
  "capabilities": ["tool"],
  "url": "${ASSET_URL}",
  "sha256": "${SHA256}"
}
JSON
    fi
else
    echo ""
    echo "==> gh is not authenticated; upload skipped."
    echo "    Zip: ${ZIP_PATH}"
    echo "    sha256: ${SHA256}"
    echo "    Upload manually with:"
    echo "      gh release upload ${RELEASE_TAG} ${ZIP_PATH}"
fi
