# zeroclaw-tool-plugin (cargo generate template)

Scaffolds a ZeroClaw **WIT tool plugin**: pure core, thin wasm shim,
host-run tests, and a manifest. It mirrors the layout of
`zeroclaw-labs/zeroclaw-plugins/plugins/redact-text` (the canonical upstream
reference plugin), on edition 2024 with `wit-bindgen` 0.46.

This template is part of [hanaclaw-plugins](https://github.com/seungjin/hanaclaw-plugins),
a personal extension of
[zeroclaw-labs/zeroclaw-plugins](https://github.com/zeroclaw-labs/zeroclaw-plugins).
It is also usable on its own for independent plugin projects.

## Usage

### Generate the project

Prerequisite (once):

```bash
rustup target add wasm32-wasip2
```

Generate from this GitHub repository:

```bash
cargo generate --git https://github.com/seungjin/hanaclaw-plugins \
  templates/zeroclaw-tool-plugin \
  --name word-count \
  --define tool_name=word_count \
  --define tool_description="Count words in text" \
  --define author=seungjin \
  --define registry=true \
  --define config_read=false
```

Omitted `--define` values are prompted for. `--name` is the project name; it
must be **kebab-case** (the host rejects other plugin names) and becomes the
crate name, the `manifest.toml` `name`, and (snake_cased) the `wasm_path`.

### Edit and build

After generation, edit:

- `src/core.rs` -- implement the tool logic.
- `src/lib.rs` -- adjust the WIT-bound argument structs if you changed the
  tool signature.
- `manifest.toml` -- update `description`, `version`, or capabilities if
  needed.

Run host tests without any wasm toolchain:

```bash
cargo test
```

Build the wasm component:

```bash
cargo build --target wasm32-wasip2 --release
```

There is deliberately **no** `.cargo/config.toml`: a default build target
would make `cargo test` try to execute a wasm module. Always pass
`--target wasm32-wasip2` when building the component, exactly as upstream CI
does.

### Package and publish

The generated crate expects a ZeroClaw WIT contract at `wit/v0` relative to the
project root. For standalone use, vendor or link your own copy of the contract.
The upstream contract lives at
`https://github.com/zeroclaw-labs/zeroclaw-plugins/tree/main/wit/v0`.

A ZeroClaw plugin package is a reproducible zip containing:

```
<name>/manifest.toml
<name>/<wasm_path>
```

Build the zip however your project packages components, publish it to a URL
your ZeroClaw host can fetch, and add the entry to your registry with the
matching `sha256` of the zip.

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
cargo invocation should be `--locked`. Commit it as soon as it exists.

## Options

| Placeholder | Default | Effect |
|---|---|---|
| `--name` | prompted | kebab-case plugin name |
| `tool_name` | prompted | snake_case tool the model calls; no `__` |
| `tool_description` | prompted | model-facing description |
| `author` | `you` | `manifest.toml` author |
| `registry` | `false` (prompt default) | `true` = publishable; `false` writes `registry = false`, so the registry skips it entirely |
| `config_read` | `false` | adds the `config_read` permission **and** `[config_schema]`, plus the `__config` plumbing in `ExecuteArgs`/`ToolInput::with_config` |

Pass `registry=true` explicitly for a plugin you intend to publish; answer
`false` while the plugin is work in progress, or delete the `registry = false`
line later.

### Config

`config_read` and `[config_schema]` are a biconditional: the host rejects a
manifest carrying one without the other. When `config_read=true` the template
wires the whole path: the host injects the validated config object into the
call arguments as `__config`, and `ToolInput::with_config` deserializes it into
`ToolConfig` (fields must all default, because a withheld grant injects `{}`).

## Publishing checklist

- Bump `version` in **both** `manifest.toml` and `Cargo.toml` for any change
  that alters the wasm or the registry metadata (description, author,
  capabilities, provides, sender_match).
- Rebuild the wasm with `--target wasm32-wasip2 --release`.
- Produce a zip with `<name>/manifest.toml` and `<name>/<wasm_path>` at the
  top level.
- Compute the zip's sha256 and record it, along with the release asset URL, in
  your registry.
- Published `name@version` identities are immutable; bump the version again
  rather than overwrite an existing release asset.

## Install

Once published, install the plugin with the ZeroClaw host binary you built:

```bash
export ZEROCLAW_PLUGIN_REGISTRY_URL=https://example.com/path/to/your/registry.json
zeroclaw plugin install word-count
```

Or pass the registry explicitly per command:

```bash
zeroclaw plugin install word-count --registry https://example.com/path/to/your/registry.json
```

Replace the registry URL with wherever you host your own `registry.json`.

---

Written with AI/LLM assistance; see the repository [NOTICE](../../NOTICE).
Licensed MIT OR Apache-2.0; the crate declares `license = "MIT OR Apache-2.0"`
and ships no license files of its own (copy them from the repository root if
you publish the crate on its own).
