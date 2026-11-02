//! Host-run integration test over the pure core: no wasm toolchain needed.

use libpod::core::{ToolInput, build_request, format_response};

fn input(op: &str) -> ToolInput {
    ToolInput {
        operation: op.to_string(),
        ..Default::default()
    }
}

#[test]
fn list_containers_request_is_well_formed() {
    let input = input("list_containers");
    let (req, summary) = build_request(&input).unwrap();
    assert_eq!(req.method, "GET");
    assert!(req.path.contains("/containers/json"));
    assert_eq!(summary, "List containers");
}

#[test]
fn run_container_request_serializes_image_and_command() {
    let mut input = input("run_container");
    input.image = Some("docker.io/library/alpine:latest".to_string());
    input.command = vec!["sleep".to_string(), "30".to_string()];
    input.name = Some("alpine-test".to_string());
    let (req, _) = build_request(&input).unwrap();
    assert_eq!(req.method, "POST");
    assert!(req.path.contains("/containers/create"));
    let body: serde_json::Value = serde_json::from_str(&req.body).unwrap();
    assert_eq!(body["Image"], "docker.io/library/alpine:latest");
    assert_eq!(body["Cmd"], serde_json::json!(["sleep", "30"]));
}

#[test]
fn format_container_info_produces_attributed_summary() {
    let mut input = input("container_info");
    input.name = Some("my-container".to_string());
    let (req, _) = build_request(&input).unwrap();
    let body = r#"{"Id":"abc123","Name":"my-container","Image":"alpine:latest","State":{"Status":"running"},"Config":{"Cmd":["sleep","30"]}}"#;
    let result = format_response(&req, 200, body).unwrap();
    assert!(result.summary.contains("libpod plugin result"));
    assert!(result.summary.contains("running"));
    assert!(result.summary.contains("Source:"));
}

#[test]
fn format_error_response_contains_status_and_url() {
    let mut input = input("container_info");
    input.name = Some("my-container".to_string());
    let (req, _) = build_request(&input).unwrap();
    let body = r#"{"message":"no such container","cause":"not found","response":404}"#;
    let err = format_response(&req, 404, body).unwrap_err();
    assert!(err.contains("404"));
    assert!(err.contains("no such container"));
}
