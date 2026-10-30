//! Pure logic for the gh2qdrant ZeroClaw plugin.
//!
//! This mirrors the interface of the native `gh2qdrant` tool but runs inside
//! the ZeroClaw wasm sandbox. Network calls (GitHub API, Ollama, Qdrant) and
//! git operations must be mediated by the host through the WIT `sockets`
//! capability; this first version validates inputs and returns a structured
//! plan. Actual HTTP/git implementation is left as a follow-up once the host
//! socket API is wired in `lib.rs`.

use serde::{Deserialize, Serialize};

/// Arguments the model sends for the `gh2qdrant` tool.
#[derive(Debug, Default, Deserialize)]
pub struct ToolInput {
    /// GitHub repository URL. Supports `https://github.com/owner/repo` and
    /// `https://github.com/owner/repo/tree/main/subpath` forms.
    pub url: String,

    /// Target Qdrant collection. Defaults to `code_<repo>` when empty.
    #[serde(default)]
    pub collection: Option<String>,

    /// Label stored on every point as `repo`.
    #[serde(default)]
    pub repo_label: Option<String>,

    /// Stop after this many files.
    #[serde(default)]
    pub limit_files: Option<usize>,

    /// Chunks per embedding request.
    #[serde(default)]
    pub batch: Option<usize>,

    /// Skip `#[cfg(test)]` modules and test symbols unless true.
    #[serde(default)]
    pub keep_tests: bool,

    /// Validate and report the plan without performing network writes.
    #[serde(default)]
    pub dry: bool,
}

/// Operator config supplied by the host. A withheld `config_read` grant
/// injects `{}`, so every field must default.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolConfig {
    #[serde(default = "default_qdrant_url")]
    pub qdrant_url: String,

    #[serde(default = "default_qdrant_auth_header")]
    pub qdrant_auth_header: String,

    #[serde(default = "default_ollama_url")]
    pub ollama_url: String,

    #[serde(default = "default_embed_model")]
    pub embed_model: String,

    #[serde(default = "default_batch")]
    pub batch: usize,

    #[serde(default = "default_work_dir")]
    pub work_dir: String,
}

fn default_qdrant_url() -> String {
    "http://localhost:6333".to_string()
}

fn default_qdrant_auth_header() -> String {
    "api-key".to_string()
}

fn default_ollama_url() -> String {
    "http://localhost:11434".to_string()
}

fn default_embed_model() -> String {
    "bge-m3".to_string()
}

fn default_batch() -> usize {
    8
}

fn default_work_dir() -> String {
    "/tmp/gh2qdrant".to_string()
}

impl ToolInput {
    /// Merge per-call overrides with operator config.
    pub fn with_config(mut self, raw: &serde_json::Value) -> Result<Self, String> {
        let cfg: ToolConfig =
            serde_json::from_value(raw.clone()).map_err(|e| format!("invalid config: {e}"))?;
        if self.batch.is_none() {
            self.batch = Some(cfg.batch);
        }
        Ok(self)
    }

    /// Effective batch size.
    pub fn effective_batch(&self) -> usize {
        self.batch.unwrap_or(8)
    }

    /// Default collection name from the repo URL.
    pub fn default_collection(&self) -> Result<String, String> {
        let repo = repo_name(&self.url)?;
        Ok(format!("code_{repo}"))
    }

    /// Effective collection name.
    pub fn effective_collection(&self) -> Result<String, String> {
        self.collection
            .clone()
            .filter(|s| !s.is_empty())
            .map(Ok)
            .unwrap_or_else(|| self.default_collection())
    }

    /// Effective repo label.
    pub fn effective_repo_label(&self) -> Result<String, String> {
        self.repo_label
            .clone()
            .filter(|s| !s.is_empty())
            .map(Ok)
            .unwrap_or_else(|| repo_name(&self.url))
    }
}

/// Extract the repo name from a GitHub URL.
fn repo_name(url: &str) -> Result<String, String> {
    let trimmed = url.trim();
    if !trimmed.starts_with("https://github.com/") && !trimmed.starts_with("http://github.com/") {
        return Err(format!(
            "unsupported URL: {url} (expected https://github.com/...)"
        ));
    }
    let path = trimmed
        .trim_start_matches("https://github.com/")
        .trim_start_matches("http://github.com/")
        .trim_end_matches(".git");
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() < 2 {
        return Err(format!("URL does not contain owner/repo: {url}"));
    }
    let repo = parts[1];
    if repo.is_empty() {
        return Err(format!("empty repository name in URL: {url}"));
    }
    Ok(repo.to_string())
}

/// Structured summary returned to the model.
#[derive(Debug, Serialize)]
struct ResultSummary {
    status: &'static str,
    url: String,
    repo: String,
    collection: String,
    batch: usize,
    limit_files: Option<usize>,
    keep_tests: bool,
    dry: bool,
    note: &'static str,
}

/// Run the tool. Currently validates inputs and returns a plan; the actual
/// clone/chunk/embed/upsert pipeline requires host-mediated sockets.
pub fn run(input: &ToolInput) -> String {
    let repo = match input.effective_repo_label() {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "success": false, "error": e }).to_string(),
    };
    let collection = match input.effective_collection() {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "success": false, "error": e }).to_string(),
    };

    let summary = ResultSummary {
        status: if input.dry { "dry-run" } else { "planned" },
        url: input.url.clone(),
        repo,
        collection,
        batch: input.effective_batch(),
        limit_files: input.limit_files,
        keep_tests: input.keep_tests,
        dry: input.dry,
        note: "wasm sandbox: clone/chunk/embed/upsert require host-mediated sockets (not yet wired)",
    };

    serde_json::to_string_pretty(&summary).unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_model_arguments() {
        let input: ToolInput =
            serde_json::from_str(r#"{"url":"https://github.com/uuid-rs/uuid"}"#).unwrap();
        assert_eq!(input.effective_repo_label().unwrap(), "uuid");
        assert_eq!(input.effective_collection().unwrap(), "code_uuid");
    }

    #[test]
    fn overrides_collection() {
        let input: ToolInput = serde_json::from_str(
            r#"{"url":"https://github.com/uuid-rs/uuid","collection":"my_uuid"}"#,
        )
        .unwrap();
        assert_eq!(input.effective_collection().unwrap(), "my_uuid");
    }

    #[test]
    fn rejects_bad_url() {
        let input = ToolInput {
            url: "https://example.com/repo".to_string(),
            ..Default::default()
        };
        assert!(input.effective_repo_label().is_err());
    }

    #[test]
    fn batch_from_config() {
        let mut input: ToolInput =
            serde_json::from_str(r#"{"url":"https://github.com/uuid-rs/uuid"}"#).unwrap();
        input = input
            .with_config(&serde_json::json!({ "batch": 16 }))
            .unwrap();
        assert_eq!(input.effective_batch(), 16);
    }
}
