//! A ZeroClaw WIT tool plugin: `mastodon`.
//!
//! Read and post to a Mastodon account
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

    struct MastodonPlugin;

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
        /// Toot text for `post_toot`; query string for `search`.
        #[serde(default)]
        text: Option<String>,
        /// Toot ID for `delete_toot`, or target account for `follow`/`unfollow`.
        #[serde(default)]
        id: Option<String>,
        /// Account identifier for `get_account`, `follow`, and `unfollow`.
        #[serde(default)]
        account: Option<String>,
        /// Maximum number of items to return for list operations.
        #[serde(default)]
        limit: Option<u32>,
        /// Mastodon instance base URL (falls back to config).
        #[serde(default)]
        instance_url: Option<String>,
        /// Bearer access token (falls back to config).
        #[serde(default)]
        access_token: Option<String>,
        /// Visibility for `post_toot`.
        #[serde(default)]
        visibility: Option<String>,
        /// Extra operation-specific options passed straight to the API.
        #[serde(default)]
        options: serde_json::Value,
    }

    impl From<ExecuteArgs> for ToolInput {
        fn from(args: ExecuteArgs) -> Self {
            ToolInput {
                operation: args.operation,
                text: args.text,
                id: args.id,
                account: args.account,
                limit: args.limit,
                instance_url: args.instance_url,
                access_token: args.access_token,
                visibility: args.visibility,
                options: args.options,
            }
        }
    }

    impl PluginInfo for MastodonPlugin {
        fn plugin_name() -> String {
            // Fixed at generation time; the host does not inject it.
            "mastodon".to_string()
        }

        fn plugin_version() -> String {
            env!("CARGO_PKG_VERSION").to_string()
        }
    }

    impl Tool for MastodonPlugin {
        fn name() -> String {
            "mastodon".to_string()
        }

        fn description() -> String {
            // Fixed at generation time, so a static string is all the host sees.
            "Read and post to a Mastodon account".to_string()
        }

        // The model's entire view of the tool: valid JSON or the host refuses
        // to register the plugin. Never declare `__config` here.
        fn parameters_schema() -> String {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "operation": {
                        "type": "string",
                        "description": "Operation to perform: get_account, get_toots, post_toot, delete_toot, get_notifications, follow, unfollow, search."
                    },
                    "text": {
                        "type": "string",
                        "description": "Toot text for post_toot; query string for search."
                    },
                    "id": {
                        "type": "string",
                        "description": "Toot ID for delete_toot, or target account ID for get_toots/follow/unfollow."
                    },
                    "account": {
                        "type": "string",
                        "description": "Account handle like @user, user ID, or URL for get_account/follow/unfollow."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 40,
                        "description": "Maximum number of items to return for list operations."
                    },
                    "instance_url": {
                        "type": "string",
                        "format": "uri",
                        "description": "Mastodon instance base URL (default: from config)."
                    },
                    "access_token": {
                        "type": "string",
                        "description": "Bearer access token (default: from config)."
                    },
                    "visibility": {
                        "type": "string",
                        "enum": ["public", "unlisted", "private", "direct"],
                        "description": "Visibility for post_toot (default: public)."
                    },
                    "options": {
                        "type": "object",
                        "description": "Extra operation-specific options passed to the API."
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
                    function_name: "mastodon::tool::execute".to_string(),
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

    /// Build the Mastodon API request, send it over the WIT `sockets`
    /// interface, and pass the HTTP response back to the pure core for
    /// formatting.
    fn fetch_and_run(input: &ToolInput) -> Result<String, String> {
        let (request, _summary) = crate::plugin_core::build_request(input)?;
        let host_port = parse_url(&request.path)?;

        let connection = connect(&zeroclaw::plugin::sockets::ConnectRequest {
            host: host_port.host.clone(),
            port: host_port.port,
            mode: ConnectMode::DirectTls,
            tls_profile: None,
        })
        .map_err(|e| format!("failed to connect to Mastodon: {e:?}"))?;

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

    /// Parse a URL like "https://mastodon.social/api/v1/..." into host and
    /// port. The request path already contains the full URL because the core
    /// embeds the instance base URL.
    fn parse_url(url: &str) -> Result<HostPort, String> {
        let url = url.trim();
        let rest = url
            .strip_prefix("https://")
            .or_else(|| url.strip_prefix("http://"))
            .unwrap_or(url);
        let host_part = rest.split('/').next().unwrap_or(rest);
        let (host, port_str) = host_part.rsplit_once(':').unwrap_or((host_part, "443"));
        let port: u16 = port_str.parse().map_err(|e| format!("invalid port: {e}"))?;
        Ok(HostPort {
            host: host.to_string(),
            port,
        })
    }

    /// Serialize an `ApiRequest` into an HTTP/1.1 request string. The path
    /// already contains the full instance URL, so we strip the origin to build
    /// the HTTP path.
    fn build_http_request(
        request: &crate::plugin_core::ApiRequest,
        host: &str,
        port: u16,
    ) -> String {
        let path_only = request
            .path
            .trim_start_matches("https://")
            .trim_start_matches("http://");
        let path_and_origin = path_only.split('/').skip(1).collect::<Vec<_>>().join("/");
        let path_and_query = if request.query.is_empty() {
            format!("/{path_and_origin}")
        } else {
            let qs: Vec<String> = request
                .query
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            format!("/{path_and_origin}?{}", qs.join("&"))
        };

        let mut lines = vec![
            format!("{} {} HTTP/1.1", request.method, path_and_query),
            format!("Host: {host}:{port}"),
            "User-Agent: hanaclaw-mastodon-plugin/0.1.0".to_string(),
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

    export!(MastodonPlugin);
}
