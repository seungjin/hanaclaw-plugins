# libpod

ZeroClaw **tool plugin** that controls Podman via the [libpod REST API](https://docs.podman.io/en/latest/_static/api.html).

Written with AI/LLM assistance; see the
repository's [NOTICE](../../NOTICE). Licensed MIT OR Apache-2.0; the crate
declares `license = "MIT OR Apache-2.0"` and ships no license files of its own
(they live at the repository root).

The model calls the `libpod` tool with an `operation` argument. The plugin
builds the matching HTTP request, sends it to the configured Podman API over
TCP, and returns a concise result that clearly says it came from the **libpod
plugin** and includes the source URL.

**This plugin talks HTTP to a TCP libpod API endpoint.** It does **not** use Unix
sockets. Start Podman with a TCP listener, for example:

```bash
podman system service --time=0 tcp://127.0.0.1:8888
```

## Operations

| Operation | Required fields | Description |
|---|---|---|
| `list_containers` | — | List containers. Set `all=true` to include stopped ones. |
| `list_images` | — | List images. Set `all=true` to include intermediate layers. |
| `container_info` | `name` | Inspect a container by name or ID. |
| `container_logs` | `name` | Fetch stdout/stderr logs for a container. |
| `run_container` | `image` | Create a container. Optional `name`, `command`, `env`, `ports`, `mounts`, and `options.create`. |
| `start_container` | `name` | Start an existing container. |
| `stop_container` | `name` | Stop an existing container. `options.timeout` and `options.ignore` are supported. |
| `remove_container` | `name` | Remove a container. `options.force` and `options.volumes` are supported. |
| `exec_in_container` | `name`, `command` | Create an exec session in a running container. |
| `pull_image` | `image` | Pull an image from a registry. |

## How it is shaped

- `src/core.rs` — pure logic (`ToolInput`, request building, response
  formatting); `cargo test` runs it on the host with no wasm toolchain.
- `src/lib.rs` — `#[cfg(target_family = "wasm")]` shim that exports the
  `tool-plugin` world from the repository-wide `wit/v0` contract
  (`path: "../../wit/v0"`). It uses the WIT `sockets` capability to make HTTP
  requests to the Podman API.
- `manifest.toml` — install metadata (`capabilities = ["tool"]`,
  `permissions = ["socket_client", "config_read"]` plus `[config_schema]`).
- `tests/smoke.rs` — host-run integration test over the core.

## Build, package, publish

From the repository root:

```bash
cargo test                                   # inside this directory, native
uv run --python 3.13 python tools/build_plugins.py --package --plugin libpod
uv run --python 3.13 python tools/publish_plugins.py
```

`build_plugins.py` writes `libpod.wasm` here (gitignored), builds
`dist/libpod-0.1.0.zip`, and regenerates `registry.json` with this repo's
release URL and the zip's sha256. `publish_plugins.py` uploads the zip to the
`plugins` release and verifies the download.

The plugin only becomes installable after the upload: the registry entry alone
points at a URL that does not exist yet. That URL is a GitHub Release asset,
identical in shape to upstream's:

```
https://github.com/seungjin/hanaclaw-plugins/releases/download/plugins/libpod-0.1.0.zip
```

## Install

```bash
# `zeroclaw` here is the host binary you built (or stock zeroclaw).
export ZEROCLAW_PLUGIN_REGISTRY_URL=https://raw.githubusercontent.com/seungjin/hanaclaw-plugins/main/registry.json
zeroclaw plugin install libpod
```

The model then sees the tool as `libpod` with a required `operation` argument.

## Configuration

Use the host's plugin config mechanism to set the Podman API URL and version:

```json
{
  "api_url": "http://127.0.0.1:8888",
  "api_version": "v5.0"
}
```

Defaults are `http://127.0.0.1:8888` and `v5.0`.

## Changing it

Published `name@version` identities are immutable. After editing source or
`manifest.toml`, bump `version` in **both** `manifest.toml` and `Cargo.toml`,
then rebuild; `build_plugins.py` refuses to reuse the old digest.
