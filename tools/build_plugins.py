#!/usr/bin/env python3
"""Build every local plugin, package it, and refresh registry.json.

This is the "build my project" entry point. For each ``plugins/<name>/`` with a
``manifest.toml`` it:

1. validates the manifest against the host's install contract
   (capabilities, permissions, Cargo.toml version == manifest version, ...),
2. runs the native checks: ``cargo fmt --all -- --check``,
   ``cargo test --locked``, and ``cargo clippy --locked --all-targets --
   -D warnings`` (the last two skippable, formatting is not),
3. builds the component: ``cargo build --locked --target wasm32-wasip2
   --release``,
4. copies the artifact next to ``manifest.toml`` at ``wasm_path`` (gitignored),
5. packages the reproducible ``<name>/<manifest.toml,wasm>`` zip that the
   release will hold, and
6. regenerates ``registry.json`` with this repo's release URL and the zip's
   sha256 (upstream catalog entries are inherited verbatim).

Then upload the zips and commit the new entry::

    uv run --python 3.13 python tools/build_plugins.py
    uv run --python 3.13 python tools/publish_plugins.py

Published ``<name>@<version>`` identities are immutable. If the bytes of an
already-published version change (source, manifest, toolchain, WIT), bump
``version`` in both ``manifest.toml`` and ``Cargo.toml``; ``--check`` reports
this and refuses to reuse the old digest. ``--package DIR`` also writes the
uploadable zips into ``DIR`` (default: ``dist/``, gitignored).

Usage (from the repository root)::

    uv run --python 3.13 python tools/build_plugins.py                 # build + refresh registry.json
    uv run --python 3.13 python tools/build_plugins.py --check         # CI: exit 2 if out of date
    uv run --python 3.13 python tools/build_plugins.py --package       # also write dist/*.zip
    uv run --python 3.13 python tools/build_plugins.py --plugin NAME   # one plugin only
    uv run --python 3.13 python tools/build_plugins.py --skip-tests --no-clippy --skip-metadata
    uv run --python 3.13 python tools/build_plugins.py --skip-build --skip-fmt-check

Formatting is never skippable: a rustfmt-clean tree is part of the contract
for reproducible review, matching upstream CI. ``--skip-build`` reuses the
``.wasm`` already next to each ``manifest.toml`` and needs no wasm target.

Exit codes: 0 = ok, 2 = drift with ``--check``, 1 = error. Requires Python
3.11+ (tomllib) and ``cargo``.
"""
from __future__ import annotations

import argparse
import os
import subprocess
import sys
import tomllib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import fetch_zeroclaw_wit
import pluginpack
import update_zeroclaw_registry as registry_tool
from pluginpack import PackageError, entry_key, render

REPO_ROOT = Path(__file__).resolve().parent.parent
WASM_TARGET = "wasm32-wasip2"
# Plugins bind `wit/v0` (see each crate's `wit_bindgen::generate!`), so a
# drifted tree silently changes the bytes of every wasm built here.
WIT_CONTRACT = REPO_ROOT / "wit" / "v0"


class BuildError(Exception):
    """A fatal build problem; reported without a traceback."""


# ── cargo ──────────────────────────────────────────────────────────────────


def run_cargo(plugin: pluginpack.Plugin, args: list[str], *, stage: str) -> None:
    command = ["cargo", *args]
    print(f"  [{plugin.name}] {stage}: {' '.join(command)}")
    result = subprocess.run(command, cwd=plugin.dir, check=False)
    if result.returncode != 0:
        raise BuildError(
            f"{plugin.name}: `cargo {args[0]}` failed (exit {result.returncode}); "
            f"rerun it in {plugin.dir} to see the output"
        )


def ensure_lockfile(plugin: pluginpack.Plugin) -> None:
    """Create Cargo.lock on a freshly generated crate.

    Every later cargo invocation uses ``--locked`` (upstream CI style, and what
    makes a packaged wasm reproducible). ``cargo generate`` from this repo's
    template does not write a lockfile, so resolve one once and commit it.
    """
    if (plugin.dir / "Cargo.lock").is_file():
        return
    print(f"  [{plugin.name}] {plugin.dir / 'Cargo.lock'} is missing: resolving")
    result = subprocess.run(
        ["cargo", "generate-lockfile"], cwd=plugin.dir, check=False
    )
    if result.returncode != 0:
        raise BuildError(f"{plugin.name}: `cargo generate-lockfile` failed")
    print(f"  [{plugin.name}] wrote Cargo.lock (commit it)")


