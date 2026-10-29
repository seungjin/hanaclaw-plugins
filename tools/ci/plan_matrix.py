#!/usr/bin/env python3
"""Plan which plugins CI must validate, as GitHub matrix shards.

Mirrors zeroclaw-plugins' planner (``tools/ci/plan_matrix.py``) with this
catalog's smaller surface:

* a pull request validates only the plugins whose ``plugins/<name>/`` files
  changed, unless a shared input changed, in which case it sweeps everything;
* any other event (push, manual dispatch, ``workflow_call``) is a full sweep;
* ``strict_plugins`` are the plugins whose own files changed: only those must
  pass rustfmt, so pre-existing debt in untouched crates cannot block CI;
* ``release_plugins`` is every plugin when ``wit/v0/`` changed (a contract
  change changes what each plugin compiles to), otherwise ``strict_plugins``.

Shared inputs: ``tools/``, ``wit/``, ``templates/``, ``.github/``,
``rust-toolchain.toml``, ``registry.json``, ``registry-retirements.json``,
``Cargo.toml``/``Cargo.lock`` at the root, and any unclassified top-level
path. Documentation-only changes (``README.md``, ``docs/``, ``AGENTS.md``,
``NOTICE``, ``LICENSE-*``) validate no plugin.

Usage (from the repository root)::

    uv run --python 3.13 python tools/ci/plan_matrix.py            # print JSON
    GITHUB_OUTPUT=... uv run --python 3.13 python \
        tools/ci/plan_matrix.py --event pull_request --base origin/main

Writes ``matrix``, ``mode``, ``count``, ``plugins``, and ``strict`` to
``$GITHUB_OUTPUT`` when set. ``plugins`` (``[{"name", "registry"}]``, where
``registry`` mirrors ``manifest.toml``) lets the packaging job copy exactly
the wasm files its shards were told to build.
Standard library only; no uv needed at runtime.
"""
from __future__ import annotations

import argparse
import json
import math
import os
import re
import subprocess
import sys
import tomllib
from pathlib import Path, PurePosixPath

PLUGIN_NAME_RE = re.compile(r"^[a-z0-9][a-z0-9-]*$")
DEFAULT_SHARD_SIZE = 8
DEFAULT_MAX_SHARDS = 4
FULL_SWEEP_PREFIXES = (
    ".github/",
    "tools/",
    "templates/",
    "wit/",
    "docs/",
)
FULL_SWEEP_FILES = {
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "registry.json",
    "registry-retirements.json",
}
# Kept apart from docs/ on purpose: a docs/ change is shared material.
DOC_ONLY_PREFIXES = ("docs/",)
DOC_ONLY_FILES = {
    "README.md",
    "AGENTS.md",
    "NOTICE",
    "LICENSE-MIT",
    "LICENSE-APACHE",
}


class PlanError(ValueError):
    pass


def plugin_directories(plugins_dir: Path) -> list[str]:
    if not plugins_dir.is_dir():
        return []
    names = []
    for path in plugins_dir.iterdir():
        if path.is_symlink():
            continue
        if path.is_dir() and PLUGIN_NAME_RE.fullmatch(path.name):
            names.append(path.name)
    return sorted(names)


def is_publishable(repository: Path, name: str) -> bool:
    """Whether ``plugins/<name>/manifest.toml`` opts into the registry.

    A lightweight read (no full manifest validation): CI only uses this to
    decide which wasm files the packaging job must collect.
    """
    manifest = repository / "plugins" / name / "manifest.toml"
    try:
        data = tomllib.loads(manifest.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError):
        return False
    return data.get("registry") is not False


def git_changed_paths(repository: Path, base: str) -> list[str]:
    try:
        subprocess.run(
            ["git", "cat-file", "-e", f"{base}^{{commit}}"],
            cwd=repository, check=True, capture_output=True,
        )
        merge_base = subprocess.run(
            ["git", "merge-base", base, "HEAD"],
            cwd=repository, check=True, capture_output=True, text=True,
        ).stdout.strip()
        raw = subprocess.run(
            ["git", "diff", "--name-only", "--diff-filter=ACDMRTUXB", "-z",
             merge_base or base, "HEAD", "--"],
            cwd=repository, check=True, capture_output=True,
        ).stdout
    except (OSError, subprocess.CalledProcessError) as error:
        stderr = getattr(error, "stderr", b"") or b""
        if isinstance(stderr, bytes):
            stderr = stderr.decode("utf-8", errors="replace")
        raise PlanError(
            f"cannot compute changes from {base}: {stderr.strip() or error}"
        ) from error
    try:
        return [part.decode("utf-8") for part in raw.split(b"\0") if part]
    except UnicodeDecodeError as error:
        raise PlanError("changed paths must be valid UTF-8") from error


