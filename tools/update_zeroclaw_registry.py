#!/usr/bin/env python3
"""Maintain the Hanaclaw plugin registry by inheriting the upstream catalog.

Hanaclaw ships its own registry because ``zeroclaw plugin install <name>``
reads exactly one ``registry.json``. That file must contain both:

1. every plugin from the official ZeroClaw registry
   (https://github.com/zeroclaw-labs/zeroclaw-plugins), and
2. the plugins published here, under ``plugins/<name>/`` with a
   ``manifest.toml`` and a built ``.wasm`` at its ``wasm_path``.

Only text is committed: entry ``url`` values point at this repo's GitHub
Release assets (tag ``plugins``), and installs verify the recorded
``sha256``. Upstream entries are copied verbatim, so their release URLs and
digests keep resolving.

This tool does not compile anything. To build the wasm, package the zips and
refresh the registry in one step, use ``tools/build_plugins.py``; to upload
the zips, use ``tools/publish_plugins.py``.

Usage (from the repository root)::

    uv run --python 3.13 python tools/update_zeroclaw_registry.py            # fetch + write
    uv run --python 3.13 python tools/update_zeroclaw_registry.py --check    # CI: drift only
    uv run --python 3.13 python tools/update_zeroclaw_registry.py --stdout   # preview

Environment: ``ZEROCLAW_REGISTRY_URL`` overrides the upstream source (useful
with a pinned tag or a mirror); ``GITHUB_TOKEN`` is sent when set.

Exit codes: 0 = ok, 2 = drift detected with ``--check``, 1 = error.
Requires Python 3.11+ (tomllib). Standard library only.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import pluginpack
from pluginpack import (
    DEFAULT_RELEASE_BASE,
    DEFAULT_UPSTREAM_REGISTRY,
    PackageError,
    entry_key,
    fetch_upstream,
    manifest_metadata,
    render,
    validate_registry,
)

RegistryError = PackageError


class _Args(argparse.Namespace):
    upstream: str
    upstream_file: Path | None
    plugins: Path
    output: Path
    release_base: str
    retirements: Path | None
    check: bool
    stdout: bool
    offline: bool


# ── local plugins ───────────────────────────────────────────────────────────


def local_entries(plugins_root: Path, release_base: str) -> list[dict]:
    """Registry entries for the ``plugins/`` directories built here.

    A directory is included when it has a ``manifest.toml`` without
    ``registry = false`` and a built wasm at ``wasm_path``. Its published
    digest is the sha256 of the reproducible package zip the release holds
    (see ``pluginpack.package_sha256``).
    """
    plugins, errors = pluginpack.load_source_plugins(plugins_root)
    if errors:
        raise RegistryError("; ".join(errors))

    entries = []
    for plugin in plugins:
        if not plugin.publishable:
            print(f"note: skipping {plugin.name}: registry = false", file=sys.stderr)
            continue
        wasm = plugin.dir / plugin.wasm_path
        if not wasm.is_file():
            raise RegistryError(
                f"{wasm}: missing; run `uv run --python 3.13 python "
                f"tools/build_plugins.py` to build it, or "
                f"`cargo build --release --target wasm32-wasip2` and copy it "
                f"next to manifest.toml"
            )
        entry = {
            **manifest_metadata(plugin.manifest),
            "url": f"{release_base.rstrip('/')}/{plugin.name}-{plugin.version}.zip",
            "sha256": pluginpack.package_sha256(
                plugin.dir, plugin.name, plugin.wasm_path
            ),
        }
        entries.append(entry)
    return entries


# ── merge ───────────────────────────────────────────────────────────────────


def merge(
    upstream: dict, local: list[dict], retirements: dict[tuple[str, str], str]
) -> dict:
    """Upstream verbatim, then local entries; local wins on identity clash.

    A local build whose name@version upstream already publishes keeps
    upstream's ``url``/``sha256`` unless its zip digest is identical (i.e. the
    local tree is an exact copy of the published package). Otherwise the entry
    would point at an asset this repo never uploaded.

    Entries listed in ``registry-retirements.json`` are dropped, matching the
    upstream catalog's own withdrawal semantics. Within a name, the newest
    version sorts last because the host resolves an unpinned install to the
    last matching entry.
    """
    by_identity: dict[tuple[str, str], dict] = {}
    for entry in upstream["plugins"]:
        by_identity[(entry["name"], entry["version"])] = entry
    for entry in local:
        identity = (entry["name"], entry["version"])
        published = by_identity.get(identity)
        if published is not None and published.get("sha256") != entry["sha256"]:
            print(
                f"note: {identity[0]}@{identity[1]} already published upstream with "
                f"different bytes; keeping upstream url/digest",
                file=sys.stderr,
            )
            continue
        by_identity[identity] = entry

    entries = [
        entry for identity, entry in by_identity.items() if identity not in retirements
    ]
    entries.sort(key=entry_key)
    return {"plugins": entries}


RETIREMENT_KEYS = ("retired", "retirements")


def load_retirements(path: Path) -> dict[tuple[str, str], str]:
    """Withdrawn ``name@version`` identities from a retirement ledger.

    Accepts the shape zeroclaw-plugins ships (``{"retired": [...]}``), the
    legacy ``{"retirements": [...]}``, or a bare array. Retired identities are
    dropped from the merged catalog exactly as upstream drops them.
    """
    if not path.is_file():
        return {}
    data = json.loads(path.read_text())
    if isinstance(data, dict):
        present = [key for key in RETIREMENT_KEYS if key in data]
        if not present:
            raise RegistryError(
                f"{path}: expected a '{RETIREMENT_KEYS[0]}' array like "
                f"zeroclaw-plugins/registry-retirements.json"
            )
        items = data[present[0]]
    else:
        items = data
    if not isinstance(items, list):
        raise RegistryError(f"{path}: expected an array of retirements")
    out = {}
    for item in items:
        if not isinstance(item, dict) or "name" not in item or "version" not in item:
            raise RegistryError(f"{path}: each retirement needs 'name' and 'version'")
        out[(item["name"], item["version"])] = str(item.get("reason", ""))
    return out


def pinned_registry_url(url: str, ref: str) -> str:
    """``.../raw.githubusercontent.com/<owner>/<repo>/main/registry.json`` at ``ref``.

    Upstream moves ``main`` independently of this repo; pinning a commit makes a
    refresh reproducible (the same ``--upstream-ref`` yields the same entries).
    """
    host = "raw.githubusercontent.com"
    prefix = f"https://{host}/"
    if not url.startswith(prefix) or not url.endswith("/registry.json"):
        raise RegistryError(
            f"--upstream-ref needs a {host}/<owner>/<repo>/<branch>/registry.json "
            f"URL; got {url}. Use --upstream with that URL instead."
        )
    if not re.fullmatch(r"[A-Za-z0-9._/-]+", ref) or ".." in ref:
        raise RegistryError(f"--upstream-ref: invalid ref {ref!r}")
    head = url[len(prefix) : -len("/registry.json")].split("/")
    if len(head) != 3 or not all(head):
        raise RegistryError(f"--upstream-ref: unexpected upstream URL {url}")
    owner, repo, _branch = head
    return f"{prefix}{owner}/{repo}/{ref}/registry.json"


# ── CLI ─────────────────────────────────────────────────────────────────────


def parse_args(argv: list[str] | None = None) -> _Args:
    parser = argparse.ArgumentParser(
        description="Rebuild registry.json from the upstream ZeroClaw registry "
        "plus local plugins/."
    )
    parser.add_argument(
        "--upstream",
        default=os.environ.get("ZEROCLAW_REGISTRY_URL", DEFAULT_UPSTREAM_REGISTRY),
        help=f"upstream registry URL (default: {DEFAULT_UPSTREAM_REGISTRY})",
    )
    parser.add_argument(
        "--upstream-file", type=Path, help="read upstream from a local file instead"
    )
    parser.add_argument(
        "--upstream-ref",
        metavar="REF",
        help="read the upstream registry at this branch, tag, or commit "
        "(default: whatever --upstream names, i.e. main)",
    )
    parser.add_argument(
        "--plugins", type=Path, default=Path("plugins"), help="local plugins directory"
    )
    parser.add_argument(
        "--output", type=Path, default=Path("registry.json"), help="registry to write"
    )
    parser.add_argument(
        "--release-base",
        default=DEFAULT_RELEASE_BASE,
        help="base URL of this repo's release assets",
    )
    parser.add_argument(
        "--retirements", type=Path, help="optional retirement ledger to honor"
    )
    parser.add_argument(
        "--check", action="store_true", help="exit 2 if the output file is stale"
    )
    parser.add_argument("--stdout", action="store_true", help="print instead of writing")
    parser.add_argument(
        "--offline", action="store_true", help="reuse --output's upstream entries"
    )
    return parser.parse_args(argv, namespace=_Args())


def build(args: _Args) -> tuple[dict, list[dict]]:
    """Resolve upstream + local entries from the CLI arguments."""
    if args.upstream_file:
        upstream = json.loads(args.upstream_file.read_text())
        validate_registry(upstream, source=str(args.upstream_file))
    elif args.offline:
        if not args.output.is_file():
            raise RegistryError("--offline needs an existing --output registry to reuse")
        upstream = json.loads(args.output.read_text())
        validate_registry(upstream, source=str(args.output))
    else:
        source = (
            pinned_registry_url(args.upstream, args.upstream_ref)
            if args.upstream_ref
            else args.upstream
        )
        upstream = fetch_upstream(source)
    pluginpack.warn_if_zlib_ng()
    local = local_entries(args.plugins, args.release_base)
    retirements = load_retirements(args.retirements) if args.retirements else {}
    return merge(upstream, local, retirements), local


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        merged, local = build(args)
        text = render(merged)
    except RegistryError as e:
        print(f"error: {e}", file=sys.stderr)
        return 1

    if args.stdout:
        sys.stdout.write(text)
        return 0

    if args.check:
        current = args.output.read_text() if args.output.is_file() else ""
        if current == text:
            print(f"{args.output} is up to date ({len(merged['plugins'])} entries)")
            return 0
        print(
            f"error: {args.output} is stale; run `uv run --python 3.13 python "
            f"tools/update_zeroclaw_registry.py` (or tools/build_plugins.py)",
            file=sys.stderr,
        )
        return 2

    tmp = args.output.with_suffix(".json.tmp")
    tmp.write_text(text)
    os.replace(tmp, args.output)
    print(
        f"wrote {args.output} with {len(merged['plugins'])} entries "
        f"({len(merged['plugins']) - len(local)} inherited, {len(local)} local)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
