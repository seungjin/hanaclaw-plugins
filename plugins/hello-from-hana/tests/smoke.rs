//! Host-run integration test over the pure core: no wasm toolchain needed.

use hello_from_hana::core::{GREETING, ToolInput, run};

#[test]
fn greets_a_named_person() {
    let input: ToolInput = serde_json::from_str(r#"{"name":"Seungjin"}"#).unwrap();
    assert!(run(&input).starts_with(GREETING));
    assert!(run(&input).contains("Seungjin"));
}

#[test]
fn greets_the_world_by_default() {
    assert_eq!(run(&ToolInput::default()), GREETING);
}
