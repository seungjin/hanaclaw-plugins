//! Pure logic for the `libpod` ZeroClaw plugin.
//!
//! Builds HTTP requests for the Podman libpod REST API and formats the JSON
//! responses into readable reports. No wit-bindgen here, so `cargo test`
//! runs on the host without a wasm toolchain.

use serde::{Deserialize, Serialize};

/// Arguments the model sends for the `libpod` tool.
#[derive(Debug, Default, Deserialize)]
pub struct ToolInput {
    /// Operation to perform. Supported: list_containers, list_images,
    /// container_info, run_container, start_container, stop_container,
    /// remove_container, container_logs, exec_in_container, pull_image.
    pub operation: String,

    /// Container or image name/ID, depending on the operation.
    #[serde(default)]
    pub name: Option<String>,

    /// Image reference for run_container / pull_image, e.g.
    /// "docker.io/library/alpine:latest".
    #[serde(default)]
    pub image: Option<String>,

    /// Command and args for run_container or exec_in_container.
    #[serde(default)]
    pub command: Vec<String>,

    /// Environment variables as "KEY=value" strings for run_container.
    #[serde(default)]
    pub env: Vec<String>,

    /// Port mappings in "host:container" form for run_container, e.g. "8080:80".
    #[serde(default)]
    pub ports: Vec<String>,

    /// Bind mounts in "host:container" form for run_container, e.g. "/data:/data".
    #[serde(default)]
    pub mounts: Vec<String>,

    /// If true, list/show all containers/images rather than only running/top-level.
    #[serde(default)]
    pub all: bool,

    /// Container create/start/stop/remove options passed straight to the API.
    #[serde(default)]
    pub options: serde_json::Value,
}

/// Operator config supplied by the host. A withheld `config_read` grant
/// injects `{}`, so every field must default.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolConfig {
    #[serde(default = "default_api_url")]
    pub api_url: String,
    #[serde(default = "default_api_version")]
    pub api_version: String,
}

fn default_api_url() -> String {
    "http://127.0.0.1:8888".to_string()
}

fn default_api_version() -> String {
    "v5.0".to_string()
}

impl ToolInput {
    /// Merge per-call overrides with operator config.
    pub fn with_config(mut self, raw: &serde_json::Value) -> Result<Self, String> {
        let cfg: ToolConfig =
            serde_json::from_value(raw.clone()).map_err(|e| format!("invalid config: {e}"))?;
        if self.options.is_null() {
            self.options = serde_json::Value::Object(serde_json::Map::new());
        }
        let opts = self.options.as_object_mut().unwrap();
        opts.entry("__config_api_url")
            .or_insert(serde_json::Value::String(cfg.api_url));
        opts.entry("__config_api_version")
            .or_insert(serde_json::Value::String(cfg.api_version));
        Ok(self)
    }

    /// Effective API base URL from injected config.
    pub fn api_url(&self) -> String {
        self.options
            .get("__config_api_url")
            .and_then(|v| v.as_str())
            .unwrap_or("http://127.0.0.1:8888")
            .trim_end_matches('/')
            .to_string()
    }

    /// Effective API version path segment from injected config.
    pub fn api_version(&self) -> String {
        self.options
            .get("__config_api_version")
            .and_then(|v| v.as_str())
            .unwrap_or("v5.0")
            .trim_start_matches('/')
            .trim_end_matches('/')
            .to_string()
    }

    /// Full base path for libpod endpoints, e.g. "http://127.0.0.1:8888/v5.0/libpod".
    pub fn base_path(&self) -> String {
        format!("{}/{}/libpod", self.api_url(), self.api_version())
    }
}

