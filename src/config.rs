use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use crate::error::LoopyError;
use crate::models::{ApprovalMode, ExecutionMode, TrackDefinition, builtin_track_definitions};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LoopyConfig {
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub knowledge_base: KnowledgeBase,
    #[serde(default)]
    pub mode: ExecutionMode,
    #[serde(default)]
    pub tracks: HashMap<String, TrackConfig>,
    #[serde(default)]
    pub track_definitions: HashMap<String, TrackDefinition>,
    #[serde(default)]
    pub ui: Option<UiConfig>,
    #[serde(default)]
    pub max_iterations: Option<u32>,
    #[serde(default)]
    pub backend: Option<String>,
    /// Argument template for a one-shot (non-interactive) agent invocation used
    /// by prompt enrichment and the LLM planner. `{prompt}` is replaced with the
    /// instruction. Defaults to a `claude -p "<prompt>"`-style call. Set this to
    /// match whatever agent CLI your `backend` points at.
    #[serde(default)]
    pub agent_oneshot_args: Option<Vec<String>>,
    /// The shell command agents should run to build + test the project (e.g.
    /// `cargo test`, `npm test`, `make check`). Woven into stage/track prompts so
    /// the agent verifies its work the way this repo actually builds. When unset,
    /// prompts fall back to a language-agnostic "build and test the project" hint.
    #[serde(default)]
    pub build_command: Option<String>,
    #[serde(default)]
    pub stages: HashMap<String, StageConfig>,
    #[serde(default)]
    pub context: Vec<ContextSource>,
    #[serde(default)]
    pub idea_doc: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KnowledgeBase {
    #[serde(default)]
    pub local: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub approval: ApprovalMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiConfig {
    #[serde(default)]
    pub theme: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StageConfig {
    #[serde(default)]
    pub max_iterations: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContextType {
    Directory,
    File,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextSource {
    #[serde(rename = "type")]
    pub source_type: ContextType,
    #[serde(alias = "url")]
    pub path: String,
    #[serde(default)]
    pub description: String,
}

fn default_true() -> bool {
    true
}

impl LoopyConfig {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let contents = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&contents)?)
    }

    pub fn load_or_default(dir: &Path) -> Self {
        let path = dir.join("loopy.yml");
        if path.exists()
            && let Some(cfg) = std::fs::read_to_string(&path)
                .ok()
                .and_then(|c| serde_yaml::from_str(&c).ok())
        {
            return cfg;
        }
        let json_path = dir.join("loopy.json");
        if json_path.exists() {
            Self::load(&json_path).unwrap_or_default()
        } else {
            Self::default()
        }
    }
}

pub fn load_track_definitions(
    dir: &Path,
    config: &LoopyConfig,
) -> BTreeMap<String, TrackDefinition> {
    let mut defs = builtin_track_definitions();

    // Merge loopy.yml track_definitions over built-ins
    for (key, def) in &config.track_definitions {
        defs.insert(key.clone(), def.clone());
    }

    // Scan .loopy/tracks/*.yml for additional definitions
    let tracks_dir = dir.join(".loopy").join("tracks");
    if let Ok(entries) = std::fs::read_dir(&tracks_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("yml")
                && let Ok(contents) = std::fs::read_to_string(&path)
                && let Ok(def) = serde_yaml::from_str::<TrackDefinition>(&contents)
            {
                let key = path.file_stem().unwrap().to_string_lossy().to_string();
                defs.insert(key, def);
            }
        }
    }

    // Apply tracks.<name> overrides (enabled, approval)
    for (key, tc) in &config.tracks {
        if let Some(def) = defs.get_mut(key) {
            def.enabled = tc.enabled;
            def.approval = tc.approval;
        }
    }

    defs
}

/// Layered credential resolution: env var → credentials.toml → error.
pub struct CredentialStore;

impl CredentialStore {
    /// Read a named token: env var `env_var_name` first, then the given key in
    /// `credentials.toml`. Returns `None` when neither is present.
    pub fn token_with_env(
        env_var_name: &str,
        key: &str,
        cred_path: &Path,
    ) -> Result<Option<String>, LoopyError> {
        if let Ok(val) = std::env::var(env_var_name) {
            return Ok(Some(val));
        }
        Self::token_from_path(key, cred_path)
    }

    /// Read a named token from a specific credentials.toml file (no env var check).
    pub fn token_from_path(key: &str, path: &Path) -> Result<Option<String>, LoopyError> {
        let Ok(contents) = std::fs::read_to_string(path) else {
            return Ok(None);
        };
        let table: toml::Table = toml::from_str(&contents).map_err(|e| {
            LoopyError::CredentialError(format!("failed to parse credentials.toml: {e}"))
        })?;
        Ok(table.get(key).and_then(|v| v.as_str()).map(|s| s.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Existing tests (must still pass) ---

    #[test]
    fn default_config_has_empty_kb() {
        let cfg = LoopyConfig::default();
        assert!(cfg.knowledge_base.local.is_empty());
    }

    #[test]
    fn deserialize_with_local_kb() {
        let json = r#"{"knowledge_base":{"local":["./docs"]}}"#;
        let cfg: LoopyConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.knowledge_base.local.len(), 1);
    }

    #[test]
    fn load_or_default_missing_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let cfg = LoopyConfig::load_or_default(dir.path());
        assert!(cfg.knowledge_base.local.is_empty());
    }

    // --- New v2 tests ---

    #[test]
    fn config_with_mode() {
        let yaml = "mode: safe\n";
        let cfg: LoopyConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.mode, ExecutionMode::Safe);

        let yaml = "mode: fast\n";
        let cfg: LoopyConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.mode, ExecutionMode::Fast);
    }

    #[test]
    fn config_with_tracks() {
        let yaml = r#"
tracks:
  security:
    enabled: true
    approval: required
  backend:
    enabled: true
"#;
        let cfg: LoopyConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.tracks["security"].approval, ApprovalMode::Required);
        assert!(cfg.tracks["security"].enabled);
        assert_eq!(cfg.tracks["backend"].approval, ApprovalMode::Auto);
        assert!(cfg.tracks["backend"].enabled);
    }

    #[test]
    fn config_defaults_backward_compat() {
        let yaml = "knowledge_base:\n  local:\n    - ./docs/\n";
        let cfg: LoopyConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.mode, ExecutionMode::Safe);
        assert!(cfg.tracks.is_empty());
        assert_eq!(cfg.knowledge_base.local.len(), 1);
    }

    #[test]
    fn config_full_v2_yaml() {
        let yaml = r#"
mode: fast
knowledge_base:
  local:
    - ./docs/
tracks:
  security:
    enabled: true
    approval: required
  infrastructure:
    enabled: false
"#;
        let cfg: LoopyConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.mode, ExecutionMode::Fast);
        assert_eq!(cfg.knowledge_base.local.len(), 1);
        assert_eq!(cfg.tracks["security"].approval, ApprovalMode::Required);
        assert!(!cfg.tracks["infrastructure"].enabled);
    }

    #[test]
    fn credential_store_token_from_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let cred_path = dir.path().join("credentials.toml");
        std::fs::write(&cred_path, "my_token = \"file-tok-456\"\n").unwrap();
        let result = CredentialStore::token_from_path("my_token", &cred_path);
        assert_eq!(result.unwrap(), Some("file-tok-456".to_string()));
    }

    #[test]
    fn credential_store_missing_token_is_ok() {
        let dir = tempfile::TempDir::new().unwrap();
        let cred_path = dir.path().join("credentials.toml");
        let result = CredentialStore::token_from_path("my_token", &cred_path);
        assert_eq!(result.unwrap(), None);
    }

    #[test]
    fn credential_store_token_env_var() {
        let var_name = "LOOPY_TOKEN_TEST_ENV_VAR";
        unsafe {
            std::env::set_var(var_name, "env-tok");
        }
        let dir = tempfile::TempDir::new().unwrap();
        let cred_path = dir.path().join("credentials.toml");
        let result = CredentialStore::token_with_env(var_name, "my_token", &cred_path);
        assert_eq!(result.unwrap(), Some("env-tok".to_string()));
        unsafe {
            std::env::remove_var(var_name);
        }
    }

    #[test]
    fn track_config_defaults() {
        let yaml = "{}";
        let tc: TrackConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(tc.enabled);
        assert_eq!(tc.approval, ApprovalMode::Auto);
    }

    #[test]
    fn config_with_ui_theme() {
        let yaml = "ui:\n  theme: light\n";
        let cfg: LoopyConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.ui.unwrap().theme, Some("light".into()));
    }

    #[test]
    fn config_without_ui_defaults_to_none() {
        let yaml = "{}";
        let cfg: LoopyConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(cfg.ui.is_none());
    }

    #[test]
    fn load_track_definitions_no_config_returns_builtins() {
        let dir = tempfile::TempDir::new().unwrap();
        let cfg = LoopyConfig::default();
        let defs = load_track_definitions(dir.path(), &cfg);
        assert_eq!(defs.len(), 7);
        assert!(defs.contains_key("backend"));
        assert!(defs.contains_key("security"));
        assert!(defs["backend"].enabled);
    }

    #[test]
    fn load_track_definitions_loopy_yml_override() {
        let dir = tempfile::TempDir::new().unwrap();
        let yaml = r#"
track_definitions:
  backend:
    name: "Custom Backend"
    icon: "🚀"
    description: "Overridden backend"
    hat: "custom-hat.yml"
"#;
        std::fs::write(dir.path().join("loopy.yml"), yaml).unwrap();
        let cfg = LoopyConfig::load_or_default(dir.path());
        let defs = load_track_definitions(dir.path(), &cfg);
        assert_eq!(defs["backend"].hat, "custom-hat.yml");
        assert_eq!(defs["backend"].name, "Custom Backend");
        // Other builtins still present
        assert_eq!(defs.len(), 7);
    }

    #[test]
    fn load_track_definitions_custom_track_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let tracks_dir = dir.path().join(".loopy").join("tracks");
        std::fs::create_dir_all(&tracks_dir).unwrap();
        let yaml = r#"
