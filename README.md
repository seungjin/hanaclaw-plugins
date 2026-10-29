# hanaclaw-plugins

Personal ZeroClaw plugin catalog for
[hanaclaw](https://github.com/seungjin/hanaclaw), a fork of ZeroClaw. It
publishes its own `registry.json` so `hanaclaw plugin install <name>` (or
`zeroclaw plugin install <name>`) resolves both the official plugins and the
ones built here.

```
registry.json                      # GENERATED: upstream catalog + local plugins
plugins/<name>/                    # local plugin crates (manifest.toml + built .wasm)
wit/v0/                            # vendored ZeroClaw contract (+ pin, .sha256)
templates/zeroclaw-tool-plugin     # cargo generate template for a new tool plugin
tools/build_plugins.py             # build + package + refresh registry.json
tools/publish_plugins.py           # upload zips to the release, verify, commit
tools/update_zeroclaw_registry.py  # upstream half only (no build)
tools/fetch_zeroclaw_wit.py        # vendor / update wit/<version>/
tools/pluginpack.py                # shared zip, manifest, and digest contract
```

Everything runs from the repository root, and every Python command goes
through `uv run --python 3.13 python …` (see [Python](#python)).

Quick answers:

| I want to… | Run |
|---|---|
| build, package, and register a plugin | `tools/build_plugins.py --package` |
| publish the zips and commit the registry | `tools/publish_plugins.py` |
| pull upstream's newest catalog entries | `tools/update_zeroclaw_registry.py` |
| move to a newer WIT contract | `tools/fetch_zeroclaw_wit.py`, then bump versions |
| check whether anything is stale | `--check` on the build, registry, and WIT tools |
| install a plugin from this catalog | `zeroclaw plugin install <name> --registry <raw URL>` |

## Build: from source to installable plugin

One command formats, tests, lints, builds every publishable plugin for
`wasm32-wasip2`, packages the reproducible release zips into `dist/`, and
rewrites `registry.json`:

```bash
rustup target add wasm32-wasip2    # once

uv run --python 3.13 python tools/build_plugins.py --package
```

Then upload the zips and commit the refreshed registry:

```bash
uv run --python 3.13 python tools/publish_plugins.py --dry-run   # show the plan
uv run --python 3.13 python tools/publish_plugins.py             # upload + verify + commit
```

| Flag | Effect |
|---|---|
| `--package [DIR]` | also write `dist/<name>-<version>.zip` (default `dist/`) |
| `--plugin NAME` | build only that plugin (repeatable) |
| `--check` | exit 2 if any wasm, zip, or registry entry is stale |
| `--skip-build` | reuse the `.wasm` next to each manifest |
| `--skip-tests`, `--no-clippy`, `--skip-fmt-check` | local iteration only; CI runs all of them |
| `--skip-metadata` | do not fail when an already-indexed version's bytes or registry fields changed |
| `--prune` | deliberately drop entries whose `plugins/<name>/` is gone |
| `--offline` | reuse `registry.json`'s upstream entries (no network) |
| `--upstream URL`, `--upstream-ref REF` | inherit upstream from another URL or branch/tag/SHA |
| `--release-base URL` | base URL of this repo's release assets |
| `--retirements FILE` | drop the `name@version` identities in a retirement ledger |

Nothing generated is committed: `*.wasm`, `target/`, and `dist/` are
gitignored. Only `registry.json` (text) records what was published.

### Publish flags

```bash
uv run --python 3.13 python tools/publish_plugins.py             # upload + verify + commit
uv run --python 3.13 python tools/publish_plugins.py --dry-run   # print the plan, change nothing
uv run --python 3.13 python tools/publish_plugins.py --no-commit # upload + verify, leave git alone
uv run --python 3.13 python tools/publish_plugins.py --verify-only --ref main
```

| Flag | Effect |
|---|---|
| `--dry-run` | print every `gh`/`git` command; no upload, download, or commit |
| `--verify-only` | re-download every local registry URL and check its sha256; no upload, no commit |
| `--ref REF` | read `git show <ref>:registry.json` instead of the working copy |
| `--no-commit` | upload and verify, but do not commit `registry.json` |
| `--release-tag TAG` | release that holds the assets (default `plugins`) |
| `--dist DIR`, `--registry FILE`, `--plugins DIR` | override the default paths |

`--verify-only` needs no upload rights, so it is the cheap check to run before
installing from the catalog or in CI against a pushed ref.

### Published versions are immutable

Each `name@version` maps to exactly one zip, verified by sha256 at install
time. Editing a plugin that was already published makes the build fail:

```
error: hello-from-hana@0.1.0 is already published with sha256 3bb8e5cc…
       but the rebuilt package is 55a58d13…; bump version in manifest.toml
       and Cargo.toml
```

Bump `version` in **both** `manifest.toml` and `Cargo.toml`, rebuild, and
publish. The new zip is uploaded next to the old one; old versions stay
installable. `publish_plugins.py` refuses to overwrite a release asset whose
bytes differ, and re-verifies every local entry's URL and digest by downloading
it.

## Registry

`registry.json` = every entry from
[zeroclaw-labs/zeroclaw-plugins](https://github.com/zeroclaw-labs/zeroclaw-plugins)
verbatim, plus one entry per `plugins/<name>/` with a `manifest.toml` lacking
`registry = false`. Local `url`s point at this repo's `plugins` release and
their `sha256` is the package zip's digest, computed in upstream's reproducible
zip format (`<name>/manifest.toml` + `<name>/<wasm>`, sorted, fixed timestamp,
deflate level 6).

To refresh only the upstream half (no build):

```bash
uv run --python 3.13 python tools/update_zeroclaw_registry.py          # fetch + rewrite
uv run --python 3.13 python tools/update_zeroclaw_registry.py --check  # CI: exit 2 if stale
uv run --python 3.13 python tools/update_zeroclaw_registry.py --offline
```

Upstream's `main` moves independently of this repo. Pin it to make a refresh
reproducible, read a local checkout with `--upstream-file`, or preview with
`--stdout`:

```bash
uv run --python 3.13 python tools/update_zeroclaw_registry.py \
  --upstream-ref <sha-or-tag>
uv run --python 3.13 python tools/update_zeroclaw_registry.py \
  --upstream-file ../zeroclaw-plugins/registry.json
uv run --python 3.13 python tools/update_zeroclaw_registry.py --stdout
```

Upstream also ships `registry-retirements.json`, the ledger of identities the
current host can no longer install
(`{"retired": [{"name", "version", "sha256", "reason"}]}`). It is **not**
vendored here; pass its path when you want those entries dropped:

```bash
uv run --python 3.13 python tools/build_plugins.py --package \
  --retirements ../zeroclaw-plugins/registry-retirements.json
```

Install with this registry:

```bash
zeroclaw plugin install <name> --registry https://raw.githubusercontent.com/seungjin/hanaclaw-plugins/main/registry.json
```

## Recipes

**Re-package without re-running cargo.** `--skip-build` trusts the `.wasm`
next to each manifest (no wasm target needed):

```bash
uv run --python 3.13 python tools/build_plugins.py --skip-build --package
```

**Iterate fast on one plugin.** This skips tests, clippy, and the
immutability check — never finish with one of these runs.

```bash
uv run --python 3.13 python tools/build_plugins.py --package \
  --plugin my-plugin --skip-tests --no-clippy --skip-metadata
```

**Publish a new version of an existing plugin.** Bump `version` in both
`manifest.toml` and `Cargo.toml`, then:

```bash
uv run --python 3.13 python tools/build_plugins.py --package --plugin my-plugin
uv run --python 3.13 python tools/publish_plugins.py
```

**Retire a plugin.** Delete `plugins/<name>/` and rebuild with `--prune`, so
dropping its registry entry is an explicit choice:

```bash
uv run --python 3.13 python tools/build_plugins.py --package --prune
```

**Work offline.** `--offline` reuses the upstream entries already in
`registry.json`; only local plugins are rebuilt.

```bash
uv run --python 3.13 python tools/build_plugins.py --package --offline
```

**Keep an unreleased plugin out of the catalog.** Leave `registry = false` in
its `manifest.toml`; both the build and the registry ignore it.

## Troubleshooting

| Symptom | Cause / fix |
|---|---|
| `… is already published with sha256 … but the rebuilt package is …` | The source changed under a published version. Bump `version` in **both** `manifest.toml` and `Cargo.toml`. `--skip-metadata` only silences the check; never publish with it. |
| `… changed registry metadata … after it was published` | Description, author, capabilities, provides, or sender_match changed for a published version. Bump the version. |
| `registry.json is stale` (exit 2) | Run `tools/build_plugins.py --package`, or `tools/update_zeroclaw_registry.py` for the upstream half only. |
| `wit/v0 does not match .sha256` | Hand-edited WIT? Restore with `tools/fetch_zeroclaw_wit.py --ref "$(cat wit/v0/UPSTREAM_REF)"`, or adopt a new contract deliberately. |
| Build fails right after a WIT refresh | A newer contract changes the wasm bytes. Bump every plugin version first. |
| `--locked` fails / `Cargo.lock` missing | `cargo generate` writes no lockfile. Build once (the tools do not pass `--locked` themselves), then commit the `Cargo.lock`. |
| New plugin builds but is absent from `registry.json` | `registry = false` is still in its manifest, or the manifest `name` differs from `plugins/<name>/`. |
| `gh: not logged in`, or push rejected | `gh auth login` with push access to this repo, and add a git `origin` remote. |
| Digests differ from upstream CI | Wrong interpreter: use `uv run --python 3.13`. A zlib-ng build recompresses differently; the tools warn about it. |
| `publish_plugins.py` reports a missing dist zip | Run `tools/build_plugins.py --package` first; publishing never builds. |
| `cargo generate` ignores template edits | It reads the template from the git index: `git add templates/` first. |

## WIT contract

Plugins bind the host's `tool-plugin` (or channel/memory) world, so `wit/v0/`
must match the host exactly. Every plugin crate points at it with
`path: "../../wit/v0"` — there is one copy, not one per plugin.

`wit/v0/` is upstream's tree (`zeroclaw/plugins/wit/v0` in
zeroclaw-labs), copied verbatim. That includes `wit/v0/README.md`: it is
listed in `.sha256`, so do not add local notes to it — a single byte change
fails `--check`. Note that its `wit/VERSIONING.md` reference points at
upstream's repo root, which this catalog does not vendor.

```bash
uv run --python 3.13 python tools/fetch_zeroclaw_wit.py             # vendor zeroclaw@master
uv run --python 3.13 python tools/fetch_zeroclaw_wit.py --diff      # what would change
uv run --python 3.13 python tools/fetch_zeroclaw_wit.py --ref <sha> # pin a specific contract
uv run --python 3.13 python tools/fetch_zeroclaw_wit.py --check     # offline drift check
```

`--ref` accepts a branch, tag, or 40-hex SHA. To copy a contract you already
have on disk (for example a zeroclaw-plugins checkout's `wit/`, which is the
authoritative source for a release) use the local form instead — the pin is
then recorded as `local`:

```bash
uv run --python 3.13 python tools/fetch_zeroclaw_wit.py \
  --upstream-dir ../zeroclaw-plugins/wit
```

`wit/v0/UPSTREAM_REF` records the ZeroClaw commit zeroclaw-plugins pins, and
`wit/v0/.sha256` records per-file digests. `build_plugins.py` refuses to
compile against a tree that does not match its own `.sha256`. A WIT refresh
changes the wasm bytes, so bump plugin versions before publishing.

## New plugin

```bash
cargo generate --path templates/zeroclaw-tool-plugin --destination plugins \
  --name my-plugin --define tool_name=my_tool \
  --define tool_description="Does the thing" --define author=seungjin \
  --define registry=true --define config_read=false

uv run --python 3.13 python tools/build_plugins.py --package --plugin my-plugin
uv run --python 3.13 python tools/publish_plugins.py
```

Then edit `plugins/my-plugin/src/core.rs` and the argument struct in
`src/lib.rs`. `registry=false` keeps work-in-progress out of the build and the
registry entirely; remove the guard when it is ready.
`config_read=true` wires the whole `config_read` + `[config_schema]` +
`__config` path.

`cargo generate` reads the template from the git **index**, so stage template
changes (`git add templates/`) before generating. The first build creates
`Cargo.lock`; commit it.

See [templates/zeroclaw-tool-plugin/README.md](./templates/zeroclaw-tool-plugin/README.md).
Requires `cargo-generate` 0.25.x and `cargo` 1.85+ (edition 2024).

## Python

All Python runs through `uv`; never call `python`/`python3` directly.

```bash
uv run --python 3.13 python tools/<script>.py --help
```

`--python 3.13` is deliberate: it is uv-managed CPython built against **stock
zlib**, so package digests match what upstream CI computes. A distro Python
linked against zlib-ng compresses differently and yields different `sha256`
values (the tools warn when they detect that).

The tools share `tools/pluginpack.py` with upstream's
`tools/build-registry.py` + `tools/registry_contract.py`: one implementation
of the manifest vocabulary, identity rules, and zip format. Change packaging
behavior there, never in one caller. Lint with `uvx ruff check tools/`
(standard library only; no requirements file).