/// One HTTP request the wasm shim should send.
#[derive(Debug, Serialize, Deserialize)]
pub struct ApiRequest {
    pub method: String,
    pub path: String,
    pub query: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

/// Result of formatting a libpod API response.
#[derive(Debug, Serialize, Deserialize)]
pub struct FormattedResult {
    pub summary: String,
    pub source_url: String,
}

/// Build the API request and a human-readable summary of what it will do.
pub fn build_request(input: &ToolInput) -> Result<(ApiRequest, String), String> {
    let op = input.operation.trim();
    if op.is_empty() {
        return Err("operation is required".to_string());
    }

    let base = input.base_path();

    match op {
        "list_containers" => list_containers_request(input, &base),
        "list_images" => list_images_request(input, &base),
        "container_info" => container_info_request(input, &base),
        "container_logs" => container_logs_request(input, &base),
        "run_container" => run_container_request(input, &base),
        "start_container" => start_container_request(input, &base),
        "stop_container" => stop_container_request(input, &base),
        "remove_container" => remove_container_request(input, &base),
        "exec_in_container" => exec_in_container_request(input, &base),
        "pull_image" => pull_image_request(input, &base),
        other => Err(format!("unknown operation: {other}")),
    }
}

fn list_containers_request(input: &ToolInput, base: &str) -> Result<(ApiRequest, String), String> {
    let mut query = vec![("all".to_string(), input.all.to_string())];
    if let Some(limit) = input.options.get("limit").and_then(|v| v.as_u64()) {
        query.push(("limit".to_string(), limit.to_string()));
    }
    Ok((
        ApiRequest {
            method: "GET".to_string(),
            path: format!("{}/containers/json", base),
            query,
            headers: vec![("Accept".to_string(), "application/json".to_string())],
            body: String::new(),
        },
        "List containers".to_string(),
    ))
}

fn list_images_request(input: &ToolInput, base: &str) -> Result<(ApiRequest, String), String> {
    let query = vec![("all".to_string(), input.all.to_string())];
    Ok((
        ApiRequest {
            method: "GET".to_string(),
            path: format!("{}/images/json", base),
            query,
            headers: vec![("Accept".to_string(), "application/json".to_string())],
            body: String::new(),
        },
        "List images".to_string(),
    ))
}

fn require_name(input: &ToolInput, op: &str) -> Result<String, String> {
    input
        .name
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string())
        .ok_or_else(|| format!("'{op}' requires a non-empty 'name'"))
}

fn container_info_request(input: &ToolInput, base: &str) -> Result<(ApiRequest, String), String> {
    let name = require_name(input, "container_info")?;
    Ok((
        ApiRequest {
            method: "GET".to_string(),
            path: format!("{}/containers/{name}/json", base),
            query: vec![],
            headers: vec![("Accept".to_string(), "application/json".to_string())],
            body: String::new(),
        },
        format!("Inspect container {name}"),
    ))
}

fn container_logs_request(input: &ToolInput, base: &str) -> Result<(ApiRequest, String), String> {
    let name = require_name(input, "container_logs")?;
    let mut query = vec![
        ("stdout".to_string(), "true".to_string()),
        ("stderr".to_string(), "true".to_string()),
    ];
    if input
        .options
        .get("follow")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        query.push(("follow".to_string(), "true".to_string()));
    }
    if let Some(tail) = input.options.get("tail").and_then(|v| v.as_u64()) {
        query.push(("tail".to_string(), tail.to_string()));
    }
    Ok((
        ApiRequest {
            method: "GET".to_string(),
            path: format!("{}/containers/{name}/logs", base),
            query,
            headers: vec![("Accept".to_string(), "application/json".to_string())],
            body: String::new(),
        },
        format!("Fetch logs for container {name}"),
    ))
}