def relative(path: PurePosixPath) -> PurePosixPath:
    """Repo-relative form of a path git may report with a ``./`` prefix."""
    parts = tuple(part for part in path.parts if part != ".")
    return PurePosixPath(*parts) if parts else PurePosixPath(".")


def requires_full_sweep(changed_paths: list[str]) -> bool:
    for raw_path in changed_paths:
        path = relative(PurePosixPath(raw_path.replace("\\", "/"))).as_posix()
        if path in FULL_SWEEP_FILES or path.startswith(FULL_SWEEP_PREFIXES):
            return True
        if path in DOC_ONLY_FILES or path.startswith(DOC_ONLY_PREFIXES):
            continue
        parts = PurePosixPath(path).parts
        if len(parts) >= 2 and parts[0] == "plugins" and PLUGIN_NAME_RE.fullmatch(
            parts[1]
        ):
            continue
        # An unclassified root path (a new config file, say) is
        # safety-relevant until someone classifies it.
        return True
    return False


def changed_plugins(changed_paths: list[str], available: list[str]) -> list[str]:
    available_set = set(available)
    selected = set()
    for raw_path in changed_paths:
        parts = PurePosixPath(raw_path.replace("\\", "/")).parts
        if len(parts) >= 2 and parts[0] == "plugins" and parts[1] in available_set:
            selected.add(parts[1])
    return sorted(selected)


def shard_plugins(
    plugins: list[str],
    shard_size: int = DEFAULT_SHARD_SIZE,
    max_shards: int = DEFAULT_MAX_SHARDS,
) -> list[list[str]]:
    if not plugins:
        return []
    desired = math.ceil(len(plugins) / shard_size)
    if desired <= max_shards:
        return [plugins[i:i + shard_size] for i in range(0, len(plugins), shard_size)]
    minimum, larger = divmod(len(plugins), max_shards)
    shards, offset = [], 0
    for index in range(max_shards):
        size = minimum + (1 if index < larger else 0)
        shards.append(plugins[offset:offset + size])
        offset += size
    return shards


def make_plan(
    repository: Path,
    event: str,
    base: str,
    shard_size: int = DEFAULT_SHARD_SIZE,
    max_shards: int = DEFAULT_MAX_SHARDS,
) -> dict[str, object]:
    available = plugin_directories(repository / "plugins")
    paths: list[str] = []
    strict: set[str]
    if event == "pull_request":
        paths = git_changed_paths(repository, base)
        if requires_full_sweep(paths):
            mode, selected = "full", available
        else:
            mode, selected = "changed", changed_plugins(paths, available)
        strict = set(changed_plugins(paths, available))
    else:
        # Push (including the publish workflow's workflow_call), manual
        # dispatch: validate everything, and hold every plugin to rustfmt.
        mode, selected, strict = "full", available, set(available)
    release_all = any(
        relative(PurePosixPath(path.replace("\\", "/"))).as_posix().startswith(
            "wit/v0/"
        )
        for path in paths
    )
    release = set(available) if release_all else set(strict)

    shards = shard_plugins(selected, shard_size, max_shards)
    matrix = {
        "include": [
            {
                "id": index,
                "plugins": shard,
                "release_plugins": [name for name in shard if name in release],
                "strict_plugins": [name for name in shard if name in strict],
            }
            for index, shard in enumerate(shards)
        ]
    }
    return {
        "matrix": matrix,
        "mode": mode,
        "count": sum(len(shard) for shard in shards),
        "plugins": [
            {"name": name, "registry": is_publishable(repository, name)}
            for name in selected
        ],
        "strict": [name for name in selected if name in strict],
    }


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--repository", type=Path, default=Path.cwd())
    parser.add_argument(
        "--event", default=os.environ.get("GITHUB_EVENT_NAME", "workflow_dispatch")
    )
    parser.add_argument(
        "--base", default=os.environ.get("GITHUB_EVENT_BEFORE") or "origin/main"
    )
    parser.add_argument("--shard-size", type=int, default=DEFAULT_SHARD_SIZE)
    parser.add_argument("--max-shards", type=int, default=DEFAULT_MAX_SHARDS)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        plan = make_plan(
            args.repository.resolve(), args.event, args.base,
            args.shard_size, args.max_shards,
        )
    except PlanError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1

    output = os.environ.get("GITHUB_OUTPUT")
    if output:
        with open(output, "a", encoding="utf-8") as handle:
            handle.write(
                "matrix=" + json.dumps(plan["matrix"], separators=(",", ":")) + "\n"
            )
            handle.write(f"mode={plan['mode']}\n")
            handle.write(f"count={plan['count']}\n")
            handle.writelines(
                f"{key}=" + json.dumps(plan[key], separators=(",", ":")) + "\n"
                for key in ("plugins", "strict")
            )
        print(f"plan: mode={plan['mode']} count={plan['count']}")
    else:
        json.dump(plan, sys.stdout, indent=2)
        sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
