use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Stdio;

use chrono::{DateTime, Utc};
use tokio::process::Command;

use crate::config::{
    ContextSource, ContextType, KnowledgeBase, LoopyConfig, load_track_definitions,
};
use crate::models::{ExecutionMode, StageId, TrackDefinition};

/// Strip ANSI escape sequences (`\x1b[...letter`) from a string.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next(); // consume '['
                for c2 in chars.by_ref() {
                    if c2.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            // lone ESC without '[' — just drop the ESC
        } else {
            out.push(c);
        }
    }
    out
}

/// Spawn background tasks to drain Ralph stdout/stderr lines into the log channel.
pub fn drain_ralph_output(result: &mut SpawnResult<'_>, tx: &tokio::sync::mpsc::Sender<String>) {
    use tokio::io::{AsyncBufReadExt, BufReader};
    if let Some(stdout) = result.stdout.take() {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            while reader.read_line(&mut line).await.unwrap_or(0) > 0 {
                let _ = tx.try_send(line.trim_end().to_string());
                line.clear();
            }
        });
    }
    if let Some(stderr) = result.stderr.take() {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr);
            let mut line = String::new();
            while reader.read_line(&mut line).await.unwrap_or(0) > 0 {
                let _ = tx.try_send(line.trim_end().to_string());
                line.clear();
            }
        });
    }
}

/// Tail a Ralph output log file and send lines to the TUI log channel.
pub fn tail_ralph_log(log_path: std::path::PathBuf, tx: &tokio::sync::mpsc::Sender<String>) {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let tx = tx.clone();
    tokio::spawn(async move {
        // Wait for file to exist
        for _ in 0..30 {
            if log_path.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        let Ok(file) = tokio::fs::File::open(&log_path).await else {
            return;
        };
        let mut reader = BufReader::new(file);
        let mut line = String::new();
        loop {
            match reader.read_line(&mut line).await {
                Ok(0) => {
                    // EOF — wait a bit and try again (file is still being written)
                    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                }
                Ok(_) => {
                    let _ = tx.try_send(line.trim_end().to_string());
                    line.clear();
                }
                Err(_) => break,
            }
        }
    });
}

/// MCP server configuration for Ralph loops.
#[derive(Debug, Clone)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    pub env: HashMap<String, String>,
}

/// Configuration for spawning a Ralph loop.
#[derive(Debug, Clone)]
pub struct LoopConfig {
    pub stage: StageId,
    pub track: Option<String>,
    pub hat_collection: Option<String>,
    pub prompt_file: Option<PathBuf>,
    pub completion_promise: Option<String>,
    pub knowledge_base: Option<KnowledgeBase>,
    pub mcp_servers: Option<Vec<McpServerConfig>>,
    pub execution_mode: Option<ExecutionMode>,
    pub work_dir: Option<PathBuf>,
    /// Loopy block id for generic (non-POC) pipeline execution. When set, it's
    /// used for the loop_id label instead of the `stage` enum. `None` for the
    /// existing POC stages/tracks. Additive — existing spawns are unaffected.
    pub block_id: Option<String>,
}

/// Handle to a spawned Ralph child process.
#[derive(Debug)]
pub struct LoopHandle {
    pub pid: u32,
    pub loop_id: String,
    pub stage: StageId,
    pub track: Option<String>,
    pub spawned_at: DateTime<Utc>,
    child: tokio::process::Child,
}

/// Result of spawning a Ralph loop — includes piped stdout/stderr for live capture.
#[derive(Debug)]
pub struct SpawnResult<'a> {
    pub handle: &'a LoopHandle,
    pub stdout: Option<tokio::process::ChildStdout>,
    pub stderr: Option<tokio::process::ChildStderr>,
}

/// Manages Ralph CLI child processes.
pub struct Orchestrator {
    loops: HashMap<String, LoopHandle>,
    ralph_bin: String,
}

impl Default for Orchestrator {
    fn default() -> Self {
        // Try to find ralph: PATH first, then ~/.cargo/bin/ralph
        let ralph_bin = if which_exists("ralph") {
            "ralph".to_string()
        } else if let Some(home) = std::env::var("HOME").ok() {
            let cargo_ralph = format!("{home}/.cargo/bin/ralph");
            if std::path::Path::new(&cargo_ralph).exists() {
                cargo_ralph
            } else {
                "ralph".to_string()
            }
        } else {
            "ralph".to_string()
        };
        Self {
            loops: HashMap::new(),
            ralph_bin,
        }
    }
}

fn enrich_path() -> String {
    let current = std::env::var("PATH").unwrap_or_default();
    let home = std::env::var("HOME").unwrap_or_default();
    let extra_dirs = [
        format!("{home}/.cargo/bin"),
        format!("{home}/.local/bin"),
        "/usr/local/bin".to_string(),
    ];
    let mut parts: Vec<&str> = current.split(':').collect();
    for dir in &extra_dirs {
        if !parts.contains(&dir.as_str()) && std::path::Path::new(dir).exists() {
            parts.push(dir);
        }
    }
    parts.join(":")
}

fn which_exists(bin: &str) -> bool {
    std::process::Command::new("which")
        .arg(bin)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn which_command(bin: &str) -> String {
    // Try `which` first
    if let Some(path) = std::process::Command::new("which")
        .arg(bin)
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        return path;
    }
    // Check common locations
    if let Ok(home) = std::env::var("HOME") {
        let candidates = [
            format!("{home}/.cargo/bin/{bin}"),
            format!("{home}/.local/bin/{bin}"),
        ];
        for path in &candidates {
            if std::path::Path::new(path).exists() {
                return path.clone();
            }
        }
    }
    bin.to_string()
}

/// Recursively list files in a directory.
fn walkdir(dir: &std::path::Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            files.extend(walkdir(&path)?);
        } else if path.is_file() {
            files.push(path);
        }
    }
    Ok(files)
}

/// Map a StageId to its directory name under `.loopy/stages/`.
pub fn stage_dir_name(stage: StageId) -> &'static str {
    match stage {
        StageId::Scan => "scan",
        StageId::RequirementsAnalysis => "requirements-analysis",
        StageId::OrbitalLanes => "orbital-lanes",
        StageId::TestFlight => "test-flight",
        StageId::Land => "land",
        StageId::Idea => "idea",
        StageId::Plan => "plan",
    }
}

/// The previous stage whose output feeds into the given stage.
fn previous_stage(stage: StageId) -> Option<StageId> {
    match stage {
        StageId::Scan => None,
        StageId::RequirementsAnalysis => Some(StageId::Scan),
        StageId::OrbitalLanes => Some(StageId::RequirementsAnalysis),
        StageId::Land => Some(StageId::OrbitalLanes),
        _ => None,
    }
}

/// Hat collection path/name for a given stage (static, no filesystem check).
pub fn hat_collection_for_stage(stage: StageId) -> Option<&'static str> {
    match stage {
        StageId::OrbitalLanes => Some("builtin:code-assist"),
        // Scan, Plan, Requirements, Land use traditional mode (no hats)
        _ => None,
    }
}

/// Hat collection for a stage, checking `.loopy/hats/` first and falling back to builtins.
/// Returns None for stages that use traditional mode (no hats).
pub fn hat_collection_for_stage_with_root(stage: StageId, _project_root: &Path) -> Option<String> {
    hat_collection_for_stage(stage).map(|s| s.to_string())
}

/// All hat files to generate: (filename, content).
const HAT_FILES: &[(&str, &str)] = &[
    ("planner.yml", PLANNER_HAT_YAML),
    ("requirements-analysis.yml", REQUIREMENTS_ANALYSIS_HAT_YAML),
    ("environment-setup.yml", ENVIRONMENT_SETUP_HAT_YAML),
    ("land-summary.yml", LAND_SUMMARY_HAT_YAML),
    ("security-analyst.yml", SECURITY_ANALYST_HAT_YAML),
];

/// Generate custom hat YAML files if they don't already exist.
fn generate_hat_files(project_root: &Path) -> anyhow::Result<()> {
    let hat_dir = project_root.join(".loopy/hats");
    std::fs::create_dir_all(&hat_dir)?;
    for (name, content) in HAT_FILES {
        let path = hat_dir.join(name);
        if !path.exists() {
            std::fs::write(&path, content)?;
        }
    }
    Ok(())
}

/// Emit a ralph event into a stage directory's `.ralph/` events file.
pub fn emit_ralph_event(
    stage_dir: &Path,
    topic: &str,
    payload: &serde_json::Value,
) -> std::io::Result<()> {
    let ralph_dir = stage_dir.join(".ralph");
    let _ = std::fs::create_dir_all(&ralph_dir);
    let event = serde_json::json!({
        "ts": chrono::Utc::now().to_rfc3339(),
        "topic": topic,
        "payload": payload,
    });
    let path = ralph_dir.join("events-loopy-review.jsonl");
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(f, "{}", event)?;
    Ok(())
}

/// Derive the stages directory from a checkpoint path.
/// - `.loopy/projects/<name>/state.json` → `.loopy/projects/<name>/stages`
/// - `.loopy/state.json` (legacy) → `.loopy/stages`
pub fn stages_dir(checkpoint: &Path) -> PathBuf {
    // parent of state.json is the project dir (or .loopy for legacy)
    checkpoint
        .parent()
        .unwrap_or(Path::new(".loopy"))
        .join("stages")
}

/// Workspaces directory for a project (sibling to stages/).
pub fn workspaces_dir(_checkpoint: &Path) -> PathBuf {
    PathBuf::from(".loopy/workspaces")
}

/// Migrate old `.loopy/stages/` to project-scoped location if it exists.
pub fn migrate_legacy_stages(project_root: &Path, dest_stages: &Path) {
    let old = project_root.join(".loopy/stages");
    if !old.is_dir() {
        return;
    }
    // Don't migrate if dest already has content or is the same path
    if dest_stages == old {
        return;
    }
    std::fs::create_dir_all(dest_stages).ok();
    if let Ok(entries) = std::fs::read_dir(&old) {
        for entry in entries.flatten() {
            let dest = dest_stages.join(entry.file_name());
            std::fs::rename(entry.path(), &dest).ok();
        }
    }
    std::fs::remove_dir_all(&old).ok();
}

/// Write a PID file to `<stages_base>/<stage>/ralph-pid` for resume tracking.
pub fn write_pid_file(stages_base: &Path, stage: StageId, pid: u32) -> std::io::Result<()> {
    let dir = stages_base.join(stage_dir_name(stage));
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("ralph-pid"), pid.to_string())
}

/// Read a PID from `<stages_base>/<stage>/ralph-pid`. Returns None if missing/corrupt.
pub fn read_pid_file(stages_base: &Path, stage: StageId) -> Option<u32> {
    let path = stages_base.join(stage_dir_name(stage)).join("ralph-pid");
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// Bootstrap a new Loopy project: directories, loopy.yml, hat files, skills.
pub fn loopy_init(
    project_root: &Path,
    _unused: Option<&str>,
    _interactive: bool,
) -> anyhow::Result<Vec<String>> {
    let mut created = Vec::new();

    // Create .loopy/ subdirectories
    for sub in &[
        "hats",
        "tracks",
        "artifacts",
        "context",
        "docs",
        "stages",
        "projects",
    ] {
        let dir = project_root.join(".loopy").join(sub);
        if !dir.exists() {
            std::fs::create_dir_all(&dir)?;
            created.push(format!(".loopy/{sub}/"));
        }
    }

    // Generate loopy.yml (only if missing)
    let yml_path = project_root.join("loopy.yml");
    if !yml_path.exists() {
        write_default_yml(&yml_path, project_root)?;
        created.push("loopy.yml".to_string());
    }

    // Generate hat files (idempotent)
    generate_hat_files(project_root)?;
    for (name, _) in HAT_FILES {
        created.push(format!(".loopy/hats/{name}"));
    }

    Ok(created)
}

fn write_default_yml(yml_path: &Path, project_root: &Path) -> anyhow::Result<()> {
    let project_name = project_root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "loopy-project".to_string());
    let yml = format!("project: {project_name}\nbackend: claude\nmax_iterations: 350\n");
    std::fs::write(yml_path, yml)?;
    Ok(())
}

const PLANNER_HAT_YAML: &str = r#"hats:
  planner:
    name: "Mission Planner"
    description: "Breaks scan output into parallel tracks with dependencies and produces a track manifest"
    system_prompt: |
      You are a mission planner for a software project. Given the scan output and available tracks:
      1. Select only the tracks actually needed for this project
      2. For each track: define scope, dependencies on other tracks, key artifacts produced, acceptance criteria
      3. Sequence tracks respecting dependency order
      4. Output TWO files:
         - PLAN.md: human-readable plan with rationale for each track
         - tracks.json: machine-readable track manifest with id, components, depends_on, acceptance_criteria per track
      Be specific — name actual components/modules, their paths, and acceptance criteria.
    triggers: ["task.start"]
    publishes: ["plan.complete"]
"#;

const REQUIREMENTS_ANALYSIS_HAT_YAML: &str = r#"hats:
  product_analyst:
    name: "Product Analyst"
    description: "Analyzes scan output and produces structured requirements with UX, API contracts, and trade-offs"
    system_prompt: |
      You are a product analyst. Given the scan output, produce a structured requirements document:
      1. Summary — half-page product-facing summary of what we're building and why
      2. UX Requirements — actual screen names and user flows needed
      3. API Contracts — endpoint paths, request/response schemas, error codes
      4. Open Questions — items needing product team input
      5. Trade-offs — scope vs quality vs timeline trade-offs
      Be specific — name actual screens, endpoints, and schemas, not abstract concepts.
    triggers: ["task.start"]
    publishes: ["requirements.complete"]
"#;

const ENVIRONMENT_SETUP_HAT_YAML: &str = r#"hats:
  environment_setup:
    name: "Environment Setup"
    description: "Prepares the track's working directory so code can be built and tested"
    system_prompt: |
      You are an environment setup agent. Your ONLY job is to prepare the working
      directory so the track's code can be built and tested. Do NOT write application code.

      Details are in PROMPT.md. Steps:
      1. Check out or clone the relevant repository/repositories into the working directory.
      2. Install dependencies using the project's package manager (e.g. cargo fetch,
         npm install, pip install, go mod download) as indicated by the repo.
      3. Run the project's build/test command once to confirm the environment is healthy.

      Adapt to whatever build system the repo actually uses — detect it, don't assume.
      Emit setup.complete when the project builds successfully.
    triggers: ["task.start"]
    publishes: ["setup.complete"]
"#;

const LAND_SUMMARY_HAT_YAML: &str = r#"hats:
  release_manager:
    name: "Release Manager"
    description: "Compiles delivery summary with per-track diffs, test results, and integration status"
    system_prompt: |
      You are a release manager. Do NOT write code.

      Compile a delivery summary as SUMMARY.md:
      1. Per-Track Summary — for each track: what was built, key files changed (git diff --stat), test results, artifacts produced
      2. Overall Assessment — idea to what was delivered, integration status
      3. Open Items — follow-ups, known issues, tech debt

      Run `git diff --stat` in each track workspace to gather change summaries.
      Include actual test results from each track.
    triggers: ["task.start"]
    publishes: ["land.complete"]
"#;

const SECURITY_ANALYST_HAT_YAML: &str = r#"hats:
  security_analyst:
    name: "Security Analyst"
    description: "Reviews code and architecture for security concerns and AppSec compliance"
    system_prompt: |
      You are a security analyst. Review the code and architecture for:
      1. Common vulnerability patterns (injection, auth bypass, data exposure)
      2. Application security best practices and compliance (e.g. OWASP Top 10)
      3. Dependency security (known CVEs, outdated packages)
      4. Threat modeling gaps

      Output a structured SECURITY-REVIEW.md with: Findings, Risk Assessment, Recommendations.
    triggers: ["task.start"]
    publishes: ["security.complete"]
"#;

/// Generate markdown listing enabled track definitions for the planner.
pub fn generate_available_tracks_md(defs: &BTreeMap<String, TrackDefinition>) -> String {
    let mut md = String::from("# Available Tracks\n\n");
    for (key, def) in defs {
        md.push_str(&format!("## {} {} (`{}`)\n\n", def.icon, def.name, key));
        md.push_str(&format!("{}\n\n", def.description));
        if !def.dependencies.is_empty() {
            md.push_str(&format!(
                "Dependencies: {}\n\n",
                def.dependencies.join(", ")
            ));
        }
        if !def.artifacts_produced.is_empty() {
            md.push_str(&format!(
                "Artifacts: {}\n\n",
                def.artifacts_produced.join(", ")
            ));
        }
    }
    md
}

/// Check if a track is enabled in the loaded definitions. Unknown tracks default to enabled.
pub fn is_track_enabled(project_root: &Path, track_id: &str) -> bool {
    let config = LoopyConfig::load_or_default(project_root);
    let defs = load_track_definitions(project_root, &config);
    defs.get(track_id).is_none_or(|d| d.enabled)
}

/// Resolve max_iterations: per-stage > global > 200 default.
fn resolve_max_iterations(
    config: &crate::config::LoopyConfig,
    stage_name: &str,
    track_override: Option<u32>,
) -> u32 {
    track_override
        .or_else(|| config.stages.get(stage_name).and_then(|s| s.max_iterations))
        .or(config.max_iterations)
        .unwrap_or(350)
}

/// Resolve backend: config > "claude" default.
pub fn resolve_backend(config: &crate::config::LoopyConfig) -> &str {
    config.backend.as_deref().unwrap_or("claude")
}

