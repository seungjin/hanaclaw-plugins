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
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use rustls::{ClientConfig, ClientConnection, StreamOwned};
use wasmtime::component::{Component, Linker, ResourceTable, bindgen};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

bindgen!({
    path: "../../wit/v0",
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
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
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
    fn get(&mut self, _name: String) -> Result<String, zeroclaw::plugin::secrets::SecretError> {
        Err(zeroclaw::plugin::secrets::SecretError::AccessDenied)
    }
}

impl zeroclaw::plugin::state::Host for Harness {
    fn get(
        &mut self,
        _key: String,
    ) -> Result<Option<zeroclaw::plugin::state::StateEntry>, zeroclaw::plugin::state::StateError>
    {
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

fn make_tls_config() -> anyhow::Result<ClientConfig> {
    let mut roots = rustls::RootCertStore::empty();
    let certs = rustls_native_certs::load_native_certs();
    for cert in certs.certs {
        roots.add(cert).ok();
    }
    Ok(ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth())
}

enum SocketConn {
    Plain(TcpStream),
    Tls(StreamOwned<ClientConnection, TcpStream>),
}

impl SocketConn {
    fn stream_mut(&mut self) -> &mut dyn ReadWrite {
        match self {
            SocketConn::Plain(s) => s,
            SocketConn::Tls(s) => s,
        }
    }
}

trait ReadWrite: Read + Write {}
impl<T: Read + Write> ReadWrite for T {}

fn as_socket_res(
    res: wasmtime::component::Resource<zeroclaw::plugin::sockets::Connection>,
) -> wasmtime::component::Resource<SocketConn> {
    wasmtime::component::Resource::new_own(res.rep())
}

fn from_socket_res(
    res: wasmtime::component::Resource<SocketConn>,
) -> wasmtime::component::Resource<zeroclaw::plugin::sockets::Connection> {
    wasmtime::component::Resource::new_own(res.rep())
}

impl zeroclaw::plugin::sockets::HostConnection for Harness {
    fn send(
        &mut self,
        self_: wasmtime::component::Resource<zeroclaw::plugin::sockets::Connection>,
        bytes: Vec<u8>,
    ) -> Result<(), zeroclaw::plugin::sockets::SocketError> {
        let typed = as_socket_res(self_);
        let conn = self
            .table
            .get_mut(&typed)
            .map_err(|_| zeroclaw::plugin::sockets::SocketError::InvalidState)?;
        if std::env::var("WASMTIME_HARNESS_SOCKET_DEBUG").is_ok() {
            eprintln!("[socket send {} bytes] {}", bytes.len(), String::from_utf8_lossy(&bytes));
        }
        conn.stream_mut()
            .write_all(&bytes)
            .map_err(|_| zeroclaw::plugin::sockets::SocketError::ConnectFailed)?;
        Ok(())
    }

    fn receive(
        &mut self,
        self_: wasmtime::component::Resource<zeroclaw::plugin::sockets::Connection>,
    ) -> Result<zeroclaw::plugin::sockets::ReceiveEvent, zeroclaw::plugin::sockets::SocketError>
    {
        let typed = as_socket_res(self_);
        let conn = self
            .table
            .get_mut(&typed)
            .map_err(|_| zeroclaw::plugin::sockets::SocketError::InvalidState)?;
        let mut buf = [0u8; 8192];
        match conn.stream_mut().read(&mut buf) {
            Ok(0) => Ok(zeroclaw::plugin::sockets::ReceiveEvent::Closed(
                zeroclaw::plugin::sockets::CloseReason::PeerClosed,
            )),
            Ok(n) => {
                if std::env::var("WASMTIME_HARNESS_SOCKET_DEBUG").is_ok() {
                    eprintln!("[socket recv {} bytes] {}", n, String::from_utf8_lossy(&buf[..n.min(200)]));
                }
                Ok(zeroclaw::plugin::sockets::ReceiveEvent::Data(
                    buf[..n].to_vec(),
                ))
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                Ok(zeroclaw::plugin::sockets::ReceiveEvent::Idle)
            }
            Err(_) => Err(zeroclaw::plugin::sockets::SocketError::ConnectFailed),
        }
    }

    fn send_negotiation(
        &mut self,
        _self_: wasmtime::component::Resource<zeroclaw::plugin::sockets::Connection>,
        _bytes: Vec<u8>,
    ) -> Result<(), zeroclaw::plugin::sockets::SocketError> {
        Err(zeroclaw::plugin::sockets::SocketError::TlsConfigurationFailed)
    }

    fn receive_negotiation(
        &mut self,
        _self_: wasmtime::component::Resource<zeroclaw::plugin::sockets::Connection>,
    ) -> Result<zeroclaw::plugin::sockets::ReceiveEvent, zeroclaw::plugin::sockets::SocketError>
    {
        Err(zeroclaw::plugin::sockets::SocketError::TlsConfigurationFailed)
    }

    fn pending(
        &mut self,
        _self_: wasmtime::component::Resource<zeroclaw::plugin::sockets::Connection>,
    ) -> Result<u32, zeroclaw::plugin::sockets::SocketError> {
        Ok(0)
    }

    fn upgrade_tls(
        &mut self,
        self_: wasmtime::component::Resource<zeroclaw::plugin::sockets::Connection>,
    ) -> Result<(), zeroclaw::plugin::sockets::SocketError> {
        let typed = as_socket_res(self_);
        let plain = self
            .table
            .delete(typed)
            .map_err(|_| zeroclaw::plugin::sockets::SocketError::InvalidState)?;
        let stream = match plain {
            SocketConn::Plain(s) => s,
            SocketConn::Tls(_) => return Err(zeroclaw::plugin::sockets::SocketError::InvalidState),
        };
        let config = make_tls_config()
            .map_err(|_| zeroclaw::plugin::sockets::SocketError::TlsConfigurationFailed)?;
        let domain = stream.peer_addr().ok().and_then(|addr| match addr {
            SocketAddr::V4(_) => None,
            SocketAddr::V6(_) => None,
        });
        let domain = domain.unwrap_or("localhost".to_string());
        let server_name = domain
            .try_into()
            .map_err(|_| zeroclaw::plugin::sockets::SocketError::TlsConfigurationFailed)?;
        let conn = ClientConnection::new(std::sync::Arc::new(config), server_name)
            .map_err(|_| zeroclaw::plugin::sockets::SocketError::TlsHandshakeFailed)?;
        let tls = StreamOwned::new(conn, stream);
        let _ = self.table.push(SocketConn::Tls(tls));
        Ok(())
    }

    fn drop(
        &mut self,
        rep: wasmtime::component::Resource<zeroclaw::plugin::sockets::Connection>,
    ) -> wasmtime::Result<()> {
        let _ = self.table.delete(as_socket_res(rep));
        Ok(())
    }
}

impl zeroclaw::plugin::sockets::Host for Harness {
    fn connect(
        &mut self,
        request: zeroclaw::plugin::sockets::ConnectRequest,
    ) -> Result<
        wasmtime::component::Resource<zeroclaw::plugin::sockets::Connection>,
        zeroclaw::plugin::sockets::SocketError,
    > {
        let port = match request.mode {
            zeroclaw::plugin::sockets::ConnectMode::Plaintext => request.port,
            zeroclaw::plugin::sockets::ConnectMode::DirectTls => request.port,
            zeroclaw::plugin::sockets::ConnectMode::StartTls => request.port,
        };
        let addrs = (request.host.as_str(), port)
            .to_socket_addrs()
            .map_err(|_| zeroclaw::plugin::sockets::SocketError::ResolutionFailed)?;
        let addr = addrs
            .into_iter()
            .next()
            .ok_or(zeroclaw::plugin::sockets::SocketError::ResolutionFailed)?;
        let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(10))
            .map_err(|_| zeroclaw::plugin::sockets::SocketError::ConnectFailed)?;
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .map_err(|_| zeroclaw::plugin::sockets::SocketError::ConnectFailed)?;
        stream
            .set_write_timeout(Some(Duration::from_secs(30)))
            .map_err(|_| zeroclaw::plugin::sockets::SocketError::ConnectFailed)?;

        let conn = match request.mode {
            zeroclaw::plugin::sockets::ConnectMode::Plaintext => SocketConn::Plain(stream),
            zeroclaw::plugin::sockets::ConnectMode::DirectTls => {
                let config = make_tls_config()
                    .map_err(|_| zeroclaw::plugin::sockets::SocketError::TlsConfigurationFailed)?;
                let server_name = request
                    .host
                    .try_into()
                    .map_err(|_| zeroclaw::plugin::sockets::SocketError::TlsConfigurationFailed)?;
                let conn = ClientConnection::new(std::sync::Arc::new(config), server_name)
                    .map_err(|_| zeroclaw::plugin::sockets::SocketError::TlsHandshakeFailed)?;
                let tls = StreamOwned::new(conn, stream);
                SocketConn::Tls(tls)
            }
            zeroclaw::plugin::sockets::ConnectMode::StartTls => SocketConn::Plain(stream),
        };

        let handle = self
            .table
            .push(conn)
            .map_err(|_| zeroclaw::plugin::sockets::SocketError::ConnectionLimit)?;
        Ok(from_socket_res(handle))
    }

    fn close(
        &mut self,
        connection: wasmtime::component::Resource<zeroclaw::plugin::sockets::Connection>,
    ) {
        let _ = self.table.delete(as_socket_res(connection));
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
    ) -> Result<
        Option<zeroclaw::plugin::websocket::Event>,
        zeroclaw::plugin::websocket::WebsocketError,
    > {
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
    ) -> Result<
        wasmtime::component::Resource<zeroclaw::plugin::websocket::Connection>,
        zeroclaw::plugin::websocket::WebsocketError,
    > {
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
        .map_err(|e| anyhow::anyhow!("loading wasm component from {wasm_path}: {e}"))?;

    let mut linker = Linker::<Harness>::new(&engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;
    let mut options = LinkOptions::default();
    options
        .plugins_wit_v0(true)
        .plugins_wit_v0_sockets(true)
        .plugins_wit_v0_websocket(true);
    ToolPlugin::add_to_linker::<Harness, wasmtime::component::HasSelf<Harness>>(
        &mut linker,
        &options,
        |state: &mut Harness| state,
    )?;

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
        Ok(r) => println!(
            "success: {}\noutput: {}\nerror: {:?}",
            r.success, r.output, r.error
        ),
        Err(e) => bail!("tool execute returned error: {e}"),
    }

    Ok(())
}