def built_wasm(plugin: pluginpack.Plugin) -> Path:
    """Cargo's release artifact for this plugin's crate.

    The crate name (from Cargo.toml) names the artifact; ``wasm_path`` names
    where the host expects it. They normally differ only in ``-`` vs ``_``.
    """
    cargo = tomllib.loads((plugin.dir / "Cargo.toml").read_text())
    # Cargo turns '-' in the package name into '_' in the artifact filename.
    crate = cargo["package"]["name"].replace("-", "_")
    return plugin.dir / "target" / WASM_TARGET / "release" / f"{crate}.wasm"


def check_formatting(args: argparse.Namespace, plugin: pluginpack.Plugin) -> None:
    if not args.skip_fmt_check:
        run_cargo(plugin, ["fmt", "--all", "--", "--check"], stage="fmt")


def build_plugin(args: argparse.Namespace, plugin: pluginpack.Plugin) -> None:
    """One plugin: check, build, and place the wasm at ``wasm_path``."""
    if not (plugin.dir / "Cargo.toml").is_file():
        raise BuildError(f"{plugin.name}: no Cargo.toml")

    ensure_lockfile(plugin)
    check_formatting(args, plugin)
    if not args.skip_tests:
        run_cargo(plugin, ["test", "--locked"], stage="test")
    if not args.no_clippy:
        run_cargo(
            plugin,
            ["clippy", "--locked", "--all-targets", "--", "-D", "warnings"],
            stage="clippy (host)",
        )
        run_cargo(
            plugin,
            [
                "clippy",
                "--locked",
                "--target",
                WASM_TARGET,
                "--",
                "-D",
                "warnings",
            ],
            stage="clippy (wasm)",
        )
    run_cargo(
        plugin,
        ["build", "--locked", "--target", WASM_TARGET, "--release"],
        stage="build",
    )

    artifact = built_wasm(plugin)
    if not artifact.is_file() or artifact.stat().st_size == 0:
        raise BuildError(f"{plugin.name}: build produced no non-empty {artifact}")
    destination = plugin.dir / plugin.wasm_path
    # Copy bytes (never link): the zip packages exactly what the host loads.
    destination.write_bytes(artifact.read_bytes())
    print(f"  [{plugin.name}] -> {destination.relative_to(REPO_ROOT)} "
          f"({destination.stat().st_size} bytes)")


def check_wit_contract() -> list[str]:
    """Offline problems with the vendored WIT contract (empty when clean)."""
    return fetch_zeroclaw_wit.verify_local(WIT_CONTRACT)


# ── packaging ──────────────────────────────────────────────────────────────


def package_plugin(
    plugin: pluginpack.Plugin,
    out: Path,
    *,
    refresh_existing: bool,
    published_sha: str | None,
) -> str:
    """Write the reproducible zip and return its sha256.

    Rerunning the build is normal, so a zip already in ``out`` is only a
    problem when its identity is published (``published_sha``) and the rebuilt
    bytes differ from it: that asset is immutable. An unpublished version can
    be repackaged freely (local iteration), and ``refresh_existing``
    (``--check``) writes the fresh bytes so the caller can report the drift.
    """
    files = pluginpack.package_files(plugin.dir, plugin.wasm_path)
    path = out / f"{plugin.name}-{plugin.version}.zip"
    digest = pluginpack.zip_sha256(path) if path.is_file() else None
    if digest is not None:
        if published_sha is not None and digest != published_sha:
            raise BuildError(
                f"{path}: {plugin.name}@{plugin.version} is already published "
                f"with sha256 {published_sha[:12]}…, so its asset cannot be "
                f"replaced; bump version in manifest.toml and Cargo.toml"
            )
        if digest == pluginpack.package_sha256(
            plugin.dir, plugin.name, plugin.wasm_path
        ):
            return digest
        if not (refresh_existing or published_sha is None):
            raise BuildError(
                f"{path} already exists with different bytes; it is not "
                f"published, so remove it to repackage this version"
            )
    pluginpack.write_zip(path, plugin.name, files)
    size = path.stat().st_size
    if size > pluginpack.MAX_PLUGIN_ZIP_BYTES:
        raise BuildError(
            f"{path}: zip size {size} exceeds the host's "
            f"{pluginpack.MAX_PLUGIN_ZIP_BYTES}-byte download cap"
        )
    return pluginpack.zip_sha256(path)


def published_digests(args: argparse.Namespace) -> dict[tuple[str, str], str]:
    """sha256 of every identity already in the registry we are about to edit.

    This is the pre-rebuild document: an identity present there was published
    (here or upstream), so its bytes are immutable.
    """
    if not args.output.is_file():
        return {}
    _, published = pluginpack.load_registry(args.output)
    return {identity: entry["sha256"] for identity, entry in published.items()}


