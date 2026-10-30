# gh2qdrant

ZeroClaw **tool plugin** that ingests a GitHub repository into a Qdrant
collection. It mirrors the native [`gh2qdrant`](https://github.com/seungjin/gh2qdrant)
tool but runs as a WIT component inside the ZeroClaw wasm sandbox.

This first version validates inputs and returns a structured plan. The full
clone → chunk → embed → upsert pipeline requires host-mediated sockets; the
plugin already declares the `socket_client` permission for that future work.

## How it is shaped

- `src/core.rs` — pure logic (`run`, `ToolInput`, `ToolConfig`); `cargo test`
  runs it on the host with no wasm toolchain.
- `src/lib.rs` — `#[cfg(target_family = "wasm")]` shim that exports the
  `tool-plugin` world from the repository-wide `wit/v0` contract
  (`path: "../../wit/v0"`).
- `manifest.toml` — install metadata (`capabilities = ["tool"]`,
  `permissions = ["config_read", "socket_client"]`, `config_schema` for
  Qdrant/Ollama URLs).
- `tests/smoke.rs` — host-run integration tests over the core.

## Build, package, publish

From the repository root:

```bash
cargo test                                   # inside this directory, native
uv run --python 3.13 python tools/build_plugins.py --package --plugin gh2qdrant
uv run --python 3.13 python tools/publish_plugins.py
```

`build_plugins.py` writes `gh2qdrant.wasm` here (gitignored), builds
`dist/gh2qdrant-0.1.0.zip`, and regenerates `registry.json` with this repo's
release URL and the zip's sha256. `publish_plugins.py` uploads the zip to the
`plugins` release and verifies the download.

## Install

```bash
export ZEROCLAW_PLUGIN_REGISTRY_URL=https://github.com/seungjin/hanaclaw-plugins/releases/download/plugins/registry.json
zeroclaw plugin install gh2qdrant
```

## Tool schema

The model calls `gh2qdrant` with JSON arguments like:

```json
{
  "url": "https://github.com/uuid-rs/uuid",
  "collection": "code_uuid",
  "dry": true
}
```

Operator config (via `config_read`) supplies Qdrant/Ollama defaults:

- `qdrant_url` — default `http://localhost:6333`
- `qdrant_auth_header` — default `api-key`
- `ollama_url` — default `http://localhost:11434`
- `embed_model` — default `bge-m3`
- `batch` — default `8`
- `work_dir` — default `/tmp/gh2qdrant`

## Changing it

Published `name@version` identities are immutable. After editing source or
`manifest.toml`, bump `version` in **both** `manifest.toml` and `Cargo.toml`,
then rebuild; `build_plugins.py` refuses to reuse the old digest.
