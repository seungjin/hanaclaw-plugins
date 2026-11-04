#!/usr/bin/env python3
"""Refuse a push whose range drops a published registry entry.

CI's growth check (``check_registry_growth.py``) compares the pushed
``registry.json`` with the remote tip it sits on, using the retirement ledger
*as of that remote tip*. Every local check misses that rule:

* ``build_plugins.py`` compares against the working-tree registry, so it is
  satisfied by adding the ledger record in the same commit;
* ``--skip-metadata`` turns that comparison off entirely.

This runs at pre-push time and walks every commit in range, so a ledger record
that is committed *with* the drop is caught locally instead of after the push.
It also catches a drop introduced by a hand edit or a merge, which no build
step sees.

Usage::

    python3 tools/ci/check_push_ledger.py            # what a hook would check
    tools/ci/check_push_ledger.py --base REMOTE_REF  # compare against one ref
    tools/ci/check_push_ledger.py --range OLD..NEW   # compare each commit

Without flags it uses the merge-base with the upstream branch (``@{u}``, else
``origin/main``, else ``main``). Reads git objects only, never the network.
Exit codes: 0 = ok, 1 = a drop needs its own ledger commit, 2 = cannot check.
Standard library only.
"""
from __future__ import annotations

import argparse
import json
import subprocess
import sys

REGISTRY = "registry.json"
LEDGER = "registry-retirements.json"
FALLBACK_BASES = ("@{u}", "origin/main", "main")


class PushCheckError(RuntimeError):
    pass


def git(*args: str) -> str:
    result = subprocess.run(
        ["git", *args], capture_output=True, text=True, check=False
    )
    if result.returncode != 0:
        raise PushCheckError(result.stderr.strip() or f"git {' '.join(args)} failed")
    return result.stdout


def git_blob(rev: str, path: str) -> str | None:
    """``path`` at ``rev``, or None when that commit lacks it."""
    result = subprocess.run(
        ["git", "show", f"{rev}:{path}"],
        capture_output=True,
        text=True,
        check=False,
    )
    return result.stdout if result.returncode == 0 else None


def ledger_at(rev: str) -> dict[tuple[str, str], str]:
    text = git_blob(rev, LEDGER)
    if text is None:
        return {}
    data = json.loads(text)
    if isinstance(data, dict):
        items = data.get("retired", data.get("retirements", []))
    else:
        items = data
    return {
        (item["name"], item["version"]): item.get("sha256", "")
        for item in items
    }


def registry_at(rev: str | None) -> dict[tuple[str, str], dict]:
    text = git_blob(rev, REGISTRY) if rev else None
    if text is None:
        return {}
    data = json.loads(text)
    return {
        (entry["name"], entry["version"]): entry for entry in data["plugins"]
    }


def commits(base: str, head: str) -> list[str]:
    return git("rev-list", "--reverse", f"{base}..{head}").split()


def find_base() -> str:
    for ref in FALLBACK_BASES:
        try:
            return git("merge-base", ref, "HEAD").strip()
        except PushCheckError:
            continue
    raise PushCheckError(
        "cannot find the upstream branch to compare against; pass --base REF"
    )


def drops(base: str, head: str) -> list[str]:
    """Published entries each commit in range removes without a retirement."""
    problems: list[str] = []
    previous = registry_at(base)
    ledger = ledger_at(base)
    for commit in commits(base, head):
        current = registry_at(commit)
        message = git("log", "-1", "--format=%B", commit).strip().splitlines()
        subject = message[0] if message else commit
        for identity, entry in sorted(previous.items()):
            label = f"{identity[0]}@{identity[1]}"
            if identity in ledger:
                continue
            if identity in current:
                if current[identity].get("sha256") != entry.get("sha256"):
                    problems.append(
                        f"{label} changed sha256 in {commit[:7]} ({subject})"
                    )
                continue
            problems.append(
                f"{label} is dropped by {commit[:7]} ({subject}) without a "
                f"retirement recorded before that commit"
            )
        # A drop later in the range is fine only if the ledger knew by then.
        ledger = ledger_at(commit)
        previous = current
    return problems


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--base", help="ref to compare against (default: upstream)")
    parser.add_argument(
        "--range", dest="range_spec", help="OLD..NEW commit range to walk"
    )
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        if args.range_spec:
            base, head = args.range_spec.split("..", 1)
        else:
            base, head = args.base or find_base(), "HEAD"
        found = drops(base, head)
    except (
        PushCheckError,
        KeyError,
        json.JSONDecodeError,
        TypeError,
        ValueError,
    ) as error:
        print(f"error: {error}", file=sys.stderr)
        return 2
    if found:
        for problem in found:
            print(f"error: {problem}", file=sys.stderr)
        print(
            "\nCommit the retirement record in its own commit first:\n"
            "  add {\"name\", \"version\", \"sha256\", \"reason\"} to "
            f"{LEDGER},\n  run: uv run --python 3.13 python "
            "tools/ci/check_retirement_ledger.py\nthen commit it and rebuild.",
            file=sys.stderr,
        )
        return 1
    print(f"push: {base[:7]}..{head[:7]} keeps every published identity")
    return 0


if __name__ == "__main__":
    sys.exit(main())
