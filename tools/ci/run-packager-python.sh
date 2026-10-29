#!/usr/bin/env bash
# Run Python inside the pinned packager image, so every zip digest computed
# here matches upstream zeroclaw-plugins byte for byte.
#
#   tools/ci/run-packager-python.sh tools/build_plugins.py --package
#   tools/ci/run-packager-python.sh -m unittest discover -s tools/tests
#
# The repository is mounted at /workspace with the network disabled and the
# container filesystem read-only. Uses docker when present (GitHub runners),
# podman otherwise (local Fedora/RHEL hosts); PACKAGER_CONTAINER overrides.
#
# The container must be able to rewrite dist/ and registry.json. Docker passes
# file ownership through, so it runs as the caller's uid:gid. Rootless podman
# idmaps bind mounts onto its single-uid mapping (host uid -> container 0), so
# the caller's files are root-owned inside and only uid 0 can write them; there
# it runs as 0:gid instead. Either way files on the host stay the caller's.
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
repository_root=$(cd -- "$script_dir/../.." && pwd -P)
image_file="$script_dir/packager-image.txt"

image=""
image_count=0
while IFS= read -r line; do
  image=$line
  image_count=$((image_count + 1))
done < <(
  sed -E 's/[[:space:]]*#.*$//' "$image_file" \
    | sed -E '/^[[:space:]]*$/d; s/^[[:space:]]+//; s/[[:space:]]+$//'
)
if [[ "$image_count" -ne 1 \
  || ! "$image" =~ ^docker\.io/library/python@sha256:[0-9a-f]{64}$ ]]; then
  echo "error: $image_file must contain exactly one immutable official Python image digest" >&2
  exit 1
fi

engine=${PACKAGER_CONTAINER:-}
if [[ -z "$engine" ]]; then
  for candidate in docker podman; do
    if command -v "$candidate" >/dev/null 2>&1; then
      engine=$candidate
      break
    fi
  done
fi
if [[ -z "$engine" ]]; then
  echo "error: need docker or podman to run $image" >&2
  exit 1
fi

# Docker keeps the caller's uid so written files stay the caller's; rootless
# podman maps host uid -> 0, so 0 is the id that owns the mounted checkout.
run_uid=$(id -u)
if [[ "$engine" == "podman" ]] && podman info --format '{{.Host.Security.Rootless}}' 2>/dev/null | grep -q true; then
  run_uid=0
fi

container_args=(
  run
  --platform linux/amd64
  --rm
  --network none
  --read-only
  --cap-drop ALL
  --security-opt no-new-privileges
  --user "$run_uid:$(id -g)"
  --env HOME=/tmp
  --env PYTHONDONTWRITEBYTECODE=1
  --tmpfs '/tmp:rw,noexec,nosuid,nodev'
)
# The mount must stay writable: the tool rewrites dist/ and registry.json.
container_args+=(--volume "$repository_root:/workspace" --workdir /workspace)
# Paths are passed through verbatim, so a directory the caller names must
# exist at the same absolute path inside the container. That is why /tmp is
# a container-private tmpfs: with docker, $RUNNER_TEMP is somewhere else (a
# real host directory, mounted as is), while locally it may be /tmp itself,
# and mounting it over the tmpfs would hide the tmpfs. The one case that
# cannot work is a RUNNER_TEMP under /tmp that is not exactly /tmp: it would
# be hidden inside the tmpfs, so refuse it instead of losing its files.
if [[ -n "${RUNNER_TEMP:-}" ]]; then
  if [[ ! -d "$RUNNER_TEMP" ]]; then
    echo "error: RUNNER_TEMP is not a directory: $RUNNER_TEMP" >&2
    exit 1
  fi
  if [[ "$RUNNER_TEMP" != "/tmp" ]]; then
    if [[ "$RUNNER_TEMP" == /tmp/* ]]; then
      echo "error: RUNNER_TEMP=$RUNNER_TEMP would be hidden by the container's private /tmp; use /tmp or a directory outside it" >&2
      exit 1
    fi
    container_args+=(--volume "$RUNNER_TEMP:$RUNNER_TEMP")
  fi
fi
if [[ "$engine" == "podman" ]]; then
  # SELinux hosts label the bind mount; disable labelling for this run.
  container_args+=(--security-opt label=disable)
fi

if ! "$engine" image inspect "$image" >/dev/null 2>&1; then
  echo "info: pulling $image" >&2
  "$engine" pull "$image" >/dev/null
fi

exec "$engine" "${container_args[@]}" "$image" python3 "$@"
