#!/usr/bin/env bash
# Refresh the bundled ZeroClaw WIT contract from upstream.
# Usage: ./scripts/update-wit.sh [REF]
#   REF defaults to main.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
UPSTREAM="https://github.com/zeroclaw-labs/zeroclaw-plugins"
REF="${1:-main}"

TMPDIR="$(mktemp -d)"
trap 'rm -rf "${TMPDIR}"' EXIT

echo "==> Fetching WIT contract from ${UPSTREAM}@${REF} ..."
git clone --quiet --depth 1 --branch "${REF}" --single-branch \
    "${UPSTREAM}.git" "${TMPDIR}/zeroclaw-plugins"

SRC="${TMPDIR}/zeroclaw-plugins/wit/v0"
DST="${PROJECT_DIR}/wit/v0"

if [[ ! -d "${SRC}" ]]; then
    echo "Error: ${UPSTREAM}@${REF} has no wit/v0 directory"
    exit 1
fi

rm -rf "${DST}"
mkdir -p "${DST}"
cp -R "${SRC}/"* "${DST}/"

echo "==> Updated ${DST} from ${UPSTREAM}@${REF}"