fn run_container_request(input: &ToolInput, base: &str) -> Result<(ApiRequest, String), String> {
    let image = input
        .image
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .ok_or("'run_container' requires a non-empty 'image'")?;

    let mut create: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
    create.insert(
        "Image".to_string(),
        serde_json::Value::String(image.to_string()),
    );
    if !input.command.is_empty() {
        create.insert(
            "Cmd".to_string(),
            serde_json::Value::Array(
                input
                    .command
                    .iter()
                    .cloned()
                    .map(serde_json::Value::String)
                    .collect(),
            ),
        );
    }
    if !input.env.is_empty() {
        create.insert(
            "Env".to_string(),
            serde_json::Value::Array(
                input
                    .env
                    .iter()
                    .cloned()
                    .map(serde_json::Value::String)
                    .collect(),
            ),
        );
    }
    if !input.ports.is_empty() {
        create.insert("ExposedPorts".to_string(), parse_ports(&input.ports));
        create.insert(
            "HostConfig".to_string(),
            serde_json::json!({"PortBindings": parse_port_bindings(&input.ports)}),
        );
    }
    if !input.mounts.is_empty() {
        let binds = input
            .mounts
            .iter()
            .map(|m| serde_json::Value::String(m.clone()))
            .collect();
        let host_config = create
            .entry("HostConfig")
            .or_insert_with(|| serde_json::json!({}))
            .as_object_mut()
            .unwrap();
        host_config.insert("Binds".to_string(), serde_json::Value::Array(binds));
    }

    // Merge any extra create options from `options.create`.
    if let Some(extra) = input.options.get("create").and_then(|v| v.as_object()) {
        for (k, v) in extra {
            create.insert(k.clone(), v.clone());
        }
    }

    let name = input.name.as_deref().unwrap_or("").trim();
    let mut query = vec![];
    if !name.is_empty() {
        query.push(("name".to_string(), name.to_string()));
    }

    let body = serde_json::Value::Object(create).to_string();
    Ok((
        ApiRequest {
            method: "POST".to_string(),
            path: format!("{}/containers/create", base),
            query,
            headers: vec![
                ("Accept".to_string(), "application/json".to_string()),
                ("Content-Type".to_string(), "application/json".to_string()),
            ],
            body,
        },
        format!("Create container from image {image}"),
    ))
}

fn parse_ports(ports: &[String]) -> serde_json::Value {
    let mut exposed = serde_json::Map::new();
    for mapping in ports {
        let parts: Vec<&str> = mapping.splitn(2, ':').collect();
        if parts.len() == 2 {
            let container = parts[1];
            exposed.insert(format!("{container}/tcp"), serde_json::json!({}));
        }
    }
    serde_json::Value::Object(exposed)
}

fn parse_port_bindings(ports: &[String]) -> serde_json::Value {
    let mut bindings = serde_json::Map::new();
    for mapping in ports {
        let parts: Vec<&str> = mapping.splitn(2, ':').collect();
        if parts.len() == 2 {
            let host = parts[0];
            let container = parts[1];
            bindings.insert(
                format!("{container}/tcp"),
                serde_json::json!([{"HostIp":"","HostPort":host}]),
            );
        }
    }
    serde_json::Value::Object(bindings)
}

fn start_container_request(input: &ToolInput, base: &str) -> Result<(ApiRequest, String), String> {
    let name = require_name(input, "start_container")?;
    Ok((
        ApiRequest {
            method: "POST".to_string(),
            path: format!("{}/containers/{name}/start", base),
            query: vec![],
            headers: vec![("Accept".to_string(), "application/json".to_string())],
            body: String::new(),
        },
        format!("Start container {name}"),
    ))
}

fn stop_container_request(input: &ToolInput, base: &str) -> Result<(ApiRequest, String), String> {
    let name = require_name(input, "stop_container")?;
    let mut query = vec![];
    if let Some(t) = input.options.get("timeout").and_then(|v| v.as_u64()) {
        query.push(("timeout".to_string(), t.to_string()));
    }
    if input
        .options
        .get("ignore")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        query.push(("ignore".to_string(), "true".to_string()));
    }
    Ok((
        ApiRequest {
            method: "POST".to_string(),
            path: format!("{}/containers/{name}/stop", base),
            query,
            headers: vec![("Accept".to_string(), "application/json".to_string())],
            body: String::new(),
        },
        format!("Stop container {name}"),
    ))
}

fn remove_container_request(input: &ToolInput, base: &str) -> Result<(ApiRequest, String), String> {
    let name = require_name(input, "remove_container")?;
    let mut query = vec![];
    if input
        .options
        .get("force")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        query.push(("force".to_string(), "true".to_string()));
    }
    if input
        .options
        .get("volumes")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        query.push(("volumes".to_string(), "true".to_string()));
    }
    Ok((
        ApiRequest {
            method: "DELETE".to_string(),
            path: format!("{}/containers/{name}", base),
            query,
            headers: vec![("Accept".to_string(), "application/json".to_string())],
            body: String::new(),
        },
        format!("Remove container {name}"),
    ))
}