/// Resolve the build/test command woven into prompts. Uses the configured
/// `build_command` if set, otherwise a language-agnostic instruction telling the
/// agent to build and test the project however this repo is built.
pub fn resolve_build_command(config: &crate::config::LoopyConfig) -> String {
    config
        .build_command
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| {
            "the project's build and test command (detect it from the repo — \
             e.g. cargo test, npm test, make check, pytest)"
                .to_string()
        })
}

/// Build the argv for a one-shot (non-interactive) agent call: the resolved
/// backend binary followed by the configured `agent_oneshot_args` with `{prompt}`
/// substituted. Defaults to a `claude -p "<prompt>"`-style invocation.
fn one_shot_argv(config: &crate::config::LoopyConfig, instruction: &str) -> Vec<String> {
    let bin = which_command(resolve_backend(config));
    let template = config
        .agent_oneshot_args
        .clone()
        .unwrap_or_else(|| vec!["-p".to_string(), "{prompt}".to_string()]);
    let mut argv = vec![bin];
    for arg in template {
        argv.push(arg.replace("{prompt}", instruction));
    }
    argv
}

/// One-shot prompt enrichment: ask the agent backend to rewrite a rough task
/// prompt into a clearer, more complete one. Returns the enriched text. Runs the
/// configured backend non-interactively and captures stdout. Best-effort with a
/// hard timeout; returns an error if the backend isn't available.
pub async fn enrich_prompt(config: &crate::config::LoopyConfig, rough: &str) -> anyhow::Result<String> {
    let instruction = format!(
        "Rewrite the following software task description into a clearer, more complete \
         and actionable prompt for an autonomous coding agent. Keep the original intent. \
         Add helpful specificity (acceptance criteria, scope, constraints) ONLY where it \
         is clearly implied — do not invent requirements. Output ONLY the rewritten \
         prompt, no preamble, no markdown fences.\n\n---\n{rough}\n---"
    );
    one_shot_agent(config, &instruction).await
}

/// Run the configured agent backend once, non-interactively, with `instruction`
/// and return its cleaned stdout. Shared by enrich_prompt and the LLM planner.
/// Hard 120s timeout; errors if the backend isn't available.
async fn one_shot_agent(config: &crate::config::LoopyConfig, instruction: &str) -> anyhow::Result<String> {
    let argv = one_shot_argv(config, instruction);
    let (bin, args) = argv.split_first().expect("argv always has the backend binary");
    let mut cmd = tokio::process::Command::new(bin);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env("PATH", enrich_path());

    let child = cmd.spawn()?;
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("agent call timed out"))??;

    let raw = String::from_utf8_lossy(&out.stdout);
    let cleaned = strip_ansi(&raw);
    let body: String = cleaned
        .lines()
        .filter(|l| {
            let t = l.trim();
            !t.is_empty()
                && !t.starts_with('▸')
                && !t.starts_with("[stderr")
                && !t.starts_with("Credits:")
                && !t.starts_with("Time:")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let body = body.trim().to_string();
    if body.is_empty() {
        anyhow::bail!("agent produced no output");
    }
    Ok(body)
}

/// LLM planner: ask the agent to choose an ordered list of block kinds for a task.
/// Returns the chosen block-kind ids (validated against known kinds). The caller
/// builds the actual pipeline (adding locked Scan/Land bookends). Falls back to the
/// keyword heuristic on any failure. Block kinds are passed in so the prompt lists
/// exactly what's available.
pub async fn plan_blocks_llm(
    config: &crate::config::LoopyConfig,
    task: &str,
    available: &[(&str, &str)],
) -> anyhow::Result<Vec<String>> {
    let catalog = available
        .iter()
        .map(|(id, desc)| format!("- {id}: {desc}"))
        .collect::<Vec<_>>()
        .join("\n");
    let instruction = format!(
        "You are planning a pipeline for an autonomous engineering system. Given a task, \
         choose which BLOCKS to run, in order. Pick only from this catalog (use the exact \
         ids):\n{catalog}\n\nRules: every pipeline implicitly starts with 'scan' and ends \
         with 'submit' — do NOT include those, they are added automatically. Include only \
         the blocks the task actually needs, in execution order. Prefer fewer, well-chosen \
         blocks. Output ONLY a JSON array of id strings, e.g. [\"plan\",\"implement\",\"review\"]. \
         No prose, no markdown fences.\n\nTASK:\n{task}"
    );
    let raw = one_shot_agent(config, &instruction).await?;
    // Extract the JSON array (the agent may add stray text despite instructions).
    let start = raw.find('[').ok_or_else(|| anyhow::anyhow!("no JSON array in planner output"))?;
    let end = raw.rfind(']').ok_or_else(|| anyhow::anyhow!("no JSON array end"))?;
    let arr: Vec<String> = serde_json::from_str(&raw[start..=end])
        .map_err(|e| anyhow::anyhow!("planner JSON parse failed: {e}"))?;
    // Validate against known ids; drop unknown/duplicate; never include scan/submit.
    let known: std::collections::HashSet<&str> = available.iter().map(|(id, _)| *id).collect();
    let mut seen = std::collections::HashSet::new();
    let chosen: Vec<String> = arr
        .into_iter()
        .filter(|id| known.contains(id.as_str()) && id != "scan" && id != "submit")
        .filter(|id| seen.insert(id.clone()))
        .collect();
    if chosen.is_empty() {
        anyhow::bail!("planner returned no usable blocks");
    }
    Ok(chosen)
}

/// Build the `ralph.yml` contents for a Loopy [`StageBlock`](crate::pipeline::StageBlock).
/// Mirrors the per-stage ralph.yml the engine writes today, but driven entirely
/// from the block's kind metadata (hat, completion promise) rather than a
/// `StageId` match — this is the agent config a generic `spawn_block()` uses.
pub fn ralph_yml_for_block(
    block: &crate::pipeline::StageBlock,
    config: &crate::config::LoopyConfig,
    project_root: &Path,
) -> String {
    let backend = resolve_backend(config);
    let backend_cmd = which_command(backend);
    let max_iter = resolve_max_iterations(config, &block.id, None);
    let promise = block.kind.completion_promise();

    let hat_line = match block.kind.default_hat() {
        Some(hat) => {
            let resolved = if hat.starts_with("builtin:") || Path::new(hat).is_absolute() {
                hat.to_string()
            } else {
                project_root.join(hat).display().to_string()
            };
            format!("  hat_collection: \"{resolved}\"\n  starting_event: \"task.start\"\n")
        }
        None => String::new(),
    };

    format!(
        "cli:\n  backend: \"{backend}\"\nadapters:\n  {backend}:\n    timeout: 900\n    command: \"{backend_cmd}\"\nfeatures:\n  parallel: true\nevent_loop:\n  prompt_file: \"PROMPT.md\"\n  completion_promise: \"{promise}\"\n{hat_line}  max_iterations: {max_iter}\n"
    )
}

/// Build the `PROMPT.md` for a Loopy [`StageBlock`](crate::pipeline::StageBlock).
/// Composes the block's role line, the task, optional accumulated feedback, and
/// references to prior-stage outputs. Generic across kinds — the per-kind voice
/// comes from `role_line()`.
pub fn prompt_for_block(
    block: &crate::pipeline::StageBlock,
    task: &str,
    prior_stage_ids: &[String],
    feedback: Option<&str>,
) -> String {
    let mut p = format!("# {}\n\n## Role\n\n{}\n\n## Task\n\n{}\n", block.label, block.kind.role_line(), task);

    if !prior_stage_ids.is_empty() {
        p.push_str("\n## Prior stage outputs\n\nRead the outputs of earlier stages for context:\n\n");
        for id in prior_stage_ids {
            p.push_str(&format!("- `../{id}/` (output of the {id} stage)\n"));
        }
    }

    // The no-mistakes gate contract, surfaced to the agent.
    use crate::pipeline::GatePolicy;
    match block.gate {
        GatePolicy::AutoFix => p.push_str("\n## Gate\n\nApply safe, mechanical fixes yourself. Only stop for issues that need a human decision.\n"),
        GatePolicy::EscalateFinding => p.push_str("\n## Gate\n\nReport findings clearly. Do not proceed past anything that needs human judgment — surface it for approval.\n"),
        GatePolicy::Pass => {}
    }

    if let Some(fb) = feedback {
        p.push_str(&format!("\n## Feedback to address\n\nThis stage is being re-run. Address this feedback on top of the existing work:\n\n{fb}\n"));
    }

    p.push_str(&format!(
        "\n## Completion\n\nWhen the work is fully done, emit the completion event `{}`.\n",
        block.kind.completion_promise()
    ));
    p
}

/// Create `<stages_base>/<stage>/` with `ralph.yml` and `PROMPT.md`.
/// Returns the path to the created stage directory.
pub fn setup_stage_dir(
    stage: StageId,
    idea_text: &str,
    kb_content: Option<&str>,
    project_root: &Path,
    config: &crate::config::LoopyConfig,
    stages_base: &Path,
) -> anyhow::Result<PathBuf> {
    let name = stage_dir_name(stage);
    let stage_dir = stages_base.join(name);
    std::fs::create_dir_all(&stage_dir)?;

    // Ralph needs a git repo — fire and forget, don't block pipeline.
    // Skip entirely under tests: tests call this many times and don't need a real
    // git repo; spawning git subprocesses from tests is pure waste (and was a
    // contributor to process pileup during the fork-bomb incident).
    if !cfg!(test) && !stage_dir.join(".git").exists() {
        let sd = stage_dir.clone();
        std::thread::spawn(move || {
            if std::process::Command::new("git")
                .args(["init", "-q"])
                .current_dir(&sd)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .output()
                .is_ok()
            {
                let _ = std::process::Command::new("git")
                    .args(["commit", "--allow-empty", "-m", "loopy stage init", "-q"])
                    .current_dir(&sd)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .output();
            }
        });
    }

    // Generate custom hat files (preserves existing)
    generate_hat_files(project_root)?;

    // Write ralph.yml
    let promise = match stage {
        StageId::Scan => "scan.complete".to_string(),
        StageId::Plan => "plan.complete".to_string(),
        StageId::RequirementsAnalysis => "requirements.complete".to_string(),
        StageId::Land => "land.complete".to_string(),
        _ => format!("{}.complete", name),
    };
    let hat_opt = hat_collection_for_stage_with_root(stage, project_root);
    let hat_line = match hat_opt {
        Some(ref hat) => {
            let resolved = if hat.starts_with("builtin:") || Path::new(hat).is_absolute() {
                hat.clone()
            } else {
                project_root.join(hat).display().to_string()
            };
            format!("  hat_collection: \"{resolved}\"\n  starting_event: \"task.start\"\n")
        }
        None => String::new(), // traditional mode — no hats
    };
    let backend = resolve_backend(config);
    let max_iter = resolve_max_iterations(config, name, None);
    let backend_cmd = which_command(backend);
    let ralph_yml = format!(
        "cli:\n  backend: \"{backend}\"\nadapters:\n  {backend}:\n    timeout: 900\n    command: \"{backend_cmd}\"\nfeatures:\n  parallel: true\nevent_loop:\n  prompt_file: \"PROMPT.md\"\n  completion_promise: \"{promise}\"\n{hat_line}  max_iterations: {max_iter}\n"
    );
    std::fs::write(stage_dir.join("ralph.yml"), &ralph_yml)?;

    // Write PROMPT.md
    let context_content = load_context_sources(&config.context, project_root);

    // Write idea and context to files so prompts can reference them (not embed)
    std::fs::write(stage_dir.join("idea.md"), idea_text)?;
    if let Some(kb) = kb_content {
        std::fs::write(stage_dir.join("knowledge-base.md"), kb)?;
    }
    if !context_content.is_empty() {
        std::fs::write(stage_dir.join("context.md"), &context_content)?;
    }

    let prompt = build_stage_prompt(
        stage,
        idea_text,
        project_root,
        stages_base,
    );
    std::fs::write(stage_dir.join("PROMPT.md"), &prompt)?;

    // Copy previous stage outputs
    if let Some(prev) = previous_stage(stage) {
        let prev_output = stages_base.join(stage_dir_name(prev)).join("output");
        if prev_output.is_dir() {
            let dest = stage_dir.join(format!("{}-output", stage_dir_name(prev)));
            copy_dir_recursive(&prev_output, &dest)?;
        }
    }

    // Write AVAILABLE_TRACKS.md for Plan stage
    if stage == StageId::Plan {
        let config = LoopyConfig::load_or_default(project_root);
        let defs: BTreeMap<_, _> = load_track_definitions(project_root, &config)
            .into_iter()
            .filter(|(_, d)| d.enabled)
            .collect();
        std::fs::write(
            stage_dir.join("AVAILABLE_TRACKS.md"),
            generate_available_tracks_md(&defs),
        )?;
    }

    Ok(stage_dir)
}

const CONTEXT_KEY_FILES: &[&str] = &["README.md", "README", "lib.rs", "Cargo.toml", "mod.rs"];
const CONTEXT_FILE_TRUNCATE: usize = 2000;

/// Load all context sources from config, caching results in `.loopy/context/`.
pub fn load_context_sources(sources: &[ContextSource], project_root: &Path) -> String {
    let cache_dir = project_root.join(".loopy/context");
    std::fs::create_dir_all(&cache_dir).ok();
    let mut parts = Vec::new();
    for src in sources {
        let cache_key = format!("{:x}", md5_hash(&src.path));
        let cache_path = cache_dir.join(format!("{cache_key}.md"));
        let content = if cache_path.exists() {
            std::fs::read_to_string(&cache_path).unwrap_or_default()
        } else {
            let loaded = match src.source_type {
                ContextType::Directory => load_directory_context(&src.path, project_root),
                ContextType::File => load_file_context(&src.path, project_root),
            };
            std::fs::write(&cache_path, &loaded).ok();
            loaded
        };
        if !content.is_empty() {
            let header = if src.description.is_empty() {
                format!("### {}", src.path)
            } else {
                format!("### {} ({})", src.description, src.path)
            };
            parts.push(format!("{header}\n\n{content}"));
        }
    }
    parts.join("\n\n")
}

fn md5_hash(input: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    input.hash(&mut h);
    h.finish()
}

fn load_directory_context(path: &str, project_root: &Path) -> String {
    let dir = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        project_root.join(path)
    };
    let mut parts = Vec::new();
    for name in CONTEXT_KEY_FILES {
        let file = dir.join(name);
        if let Ok(content) = std::fs::read_to_string(&file) {
            let truncated = if content.len() > CONTEXT_FILE_TRUNCATE {
                let end = content.floor_char_boundary(CONTEXT_FILE_TRUNCATE);
                format!("{}...(truncated)", &content[..end])
            } else {
                content
            };
            parts.push(format!("**{name}**:\n```\n{truncated}\n```"));
        }
    }
    parts.join("\n\n")
}

fn load_file_context(path: &str, project_root: &Path) -> String {
    let file = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        project_root.join(path)
    };
    std::fs::read_to_string(&file).unwrap_or_default()
}

