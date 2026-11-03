//! Host-run integration test over the pure core: no wasm toolchain needed.

use mastodon::core::{ToolInput, build_request, format_response};

fn input(op: &str) -> ToolInput {
    ToolInput {
        operation: op.to_string(),
        ..Default::default()
    }
}

#[test]
fn get_account_request_is_well_formed() {
    let mut input = input("get_account");
    input.account = Some("@example@mastodon.social".to_string());
    let (req, summary) = build_request(&input).unwrap();
    assert_eq!(req.method, "GET");
    assert!(req.path.contains("/accounts/lookup"));
    assert_eq!(summary, "Look up account");
}

#[test]
fn post_toot_request_serializes_status_and_visibility() {
    let mut input = input("post_toot");
    input.text = Some("Hello Mastodon!".to_string());
    input.visibility = Some("unlisted".to_string());
    let (req, _) = build_request(&input).unwrap();
    assert_eq!(req.method, "POST");
    assert!(req.path.ends_with("/statuses"));
    let body: serde_json::Value = serde_json::from_str(&req.body).unwrap();
    assert_eq!(body["status"], "Hello Mastodon!");
    assert_eq!(body["visibility"], "unlisted");
}

#[test]
fn delete_toot_request_uses_delete_method() {
    let mut input = input("delete_toot");
    input.id = Some("12345".to_string());
    let (req, _) = build_request(&input).unwrap();
    assert_eq!(req.method, "DELETE");
    assert!(req.path.ends_with("/statuses/12345"));
}

#[test]
fn format_posted_status_produces_attributed_summary() {
    let mut input = input("post_toot");
    input.text = Some("Hello".to_string());
    let (req, _) = build_request(&input).unwrap();
    let body = r#"{"id":"99","url":"https://mastodon.social/@user/99","content":"<p>Hello</p>"}"#;
    let result = format_response(&req, 200, body).unwrap();
    assert!(result.summary.contains("Mastodon plugin result"));
    assert!(result.summary.contains("Hello"));
    assert!(result.summary.contains("Source:"));
}

#[test]
fn format_error_response_contains_status_and_url() {
    let mut input = input("post_toot");
    input.text = Some("Hello".to_string());
    let (req, _) = build_request(&input).unwrap();
    let body = r#"{"error":"Validation failed: Status text is empty","error_description":"Status text is empty"}"#;
    let err = format_response(&req, 422, body).unwrap_err();
    assert!(err.contains("422"));
    assert!(err.contains("Validation failed"));
}
