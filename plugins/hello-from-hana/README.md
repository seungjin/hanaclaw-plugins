# hello-from-hana

Minimal ZeroClaw **tool plugin** in this catalog: `hello_from_hana`.

Written with AI/LLM assistance (OpenAI Codex, `deepseek-v4.1-flash`); see the
repository's [NOTICE](../../NOTICE). Licensed MIT OR Apache-2.0; the crate
declares `license = "MIT OR Apache-2.0"` and ships no license files of its own
(they live at the repository root).

It returns "Hello from Hana!" and, when the model passes `name`, adds a
greeting for that name. It is the smallest end-to-end example of the
build → package → publish → install flow.

## How it is shaped

- `src/core.rs` — pure logic (`run`, `ToolInput`); `cargo test` runs it on the
  host with no wasm toolchain.
- `src/lib.rs` — `#[cfg(target_family = "wasm")]` shim that exports the
  `tool-plugin` world from the repository-wide `wit/v0` contract
  (`path: "../../wit/v0"`). There is no plugin-local `wit/` copy.
- `manifest.toml` — install metadata (`capabilities = ["tool"]`, no
  permissions required, no `registry = false`, so it is published).
- `tests/smoke.rs` — host-run integration test over the core.

## Build, package, publish

From the repository root:

```bash
cargo test                                   # inside this directory, native
uv run --python 3.13 python tools/build_plugins.py --package --plugin hello-from-hana
uv run --python 3.13 python tools/publish_plugins.py
```

`build_plugins.py` writes `hello_from_hana.wasm` here (gitignored), builds
`dist/hello-from-hana-0.1.1.zip`, and regenerates `registry.json` with this
repo's release URL and the zip's sha256. `publish_plugins.py` uploads the zip
to the `plugins` release and verifies the download.

The plugin only becomes installable after the upload: the registry entry
alone points at a URL that does not exist yet. That URL is a GitHub Release
asset, identical in shape to upstream's:

```
https://github.com/seungjin/hanaclaw-plugins/releases/download/plugins/hello-from-hana-0.1.1.zip
```

`publish_plugins.py` creates the `plugins` release on first use, then only
adds missing `name-version.zip` assets (never overwrites bytes).

## Install

```bash
# `zeroclaw` here is the host binary you built (or stock zeroclaw).
zeroclaw plugin install hello-from-hana \
  --registry https://raw.githubusercontent.com/seungjin/hanaclaw-plugins/main/registry.json
```

The model then sees the tool as `hello_from_hana` with an optional `name`
argument.

## Changing it

Published `name@version` identities are immutable. After editing source or
`manifest.toml`, bump `version` in **both** `manifest.toml` and `Cargo.toml`,
then rebuild; `build_plugins.py` refuses to reuse the old digest:

```bash
# manifest.toml: version = "0.1.1"   Cargo.toml: version = "0.1.1"
uv run --python 3.13 python tools/build_plugins.py --package --plugin hello-from-hana
uv run --python 3.13 python tools/publish_plugins.py
```

If `publish_plugins.py` exits 2 with "already exists with different bytes",
the version was not bumped before the rebuild.
