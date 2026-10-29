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

CI splits the work: a shard builds one plugin's wasm, then the packaging job
re-zips it inside the pinned image, which is the only place a digest is
decided. Two flags exist for that split:

* ``--skip-package`` builds (or, with ``--skip-build``, checks) the wasm but
  writes no zip, does not check or rewrite ``--output``, and never fails on an
  already-published identity. It exits 0 when every built plugin packaged
  cleanly -- the point is that a *later* job must reproduce the committed
  digest from these exact bytes.
* ``--baseline PATH`` (only meaningful with ``--offline``) inherits entries
  from ``PATH`` instead of from ``--output``, so a partial build can render a
  registry beside the checkout and compare it to the committed one. Local
  entries not rebuilt this run keep their published ``url``/``sha256``.

A partial build is not a replacement for the full sweep: only a push that
builds every plugin proves the whole ``registry.json`` is reproducible.

Exit codes: 0 = ok, 2 = drift with ``--check``, 1 = error. Requires Python
3.11+ (tomllib) and ``cargo``.
"""
from __future__ import annotations

import argparse
import hashlib
import os
import re
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
# The toolchain the pinned release bytes were built with. Each plugin is a
# standalone crate (`[workspace]`), so the repository-root rust-toolchain.toml
# does NOT apply to `cargo` inside plugins/<name>/: without this every build
# would silently use whatever the machine/runner defaults to, and changing a
# published digest as a side effect.
TOOLCHAIN_FILE = REPO_ROOT / "rust-toolchain.toml"
# Compiler flags that make a wasm byte-identical across checkouts. The
# component embeds absolute paths of every dependency source it names, so the
# same lockfile under /home/<user>/.cargo and /home/runner/.cargo otherwise
# produces different (and differently sized) bytes for the same sources.
REMAP_FLAG = "--remap-path-prefix={source}=/cargo/registry"
# Plugins bind `wit/v0` (see each crate's `wit_bindgen::generate!`), so a
# drifted tree silently changes the bytes of every wasm built here.
WIT_CONTRACT = REPO_ROOT / "wit" / "v0"


class BuildError(Exception):
    """A fatal build problem; reported without a traceback."""


# ── cargo ──────────────────────────────────────────────────────────────────


def cargo_home() -> Path:
    """The registry a plain cargo build here uses (``$CARGO_HOME`` or ~/.cargo).

    Remapping its path is what makes the bytes identical on another machine.
    """
    return Path(os.environ.get("CARGO_HOME") or Path.home() / ".cargo")


def pinned_toolchain() -> str:
    """The ``channel`` from rust-toolchain.toml (the only source of truth)."""
    try:
        channel = tomllib.loads(TOOLCHAIN_FILE.read_text())["toolchain"]["channel"]
    except (OSError, tomllib.TOMLDecodeError, KeyError) as e:
        raise BuildError(
            f"cannot read [toolchain] channel from {TOOLCHAIN_FILE}: {e}"
        ) from e
    if not isinstance(channel, str) or not re.fullmatch(
        r"\d+\.\d+\.\d+", channel
    ):
        raise BuildError(
            f"{TOOLCHAIN_FILE}: channel must be a pinned x.y.z release, "
            f"not {channel!r}; a moving channel changes published bytes"
        )
    return channel


def cargo_env(plugin: pluginpack.Plugin) -> dict[str, str]:
    """Environment for every cargo run: pinned toolchain + path remapping."""
    env = dict(os.environ)
    env["RUSTUP_TOOLCHAIN"] = pinned_toolchain()
    registry = cargo_home() / "registry"
    # Both spellings: a symlinked home (/home vs /var/home) makes cargo print
    # whichever path it resolved, and a missing remap silently changes bytes.
    sources = dict.fromkeys(
        [registry, registry.resolve(), Path.home() / ".cargo" / "registry"]
    )
    flags = env.get("RUSTFLAGS", "").split()
    flags += [REMAP_FLAG.format(source=source) for source in sources]
    # The crate root is embedded as well; remap it so a checkout under a
    # different path (a second clone, CI) yields the same component.
    flags.append(f"--remap-path-prefix={plugin.dir.resolve()}=/plugin")
    env["RUSTFLAGS"] = " ".join(flags)
    return env


def cargo(plugin: pluginpack.Plugin, args: list[str]) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["cargo", *args], cwd=plugin.dir, env=cargo_env(plugin), check=False
    )


def run_cargo(plugin: pluginpack.Plugin, args: list[str], *, stage: str) -> None:
    command = ["cargo", *args]
    print(f"  [{plugin.name}] {stage}: {' '.join(command)}")
    result = cargo(plugin, args)
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
    result = cargo(plugin, ["generate-lockfile"])
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
    destination = place_wasm(args, plugin, artifact)
    shown = (
        destination.relative_to(Path.cwd())
        if destination.is_relative_to(Path.cwd())
        else destination
    )
    print(f"  [{plugin.name}] -> {shown} ({destination.stat().st_size} bytes)")


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

    A partial build (``--plugin`` in CI's pull-request path) only has the wasm
    of the plugins it built. Published local entries it did not rebuild keep
    the recorded ``url``/``sha256`` from the baseline document: their bytes
    cannot change without a version bump (``stale_identities`` enforces it),
    and inheriting them is what makes the result comparable to a full render.
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
            *(
                ("--baseline", str(args.baseline))
                if args.offline and args.baseline
                else ()
            ),
            *(("--retirements", str(args.retirements)) if args.retirements else ()),
        ]
    )
    merged, _local, release_base = registry_tool.build(registry_args)
    recorded = recorded_local_entries(args)
    release_prefix = release_base.rstrip("/") + "/"
    for entry in merged["plugins"]:
        identity = (entry["name"], entry["version"])
        if not entry["url"].startswith(release_prefix):
            continue
        if identity in local_hashes:
            entry["sha256"] = local_hashes[identity]
        elif identity in recorded:
            entry.update(recorded[identity])
    merged["plugins"].sort(key=entry_key)
    return merged


def recorded_local_entries(
    args: argparse.Namespace,
) -> dict[tuple[str, str], dict]:
    """Published local entries (url, sha256) from the document being edited.

    ``--baseline`` names it when a partial build writes somewhere else; a full
    build reads its own ``--output``. Entries from the wrong release base are
    filtered by the caller.
    """
    source = args.baseline if args.offline and args.baseline else args.output
    if not source.is_file():
        return {}
    _, published = pluginpack.load_registry(source)
    return {identity: entry for identity, entry in published.items()}


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
    parser.add_argument(
        "--wasm-dir",
        type=Path,
        metavar="DIR",
        help="place built components at DIR/<name>/<wasm_path> and read them "
        "there afterwards (default: the plugins directory)",
    )
    parser.add_argument("--plugin", action="append", metavar="NAME",
                        help="build only these plugins (repeatable); also "
                        "checks registry = false ones without indexing them")
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
        "--skip-package", action="store_true",
        help="build the wasm only: write no zips, decide no identity, and "
        "leave --output untouched (CI's per-plugin shard step)",
    )
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
        help="reuse --baseline's (default: --output's) upstream entries, no network",
    )
    parser.add_argument(
        "--baseline",
        type=Path,
        help="registry to inherit entries from with --offline (default: --output)",
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


def wasm_dir(args: argparse.Namespace) -> Path:
    """Where ``<name>/<wasm_path>`` lives for this run."""
    return args.wasm_dir if args.wasm_dir is not None else args.plugins


def staged_wasm(args: argparse.Namespace, plugin: pluginpack.Plugin) -> Path:
    """The copy of the component that packaging reads this run."""
    return wasm_dir(args) / plugin.name / plugin.wasm_path


def place_wasm(
    args: argparse.Namespace, plugin: pluginpack.Plugin, artifact: Path
) -> Path:
    """Copy the cargo artifact both beside the manifest and to --wasm-dir.

    Both copies matter: the manifest directory is what plain `cargo build`
    plus a rerun of this tool sees, and --wasm-dir is what CI's shards
    upload and the packaging job re-reads.
    """
    contents = pluginpack.read_wasm(artifact)
    destinations = [
        args.plugins / plugin.name / plugin.wasm_path,
        staged_wasm(args, plugin),
    ]
    for destination in dict.fromkeys(destinations):
        destination.parent.mkdir(parents=True, exist_ok=True)
        # Copy bytes (never link): the zip packages exactly what the host loads.
        destination.write_bytes(contents)
    return staged_wasm(args, plugin)


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
                # `--plugin NAME` names work in progress explicitly; build and
                # check it, but never package or index it (registry = false).
                if not args.plugin:
                    print(f"  [{plugin.name}] registry = false: skipping build")
                    continue
                print(f"  [{plugin.name}] registry = false: building unindexed")
                if args.skip_build:
                    wasm = staged_wasm(args, plugin)
                    if not wasm.is_file():
                        raise BuildError(
                            f"{plugin.name}: --skip-build but {wasm} is missing"
                        )
                else:
                    build_plugin(args, plugin)
                continue

            wasm = staged_wasm(args, plugin)
            if args.skip_build:
                if not wasm.is_file():
                    raise BuildError(f"{plugin.name}: --skip-build but {wasm} is missing")
            else:
                build_plugin(args, plugin)
            if args.skip_package:
                # CI's shard step: prove the bytes build, write no zip, and
                # decide no identity. The pinned packaging job re-runs this
                # from the staged .wasm, so --output is not consulted here.
                hashes[plugin.identity] = hashlib.sha256(
                    pluginpack.read_wasm(wasm)
                ).hexdigest()
                continue
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

        if args.skip_package:
            print(f"built {len(publishable)} packageable plugin(s); no packaging")
            return 0

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

    if args.skip_package:
        # CI's components stage: the shards publish their .wasm artifacts and
        # a later job re-packages them (and the registry) in the pinned image.
        print(f"built {len(publishable)} packageable plugin(s); no packaging")
        return 0

    current = args.output.read_text() if args.output.is_file() else ""
    if args.check:
        if text == current:
            print(f"{args.output} is up to date ({len(merged['plugins'])} entries)")
            return 0
        # CI's partial build renders beside the checkout instead of
        # overwriting it, so --check must also accept the baseline's bytes:
        # the result has to equal the committed registry, not merely be
        # "no worse than" a stale copy of the output path.
        if (
            args.baseline
            and args.baseline.is_file()
            and args.output != args.baseline
            and text == args.baseline.read_text()
        ):
            print(
                f"{args.baseline} is up to date "
                f"({len(merged['plugins'])} entries)"
            )
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
