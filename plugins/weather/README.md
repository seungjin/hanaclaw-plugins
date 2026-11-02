# weather

ZeroClaw **tool plugin** that fetches current weather from [wttr.in](https://wttr.in).

Written with AI/LLM assistance (OpenAI Codex, `deepseek-v4.1-flash`); see the
repository's [NOTICE](../../NOTICE). Licensed MIT OR Apache-2.0; the crate
declares `license = "MIT OR Apache-2.0"` and ships no license files of its own
(they live at the repository root).

The model calls the `weather` tool with a `city` argument. The plugin returns a
concise current-weather report that explicitly says it came from the **weather
plugin** and includes the source URL (`https://wttr.in/<city>?format=j1`).

## How it is shaped

- `src/core.rs` — pure logic (`run`, `ToolInput`, `format_report`); `cargo test`
  runs it on the host with no wasm toolchain.
- `src/lib.rs` — `#[cfg(target_family = "wasm")]` shim that exports the
  `tool-plugin` world from the repository-wide `wit/v0` contract
  (`path: "../../wit/v0"`). It uses the WIT `sockets` capability to make an
  HTTPS request to wttr.in.
- `manifest.toml` — install metadata (`capabilities = ["tool"]`,
  `permissions = ["socket_client"]`).
- `tests/smoke.rs` — host-run integration test over the core.

## Build, package, publish

From the repository root:

```bash
cargo test                                   # inside this directory, native
uv run --python 3.13 python tools/build_plugins.py --package --plugin weather
uv run --python 3.13 python tools/publish_plugins.py
```

`build_plugins.py` writes `weather.wasm` here (gitignored), builds
`dist/weather-0.1.0.zip`, and regenerates `registry.json` with this repo's
release URL and the zip's sha256. `publish_plugins.py` uploads the zip to the
`plugins` release and verifies the download.

The plugin only becomes installable after the upload: the registry entry alone
points at a URL that does not exist yet. That URL is a GitHub Release asset,
identical in shape to upstream's:

```
https://github.com/seungjin/hanaclaw-plugins/releases/download/plugins/weather-0.1.0.zip
```

`publish_plugins.py` creates the `plugins` release on first use, then only
adds missing `name-version.zip` assets (never overwrites bytes).

## Install

```bash
# `zeroclaw` here is the host binary you built (or stock zeroclaw).
export ZEROCLAW_PLUGIN_REGISTRY_URL=https://github.com/seungjin/hanaclaw-plugins/releases/download/plugins/registry.json
zeroclaw plugin install weather
```

The model then sees the tool as `weather` with a required `city` argument.

## Changing it

Published `name@version` identities are immutable. After editing source or
`manifest.toml`, bump `version` in **both** `manifest.toml` and `Cargo.toml`,
then rebuild; `build_plugins.py` refuses to reuse the old digest:

```bash
# manifest.toml: version = "0.1.0"   Cargo.toml: version = "0.1.0"
uv run --python 3.13 python tools/build_plugins.py --package --plugin weather
uv run --python 3.13 python tools/publish_plugins.py
```

If `publish_plugins.py` exits 2 with "already exists with different bytes",
the version was not bumped before the rebuild.