name: "Load Testing"
icon: "⚡"
description: "Performance testing"
"#;
        std::fs::write(tracks_dir.join("load-testing.yml"), yaml).unwrap();
        let cfg = LoopyConfig::default();
        let defs = load_track_definitions(dir.path(), &cfg);
        assert_eq!(defs.len(), 8);
        assert_eq!(defs["load-testing"].name, "Load Testing");
        assert_eq!(defs["load-testing"].hat, "builtin:code-assist"); // default
    }

    #[test]
    fn load_track_definitions_no_medium_defaults_to_brazil() {
        use crate::models::{MediumConfig, TrackMedium};
        let dir = tempfile::TempDir::new().unwrap();
        let tracks_dir = dir.path().join(".loopy").join("tracks");
        std::fs::create_dir_all(&tracks_dir).unwrap();
        let yaml = r#"
name: "Minimal"
icon: "📦"
description: "No medium specified"
"#;
        std::fs::write(tracks_dir.join("minimal.yml"), yaml).unwrap();
        let cfg = LoopyConfig::default();
        let defs = load_track_definitions(dir.path(), &cfg);
        let minimal = &defs["minimal"];
        assert_eq!(minimal.medium, TrackMedium::Brazil);
        assert!(
            matches!(&minimal.medium_config, MediumConfig::Brazil { packages, .. } if packages.is_empty())
        );
    }

    #[test]
    fn load_track_definitions_tracks_override_disables() {
        let dir = tempfile::TempDir::new().unwrap();
        let yaml = r#"
tracks:
  infrastructure:
    enabled: false
  security:
    approval: required
"#;
        std::fs::write(dir.path().join("loopy.yml"), yaml).unwrap();
        let cfg = LoopyConfig::load_or_default(dir.path());
        let defs = load_track_definitions(dir.path(), &cfg);
        assert!(!defs["infrastructure"].enabled);
        assert_eq!(defs["security"].approval, ApprovalMode::Required);
        // Others unchanged
        assert!(defs["backend"].enabled);
    }

    #[test]
    fn config_defaults_have_no_max_iterations_or_backend() {
        let cfg = LoopyConfig::default();
        assert!(cfg.max_iterations.is_none());
        assert!(cfg.backend.is_none());
        assert!(cfg.stages.is_empty());
    }

    #[test]
    fn config_deserialize_max_iterations_and_backend() {
        let yaml = r#"
max_iterations: 200
backend: "claude"
"#;
        let cfg: LoopyConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.max_iterations, Some(200));
        assert_eq!(cfg.backend.as_deref(), Some("claude"));
    }

    #[test]
    fn config_deserialize_stages_with_max_iterations() {
        let yaml = r#"
max_iterations: 200
stages:
  scan:
    max_iterations: 100
  plan:
    max_iterations: 50
"#;
        let cfg: LoopyConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.max_iterations, Some(200));
        assert_eq!(cfg.stages["scan"].max_iterations, Some(100));
        assert_eq!(cfg.stages["plan"].max_iterations, Some(50));
    }

    #[test]
    fn config_stages_default_empty() {
        let yaml = "max_iterations: 300\n";
        let cfg: LoopyConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(cfg.stages.is_empty());
    }

    #[test]
    fn track_definition_with_max_iterations() {
        let dir = tempfile::TempDir::new().unwrap();
        let tracks_dir = dir.path().join(".loopy").join("tracks");
        std::fs::create_dir_all(&tracks_dir).unwrap();
        let yaml = r#"
name: "Backend"
icon: "⚙️"
description: "Backend track"
max_iterations: 150
"#;
        std::fs::write(tracks_dir.join("backend-custom.yml"), yaml).unwrap();
        let cfg = LoopyConfig::default();
        let defs = load_track_definitions(dir.path(), &cfg);
        assert_eq!(defs["backend-custom"].max_iterations, Some(150));
    }

    #[test]
    fn track_definition_default_no_max_iterations() {
        let defs = builtin_track_definitions();
        assert!(defs["backend"].max_iterations.is_none());
    }

    #[test]
    fn config_deserialize_context_sources() {
        let yaml = r#"
context:
  - type: directory
    path: "src/MyService/"
    description: "Existing backend service"
  - type: file
    path: "docs/design.md"
    description: "Design doc"
"#;
        let cfg: LoopyConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.context.len(), 2);
        assert_eq!(cfg.context[0].source_type, ContextType::Directory);
        assert_eq!(cfg.context[0].path, "src/MyService/");
        assert_eq!(cfg.context[0].description, "Existing backend service");
        assert_eq!(cfg.context[1].source_type, ContextType::File);
    }

    #[test]
    fn config_default_has_empty_context() {
        let cfg = LoopyConfig::default();
        assert!(cfg.context.is_empty());
    }

    #[test]
    fn config_context_backward_compat() {
        let yaml = "max_iterations: 200\n";
        let cfg: LoopyConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(cfg.context.is_empty());
    }

    #[test]
    fn config_project_field_from_loopy_yml() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("loopy.yml"), "project: my-cool-project\n").unwrap();
        let cfg = LoopyConfig::load_or_default(dir.path());
        assert_eq!(cfg.project.as_deref(), Some("my-cool-project"));
    }

    #[test]
    fn config_project_field_defaults_to_none() {
        let cfg = LoopyConfig::default();
        assert!(cfg.project.is_none());
    }
}
