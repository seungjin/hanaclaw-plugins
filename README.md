> **Written with AI/LLM assistance.** Every file in this repository — the
> Rust plugin crates, the `cargo generate` template, the Python tools,
> the vendored WIT contract handling, and this documentation — was
> authored by an AI/LLM (OpenAI Codex, model `deepseek-v4.1-flash`) under
> human direction and review. Commits record this in the `Assisted-by:`
> trailer. The code is provided as-is: verify anything you intend to run in
> production, and do not assume it carries the provenance of hand-written
> upstream code.

# hanaclaw-plugins

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

Licensed under either of [Apache License, Version 2.0](./LICENSE-APACHE) or
[MIT](./LICENSE-MIT), at your option.

Personal ZeroClaw plugin catalog, used with
[hanaclaw](https://github.com/seungjin/hanaclaw) — my own build of ZeroClaw,
not a separate CLI. There is therefore no `hanaclaw` command: you install with
the binary you built (or stock `zeroclaw`) plus this catalog's `registry.json`,
which carries the official plugins and the ones built here.

```bash
# stock ZeroClaw
zeroclaw plugin install <name> --registry \
  https://raw.githubusercontent.com/seungjin/hanaclaw-plugins/main/registry.json

# your own build: the command is the binary you compiled
./target/release/zeroclaw plugin install <name> --registry \
  https://raw.githubusercontent.com/seungjin/hanaclaw-plugins/main/registry.json
```

Or point the host at this catalog once, for every install:

```bash
export ZEROCLAW_PLUGIN_REGISTRY_URL=https://raw.githubusercontent.com/seungjin/hanaclaw-plugins/main/registry.json
```

This repository is **originated from and modelled on**
[zeroclaw-labs/zeroclaw-plugins](https://github.com/zeroclaw-labs/zeroclaw-plugins):
its layout, `registry.json` schema, reproducible zip format,
`registry-retirements.json` semantics, and CI stage order are referenced from
that project, and its tooling is a smaller reimplementation of the same contract.
`wit/v0/` is upstream ZeroClaw code copied verbatim. See [NOTICE](./NOTICE).

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
tools/ci/                          # CI helpers (see CI section) + packager image pin
rust-toolchain.toml                # pinned compiler (every plugin pins it too)
LICENSE-MIT, LICENSE-APACHE        # MIT OR Apache-2.0 (wit/v0/ excluded)
NOTICE                             # AI/LLM authorship + third-party terms
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
| install a plugin from this catalog | `zeroclaw plugin install <name> --registry <raw URL>` (binary = your build) |

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
| `--skip-package` | build and stage wasm only; hash it, write no zip, touch no registry (CI shards) |
| `--wasm-dir DIR` | also copy each built wasm to `DIR/<name>/<wasm_path>` for artifact upload |
| `--baseline FILE` | with `--offline`, inherit the entries of `FILE` and render beside it (CI's PR path) |
| `--skip-tests`, `--no-clippy`, `--skip-fmt-check` | local iteration only; CI runs all of them |
| `--skip-metadata` | do not fail when an already-indexed version's bytes or registry fields changed |
| `--prune` | deliberately drop entries whose `plugins/<name>/` is gone |
| `--offline` | reuse `registry.json`'s upstream entries (no network); this repo's own entries are adopted from that document, and any no manifest produces any more are dropped |
| `--upstream URL`, `--upstream-ref REF` | inherit upstream from another URL or branch/tag/SHA |
| `--release-base URL` | base URL of this repo's release assets |
| `--retirements FILE` | drop the `name@version` identities in a retirement ledger |

Nothing generated is committed: `*.wasm`, `target/`, and `dist/` are
gitignored. Only `registry.json` (text) records what was published.

### The compiler is part of the artifact

Every publishable byte comes from compiler version + sources + flags, so all
three are pinned. `hello-from-hana@0.1.1`, the current example, is built by **1.98.1**
(`153973` bytes); building it with 1.96.1 produces a different binary and a
different, wrong `sha256`.

`rust-toolchain.toml` exists at the repository root **and** in each plugin
and in the template. The copies are not redundant: every `plugins/<name>/` is
its own workspace (`[workspace]` at the end of its `Cargo.toml`), so a plain
`cargo build` run inside one never reads the root pin. `tools/build_plugins.py`
therefore also exports `RUSTUP_TOOLCHAIN` from the root pin, so its cargo
invocations agree no matter what directory they start in.

```bash
python3 tools/ci/check_toolchain.py   # every pin + the workflow env agree
```

When the channel changes, bump it everywhere and rebuild; a new compiler is a
new binary, so every published plugin needs a new version too.

### Bytes must not depend on where you cloned

cargo embeds absolute dependency paths (`$CARGO_HOME/registry/...`) and the
crate's own absolute path in the wasm. Two checkouts, identical sources,
produced three different sizes (`154517`, `154021`, `153973` bytes). The build
therefore sets:

```
--remap-path-prefix=$CARGO_HOME/registry=/cargo/registry
--remap-path-prefix=<plugin dir>=/plugin
```

(`cargo_home()`/`cargo_env()` in `tools/build_plugins.py`.) The same commit
now yields the same `sha256` from any directory, which is what makes the
registry verifiable at all.

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
error: hello-from-hana@0.1.1 is already published with sha256 162c1cd5…
       but the rebuilt package is 9e1f4a2b…; bump version in manifest.toml
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

### `registry-retirements.json`

Upstream also ships `registry-retirements.json`, the ledger of identities the
current host can no longer install
(`{"retired": [{"name", "version", "sha256", "reason"}]}`). It answers "why did
this entry disappear?" — an installed catalog can keep advertising
`name@version`, but the host refuses to load it, so listing it is a broken
install. Each record is a permanent withdrawal, kept even after the entry
leaves `registry.json`, so users who saw it removed can find the reason.

This repo vendors upstream's copy at the repository root (same content, same
shape) so the two catalogs withdraw the same identities. It is honored only
when you pass it — nothing is dropped implicitly:

```bash
uv run --python 3.13 python tools/build_plugins.py --package \
  --retirements registry-retirements.json
```

To withdraw one of **your** entries, add a `{"name", "version", "reason"}`
record to `registry-retirements.json` *before* removing the plugin, commit
both, and let `tools/ci/check_registry_growth.py` verify the ledger already
covered the entry that vanished. Editing a plugin in place is never the answer
— retire the old identity, then publish a new version.

Install with this registry (the `zeroclaw` in the command is your build of
the host — this repository ships no binary):

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

## CI

`.github/workflows/validate.yml` mirrors upstream zeroclaw-plugins' stage order
— `fmt → changes → (registry ‖ wit-drift ‖ template) → components (matrix) →
package → gate` — but drives this repo's tools. `publish.yml` is the release
workflow.

| Job | What it proves |
|---|---|
| `fmt` | `git diff --check`, plus rustfmt on plugins whose `.rs` files this PR touched (untouched crates may carry pre-existing debt; their published bytes cannot be reformatted without a version bump) |
| `changes` | `tools/ci/plan_matrix.py` shards plugins; `mode=changed` for a PR, `all` for a push |
| `registry` | toolchain pins agree, `ruff`, WIT checksums, `update_zeroclaw_registry.py --check --offline` in the pinned image, and that `registry.json` only grows |
| `wit-drift` | vendored `wit/v0/` still equals the pinned ZeroClaw source |
| `template` | `cargo generate` from `templates/` builds every combination |
| `components` | per shard: tests, clippy, `--skip-fmt-check` where allowed, then `build_plugins.py --plugin … --wasm-dir staged --skip-package`; uploads `staged/<name>/<wasm>.wasm` |
| `package` | re-packages those wasm **inside the pinned image** and checks the registry |

Two invariants make the split safe:

* The **wasm** is built by cargo on the runner; the **zip** is built only in
  the pinned image, so no digest is computed twice by different zlibs.
* A pull request builds only the touched plugins, so its registry is rendered
  beside the commit (`--baseline registry.json --output $RUNNER_TEMP/registry.json`)
  and must differ only in those plugins. A push builds everything and must
  reproduce `registry.json` byte for byte (`--check` with no baseline).

### The pinned packager image

Every digest-producing step runs in the image named by
`tools/ci/packager-image.txt`: the official Python image pinned by digest
(CPython 3.12, zlib 1.2.13). Deflate output is not guaranteed stable across
zlib versions or builds, and a one-byte difference changes the `sha256` that
users verify. The pin removes the question; this host happens to agree today
(3.13 / zlib 1.3.1 yields the same `162c1cd5…`), but that is luck, not a
contract. Never compute a published digest outside it:

```bash
tools/ci/run-packager-python.sh tools/build_plugins.py --skip-build --package dist --offline
```

The wrapper picks `docker`, else `podman`, mounts the repo at `/workspace`
writable (the tool rewrites `dist/` and `registry.json`), and runs with
`--network none`, a read-only root, and all capabilities dropped. Rootless
podman idmaps the mount so the caller's files are root-owned inside; the script
runs as `0:gid` there and as the caller's uid under docker, keeping host
ownership either way. `PACKAGER_CONTAINER` overrides the engine.

### Reproducing CI locally

```bash
# shard step (build + stage, no zip)
uv run --python 3.13 python tools/build_plugins.py \
  --plugin hello-from-hana --wasm-dir staged --skip-package

# collect, then package in the pinned image
cp staged/hello-from-hana/hello_from_hana.wasm plugins/hello-from-hana/
RUNNER_TEMP=/var/tmp tools/ci/run-packager-python.sh \
  tools/build_plugins.py --skip-build --package dist --offline --check

# PR path: render beside the commit
RUNNER_TEMP=/var/tmp tools/ci/run-packager-python.sh \
  tools/build_plugins.py --skip-build --package /var/tmp/dist --offline --check \
  --baseline registry.json --output "$RUNNER_TEMP/registry.json"
```

Deviations from upstream's workflows, deliberate:

* No `summary.py` / GitHub Check-Run attestation step; the `gate` job is a
  plain required check over `needs`.
* No zips are cached between jobs — only `~/.cargo` and `target/`. Artifacts
  carry wasm only, so a zip is always produced once, in one place.

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
| Same sources, different `sha256` between machines/checkouts | Absolute paths leaked into the wasm. Build through `tools/build_plugins.py` (it remaps `$CARGO_HOME/registry` and the plugin dir), never a bare `cargo build`, and check the compiler: `python3 tools/ci/check_toolchain.py`. |
| `error: … is already published with sha256 …` after only editing `Cargo.toml` | The version lives in two places. Bump `manifest.toml` too, then `cargo update -p <name> --precise <version>` (or let the build do it) so `Cargo.lock` matches. |
| `--offline` registry keeps an old version that no plugin builds | Fixed: entries under this repo's own release base are now dropped when no manifest produces them. If you still see one, it is upstream's and needs `--retirements`. |
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

## Provenance

This project is AI/LLM-written. See [NOTICE](./NOTICE) for the full statement.
In short:

* All source, tooling, and docs were generated by OpenAI Codex running
  `deepseek-v4.1-flash`, directed and reviewed by Seungjin Kim.
* Commits carry an `Assisted-by: codex-cli/<version> (<model>)` trailer next
  to the human `Signed-off-by:`.
* `wit/v0/` is the exception to "written here": it is upstream ZeroClaw code,
  copied verbatim and pinned by `wit/v0/UPSTREAM_REF`, with digests in
  `wit/v0/.sha256`. Its copyright and license are upstream's.

## License

Dual-licensed under either of

* Apache License, Version 2.0 ([LICENSE-APACHE](./LICENSE-APACHE))
* MIT license ([LICENSE-MIT](./LICENSE-MIT))

at your option. This is the same choice Rust itself and most of the
ecosystem offer.

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in this project by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.

`wit/v0/` is excluded from this grant: it is upstream ZeroClaw material and
carries upstream's terms. See [NOTICE](./NOTICE).