def newly_unpublishable(
    args: argparse.Namespace, plugins: list[pluginpack.Plugin]
) -> list[str]:
    """Published plugins that would silently vanish from the registry.

    Removing a plugin from the registry makes installs of that name/version
    fail. Retire it deliberately instead: delete the entry from registry.json
    in a dedicated commit, or keep it by leaving ``registry = false`` out.
    """
    if not args.output.is_file():
        return []
    _, published = pluginpack.load_registry(args.output)
    release_prefix = args.release_base.rstrip("/") + "/"
    still_here = {plugin.identity for plugin in plugins if plugin.publishable}
    return [
        f"{name}@{version} is in registry.json under {release_prefix} but no "
        f"publishable plugins/<name>/ builds it any more; removing it breaks "
        f"installs — re-add the plugin, or rerun with --prune"
        for (name, version), entry in sorted(published.items())
        if entry["url"].startswith(release_prefix)
        and (name, version) not in still_here
    ]


def stale_identities(
    args: argparse.Namespace, plugins: list[pluginpack.Plugin], hashes: dict
) -> list[str]:
    """Published identities whose bytes changed (must get a new version)."""
    if not args.output.is_file():
        return []
    _, published = pluginpack.load_registry(args.output)
    messages = []
    for plugin in plugins:
        old = published.get(plugin.identity)
        if old is None:
            continue
        if old["sha256"] != hashes[plugin.identity]:
            messages.append(
                f"{plugin.name}@{plugin.version} is already published with "
                f"sha256 {old['sha256'][:12]}… but the rebuilt package is "
                f"{hashes[plugin.identity][:12]}…; published identities are "
                f"immutable — bump version in manifest.toml and Cargo.toml"
            )
        elif pluginpack.registry_metadata_view(old) != pluginpack.manifest_metadata(
            plugin.manifest
        ):
            messages.append(
                f"{plugin.name}@{plugin.version} changed registry metadata "
                f"(description/author/capabilities/provides/sender_match) after "
                f"it was published; bump the version instead"
            )
    return messages


# ── registry refresh ───────────────────────────────────────────────────────


def rebuild_registry(
    args: argparse.Namespace, local_hashes: dict[tuple[str, str], str]
) -> dict:
    """Merge upstream + local entries, reusing known-good local digests.

    ``update_zeroclaw_registry.local_entries`` recomputes each digest from the
    bytes on disk; local_hashes lets this tool skip re-zipping (same bytes),
    while ``--check`` can still show a drift for a changed package without
    refusing to render the drift.
    """
    registry_args = registry_tool.parse_args(
        [
            "--upstream",
            args.upstream,
            *(
                ("--upstream-ref", args.upstream_ref)
                if args.upstream_ref
                else ()
            ),
            "--plugins",
            str(args.plugins),
            "--output",
            str(args.output),
            "--release-base",
            args.release_base,
            *(("--offline",) if args.offline else ()),
            *(("--retirements", str(args.retirements)) if args.retirements else ()),
        ]
    )
    merged, _local = registry_tool.build(registry_args)
    for entry in merged["plugins"]:
        identity = (entry["name"], entry["version"])
        if entry["url"].startswith(args.release_base.rstrip("/") + "/"):
            entry["sha256"] = local_hashes.get(identity, entry["sha256"])
    merged["plugins"].sort(key=entry_key)
    return merged


# ── CLI ────────────────────────────────────────────────────────────────────


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Build all plugins, package them, and refresh registry.json."
    )
    parser.add_argument(
        "--plugins", type=Path, default=REPO_ROOT / "plugins", help="plugins directory"
    )
    parser.add_argument(
        "--output", type=Path, default=REPO_ROOT / "registry.json", help="registry to write"
    )
    parser.add_argument(
        "--package",
        nargs="?",
        type=Path,
        const=REPO_ROOT / "dist",
        metavar="DIR",
        help="write uploadable zips here (default: dist/)",
    )
    parser.add_argument("--plugin", action="append", metavar="NAME",
                        help="build only these plugins (repeatable)")
    parser.add_argument("--check", action="store_true",
                        help="exit 2 if any wasm, zip, or registry entry is stale")
    parser.add_argument("--skip-tests", action="store_true", help="skip cargo test")
    parser.add_argument("--no-clippy", action="store_true", help="skip both clippy runs")
    parser.add_argument(
        "--skip-build", action="store_true",
        help="use the .wasm already next to each manifest (no cargo build)",
    )
    parser.add_argument(
        "--skip-fmt-check", action="store_true",
        help="skip `cargo fmt --check` (local iteration only; CI forbids it)",
    )
    parser.add_argument("--skip-metadata", action="store_true",
                        help="do not check whether an indexed version changed")
    parser.add_argument(
        "--prune", action="store_true",
        help="deliberately drop registry entries whose plugins/<name>/ is gone",
    )
    parser.add_argument(
        "--upstream",
        default=os.environ.get(
            "ZEROCLAW_REGISTRY_URL", pluginpack.DEFAULT_UPSTREAM_REGISTRY
        ),
        help="upstream registry URL to inherit (default: %(default)s)",
    )
    parser.add_argument(
        "--upstream-ref",
        metavar="REF",
        help="inherit upstream at this branch/tag/commit instead of main",
    )
    parser.add_argument(
        "--offline",
        action="store_true",
        help="reuse --output's upstream entries (no network)",
    )
    parser.add_argument(
        "--release-base",
        default=pluginpack.DEFAULT_RELEASE_BASE,
        help="base URL of this repo's release assets (default: %(default)s)",
    )
    parser.add_argument(
        "--retirements",
        type=Path,
        help="retirement ledger whose name@version identities to drop",
    )
    return parser.parse_args(argv)