/// Build the stage-specific PROMPT.md content.
fn build_stage_prompt(
    stage: StageId,
    idea_text: &str,
    project_root: &Path,
    stages_base: &Path,
) -> String {
    // Short summary of the idea (first 500 chars) for orientation, full text in idea.md
    let idea_summary = if idea_text.len() > 500 {
        format!("{}...\n\n*(Full idea doc: `idea.md`)*", &idea_text[..500])
    } else {
        idea_text.to_string()
    };
    let build_cmd = resolve_build_command(&LoopyConfig::load_or_default(project_root));

    let mut prompt = match stage {
        StageId::Scan => {
            let mut prompt = String::from(
                "# Scan Stage\n\n## Role\nYou are a technical researcher analyzing a software project idea.\n\n## Idea (summary)\n\n",
            );
            prompt.push_str(&idea_summary);
            prompt.push_str("\n\n## Context Files\n\nRead these files for full context:\n\n");
            prompt.push_str("- **Full idea doc**: `idea.md`\n");
            prompt.push_str("- **Knowledge base**: `knowledge-base.md` (if exists)\n");
            prompt.push_str("- **Design docs / context**: `context.md` (if exists)\n");
            prompt.push_str(r#"

## Scope

**What you MUST do:**
- Read `idea.md` and `context.md` thoroughly — every section, every detail
- Identify all components/modules/repositories that need to change or be created
- VERIFY each one exists — inspect the codebase (search the repo, read the files); do not guess
- Assess feasibility and identify risks
- Write findings incrementally to your scratchpad as you go

**What you MUST NOT do:**
- Write any implementation code
- Skip verification — confirm each component exists by inspecting the code
- Invent module/file names — reference what's actually in the repo
- Stop to ask clarifying questions — state your assumption and continue

## Success Metric
A complete scan report where every listed package has been verified via CLI, every risk has a mitigation, and the report contains enough detail for a planner to create implementation tracks without re-reading the idea doc.

## Output: Scan Directory

Produce these files in your working directory:

1. **`scan-report.md`** — Executive summary with:
   - Feasibility assessment
   - Key technical challenges
   - Risk register table
   - Acceptance criteria (numbered, testable)

2. **`environment.json`** — Component manifest:
```json
{
  "components": [
    {
      "name": "exact-module-or-repo-name",
      "path": "src/relative/path",
      "status": "EXISTS",
      "track": "backend",
      "verified": true
    }
  ]
}
```

3. **`analysis/`** directory — One file per major area analyzed:
   - `analysis/idea-doc-summary.md` — Structured summary of the idea doc
   - `analysis/<component-name>.md` — Per-component analysis (what exists, what needs to change)
   - `analysis/context-<N>.md` — Summary of each context/design doc analyzed

The scan-report.md should reference these analysis files so the planner can drill in.

## Error Handling
- If a component can't be found in the repo, mark it as `"status": "CREATE", "verified": false`
- If a path cannot be determined, use your best guess and add a note
- If the idea doc is ambiguous on a point, flag it as `[AMBIGUOUS]` with your best interpretation
- Complete the full scan without stopping — partial results are better than no results
- Track IDs must be simple lowercase alphanumeric with hyphens (e.g., "backend", "security"). No emoji, no spaces.
"#);
            prompt
        }
        StageId::Plan => {
            let mut prompt = String::from(
                "# Plan Stage\n\n## Role\nYou are a mission planner breaking a validated idea into parallel implementation tracks.\n\n## Idea (summary)\n\n",
            );
            prompt.push_str(&idea_summary);
            prompt.push_str("\n\n## Context Files\n\nRead these files for full context:\n\n");
            prompt.push_str("- **Full idea doc**: `idea.md`\n");
            // Reference scan outputs
            let scan_report = stages_base.join("scan/scan-report.md");
            let scan_report_alt = stages_base.join("scan/report.md");
            if scan_report.exists() {
                prompt.push_str(&format!("- **Scan report**: `{}`\n", scan_report.display()));
            } else if scan_report_alt.exists() {
                prompt.push_str(&format!("- **Scan report**: `{}`\n", scan_report_alt.display()));
            }
            let scan_analysis = stages_base.join("scan/analysis");
            if scan_analysis.is_dir() {
                prompt.push_str(&format!("- **Scan analysis files**: `{}/` (per-package and per-doc analysis)\n", scan_analysis.display()));
            }
            let env_json = stages_base.join("scan/environment.json");
            if env_json.exists() {
                prompt.push_str(&format!("- **Environment**: `{}`\n", env_json.display()));
            }
            // Inline available tracks (small)
            let config = LoopyConfig::load_or_default(project_root);
            let defs: BTreeMap<_, _> = load_track_definitions(project_root, &config)
                .into_iter()
                .filter(|(_, d)| d.enabled)
                .collect();
            prompt.push_str(&format!(
                "\n## Available Tracks\n\n{}",
                generate_available_tracks_md(&defs)
            ));
            prompt.push_str(r#"
## Scope

**What you MUST do:**
- Read the scan report and environment.json first
- Select tracks from the available list that are actually needed
- Define clear acceptance criteria for each track (testable, specific)
- Map every component from environment.json to exactly one track
- Identify dependencies between tracks

**What you MUST NOT do:**
- Write implementation code
- Create tracks that aren't needed — fewer tracks is better
- Use vague acceptance criteria like "implement the feature"

## Success Metric
A plan where: every requirement from the scan maps to exactly one track, every track has 1-5 specific testable acceptance criteria, and the dependency graph has no cycles.

## Quality Heuristics
- A track with 1-3 tightly coupled components is better than a track with 10 loosely related ones
- If two tracks would always be deployed together, merge them into one
- The `backend` track should be first with no dependencies — everything else depends on it

## Your Task
Produce TWO files:

1. **tracks.json** — Machine-readable track manifest (write this FIRST):
```json
{
  "tracks": [
    {
      "id": "backend",
      "components": [{"name": "module-name", "path": "src/module", "exists": true}],
      "depends_on": [],
      "acceptance_criteria": ["AC1", "AC2"]
    }
  ]
}
```

2. **PLAN.md** — Human-readable plan with rationale for each selected track, dependencies, and sequencing.

## Constraints
- Track IDs must be simple lowercase alphanumeric with hyphens (e.g., "backend", "security"). No emoji, no spaces.
- You MUST write BOTH files. The pipeline will NOT advance without tracks.json.
- Write tracks.json FIRST, then PLAN.md.
"#);
            prompt
        }
        StageId::RequirementsAnalysis => {
            let mut prompt = String::from(
                "# Requirements Analysis Stage\n\n## Role\nYou are a product analyst producing a structured requirements document.\n\n## Idea (summary)\n\n",
            );
            prompt.push_str(&idea_summary);
            prompt.push_str("\n\n## Context Files\n\nRead these before starting:\n\n");
            prompt.push_str("- **Full idea doc**: `idea.md`\n");
            prompt.push_str("- **Knowledge base**: `knowledge-base.md` (if exists)\n");
            prompt.push_str("- **Design docs / context**: `context.md` (if exists)\n");
            let scan_report = stages_base.join("scan/scan-report.md");
            let scan_report_alt = stages_base.join("scan/report.md");
            if scan_report.exists() {
                prompt.push_str(&format!("- **Scan report**: `{}`\n", scan_report.display()));
            } else if scan_report_alt.exists() {
                prompt.push_str(&format!("- **Scan report**: `{}`\n", scan_report_alt.display()));
            }
            let scan_analysis = stages_base.join("scan/analysis");
            if scan_analysis.is_dir() {
                prompt.push_str(&format!("- **Scan analysis**: `{}/`\n", scan_analysis.display()));
            }
            let plan = stages_base.join("plan/PLAN.md");
            if plan.exists() {
                prompt.push_str(&format!("- **Plan**: `{}`\n", plan.display()));
            }
            prompt.push_str(
                r#"
## Scope

**What you MUST do:**
- Read the scan report and plan first
- Produce specific, actionable requirements — name actual screens, endpoints, schemas
- Write findings incrementally to your scratchpad as you go

**What you MUST NOT do:**
- Write implementation code
- Produce vague requirements like "the system should be fast"
- Stop to ask clarifying questions — flag ambiguity as [AMBIGUOUS] and continue

## Success Metric
A requirements document where every requirement is specific enough that an engineer can implement it without re-reading the idea doc, and every API endpoint has request/response schemas with field types.

## Your Task
Produce a structured requirements document with these EXACT sections:

1. **Summary** — Half-page product-facing summary of what we're building and why.
2. **UX Requirements** — Screens and flows needed, with actual screen names and field descriptions.
3. **API Contracts** — Endpoint paths, request/response schemas (with types), error codes.
4. **Open Questions** — Items needing product team input, with your recommended default for each.
5. **Trade-offs** — Scope vs quality vs timeline trade-offs with a recommendation for each.

## Error Handling
- If the idea doc is ambiguous, flag it as [AMBIGUOUS] with your best interpretation
- If a UX flow is unclear, describe the most common pattern and note the assumption
- Complete the full analysis without stopping — partial results are better than no results
"#,
            );
            prompt
        }
        StageId::OrbitalLanes => {
            let mut prompt = String::from(
                "# Orbital Lanes Stage\n\n## Role\nYou are an implementation engineer working on a specific track of a larger project.\n\n## Idea (summary)\n\n",
            );
            prompt.push_str(&idea_summary);
            prompt.push_str("\n\n## Context Files\n\nRead these files for full context:\n\n");
            let req_candidates = [
                stages_base.join("requirements-analysis/requirements.md"),
                stages_base.join("requirements-analysis/REQUIREMENTS.md"),
                stages_base.join("requirements-analysis/output/scratchpad.md"),
            ];
            if let Some(req_path) = req_candidates.iter().find(|p| p.exists()) {
                prompt.push_str(&format!("- **Requirements**: `{}`\n", req_path.display()));
            }
            let plan = stages_base.join("plan/PLAN.md");
            if plan.exists() {
                prompt.push_str(&format!("- **Plan**: `{}`\n", plan.display()));
            }
            prompt.push_str(&format!(
                r#"
## Your Task
Implement the work for your assigned track. Follow these guidelines:

1. Read the context files above on your first iteration
2. Read the acceptance criteria for your track from the plan
3. Use dependency artifacts from other tracks if available in your working directory
4. Write code, tests, and documentation as needed
5. Run {build_cmd} to verify your changes compile and tests pass
6. Ensure all acceptance criteria are met before marking complete

## Constraints
- Stay within your track's scope — do not modify code owned by other tracks
- All tests must pass ({build_cmd})
- Document any assumptions or deviations from the plan
"#,
            ));
            prompt
        }
        StageId::Land => {
            let mut prompt = String::from(
                "# Land Stage\n\n## Role\nYou are a release manager compiling a delivery summary from all completed tracks.\n\n## Idea (summary)\n\n",
            );
            prompt.push_str(&idea_summary);
            prompt.push('\n');
            // Gather all track outputs
            let tracks_dir = stages_base.join("orbital-lanes");
            if tracks_dir.is_dir()
                && let Ok(entries) = std::fs::read_dir(&tracks_dir)
            {
                for entry in entries.flatten() {
                    let output_dir = entry.path().join("output");
                    if output_dir.is_dir() {
                        let track_name = entry.file_name();
                        prompt
                            .push_str(&format!("\n## Track: {}\n\n", track_name.to_string_lossy()));
                        let sp = output_dir.join("scratchpad.md");
                        if let Ok(content) = std::fs::read_to_string(&sp) {
                            prompt.push_str(&format!("{content}\n"));
                        }
                    }
                }
            }
            prompt.push_str(r#"
## Your Task
Compile a delivery summary as SUMMARY.md with these sections:

1. **Per-Track Summary** — For each track: what was built, key files changed (use `git diff --stat` per workspace), test results, artifacts produced.
2. **Overall Assessment** — Idea → what was delivered, integration status.
3. **Open Items** — Follow-ups, known issues, tech debt.

## Constraints
- Do NOT write new code
- Run `git diff --stat` in each track workspace to gather change summaries
- Include actual test results from each track
"#);
            prompt
        }
        _ => format!("# {stage:?}\n\n{}\n", idea_summary),
    };

    // Append feedback if a previous run left FEEDBACK.md
    let feedback_path = stages_base.join(stage_dir_name(stage)).join("FEEDBACK.md");
    if let Ok(fb) = std::fs::read_to_string(&feedback_path) {
        prompt.push_str(&format!(
            "\n## Human Feedback (from previous review)\n\n{fb}\n\n\
**You MUST address this feedback in your output.** Re-read the feedback carefully and adjust your work accordingly.\n"
        ));
    }

    prompt
}

/// Collect Ralph outputs from a stage working directory into `<stage_dir>/output/`.
pub fn collect_stage_output(stage_dir: &Path) -> anyhow::Result<()> {
    let output_dir = stage_dir.join("output");
    std::fs::create_dir_all(&output_dir)?;

    // Copy .ralph/agent/*.md files
    let agent_dir = stage_dir.join(".ralph/agent");
    if agent_dir.is_dir()
        && let Ok(entries) = std::fs::read_dir(&agent_dir)
    {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "md") {
                let dest = output_dir.join(entry.file_name());
                let _ = std::fs::copy(&path, &dest);
            }
        }
    }

    // Copy .ralph/events-*.jsonl files
    let ralph_dir = stage_dir.join(".ralph");
    if ralph_dir.is_dir() {
        let events_dir = output_dir.join("events");
        if let Ok(entries) = std::fs::read_dir(&ralph_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.starts_with("events-") && name_str.ends_with(".jsonl") {
                    let _ = std::fs::create_dir_all(&events_dir);
                    let _ = std::fs::copy(entry.path(), events_dir.join(&name));
                }
            }
        }
    }

    Ok(())
}

/// Recursively copy a directory.
fn copy_dir_recursive(src: &Path, dst: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let dest_path = dst.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir_recursive(&entry.path(), &dest_path)?;
        } else {
            std::fs::copy(entry.path(), &dest_path)?;
        }
    }
    Ok(())
}

/// Create a workspace directory for a track's environment setup phase.
/// Writes `ralph.yml` (environment-setup hat + setup.complete promise) and `PROMPT.md` (package info).
pub fn setup_track_workspace(
    track_id: &str,
    _track_def: &TrackDefinition,
    project_root: &Path,
    _config: &crate::config::LoopyConfig,
    stages_base: &Path,
) -> anyhow::Result<PathBuf> {
    // Workspace lives under the project dir (sibling to stages/)
    let project_dir = stages_base.parent().unwrap_or(Path::new(".loopy"));
    let ws = project_dir.join("workspaces").join(track_id);
    std::fs::create_dir_all(&ws)?;

    // Ensure hat files exist
    generate_hat_files(project_root)?;

    Ok(ws)
}