fn exec_in_container_request(
    input: &ToolInput,
    base: &str,
) -> Result<(ApiRequest, String), String> {
    let name = require_name(input, "exec_in_container")?;
    if input.command.is_empty() {
        return Err("'exec_in_container' requires a non-empty 'command'".to_string());
    }
    let body = serde_json::json!({
        "Cmd": input.command,
        "AttachStdout": true,
        "AttachStderr": true,
    })
    .to_string();
    Ok((
        ApiRequest {
            method: "POST".to_string(),
            path: format!("{}/containers/{name}/exec", base),
            query: vec![],
            headers: vec![
                ("Accept".to_string(), "application/json".to_string()),
                ("Content-Type".to_string(), "application/json".to_string()),
            ],
            body,
        },
        format!("Create exec session in container {name}"),
    ))
}

fn pull_image_request(input: &ToolInput, base: &str) -> Result<(ApiRequest, String), String> {
    let reference = input
        .image
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .ok_or("'pull_image' requires a non-empty 'image'")?;
    let mut query = vec![
        ("reference".to_string(), reference.to_string()),
        ("quiet".to_string(), "true".to_string()),
    ];
    if input
        .options
        .get("compatMode")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        query.retain(|(k, _)| k != "quiet");
        query.push(("compatMode".to_string(), "true".to_string()));
    }
    Ok((
        ApiRequest {
            method: "POST".to_string(),
            path: format!("{}/images/pull", base),
            query,
            headers: vec![("Accept".to_string(), "application/json".to_string())],
            body: String::new(),
        },
        format!("Pull image {reference}"),
    ))
}

/// Format a libpod JSON response into a concise report.
pub fn format_response(
    request: &ApiRequest,
    status_code: u16,
    body: &str,
) -> Result<FormattedResult, String> {
    let source_url = build_source_url(request);

    if status_code >= 400 {
        let msg = parse_error(body).unwrap_or_else(|| body.to_string());
        return Err(format!(
            "libpod API returned HTTP {status_code}\nURL: {source_url}\n{msg}"
        ));
    }

    let summary = match request.path.split('/').next_back() {
        Some("json") if request.path.contains("/containers/") => format_container_info(body)?,
        Some("json") if request.path.contains("/images/") => format_image_info(body)?,
        Some("logs") => format_logs(body),
        Some("create") if request.path.contains("/containers/") => format_container_create(body)?,
        Some("create") if request.path.contains("/exec/") => format_exec_create(body)?,
        Some("start") | Some("stop") => format_empty_ok(request),
        Some("exec") if request.path.contains("/containers/") => format_exec_create(body)?,
        Some("pull") => format_image_pull(body)?,
        _ => {
            // Generic list or other: pretty-print JSON array/object summary.
            format_json_list(body)?
        }
    };

    Ok(FormattedResult {
        summary: format!("🦭 libpod plugin result\n\n{summary}\n\nSource: {source_url}"),
        source_url,
    })
}

fn build_source_url(request: &ApiRequest) -> String {
    let mut url = format!("{} {}", request.method, request.path);
    if !request.query.is_empty() {
        let qs: Vec<String> = request
            .query
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        url.push('?');
        url.push_str(&qs.join("&"));
    }
    url
}

fn parse_error(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let message = value.get("message")?.as_str()?;
    let cause = value.get("cause").and_then(|v| v.as_str()).unwrap_or("");
    if cause.is_empty() {
        Some(message.to_string())
    } else {
        Some(format!("{message} ({cause})"))
    }
}

fn format_container_info(body: &str) -> Result<String, String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("failed to parse container info: {e}"))?;
    let id = value["Id"].as_str().unwrap_or("?");
    let names = value["Name"]
        .as_str()
        .or_else(|| {
            value["Names"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|v| v.as_str())
        })
        .unwrap_or("?");
    let image = value["Image"].as_str().unwrap_or("?");
    let state = value["State"]["Status"].as_str().unwrap_or("?");
    let cmd = value["Config"]["Cmd"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
    Ok(format!(
        "Container: {names}\nID: {id}\nImage: {image}\nState: {state}\nCommand: {cmd}"
    ))
}

fn format_image_info(body: &str) -> Result<String, String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("failed to parse image info: {e}"))?;
    let id = value["Id"].as_str().unwrap_or("?");
    let tags = value["RepoTags"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let size = value["Size"].as_u64().unwrap_or(0);
    Ok(format!("Image: {tags}\nID: {id}\nSize: {size} bytes"))
}

