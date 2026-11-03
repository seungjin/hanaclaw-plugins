//! Pure logic for the `mastodon` ZeroClaw plugin.
//!
//! Builds Mastodon REST API requests and formats responses. This module has
//! no wit-bindgen dependency, so `cargo test` runs on the host without a wasm
//! toolchain.

use serde::{Deserialize, Serialize};

/// Arguments the model sends for the `mastodon` tool.
#[derive(Debug, Default, Deserialize)]
pub struct ToolInput {
    /// Operation to perform. Supported: `get_account`, `get_toots`, `post_toot`,
    /// `delete_toot`, `get_notifications`, `follow`, `unfollow`, `search`.
    pub operation: String,

    /// Toot text for `post_toot`; query string for `search`.
    #[serde(default)]
    pub text: Option<String>,

    /// Toot ID for `delete_toot`, or target account for `follow`/`unfollow`.
    #[serde(default)]
    pub id: Option<String>,

    /// Account identifier (handle like `@user`, user ID, or URL) for
    /// `get_account`, `follow`, and `unfollow`.
    #[serde(default)]
    pub account: Option<String>,

    /// Maximum number of items to return for list operations.
    #[serde(default)]
    pub limit: Option<u32>,

    /// Mastodon instance base URL supplied by the model (falls back to config).
    #[serde(default)]
    pub instance_url: Option<String>,

    /// Bearer access token supplied by the model (falls back to config).
    #[serde(default)]
    pub access_token: Option<String>,

    /// Visibility for `post_toot`: `public`, `unlisted`, `private`, or `direct`.
    #[serde(default)]
    pub visibility: Option<String>,

    /// Extra operation-specific options passed straight to the API.
    #[serde(default)]
    pub options: serde_json::Value,
}

/// Operator config supplied by the host. A withheld `config_read` grant
/// injects `{}`, so every field must default.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolConfig {
    #[serde(default = "default_instance_url")]
    pub instance_url: String,
    #[serde(default = "default_access_token")]
    pub access_token: String,
}

fn default_instance_url() -> String {
    "https://mastodon.social".to_string()
}

fn default_access_token() -> String {
    String::new()
}

impl ToolInput {
    /// Merge per-call overrides with operator config.
    pub fn with_config(mut self, raw: &serde_json::Value) -> Result<Self, String> {
        let cfg: ToolConfig =
            serde_json::from_value(raw.clone()).map_err(|e| format!("invalid config: {e}"))?;
        if self.instance_url.is_none() || self.instance_url.as_deref() == Some("") {
            self.instance_url = Some(cfg.instance_url);
        }
        if self.access_token.is_none() || self.access_token.as_deref() == Some("") {
            self.access_token = Some(cfg.access_token);
        }
        Ok(self)
    }

    /// Effective instance URL.
    pub fn instance_url(&self) -> String {
        self.instance_url
            .as_deref()
            .unwrap_or("https://mastodon.social")
            .trim_end_matches('/')
            .to_string()
    }

    /// Effective access token.
    pub fn access_token(&self) -> String {
        self.access_token.as_deref().unwrap_or("").to_string()
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

/// Result of formatting a Mastodon API response.
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

    match op {
        "get_account" => get_account_request(input),
        "get_toots" => get_toots_request(input),
        "post_toot" => post_toot_request(input),
        "delete_toot" => delete_toot_request(input),
        "get_notifications" => get_notifications_request(input),
        "follow" => follow_request(input),
        "unfollow" => unfollow_request(input),
        "search" => search_request(input),
        other => Err(format!("unknown operation: {other}")),
    }
}

fn api_headers(input: &ToolInput) -> Vec<(String, String)> {
    let mut headers = vec![
        ("Accept".to_string(), "application/json".to_string()),
        ("Content-Type".to_string(), "application/json".to_string()),
    ];
    let token = input.access_token();
    if !token.is_empty() {
        headers.push(("Authorization".to_string(), format!("Bearer {token}")));
    }
    headers
}

fn base_url(input: &ToolInput) -> String {
    input.instance_url()
}

fn require_text(input: &ToolInput, op: &str) -> Result<String, String> {
    input
        .text
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string())
        .ok_or_else(|| format!("'{op}' requires non-empty 'text'"))
}

