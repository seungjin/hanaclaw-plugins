//! A ZeroClaw WIT tool plugin: `hello-from-hana`.
//!
//! Return a friendly greeting from Hana
//!
//! The pure logic lives in [`core`] with no wasm dependency, so `cargo test`
//! runs on the host; the wasm component reuses it through the thin shim below.
//!
//! Test:   cargo test                        (host; no wasm toolchain needed)
//! Build:  uv run --python 3.13 python tools/build_plugins.py

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
        // root. Plugins here live at `plugins/<name>/`, so the contract is two
        // levels up; keep that depth if you move a plugin elsewhere.
        path: "../../wit/v0",
        world: "tool-plugin",
        features: ["plugins-wit-v0"],
    });

    use crate::plugin_core::{ToolInput, run};
    use exports::zeroclaw::plugin::plugin_info::Guest as PluginInfo;
    use exports::zeroclaw::plugin::tool::{Guest as Tool, ToolResult};
    use zeroclaw::plugin::logging::{
        LogLevel, PluginAction, PluginEvent, PluginOutcome, log_record,
    };

    struct HelloFromHanaPlugin;

    #[derive(serde::Deserialize)]
    struct ExecuteArgs {
        /// Optional: the host passes whatever the model sent, and `null` is a
        /// legitimate value for an optional field.
        #[serde(default)]
        name: Option<String>,
    }

    impl PluginInfo for HelloFromHanaPlugin {
        fn plugin_name() -> String {
            // Fixed at generation time; the host does not inject it.
            "hello-from-hana".to_string()
        }

        fn plugin_version() -> String {
            env!("CARGO_PKG_VERSION").to_string()
        }
    }

    impl Tool for HelloFromHanaPlugin {
        fn name() -> String {
            "hello_from_hana".to_string()
        }

        fn description() -> String {
            // Fixed at generation time, so a static string is all the host sees.
            "Return a friendly greeting from Hana".to_string()
        }

        // The model's entire view of the tool: valid JSON or the host refuses
        // to register the plugin. Never declare `__config` here.
        fn parameters_schema() -> String {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Who to greet. Optional; omit to greet everyone."
                    }
                },
                "required": []
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

            let input = ToolInput { name: parsed.name };

            let output = run(&input);

            log_record(
                LogLevel::Info,
                &PluginEvent {
                    function_name: "hello_from_hana::tool::execute".to_string(),
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

    export!(HelloFromHanaPlugin);
}
