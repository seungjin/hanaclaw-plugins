//! A ZeroClaw WIT tool plugin: `{{project-name}}`.
//!
//! {{tool_description}}
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
        features: ["plugins-wit-v0"],
    });

    use crate::plugin_core::{run, ToolInput};
    use exports::zeroclaw::plugin::plugin_info::Guest as PluginInfo;
    use exports::zeroclaw::plugin::tool::{Guest as Tool, ToolResult};
    use zeroclaw::plugin::logging::{
        log_record, LogLevel, PluginAction, PluginEvent, PluginOutcome,
    };

    struct {{crate_name | pascal_case}}Plugin;

{% if config_read %}
    /// The host merges this plugin's validated config object into the call
    /// arguments under `__config`, after deleting any model-supplied value of
    /// that name. Captured as a raw `Value` so a config-shaped problem is
    /// reported as a config error, not as "invalid arguments".
    #[derive(serde::Deserialize)]
    struct ExecuteArgs {
        #[serde(default)]
        #[serde(rename = "__config")]
        __config: serde_json::Value,
        // TODO: model the tool's arguments here, e.g. `text: String,`.
    }
{%- else %}
    #[derive(serde::Deserialize)]
    struct ExecuteArgs {
        // TODO: model the tool's arguments here, e.g. `text: String,`.
    }
{%- endif %}

    impl PluginInfo for {{crate_name | pascal_case}}Plugin {
        fn plugin_name() -> String {
            // Fixed at generation time; the host does not inject it.
            "{{project-name}}".to_string()
        }

        fn plugin_version() -> String {
            env!("CARGO_PKG_VERSION").to_string()
        }
    }

    impl Tool for {{crate_name | pascal_case}}Plugin {
        fn name() -> String {
            "{{tool_name}}".to_string()
        }

        fn description() -> String {
            // Fixed at generation time, so a static string is all the host sees.
            "{{tool_description}}".to_string()
        }

        // The model's entire view of the tool: valid JSON or the host refuses
        // to register the plugin. Never declare `__config` here.
        fn parameters_schema() -> String {
            serde_json::json!({
                "type": "object",
                "properties": {
                    // TODO: declare the tool's arguments here.
                    "_placeholder": {
                        "type": "string",
                        "description": "Replace with the real arguments."
                    }
                },
                "required": ["_placeholder"]
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

            // TODO: map `parsed` fields onto the input.
            let input = ToolInput::default();
{% if config_read %}
            let input = input.with_config(&parsed.__config)?;
{% else %}
            let _ = parsed;
{% endif %}
            let output = run(&input);

            log_record(
                LogLevel::Info,
                &PluginEvent {
                    function_name: "{{tool_name}}::tool::execute".to_string(),
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

    export!({{crate_name | pascal_case}}Plugin);
}
