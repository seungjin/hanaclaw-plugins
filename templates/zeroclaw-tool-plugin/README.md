# zeroclaw-tool-plugin (cargo generate template)

Scaffolds a ZeroClaw **WIT tool plugin** in the shape the registry requires:
pure core, thin wasm shim, host-run tests, and a manifest. It mirrors the
layout of `zeroclaw-labs/zeroclaw-plugins/plugins/redact-text` (the canonical
reference plugin), on edition 2024 with `wit-bindgen` 0.46.

Generated crates carry `license = "MIT OR Apache-2.0"`, matching this repo;
the template does not copy `LICENSE-MIT`/`LICENSE-APACHE` into the new crate,
so copy them from the repository root if you publish the crate on its own.

## Usage

From the repository root:

```bash
rustup target add wasm32-wasip2   # once

cargo generate --path templates/zeroclaw-tool-plugin --destination plugins \
  --name word-count \
  --define tool_name=word_count \
  --define tool_description="Count words in text" \
  --define author=seungjin \
  --define registry=true \
  --define config_read=false
```

`cargo generate` reads the template from the git index, so stage template
changes (`git add templates/`) before generating.

Then edit `plugins/word-count/src/core.rs` (and the argument struct in
`src/lib.rs`) and build. One command formats, tests, lints, builds, packages,
and refreshes `registry.json`:

```bash
uv run --python 3.13 python tools/build_plugins.py --package --plugin word-count
uv run --python 3.13 python tools/publish_plugins.py     # upload + verify
```

The template vendors **no** WIT: the crate binds the repository-wide contract
with `path: "../../wit/v0"` (the layout upstream uses), so refreshing
`wit/v0` once with `tools/fetch_zeroclaw_wit.py` updates every plugin.

Omitted `--define` values are prompted for. `--name` is the project name; it
must be kebab-case (the host rejects other plugin names) and becomes the crate
name, the `manifest.toml` `name`, and (snake_cased) the `wasm_path`.
`--destination plugins` is a path, not a template placeholder.

## What it generates

```
Cargo.toml           # cdylib + rlib, edition 2024, standalone [workspace],
                     # release profile (opt-level "s", lto, strip)
manifest.toml        # name, version, wasm_path, capabilities, permissions
src/core.rs          # pure logic (host-testable, no wit-bindgen)
src/lib.rs           # #[cfg(target_family = "wasm")] component shim
tests/smoke.rs       # host-run integration test
.gitignore           # /target, *.wasm (build outputs are never committed)
```

No `Cargo.lock` is generated: the first build creates it and every later
cargo invocation is `--locked`. Commit it as soon as it exists.

There is deliberately **no** `.cargo/config.toml`: a default build target
would make `cargo test` try to execute a wasm module. Pass
`--target wasm32-wasip2` when building the component, exactly as upstream CI
does.

## Options

| Placeholder | Default | Effect |
|---|---|---|
| `--name` | prompted | kebab-case plugin name |
| `tool_name` | prompted | snake_case tool the model calls; no `__` |
| `tool_description` | prompted | model-facing description |
| `author` | `you` | `manifest.toml` author |
| `registry` | `false` (prompt default) | `true` = publishable; `false` writes `registry = false`, so the build and the registry skip it entirely |
| `config_read` | `false` | adds the `config_read` permission **and** `[config_schema]`, plus the `__config` plumbing in `ExecuteArgs`/`ToolInput::with_config` |

Pass `registry=true` explicitly (as in the usage above) for a plugin you
intend to publish; answer `false` while the plugin is work in progress, or
delete the `registry = false` line later.

### Config

`config_read` and `[config_schema]` are a biconditional: the host rejects a
manifest carrying one without the other. When `config_read=true` the template
wires the whole path: the host injects the validated config object into the
call arguments as `__config`, and `ToolInput::with_config` deserializes it into
`ToolConfig` (fields must all default, because a withheld grant injects `{}`).

## Registry

`registry.json` at the repo root is generated — never hand-edit it.
`tools/build_plugins.py` rebuilds it from every publishable
`plugins/<name>/` (a `manifest.toml` without `registry = false`) plus the
upstream catalog, with this repo's release URL and the package zip's sha256:

```bash
uv run --python 3.13 python tools/build_plugins.py --check   # exit 2 if stale
```

`tools/update_zeroclaw_registry.py` refreshes the upstream half without
building anything; `--offline` reuses the entries already in
`registry.json`.

Published `name@version` identities are immutable. Change the code, bump
`version` in **both** `manifest.toml` and `Cargo.toml`, then rebuild; the build
refuses to reuse the old digest. New entries appear in `registry.json`
immediately, but they are only installable after
`tools/publish_plugins.py` has uploaded the zip to the `plugins` release.

For local iteration, `build_plugins.py` accepts `--skip-tests`,
`--no-clippy`, `--skip-fmt-check`, and `--skip-metadata`; CI runs all of the
checks, so do not merge with them skipped.

## Install

Once published, install the plugin with the ZeroClaw host binary you built:

```bash
zeroclaw plugin install word-count \
  --registry https://raw.githubusercontent.com/seungjin/hanaclaw-plugins/main/registry.json
# or, once the release asset exists:
zeroclaw plugin install word-count \
  --registry https://github.com/seungjin/hanaclaw-plugins/releases/download/plugins/registry.json
```

Or set the catalog once for every install:

```bash
export ZEROCLAW_PLUGIN_REGISTRY_URL=https://raw.githubusercontent.com/seungjin/hanaclaw-plugins/main/registry.json
# or, after the first publish:
export ZEROCLAW_PLUGIN_REGISTRY_URL=https://github.com/seungjin/hanaclaw-plugins/releases/download/plugins/registry.json
zeroclaw plugin install word-count
```
