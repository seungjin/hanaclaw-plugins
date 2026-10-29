//! Pure logic, no wit-bindgen imports.
//!
//! Keeping this separate means `cargo test` works natively on the host without
//! a wasm toolchain. The component glue in `lib.rs` should stay too thin to be
//! wrong.

use serde::Deserialize;

/// Arguments the model sends for the `{{tool_name}}` tool.
#[derive(Debug, Default, Deserialize)]
pub struct ToolInput {
    // TODO: add the tool's arguments here, e.g. `pub text: String,`.
}
{%- if config_read %}

/// The plugin's own operator config section, after the host has validated it
/// against `[config_schema]` in `manifest.toml` and materialized typed JSON.
///
/// Every field must default: a withheld `config_read` grant injects `{}` and
/// the plugin then runs on these defaults.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolConfig {
    // TODO: add configurable settings, e.g.
    // `#[serde(default)] pub prefix: Option<String>,`.
}

impl ToolInput {
    /// Deserialize the injected `__config` object. The host already validated
    /// it against the schema; this only catches plugin/schema drift.
    pub fn with_config(self, raw: &serde_json::Value) -> Result<Self, String> {
        let cfg: ToolConfig = serde_json::from_value(raw.clone())
            .map_err(|e| format!("invalid config: {e}"))?;
        // TODO: store `cfg` on `self` once `ToolConfig` has fields.
        let _ = cfg;
        Ok(self)
    }
}
{%- endif %}

/// Run the tool. Replace this with the real logic.
pub fn run(input: &ToolInput) -> String {
    let _ = input;
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_model_arguments() {
        let input: ToolInput = serde_json::from_str("{}").unwrap();
        assert_eq!(run(&input), "");
    }
}
