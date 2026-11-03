//! Minimal wasmtime harness for a ZeroClaw `tool-plugin` component.
//!
//! Usage:
//!   cargo run --release -- <path-to.wasm>
//!
//! Pipe JSON arguments on stdin; if none, the tool receives `{}`. Imported host
//! services are stubbed and panic only if the plugin actually calls them.
//!
//! This is a debugging helper, not the supported runtime. For real use,
//! load the plugin into the ZeroClaw host binary.
use std::io::{self, Read};

use anyhow::{Context, Result, bail};
use wasmtime::component::{bindgen, Component, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiView};

bindgen!({
    path: "wit",
    world: "tool-plugin",
});

struct Harness {
    table: ResourceTable,
    wasi: WasiCtx,
}

impl Harness {
    fn new() -> Self {
        Self {
            table: ResourceTable::new(),
            wasi: WasiCtxBuilder::new().inherit_stdio().build(),
        }
    }
}


impl WasiView for Harness {
    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
    fn ctx(&mut self) -> &mut WasiCtx {
        &mut self.wasi
    }
}

impl zeroclaw::plugin::types::Host for Harness {}

impl zeroclaw::plugin::logging::Host for Harness {
    fn log_record(
        &mut self,
        level: zeroclaw::plugin::logging::LogLevel,
        event: zeroclaw::plugin::logging::PluginEvent,
    ) {
        eprintln!("[log] level={level:?} event={event:?}");
    }
}

impl zeroclaw::plugin::secrets::Host for Harness {
    fn get(
        &mut self,
        _name: String,
    ) -> Result<String, zeroclaw::plugin::secrets::SecretError> {
        Err(zeroclaw::plugin::secrets::SecretError::AccessDenied)
    }
}

impl zeroclaw::plugin::state::Host for Harness {
    fn get(
        &mut self,
        _key: String,
    ) -> Result<Option<zeroclaw::plugin::state::StateEntry>, zeroclaw::plugin::state::StateError> {
        Err(zeroclaw::plugin::state::StateError::AccessDenied)
    }

    fn put(
        &mut self,
        _key: String,
        _value: Vec<u8>,
        _expected_revision: Option<u64>,
    ) -> Result<u64, zeroclaw::plugin::state::StateError> {
        Err(zeroclaw::plugin::state::StateError::AccessDenied)
    }

    fn delete(
        &mut self,
        _key: String,
        _expected_revision: u64,
    ) -> Result<(), zeroclaw::plugin::state::StateError> {
        Err(zeroclaw::plugin::state::StateError::AccessDenied)
    }
}

impl zeroclaw::plugin::sockets::HostConnection for Harness {
    fn send(
        &mut self,
        _self_: wasmtime::component::Resource<zeroclaw::plugin::sockets::Connection>,
        _bytes: Vec<u8>,
    ) -> Result<(), zeroclaw::plugin::sockets::SocketError> {
        unimplemented!("sockets are not supported by this harness")
    }

    fn receive(
        &mut self,
        _self_: wasmtime::component::Resource<zeroclaw::plugin::sockets::Connection>,
    ) -> Result<zeroclaw::plugin::sockets::ReceiveEvent, zeroclaw::plugin::sockets::SocketError> {
        unimplemented!("sockets are not supported by this harness")
    }

    fn send_negotiation(
        &mut self,
        _self_: wasmtime::component::Resource<zeroclaw::plugin::sockets::Connection>,
        _bytes: Vec<u8>,
    ) -> Result<(), zeroclaw::plugin::sockets::SocketError> {
        unimplemented!("sockets are not supported by this harness")
    }

    fn receive_negotiation(
        &mut self,
        _self_: wasmtime::component::Resource<zeroclaw::plugin::sockets::Connection>,
    ) -> Result<zeroclaw::plugin::sockets::ReceiveEvent, zeroclaw::plugin::sockets::SocketError> {
        unimplemented!("sockets are not supported by this harness")
    }

    fn pending(
        &mut self,
        _self_: wasmtime::component::Resource<zeroclaw::plugin::sockets::Connection>,
    ) -> Result<u32, zeroclaw::plugin::sockets::SocketError> {
        unimplemented!("sockets are not supported by this harness")
    }

    fn upgrade_tls(
        &mut self,
        _self_: wasmtime::component::Resource<zeroclaw::plugin::sockets::Connection>,
    ) -> Result<(), zeroclaw::plugin::sockets::SocketError> {
        unimplemented!("sockets are not supported by this harness")
    }