fn require_id(input: &ToolInput, op: &str) -> Result<String, String> {
    input
        .id
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string())
        .ok_or_else(|| format!("'{op}' requires non-empty 'id'"))
}

fn require_account(input: &ToolInput, op: &str) -> Result<String, String> {
    input
        .account
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string())
        .ok_or_else(|| format!("'{op}' requires non-empty 'account'"))
}

fn limit_query(input: &ToolInput) -> Vec<(String, String)> {
    let mut query = Vec::new();
    if let Some(limit) = input.limit
        && limit > 0
    {
        query.push(("limit".to_string(), limit.to_string()));
    }
    query
}

fn get_account_request(input: &ToolInput) -> Result<(ApiRequest, String), String> {
    let account = require_account(input, "get_account")?;
    let query = vec![("acct".to_string(), account)];
    Ok((
        ApiRequest {
            method: "GET".to_string(),
            path: format!("{}/api/v1/accounts/lookup", base_url(input)),
            query,
            headers: api_headers(input),
            body: String::new(),
        },
        "Look up account".to_string(),
    ))
}

fn get_toots_request(input: &ToolInput) -> Result<(ApiRequest, String), String> {
    let id = require_id(input, "get_toots")?;
    let mut query = limit_query(input);
    query.extend(
        input
            .options
            .as_object()
            .map(|o| {
                o.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
    );
    Ok((
        ApiRequest {
            method: "GET".to_string(),
            path: format!("{}/api/v1/accounts/{id}/statuses", base_url(input)),
            query,
            headers: api_headers(input),
            body: String::new(),
        },
        format!("List toots for account {id}"),
    ))
}

fn post_toot_request(input: &ToolInput) -> Result<(ApiRequest, String), String> {
    let text = require_text(input, "post_toot")?;
    let visibility = input
        .visibility
        .as_deref()
        .map(|v| v.trim().to_lowercase())
        .filter(|v| matches!(v.as_str(), "public" | "unlisted" | "private" | "direct"))
        .unwrap_or_else(|| "public".to_string());
    let body = serde_json::json!({
        "status": text,
        "visibility": visibility,
    })
    .to_string();
    Ok((
        ApiRequest {
            method: "POST".to_string(),
            path: format!("{}/api/v1/statuses", base_url(input)),
            query: vec![],
            headers: api_headers(input),
            body,
        },
        "Post a toot".to_string(),
    ))
}

fn delete_toot_request(input: &ToolInput) -> Result<(ApiRequest, String), String> {
    let id = require_id(input, "delete_toot")?;
    Ok((
        ApiRequest {
            method: "DELETE".to_string(),
            path: format!("{}/api/v1/statuses/{id}", base_url(input)),
            query: vec![],
            headers: api_headers(input),
            body: String::new(),
        },
        format!("Delete toot {id}"),
    ))
}

fn get_notifications_request(input: &ToolInput) -> Result<(ApiRequest, String), String> {
    let mut query = limit_query(input);
    if let Some(exclude) = input
        .options
        .get("exclude_types")
        .and_then(|v| v.as_array())
    {
        for item in exclude {
            if let Some(t) = item.as_str() {
                query.push(("exclude_types[]".to_string(), t.to_string()));
            }
        }
    }
    Ok((
        ApiRequest {
            method: "GET".to_string(),
            path: format!("{}/api/v1/notifications", base_url(input)),
            query,
            headers: api_headers(input),
            body: String::new(),
        },
        "Get notifications".to_string(),
    ))
}

fn follow_request(input: &ToolInput) -> Result<(ApiRequest, String), String> {
    let id = require_account(input, "follow")?;
    Ok((
        ApiRequest {
            method: "POST".to_string(),
            path: format!("{}/api/v1/accounts/{id}/follow", base_url(input)),
            query: vec![],
            headers: api_headers(input),
            body: String::new(),
        },
        format!("Follow account {id}"),
    ))
}

fn unfollow_request(input: &ToolInput) -> Result<(ApiRequest, String), String> {
    let id = require_account(input, "unfollow")?;
    Ok((
        ApiRequest {
            method: "POST".to_string(),
            path: format!("{}/api/v1/accounts/{id}/unfollow", base_url(input)),
            query: vec![],
            headers: api_headers(input),
            body: String::new(),
        },
        format!("Unfollow account {id}"),
    ))
}

fn search_request(input: &ToolInput) -> Result<(ApiRequest, String), String> {
    let text = require_text(input, "search")?;
    let mut query = vec![
        ("q".to_string(), text),
        ("type".to_string(), "statuses".to_string()),
    ];
    query.extend(limit_query(input));
    Ok((
        ApiRequest {
            method: "GET".to_string(),
            path: format!("{}/api/v2/search", base_url(input)),
            query,
            headers: api_headers(input),
            body: String::new(),
        },
        "Search statuses".to_string(),
    ))
}

/// Format a Mastodon JSON response into a concise report.
pub fn format_response(
    request: &ApiRequest,
    status_code: u16,
    body: &str,
) -> Result<FormattedResult, String> {
    let source_url = build_source_url(request);

    if status_code >= 400 {
        let msg = parse_error(body).unwrap_or_else(|| body.to_string());
        return Err(format!(
            "Mastodon API returned HTTP {status_code}\nURL: {source_url}\n{msg}"
        ));
    }

    let summary = match request.path.split('/').next_back() {
        Some("lookup") => format_account_lookup(body)?,
        Some("statuses") if request.method == "GET" => format_status_list(body)?,
        Some("statuses") if request.method == "POST" => format_posted_status(body)?,
        Some("notifications") => format_notification_list(body)?,
        Some("search") => format_search_results(body)?,
        Some(last) if request.path.ends_with("/follow") || request.path.ends_with("/unfollow") => {
            format_relationship(body, last)?
        }
        Some("unfollow") => format_relationship(body, "unfollow")?,
        _ => format_generic(body)?,
    };

    Ok(FormattedResult {
        summary: format!("🦣 Mastodon plugin result\n\n{summary}\n\nSource: {source_url}"),
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
    let message = value.get("error")?.as_str()?;
    let detail = value
        .get("error_description")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if detail.is_empty() {
        Some(message.to_string())
    } else {
        Some(format!("{message}: {detail}"))
    }
}

fn format_account_lookup(body: &str) -> Result<String, String> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| format!("failed to parse account lookup response: {e}"))?;
    let id = value["id"].as_str().unwrap_or("?");
    let acct = value["acct"].as_str().unwrap_or("?");
    let display = value["display_name"].as_str().unwrap_or("?");
    let url = value["url"].as_str().unwrap_or("?");
    let followers = value["followers_count"].as_u64().unwrap_or(0);
    let following = value["following_count"].as_u64().unwrap_or(0);
    let statuses = value["statuses_count"].as_u64().unwrap_or(0);
    Ok(format!(
        "Account: @{acct} ({display})\nID: {id}\nProfile: {url}\n\
        Followers: {followers}  Following: {following}  Toots: {statuses}"
    ))
}

fn format_status_list(body: &str) -> Result<String, String> {
    let items: Vec<serde_json::Value> =
        serde_json::from_str(body).map_err(|e| format!("failed to parse statuses: {e}"))?;
    if items.is_empty() {
        return Ok("No toots found.".to_string());
    }
    let mut lines = Vec::new();
    for item in items.iter().take(20) {
        let id = item["id"].as_str().unwrap_or("?");
        let content = strip_html(item["content"].as_str().unwrap_or(""));
        let created = item["created_at"].as_str().unwrap_or("?");
        let url = item["url"].as_str().unwrap_or("?");
        let reblogs = item["reblogs_count"].as_u64().unwrap_or(0);
        let favourites = item["favourites_count"].as_u64().unwrap_or(0);
        lines.push(format!(
            "- [{id}] {content}\n  {created} | 🔁 {reblogs} | ⭐ {favourites}\n  {url}"
        ));
    }
    Ok(lines.join("\n\n"))
}

fn format_posted_status(body: &str) -> Result<String, String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("failed to parse posted status: {e}"))?;
    let id = value["id"].as_str().unwrap_or("?");
    let url = value["url"].as_str().unwrap_or("?");
    let content = strip_html(value["content"].as_str().unwrap_or(""));
    Ok(format!("Posted toot {id}:\n\n{content}\n\nURL: {url}"))
}

