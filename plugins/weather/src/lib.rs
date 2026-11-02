//! A ZeroClaw WIT tool plugin: `weather`.
//!
//! Get current weather for a city from wttr.in
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

    use crate::plugin_core::{ToolInput, run};
    use exports::zeroclaw::plugin::plugin_info::Guest as PluginInfo;
    use exports::zeroclaw::plugin::tool::{Guest as Tool, ToolResult};
    use zeroclaw::plugin::logging::{
        LogLevel, PluginAction, PluginEvent, PluginOutcome, log_record,
    };
    use zeroclaw::plugin::sockets::{ConnectMode, connect};

    struct WeatherPlugin;

    #[derive(serde::Deserialize)]
    struct ExecuteArgs {
        /// City name or location query accepted by wttr.in.
        city: String,
    }

    impl PluginInfo for WeatherPlugin {
        fn plugin_name() -> String {
            // Fixed at generation time; the host does not inject it.
            "weather".to_string()
        }

        fn plugin_version() -> String {
            env!("CARGO_PKG_VERSION").to_string()
        }
    }

    impl Tool for WeatherPlugin {
        fn name() -> String {
            "weather".to_string()
        }

        fn description() -> String {
            // Fixed at generation time, so a static string is all the host sees.
            "Get current weather for a city from wttr.in".to_string()
        }

        // The model's entire view of the tool: valid JSON or the host refuses
        // to register the plugin. Never declare `__config` here.
        fn parameters_schema() -> String {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "city": {
                        "type": "string",
                        "description": "City name or location (e.g. \"Seoul\", \"New York\")."
                    }
                },
                "required": ["city"]
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

            let input = ToolInput { city: parsed.city };
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
                    function_name: "weather::tool::execute".to_string(),
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

    /// Build the wttr.in JSON URL for `city` and fetch it over the WIT
    /// `sockets` interface. Then pass the response to the pure core.
    fn fetch_and_run(input: &ToolInput) -> Result<String, String> {
        if input.city.trim().is_empty() {
            return Err("city is required".to_string());
        }

        let query = input.city.trim().replace(' ', "_").replace(['/', '\\'], "");
        let host = "wttr.in";
        let path_and_query = format!("/{query}?format=j1");

        let connection = connect(&zeroclaw::plugin::sockets::ConnectRequest {
            host: host.to_string(),
            port: 443,
            mode: ConnectMode::DirectTls,
            tls_profile: None,
        })
        .map_err(|e| format!("failed to connect to wttr.in: {e:?}"))?;

        let request = format!(
            "GET {path_and_query} HTTP/1.1\r\n\
            Host: {host}\r\n\
            User-Agent: hanaclaw-weather-plugin/0.1.0\r\n\
            Accept: application/json\r\n\
            Connection: close\r\n\
            \r\n"
        );
        connection
            .send(request.as_bytes())
            .map_err(|e| format!("failed to send request: {e:?}"))?;

        let mut response = Vec::new();
        loop {
            match connection.receive() {
                Ok(zeroclaw::plugin::sockets::ReceiveEvent::Data(chunk)) => {
                    response.extend_from_slice(&chunk);
                }
                Ok(zeroclaw::plugin::sockets::ReceiveEvent::Idle) => {
                    // Non-blocking idle is not terminal; retry briefly before
                    // assuming the response is complete.
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

        let body = String::from_utf8_lossy(&response);
        let body = extract_http_body(&body)?;
        Ok(run(input, body))
    }

    /// Strip the HTTP response headers and return the body.
    fn extract_http_body(response: &str) -> Result<&str, String> {
        let (_headers, body) = response
            .split_once("\r\n\r\n")
            .ok_or("invalid HTTP response: no header/body delimiter")?;
        Ok(body)
    }

    export!(WeatherPlugin);
}
