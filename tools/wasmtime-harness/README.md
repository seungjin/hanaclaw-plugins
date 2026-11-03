# wasmtime-harness

A minimal standalone [wasmtime](https://github.com/bytecodealliance/wasmtime)
runner for ZeroClaw `tool-plugin` WASM components.

It instantiates the component, calls the metadata exports
(`plugin-name`, `plugin-version`, `name`, `description`, `parameters-schema`),
then calls `execute` with JSON arguments from stdin. All host imports
(logging, config, state, secrets, sockets, websocket, WASI) are stubbed or
implemented minimally.

## Build

```bash
cd tools/wasmtime-harness
cargo build --release
```

## Run

```bash
cargo run --release -- /path/to/plugin.wasm < args.json
```

Example:

```bash
cd tools/wasmtime-harness
cargo run --release -- \
  /path/to/word-count-0.1.0.wasm \
  <<<'{"text":"hello world"}'
```

If no JSON is supplied on stdin, the harness passes `{}`.

## Caveats

* This is a debugging/development helper, **not** the supported runtime.
  For real use, load the plugin into the ZeroClaw host binary.
* The harness links a full WASI Preview 2 implementation. It is pinned to
  the same `wasmtime`/`wasmtime-wasi` 48.x line used by the ZeroClaw host,
  so plugins built against `wit/v0` should link without resource mismatches.
* Sockets are implemented with real TCP/TLS (via `rustls`) for HTTP-style
  requests. WebSocket imports still panic if the plugin calls them.
* Logging records are printed to stderr.
