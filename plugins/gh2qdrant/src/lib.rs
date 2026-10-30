//! A ZeroClaw WIT tool plugin: `gh2qdrant`.
//!
//! Ingest a GitHub repository into a Qdrant collection.
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

    use crate::plugin_core::{ToolInput, run};
    use exports::zeroclaw::plugin::plugin_info::Guest as PluginInfo;
    use exports::zeroclaw::plugin::tool::{Guest as Tool, ToolResult};
    use zeroclaw::plugin::logging::{
        LogLevel, PluginAction, PluginEvent, PluginOutcome, log_record,
    };

    struct Gh2qdrantPlugin;

    /// The host merges this plugin's validated config object into the call
    /// arguments under `__config`, after deleting any model-supplied value of
    /// that name. Captured as a raw `Value` so a config-shaped problem is
    /// reported as a config error, not as "invalid arguments".
    #[derive(serde::Deserialize)]
    struct ExecuteArgs {
        #[serde(default)]
        #[serde(rename = "__config")]
        __config: serde_json::Value,
        /// GitHub repository URL.
        url: String,
        /// Optional Qdrant collection override.
        #[serde(default)]
        collection: Option<String>,
        /// Optional `repo` payload label override.
        #[serde(default)]
        repo_label: Option<String>,
        /// Stop after this many files.
        #[serde(default)]
        limit_files: Option<usize>,
        /// Chunks per embedding request.
        #[serde(default)]
        batch: Option<usize>,
        /// Keep test symbols.
        #[serde(default)]
        keep_tests: bool,
        /// Validate and report only.
        #[serde(default)]
        dry: bool,
    }

    impl From<ExecuteArgs> for ToolInput {
        fn from(args: ExecuteArgs) -> Self {
            ToolInput {
                url: args.url,
                collection: args.collection,
                repo_label: args.repo_label,
                limit_files: args.limit_files,
                batch: args.batch,
                keep_tests: args.keep_tests,
                dry: args.dry,
            }
        }
    }

    impl PluginInfo for Gh2qdrantPlugin {
        fn plugin_name() -> String {
            // Fixed at generation time; the host does not inject it.
            "gh2qdrant".to_string()
        }

        fn plugin_version() -> String {
            env!("CARGO_PKG_VERSION").to_string()
        }
    }

    impl Tool for Gh2qdrantPlugin {
        fn name() -> String {
            "gh2qdrant".to_string()
        }

        fn description() -> String {
            // Fixed at generation time, so a static string is all the host sees.
            "Clone a GitHub repo, chunk it with tree-sitter, embed with bge-m3, and upsert into Qdrant".to_string()
        }

        // The model's entire view of the tool: valid JSON or the host refuses
        // to register the plugin. Never declare `__config` here.
        fn parameters_schema() -> String {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "format": "uri",
                        "description": "GitHub repository URL, e.g. https://github.com/uuid-rs/uuid"
                    },
                    "collection": {
                        "type": "string",
                        "description": "Target Qdrant collection (default: code_<repo>)"
                    },
                    "repo_label": {
                        "type": "string",
                        "description": "Value stored as the repo payload field (default: repo name)"
                    },
                    "limit_files": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Stop after processing this many files"
                    },
                    "batch": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Chunks per embedding request"
                    },
                    "keep_tests": {
                        "type": "boolean",
                        "description": "Keep test modules and symbols"
                    },
                    "dry": {
                        "type": "boolean",
                        "description": "Validate and report the plan without writing"
                    }
                },
                "required": ["url"]
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

            let config = parsed.__config.clone();
            let input: ToolInput = parsed.into();
            let input = input.with_config(&config)?;
            let output = run(&input);

            log_record(
                LogLevel::Info,
                &PluginEvent {
                    function_name: "gh2qdrant::tool::execute".to_string(),
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

    export!(Gh2qdrantPlugin);
}