fn format_notification_list(body: &str) -> Result<String, String> {
    let items: Vec<serde_json::Value> =
        serde_json::from_str(body).map_err(|e| format!("failed to parse notifications: {e}"))?;
    if items.is_empty() {
        return Ok("No notifications found.".to_string());
    }
    let mut lines = Vec::new();
    for item in items.iter().take(20) {
        let kind = item["type"].as_str().unwrap_or("?");
        let account = item["account"]["acct"].as_str().unwrap_or("?");
        let status = item["status"]["content"]
            .as_str()
            .map(strip_html)
            .unwrap_or_default();
        let text = if status.is_empty() {
            format!("{kind} from @{account}")
        } else {
            format!("{kind} from @{account}: {status}")
        };
        lines.push(format!("- {text}"));
    }
    Ok(lines.join("\n"))
}

fn format_search_results(body: &str) -> Result<String, String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("failed to parse search results: {e}"))?;
    let statuses_binding = Vec::new();
    let statuses = value["statuses"].as_array().unwrap_or(&statuses_binding);
    if statuses.is_empty() {
        return Ok("No matching statuses found.".to_string());
    }
    let mut lines = Vec::new();
    for item in statuses.iter().take(20) {
        let id = item["id"].as_str().unwrap_or("?");
        let acct = item["account"]["acct"].as_str().unwrap_or("?");
        let content = strip_html(item["content"].as_str().unwrap_or(""));
        let url = item["url"].as_str().unwrap_or("?");
        lines.push(format!("- [{id}] @{acct}: {content}\n  {url}"));
    }
    Ok(lines.join("\n\n"))
}