    fn drop(
        &mut self,
        _rep: wasmtime::component::Resource<zeroclaw::plugin::sockets::Connection>,
    ) -> wasmtime::Result<()> {
        Ok(())
    }
}

impl zeroclaw::plugin::sockets::Host for Harness {
    fn connect(
        &mut self,
        _request: zeroclaw::plugin::sockets::ConnectRequest,
    ) -> Result<wasmtime::component::Resource<zeroclaw::plugin::sockets::Connection>, zeroclaw::plugin::sockets::SocketError> {
        unimplemented!("sockets are not supported by this harness")
    }

    fn close(
        &mut self,
        _connection: wasmtime::component::Resource<zeroclaw::plugin::sockets::Connection>,
    ) {
        // no-op
    }
}

impl zeroclaw::plugin::websocket::HostConnection for Harness {
    fn send(
        &mut self,
        _self_: wasmtime::component::Resource<zeroclaw::plugin::websocket::Connection>,
        _message: zeroclaw::plugin::websocket::Message,
    ) -> Result<(), zeroclaw::plugin::websocket::WebsocketError> {
        unimplemented!("websocket is not supported by this harness")
    }

    fn receive(
        &mut self,
        _self_: wasmtime::component::Resource<zeroclaw::plugin::websocket::Connection>,
    ) -> Result<Option<zeroclaw::plugin::websocket::Event>, zeroclaw::plugin::websocket::WebsocketError> {
        unimplemented!("websocket is not supported by this harness")
    }

    fn close(
        &mut self,
        _self_: wasmtime::component::Resource<zeroclaw::plugin::websocket::Connection>,
        _frame: Option<zeroclaw::plugin::websocket::CloseFrame>,
    ) -> Result<(), zeroclaw::plugin::websocket::WebsocketError> {
        unimplemented!("websocket is not supported by this harness")
    }

    fn negotiated_subprotocol(
        &mut self,
        _self_: wasmtime::component::Resource<zeroclaw::plugin::websocket::Connection>,
    ) -> Result<Option<String>, zeroclaw::plugin::websocket::WebsocketError> {
        unimplemented!("websocket is not supported by this harness")
    }

    fn drop(
        &mut self,
        _rep: wasmtime::component::Resource<zeroclaw::plugin::websocket::Connection>,
    ) -> wasmtime::Result<()> {
        Ok(())
    }
}

impl zeroclaw::plugin::websocket::Host for Harness {
    fn connect(
        &mut self,
        _options: zeroclaw::plugin::websocket::ConnectOptions,
    ) -> Result<wasmtime::component::Resource<zeroclaw::plugin::websocket::Connection>, zeroclaw::plugin::websocket::WebsocketError> {
        unimplemented!("websocket is not supported by this harness")
    }
}

fn main() -> Result<()> {
    let wasm_path = std::env::args()
        .nth(1)
        .context("usage: wasmtime-harness <path-to.wasm>")?;

    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let args = if input.trim().is_empty() {
        "{}".to_string()
    } else {
        input.trim().to_string()
    };

    let mut config = Config::new();
    config.wasm_component_model(true);
    let engine = Engine::new(&config)?;
    let component = Component::from_file(&engine, &wasm_path)
        .with_context(|| format!("loading wasm component from {wasm_path}"))?;

    let mut linker = Linker::<Harness>::new(&engine);
    let mut options = LinkOptions::default();
    options
        .plugins_wit_v0(true)
        .plugins_wit_v0_sockets(true)
        .plugins_wit_v0_websocket(true);
    ToolPlugin::add_to_linker(&mut linker, &options, |state| state)?;

    let mut store = Store::new(&engine, Harness::new());
    let plugin = ToolPlugin::instantiate(&mut store, &component, &mut linker)?;

    let info = plugin.zeroclaw_plugin_plugin_info();
    let name = info.call_plugin_name(&mut store)?;
    let version = info.call_plugin_version(&mut store)?;

    let tool = plugin.zeroclaw_plugin_tool();
    let tool_name = tool.call_name(&mut store)?;
    let description = tool.call_description(&mut store)?;
    let schema = tool.call_parameters_schema(&mut store)?;
    let result = tool.call_execute(&mut store, &args)?;

    println!("plugin: {name} v{version}");
    println!("tool:   {tool_name} -- {description}");
    println!("schema: {schema}");
    println!("args:   {args}");
    match result {
        Ok(r) => println!("success: {}\noutput: {}\nerror: {:?}", r.success, r.output, r.error),
        Err(e) => bail!("tool execute returned error: {e}"),
    }

    Ok(())
}
