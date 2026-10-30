#!/usr/bin/env bash
# Generate every cargo-generate template combination and build each result.
#
#   tools/ci/check_template.sh [workdir]
#
# The template is read from the git *index*, so stage edits with
# `git add templates/` first (CI checks out the commit, so it is staged
# already). Generated plugins are built from a throwaway plugins/ tree, which
# also exercises the "first build resolves Cargo.lock" path; nothing is
# written into the real plugins/ or registry.json.
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
repository_root=$(cd -- "$script_dir/../.." && pwd -P)
cd "$repository_root"

command -v cargo-generate >/dev/null 2>&1 || {
  echo "error: cargo-generate is required: cargo install cargo-generate --locked" >&2
  exit 1
}
if ! git diff --quiet -- templates; then
  echo "error: templates/ has unstaged edits; cargo generate reads the index." >&2
  echo "       stage them with \`git add templates/\` first." >&2
  exit 1
fi

workdir=${1:-$(mktemp -d)}
scratch="$workdir/scratch-plugins"
mkdir -p "$scratch"
printf '{\n  "plugins": []\n}\n' > "$workdir/registry.json"

# cargo-generate has a bug when a path template is read from inside a larger
# git repository: for the registry=false + config_read=false combo it fails
# with "No such file or directory (os error 2)". Work around it by exporting
# the template into a fresh, standalone git repo and pointing cargo-generate at
# that isolated copy. The template contents still come from the parent repo's
# index because we enforce above that templates/ has no unstaged edits.
tmpl_repo="$workdir/template-repo"
mkdir -p "$tmpl_repo"
cp -R templates/zeroclaw-tool-plugin/. "$tmpl_repo/"
(
  cd "$tmpl_repo"
  git init -q
  git add .
  git -c user.email=ci@example.com -c user.name=ci commit -q -m "template"
)

status=0
for registry in true false; do
  for config_read in true false; do
    name="tmpl-r${registry:0:1}-c${config_read:0:1}"
    echo "::group::template registry=$registry config_read=$config_read"
    if ! cargo generate \
      --path "$tmpl_repo" \
      --destination "$scratch" \
      --name "$name" \
      --define "tool_name=greet_model" \
      --define "tool_description=CI template probe" \
      --define "author=ci" \
      --define "registry=$registry" \
      --define "config_read=$config_read" \
      --silent; then
      echo "::error::cargo generate failed for registry=$registry config_read=$config_read"
      status=1
      echo "::endgroup::"
      continue
    fi

    # Crates bind `../../wit/v0`; place the vendored contract beside the
    # scratch plugins/ directory (a copy, so the real wit/ is untouched).
    if ! cp -R wit "$workdir/wit"; then
      echo "::error::could not stage wit/ beside $name"
      status=1
      echo "::endgroup::"
      continue
    fi

    # cargo generate writes no Cargo.lock; resolve it as a first build would.
    if ! (cd "$scratch/$name" && cargo generate-lockfile --quiet); then
      echo "::error::$name: cargo generate-lockfile failed"
      status=1
    elif ! uv run --python 3.13 python tools/build_plugins.py \
      --plugins "$scratch" \
      --output "$workdir/registry.json" \
      --offline \
      --plugin "$name" --skip-metadata; then
      echo "::error::$name: build_plugins.py failed"
      status=1
    else
      # `registry = false` means "source only": never indexed. The scratch
      # registry only ever holds the local plugins built by this run, so a
      # name match means the flag was ignored.
      indexed=$(jq -r --arg name "$name" \
        '[.plugins[] | select(.name == $name)] | length' "$workdir/registry.json")
      if [[ $registry == false && $indexed == 1 ]]; then
        echo "::error::$name: registry=false plugin leaked into registry.json"
        status=1
      elif [[ $registry == true && $indexed != 1 ]]; then
        echo "::error::$name: registry=true plugin missing from registry.json"
        status=1
      fi
    fi
    echo "::endgroup::"
  done
done

if [[ "$status" -ne 0 ]]; then
  echo "error: template smoke test failed (workdir: $workdir)" >&2
  exit 1
fi
echo "template: all four combos generate and build"
