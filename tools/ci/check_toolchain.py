#!/usr/bin/env python3
"""Fail when the Rust toolchain pin drifts between the places that define it.

Every plugins/<name>/ is a standalone crate (`[workspace]` in its
Cargo.toml), so `cargo` inside it ignores the repository-root
rust-toolchain.toml and follows the nearest one above the crate instead.
Reproducible release bytes therefore need the pin next to every crate:

* ``rust-toolchain.toml`` (repository root: repo-level `cargo`/`cargo install`)
* ``plugins/<name>/rust-toolchain.toml`` (what plain `cargo` in the plugin obeys)
* ``templates/zeroclaw-tool-plugin/rust-toolchain.toml`` (copied by cargo generate)
* ``RUST_TOOLCHAIN`` in ``.github/workflows/*.yml`` (what the CI action installs)

A toolchain bump changes wasm bytes, so a drift here is never cosmetic.
"""

from __future__ import annotations

import argparse
import re
import sys
import tomllib
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
PIN_KEYS = ("channel", "profile", "components", "targets")
WORKFLOW_PIN = re.compile(r'^\s*RUST_TOOLCHAIN:\s*"?([^"\s]+)"?\s*$', re.MULTILINE)


def pins(root: Path) -> dict[Path, dict]:
    """Every rust-toolchain.toml that decides a build, keyed by path."""
    files = [root / "rust-toolchain.toml"]
    files += sorted((root / "plugins").glob("*/rust-toolchain.toml"))
    files += sorted((root / "templates").glob("*/rust-toolchain.toml"))
    found = {}
    for path in files:
        if not path.is_file():
            found[path] = {}
            continue
        try:
            found[path] = tomllib.loads(path.read_text())["toolchain"]
        except (tomllib.TOMLDecodeError, KeyError) as e:
            found[path] = {"error": str(e)}
    return found


def workflow_pins(root: Path) -> dict[Path, set[str]]:
    """RUST_TOOLCHAIN values declared per workflow file."""
    found = {}
    for path in sorted((root / ".github" / "workflows").glob("*.y*ml")):
        found[path] = set(WORKFLOW_PIN.findall(path.read_text()))
    return found


def check(root: Path = REPO_ROOT) -> list[str]:
    problems = []
    found = pins(root)
    if not found:
        return [f"{root}: no rust-toolchain.toml found"]

    channels = {
        path: meta.get("channel")
        for path, meta in found.items()
        if isinstance(meta.get("channel"), str)
    }
    for path, meta in found.items():
        if "error" in meta:
            problems.append(f"{path}: {meta['error']}")
        elif path.name == "rust-toolchain.toml" and not meta:
            problems.append(f"{path}: missing")
    if not channels:
        return problems + [f"{root}: no pinned channel anywhere"]

    expected = channels[root / "rust-toolchain.toml"]
    if not re.fullmatch(r"\d+\.\d+\.\d+", expected or ""):
        problems.append(
            f"{root / 'rust-toolchain.toml'}: channel {expected!r} is not a "
            f"pinned x.y.z release"
        )
    for path, channel in channels.items():
        if channel != expected:
            shown = path.relative_to(root)
            problems.append(
                f"{shown}: channel {channel!r}, expected {expected!r} "
                f"(from rust-toolchain.toml)"
            )
    for path, meta in found.items():
        if meta and meta != found[root / "rust-toolchain.toml"]:
            shown = path.relative_to(root)
            keys = [key for key in PIN_KEYS if meta.get(key) != found[
                root / "rust-toolchain.toml"
            ].get(key)]
            if keys and not any(str(shown) in p for p in problems):
                problems.append(
                    f"{shown}: {', '.join(keys)} differ from "
                    f"rust-toolchain.toml"
                )
    for path, values in workflow_pins(root).items():
        shown = path.relative_to(root)
        for value in values:
            if value != expected:
                problems.append(
                    f"{shown}: RUST_TOOLCHAIN {value!r}, expected {expected!r}"
                )
    return problems


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--root", type=Path, default=REPO_ROOT, help="repository root (default: %(default)s)"
    )
    args = parser.parse_args(argv)
    problems = check(args.root)
    for problem in problems:
        print(f"error: {problem}", file=sys.stderr)
    if problems:
        return 1
    channel = pins(args.root)[args.root / "rust-toolchain.toml"]["channel"]
    print(f"toolchain: every pin agrees on {channel}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
