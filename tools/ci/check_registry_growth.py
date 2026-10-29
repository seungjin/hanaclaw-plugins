#!/usr/bin/env python3
"""Fail when a change drops an entry or rewrites a published sha256.

``registry.json`` is an append-only ledger: a published ``name@version`` maps
to exactly one zip, so history may grow but never shrink or change in place.
The only sanctioned removal is a *retirement* recorded in
``registry-retirements.json`` (the host can no longer install it).

Usage (from the repository root)::

    git show "$BASE:registry.json" > base-registry.json
    git show "$BASE:registry-retirements.json" > base-retirements.json
    uv run --python 3.13 python tools/ci/check_registry_growth.py \\
        --base base-registry.json \\
        --retirements base-retirements.json registry.json

An empty or missing ``--base`` (first push to a branch) passes trivially.
``--retirements`` must be the ledger as of the *base* commit: retiring an entry
and dropping it in the same change is refused, so the ledger always leads the
registry (upstream applies the same rule).
Standard library only.
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path


class GrowthError(ValueError):
    pass


def load_entries(path: Path) -> dict[tuple[str, str], dict]:
    try:
        document = json.loads(path.read_text())
    except FileNotFoundError:
        return {}
    except json.JSONDecodeError as error:
        raise GrowthError(f"{path}: not valid JSON: {error}") from error
    plugins = document.get("plugins") if isinstance(document, dict) else None
    if not isinstance(plugins, list):
        raise GrowthError(f"{path}: expected an object with a 'plugins' array")
    entries = {}
    for entry in plugins:
        if not isinstance(entry, dict) or "name" not in entry or "version" not in entry:
            raise GrowthError(f"{path}: every entry needs 'name' and 'version'")
        entries[(entry["name"], entry["version"])] = entry
    return entries


def load_retirements(path: Path | None) -> set[tuple[str, str]]:
    if path is None or not path.is_file():
        return set()
    document = json.loads(path.read_text())
    if isinstance(document, list):
        items = document
    elif isinstance(document, dict):
        items = document.get("retired", document.get("retirements", []))
    else:
        raise GrowthError(f"{path}: expected an object or an array")
    return {(item["name"], item["version"]) for item in items}


def problems(
    base: dict[tuple[str, str], dict],
    head: dict[tuple[str, str], dict],
    retired: set[tuple[str, str]],
) -> list[str]:
    found = []
    for identity, entry in sorted(base.items()):
        if identity in retired:
            continue
        label = f"{identity[0]}@{identity[1]}"
        current = head.get(identity)
        if current is None:
            found.append(f"{label} was dropped without a retirement entry")
        elif current.get("sha256") != entry.get("sha256"):
            found.append(f"{label} changed sha256 after publication")
    return found


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--base", type=Path, help="registry.json at the base commit")
    parser.add_argument(
        "--retirements",
        type=Path,
        help="base commit's ledger: identities it already retired",
    )
    parser.add_argument("head", type=Path, nargs="?", default=Path("registry.json"))
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        if args.base is None:
            print("no base registry; growth check skipped")
            return 0
        base = load_entries(args.base)
        head = load_entries(args.head)
        retired = load_retirements(args.retirements)
    except (GrowthError, OSError, KeyError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1

    found = problems(base, head, retired)
    for problem in found:
        print(f"::error file={args.head}::{problem}")
    if found:
        return 1
    added = len(set(head) - set(base))
    print(
        f"registry grew by {added} entr{'y' if added == 1 else 'ies'}; "
        f"{len(head)} total, no published identity changed"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
