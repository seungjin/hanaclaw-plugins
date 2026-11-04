#!/usr/bin/env python3
"""Check ``registry-retirements.json`` against the registry it explains.

The ledger is the only record of *why* an identity left ``registry.json``,
and CI's growth check trusts it to decide that a drop was deliberate. A
wrong ledger is worse than a missing one, because it silences that check.
This validates the record itself, without a build:

* ``{"retired": [...]}`` parses and every record has a kebab-case ``name``, a
  ``version``, a 64-hex ``sha256``, and a non-empty ``reason``;
* no ``name@version`` appears twice -- a second record cannot correct the
  first, it only makes "which reason applies" ambiguous;
* every record either names an identity ``registry.json`` no longer holds
  (already withdrawn) or one it holds with exactly the recorded digest. A
  record whose identity is live under *different* bytes is an error: retiring
  it would let the growth check accept dropping a digest nobody published.

Retiring an identity this catalog never listed is not an error; upstream's
ledger carries some, and a record may be written before the entry it retires
reaches a release.

Usage (from the repository root)::

    uv run --python 3.13 python tools/ci/check_retirement_ledger.py
    uv run --python 3.13 python tools/ci/check_retirement_ledger.py \\
        --ledger registry-retirements.json --registry registry.json

Exit codes: 0 = ok, 1 = contract violation or unreadable input.
Standard library only.
"""
from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import pluginpack

NAME_RE = re.compile(r"^[a-z0-9][a-z0-9-]*$")
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
RETIREMENT_KEYS = ("retired", "retirements")


def load_records(path: Path) -> list:
    """The raw record list, so a malformed entry is reported, not skipped."""
    data = json.loads(path.read_text())
    if isinstance(data, list):
        return data
    if not isinstance(data, dict):
        raise pluginpack.PackageError(f"{path}: expected an object or an array")
    present = [key for key in RETIREMENT_KEYS if key in data]
    if not present:
        raise pluginpack.PackageError(
            f"{path}: expected a '{RETIREMENT_KEYS[0]}' array"
        )
    if len(present) > 1:
        raise pluginpack.PackageError(
            f"{path}: has both {', '.join(repr(key) for key in present)}; "
            f"keep only '{RETIREMENT_KEYS[0]}'"
        )
    items = data[present[0]]
    if not isinstance(items, list):
        raise pluginpack.PackageError(f"{path}: expected an array of retirements")
    return items


def record_problem(index: int, item: object) -> str | None:
    """Why record ``index`` is malformed, if so."""
    if not isinstance(item, dict):
        return f"retirement #{index}: expected an object"
    name, version = item.get("name"), item.get("version")
    if not isinstance(name, str) or not NAME_RE.fullmatch(name):
        return f"retirement #{index}: 'name' must be kebab-case, got {name!r}"
    if not isinstance(version, str) or not version:
        return f"{name}: 'version' must be a non-empty string"
    label = f"{name}@{version}"
    sha256 = item.get("sha256")
    if not isinstance(sha256, str) or not SHA256_RE.fullmatch(sha256):
        return f"{label}: 'sha256' must be 64 lowercase hex digits"
    reason = item.get("reason")
    if not isinstance(reason, str) or not reason.strip():
        return f"{label}: 'reason' must say why the identity cannot install"
    return None


def check(ledger_path: Path, registry_path: Path) -> tuple[list[str], str]:
    records = load_records(ledger_path)
    registry, by_identity = pluginpack.load_registry(registry_path)
    problems: list[str] = []
    seen: set[tuple[str, str]] = set()
    for index, item in enumerate(records):
        problem = record_problem(index, item)
        if problem:
            problems.append(problem)
            continue
        identity = (item["name"], item["version"])
        label = f"{identity[0]}@{identity[1]}"
        if identity in seen:
            problems.append(
                f"{label}: retired twice; edit the existing record instead"
            )
            continue
        seen.add(identity)
        live = by_identity.get(identity)
        if live is not None and live["sha256"] != item["sha256"]:
            problems.append(
                f"{label}: ledger records sha256 {item['sha256'][:12]}… but "
                f"registry.json publishes {live['sha256'][:12]}…; a "
                f"retirement must name the bytes that were published"
            )
    withdrawn = sum(1 for identity in seen if identity not in by_identity)
    summary = (
        f"{len(records)} record(s): {len(seen) - withdrawn} live, "
        f"{withdrawn} withdrawn, {len(registry['plugins'])} registry entries"
    )
    return problems, summary


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--ledger", type=Path, default=Path("registry-retirements.json")
    )
    parser.add_argument("--registry", type=Path, default=Path("registry.json"))
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        problems, summary = check(args.ledger, args.registry)
    except (pluginpack.PackageError, OSError, json.JSONDecodeError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    if problems:
        for problem in problems:
            print(f"::error file={args.ledger}::{problem}")
        return 1
    print(f"retirements: {summary}, all consistent")
    return 0


if __name__ == "__main__":
    sys.exit(main())
