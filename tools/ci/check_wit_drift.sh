#!/usr/bin/env bash
# Require wit/v0/ to be byte-identical to zeroclaw-labs/zeroclaw's wit/v0 at
# the commit named by wit/v0/UPSTREAM_REF, and the local .sha256 to match.
#
#   tools/ci/check_wit_drift.sh
#
# Offline parts run first (pin format + .sha256), so a corrupted checkout
# fails without a network round trip. Needs git and network access.
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
repository_root=$(cd -- "$script_dir/../.." && pwd -P)
cd "$repository_root"

upstream_url=https://github.com/zeroclaw-labs/zeroclaw.git
pinned_ref=$(tr -d '[:space:]' < wit/v0/UPSTREAM_REF)
if [[ ! "$pinned_ref" =~ ^[0-9a-f]{40}$ ]]; then
  echo "::error file=wit/v0/UPSTREAM_REF::UPSTREAM_REF must be one 40-hex commit, got '$pinned_ref'"
  exit 1
fi

# Offline: every vendored file still hashes to its recorded value.
python3 tools/fetch_zeroclaw_wit.py --check

workdir=$(mktemp -d)
trap 'rm -rf -- "$workdir"' EXIT
git init --quiet "$workdir/upstream"
git -C "$workdir/upstream" remote add origin "$upstream_url"
git -C "$workdir/upstream" sparse-checkout init --cone
git -C "$workdir/upstream" sparse-checkout set wit/v0
git -C "$workdir/upstream" -c protocol.version=2 fetch \
  --quiet --no-tags --depth=1 --filter=blob:none origin "$pinned_ref"
fetched=$(git -C "$workdir/upstream" rev-parse FETCH_HEAD)
if [[ "$fetched" != "$pinned_ref" ]]; then
  echo "::error::Fetched commit $fetched does not match pin $pinned_ref"
  exit 1
fi
git -C "$workdir/upstream" checkout --quiet --detach FETCH_HEAD

# .sha256 and UPSTREAM_REF are this repo's bookkeeping, not upstream files.
if ! diff -ru \
  --exclude=.sha256 \
  --exclude=UPSTREAM_REF \
  wit/v0 "$workdir/upstream/wit/v0"; then
  echo "::error file=wit/v0::Vendored WIT differs from zeroclaw@$pinned_ref"
  echo "Refresh with: uv run --python 3.13 python tools/fetch_zeroclaw_wit.py --ref $pinned_ref"
  exit 1
fi
echo "wit/v0 matches zeroclaw@$pinned_ref"
