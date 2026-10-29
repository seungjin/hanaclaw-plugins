//! Pure logic, no wit-bindgen imports.
//!
//! Keeping this separate means `cargo test` works natively on the host without
//! a wasm toolchain. The component glue in `lib.rs` should stay too thin to be
//! wrong.

use serde::Deserialize;

/// Arguments the model sends for the `hello_from_hana` tool.
#[derive(Debug, Default, Deserialize)]
pub struct ToolInput {
    /// Who to greet. Optional: omitted or blank greets the whole world.
    #[serde(default)]
    pub name: Option<String>,
}

/// The greeting Hana returns, before any name is appended.
pub const GREETING: &str = "Hello from Hana!";

/// Run the tool: greet `name`, or everyone when it is absent or blank.
pub fn run(input: &ToolInput) -> String {
    let name = input
        .name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty());
    match name {
        Some(name) => format!("{GREETING} Nice to meet you, {name}."),
        None => GREETING.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greets_the_world_by_default() {
        assert_eq!(run(&ToolInput::default()), GREETING);
    }

    #[test]
    fn greets_a_named_person() {
        let input: ToolInput =
            serde_json::from_str(r#"{"name":"Seungjin"}"#).unwrap();
        assert_eq!(run(&input), "Hello from Hana! Nice to meet you, Seungjin.");
    }

    #[test]
    fn blank_name_falls_back_to_the_world() {
        let input: ToolInput =
            serde_json::from_str(r#"{"name":"   "}"#).unwrap();
        assert_eq!(run(&input), GREETING);
    }

    #[test]
    fn missing_name_field_is_accepted() {
        let input: ToolInput = serde_json::from_str("{}").unwrap();
        assert_eq!(run(&input), GREETING);
    }
}
