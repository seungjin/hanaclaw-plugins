//! A ZeroClaw WIT tool plugin: `libpod`.
//!
//! Control Podman containers and images via the libpod REST API.
//!
//! The pure logic lives in [`core`] with no wasm dependency, so `cargo test`
//! runs on the host; the wasm component reuses it through the thin shim below.
//!
//! Test:   cargo test                        (host; no wasm toolchain needed)
//! Build:  uv run --python 3.13 python tools/build_plugins.py   (repo root)

// `core` is shadowed by Rust's core crate, so refer to it as `crate::core`.
#[allow(clippy::module_inception)]
pub mod core;
pub use crate::core as plugin_core;

#[cfg(target_family = "wasm")]
mod component {
    // `features` must list the unstable WIT features this world uses; without
    // `plugins-wit-v0` every item in the package is filtered out.
    wit_bindgen::generate!({
        // Relative to this crate: the WIT contract vendored at the repository
        // root (`plugins/<name>/` -> `wit/v0`). Keep that layout, or adjust
        // this path if the crate lives elsewhere.
        path: "../../wit/v0",
        world: "tool-plugin",
        features: ["plugins-wit-v0", "plugins-wit-v0-sockets"],
    });

    use crate::plugin_core::{ToolInput, run as run_core};
    use exports::zeroclaw::plugin::plugin_info::Guest as PluginInfo;
    use exports::zeroclaw::plugin::tool::{Guest as Tool, ToolResult};
    use zeroclaw::plugin::logging::{
        LogLevel, PluginAction, PluginEvent, PluginOutcome, log_record,
    };
    use zeroclaw::plugin::sockets::{ConnectMode, connect};

    struct LibpodPlugin;

    /// The host merges this plugin's validated config object into the call
    /// arguments under `__config`, after deleting any model-supplied value of
    /// that name. Captured as a raw `Value` so a config-shaped problem is
    /// reported as a config error, not as "invalid arguments".
    #[derive(serde::Deserialize)]
    struct ExecuteArgs {
        #[serde(default)]
        #[serde(rename = "__config")]
        __config: serde_json::Value,
        /// Operation to perform.
        operation: String,
        /// Container or image name/ID.
        #[serde(default)]
        name: Option<String>,
        /// Image reference for run_container / pull_image.
        #[serde(default)]
        image: Option<String>,
        /// Command and args for run_container or exec_in_container.
        #[serde(default)]
        command: Vec<String>,
        /// Environment variables as "KEY=value" strings for run_container.
        #[serde(default)]
        env: Vec<String>,
        /// Port mappings in "host:container" form for run_container.
        #[serde(default)]
        ports: Vec<String>,
        /// Bind mounts in "host:container" form for run_container.
        #[serde(default)]
        mounts: Vec<String>,
        /// If true, list/show all containers/images rather than only
        /// running/top-level.
        #[serde(default)]
        all: bool,
        /// Container create/start/stop/remove options passed straight to the
        /// API.
        #[serde(default)]
        options: serde_json::Value,
    }

    impl From<ExecuteArgs> for ToolInput {
        fn from(args: ExecuteArgs) -> Self {
            ToolInput {
                operation: args.operation,
                name: args.name,
                image: args.image,
                command: args.command,
                env: args.env,
                ports: args.ports,
                mounts: args.mounts,
                all: args.all,
                options: args.options,
            }
        }
    }

    impl PluginInfo for LibpodPlugin {
        fn plugin_name() -> String {
            // Fixed at generation time; the host does not inject it.
            "libpod".to_string()
        }

        fn plugin_version() -> String {
            env!("CARGO_PKG_VERSION").to_string()
        }
    }

    impl Tool for LibpodPlugin {
        fn name() -> String {
            "libpod".to_string()
        }

        fn description() -> String {
            // Fixed at generation time, so a static string is all the host sees.
            "Control Podman containers and images via the libpod REST API".to_string()
        }