fn format_json_list(body: &str) -> Result<String, String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("failed to parse response: {e}"))?;
    match value {
        serde_json::Value::Array(arr) => {
            if arr.is_empty() {
                Ok("No items found.".to_string())
            } else {
                Ok(format!("{} item(s) returned.", arr.len()))
            }
        }
        serde_json::Value::Object(_) => {
            Ok(serde_json::to_string_pretty(&value).unwrap_or_else(|_| body.to_string()))
        }
        _ => Ok(body.to_string()),
    }
}

fn format_logs(body: &str) -> String {
    // libpod logs are streamed with a framing header per chunk. Strip the
    // 8-byte header (1-byte stream + 3 padding + 4 size) best-effort.
    let mut out = String::new();
    let mut remaining = body;
    while !remaining.is_empty() {
        if remaining.len() > 8 && remaining.as_bytes()[0] <= 2 {
            let size = u32::from_be_bytes([
                remaining.as_bytes()[4],
                remaining.as_bytes()[5],
                remaining.as_bytes()[6],
                remaining.as_bytes()[7],
            ]) as usize;
            if 8 + size <= remaining.len() {
                out.push_str(&remaining[8..8 + size]);
                remaining = &remaining[8 + size..];
                continue;
            }
        }
        // Fallback: append whatever is left if framing is malformed.
        out.push_str(remaining);
        break;
    }
    if out.trim().is_empty() {
        "(no logs)".to_string()
    } else {
        out
    }
}

fn format_container_create(body: &str) -> Result<String, String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("failed to parse create response: {e}"))?;
    let id = value["Id"].as_str().unwrap_or("?");
    let warnings = value["Warnings"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        })
        .unwrap_or_default();
    let mut result = format!("Created container {id}");
    if !warnings.is_empty() {
        result.push_str("\nWarnings: ");
        result.push_str(&warnings);
    }
    Ok(result)
}

fn format_exec_create(body: &str) -> Result<String, String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("failed to parse exec response: {e}"))?;
    let id = value["Id"].as_str().unwrap_or("?");
    Ok(format!(
        "Created exec session {id}.\nNote: start it with operation=exec_start and name={id}"
    ))
}

fn format_image_pull(body: &str) -> Result<String, String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("failed to parse pull response: {e}"))?;
    let id = value["id"]
        .as_str()
        .or_else(|| value["Id"].as_str())
        .unwrap_or("?");
    Ok(format!("Pulled image {id}"))
}

fn format_empty_ok(request: &ApiRequest) -> String {
    format!("Operation succeeded: {} {}", request.method, request.path)
}

