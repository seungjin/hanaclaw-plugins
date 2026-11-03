#!/usr/bin/env bash
# Build, package, and emit a ZeroClaw tool plugin release locally.
# Usage: ./scripts/release.sh
# Outputs to dist/:
#   <name>-<version>.zip
#   <name>-<version>.zip.sha256
#   <name>-<version>.zip.minisig   (if minisign is available)
set -euo pipefail

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
SHA_PATH="${ZIP_PATH}.sha256"
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
rm -f "${ZIP_PATH}" "${SHA_PATH}" "${ZIP_PATH}.minisig"
python3 - "${NAME}" "${ZIP_PATH}" manifest.toml "${WASM_SRC}" "${WASM_PATH}" <<'PY'
import os, sys, zipfile

name, zip_path, manifest_src, wasm_src, wasm_name = sys.argv[1:]
# Fixed timestamp: 1980-01-01 00:00 UTC, same as upstream reproducible zips.
FIXED_TIME = (1980, 1, 1, 0, 0, 0)

with zipfile.ZipFile(zip_path, 'w', compression=zipfile.ZIP_DEFLATED) as zf:
    for src, arcname in [(manifest_src, f"{name}/manifest.toml"),
                         (wasm_src, f"{name}/{wasm_name}")]:
        zf.write(src, arcname)
        info = zf.getinfo(arcname)
        info.date_time = FIXED_TIME
PY

# --- checksum ---
SHA256="$(sha256sum "${ZIP_PATH}" | awk '{print $1}')"
echo "${SHA256}  ${ZIP_NAME}" > "${SHA_PATH}"
echo "==> sha256: ${SHA256}"

# --- sign (optional) ---
if command -v minisign >/dev/null 2>&1; then
    echo "==> Signing with minisign ..."
    minisign -Sm "${ZIP_PATH}" -x "${ZIP_PATH}.minisig"
else
    echo "==> minisign not found; skipping signature. Install minisign to produce .minisig files."
fi

# --- print registry snippet ---
echo ""
echo "==> Release artifacts in ${PROJECT_DIR}/dist/:"
ls -la "dist/"
echo ""
echo "==> Registry entry snippet (update description/author/capabilities as needed):"
cat <<JSON
{
  "name": "${NAME}",
  "version": "${VERSION}",
  "description": "UPDATE_ME",
  "author": "UPDATE_ME",
  "capabilities": ["tool"],
  "url": "UPDATE_ME",
  "sha256": "${SHA256}"
}
JSON