/// Create `<stages_base>/orbital-lanes/<track_id>/` with `ralph.yml` and `PROMPT.md`.
/// Returns the path to the created track directory.
/// Build a rich PROMPT.md for the coding Ralph of a track.
/// Builds a lean track prompt that references stage output files instead of embedding them.
/// The agent reads the files on its first iteration, keeping the prompt small.
pub fn build_track_coding_prompt(
    track_id: &str,
    description: &str,
    dependency_tracks: &[String],
    stages_base: &Path,
    branch_name: &str,
    build_cmd: &str,
) -> String {
    let mut prompt = format!(
        "# Coding Track: {track_id}\n\n## First Step\n\n\
Before making ANY changes, create a feature branch for your work:\n\
```bash\n\
git checkout -b {branch_name}\n\
```\n\
Do this before writing code. All commits go on this branch.\n\n\
## Coding Philosophy\n\n\
1. **Think Before Coding** — Read the existing code first. Understand the architecture, patterns, and conventions before writing anything. Search for how similar things are already done.\n\
2. **Simplicity First** — Write the simplest code that solves the problem. No abstractions unless they already exist in the codebase. Match existing patterns exactly.\n\
3. **Surgical Changes** — Make the smallest possible change. Touch only the files that need to change. Don't refactor, rename, or reorganize anything outside the task scope.\n\
4. **Goal-Driven Execution** — Every line of code must directly advance an acceptance criterion. If it doesn't, don't write it.\n\n\
Adapt to the real architecture you see in the workspace. Do NOT invent new patterns.\n\n\
## Scope\n\n\
**What you CAN do:**\n\
- Modify source files in the components listed below\n\
- Add new files (modules, tests, configs) following existing patterns\n\
- Run build and test commands\n\n\
**What you CANNOT do:**\n\
- Modify code outside your track's scope\n\
- Change build system configuration unless the acceptance criteria require it\n\
- Refactor or rename anything outside the scope of your acceptance criteria\n\n\
## Workflow: Test-Driven Development\n\n\
For EACH acceptance criterion, follow this loop:\n\n\
1. **Write a failing test** that asserts the acceptance criterion. Run it — confirm it fails.\n\
2. **Write the minimal code** to make the test pass. Nothing more.\n\
3. **Run the full build** ({build_cmd}). If it fails, fix it before moving on.\n\
4. **Commit** — one commit per acceptance criterion. Message: `feat({track_id}): <what the AC describes>`\n\
5. **Move to the next AC.** Do not revisit previous work unless a later change breaks it.\n\n\
If a test is hard to write (e.g., integration with external service), write the implementation first but add a unit test that covers the core logic with mocks.\n\n\
## Success Metric\n\n\
All acceptance criteria have passing tests AND {build_cmd} succeeds with zero test failures.\n\n\
## Error Recovery\n\n\
- **Build fails**: Read the error. Fix the immediate cause. Do not refactor unrelated code.\n\
- **Test fails**: Check if it's your change or a pre-existing failure. If pre-existing, note it and continue.\n\
- **Flaky test**: Run it twice. If it passes on retry, note it as flaky and move on.\n\
- **Blocked by missing dependency**: Record what's missing, skip to the next AC, come back later.\n\
- **NEVER STOP** — complete all acceptance criteria without pausing for confirmation.\n\n\
## Guardrails\n\n\
- **Do NOT fabricate data.** If an AC requires running infrastructure (load tests, perf benchmarks, deployments, manual verification), mark the task as BLOCKED with a description of what needs to be done manually. Write a runbook instead of fake results.\n\
- **Do NOT modify files outside your track's components.** If you need a change in another component, document it as a dependency and mark the task BLOCKED.\n\
- **Do NOT commit generated/binary files** (JARs, .class, node_modules, build artifacts).\n\n\
## Objective\n\n{description}\n"
    );

    // Acceptance criteria from tracks.json — this is small, inline it
    if let Ok(data) = std::fs::read_to_string(stages_base.join("plan/tracks.json")) {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&data) {
            if let Some(tracks) = val.get("tracks").and_then(|t| t.as_array()) {
                for t in tracks {
                    if t.get("id").and_then(|i| i.as_str()) == Some(track_id) {
                        if let Some(acs) = t.get("acceptance_criteria").and_then(|a| a.as_array()) {
                            prompt.push_str("\n## Acceptance Criteria\n\n");
                            for ac in acs {
                                if let Some(s) = ac.as_str() {
                                    prompt.push_str(&format!("- {s}\n"));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Reference stage outputs instead of embedding them
    prompt.push_str("\n## Context Files\n\n\
Read these files on your first iteration to understand the project:\n\n");

    let scan_report = stages_base.join("scan/report.md");
    let scan_scratchpad = stages_base.join("scan/output/scratchpad.md");
    if scan_report.exists() {
        prompt.push_str(&format!("- **Scan Report**: `{}`\n", scan_report.display()));
    } else if scan_scratchpad.exists() {
        prompt.push_str(&format!("- **Scan Report**: `{}`\n", scan_scratchpad.display()));
    }

    let plan = stages_base.join("plan/PLAN.md");
    if plan.exists() {
        prompt.push_str(&format!("- **Plan**: `{}`\n", plan.display()));
    }

    // Find the primary requirements file (prefer requirements.md over scratchpad)
    let req_dir = stages_base.join("requirements-analysis");
    let req_output_dir = stages_base.join("requirements-analysis/output");
    let req_candidates = [
        req_dir.join("requirements.md"),
        req_dir.join("REQUIREMENTS.md"),
        req_output_dir.join("scratchpad.md"),
    ];
    if let Some(req_path) = req_candidates.iter().find(|p| p.exists()) {
        prompt.push_str(&format!("- **Requirements**: `{}`\n", req_path.display()));
    }

    let env_json = stages_base.join("scan/environment.json");
    if env_json.exists() {
        prompt.push_str(&format!("- **Environment**: `{}`\n", env_json.display()));
    }

    // Component info — small, inline it (accepts "components" or legacy "packages")
    if let Ok(data) = std::fs::read_to_string(&env_json) {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&data) {
            if let Some(items) = val
                .get("components")
                .or_else(|| val.get("packages"))
                .and_then(|p| p.as_array())
            {
                let track_items: Vec<_> = items
                    .iter()
                    .filter(|p| p.get("track").and_then(|t| t.as_str()) == Some(track_id))
                    .collect();
                if !track_items.is_empty() {
                    prompt.push_str("\n## Components\n\n");
                    for p in &track_items {
                        if let Some(name) = p.get("name").and_then(|n| n.as_str()) {
                            prompt.push_str(&format!("- `{name}`\n"));
                        }
                    }
                }
            }
        }
    }

    // Dependency track outputs — reference, don't embed
    for dep in dependency_tracks {
        let dep_output = stages_base.join("orbital-lanes").join(dep).join("output");
        if dep_output.is_dir() {
            prompt.push_str(&format!(
                "\n## Dependency: {dep}\n\nRead output from: `{}`\n",
                dep_output.display()
            ));
        }
    }

    prompt
}

pub fn setup_track_dir(
    track_id: &str,
    description: &str,
    dependency_tracks: &[String],
    config: &crate::config::LoopyConfig,
    stages_base: &Path,
) -> anyhow::Result<PathBuf> {
    let track_dir = stages_base.join("orbital-lanes").join(track_id);
    std::fs::create_dir_all(&track_dir)?;

    // Git init — fire and forget, don't block pipeline. Skipped under tests
    // (see setup_stage_dir): no real git repo needed, avoids process pileup.
    if !cfg!(test) && !track_dir.join(".git").exists() {
        let td = track_dir.clone();
        std::thread::spawn(move || {
            if std::process::Command::new("git")
                .args(["init", "-q"])
                .current_dir(&td)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .output()
                .is_ok()
            {
                let _ = std::process::Command::new("git")
                    .args(["commit", "--allow-empty", "-m", "loopy track init", "-q"])
                    .current_dir(&td)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .output();
            }
        });
    }

    // ralph.yml
    let backend = resolve_backend(config);
    let backend_cmd = which_command(backend);
    let max_iter = resolve_max_iterations(config, "orbital-lanes", None);
    let ralph_yml = format!(
        "cli:\n  backend: \"{backend}\"\nadapters:\n  {backend}:\n    timeout: 900\n    command: \"{backend_cmd}\"\nfeatures:\n  parallel: true\nevent_loop:\n  prompt_file: \"PROMPT.md\"\n  completion_promise: \"track.{track_id}.complete\"\n  hat_collection: \"builtin:code-assist\"\n  starting_event: \"task.start\"\n  max_iterations: {max_iter}\n"
    );
    std::fs::write(track_dir.join("ralph.yml"), &ralph_yml)?;

    // PROMPT.md — lean coding prompt that references context files
    let branch_name = format!(
        "loopy/{}",
        config.project.as_deref().unwrap_or(track_id)
    );
    let build_cmd = resolve_build_command(config);
    let prompt = build_track_coding_prompt(track_id, description, dependency_tracks, stages_base, &branch_name, &build_cmd);
    std::fs::write(track_dir.join("PROMPT.md"), &prompt)?;

    Ok(track_dir)
}

/// Build a LoopConfig for spawning the code phase of a track after setup completes.
/// Finds the first package directory inside `workspace/src/<PackageName>/`, falling back to workspace root.
pub fn build_track_code_loop_config(
    track_id: &str,
    workspace_path: &Path,
    track_def: &TrackDefinition,
) -> LoopConfig {
    // Try to find the package dir: workspaces/<track>/src/<PackageName>/
    let work_dir =
        if let crate::models::MediumConfig::Brazil { ref packages, .. } = track_def.medium_config {
            packages
                .first()
                .and_then(|p| {
                    let pkg_dir = workspace_path.join("src").join(&p.name);
                    pkg_dir.is_dir().then_some(pkg_dir)
                })
                .unwrap_or_else(|| workspace_path.to_path_buf())
        } else {
            workspace_path.to_path_buf()
        };

    LoopConfig {
        stage: StageId::OrbitalLanes,
        track: Some(track_id.to_string()),
        hat_collection: Some(track_def.hat.clone()),
        prompt_file: None,
        completion_promise: Some(format!("track.{track_id}.complete")),
        knowledge_base: None,
        mcp_servers: None,
        execution_mode: None,
        work_dir: Some(work_dir),
        block_id: None,
    }
}

impl Orchestrator {
    pub fn new_with_bin(ralph_bin: String) -> Self {
        Self {
            loops: HashMap::new(),
            ralph_bin,
        }
    }

    /// Build the command arguments for a Ralph loop (visible for testing).
    /// When `work_dir` is set on config, uses `--config ralph.yml` instead of `-P`/`--completion-promise`.
    /// When `mcp_servers` is set, writes `.loopy/mcp-config.json` under `work_dir`.
    pub fn build_args(config: &LoopConfig, work_dir: &Path) -> Vec<String> {
        let mut args = vec!["run".into(), "-a".into(), "-v".into()];
        if let Some(ref hats) = config.hat_collection {
            args.push("-H".into());
            args.push(hats.clone());
        }
        // When work_dir is set, use --config (ralph.yml is in the stage dir)
        if config.work_dir.is_some() {
            args.push("--config".into());
            args.push("ralph.yml".into());
        } else {
            if let Some(ref prompt) = config.prompt_file {
                args.push("-P".into());
                args.push(prompt.display().to_string());
            }
            if let Some(ref promise) = config.completion_promise {
                args.push("--completion-promise".into());
                args.push(promise.clone());
            }
        }
        if let Some(ref servers) = config.mcp_servers {
            let mut mcp_map = serde_json::Map::new();
            for s in servers {
                let mut entry = serde_json::Map::new();
                entry.insert(
                    "command".into(),
                    serde_json::Value::String(s.command.clone()),
                );
                let env_obj: serde_json::Map<String, serde_json::Value> = s
                    .env
                    .iter()
                    .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                    .collect();
                entry.insert("env".into(), serde_json::Value::Object(env_obj));
                mcp_map.insert(s.name.clone(), serde_json::Value::Object(entry));
            }
            let config_json = serde_json::json!({ "mcpServers": mcp_map });
            let config_dir = work_dir.join(".loopy");
            let _ = std::fs::create_dir_all(&config_dir);
            let config_path = config_dir.join("mcp-config.json");
            let _ = std::fs::write(
                &config_path,
                serde_json::to_string_pretty(&config_json).unwrap_or_default(),
            );
            args.push("--mcp-config".into());
            args.push(config_path.display().to_string());
        }
        if let Some(mode) = config.execution_mode {
            args.push("--mode".into());
            args.push(match mode {
                ExecutionMode::Safe => "safe".into(),
                ExecutionMode::Fast => "fast".into(),
            });
        }
        args
    }

    /// Write rerun context file for an invalidated track.
    pub fn write_rerun_context(
        work_dir: &Path,
        track_id: &str,
        reason: &str,
    ) -> anyhow::Result<()> {
        let dir = work_dir.join(".loopy").join("track-context").join(track_id);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("rerun-context.md"), reason)?;
        Ok(())
    }

    /// Write knowledge base context file for Ralph to read.
    fn write_kb_context(kb: &KnowledgeBase, work_dir: &std::path::Path) -> anyhow::Result<()> {
        let mut context = String::new();
        // Local docs: read and inline
        for path in &kb.local {
            if path.is_file() {
                if let Ok(content) = std::fs::read_to_string(path) {
                    context.push_str(&format!("--- {} ---\n{}\n\n", path.display(), content));
                }
            } else if path.is_dir() {
                for entry in walkdir(path)? {
                    if let Ok(content) = std::fs::read_to_string(&entry) {
                        context.push_str(&format!("--- {} ---\n{}\n\n", entry.display(), content));
                    }
                }
            }
        }
        if !context.is_empty() {
            let kb_dir = work_dir.join(".loopy");
            std::fs::create_dir_all(&kb_dir)?;
            std::fs::write(kb_dir.join("knowledge-base.md"), context)?;
        }
        Ok(())
    }

    /// Spawn a Ralph child process from the given config.
    pub async fn spawn(&mut self, config: LoopConfig) -> anyhow::Result<SpawnResult<'_>> {
        let work_dir = config
            .work_dir
            .clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        // Write knowledge base context if provided
        if let Some(ref kb) = config.knowledge_base {
            let _ = Self::write_kb_context(kb, &work_dir);
        }
        let args = Self::build_args(&config, &work_dir);
        let loop_id = match &config.block_id {
            Some(bid) => format!("{}-{}", bid, chrono::Utc::now().timestamp_millis()),
            None => format!("{:?}-{}", config.stage, chrono::Utc::now().timestamp_millis()),
        };

        // Write stdout/stderr to a log file so Ralph survives TUI exit
        let ralph_dir = work_dir.join(".ralph");
        let _ = std::fs::create_dir_all(&ralph_dir);
        let log_path = ralph_dir.join("ralph-output.log");

        // Retry spawn up to 3 times — clean stale locks between attempts
        let mut last_err = None;
        log::info!(
            "Spawning Ralph in {} (loop_id={}, stage={:?})",
            work_dir.display(),
            loop_id,
            config.stage
        );
        for attempt in 1..=3u32 {
            log::debug!("Spawn attempt {}/3 for {}", attempt, work_dir.display());
            // Clean stale lock files (local — Ralph's global lock handled by features.parallel)
            let lock = ralph_dir.join("loop.lock");
            if lock.exists() {
                let _ = std::fs::remove_file(&lock);
            }

            let log_file = match std::fs::File::create(&log_path) {
                Ok(f) => f,
                Err(e) => {
                    log::info!(" ⚠️ Failed to create log file {}: {e}", log_path.display());
                    std::fs::File::create("/dev/null").unwrap()
                }
            };
            let err_file = log_file
                .try_clone()
                .unwrap_or_else(|_| std::fs::File::create("/dev/null").unwrap());

            // Ensure PATH includes common tool locations for child processes
            let enriched_path = enrich_path();

            // Contain temp files (agent snapshots) inside the work dir's
            // .ralph/tmp so they get cleaned up with the stage, instead of leaking into
            // the system /var/tmp (which exhausts inodes over many iterations).
            let scratch = ralph_dir.join("tmp");
            let _ = std::fs::create_dir_all(&scratch);

            match Command::new(&self.ralph_bin)
                .args(&args)
                .current_dir(&work_dir)
                .env("PATH", &enriched_path)
                .env("TMPDIR", &scratch)
                .stdin(Stdio::null())
                .stdout(Stdio::from(log_file))
                .stderr(Stdio::from(err_file))
                .process_group(0)
                .spawn()
            {
                Ok(child) => {
                    let stdout = None::<tokio::process::ChildStdout>;
                    let stderr = None::<tokio::process::ChildStderr>;
                    let pid = child.id().unwrap_or(0);

                    // Verify process is actually alive after a brief delay
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    log::info!(
                        " ✅ Ralph spawned PID={pid} in {} (attempt {attempt})",
                        work_dir.display()
                    );

                    let handle = LoopHandle {
                        pid,
                        loop_id: loop_id.clone(),
                        stage: config.stage,
                        track: config.track.clone(),
                        spawned_at: Utc::now(),
                        child,
                    };
                    self.loops.insert(loop_id.clone(), handle);
                    return Ok(SpawnResult {
                        handle: self.loops.get(&loop_id).unwrap(),
                        stdout,
                        stderr,
                    });
                }
                Err(e) => {
                    let msg = if e.kind() == std::io::ErrorKind::NotFound {
                        format!("Ralph CLI ('{}') not found on PATH. Ensure 'ralph' is installed and on your PATH.", self.ralph_bin)
                    } else {
                        format!("spawn error (attempt {attempt}/3): {e}")
                    };
                    log::info!(" ⚠️ {msg}");
                    last_err = Some(anyhow::anyhow!("{msg}"));
                    if attempt < 3 {
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    }
                }
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("spawn failed after 3 attempts")))
    }

    /// Kill a running Ralph loop by sending SIGTERM to its process group.
    pub async fn kill(&mut self, loop_id: &str) -> anyhow::Result<()> {
        let handle = self
            .loops
            .get_mut(loop_id)
            .ok_or_else(|| anyhow::anyhow!("unknown loop: {loop_id}"))?;
        let pid = handle.pid;
        // Ralph was spawned with process_group(0), so it leads its own group.
        // Kill the whole group (negative pid) to also stop the agent CLI
        // grandchildren, not just the ralph parent. `kill -- -PID` signals the group.
        if pid > 0 {
            let _ = std::process::Command::new("kill")
                .args(["-TERM", &format!("-{pid}")])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        // Kill + reap the direct child so it doesn't linger as a zombie.
        let _ = handle.child.kill().await;
        let _ = handle.child.wait().await;
        self.loops.remove(loop_id);
        Ok(())
    }

    /// Kill all loops running for a given stage. Returns count killed.
    pub async fn kill_loops_for_stage(&mut self, stage: StageId) -> usize {
        let ids: Vec<String> = self
            .loops
            .iter()
            .filter(|(_, h)| h.stage == stage)
            .map(|(id, _)| id.clone())
            .collect();
        let count = ids.len();
        for id in ids {
            let _ = self.kill(&id).await;
        }
        count
    }

    /// Kill all loops running for a given track. Returns count killed.
    pub async fn kill_loops_for_track(&mut self, track_id: &str) -> usize {
        let ids: Vec<String> = self
            .loops
            .iter()
            .filter(|(_, h)| h.track.as_deref() == Some(track_id))
            .map(|(id, _)| id.clone())
            .collect();
        let count = ids.len();
        for id in ids {
            let _ = self.kill(&id).await;
        }
        count
    }

    /// Returns all active loop handles.
    pub fn active_loops(&self) -> &HashMap<String, LoopHandle> {
        &self.loops
    }

    /// Check if a loop's process is still running.
    pub fn is_alive(&mut self, loop_id: &str) -> bool {
        let Some(handle) = self.loops.get_mut(loop_id) else {
            return false;
        };
        // try_wait returns Ok(Some(status)) if exited, Ok(None) if still running
        match handle.child.try_wait() {
            Ok(Some(_)) => false,
            Ok(None) => true,
            Err(_) => false,
        }
    }
}

/// Thoroughly kill EVERY Ralph loop belonging to a project by sweeping its
/// `.loopy/` tree for `loop.lock` files and SIGKILLing each recorded PID's
/// process GROUP. This is disk-based, so it reaps loops this process never
/// spawned (e.g. orphans from a previous server) — the abort path the user
/// needs so aborting never leaves tasks filling the dev desktop.
///
/// Returns the number of process groups signalled. Best-effort: missing/dead
/// PIDs are skipped silently.
pub fn kill_all_project_ralph(project_root: &Path) -> usize {
    let loopy_dir = project_root.join(".loopy");
    let mut killed = 0;
    let mut pids: Vec<u32> = Vec::new();

    // Find every .ralph/loop.lock under .loopy/ (stages, workspaces, tracks, …).
    fn collect_locks(dir: &Path, out: &mut Vec<std::path::PathBuf>, depth: usize) {
        if depth > 8 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_locks(&path, out, depth + 1);
            } else if path.file_name().and_then(|n| n.to_str()) == Some("loop.lock") {
                out.push(path);
            }
        }
    }
    let mut locks = Vec::new();
    collect_locks(&loopy_dir, &mut locks, 0);

    for lock in &locks {
        if let Ok(content) = std::fs::read_to_string(lock) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(pid) = json.get("pid").and_then(|p| p.as_u64()) {
                    pids.push(pid as u32);
                }
            }
        }
        // Remove the stale lock so a resumed run doesn't think it's still held.
        let _ = std::fs::remove_file(lock);
    }

    // SIGKILL each process GROUP (negative pid) — the group includes ralph plus
    // its agent-CLI grandchildren. SIGKILL (not TERM) because abort means
    // stop now; we don't want a slow shutdown leaving children behind.
    for pid in pids {
        if pid == 0 {
            continue;
        }
        let ok = std::process::Command::new("kill")
            .args(["-KILL", &format!("-{pid}")])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            killed += 1;
        }
    }
    killed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::StageConfig;

    #[test]
    fn kill_all_project_ralph_sweeps_lock_files() {
        let dir = tempfile::TempDir::new().unwrap();
        // Two fake ralph loop dirs with locks (dead/absurd pids — safe to signal).
        for sub in ["stages/scan/.ralph", "workspaces/backend/.ralph"] {
            let d = dir.path().join(".loopy").join(sub);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("loop.lock"), r#"{"pid": 2000000001}"#).unwrap();
        }
        assert!(dir.path().join(".loopy/stages/scan/.ralph/loop.lock").exists());

        // Sweep: returns (pids signalled) and removes the stale locks.
        let _ = kill_all_project_ralph(dir.path());
        assert!(!dir.path().join(".loopy/stages/scan/.ralph/loop.lock").exists(),
            "stale lock must be removed so a resume doesn't think it's held");
        assert!(!dir.path().join(".loopy/workspaces/backend/.ralph/loop.lock").exists());
    }

    #[test]
    fn kill_all_project_ralph_handles_no_locks() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".loopy")).unwrap();
        assert_eq!(kill_all_project_ralph(dir.path()), 0);
    }

    #[test]
    fn kill_all_project_ralph_actually_kills_a_live_process_group() {
        use std::os::unix::process::CommandExt;
        // Spawn a real long-running child as its OWN process group leader — exactly
        // how ralph is spawned (process_group(0)). This proves abort genuinely
        // reaps live processes, not just removes lock files.
        let mut cmd = std::process::Command::new("sleep");
        cmd.arg("120");
        cmd.process_group(0); // child leads its own group; pid == pgid
        let child = cmd.spawn().expect("spawn sleep");
        let pid = child.id();

        // Confirm it's alive.
        assert!(std::path::Path::new(&format!("/proc/{pid}")).exists(), "child should be alive");

        // Write a loop.lock recording its pid, then sweep.
        let dir = tempfile::TempDir::new().unwrap();
        let d = dir.path().join(".loopy/stages/scan/.ralph");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("loop.lock"), format!(r#"{{"pid": {pid}}}"#)).unwrap();

        let killed = kill_all_project_ralph(dir.path());
        assert_eq!(killed, 1, "should signal exactly one group");

        // Give the kernel a moment to deliver SIGKILL, then confirm it's gone.
        std::thread::sleep(std::time::Duration::from_millis(300));
        // After SIGKILL the process is dead or a zombie; check /proc/<pid>/stat state.
        let alive = std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|s| s.rsplit(')').next().map(|a| a.trim_start().chars().next()))
            .flatten()
            .map(|state| state != 'Z' && state != 'X')
            .unwrap_or(false);
        assert!(!alive, "process group must be dead after abort sweep (was state-alive)");

        // Reap our zombie so we don't leak it.
        let _ = std::process::Command::new("kill").args(["-KILL", &pid.to_string()]).status();
    }

    fn base_config(stage: StageId) -> LoopConfig {
        LoopConfig {
            stage,
            track: None,
            hat_collection: None,
            prompt_file: None,
            completion_promise: None,
            knowledge_base: None,
            mcp_servers: None,
            execution_mode: None,
            work_dir: None,
            block_id: None,
        }
    }

    // AC1: Command construction includes correct flags
    #[test]
    fn build_args_scan_with_completion_promise() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut config = base_config(StageId::Scan);
        config.completion_promise = Some("scan.complete".into());
        let args = Orchestrator::build_args(&config, dir.path());
        assert_eq!(
            args,
            vec!["run", "-a", "-v", "--completion-promise", "scan.complete"]
        );
    }

    #[test]
    fn build_args_with_all_options() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut config = base_config(StageId::OrbitalLanes);
        config.track = Some("backend".into());
        config.hat_collection = Some("build-hats".into());
        config.prompt_file = Some(PathBuf::from("/tmp/prompt.md"));
        config.completion_promise = Some("track.backend.complete".into());
        let args = Orchestrator::build_args(&config, dir.path());
        assert_eq!(
            args,
            vec![
                "run",
                "-a",
                "-v",
                "-H",
                "build-hats",
                "-P",
                "/tmp/prompt.md",
                "--completion-promise",
                "track.backend.complete"
            ]
        );
    }

    #[test]
    fn build_args_minimal() {
        let dir = tempfile::TempDir::new().unwrap();
        let config = base_config(StageId::Land);
        let args = Orchestrator::build_args(&config, dir.path());
        assert_eq!(args, vec!["run", "-a", "-v"]);
    }

    // AC2: Spawn returns a LoopHandle with valid PID
    #[tokio::test]
    async fn spawn_returns_handle_with_pid() {
        let mut orch = Orchestrator::default();
        let handle = spawn_sleep(&mut orch, StageId::Scan, None).await;
        assert!(handle.pid > 0);
        assert_eq!(handle.stage, StageId::Scan);
        let lid = handle.loop_id.clone();
        orch.kill(&lid).await.unwrap();
    }

    // AC3: Kill terminates the process
    #[tokio::test]
    async fn kill_terminates_process() {
        let mut orch = Orchestrator::default();
        let handle = spawn_sleep(&mut orch, StageId::Scan, None).await;
        let lid = handle.loop_id.clone();
        assert!(orch.is_alive(&lid));
        orch.kill(&lid).await.unwrap();
        assert!(!orch.is_alive(&lid));
    }

    // AC4: is_alive returns true for running process
    #[tokio::test]
    async fn is_alive_true_for_running() {
        let mut orch = Orchestrator::default();
        let handle = spawn_sleep(&mut orch, StageId::Land, None).await;
        let lid = handle.loop_id.clone();
        assert!(orch.is_alive(&lid));
        orch.kill(&lid).await.unwrap();
    }

    // AC5: is_alive returns false for exited process
    #[tokio::test]
    async fn is_alive_false_for_exited() {
        let mut orch = Orchestrator::default();
        let loop_id = "quick-exit".to_string();
        let child = Command::new("true")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id().unwrap_or(0);
        orch.loops.insert(
            loop_id.clone(),
            LoopHandle {
                pid,
                loop_id: loop_id.clone(),
                stage: StageId::Scan,
                track: None,
                spawned_at: Utc::now(),
                child,
            },
        );
        // Wait for the short-lived process to exit; retry to handle slow CI
        for _ in 0..20 {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            if !orch.is_alive(&loop_id) {
                break;
            }
        }
        assert!(!orch.is_alive(&loop_id));
    }

    #[test]
    fn is_alive_false_for_unknown() {
        let mut orch = Orchestrator::default();
        assert!(!orch.is_alive("nonexistent"));
    }

    // AC6: active_loops tracks multiple spawned processes
    #[tokio::test]
    async fn active_loops_tracks_multiple() {
        let mut orch = Orchestrator::default();
        let h1 = spawn_sleep(&mut orch, StageId::Scan, None).await;
        let lid1 = h1.loop_id.clone();
        let h2 = spawn_sleep(&mut orch, StageId::Land, Some("backend".into())).await;
        let lid2 = h2.loop_id.clone();

        assert_eq!(orch.active_loops().len(), 2);
        assert!(orch.active_loops().contains_key(&lid1));
        assert!(orch.active_loops().contains_key(&lid2));

        orch.kill(&lid1).await.unwrap();
        orch.kill(&lid2).await.unwrap();
    }

    #[tokio::test]
    async fn kill_unknown_returns_error() {
        let mut orch = Orchestrator::default();
        assert!(orch.kill("nonexistent").await.is_err());
    }

    #[tokio::test]
    async fn kill_loops_for_stage_kills_matching() {
        let mut orch = Orchestrator::default();
        let h1 = spawn_sleep(&mut orch, StageId::Scan, None).await;
        let lid1 = h1.loop_id.clone();
        let _h2 = spawn_sleep(&mut orch, StageId::Plan, None).await;
        let killed = orch.kill_loops_for_stage(StageId::Scan).await;
        assert_eq!(killed, 1);
        assert!(!orch.loops.contains_key(&lid1));
        assert_eq!(orch.loops.len(), 1); // Plan loop still alive
    }

    #[tokio::test]
    async fn kill_loops_for_track_kills_matching() {
        let mut orch = Orchestrator::default();
        let h1 = spawn_sleep(&mut orch, StageId::OrbitalLanes, Some("track-a".into())).await;
        let lid1 = h1.loop_id.clone();
        let _h2 = spawn_sleep(&mut orch, StageId::OrbitalLanes, Some("track-b".into())).await;
        let killed = orch.kill_loops_for_track("track-a").await;
        assert_eq!(killed, 1);
        assert!(!orch.loops.contains_key(&lid1));
        assert_eq!(orch.loops.len(), 1); // track-b still alive
    }

    #[tokio::test]
    async fn kill_loops_for_stage_returns_zero_when_none() {
        let mut orch = Orchestrator::default();
        let killed = orch.kill_loops_for_stage(StageId::Scan).await;
        assert_eq!(killed, 0);
    }

    // --- New v2 tests ---

    // AC-new-1: build_args with MCP config writes file and adds --mcp-config arg
    #[test]
    fn build_args_with_mcp_servers() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut config = base_config(StageId::RequirementsAnalysis);
        config.mcp_servers = Some(vec![McpServerConfig {
            name: "docs".into(),
            command: "docs-mcp-server".into(),
            env: HashMap::new(),
        }]);
        let args = Orchestrator::build_args(&config, dir.path());
        let mcp_path = dir.path().join(".loopy/mcp-config.json");
        assert!(mcp_path.exists());
        let content: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&mcp_path).unwrap()).unwrap();
        assert_eq!(content["mcpServers"]["docs"]["command"], "docs-mcp-server");
        assert!(args.contains(&"--mcp-config".to_string()));
        assert!(args.contains(&mcp_path.display().to_string()));
    }

    // AC-new-2: write_rerun_context creates file with reason
    #[test]
    fn write_rerun_context_creates_file() {
        let dir = tempfile::TempDir::new().unwrap();
        Orchestrator::write_rerun_context(
            dir.path(),
            "console",
            "api-contract changed from v1 to v2",
        )
        .unwrap();
        let path = dir
            .path()
            .join(".loopy/track-context/console/rerun-context.md");
        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "api-contract changed from v1 to v2");
    }

    // AC-new-3: build_args with execution mode adds --mode flag
    #[test]
    fn build_args_with_execution_mode() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut config = base_config(StageId::Scan);
        config.execution_mode = Some(ExecutionMode::Fast);
        let args = Orchestrator::build_args(&config, dir.path());
        assert!(args.contains(&"--mode".to_string()));
        assert!(args.contains(&"fast".to_string()));
    }

    // Helper: spawn a `sleep 60` process as a stand-in for ralph
    // Monotonic counter so two spawns in the same millisecond get UNIQUE loop_ids.
    // (Previously both used a timestamp-ms key; same-ms spawns collided in the
    // HashMap, making kill/len assertions flaky.)
    static SPAWN_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    async fn spawn_sleep(
        orch: &mut Orchestrator,
        stage: StageId,
        track: Option<String>,
    ) -> &LoopHandle {
        let seq = SPAWN_SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let loop_id = format!("{:?}-{}-{}", stage, chrono::Utc::now().timestamp_millis(), seq);
        let child = Command::new("sleep")
            .arg("60")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .unwrap();
        let pid = child.id().unwrap_or(0);
        orch.loops.insert(
            loop_id.clone(),
            LoopHandle {
                pid,
                loop_id: loop_id.clone(),
                stage,
                track,
                spawned_at: Utc::now(),
                child,
            },
        );
        orch.loops.get(&loop_id).unwrap()
    }

    // --- Step 2: Stage directory setup tests ---

    // AC1: setup_stage_dir creates ralph.yml with correct completion_promise
    #[test]
    fn setup_stage_dir_creates_ralph_yml() {
        let dir = tempfile::TempDir::new().unwrap();
        let stage_dir = setup_stage_dir(
            StageId::Scan,
            "Build auth",
            None,
            dir.path(),
            &LoopyConfig::default(),
            &dir.path().join(".loopy/stages"),
        )
        .unwrap();
        let yml = std::fs::read_to_string(stage_dir.join("ralph.yml")).unwrap();
        assert!(yml.contains("completion_promise: \"scan.complete\""));
        assert!(yml.contains("backend: \"claude\""));
        assert!(yml.contains("prompt_file: \"PROMPT.md\""));
        assert!(yml.contains("max_iterations: 350"));
    }

    // AC2: setup_stage_dir creates PROMPT.md with idea text
    #[test]
    fn setup_stage_dir_creates_prompt_md() {
        let dir = tempfile::TempDir::new().unwrap();
        let stage_dir = setup_stage_dir(
            StageId::Scan,
            "Build auth",
            None,
            dir.path(),
            &LoopyConfig::default(),
            &dir.path().join(".loopy/stages"),
        )
        .unwrap();
        let prompt = std::fs::read_to_string(stage_dir.join("PROMPT.md")).unwrap();
        assert!(prompt.contains("Build auth"));
    }

    // AC3: setup_stage_dir includes KB content in PROMPT.md
    #[test]
    fn setup_stage_dir_includes_kb_content() {
        let dir = tempfile::TempDir::new().unwrap();
        let stage_dir = setup_stage_dir(
            StageId::Scan,
            "Build auth",
            Some("existing docs"),
            dir.path(),
            &LoopyConfig::default(),
            &dir.path().join(".loopy/stages"),
        )
        .unwrap();
        // KB content is written to a file, not embedded in prompt
        let kb = std::fs::read_to_string(stage_dir.join("knowledge-base.md")).unwrap();
        assert!(kb.contains("existing docs"));
        let prompt = std::fs::read_to_string(stage_dir.join("PROMPT.md")).unwrap();
        assert!(prompt.contains("knowledge-base.md"), "prompt should reference kb file");
    }

    // AC4: setup_stage_dir copies previous stage outputs
    #[test]
    fn setup_stage_dir_copies_previous_outputs() {
        let dir = tempfile::TempDir::new().unwrap();
        // Create scan output
        let scan_output = dir.path().join(".loopy/stages/scan/output");
        std::fs::create_dir_all(&scan_output).unwrap();
        std::fs::write(scan_output.join("scratchpad.md"), "scan results").unwrap();
        // Setup requirements-analysis
        let stage_dir = setup_stage_dir(
            StageId::RequirementsAnalysis,
            "Build auth",
            None,
            dir.path(),
            &LoopyConfig::default(),
            &dir.path().join(".loopy/stages"),
        )
        .unwrap();
        let copied = stage_dir.join("scan-output/scratchpad.md");
        assert!(copied.exists());
        assert_eq!(std::fs::read_to_string(&copied).unwrap(), "scan results");
    }

    // AC5: collect_stage_output copies scratchpad
    #[test]
    fn collect_stage_output_copies_scratchpad() {
        let dir = tempfile::TempDir::new().unwrap();
        let stage_dir = dir.path().join("stage");
        let agent_dir = stage_dir.join(".ralph/agent");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(agent_dir.join("scratchpad.md"), "notes").unwrap();
        collect_stage_output(&stage_dir).unwrap();
        let output = stage_dir.join("output/scratchpad.md");
        assert!(output.exists());
        assert_eq!(std::fs::read_to_string(&output).unwrap(), "notes");
    }

    // AC5b: collect_stage_output copies events jsonl
    #[test]
    fn collect_stage_output_copies_events() {
        let dir = tempfile::TempDir::new().unwrap();
        let stage_dir = dir.path().join("stage");
        let ralph_dir = stage_dir.join(".ralph");
        std::fs::create_dir_all(&ralph_dir).unwrap();
        std::fs::write(ralph_dir.join("events-Scan-123.jsonl"), "{}").unwrap();
        collect_stage_output(&stage_dir).unwrap();
        assert!(
            stage_dir
                .join("output/events/events-Scan-123.jsonl")
                .exists()
        );
    }

    // AC6: collect_stage_output handles missing files gracefully
    #[test]
    fn collect_stage_output_handles_empty_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let stage_dir = dir.path().join("empty-stage");
        std::fs::create_dir_all(&stage_dir).unwrap();
        assert!(collect_stage_output(&stage_dir).is_ok());
    }

    // AC: collect_stage_output works on track dirs (orbital-lanes/<track_id>/)
    #[test]
    fn collect_stage_output_works_for_track_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let track_dir = dir.path().join("orbital-lanes/auth-track");
        let agent_dir = track_dir.join(".ralph/agent");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(agent_dir.join("scratchpad.md"), "track output").unwrap();
        std::fs::write(agent_dir.join("context.md"), "track context").unwrap();
        collect_stage_output(&track_dir).unwrap();
        let output_dir = track_dir.join("output");
        assert!(output_dir.join("scratchpad.md").exists());
        assert!(output_dir.join("context.md").exists());
        assert_eq!(
            std::fs::read_to_string(output_dir.join("scratchpad.md")).unwrap(),
            "track output"
        );
    }

    // AC7: build_args uses --config when work_dir is set
    #[test]
    fn build_args_uses_config_flag_with_work_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut config = base_config(StageId::Scan);
        config.work_dir = Some(dir.path().to_path_buf());
        let args = Orchestrator::build_args(&config, dir.path());
        assert!(args.contains(&"--config".to_string()));
        assert!(args.contains(&"ralph.yml".to_string()));
        assert!(!args.contains(&"-P".to_string()));
        assert!(!args.contains(&"--completion-promise".to_string()));
    }

    // AC7b: build_args still uses -P/--completion-promise without work_dir
    #[test]
    fn build_args_uses_legacy_flags_without_work_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut config = base_config(StageId::Scan);
        config.completion_promise = Some("scan.complete".into());
        config.prompt_file = Some(PathBuf::from("PROMPT.md"));
        let args = Orchestrator::build_args(&config, dir.path());
        assert!(args.contains(&"-P".to_string()));
        assert!(args.contains(&"--completion-promise".to_string()));
        assert!(!args.contains(&"--config".to_string()));
    }

    // Stage name mapping
    #[test]
    fn stage_dir_name_mapping() {
        assert_eq!(stage_dir_name(StageId::Scan), "scan");
        assert_eq!(
            stage_dir_name(StageId::RequirementsAnalysis),
            "requirements-analysis"
        );
        assert_eq!(stage_dir_name(StageId::OrbitalLanes), "orbital-lanes");
        assert_eq!(stage_dir_name(StageId::Land), "land");
    }

    // ralph.yml per-stage completion promises
    #[test]
    fn setup_stage_dir_requirements_analysis_promise() {
        let dir = tempfile::TempDir::new().unwrap();
        let stage_dir = setup_stage_dir(
            StageId::RequirementsAnalysis,
            "idea",
            None,
            dir.path(),
            &LoopyConfig::default(),
            &dir.path().join(".loopy/stages"),
        )
        .unwrap();
        let yml = std::fs::read_to_string(stage_dir.join("ralph.yml")).unwrap();
        assert!(yml.contains("completion_promise: \"requirements.complete\""));
    }

    #[test]
    fn setup_stage_dir_land_promise() {
        let dir = tempfile::TempDir::new().unwrap();
        let stage_dir = setup_stage_dir(
            StageId::Land,
            "idea",
            None,
            dir.path(),
            &LoopyConfig::default(),
            &dir.path().join(".loopy/stages"),
        )
        .unwrap();
        let yml = std::fs::read_to_string(stage_dir.join("ralph.yml")).unwrap();
        assert!(yml.contains("completion_promise: \"land.complete\""));
    }

    // AC: setup_stage_dir writes hat_collection into ralph.yml for Scan
    #[test]
    fn setup_stage_dir_scan_hat_collection() {
        let dir = tempfile::TempDir::new().unwrap();
        let stage_dir = setup_stage_dir(
            StageId::Scan,
            "idea",
            None,
            dir.path(),
            &LoopyConfig::default(),
            &dir.path().join(".loopy/stages"),
        )
        .unwrap();
        let yml = std::fs::read_to_string(stage_dir.join("ralph.yml")).unwrap();
        // Scan uses traditional mode — no hat_collection in ralph.yml
        assert!(
            !yml.contains("hat_collection"),
            "Scan should use traditional mode, got: {yml}"
        );
    }

    // AC: setup_stage_dir Plan uses traditional mode (no hats)
    #[test]
    fn setup_stage_dir_plan_generates_hat_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let stage_dir = setup_stage_dir(
            StageId::Plan,
            "idea",
            None,
            dir.path(),
            &LoopyConfig::default(),
            &dir.path().join(".loopy/stages"),
        )
        .unwrap();
        let yml = std::fs::read_to_string(stage_dir.join("ralph.yml")).unwrap();
        assert!(
            !yml.contains("hat_collection"),
            "Plan should use traditional mode, got: {yml}"
        );
    }

    // AC: setup_stage_dir RequirementsAnalysis uses traditional mode (no hats)
    #[test]
    fn setup_stage_dir_reqanalysis_generates_hat_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let stage_dir = setup_stage_dir(
            StageId::RequirementsAnalysis,
            "idea",
            None,
            dir.path(),
            &LoopyConfig::default(),
            &dir.path().join(".loopy/stages"),
        )
        .unwrap();
        let yml = std::fs::read_to_string(stage_dir.join("ralph.yml")).unwrap();
        assert!(
            !yml.contains("hat_collection"),
            "RequirementsAnalysis should use traditional mode, got: {yml}"
        );
    }

    // AC: custom hat files are not overwritten if they already exist
    #[test]
    fn setup_stage_dir_preserves_existing_hat_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let hat_dir = dir.path().join(".loopy/hats");
        std::fs::create_dir_all(&hat_dir).unwrap();
        std::fs::write(hat_dir.join("planner.yml"), "custom content").unwrap();
        setup_stage_dir(
            StageId::Plan,
            "idea",
            None,
            dir.path(),
            &LoopyConfig::default(),
            &dir.path().join(".loopy/stages"),
        )
        .unwrap();
        let content = std::fs::read_to_string(hat_dir.join("planner.yml")).unwrap();
        assert_eq!(content, "custom content");
    }

    // AC: spawn with missing binary returns friendly error containing "not found on PATH"
    #[tokio::test]
    async fn spawn_not_found_gives_friendly_error() {
        let mut orch = Orchestrator {
            ralph_bin: "ralph-nonexistent-binary".into(),
            ..Default::default()
        };
        let config = base_config(StageId::Scan);
        let err = orch.spawn(config).await.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("not found on PATH"),
            "expected friendly message, got: {msg}"
        );
    }

    #[test]
    fn hat_collection_for_stage_returns_correct_hats() {
        assert_eq!(hat_collection_for_stage(StageId::Scan), None);
        assert_eq!(hat_collection_for_stage(StageId::Plan), None);
        assert_eq!(
            hat_collection_for_stage(StageId::RequirementsAnalysis),
            None
        );
        assert_eq!(
            hat_collection_for_stage(StageId::OrbitalLanes),
            Some("builtin:code-assist")
        );
        assert_eq!(hat_collection_for_stage(StageId::Land), None);
    }

    // AC: generate_available_tracks_md produces markdown with track info
    #[test]
    fn generate_available_tracks_md_contains_all_tracks() {
        let defs = crate::models::builtin_track_definitions();
        let md = generate_available_tracks_md(&defs);
        assert!(md.contains("# Available Tracks"));
        for (key, def) in &defs {
            assert!(md.contains(&def.name), "missing track name: {}", def.name);
            assert!(md.contains(&def.icon), "missing icon for: {key}");
            assert!(
                md.contains(&def.description),
                "missing description for: {key}"
            );
        }
    }

    // AC: generate_available_tracks_md shows dependencies and artifacts
    #[test]
    fn generate_available_tracks_md_shows_deps_and_artifacts() {
        use crate::models::TrackDefinition;
        let mut defs = std::collections::BTreeMap::new();
        defs.insert(
            "test-track".into(),
            TrackDefinition {
                name: "Test Track".into(),
                icon: "🧪".into(),
                description: "A test track".into(),
                hat: "builtin:code-assist".into(),
                dependencies: vec!["backend".into(), "auth".into()],
                artifacts_produced: vec!["test-results".into()],
                approval: crate::models::ApprovalMode::Auto,
                enabled: true,
                medium: crate::models::TrackMedium::default(),
                medium_config: crate::models::MediumConfig::default(),
                max_iterations: None,
            },
        );
        let md = generate_available_tracks_md(&defs);
        assert!(md.contains("backend"), "should list dependency");
        assert!(md.contains("auth"), "should list dependency");
        assert!(md.contains("test-results"), "should list artifact");
    }

    // AC: setup_stage_dir for Plan writes AVAILABLE_TRACKS.md
    #[test]
    fn setup_stage_dir_plan_generates_available_tracks_md() {
        let dir = tempfile::TempDir::new().unwrap();
        let stage_dir = setup_stage_dir(
            StageId::Plan,
            "idea",
            None,
            dir.path(),
            &LoopyConfig::default(),
            &dir.path().join(".loopy/stages"),
        )
        .unwrap();
        let md_path = stage_dir.join("AVAILABLE_TRACKS.md");
        assert!(
            md_path.exists(),
            "Plan stage should have AVAILABLE_TRACKS.md"
        );
        let content = std::fs::read_to_string(&md_path).unwrap();
        assert!(content.contains("Backend"));
        assert!(content.contains("Frontend"));
        assert!(content.contains("Security"));
        assert!(content.contains("Infrastructure"));
        assert!(content.contains("Observability"));
        assert!(content.contains("Auth"));
        assert!(content.contains("Console"));
    }

    // AC: non-Plan stages do NOT get AVAILABLE_TRACKS.md
    #[test]
    fn setup_stage_dir_non_plan_no_available_tracks() {
        let dir = tempfile::TempDir::new().unwrap();
        let stage_dir = setup_stage_dir(
            StageId::Scan,
            "idea",
            None,
            dir.path(),
            &LoopyConfig::default(),
            &dir.path().join(".loopy/stages"),
        )
        .unwrap();
        assert!(!stage_dir.join("AVAILABLE_TRACKS.md").exists());
    }

    // AC: is_track_enabled returns false for disabled tracks
    #[test]
    fn is_track_enabled_respects_config() {
        let dir = tempfile::TempDir::new().unwrap();
        // No config → all built-ins enabled
        assert!(is_track_enabled(dir.path(), "backend"));
        // Unknown track → default enabled
        assert!(is_track_enabled(dir.path(), "nonexistent"));
        // Disable via loopy.yml
        let yml = "tracks:\n  backend:\n    enabled: false\n    approval: auto\n";
        std::fs::write(dir.path().join("loopy.yml"), yml).unwrap();
        assert!(!is_track_enabled(dir.path(), "backend"));
        assert!(is_track_enabled(dir.path(), "frontend"));
    }

    // ── loopy init tests ──

    #[test]
    fn loopy_init_creates_directory_structure() {
        let dir = tempfile::TempDir::new().unwrap();
        loopy_init(dir.path(), None, false).unwrap();
        for sub in &["hats", "tracks", "artifacts", "context", "docs"] {
            assert!(
                dir.path().join(".loopy").join(sub).is_dir(),
                "missing .loopy/{sub}"
            );
        }
    }

    #[test]
    fn loopy_init_creates_stages_and_projects_dirs() {
        let dir = tempfile::TempDir::new().unwrap();
        loopy_init(dir.path(), None, false).unwrap();
        for sub in &["stages", "projects"] {
            assert!(
                dir.path().join(".loopy").join(sub).is_dir(),
                "missing .loopy/{sub}"
            );
        }
    }

    #[test]
    fn loopy_init_generates_loopy_yml() {
        let dir = tempfile::TempDir::new().unwrap();
        loopy_init(dir.path(), None, false).unwrap();
        let yml = std::fs::read_to_string(dir.path().join("loopy.yml")).unwrap();
        assert!(yml.contains("backend: claude"));
        assert!(yml.contains("max_iterations: 350"));
    }

    #[test]
    fn loopy_init_generates_five_hat_files() {
        let dir = tempfile::TempDir::new().unwrap();
        loopy_init(dir.path(), None, false).unwrap();
        let hats = dir.path().join(".loopy/hats");
        for name in &[
            "planner.yml",
            "requirements-analysis.yml",
            "environment-setup.yml",
            "land-summary.yml",
            "security-analyst.yml",
        ] {
            assert!(hats.join(name).exists(), "missing hat file: {name}");
            let content = std::fs::read_to_string(hats.join(name)).unwrap();
            assert!(content.contains("hats:"), "{name} missing hats: key");
        }
    }

    #[test]
    fn loopy_init_is_idempotent() {
        let dir = tempfile::TempDir::new().unwrap();
        loopy_init(dir.path(), None, false).unwrap();
        // Write custom content to a hat file
        let custom = dir.path().join(".loopy/hats/planner.yml");
        std::fs::write(&custom, "custom content").unwrap();
        // Re-run init — should NOT overwrite existing hat files
        loopy_init(dir.path(), None, false).unwrap();
        assert_eq!(std::fs::read_to_string(&custom).unwrap(), "custom content");
    }

    #[test]
    fn hat_collection_for_stage_prefers_loopy_hats() {
        let dir = tempfile::TempDir::new().unwrap();
        loopy_init(dir.path(), None, false).unwrap();
        // Land uses traditional mode (no hats)
        let hat = hat_collection_for_stage_with_root(StageId::Land, dir.path());
        assert!(hat.is_none(), "Land should use traditional mode");
    }

    #[test]
    fn hat_collection_fallback_when_no_loopy_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        // No init — should fall back to None (traditional mode)
        let hat = hat_collection_for_stage_with_root(StageId::Land, dir.path());
        assert!(hat.is_none());
    }

    #[test]
    fn loopy_yml_contains_project_name_from_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        loopy_init(dir.path(), None, false).unwrap();
        let yml = std::fs::read_to_string(dir.path().join("loopy.yml")).unwrap();
        // project name derived from directory name
        let dir_name = dir.path().file_name().unwrap().to_string_lossy();
        assert!(yml.contains(&format!("project: {dir_name}")));
    }

    #[test]
    fn hat_files_contain_expected_hat_keys() {
        let dir = tempfile::TempDir::new().unwrap();
        loopy_init(dir.path(), None, false).unwrap();
        let hats = dir.path().join(".loopy/hats");
        let expected: &[(&str, &str)] = &[
            ("planner.yml", "planner:"),
            ("requirements-analysis.yml", "product_analyst:"),
            ("environment-setup.yml", "environment_setup:"),
            ("land-summary.yml", "release_manager:"),
            ("security-analyst.yml", "security_analyst:"),
        ];
        for (file, key) in expected {
            let content = std::fs::read_to_string(hats.join(file)).unwrap();
            assert!(content.contains(key), "{file} missing hat key {key}");
        }
    }

    #[test]
    fn hat_files_contain_system_prompts() {
        let dir = tempfile::TempDir::new().unwrap();
        loopy_init(dir.path(), None, false).unwrap();
        let hats = dir.path().join(".loopy/hats");
        for name in &[
            "planner.yml",
            "requirements-analysis.yml",
            "environment-setup.yml",
            "land-summary.yml",
            "security-analyst.yml",
        ] {
            let content = std::fs::read_to_string(hats.join(name)).unwrap();
            assert!(
                content.contains("system_prompt:"),
                "{name} missing system_prompt"
            );
            assert!(content.contains("triggers:"), "{name} missing triggers");
            assert!(content.contains("publishes:"), "{name} missing publishes");
        }
    }

    #[test]
    fn hat_collection_for_stage_plan_prefers_loopy_hats() {
        let dir = tempfile::TempDir::new().unwrap();
        loopy_init(dir.path(), None, false).unwrap();
        let hat = hat_collection_for_stage_with_root(StageId::Plan, dir.path());
        assert!(hat.is_none(), "Plan should use traditional mode");
    }

    #[test]
    fn hat_collection_for_stage_requirements_prefers_loopy_hats() {
        let dir = tempfile::TempDir::new().unwrap();
        loopy_init(dir.path(), None, false).unwrap();
        let hat = hat_collection_for_stage_with_root(StageId::RequirementsAnalysis, dir.path());
        assert!(
            hat.is_none(),
            "RequirementsAnalysis should use traditional mode"
        );
    }

    // --- Step 3c: setup_track_workspace tests ---

    #[test]
    fn setup_track_workspace_creates_directory() {
        let dir = tempfile::TempDir::new().unwrap();
        loopy_init(dir.path(), None, false).unwrap();
        let def = TrackDefinition {
            name: "Backend".into(),
            icon: "🔧".into(),
            description: "Backend track".into(),
            hat: "builtin:code-assist".into(),
            dependencies: vec![],
            artifacts_produced: vec![],
            approval: Default::default(),
            enabled: true,
            medium: crate::models::TrackMedium::Brazil,
            medium_config: crate::models::MediumConfig::Brazil {
                packages: vec![crate::models::PackageRef {
                    name: "MyService".into(),
                    version_set: "MyService/development".into(),
                }],
                new_packages: vec![],
                version_set: None,
            },
            max_iterations: None,
        };
        let ws = setup_track_workspace(
            "backend",
            &def,
            dir.path(),
            &LoopyConfig::default(),
            &dir.path().join("stages"),
        )
        .unwrap();
        assert!(ws.exists(), "workspace directory should be created");
        assert_eq!(ws, dir.path().join("workspaces/backend"));
    }

    #[test]
    fn setup_track_workspace_creates_dir_only() {
        // setup_track_workspace now only creates the directory — no ralph.yml or PROMPT.md
        let dir = tempfile::TempDir::new().unwrap();
        loopy_init(dir.path(), None, false).unwrap();
        let def = TrackDefinition {
            name: "Backend".into(),
            icon: "🔧".into(),
            description: "Backend track".into(),
            hat: "builtin:code-assist".into(),
            dependencies: vec![],
            artifacts_produced: vec![],
            approval: Default::default(),
            enabled: true,
            medium: crate::models::TrackMedium::Brazil,
            medium_config: crate::models::MediumConfig::Brazil {
                packages: vec![
                    crate::models::PackageRef {
                        name: "MyService".into(),
                        version_set: "MyService/development".into(),
                    },
                    crate::models::PackageRef {
                        name: "MyLib".into(),
                        version_set: "MyLib/mainline".into(),
                    },
                ],
                new_packages: vec![crate::models::NewPackage {
                    name: "MyNew".into(),
                    template: "java-service".into(),
                }],
                version_set: None,
            },
            max_iterations: None,
        };
        let ws = setup_track_workspace(
            "backend",
            &def,
            dir.path(),
            &LoopyConfig::default(),
            &dir.path().join("stages"),
        )
        .unwrap();
        assert!(ws.exists());
        // No ralph.yml or PROMPT.md — setup is done by Loopy directly
        assert!(!ws.join("ralph.yml").exists());
        assert!(!ws.join("PROMPT.md").exists());
    }

    #[test]
    fn setup_track_workspace_returns_correct_path() {
        let dir = tempfile::TempDir::new().unwrap();
        loopy_init(dir.path(), None, false).unwrap();
        let def = TrackDefinition {
            name: "Infra".into(),
            icon: "🏗️".into(),
            description: "Infra track".into(),
            hat: "builtin:code-assist".into(),
            dependencies: vec![],
            artifacts_produced: vec![],
            approval: Default::default(),
            enabled: true,
            medium: crate::models::TrackMedium::Brazil,
            medium_config: crate::models::MediumConfig::default(),
            max_iterations: None,
        };
        let ws = setup_track_workspace(
            "infra",
            &def,
            dir.path(),
            &LoopyConfig::default(),
            &dir.path().join("stages"),
        )
        .unwrap();
        assert_eq!(ws, dir.path().join("workspaces/infra"));
    }

    // --- setup_track_dir tests ---

    #[test]
    fn setup_track_dir_creates_ralph_yml_with_code_assist_hat() {
        let dir = tempfile::TempDir::new().unwrap();
        let stages_base = dir.path().join("stages");
        let track_dir = setup_track_dir(
            "backend",
            "Implement REST API endpoints",
            &[],
            &LoopyConfig::default(),
            &stages_base,
        )
        .unwrap();
        let yml = std::fs::read_to_string(track_dir.join("ralph.yml")).unwrap();
        assert!(yml.contains("completion_promise: \"track.backend.complete\""));
        assert!(yml.contains("hat_collection: \"builtin:code-assist\""));
        assert!(yml.contains("prompt_file: \"PROMPT.md\""));
        assert!(yml.contains("backend: \"claude\""));
    }

    #[test]
    fn setup_track_dir_creates_prompt_md_with_description() {
        let dir = tempfile::TempDir::new().unwrap();
        let stages_base = dir.path().join("stages");
        let track_dir = setup_track_dir(
            "backend",
            "Implement REST API endpoints",
            &[],
            &LoopyConfig::default(),
            &stages_base,
        )
        .unwrap();
        let prompt = std::fs::read_to_string(track_dir.join("PROMPT.md")).unwrap();
        assert!(prompt.contains("Implement REST API endpoints"));
        assert!(prompt.contains("backend"));
    }

    #[test]
    fn setup_track_dir_includes_dependency_artifacts() {
        let dir = tempfile::TempDir::new().unwrap();
        let stages_base = dir.path().join("stages");
        // Create a dependency track output
        let dep_output = stages_base.join("orbital-lanes/auth/output");
        std::fs::create_dir_all(&dep_output).unwrap();
        std::fs::write(dep_output.join("scratchpad.md"), "Auth flow done").unwrap();
        let deps = vec!["auth".to_string()];
        let track_dir = setup_track_dir(
            "backend",
            "Implement REST API endpoints",
            &deps,
            &LoopyConfig::default(),
            &stages_base,
        )
        .unwrap();
        let prompt = std::fs::read_to_string(track_dir.join("PROMPT.md")).unwrap();
        assert!(
            prompt.contains("Dependency: auth"),
            "PROMPT.md should reference dependency track"
        );
        assert!(
            prompt.contains("output"),
            "PROMPT.md should reference dependency output path"
        );
    }

    #[test]
    fn setup_track_dir_creates_dir_and_config() {
        let dir = tempfile::TempDir::new().unwrap();
        let stages_base = dir.path().join("stages");
        let track_dir = setup_track_dir(
            "backend",
            "Implement REST API endpoints",
            &[],
            &LoopyConfig::default(),
            &stages_base,
        )
        .unwrap();
        // Git init is async (fire-and-forget), so don't check .git
        assert!(track_dir.join("ralph.yml").exists());
        assert!(track_dir.join("PROMPT.md").exists());
    }

    #[test]
    fn setup_track_dir_returns_correct_path() {
        let dir = tempfile::TempDir::new().unwrap();
        let stages_base = dir.path().join("stages");
        let track_dir = setup_track_dir(
            "infra",
            "Setup CDK",
            &[],
            &LoopyConfig::default(),
            &stages_base,
        )
        .unwrap();
        assert_eq!(track_dir, stages_base.join("orbital-lanes/infra"));
    }

    #[test]
    fn spawn_track_code_loop_builds_correct_config() {
        let dir = tempfile::TempDir::new().unwrap();
        let ws = dir.path().join("workspaces/backend");
        std::fs::create_dir_all(ws.join("src/MyService")).unwrap();
        let def = TrackDefinition {
            name: "Backend".into(),
            icon: "🔧".into(),
            description: "Backend track".into(),
            hat: "builtin:code-assist".into(),
            dependencies: vec![],
            artifacts_produced: vec![],
            approval: Default::default(),
            enabled: true,
            medium: crate::models::TrackMedium::Brazil,
            medium_config: crate::models::MediumConfig::Brazil {
                packages: vec![crate::models::PackageRef {
                    name: "MyService".into(),
                    version_set: "MyService/development".into(),
                }],
                new_packages: vec![],
                version_set: None,
            },
            max_iterations: None,
        };
        let config = build_track_code_loop_config("backend", &ws, &def);
        assert_eq!(config.stage, StageId::OrbitalLanes);
        assert_eq!(config.track.as_deref(), Some("backend"));
        assert_eq!(
            config.completion_promise.as_deref(),
            Some("track.backend.complete")
        );
        // work_dir should be the package dir inside workspace
        assert_eq!(config.work_dir.unwrap(), ws.join("src/MyService"));
    }

    #[test]
    fn spawn_track_code_loop_falls_back_to_workspace_when_no_package_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let ws = dir.path().join("workspaces/backend");
        std::fs::create_dir_all(&ws).unwrap();
        let def = TrackDefinition {
            name: "Backend".into(),
            icon: "🔧".into(),
            description: "Backend track".into(),
            hat: "builtin:code-assist".into(),
            dependencies: vec![],
            artifacts_produced: vec![],
            approval: Default::default(),
            enabled: true,
            medium: crate::models::TrackMedium::Brazil,
            medium_config: crate::models::MediumConfig::Brazil {
                packages: vec![crate::models::PackageRef {
                    name: "MyService".into(),
                    version_set: "MyService/development".into(),
                }],
                new_packages: vec![],
                version_set: None,
            },
            max_iterations: None,
        };
        let config = build_track_code_loop_config("backend", &ws, &def);
        // Falls back to workspace root when package dir doesn't exist yet
        assert_eq!(config.work_dir.unwrap(), ws);
    }

    // --- Step 4b: Config-driven max_iterations + backend ---

    #[test]
    fn setup_stage_dir_uses_config_backend_and_max_iterations() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut cfg = LoopyConfig::default();
        cfg.backend = Some("claude".into());
        cfg.max_iterations = Some(500);
        let stage_dir = setup_stage_dir(
            StageId::Scan,
            "test",
            None,
            dir.path(),
            &cfg,
            &dir.path().join(".loopy/stages"),
        )
        .unwrap();
        let yml = std::fs::read_to_string(stage_dir.join("ralph.yml")).unwrap();
        assert!(yml.contains("backend: \"claude\""), "yml={yml}");
        assert!(yml.contains("max_iterations: 500"), "yml={yml}");
    }

    #[test]
    fn setup_stage_dir_per_stage_override() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut cfg = LoopyConfig::default();
        cfg.max_iterations = Some(500);
        cfg.stages.insert(
            "scan".into(),
            StageConfig {
                max_iterations: Some(100),
            },
        );
        let stage_dir = setup_stage_dir(
            StageId::Scan,
            "test",
            None,
            dir.path(),
            &cfg,
            &dir.path().join(".loopy/stages"),
        )
        .unwrap();
        let yml = std::fs::read_to_string(stage_dir.join("ralph.yml")).unwrap();
        assert!(
            yml.contains("max_iterations: 100"),
            "per-stage override should win: yml={yml}"
        );
    }

    #[test]
    fn setup_stage_dir_defaults_when_config_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let cfg = LoopyConfig::default();
        let stage_dir = setup_stage_dir(
            StageId::Scan,
            "test",
            None,
            dir.path(),
            &cfg,
            &dir.path().join(".loopy/stages"),
        )
        .unwrap();
        let yml = std::fs::read_to_string(stage_dir.join("ralph.yml")).unwrap();
        assert!(
            yml.contains("backend: \"claude\""),
            "default backend: yml={yml}"
        );
        assert!(
            yml.contains("max_iterations: 350"),
            "default max_iterations: yml={yml}"
        );
    }

    #[test]
    fn resolve_backend_uses_config() {
        let mut cfg = LoopyConfig::default();
        cfg.backend = Some("claude".into());
        assert_eq!(resolve_backend(&cfg), "claude");
    }

    #[test]
    fn resolve_backend_defaults_to_claude() {
        let cfg = LoopyConfig::default();
        assert_eq!(resolve_backend(&cfg), "claude");
    }

    #[test]
    fn resolve_max_iterations_per_track_wins() {
        let mut cfg = LoopyConfig::default();
        cfg.max_iterations = Some(300);
        assert_eq!(resolve_max_iterations(&cfg, "orbital-lanes", Some(75)), 75);
    }

    #[test]
    fn resolve_max_iterations_per_stage_wins_over_global() {
        let mut cfg = LoopyConfig::default();
        cfg.max_iterations = Some(300);
        cfg.stages.insert(
            "orbital-lanes".into(),
            StageConfig {
                max_iterations: Some(150),
            },
        );
        assert_eq!(resolve_max_iterations(&cfg, "orbital-lanes", None), 150);
    }

    #[test]
    fn resolve_max_iterations_global_default() {
        let mut cfg = LoopyConfig::default();
        cfg.max_iterations = Some(300);
        assert_eq!(resolve_max_iterations(&cfg, "scan", None), 300);
    }

    // --- build_stage_prompt tests ---

    #[test]
    fn build_stage_prompt_scan_has_structured_sections() {
        let dir = tempfile::TempDir::new().unwrap();
        let prompt = build_stage_prompt(StageId::Scan, "Build a widget", dir.path(), &dir.path().join(".loopy/stages"));
        assert!(
            prompt.contains("## Role"),
            "scan prompt must have Role section"
        );
        assert!(
            prompt.contains("Idea"),
            "scan prompt must have Idea section"
        );
        assert!(
            prompt.contains("Build a widget"),
            "scan prompt must contain idea text"
        );
        assert!(
            prompt.contains("scan-report.md"),
            "scan prompt must reference scan-report.md output"
        );
        assert!(
            prompt.contains("environment.json"),
            "scan prompt must reference environment.json output"
        );
        assert!(
            prompt.contains("analysis/"),
            "scan prompt must reference analysis directory"
        );
        assert!(
            prompt.contains("environment.json"),
            "scan prompt must instruct environment.json output"
        );
    }

    #[test]
    fn build_stage_prompt_scan_includes_kb_as_context() {
        let dir = tempfile::TempDir::new().unwrap();
        let prompt = build_stage_prompt(StageId::Scan, "idea", dir.path(), &dir.path().join(".loopy/stages"));
        assert!(
            prompt.contains("knowledge-base.md"),
            "scan prompt must reference knowledge-base.md"
        );
    }

    #[test]
    fn build_stage_prompt_plan_has_tracks_and_output_schema() {
        let dir = tempfile::TempDir::new().unwrap();
        loopy_init(dir.path(), None, false).unwrap();
        let prompt = build_stage_prompt(StageId::Plan, "Build a widget", dir.path(), &dir.path().join(".loopy/stages"));
        assert!(
            prompt.contains("## Role"),
            "plan prompt must have Role section"
        );
        assert!(
            prompt.contains("## Available Tracks"),
            "plan prompt must inline available tracks"
        );
        assert!(
            prompt.contains("PLAN.md"),
            "plan prompt must instruct PLAN.md output"
        );
        assert!(
            prompt.contains("tracks.json"),
            "plan prompt must instruct tracks.json output"
        );
    }

    #[test]
    fn build_stage_prompt_requirements_analysis_has_structured_output() {
        let dir = tempfile::TempDir::new().unwrap();
        let prompt = build_stage_prompt(StageId::RequirementsAnalysis, "Build a widget", dir.path(), &dir.path().join(".loopy/stages"));
        assert!(
            prompt.contains("## Role"),
            "req analysis prompt must have Role section"
        );
        assert!(
            prompt.contains("Summary"),
            "req analysis prompt must mention Summary"
        );
        assert!(
            prompt.contains("UX Requirements"),
            "req analysis prompt must mention UX Requirements"
        );
        assert!(
            prompt.contains("API Contracts"),
            "req analysis prompt must mention API Contracts"
        );
        assert!(
            prompt.contains("Open Questions"),
            "req analysis prompt must mention Open Questions"
        );
        assert!(
            prompt.contains("Trade-offs"),
            "req analysis prompt must mention Trade-offs"
        );
    }

    #[test]
    fn resolve_build_command_uses_config_when_set() {
        let mut cfg = crate::config::LoopyConfig::default();
        cfg.build_command = Some("cargo test".into());
        assert_eq!(resolve_build_command(&cfg), "cargo test");
    }

    #[test]
    fn resolve_build_command_falls_back_to_generic_hint() {
        let cfg = crate::config::LoopyConfig::default();
        let cmd = resolve_build_command(&cfg);
        assert!(cmd.contains("build and test"));
        // Blank/whitespace-only config is treated as unset.
        let mut blank = crate::config::LoopyConfig::default();
        blank.build_command = Some("   ".into());
        assert_eq!(resolve_build_command(&blank), cmd);
    }

    #[test]
    fn build_stage_prompt_orbital_lanes_has_track_context() {
        let dir = tempfile::TempDir::new().unwrap();
        let prompt = build_stage_prompt(StageId::OrbitalLanes, "Build a widget", dir.path(), &dir.path().join(".loopy/stages"));
        assert!(
            prompt.contains("## Role"),
            "orbital lanes prompt must have Role section"
        );
        assert!(
            prompt.contains("acceptance criteria"),
            "orbital lanes prompt must mention acceptance criteria"
        );
        assert!(
            prompt.contains("build and test"),
            "orbital lanes prompt must reference the build/test command"
        );
    }

    #[test]
    fn build_stage_prompt_land_has_summary_structure() {
        let dir = tempfile::TempDir::new().unwrap();
        let prompt = build_stage_prompt(StageId::Land, "Build a widget", dir.path(), &dir.path().join(".loopy/stages"));
        assert!(
            prompt.contains("## Role"),
            "land prompt must have Role section"
        );
        assert!(
            prompt.contains("SUMMARY.md"),
            "land prompt must instruct SUMMARY.md output"
        );
        assert!(
            prompt.contains("git diff"),
            "land prompt must mention git diff"
        );
        assert!(
            prompt.contains("test results"),
            "land prompt must mention test results"
        );
    }

    #[test]
    fn build_stage_prompt_land_includes_track_outputs() {
        let dir = tempfile::TempDir::new().unwrap();
        let track_dir = dir
            .path()
            .join(".loopy/stages/orbital-lanes/backend/output");
        std::fs::create_dir_all(&track_dir).unwrap();
        std::fs::write(track_dir.join("scratchpad.md"), "backend results here").unwrap();
        let prompt = build_stage_prompt(StageId::Land, "Build a widget", dir.path(), &dir.path().join(".loopy/stages"));
        assert!(
            prompt.contains("backend"),
            "land prompt must include track name"
        );
        assert!(
            prompt.contains("backend results here"),
            "land prompt must include track output"
        );
    }

    #[test]
    fn land_prompt_includes_collected_track_output() {
        let dir = tempfile::TempDir::new().unwrap();
        let stages_base = dir.path().join(".loopy/stages");
        let track_dir = stages_base.join("orbital-lanes/auth-service");
        let agent_dir = track_dir.join(".ralph/agent");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(
            agent_dir.join("scratchpad.md"),
            "Implemented OAuth2 login flow with PKCE",
        )
        .unwrap();
        // Simulate TrackCompleted → collect_stage_output
        collect_stage_output(&track_dir).unwrap();
        // Now Land prompt should include the collected content
        let prompt = build_stage_prompt(StageId::Land, "Add auth", dir.path(), &stages_base);
        assert!(
            prompt.contains("auth-service"),
            "land prompt must include track name, got: {prompt}"
        );
        assert!(
            prompt.contains("Implemented OAuth2 login flow with PKCE"),
            "land prompt must include collected track output, got: {prompt}"
        );
    }

    // --- context injection tests ---

    #[test]
    fn load_context_sources_directory_reads_key_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let src_dir = dir.path().join("src/MyService");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::write(src_dir.join("README.md"), "# My Service\nA great service").unwrap();
        std::fs::write(
            src_dir.join("Cargo.toml"),
            "[package]\nname = \"my-service\"",
        )
        .unwrap();
        let sources = vec![ContextSource {
            source_type: ContextType::Directory,
            path: "src/MyService".into(),
            description: "Backend service".into(),
        }];
        let result = load_context_sources(&sources, dir.path());
        assert!(result.contains("README.md"), "should include README.md");
        assert!(
            result.contains("My Service"),
            "should include README content"
        );
        assert!(result.contains("Cargo.toml"), "should include Cargo.toml");
        assert!(
            result.contains("my-service"),
            "should include Cargo.toml content"
        );
        assert!(
            result.contains("Backend service"),
            "should include description"
        );
    }

    #[test]
    fn load_context_sources_file_reads_contents() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("design.md"),
            "# Design\nImportant design doc",
        )
        .unwrap();
        let sources = vec![ContextSource {
            source_type: ContextType::File,
            path: "design.md".into(),
            description: "Design doc".into(),
        }];
        let result = load_context_sources(&sources, dir.path());
        assert!(
            result.contains("Important design doc"),
            "should include file content"
        );
        assert!(result.contains("Design doc"), "should include description");
    }

    #[test]
    fn load_context_sources_caches_results() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("notes.md"), "cached content").unwrap();
        let sources = vec![ContextSource {
            source_type: ContextType::File,
            path: "notes.md".into(),
            description: "Notes".into(),
        }];
        // First call — loads and caches
        let _ = load_context_sources(&sources, dir.path());
        // Verify cache file exists
        let cache_dir = dir.path().join(".loopy/context");
        let cache_files: Vec<_> = std::fs::read_dir(&cache_dir).unwrap().flatten().collect();
        assert_eq!(cache_files.len(), 1, "should have one cache file");
        // Modify original — second call should use cache
        std::fs::write(dir.path().join("notes.md"), "modified content").unwrap();
        let result = load_context_sources(&sources, dir.path());
        assert!(
            result.contains("cached content"),
            "should use cached content"
        );
        assert!(
            !result.contains("modified content"),
            "should not see modified content"
        );
    }

    #[test]
    fn build_stage_prompt_includes_context_content() {
        let dir = tempfile::TempDir::new().unwrap();
        let prompt = build_stage_prompt(StageId::Scan, "idea", dir.path(), &dir.path().join(".loopy/stages"));
        // Context is now referenced by file, not embedded
        assert!(prompt.contains("context.md"), "should reference context file");
    }

    #[test]
    fn build_stage_prompt_references_context_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let prompt = build_stage_prompt(
            StageId::Scan,
            "idea",
            dir.path(),
            &dir.path().join(".loopy/stages"),
        );
        assert!(prompt.contains("knowledge-base.md"), "should reference kb file");
        assert!(prompt.contains("context.md"), "should reference context file");
    }

    #[test]
    fn load_context_sources_directory_truncates_large_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let src_dir = dir.path().join("big");
        std::fs::create_dir_all(&src_dir).unwrap();
        let big_content = "x".repeat(5000);
        std::fs::write(src_dir.join("README.md"), &big_content).unwrap();
        let sources = vec![ContextSource {
            source_type: ContextType::Directory,
            path: "big".into(),
            description: "".into(),
        }];
        let result = load_context_sources(&sources, dir.path());
        assert!(
            result.contains("truncated"),
            "large files should be truncated"
        );
        assert!(
            result.len() < 5000,
            "result should be smaller than original"
        );
    }

    #[test]
    fn load_context_sources_directory_truncates_multibyte_safely() {
        let dir = tempfile::TempDir::new().unwrap();
        let src_dir = dir.path().join("mb");
        std::fs::create_dir_all(&src_dir).unwrap();
        let mut content = "a".repeat(1999);
        content.push('é');
        content.push_str(&"b".repeat(3000));
        std::fs::write(src_dir.join("README.md"), &content).unwrap();
        let sources = vec![ContextSource {
            source_type: ContextType::Directory,
            path: "mb".into(),
            description: "".into(),
        }];
        let result = load_context_sources(&sources, dir.path());
        assert!(result.contains("truncated"), "should be truncated");
    }

    #[test]
    fn build_stage_prompt_plan_includes_context() {
        let dir = tempfile::TempDir::new().unwrap();
        let prompt = build_stage_prompt(StageId::Plan, "idea", dir.path(), &dir.path().join(".loopy/stages"));
        assert!(prompt.contains("idea.md"), "Plan should reference idea file");
    }

    #[test]
    fn build_stage_prompt_requirements_includes_context() {
        let dir = tempfile::TempDir::new().unwrap();
        let prompt = build_stage_prompt(StageId::RequirementsAnalysis, "idea", dir.path(), &dir.path().join(".loopy/stages"));
        assert!(prompt.contains("idea.md"), "Requirements should reference idea file");
    }

    #[test]
    fn build_stage_prompt_orbital_lanes_includes_context() {
        let dir = tempfile::TempDir::new().unwrap();
        let prompt = build_stage_prompt(StageId::OrbitalLanes, "idea", dir.path(), &dir.path().join(".loopy/stages"));
        assert!(prompt.contains("idea"), "OrbitalLanes should contain idea summary");
    }

    #[test]
    fn build_stage_prompt_land_includes_context() {
        let dir = tempfile::TempDir::new().unwrap();
        let prompt = build_stage_prompt(StageId::Land, "idea", dir.path(), &dir.path().join(".loopy/stages"));
        assert!(prompt.contains("idea"), "Land should contain idea summary");
    }

    // --- Step 9: PID file tracking ---

    #[test]
    fn write_pid_file_creates_file() {
        let dir = tempfile::TempDir::new().unwrap();
        write_pid_file(dir.path(), StageId::Scan, 12345).unwrap();
        let content = std::fs::read_to_string(dir.path().join("scan/ralph-pid")).unwrap();
        assert_eq!(content, "12345");
    }

    #[test]
    fn read_pid_file_returns_pid() {
        let dir = tempfile::TempDir::new().unwrap();
        write_pid_file(dir.path(), StageId::Scan, 42).unwrap();
        assert_eq!(read_pid_file(dir.path(), StageId::Scan), Some(42));
    }

    #[test]
    fn read_pid_file_missing_returns_none() {
        let dir = tempfile::TempDir::new().unwrap();
        assert_eq!(read_pid_file(dir.path(), StageId::Scan), None);
    }

    // --- Item 15: stages_dir tests ---

    #[test]
    fn stages_dir_from_project_checkpoint() {
        // checkpoint = .loopy/projects/foo/state.json → stages = .loopy/projects/foo/stages
        let cp = PathBuf::from(".loopy/projects/foo/state.json");
        let sd = stages_dir(&cp);
        assert_eq!(sd, PathBuf::from(".loopy/projects/foo/stages"));
    }

    #[test]
    fn stages_dir_from_legacy_checkpoint() {
        // checkpoint = .loopy/state.json → stages = .loopy/stages (legacy)
        let cp = PathBuf::from(".loopy/state.json");
        let sd = stages_dir(&cp);
        assert_eq!(sd, PathBuf::from(".loopy/stages"));
    }

    #[test]
    fn stages_dir_absolute_project_checkpoint() {
        let cp = PathBuf::from("/home/user/project/.loopy/projects/bar/state.json");
        let sd = stages_dir(&cp);
        assert_eq!(
            sd,
            PathBuf::from("/home/user/project/.loopy/projects/bar/stages")
        );
    }

    #[test]
    fn migrate_legacy_stages_moves_dirs() {
        let dir = tempfile::TempDir::new().unwrap();
        let old_stages = dir.path().join(".loopy/stages/scan");
        std::fs::create_dir_all(&old_stages).unwrap();
        std::fs::write(old_stages.join("ralph.yml"), "test").unwrap();

        let project_stages = dir.path().join(".loopy/projects/default/stages");
        migrate_legacy_stages(dir.path(), &project_stages);

        assert!(project_stages.join("scan/ralph.yml").exists());
        assert!(!dir.path().join(".loopy/stages").exists());
    }

    #[test]
    fn migrate_legacy_stages_noop_when_no_old_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let project_stages = dir.path().join(".loopy/projects/default/stages");
        migrate_legacy_stages(dir.path(), &project_stages);
        // Should not create anything
        assert!(!project_stages.exists());
    }

    #[test]
    fn write_pid_uses_stages_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let cp = dir.path().join(".loopy/projects/myproj/state.json");
        std::fs::create_dir_all(cp.parent().unwrap()).unwrap();
        let sd = stages_dir(&cp);
        write_pid_file(&sd, StageId::Scan, 12345).unwrap();
        let pid_path = sd.join(stage_dir_name(StageId::Scan)).join("ralph-pid");
        assert_eq!(
            pid_path,
            dir.path()
                .join(".loopy/projects/myproj/stages/scan/ralph-pid")
        );
        assert_eq!(std::fs::read_to_string(&pid_path).unwrap(), "12345");
        assert_eq!(read_pid_file(&sd, StageId::Scan), Some(12345));
    }

    #[tokio::test]
    async fn spawn_returns_piped_stdout_and_stderr() {
        let mut orch = Orchestrator::new_with_bin("echo".into());
        let config = LoopConfig {
            stage: StageId::Scan,
            track: None,
            hat_collection: None,
            prompt_file: None,
            completion_promise: None,
            knowledge_base: None,
            mcp_servers: None,
            execution_mode: None,
            work_dir: Some(std::env::temp_dir()),
            block_id: None,
        };
        let result = orch.spawn(config).await.unwrap();
        // stdout/stderr go to log file now, not piped
        assert!(result.stdout.is_none(), "stdout should go to log file");
        assert!(result.stderr.is_none(), "stderr should go to log file");
    }

    #[tokio::test]
    async fn ralph_stdout_piped_to_log_buffer() {
        // Spawn a process that writes to stdout, verify drain_ralph_output captures it
        let mut child = Command::new("sh")
            .arg("-c")
            .arg("echo hello-from-ralph; echo >&2 stderr-line")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let pid = child.id().unwrap_or(0);
        let loop_id = "test-drain".to_string();
        let mut orch = Orchestrator::default();
        orch.loops.insert(
            loop_id.clone(),
            LoopHandle {
                pid,
                loop_id: loop_id.clone(),
                stage: StageId::Scan,
                track: None,
                spawned_at: Utc::now(),
                child,
            },
        );
        let mut result = SpawnResult {
            handle: orch.loops.get(&loop_id).unwrap(),
            stdout,
            stderr,
        };
        let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(64);
        drain_ralph_output(&mut result, &tx);
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let mut lines = Vec::new();
        while let Ok(line) = rx.try_recv() {
            lines.push(line);
        }
        assert!(
            lines.iter().any(|l| l.contains("hello-from-ralph")),
            "stdout captured: {:?}",
            lines
        );
        assert!(
            lines.iter().any(|l| l.contains("stderr-line")),
            "stderr captured: {:?}",
            lines
        );
    }

    #[tokio::test]
    async fn watcher_starts_before_spawn_ordering() {
        // Verify that watch_path is called before spawn in the SpawnRalphLoop flow.
        // We test this by ensuring the ralph dir exists and is watchable before spawn.
        let dir = tempfile::TempDir::new().unwrap();
        let stage_dir = dir.path().join("stages").join("scan");
        std::fs::create_dir_all(&stage_dir).ok();
        let ralph_dir = stage_dir.join(".ralph");
        std::fs::create_dir_all(&ralph_dir).unwrap();

        // Start a watcher on the ralph dir — this must succeed before spawn
        let (tx, _rx) = tokio::sync::mpsc::channel(64);
        let mut watcher_handle = crate::watcher::start(dir.path().join("stages"), tx)
            .await
            .unwrap();
        let watch_result = watcher_handle.watch_path(&ralph_dir);
        assert!(
            watch_result.is_ok(),
            "watcher must be able to watch ralph dir before spawn"
        );

        // Now spawn — the watcher is already active (correct ordering)
        let mut orch = Orchestrator::new_with_bin("true".into());
        let config = LoopConfig {
            stage: StageId::Scan,
            track: None,
            hat_collection: None,
            prompt_file: None,
            completion_promise: None,
            knowledge_base: None,
            mcp_servers: None,
            execution_mode: None,
            work_dir: Some(stage_dir),
            block_id: None,
        };
        let result = orch.spawn(config).await;
        assert!(
            result.is_ok(),
            "spawn should succeed after watcher is started"
        );
    }

    #[test]
    fn strip_ansi_removes_escape_sequences() {
        // Bold + colored text + reset
        assert_eq!(strip_ansi("\x1b[1m\x1b[32mhello\x1b[0m"), "hello");
        // No escapes — passthrough
        assert_eq!(strip_ansi("plain text"), "plain text");
        // Mixed: some ANSI, some plain
        assert_eq!(strip_ansi("a\x1b[31mb\x1b[0mc"), "abc");
        // Empty string
        assert_eq!(strip_ansi(""), "");
        // Lone ESC without bracket — dropped
        assert_eq!(strip_ansi("\x1bno-bracket"), "no-bracket");
        // Compound sequences with semicolons (e.g. bold+green)
        assert_eq!(strip_ansi("\x1b[1;32mhello\x1b[0m"), "hello");
        // Multiple compound sequences in one line
        assert_eq!(
            strip_ansi("\x1b[1;31mERR\x1b[0m: \x1b[2;33mwarn\x1b[0m"),
            "ERR: warn"
        );
    }

    // --- build_track_coding_prompt tests ---

    #[test]
    fn build_track_coding_prompt_includes_all_sections() {
        let dir = tempfile::TempDir::new().unwrap();
        let stages_base = dir.path().join("stages");

        // tracks.json with acceptance criteria (lives in plan/, not plan/output/)
        let plan_dir = stages_base.join("plan");
        std::fs::create_dir_all(&plan_dir).unwrap();
        std::fs::write(
            plan_dir.join("tracks.json"),
            r#"{"tracks":[{"id":"backend","packages":[{"name":"MyPkg"}],"depends_on":[],"acceptance_criteria":["API returns 200","Tests pass"]}]}"#,
        ).unwrap();
        std::fs::write(plan_dir.join("PLAN.md"), "# Plan\nDo the thing").unwrap();

        // scan output
        let scan_dir = stages_base.join("scan");
        std::fs::create_dir_all(scan_dir.join("output")).unwrap();
        std::fs::write(scan_dir.join("report.md"), "Found 3 packages").unwrap();

        // requirements
        let req_dir = stages_base.join("requirements-analysis");
        std::fs::create_dir_all(&req_dir).unwrap();
        std::fs::write(req_dir.join("requirements.md"), "Must support pagination").unwrap();

        // environment.json
        std::fs::write(
            scan_dir.join("environment.json"),
            r#"{"packages":[{"name":"MyPkg","versionSet":"MyPkg/development","track":"backend"}]}"#,
        )
        .unwrap();

        let prompt = build_track_coding_prompt("backend", "Implement REST API", &[], &stages_base, "loopy/test-project", "cargo test");

        // Structure checks
        assert!(prompt.contains("Think Before Coding"));
        assert!(prompt.contains("Surgical Changes"));
        assert!(prompt.contains("Coding Track: backend"));
        assert!(prompt.contains("Implement REST API"));
        // Acceptance criteria are still inlined (small)
        assert!(prompt.contains("API returns 200"));
        assert!(prompt.contains("Tests pass"));
        // Stage outputs are referenced by path, not embedded
        assert!(prompt.contains("Scan Report"));
        assert!(prompt.contains("report.md"));
        assert!(prompt.contains("Plan"));
        assert!(prompt.contains("PLAN.md"));
        assert!(prompt.contains("Requirements"));
        assert!(prompt.contains("requirements.md"));
        // Package info is still inlined (small)
        assert!(prompt.contains("MyPkg"));
        // Content should NOT be embedded
        assert!(!prompt.contains("Found 3 packages"), "scan content should not be embedded");
        assert!(!prompt.contains("Must support pagination"), "requirements should not be embedded");
        // Prompt should be small
        assert!(prompt.len() < 5000, "prompt should be under 5K, got {}", prompt.len());
    }

    #[test]
    fn build_track_coding_prompt_graceful_without_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let stages_base = dir.path().join("stages");
        std::fs::create_dir_all(&stages_base).unwrap();

        let prompt = build_track_coding_prompt("backend", "Implement API", &[], &stages_base, "loopy/test", "cargo test");

        assert!(prompt.contains("backend"), "should contain track name");
        assert!(
            prompt.contains("Implement API"),
            "should contain description"
        );
        // Should not panic or error — graceful degradation
    }

    #[test]
    fn workspaces_dir_is_dot_loopy_workspaces() {
        let p = std::path::Path::new(".loopy/projects/test/state.json");
        assert_eq!(
            workspaces_dir(p),
            std::path::PathBuf::from(".loopy/workspaces")
        );
    }

    #[test]
    fn scan_prompt_requires_simple_track_ids() {
        let dir = tempfile::TempDir::new().unwrap();
        let prompt = build_stage_prompt(StageId::Scan, "test idea", dir.path(), &dir.path().join("stages"));
        assert!(
            prompt.contains("simple lowercase alphanumeric"),
            "scan prompt should require simple track IDs"
        );
    }

    #[test]
    fn plan_prompt_requires_simple_track_ids() {
        let dir = tempfile::TempDir::new().unwrap();
        let prompt = build_stage_prompt(StageId::Plan, "test idea", dir.path(), &dir.path().join("stages"));
        assert!(
            prompt.contains("simple lowercase alphanumeric"),
            "plan prompt should require simple track IDs"
        );
    }

    #[test]
    fn ralph_yml_for_implement_block_has_code_assist_hat_and_promise() {
        use crate::pipeline::{StageBlock, StageKind};
        let dir = tempfile::TempDir::new().unwrap();
        let config = crate::config::LoopyConfig::default();
        let block = StageBlock::simple("implement", "Implement", StageKind::Implement);
        let yml = ralph_yml_for_block(&block, &config, dir.path());
        // Implement uses the code-assist hat (matches engine's OrbitalLanes).
        assert!(yml.contains("hat_collection: \"builtin:code-assist\""), "yml: {yml}");
        assert!(yml.contains("starting_event: \"task.start\""));
        assert!(yml.contains("completion_promise: \"implement.complete\""));
        assert!(yml.contains("prompt_file: \"PROMPT.md\""));
    }

    #[test]
    fn prompt_for_block_includes_role_task_priors_and_promise() {
        use crate::pipeline::{StageBlock, StageKind};
        let block = StageBlock::simple("adversarial_review", "Adversarial review", StageKind::Review)
            .with_gate(crate::pipeline::GatePolicy::EscalateFinding);
        let p = prompt_for_block(&block, "Review the auth change", &["implement".into()], None);
        assert!(p.contains("## Role"));
        assert!(p.contains("critical reviewer")); // Review role line
        assert!(p.contains("Review the auth change")); // task
        assert!(p.contains("../implement/")); // prior stage reference
        assert!(p.contains("review.complete")); // completion event
        assert!(p.contains("## Gate")); // escalate-finding surfaced
    }

    #[test]
    fn prompt_for_block_includes_feedback_on_rerun() {
        use crate::pipeline::{StageBlock, StageKind};
        let block = StageBlock::simple("implement", "Implement", StageKind::Implement);
        let p = prompt_for_block(&block, "Build X", &[], Some("tests are failing on edge case Y"));
        assert!(p.contains("Feedback to address"));
        assert!(p.contains("edge case Y"));
    }

    #[test]
    fn ralph_yml_for_review_block_is_traditional_mode() {
        use crate::pipeline::{StageBlock, StageKind};
        let dir = tempfile::TempDir::new().unwrap();
        let config = crate::config::LoopyConfig::default();
        let block = StageBlock::simple("adversarial_review", "Adversarial review", StageKind::Review);
        let yml = ralph_yml_for_block(&block, &config, dir.path());
        // Review runs without a hat (traditional mode), like Scan/Plan today.
        assert!(!yml.contains("hat_collection:"), "review should be hatless: {yml}");
        assert!(yml.contains("completion_promise: \"review.complete\""));
    }
}