/// Run the tool logic against a raw HTTP response body.
///
/// In the real wasm guest the shim sends the request and passes the status
/// code + body back here; for host-run tests we call this directly.
pub fn run(input: &ToolInput, status_code: u16, response_body: &str) -> String {
    match build_request(input) {
        Ok((req, summary)) => match format_response(&req, status_code, response_body) {
            Ok(result) => result.summary,
            Err(e) => format!("Error: {e}\n\nIntended action: {summary}"),
        },
        Err(e) => format!("Error: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(op: &str) -> ToolInput {
        ToolInput {
            operation: op.to_string(),
            ..Default::default()
        }
    }

    fn input_with_name(op: &str, name: &str) -> ToolInput {
        let mut input = input(op);
        input.name = Some(name.to_string());
        input
    }

    fn input_with_image(op: &str, image: &str) -> ToolInput {
        let mut input = input(op);
        input.image = Some(image.to_string());
        input
    }

    #[test]
    fn list_containers_builds_get_request() {
        let input = input("list_containers");
        let (req, summary) = build_request(&input).unwrap();
        assert_eq!(req.method, "GET");
        assert!(req.path.ends_with("/containers/json"));
        assert_eq!(summary, "List containers");
    }

    #[test]
    fn list_images_builds_get_request() {
        let input = input("list_images");
        let (req, _) = build_request(&input).unwrap();
        assert_eq!(req.method, "GET");
        assert!(req.path.ends_with("/images/json"));
    }

    #[test]
    fn container_info_requires_name() {
        let input = input("container_info");
        assert!(build_request(&input).is_err());
    }

    #[test]
    fn container_info_builds_inspect_request() {
        let input = input_with_name("container_info", "my-container");
        let (req, _) = build_request(&input).unwrap();
        assert_eq!(req.method, "GET");
        assert!(req.path.ends_with("/containers/my-container/json"));
    }

    #[test]
    fn run_container_requires_image() {
        let input = input("run_container");
        assert!(build_request(&input).is_err());
    }

    #[test]
    fn run_container_builds_create_request() {
        let mut input = input_with_image("run_container", "docker.io/library/alpine:latest");
        input.command = vec!["sleep".to_string(), "30".to_string()];
        input.name = Some("alpine-test".to_string());
        let (req, _) = build_request(&input).unwrap();
        assert_eq!(req.method, "POST");
        assert!(req.path.ends_with("/containers/create"));
        assert!(
            req.query
                .contains(&("name".to_string(), "alpine-test".to_string()))
        );
        let body: serde_json::Value = serde_json::from_str(&req.body).unwrap();
        assert_eq!(body["Image"], "docker.io/library/alpine:latest");
        assert_eq!(body["Cmd"], serde_json::json!(["sleep", "30"]));
    }

    #[test]
    fn run_container_parses_ports_and_mounts() {
        let mut input = input_with_image("run_container", "docker.io/library/nginx:latest");
        input.ports = vec!["8080:80".to_string()];
        input.mounts = vec!["/tmp:/data".to_string()];
        let (req, _) = build_request(&input).unwrap();
        let body: serde_json::Value = serde_json::from_str(&req.body).unwrap();
        assert!(body["ExposedPorts"]["80/tcp"].is_object());
        assert_eq!(
            body["HostConfig"]["PortBindings"]["80/tcp"][0]["HostPort"],
            "8080"
        );
        assert_eq!(body["HostConfig"]["Binds"][0], "/tmp:/data");
    }

    #[test]
    fn exec_in_container_requires_command() {
        let input = input_with_name("exec_in_container", "my-container");
        assert!(build_request(&input).is_err());
    }

    #[test]
    fn exec_in_container_builds_exec_request() {
        let mut input = input_with_name("exec_in_container", "my-container");
        input.command = vec!["echo".to_string(), "hi".to_string()];
        let (req, _) = build_request(&input).unwrap();
        assert_eq!(req.method, "POST");
        assert!(req.path.ends_with("/containers/my-container/exec"));
        let body: serde_json::Value = serde_json::from_str(&req.body).unwrap();
        assert_eq!(body["Cmd"], serde_json::json!(["echo", "hi"]));
    }

    #[test]
    fn pull_image_builds_pull_request() {
        let input = input_with_image("pull_image", "docker.io/library/alpine:latest");
        let (req, _) = build_request(&input).unwrap();
        assert_eq!(req.method, "POST");
        assert!(req.path.ends_with("/images/pull"));
        assert!(req.query.contains(&(
            "reference".to_string(),
            "docker.io/library/alpine:latest".to_string(),
        )));
        assert!(
            req.query
                .contains(&("quiet".to_string(), "true".to_string()))
        );
    }

    #[test]
    fn formats_container_info_response() {
        let input = input_with_name("container_info", "my-container");
        let (req, _) = build_request(&input).unwrap();
        let body = r#"{"Id":"abc123","Name":"my-container","Image":"alpine:latest","State":{"Status":"running"},"Config":{"Cmd":["sleep","30"]}}"#;
        let result = format_response(&req, 200, body).unwrap();
        assert!(result.summary.contains("libpod plugin result"));
        assert!(result.summary.contains("running"));
        assert!(result.summary.contains("Source:"));
    }

    #[test]
    fn formats_error_response() {
        let input = input_with_name("container_info", "missing");
        let (req, _) = build_request(&input).unwrap();
        let body = r#"{"message":"no such container","cause":"not found","response":404}"#;
        let err = format_response(&req, 404, body).unwrap_err();
        assert!(err.contains("404"));
        assert!(err.contains("no such container"));
    }

    #[test]
    fn strips_log_framing() {
        let input = input_with_name("container_logs", "c");
        let (req, _) = build_request(&input).unwrap();
        let body = format!(
            "{}hello",
            std::str::from_utf8(&[0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05]).unwrap()
        );
        let result = format_response(&req, 200, &body).unwrap();
        assert!(result.summary.contains("hello"));
    }
}
