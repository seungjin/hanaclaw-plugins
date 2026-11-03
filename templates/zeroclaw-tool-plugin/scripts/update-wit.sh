#!/usr/bin/env bash
# Update the bundled wit/v0 contract from zeroclaw-labs/zeroclaw-plugins@main.
# Usage: ./scripts/update-wit.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${PROJECT_DIR}"

WIT_DIR="wit/v0"
UPSTREAM_URL="https://github.com/zeroclaw-labs/zeroclaw-plugins"
UPSTREAM_BRANCH="main"

echo "==> Updating ${WIT_DIR} from ${UPSTREAM_URL}@${UPSTREAM_BRANCH} ..."

TMPDIR="$(mktemp -d)"
trap 'rm -rf "${TMPDIR}"' EXIT

TARBALL="${TMPDIR}/upstream.tar.gz"
curl -fsSL "${UPSTREAM_URL}/archive/refs/heads/${UPSTREAM_BRANCH}.tar.gz" -o "${TARBALL}"

EXTRACT_DIR="${TMPDIR}/wit"
mkdir -p "${EXTRACT_DIR}"
tar -xzf "${TARBALL}" -C "${EXTRACT_DIR}" --strip-components=2 "zeroclaw-plugins-${UPSTREAM_BRANCH}/wit/v0"

if [[ ! -d "${EXTRACT_DIR}/v0" ]]; then
    echo "Error: could not extract wit/v0 from upstream tarball"
    exit 1
fi

# Preserve local files we intentionally keep, if any, by replacing only upstream files.
rm -rf "${WIT_DIR}"
mkdir -p "$(dirname "${WIT_DIR}")"
mv "${EXTRACT_DIR}/v0" "${WIT_DIR}"

echo "==> Updated ${WIT_DIR}:"
ls -1 "${WIT_DIR}"