        // The model's entire view of the tool: valid JSON or the host refuses
        // to register the plugin. Never declare `__config` here.
        fn parameters_schema() -> String {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "operation": {
                        "type": "string",
                        "description": "Operation to perform: list_containers, list_images, container_info, container_logs, run_container, start_container, stop_container, remove_container, exec_in_container, pull_image."
                    },
                    "name": {
                        "type": "string",
                        "description": "Container or image name/ID, depending on operation."
                    },
                    "image": {
                        "type": "string",
                        "description": "Image reference for run_container or pull_image, e.g. docker.io/library/alpine:latest."
                    },
                    "command": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Command and args for run_container or exec_in_container."
                    },
                    "env": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Environment variables as KEY=value strings for run_container."
                    },
                    "ports": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Port mappings in host:container form for run_container, e.g. 8080:80."
                    },
                    "mounts": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Bind mounts in host:container form for run_container, e.g. /tmp:/data."
                    },
                    "all": {
                        "type": "boolean",
                        "description": "If true, include all containers/images rather than only running/top-level."
                    },
                    "options": {
                        "type": "object",
                        "description": "Extra operation-specific options passed straight to the libpod API."
                    }
                },
                "required": ["operation"]
            })
            .to_string()
        }

        fn execute(args: String) -> Result<ToolResult, String> {
            let parsed: ExecuteArgs = match serde_json::from_str(&args) {
                Ok(a) => a,
                Err(e) => {
                    // Bad input is a normal, recoverable tool result: the model
                    // sees it and can retry. Reserve Err(..) for real faults.
                    return Ok(ToolResult {
                        success: false,
                        output: String::new(),
                        error: Some(format!("invalid arguments: {e}")),
                    });
                }
            };

            let config = parsed.__config.clone();
            let mut input: ToolInput = parsed.into();
            input = input.with_config(&config)?;

            let output = match fetch_and_run(&input) {
                Ok(text) => text,
                Err(e) => {
                    return Ok(ToolResult {
                        success: false,
                        output: String::new(),
                        error: Some(e),
                    });
                }
            };

            log_record(
                LogLevel::Info,
                &PluginEvent {
                    function_name: "libpod::tool::execute".to_string(),
                    action: PluginAction::Complete,
                    outcome: Some(PluginOutcome::Success),
                    duration_ms: None,
                    attrs: None,
                    message: "completed".to_string(),
                },
            );

            Ok(ToolResult {
                success: true,
                output,
                error: None,
            })
        }
    }

    /// Build the libpod API request, send it over the WIT `sockets` interface,
    /// and pass the HTTP response back to the pure core for formatting.
    fn fetch_and_run(input: &ToolInput) -> Result<String, String> {
        let (request, _summary) = crate::plugin_core::build_request(input)?;
        let host_port = parse_url(&input.api_url())?;

        let connection = connect(&zeroclaw::plugin::sockets::ConnectRequest {
            host: host_port.host.clone(),
            port: host_port.port,
            mode: ConnectMode::Plaintext,
            tls_profile: None,
        })
        .map_err(|e| format!("failed to connect to libpod API: {e:?}"))?;

        let http_request = build_http_request(&request, &host_port.host, host_port.port);
        connection
            .send(http_request.as_bytes())
            .map_err(|e| format!("failed to send request: {e:?}"))?;

        let mut response = Vec::new();
        loop {
            match connection.receive() {
                Ok(zeroclaw::plugin::sockets::ReceiveEvent::Data(chunk)) => {
                    response.extend_from_slice(&chunk);
                }
                Ok(zeroclaw::plugin::sockets::ReceiveEvent::Idle) => {
                    if response.is_empty() {
                        std::thread::sleep(std::time::Duration::from_millis(50));
                        continue;
                    }
                    break;
                }
                Ok(zeroclaw::plugin::sockets::ReceiveEvent::Closed(_)) => {
                    break;
                }
                Err(e) => return Err(format!("failed to receive response: {e:?}")),
            }
        }

        let text = String::from_utf8_lossy(&response);
        let (status_code, body) = parse_http_response(&text)?;
        Ok(run_core(input, status_code, body))
    }

    struct HostPort {
        host: String,
        port: u16,
    }

    /// Parse a URL like "http://127.0.0.1:8888" into host and port.
    fn parse_url(url: &str) -> Result<HostPort, String> {
        let url = url.trim();
        let rest = url
            .strip_prefix("http://")
            .or_else(|| url.strip_prefix("https://"))
            .unwrap_or(url);
        let (host, port_str) = rest
            .rsplit_once(':')
            .ok_or("libpod API URL must include a port, e.g. http://127.0.0.1:8888")?;
        let port: u16 = port_str
            .parse()
            .map_err(|e| format!("invalid port in libpod API URL: {e}"))?;
        Ok(HostPort {
            host: host.to_string(),
            port,
        })
    }

    /// Serialize an `ApiRequest` into an HTTP/1.1 request string.
    fn build_http_request(
        request: &crate::plugin_core::ApiRequest,
        host: &str,
        port: u16,
    ) -> String {
        let path_and_query = if request.query.is_empty() {
            request.path.clone()
        } else {
            let qs: Vec<String> = request
                .query
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            format!("{}?{}", request.path, qs.join("&"))
        };

        let mut lines = vec![
            format!("{} {} HTTP/1.1", request.method, path_and_query),
            format!("Host: {host}:{port}"),
            "User-Agent: hanaclaw-libpod-plugin/0.1.0".to_string(),
            "Accept: application/json".to_string(),
            "Connection: close".to_string(),
        ];
        for (k, v) in &request.headers {
            if k.eq_ignore_ascii_case("Accept") {
                continue;
            }
            lines.push(format!("{k}: {v}"));
        }
        if !request.body.is_empty()
            && !request
                .headers
                .iter()
                .any(|(k, _)| k.eq_ignore_ascii_case("Content-Length"))
        {
            lines.push(format!("Content-Length: {}", request.body.len()));
        }
        lines.push(String::new());
        lines.push(request.body.clone());
        lines.join("\r\n")
    }

    /// Strip the HTTP response headers and return (status, body).
    fn parse_http_response(response: &str) -> Result<(u16, &str), String> {
        let (head, body) = response
            .split_once("\r\n\r\n")
            .ok_or("invalid HTTP response: no header/body delimiter")?;
        let status_line = head
            .lines()
            .next()
            .ok_or("invalid HTTP response: empty headers")?;
        let status: u16 = status_line
            .split_whitespace()
            .nth(1)
            .ok_or("invalid HTTP response: no status code")?
            .parse()
            .map_err(|e| format!("invalid HTTP status code: {e}"))?;
        Ok((status, body))
    }

    export!(LibpodPlugin);
}
