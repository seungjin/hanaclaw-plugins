//! Host-run integration test over the pure core: no wasm toolchain needed.

use gh2qdrant::core::{ToolInput, run};

#[test]
fn run_rejects_non_github_url() {
    let input = ToolInput {
        url: "https://example.com/repo".to_string(),
        ..Default::default()
    };
    let output = run(&input);
    assert!(output.contains("unsupported URL"));
}

#[test]
fn run_plans_github_url() {
    let input = ToolInput {
        url: "https://github.com/uuid-rs/uuid".to_string(),
        dry: true,
        ..Default::default()
    };
    let output = run(&input);
    assert!(output.contains("code_uuid"));
    assert!(output.contains("dry-run"));
}
