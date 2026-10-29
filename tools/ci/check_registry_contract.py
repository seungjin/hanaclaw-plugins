#!/usr/bin/env python3
"""Check ``registry.json`` against the release bases this repo publishes to.

A fresh clone carries no ``.wasm`` files (they are gitignored), so the
committed digests cannot be recomputed here; the ``package`` job does that
once the shards upload their artifacts. What this check can prove without a
build is that the document itself is trustworthy:

* it parses and satisfies the host contract (``pluginpack.validate_registry``):
  every entry is well-formed and no ``name@version`` appears twice;
* every ``url`` is the asset that entry's installer will fetch, on a release
  base this catalog actually serves: this repo's ``DEFAULT_RELEASE_BASE``
  (tag ``plugins``) or the upstream ``zeroclaw-labs/zeroclaw-plugins``
  release that inherited entries keep pointing at. A typo'd host or a
  mismatched asset name would 404 at install time, or fetch bytes nobody
  here validated.

Usage (from the repository root)::

    uv run --python 3.13 python tools/ci/check_registry_contract.py
    uv run --python 3.13 python tools/ci/check_registry_contract.py \\
        --registry registry.json --plugins plugins

Exit codes: 0 = ok, 1 = contract violation or unreadable input.
Standard library only.
"""
from __future__ import annotations

import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import pluginpack

# Inherited entries keep pointing at the release that already hosts them.
UPSTREAM_RELEASE_BASE = (
    "https://github.com/zeroclaw-labs/zeroclaw-plugins/releases/download/"
    + pluginpack.PLUGIN_RELEASE_TAG
)
ASSET_SUFFIX = ".zip"


def release_bases() -> tuple[str, ...]:
    """Complete download prefixes (release tag included) this repo serves."""
    return tuple(
        base.rstrip("/")
        for base in (pluginpack.DEFAULT_RELEASE_BASE, UPSTREAM_RELEASE_BASE)
    )


def url_problem(name: str, version: str, url: str) -> str | None:
    """Why ``url`` is not the expected asset on a known release, if so."""
    expected = f"{name}-{version}{ASSET_SUFFIX}"
    base = next(
        (b for b in release_bases() if url.startswith(f"{b}/")),
        None,
    )
    if base is None:
        return f"{name}@{version}: {url} is not on a known release base"
    if url != f"{base}/{expected}":
        asset = url.rsplit("/", 1)[-1]
        return f"{name}@{version}: asset {asset!r} should be {expected!r}"
    return None


def check(registry_path: Path, plugins_root: Path) -> list[str]:
    registry, by_identity = pluginpack.load_registry(registry_path)
    sources, errors = pluginpack.load_source_plugins(plugins_root)
    problems = list(errors)
    for entry in registry["plugins"]:
        problem = url_problem(entry["name"], entry["version"], entry["url"])
        if problem:
            problems.append(problem)
    # A publishable source directory the registry never recorded would be
    # silently unreachable; name it rather than pass.
    unrecorded = sorted(
        plugin.identity for plugin in sources if plugin.publishable
    )
    problems += [
        f"{name}@{version}: in plugins/ but absent from {registry_path}"
        for name, version in unrecorded
        if (name, version) not in by_identity
    ]
    return problems


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--registry", type=Path, default=Path("registry.json"))
    parser.add_argument("--plugins", type=Path, default=Path("plugins"))
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        problems = check(args.registry, args.plugins)
    except pluginpack.PackageError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    if problems:
        for problem in problems:
            print(f"::error file={args.registry}::{problem}")
        return 1
    registry, _ = pluginpack.load_registry(args.registry)
    print(f"registry: {len(registry['plugins'])} entries, all well-formed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
