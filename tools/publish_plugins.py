#!/usr/bin/env python3
"""Upload the zips registry.json references, then verify and commit it.

Run ``tools/build_plugins.py --package`` first. This tool mirrors the upstream
``zeroclaw-plugins`` publish workflow, minus its CI plumbing:

1. checks every ``dist/*.zip`` against ``registry.json`` (name, digest), and
   downloads the URL of each local entry to confirm an already-uploaded
   asset is byte-identical (published identities are immutable),
2. creates the ``plugins`` release if needed and uploads only missing assets
   (``gh release upload``),
3. re-downloads every *local* entry's URL and verifies its sha256, and
4. commits the refreshed ``registry.json`` (only that file).

Upstream catalog entries are inherited verbatim and are not uploaded or
fetched; their URLs point at zeroclaw-labs.

Requires an authenticated ``gh`` (``gh auth login``) with push access to this
repo, and a git remote ``origin``.

Usage (from the repository root)::

    uv run --python 3.13 python tools/build_plugins.py --package
    uv run --python 3.13 python tools/publish_plugins.py --dry-run   # show the plan
    uv run --python 3.13 python tools/publish_plugins.py             # upload + commit
    uv run --python 3.13 python tools/publish_plugins.py --no-commit # upload only
    uv run --python 3.13 python tools/publish_plugins.py --verify-only --ref <git-ref>

``--ref`` (default: the checked-out ``registry.json``) lets you verify the
committed catalog instead of the working copy, e.g. in CI after a push.

Exit codes: 0 = ok, 2 = an already-published asset has different bytes,
1 = error.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import pluginpack
from pluginpack import PackageError

REPO_ROOT = Path(__file__).resolve().parent.parent


class PublishError(Exception):
    """A fatal publish problem; reported without a traceback."""


class ImmutableAssetError(PublishError):
    """An existing release asset differs from the local package."""


class Runner:
    """Subprocess wrapper that prints and honors ``--dry-run``."""

    def __init__(self, *, dry_run: bool) -> None:
        self.dry_run = dry_run

    def run(self, *command: str, capture: bool = False) -> str:
        printable = " ".join(command)
        if self.dry_run and command[0] not in ("git",):
            print(f"  [dry-run] {printable}")
            return ""
        print(f"  $ {printable}")
        result = subprocess.run(
            command,
            cwd=REPO_ROOT,
            text=True,
            stdout=subprocess.PIPE if capture else None,
            check=False,
        )
        if result.returncode != 0:
            raise PublishError(f"`{printable}` failed (exit {result.returncode})")
        return result.stdout or ""

    def git(self, *args: str, capture: bool = False) -> str:
        if self.dry_run and args[0] not in ("rev-parse", "diff", "status"):
            print(f"  [dry-run] git {' '.join(args)}")
            return ""
        return self.run("git", *args, capture=capture)


# ── discovery ──────────────────────────────────────────────────────────────


def local_prefix(release_base: str) -> str:
    return release_base.rstrip("/") + "/"


def local_entries(
    registry: dict, plugins_root: Path, release_base: str
) -> list[tuple[pluginpack.Plugin, dict]]:
    """Registry entries that point at this repo's release, with their plugin.

    A local entry that is missing from the checked-out ``registry.json`` (a
    plugin added by this change but not yet committed) is picked up from its
    manifest and package bytes, so it can be uploaded even before the
    registry commit lands.
    """
    plugins, errors = pluginpack.load_source_plugins(plugins_root)
    if errors:
        raise PublishError("; ".join(errors))
    by_identity = {plugin.identity: plugin for plugin in plugins if plugin.publishable}

    base = local_prefix(release_base)
    entries = []
    seen = set()
    for entry in registry["plugins"]:
        if not entry["url"].startswith(base):
            continue
        identity = (entry["name"], entry["version"])
        plugin = by_identity.get(identity)
        if plugin is None:
            raise PublishError(
                f"{identity[0]}@{identity[1]}: registry entry points at this repo "
                f"but plugins/{identity[0]}/ has no publishable manifest at that "
                f"version; bump the version or restore the source"
            )
        if pluginpack.registry_metadata_view(entry) != pluginpack.manifest_metadata(
            plugin.manifest
        ):
            raise PublishError(
                f"{identity[0]}@{identity[1]}: manifest fields differ from the "
                f"registry entry; run tools/build_plugins.py first"
            )
        entries.append((plugin, entry))
        seen.add(identity)

    for identity, plugin in sorted(by_identity.items()):
        if identity in seen:
            continue
        wasm = plugin.dir / plugin.wasm_path
        if not wasm.is_file():
            continue  # not built: run tools/build_plugins.py first
        entry = {
            **pluginpack.manifest_metadata(plugin.manifest),
            "url": f"{base}{plugin.name}-{plugin.version}.zip",
            "sha256": pluginpack.package_sha256(
                plugin.dir, plugin.name, plugin.wasm_path
            ),
        }
        entries.append((plugin, entry))

    entries.sort(key=lambda item: pluginpack.entry_key(item[1]))
    return entries


def checked_artifacts(
    entries: list[tuple[pluginpack.Plugin, dict]], dist: Path
) -> dict[tuple[str, str], Path]:
    """Map each local identity to its zip in ``dist``, verifying its digest."""
    archives: dict[tuple[str, str], Path] = {}
    for plugin, entry in entries:
        identity = (entry["name"], entry["version"])
        expected_name = f"{identity[0]}-{identity[1]}.zip"
        archive = dist / expected_name
        if not archive.is_file():
            raise PublishError(
                f"{archive}: missing; run `uv run --python 3.13 python "
                f"tools/build_plugins.py --package`"
            )
        actual = pluginpack.zip_sha256(archive)
        if actual != entry["sha256"]:
            raise PublishError(
                f"{archive}: sha256 {actual[:12]}… does not match registry "
                f"{entry['sha256'][:12]}…; rebuild with tools/build_plugins.py"
            )
        archives[identity] = archive
    return archives


# ── remote release ─────────────────────────────────────────────────────────


def release_assets(runner: Runner, release_tag: str) -> set[str]:
    try:
        output = runner.run(
            "gh", "release", "view", release_tag, "--json", "assets",
            "--jq", ".assets[].name", capture=True,
        )
    except PublishError:
        print(f"  release {release_tag!r} does not exist yet")
        return set()
    return {line.strip() for line in output.splitlines() if line.strip()}


def download(url: str, destination: Path) -> None:
    token = os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN")
    request = urllib.request.Request(
        url,
        headers={
            "User-Agent": "hanaclaw-publish-verifier",
            **({"Authorization": f"Bearer {token}"} if token else {}),
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=120) as response:
            destination.write_bytes(response.read())
    except (urllib.error.HTTPError, urllib.error.URLError, TimeoutError) as e:
        raise PublishError(f"downloading {url}: {e}") from e


def file_sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def verify_remote_asset(runner: Runner, release_tag: str, archive: Path) -> bool:
    """True when the uploaded asset is byte-identical to ``archive``.

    Returns False when the asset is absent. Raises ImmutableAssetError when
    it exists with different bytes.
    """
    asset = archive.name
    if asset not in release_assets(runner, release_tag):
        return False
    with tempfile.TemporaryDirectory(prefix="hanaclaw-verify-") as tmp:
        runner.run(
            "gh", "release", "download", release_tag,
            "--pattern", asset, "--dir", tmp, capture=not runner.dry_run,
        )
        remote = Path(tmp) / asset
        if file_sha256(remote) != file_sha256(archive):
            raise ImmutableAssetError(
                f"{asset} already exists on release {release_tag!r} with different "
                f"bytes; published identities are immutable — bump version in "
                f"manifest.toml and Cargo.toml, then rebuild"
            )
    print(f"  verified existing immutable asset {asset}")
    return True


def verify_urls(entries: list[tuple[pluginpack.Plugin, dict]]) -> None:
    with tempfile.TemporaryDirectory(prefix="hanaclaw-urls-") as tmp:
        for plugin, entry in entries:
            asset = f"{entry['name']}-{entry['version']}.zip"
            path = Path(tmp) / asset
            download(entry["url"], path)
            actual = file_sha256(path)
            if actual != entry["sha256"]:
                raise PublishError(
                    f"{entry['name']}@{entry['version']} release digest {actual} "
                    f"does not match registry {entry['sha256']}"
                )
            print(f"  verified {asset}")


# ── registry ───────────────────────────────────────────────────────────────


def registry_from_ref(runner: Runner, ref: str) -> dict:
    text = runner.git("show", f"{ref}:registry.json", capture=True)
    try:
        return json.loads(text)
    except json.JSONDecodeError as e:
        raise PublishError(f"{ref}:registry.json is not valid JSON: {e}") from e


def commit_registry(runner: Runner, registry_path: Path, source_ref: str) -> None:
    relative = registry_path.relative_to(REPO_ROOT).as_posix()
    runner.git("add", "--", relative)
    staged = runner.git("diff", "--cached", "--name-only", capture=True)
    if not staged.split():
        print("  registry.json already committed and current; nothing to do")
        return
    if staged.split() != [relative]:
        raise PublishError(f"only {relative} may be committed, staged: {staged!r}")
    head = runner.git("rev-parse", "HEAD", capture=True).strip()
    if source_ref != head:
        print(
            f"  note: registry.json comes from {source_ref}, but HEAD is {head}; "
            f"commit the plugin source first"
        )
    runner.git(
        "commit", "--only", "-m", "chore: publish plugin registry entries",
        "--", relative,
    )
    print(f"  committed {relative}. Push it, then install from the raw URL.")


# ── CLI ────────────────────────────────────────────────────────────────────


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Upload registry.json's zips to the plugins release, "
        "verify them, and commit registry.json."
    )
    parser.add_argument("--registry", type=Path, default=REPO_ROOT / "registry.json")
    parser.add_argument("--plugins", type=Path, default=REPO_ROOT / "plugins")
    parser.add_argument("--dist", type=Path, default=REPO_ROOT / "dist",
                        help="directory holding the zips built by build_plugins.py")
    parser.add_argument("--release-tag", default=pluginpack.PLUGIN_RELEASE_TAG)
    parser.add_argument("--release-base", default=pluginpack.DEFAULT_RELEASE_BASE)
    parser.add_argument("--ref", help="verify/commit the registry at this git ref "
                        "instead of the working copy")
    parser.add_argument("--dry-run", action="store_true", help="print the plan only")
    parser.add_argument("--verify-only", action="store_true",
                        help="check URLs and digests; no upload, no commit")
    parser.add_argument("--no-commit", action="store_true", help="upload but do not commit")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    runner = Runner(dry_run=args.dry_run)
    try:
        registry_path = args.registry.resolve()
        if args.ref:
            registry = registry_from_ref(runner, args.ref)
        else:
            registry, _ = pluginpack.load_registry(registry_path)

        entries = local_entries(registry, args.plugins, args.release_base)
        if not entries:
            print("no local registry entries; nothing to publish")
            return 0

        if args.verify_only:
            print(f"verify: {len(entries)} local entr{'y' if len(entries) == 1 else 'ies'}")
            verify_urls(entries)
            print("verified every local registry URL and digest")
            return 0

        archives = checked_artifacts(entries, args.dist)
        print(f"publish: {len(archives)} local package(s)")

        missing = [
            archive
            for archive in archives.values()
            if not verify_remote_asset(runner, args.release_tag, archive)
        ]
        if missing:
            if not release_assets(runner, args.release_tag):
                runner.run(
                    "gh", "release", "create", args.release_tag,
                    "--title", "Plugin artifacts",
                    "--notes", "Rolling plugin zips for the registry.",
                )
            for archive in missing:
                runner.run("gh", "release", "upload", args.release_tag, str(archive))
                print(f"  uploaded {archive.name}")

        if runner.dry_run:
            print("dry run: skipped URL verification, commit, and push")
            return 0
        verify_urls(entries)
        print("verified every local registry URL and digest")

        if not args.no_commit:
            source_ref = args.ref or runner.git("rev-parse", "HEAD", capture=True).strip()
            commit_registry(runner, registry_path, source_ref)
    except ImmutableAssetError as e:
        print(f"error: {e}", file=sys.stderr)
        return 2
    except (PublishError, PackageError) as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