def select_plugins(args: argparse.Namespace) -> list[pluginpack.Plugin]:
    plugins, errors = pluginpack.load_source_plugins(args.plugins)
    if errors:
        raise BuildError("; ".join(errors))
    if args.plugin:
        known = {plugin.name: plugin for plugin in plugins}
        missing = sorted(set(args.plugin) - known.keys())
        if missing:
            raise BuildError(f"unknown plugin(s): {', '.join(missing)}")
        plugins = [known[name] for name in args.plugin]
    if not plugins:
        raise BuildError(f"no plugins with a manifest.toml under {args.plugins}")
    return plugins


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    pluginpack.warn_if_zlib_ng()
    try:
        plugins = select_plugins(args)
        publishable = [p for p in plugins if p.publishable]
        hashes: dict[tuple[str, str], str] = {}
        # Identity -> sha256 currently in registry.json. Upstream entries are
        # here too, so a local plugin reusing an upstream name@version can
        # never upload different bytes.
        published = published_digests(args)

        wit_problems = check_wit_contract()
        if wit_problems and not args.skip_build:
            # The wasm is compiled against this tree: refuse to package bytes
            # from a contract that does not match its own checksum.
            raise BuildError(
                "; ".join(wit_problems)
                + "; refresh with `uv run --python 3.13 python "
                "tools/fetch_zeroclaw_wit.py`"
            )
        for problem in wit_problems:
            print(f"warning: {problem}", file=sys.stderr)

        print(f"build: {len(plugins)} plugin(s) under {args.plugins}")
        for plugin in plugins:
            if not plugin.publishable:
                print(f"  [{plugin.name}] registry = false: skipping build")
                continue
            if args.skip_build:
                wasm = plugin.dir / plugin.wasm_path
                if not wasm.is_file():
                    raise BuildError(f"{plugin.name}: --skip-build but {wasm} is missing")
            else:
                build_plugin(args, plugin)
            if args.package is not None:
                args.package.mkdir(parents=True, exist_ok=True)
                hashes[plugin.identity] = package_plugin(
                    plugin,
                    args.package,
                    refresh_existing=args.check,
                    published_sha=published.get(plugin.identity),
                )
            else:
                hashes[plugin.identity] = pluginpack.package_sha256(
                    plugin.dir, plugin.name, plugin.wasm_path
                )

        if not args.skip_metadata:
            stale = stale_identities(args, publishable, hashes)
            if stale:
                if not args.check:
                    raise BuildError("; ".join(stale))
                for message in stale:
                    print(f"error: {message}", file=sys.stderr)
            vanished = newly_unpublishable(args, plugins)
            if vanished and not args.prune:
                raise BuildError("; ".join(vanished))
            for message in vanished:
                print(f"note: pruning: {message}", file=sys.stderr)

        merged = rebuild_registry(args, hashes)
        text = render(merged)
    except (BuildError, PackageError) as e:
        print(f"error: {e}", file=sys.stderr)
        return 1

    current = args.output.read_text() if args.output.is_file() else ""
    if args.check:
        if text == current:
            print(f"{args.output} is up to date ({len(merged['plugins'])} entries)")
            return 0
        print(
            f"error: {args.output} is stale; run `uv run --python 3.13 python "
            f"tools/build_plugins.py`",
            file=sys.stderr,
        )
        return 2

    if text != current:
        tmp = args.output.with_suffix(".json.tmp")
        tmp.write_text(text)
        os.replace(tmp, args.output)
        print(f"wrote {args.output} with {len(merged['plugins'])} entries")
    else:
        print(f"{args.output} is already current ({len(merged['plugins'])} entries)")

    if publishable:
        print("\nnext: uv run --python 3.13 python tools/publish_plugins.py")
    return 0


if __name__ == "__main__":
    sys.exit(main())