fn format_relationship(body: &str, action: &str) -> Result<String, String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("failed to parse relationship: {e}"))?;
    let id = value["id"].as_str().unwrap_or("?");
    let following = value["following"].as_bool().unwrap_or(false);
    let followed_by = value["followed_by"].as_bool().unwrap_or(false);
    Ok(format!(
        "Account {id}: action={action}, following={following}, followed_by={followed_by}"
    ))
}

fn format_generic(body: &str) -> Result<String, String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("failed to parse response: {e}"))?;
    serde_json::to_string_pretty(&value).map_err(|e| format!("failed to format response: {e}"))
}

/// Strip simple HTML tags from Mastodon content strings.
fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        if ch == '<' {
            in_tag = true;
        } else if ch == '>' {
            in_tag = false;
        } else if !in_tag {
            out.push(ch);
        }
    }
    // Collapse runs of whitespace left by removed tags.
    let mut collapsed = String::with_capacity(out.len());
    let mut prev_space = false;
    for ch in out.chars() {
        if ch.is_whitespace() {
            if !prev_space {
                collapsed.push(' ');
                prev_space = true;
            }
        } else {
            collapsed.push(ch);
            prev_space = false;
        }
    }
    collapsed.trim().to_string()
}

/// Run the tool logic against a raw HTTP response body.
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

    fn input_with_text(op: &str, text: &str) -> ToolInput {
        let mut input = input(op);
        input.text = Some(text.to_string());
        input
    }

    fn input_with_id(op: &str, id: &str) -> ToolInput {
        let mut input = input(op);
        input.id = Some(id.to_string());
        input
    }

    fn input_with_account(op: &str, account: &str) -> ToolInput {
        let mut input = input(op);
        input.account = Some(account.to_string());
        input
    }

    #[test]
    fn get_account_builds_lookup_request() {
        let input = input_with_account("get_account", "@example@mastodon.social");
        let (req, summary) = build_request(&input).unwrap();
        assert_eq!(req.method, "GET");
        assert!(req.path.contains("/accounts/lookup"));
        assert_eq!(summary, "Look up account");
    }

    #[test]
    fn get_toots_requires_id() {
        let input = input("get_toots");
        assert!(build_request(&input).is_err());
    }

    #[test]
    fn post_toot_requires_text() {
        let input = input("post_toot");
        assert!(build_request(&input).is_err());
    }

    #[test]
    fn post_toot_builds_post_request() {
        let input = input_with_text("post_toot", "Hello Mastodon!");
        let (req, summary) = build_request(&input).unwrap();
        assert_eq!(req.method, "POST");
        assert!(req.path.ends_with("/statuses"));
        assert_eq!(summary, "Post a toot");
        let body: serde_json::Value = serde_json::from_str(&req.body).unwrap();
        assert_eq!(body["status"], "Hello Mastodon!");
        assert_eq!(body["visibility"], "public");
    }

    #[test]
    fn delete_toot_requires_id() {
        let input = input("delete_toot");
        assert!(build_request(&input).is_err());
    }

    #[test]
    fn delete_toot_builds_delete_request() {
        let input = input_with_id("delete_toot", "12345");
        let (req, _) = build_request(&input).unwrap();
        assert_eq!(req.method, "DELETE");
        assert!(req.path.ends_with("/statuses/12345"));
    }

    #[test]
    fn format_account_lookup_works() {
        let input = input_with_account("get_account", "@example@mastodon.social");
        let (req, _) = build_request(&input).unwrap();
        let body = r#"{"id":"42","acct":"example","display_name":"Example","url":"https://mastodon.social/@example","followers_count":5,"following_count":3,"statuses_count":12}"#;
        let result = format_response(&req, 200, body).unwrap();
        assert!(result.summary.contains("Mastodon plugin result"));
        assert!(result.summary.contains("@example"));
        assert!(result.summary.contains("Example"));
    }

    #[test]
    fn format_posted_status_works() {
        let input = input_with_text("post_toot", "Hello");
        let (req, _) = build_request(&input).unwrap();
        let body =
            r#"{"id":"99","url":"https://mastodon.social/@user/99","content":"<p>Hello</p>"}"#;
        let result = format_response(&req, 200, body).unwrap();
        assert!(result.summary.contains("Posted toot"));
        assert!(result.summary.contains("Hello"));
    }

    #[test]
    fn format_status_list_works() {
        let input = input_with_id("get_toots", "42");
        let (req, _) = build_request(&input).unwrap();
        let body = r#"[{"id":"1","content":"<p>First</p>","created_at":"2024-01-01","url":"https://x/1","reblogs_count":2,"favourites_count":5}]"#;
        let result = format_response(&req, 200, body).unwrap();
        assert!(result.summary.contains("First"));
        assert!(result.summary.contains("2024-01-01"));
    }

    #[test]
    fn strip_html_removes_tags() {
        assert_eq!(strip_html("<p>Hello <b>world</b></p>"), "Hello world");
    }

    #[test]
    fn config_defaults_fall_back() {
        let input: ToolInput =
            serde_json::from_str(r#"{"operation":"get_account","account":"@x"}"#).unwrap();
        let input = input.with_config(&serde_json::json!({})).unwrap();
        assert_eq!(input.instance_url(), "https://mastodon.social");
        assert!(input.access_token().is_empty());
    }

    #[test]
    fn per_call_token_overrides_config() {
        let mut input = input("post_toot");
        input.text = Some("hi".to_string());
        input.access_token = Some("abc123".to_string());
        let input = input
            .with_config(&serde_json::json!({"access_token":"default"}))
            .unwrap();
        let (req, _) = build_request(&input).unwrap();
        assert!(
            req.headers
                .iter()
                .any(|(k, v)| k == "Authorization" && v == "Bearer abc123")
        );
    }
}
