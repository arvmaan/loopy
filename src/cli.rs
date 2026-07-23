use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "loopy", about = "Loop coding agents through a composable pipeline of blocks")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug, PartialEq)]
pub enum Command {
    /// Start the web UI — create and manage pipelines in the browser
    Start {
        /// Idea text (optional — can also create projects in the UI)
        idea: Option<String>,
        /// Project name (auto-derived from idea if omitted)
        #[arg(long)]
        name: Option<String>,
        /// Web server port
        #[arg(long, default_value = "3000")]
        port: u16,
        /// Don't open browser
        #[arg(long)]
        no_open: bool,
    },
    /// List all projects
    List,
    /// Print pipeline status (non-interactive)
    Status {
        /// Project name
        #[arg(long)]
        name: Option<String>,
    },
    /// Bootstrap a new Loopy project
    Init,
    /// Kill agent processes and remove the .loopy/ directory
    Clean {
        /// Keep state.json (only remove stages)
        #[arg(long)]
        keep_state: bool,
        /// Only clean a specific stage (by dir name, e.g. scan, plan)
        #[arg(long)]
        stage: Option<String>,
    },
    /// Run preflight checks
    Doctor,
    /// List all pipeline blocks, or smoke-test that each block produces a valid
    /// agent config + prompt (dry-run, spawns nothing).
    Blocks {
        /// Validate every block's config/prompt generation (no processes spawned).
        #[arg(long)]
        check: bool,
    },
}

#[derive(Subcommand, Debug, PartialEq)]
pub enum TracksAction {
    /// Add a custom track definition
    Add {
        /// Track name (used as filename)
        #[arg(long)]
        name: String,
        /// Track description
        #[arg(long)]
        description: Option<String>,
        /// Hat to use for this track
        #[arg(long)]
        hat: Option<String>,
        /// Tracks this depends on
        #[arg(long)]
        depends_on: Vec<String>,
    },
    /// Create a new track from feedback (generates a PROMPT.md for Ralph)
    Create {
        /// Track name
        #[arg(long)]
        name: String,
        /// Feedback or task description
        #[arg(long)]
        feedback: String,
        /// Base track to iterate on (reuses its workspace)
        #[arg(long)]
        from: Option<String>,
        /// Packages to include (comma-separated)
        #[arg(long)]
        packages: Option<String>,
    },
    /// Set up workspaces for tracks (all tracks if no name given)
    Setup {
        /// Track name (omit to setup all tracks)
        name: Option<String>,
    },
    /// Launch coding Ralph for a track (all tracks if no name given)
    Launch {
        /// Track name (omit to launch all tracks)
        name: Option<String>,
    },
    /// List tracks and their status
    List,
}

/// Create a fresh PipelineState with all stages pending except Idea (Running).
pub fn fresh_pipeline(mode: crate::models::ExecutionMode) -> crate::models::PipelineState {
    use crate::models::*;
    use chrono::Utc;

    let now = Utc::now();
    PipelineState {
        version: 2,
        idea_text: String::new(),
        stages: vec![
            StageState {
                id: StageId::Idea,
                status: StageStatus::Running,
                loop_id: None,
                loop_pid: None,
                started_at: Some(now),
                completed_at: None,
                error: None,
            },
            StageState {
                id: StageId::Scan,
                status: StageStatus::Pending,
                loop_id: None,
                loop_pid: None,
                started_at: None,
                completed_at: None,
                error: None,
            },
            StageState {
                id: StageId::Plan,
                status: StageStatus::Pending,
                loop_id: None,
                loop_pid: None,
                started_at: None,
                completed_at: None,
                error: None,
            },
            StageState {
                id: StageId::RequirementsAnalysis,
                status: StageStatus::Pending,
                loop_id: None,
                loop_pid: None,
                started_at: None,
                completed_at: None,
                error: None,
            },
            StageState {
                id: StageId::OrbitalLanes,
                status: StageStatus::Pending,
                loop_id: None,
                loop_pid: None,
                started_at: None,
                completed_at: None,
                error: None,
            },
            StageState {
                id: StageId::Land,
                status: StageStatus::Pending,
                loop_id: None,
                loop_pid: None,
                started_at: None,
                completed_at: None,
                error: None,
            },
        ],
        tracks: None,
        created_at: now,
        updated_at: now,
        execution_mode: mode,
        artifact_registry: ArtifactRegistry::default(),
        active_conflicts: vec![],
        awaiting_approval: None,
    }
}

/// Create a canned PipelineState for demo mode.
pub fn fresh_demo_pipeline() -> crate::models::PipelineState {
    use crate::models::*;
    use chrono::Utc;

    let now = Utc::now();
    let earlier = now - chrono::Duration::seconds(120);
    PipelineState {
        version: 2,
        idea_text: "Demo: explore the Loopy TUI".into(),
        stages: vec![
            StageState {
                id: StageId::Idea,
                status: StageStatus::Complete,
                loop_id: None,
                loop_pid: None,
                started_at: Some(earlier),
                completed_at: Some(earlier + chrono::Duration::seconds(10)),
                error: None,
            },
            StageState {
                id: StageId::Scan,
                status: StageStatus::Complete,
                loop_id: None,
                loop_pid: None,
                started_at: Some(earlier + chrono::Duration::seconds(10)),
                completed_at: Some(earlier + chrono::Duration::seconds(30)),
                error: None,
            },
            StageState {
                id: StageId::Plan,
                status: StageStatus::Complete,
                loop_id: None,
                loop_pid: None,
                started_at: Some(earlier + chrono::Duration::seconds(30)),
                completed_at: Some(earlier + chrono::Duration::seconds(60)),
                error: None,
            },
            StageState {
                id: StageId::RequirementsAnalysis,
                status: StageStatus::Running,
                loop_id: None,
                loop_pid: None,
                started_at: Some(earlier + chrono::Duration::seconds(60)),
                completed_at: None,
                error: None,
            },
            StageState {
                id: StageId::OrbitalLanes,
                status: StageStatus::Pending,
                loop_id: None,
                loop_pid: None,
                started_at: None,
                completed_at: None,
                error: None,
            },
            StageState {
                id: StageId::Land,
                status: StageStatus::Pending,
                loop_id: None,
                loop_pid: None,
                started_at: None,
                completed_at: None,
                error: None,
            },
        ],
        tracks: Some(vec![
            TrackState {
                id: "backend".into(),
                name: "Backend".into(),
                status: TrackStatus::Running,
                loop_id: None,
                loop_pid: None,
                current_sub_stage: None,
                depends_on: vec![],
                blocking_artifact: None,
                consumed_versions: vec![],
                run_count: 0,
                medium: TrackMedium::default(),
                review_status: Default::default(),
                cr_url: None,
            },
            TrackState {
                id: "console".into(),
                name: "Console".into(),
                status: TrackStatus::Pending,
                loop_id: None,
                loop_pid: None,
                current_sub_stage: None,
                depends_on: vec![],
                blocking_artifact: None,
                consumed_versions: vec![],
                run_count: 0,
                medium: TrackMedium::default(),
                review_status: Default::default(),
                cr_url: None,
            },
            TrackState {
                id: "frontend".into(),
                name: "Frontend".into(),
                status: TrackStatus::Pending,
                loop_id: None,
                loop_pid: None,
                current_sub_stage: None,
                depends_on: vec![],
                blocking_artifact: None,
                consumed_versions: vec![],
                run_count: 0,
                medium: TrackMedium::default(),
                review_status: Default::default(),
                cr_url: None,
            },
        ]),
        created_at: earlier,
        updated_at: now,
        execution_mode: ExecutionMode::Safe,
        artifact_registry: ArtifactRegistry::default(),
        active_conflicts: vec![],
        awaiting_approval: None,
    }
}

/// Load pipeline state for resume. Returns fresh if missing/corrupt.
pub fn load_or_fresh(checkpoint_path: &std::path::Path) -> crate::models::PipelineState {
    crate::checkpoint::load(checkpoint_path)
        .ok()
        .flatten()
        .unwrap_or_else(|| fresh_pipeline(Default::default()))
}

/// Resolve the checkpoint path for a project.
/// `None` → `.loopy/state.json` (legacy/default)
/// `Some(name)` → `.loopy/projects/<name>/state.json`
/// Resolve project name: explicit `--name` > idea slug > loopy.yml `project` field.
pub fn resolve_project_name(
    name: Option<String>,
    idea_text: &str,
    config_project: Option<String>,
) -> Option<String> {
    // Priority: --name > loopy.yml project > idea slug
    name.or(config_project).or_else(|| {
        let slug = slugify(idea_text);
        if slug.is_empty() { None } else { Some(slug) }
    })
}

pub fn project_path(name: Option<&str>) -> std::path::PathBuf {
    match name {
        None => std::path::PathBuf::from(".loopy/state.json"),
        Some(n) => std::path::PathBuf::from(format!(".loopy/projects/{n}/state.json")),
    }
}

/// Slugify text for use as a project name: lowercase, non-alnum→hyphens, collapse, truncate to 30.
pub fn slugify(text: &str) -> String {
    let s: String = text
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let s: String = s
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    s.chars()
        .take(30)
        .collect::<String>()
        .trim_end_matches('-')
        .to_string()
}

/// List projects under the given `.loopy` base directory. Testable variant.
pub fn list_projects_in(base: &std::path::Path) -> String {
    let projects_dir = base.join("projects");
    let mut entries: Vec<(String, crate::models::PipelineState)> = Vec::new();

    if projects_dir.is_dir() {
        if let Ok(rd) = std::fs::read_dir(&projects_dir) {
            for entry in rd.flatten() {
                let state_path = entry.path().join("state.json");
                if let Some(state) = crate::checkpoint::load(&state_path).ok().flatten()
                    && let Some(name) = entry.file_name().to_str()
                {
                    entries.push((name.to_string(), state));
                }
            }
        }
    } else if base.join("state.json").exists()
        && let Some(state) = crate::checkpoint::load(&base.join("state.json"))
            .ok()
            .flatten()
    {
        entries.push(("default".to_string(), state));
    }

    if entries.is_empty() {
        return "No projects found\n".to_string();
    }

    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let color = std::io::IsTerminal::is_terminal(&std::io::stdout());
    let mut out = String::new();
    for (name, state) in &entries {
        let current = state
            .stages
            .iter()
            .find(|s| s.status == crate::models::StageStatus::Running)
            .or_else(|| state.stages.last());
        let (stage_label, icon) = current
            .map(|s| {
                let (_, label, _) = STAGE_INFO.iter().find(|(id, _, _)| *id == s.id).unwrap();
                (*label, status_icon(s.status))
            })
            .unwrap_or(("?", "?"));
        let ts = state.updated_at.format("%Y-%m-%d %H:%M:%S UTC");
        if color {
            out.push_str(&format!(
                "  {icon} {BOLD}{name}{RESET}  {stage_label}  {GRAY}{ts}{RESET}\n"
            ));
        } else {
            out.push_str(&format!("  {icon} {name}  {stage_label}  {ts}\n"));
        }
    }
    out
}

/// List projects (production entry point, uses `.loopy` as base).
pub fn list_projects() -> String {
    list_projects_in(std::path::Path::new(".loopy"))
}

/// Write a track definition YAML file to `.loopy/tracks/<name>.yml`.
pub fn handle_tracks_add(
    root: &std::path::Path,
    name: &str,
    description: Option<&str>,
    hat: Option<&str>,
    depends_on: &[String],
) -> anyhow::Result<()> {
    let tracks_dir = root.join(".loopy").join("tracks");
    std::fs::create_dir_all(&tracks_dir)?;
    let def = crate::models::TrackDefinition {
        name: name.into(),
        icon: "🔧".into(),
        description: description.unwrap_or("").into(),
        hat: hat.unwrap_or("builtin:code-assist").into(),
        dependencies: depends_on.to_vec(),
        artifacts_produced: vec![],
        approval: crate::models::ApprovalMode::Auto,
        enabled: true,
        medium: crate::models::TrackMedium::Brazil,
        medium_config: crate::models::MediumConfig::default(),
        max_iterations: None,
    };
    let yaml = serde_yaml::to_string(&def)?;
    std::fs::write(tracks_dir.join(format!("{name}.yml")), yaml)?;
    Ok(())
}

const STAGE_INFO: [(crate::models::StageId, &str, &str); 6] = {
    use crate::models::StageId::*;
    [
        (Idea, "Idea", "🚀"),
        (Scan, "Scan", "🔭"),
        (Plan, "Plan", "📋"),
        (RequirementsAnalysis, "Req Analysis", "🔬"),
        (OrbitalLanes, "Orbital Lanes", "🛸"),
        (Land, "Land", "🌍"),
    ]
};

fn status_icon(status: crate::models::StageStatus) -> &'static str {
    match status {
        crate::models::StageStatus::Complete => "✅",
        crate::models::StageStatus::Running => "⏳",
        crate::models::StageStatus::Failed => "❌",
        crate::models::StageStatus::Pending => "○",
    }
}

fn track_status_icon(status: crate::models::TrackStatus) -> &'static str {
    use crate::models::TrackStatus::*;
    match status {
        Complete => "✅",
        Running => "⏳",
        Failed => "❌",
        Pending => "○",
        PendingSetup => "🔧",
        SettingUp => "🔧",
        Skipped => "⏭️",
        Blocked => "🚫",
        Invalidated => "♻️",
        ConflictPaused => "⏸️",
    }
}

fn format_elapsed(secs: i64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else {
        format!("{}m {}s", secs / 60, secs % 60)
    }
}

fn is_pid_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn status_color(status: crate::models::StageStatus) -> &'static str {
    match status {
        crate::models::StageStatus::Complete => "\x1b[32m", // green
        crate::models::StageStatus::Running => "\x1b[33m",  // yellow
        crate::models::StageStatus::Failed => "\x1b[31m",   // red
        crate::models::StageStatus::Pending => "\x1b[90m",  // gray
    }
}

const RESET: &str = "\x1b[0m";
const GRAY: &str = "\x1b[90m";
const BOLD: &str = "\x1b[1m";

/// Format pipeline status. When `color` is true, wraps output in ANSI codes.
pub fn format_status(state: &crate::models::PipelineState, color: bool) -> String {
    use chrono::Utc;
    let now = Utc::now();
    let mut out = String::new();

    for stage in &state.stages {
        let (_, label, icon) = STAGE_INFO
            .iter()
            .find(|(id, _, _)| *id == stage.id)
            .unwrap();
        let icon_s = status_icon(stage.status);
        if color {
            out.push_str(status_color(stage.status));
        }
        out.push_str(&format!("{icon_s} {icon} {label}"));
        if color {
            out.push_str(RESET);
        }
        if stage.status == crate::models::StageStatus::Running
            && let Some(started) = stage.started_at
        {
            let elapsed = (now - started).num_seconds().max(0);
            if color {
                out.push_str(&format!(
                    "  \x1b[33mRunning for {}{RESET}",
                    format_elapsed(elapsed)
                ));
            } else {
                out.push_str(&format!("  Running for {}", format_elapsed(elapsed)));
            }
        }
        out.push('\n');
    }

    if let Some(tracks) = &state.tracks {
        if color {
            out.push_str(&format!("\n{BOLD}Tracks:{RESET}\n"));
        } else {
            out.push_str("\nTracks:\n");
        }
        for t in tracks {
            let icon = track_status_icon(t.status);
            let pid_status = if let Some(pid) = t.loop_pid {
                if is_pid_alive(pid) {
                    format!(" (PID {pid})")
                } else {
                    " (💀 dead)".to_string()
                }
            } else {
                String::new()
            };
            out.push_str(&format!("  {icon} {}{pid_status}", t.name));
            if let Some(sub) = &t.current_sub_stage {
                if color {
                    out.push_str(&format!(" {GRAY}({sub}){RESET}"));
                } else {
                    out.push_str(&format!(" ({sub})"));
                }
            }
            if let Some(url) = &t.cr_url {
                if color {
                    out.push_str(&format!(" {GRAY}CR: {url}{RESET}"));
                } else {
                    out.push_str(&format!(" CR: {url}"));
                }
            }
            out.push('\n');
        }
    }

    let ts = state.updated_at.format("%Y-%m-%d %H:%M:%S UTC");
    if color {
        out.push_str(&format!("\n{GRAY}Updated: {ts}{RESET}\n"));
    } else {
        out.push_str(&format!("\nUpdated: {ts}\n"));
    }

    let mut alive = 0u32;
    for s in &state.stages {
        if let Some(pid) = s.loop_pid
            && is_pid_alive(pid)
        {
            alive += 1;
        }
    }
    if let Some(tracks) = &state.tracks {
        for t in tracks {
            if let Some(pid) = t.loop_pid
                && is_pid_alive(pid)
            {
                alive += 1;
            }
        }
    }
    out.push_str(&format!("Active processes: {alive}\n"));

    // Next command hint
    use crate::models::{StageId, StageStatus, TrackStatus};
    let scan_running = state
        .stages
        .iter()
        .any(|s| s.id == StageId::Scan && s.status == StageStatus::Running);
    let plan_running = state
        .stages
        .iter()
        .any(|s| s.id == StageId::Plan && s.status == StageStatus::Running);
    let plan_done = state
        .stages
        .iter()
        .any(|s| s.id == StageId::Plan && s.status == StageStatus::Complete);
    let req_pending = state
        .stages
        .iter()
        .any(|s| s.id == StageId::RequirementsAnalysis && s.status == StageStatus::Pending);
    let req_running = state
        .stages
        .iter()
        .any(|s| s.id == StageId::RequirementsAnalysis && s.status == StageStatus::Running);
    let req_done = state
        .stages
        .iter()
        .any(|s| s.id == StageId::RequirementsAnalysis && s.status == StageStatus::Complete);
    let ol_pending = state
        .stages
        .iter()
        .any(|s| s.id == StageId::OrbitalLanes && s.status == StageStatus::Pending);
    let tracks_pending = state.tracks.as_ref().is_some_and(|t| {
        t.iter()
            .any(|tr| tr.status == TrackStatus::Pending || tr.status == TrackStatus::PendingSetup)
    });
    let tracks_running = state
        .tracks
        .as_ref()
        .is_some_and(|t| t.iter().any(|tr| tr.status == TrackStatus::Running));

    let hint = if scan_running || plan_running {
        "⏳ Waiting for scan/plan to complete..."
    } else if plan_done && req_pending {
        "Next: loopy review"
    } else if req_running {
        "⏳ Waiting for requirements analysis..."
    } else if req_done && ol_pending {
        "Next: loopy review --approve"
    } else if tracks_pending {
        "Next: loopy tracks setup && loopy tracks launch"
    } else if tracks_running {
        "Next: loopy logs"
    } else {
        ""
    };
    if !hint.is_empty() {
        out.push_str(&format!("\n{hint}\n"));
    }

    out
}

/// Format pipeline status as plain text (no ANSI codes). Used for piped output and testing.
pub fn format_status_plain(state: &crate::models::PipelineState) -> String {
    format_status(state, false)
}

/// Print pipeline status to stdout (non-interactive).
/// Returns colorized string for terminals, plain for piped output.
pub fn print_status(state: &crate::models::PipelineState) -> String {
    use std::io::IsTerminal;
    format_status(state, std::io::stdout().is_terminal())
}

/// Collect all ralph-output.log contents from stages and tracks with headers.
pub fn collect_logs(
    stages_dir: &std::path::Path,
    tracks: &Option<Vec<crate::models::TrackState>>,
) -> String {
    use crate::models::StageId;
    let mut out = String::new();
    for stage in [
        StageId::Idea,
        StageId::Scan,
        StageId::Plan,
        StageId::RequirementsAnalysis,
        StageId::OrbitalLanes,
        StageId::Land,
    ] {
        let log_path = stages_dir
            .join(crate::orchestrator::stage_dir_name(stage))
            .join(".ralph/ralph-output.log");
        if let Ok(content) = std::fs::read_to_string(&log_path) {
            out.push_str(&format!(
                "=== Stage: {} ===\n",
                crate::orchestrator::stage_dir_name(stage)
            ));
            out.push_str(&content);
            if !content.ends_with('\n') {
                out.push('\n');
            }
        }
    }
    if let Some(tracks) = tracks {
        for track in tracks {
            let log_path = stages_dir.join(format!(
                "orbital-lanes/{}/.ralph/ralph-output.log",
                track.id
            ));
            if let Ok(content) = std::fs::read_to_string(&log_path) {
                out.push_str(&format!("=== Track: {} ({}) ===\n", track.name, track.id));
                out.push_str(&content);
                if !content.ends_with('\n') {
                    out.push('\n');
                }
            }
        }
    }
    out
}

/// Check which running stages have dead PIDs and need respawning on resume.
/// Returns a list of StageIds whose stored PID is no longer alive.
pub fn dead_stage_pids(
    state: &crate::models::PipelineState,
    stages_base: &std::path::Path,
) -> Vec<crate::models::StageId> {
    state
        .stages
        .iter()
        .filter(|s| s.status == crate::models::StageStatus::Running)
        .filter(|s| {
            let stored_pid = s
                .loop_pid
                .or_else(|| crate::orchestrator::read_pid_file(stages_base, s.id));
            match stored_pid {
                Some(pid) => !is_pid_alive(pid),
                None => true, // no PID recorded → treat as dead
            }
        })
        .filter(|s| {
            // Don't respawn if the stage already completed (event exists in JSONL)
            !stage_has_completion_event(stages_base, s.id)
        })
        .map(|s| s.id)
        .collect()
}

/// Check if a stage's JSONL events or Ralph output log contain a completion event.
pub fn stage_has_completion_event(
    stages_base: &std::path::Path,
    stage: crate::models::StageId,
) -> bool {
    let stage_name = crate::orchestrator::stage_dir_name(stage);
    let ralph_dir = stages_base.join(stage_name).join(".ralph");
    let promises: &[&str] = match stage {
        crate::models::StageId::Scan => &["scan.complete", "scan.done", "stage.done"],
        crate::models::StageId::Plan => &["plan.complete", "plan.done", "stage.done"],
        crate::models::StageId::RequirementsAnalysis => {
            &["requirements.complete", "requirements.done", "stage.done"]
        }
        crate::models::StageId::Land => &["land.complete", "land.done", "stage.done"],
        _ => return false,
    };
    // Check JSONL events
    if let Ok(entries) = std::fs::read_dir(&ralph_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                if let Ok(contents) = std::fs::read_to_string(&path) {
                    if promises.iter().any(|p| contents.contains(p)) {
                        return true;
                    }
                }
            }
        }
    }

    // Fallback: if Ralph died but output files exist, treat as complete
    let stage_dir = stages_base.join(stage_name);
    let has_output = match stage {
        crate::models::StageId::Scan => {
            stage_dir.join("scan-report.md").exists() && stage_dir.join("environment.json").exists()
        }
        crate::models::StageId::Plan => {
            stage_dir.join("PLAN.md").exists() && stage_dir.join("tracks.json").exists()
        }
        crate::models::StageId::RequirementsAnalysis => {
            stage_dir.join("REQUIREMENTS.md").exists() || stage_dir.join("requirements.md").exists()
        }
        _ => false,
    };
    if has_output {
        return true;
    }

    false
}

/// Return Running stages whose JSONL/log already contains a completion event.
/// Used by the headless polling loop to detect completions the watcher missed.
pub fn completed_running_stages(
    state: &crate::models::PipelineState,
    stages_base: &std::path::Path,
) -> Vec<crate::models::StageId> {
    state
        .stages
        .iter()
        .filter(|s| s.status == crate::models::StageStatus::Running)
        .filter(|s| stage_has_completion_event(stages_base, s.id))
        .map(|s| s.id)
        .collect()
}

/// Check which running stages have alive PIDs and can be reattached on resume.
/// Returns a list of (StageId, pid) pairs whose stored PID is still alive.
pub fn alive_stage_pids(
    state: &crate::models::PipelineState,
    stages_base: &std::path::Path,
) -> Vec<(crate::models::StageId, u32)> {
    state
        .stages
        .iter()
        .filter(|s| s.status == crate::models::StageStatus::Running)
        .filter_map(|s| {
            let stored_pid = s
                .loop_pid
                .or_else(|| crate::orchestrator::read_pid_file(stages_base, s.id));
            match stored_pid {
                Some(pid) if is_pid_alive(pid) => Some((s.id, pid)),
                _ => None,
            }
        })
        .collect()
}

/// Kill stale Ralph loops belonging to THIS project (from a previous run) so a
/// restart doesn't stack fresh loops on orphans. Scoped to the project via the
/// `loop.lock` PID sweep in `orchestrator::kill_all_project_ralph` — it never
/// touches other projects' or other users' `ralph` processes on a shared host.
/// Returns the number of process groups signalled. Best-effort.
pub fn kill_stale_ralph(project_root: &std::path::Path) -> usize {
    crate::orchestrator::kill_all_project_ralph(project_root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    // AC3: Parse `loopy status`
    #[test]
    fn parse_status() {
        let cli = Cli::parse_from(["loopy", "status"]);
        assert!(matches!(cli.command, Command::Status { name: None }));
    }

    #[test]
    fn parse_status_with_name() {
        let cli = Cli::parse_from(["loopy", "status", "--name", "foo"]);
        match cli.command {
            Command::Status { name } => assert_eq!(name.as_deref(), Some("foo")),
            other => panic!("expected Status, got {:?}", other),
        }
    }

    // AC4: Resume from checkpoint loads saved state
    #[test]
    fn resume_from_checkpoint() {
        use crate::models::*;
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("state.json");
        let mut state = fresh_pipeline(ExecutionMode::Safe);
        state.stages[0].status = StageStatus::Complete;
        state.stages[1].status = StageStatus::Running;
        state.idea_text = "test idea".into();
        crate::checkpoint::save(&path, &state).unwrap();

        let loaded = load_or_fresh(&path);
        assert_eq!(loaded.stages[1].status, StageStatus::Running);
        assert_eq!(loaded.idea_text, "test idea");
    }

    // AC5: Resume with missing checkpoint starts fresh
    #[test]
    fn resume_missing_checkpoint_starts_fresh() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.json");
        let state = load_or_fresh(&path);
        assert_eq!(state.stages[0].status, crate::models::StageStatus::Running);
        assert_eq!(state.stages[0].id, crate::models::StageId::Idea);
    }

    // Fresh pipeline has correct structure
    #[test]
    fn fresh_pipeline_structure() {
        let state = fresh_pipeline(crate::models::ExecutionMode::Safe);
        assert_eq!(state.stages.len(), 6);
        assert_eq!(state.stages[0].status, crate::models::StageStatus::Running);
        for s in &state.stages[1..] {
            assert_eq!(s.status, crate::models::StageStatus::Pending);
        }
    }

    // Status output contains all stages
    #[test]
    fn status_output_contains_stages() {
        let state = fresh_pipeline(crate::models::ExecutionMode::Safe);
        let output = print_status(&state);
        assert!(output.contains("Idea"));
        assert!(output.contains("Scan"));
        assert!(output.contains("Plan"));
        assert!(output.contains("Req Analysis"));
        assert!(output.contains("Orbital Lanes"));
        assert!(output.contains("Land"));
    }

    // Status output shows correct icons
    #[test]
    fn status_output_correct_icons() {
        use crate::models::StageStatus;
        let mut state = fresh_pipeline(crate::models::ExecutionMode::Safe);
        state.stages[0].status = StageStatus::Complete;
        state.stages[1].status = StageStatus::Running;
        state.stages[2].status = StageStatus::Failed;
        let output = print_status(&state);
        assert!(output.contains("✅ 🚀 Idea"));
        assert!(output.contains("⏳ 🔭 Scan"));
        assert!(output.contains("❌ 📋 Plan"));
        assert!(output.contains("○ 🛸 Orbital Lanes"));
    }

    // Step 8 AC4: Fresh pipeline with Fast mode
    #[test]
    fn fresh_pipeline_with_mode() {
        let state = fresh_pipeline(crate::models::ExecutionMode::Fast);
        assert_eq!(state.version, 2);
        assert_eq!(state.stages.len(), 6);
        assert_eq!(state.execution_mode, crate::models::ExecutionMode::Fast);
        assert!(state.artifact_registry.artifacts.is_empty());
        assert!(state.active_conflicts.is_empty());
    }

    // kill_stale_ralph returns without error on a project dir with no live loops
    #[test]
    fn kill_stale_ralph_does_not_panic() {
        let dir = tempfile::TempDir::new().unwrap();
        let n = kill_stale_ralph(dir.path()); // must not panic; nothing to kill
        assert_eq!(n, 0);
    }

    // AC-Clean1: Parse `loopy clean`
    #[test]
    fn parse_clean() {
        let cli = Cli::parse_from(["loopy", "clean"]);
        assert_eq!(
            cli.command,
            Command::Clean {
                keep_state: false,
                stage: None
            }
        );
    }

    // AC-Clean2: Parse `loopy clean --keep-state`
    #[test]
    fn parse_clean_keep_state() {
        let cli = Cli::parse_from(["loopy", "clean", "--keep-state"]);
        assert_eq!(
            cli.command,
            Command::Clean {
                keep_state: true,
                stage: None
            }
        );
    }

    // AC-Clean3: loopy clean --help works (clap generates help)
    #[test]
    fn clean_help_text() {
        let result = Cli::try_parse_from(["loopy", "clean", "--help"]);
        // clap returns Err with DisplayHelp kind for --help
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::DisplayHelp);
    }

    #[test]
    fn parse_clean_with_stage() {
        let cli = Cli::parse_from(["loopy", "clean", "--stage", "scan"]);
        assert_eq!(
            cli.command,
            Command::Clean {
                keep_state: false,
                stage: Some("scan".to_string()),
            }
        );
    }

    #[test]
    fn parse_clean_stage_and_keep_state() {
        let cli = Cli::parse_from(["loopy", "clean", "--stage", "plan", "--keep-state"]);
        assert_eq!(
            cli.command,
            Command::Clean {
                keep_state: true,
                stage: Some("plan".to_string()),
            }
        );
    }

    // Step 8 AC5: Status output includes stage icons
    #[test]
    fn status_output_has_stage_icons() {
        let state = fresh_pipeline(crate::models::ExecutionMode::Safe);
        let output = print_status(&state);
        assert!(output.contains("🚀"), "missing Idea icon");
        assert!(output.contains("🔭"), "missing Scan icon");
        assert!(output.contains("📋"), "missing Plan icon");
        assert!(output.contains("🔬"), "missing RequirementsAnalysis icon");
        assert!(output.contains("🛸"), "missing OrbitalLanes icon");
        assert!(output.contains("🌍"), "missing Land icon");
    }

    // --- Step 2: Rich loopy status tests ---

    // Rich status uses human-readable stage names (not Debug format)
    #[test]
    fn rich_status_human_readable_names() {
        let state = fresh_pipeline(crate::models::ExecutionMode::Safe);
        let output = format_status_plain(&state);
        assert!(output.contains("Idea"), "missing Idea");
        assert!(output.contains("Scan"), "missing Scan");
        assert!(output.contains("Plan"), "missing Plan");
        assert!(output.contains("Req Analysis"), "missing Req Analysis");
        assert!(output.contains("Orbital Lanes"), "missing Orbital Lanes");
        assert!(output.contains("Land"), "missing Land");
    }

    // Rich status shows elapsed time for running stage
    #[test]
    fn rich_status_elapsed_time() {
        use crate::models::*;
        let mut state = fresh_pipeline(ExecutionMode::Safe);
        state.stages[0].status = StageStatus::Complete;
        state.stages[1].status = StageStatus::Running;
        state.stages[1].started_at = Some(chrono::Utc::now() - chrono::Duration::seconds(222));
        let output = format_status_plain(&state);
        assert!(
            output.contains("Running for 3m"),
            "expected elapsed time, got:\n{output}"
        );
    }

    // Rich status shows track status when in OrbitalLanes
    #[test]
    fn rich_status_with_tracks() {
        use crate::models::*;
        let mut state = fresh_pipeline(ExecutionMode::Safe);
        for s in &mut state.stages {
            s.status = StageStatus::Complete;
        }
        state.stages[4].status = StageStatus::Running; // OrbitalLanes
        state.tracks = Some(vec![
            TrackState {
                id: "backend".into(),
                name: "Backend".into(),
                status: TrackStatus::Running,
                loop_id: None,
                loop_pid: None,
                current_sub_stage: Some("coding".into()),
                depends_on: vec![],
                blocking_artifact: None,
                consumed_versions: vec![],
                run_count: 1,
                medium: TrackMedium::default(),
                review_status: Default::default(),
                cr_url: None,
            },
            TrackState {
                id: "frontend".into(),
                name: "Frontend".into(),
                status: TrackStatus::Complete,
                loop_id: None,
                loop_pid: None,
                current_sub_stage: None,
                depends_on: vec![],
                blocking_artifact: None,
                consumed_versions: vec![],
                run_count: 1,
                medium: TrackMedium::default(),
                review_status: Default::default(),
                cr_url: None,
            },
        ]);
        let output = format_status_plain(&state);
        assert!(output.contains("Backend"), "missing Backend track");
        assert!(output.contains("Frontend"), "missing Frontend track");
        assert!(output.contains("coding"), "missing sub-stage");
    }

    // Rich status shows last updated timestamp
    #[test]
    fn rich_status_last_updated() {
        let state = fresh_pipeline(crate::models::ExecutionMode::Safe);
        let output = format_status_plain(&state);
        assert!(
            output.contains("Updated:"),
            "missing last updated line, got:\n{output}"
        );
    }

    // Rich status shows active PID count
    #[test]
    fn rich_status_pid_count() {
        let state = fresh_pipeline(crate::models::ExecutionMode::Safe);
        let output = format_status_plain(&state);
        // No PIDs alive in test, should show 0
        assert!(
            output.contains("Active processes: 0"),
            "missing PID count, got:\n{output}"
        );
    }

    // Rich status mixed stage states
    #[test]
    fn rich_status_mixed_states() {
        use crate::models::*;
        let mut state = fresh_pipeline(ExecutionMode::Safe);
        state.stages[0].status = StageStatus::Complete;
        state.stages[1].status = StageStatus::Complete;
        state.stages[2].status = StageStatus::Failed;
        state.stages[2].error = Some("plan generation failed".into());
        let output = format_status_plain(&state);
        assert!(output.contains("✅"), "missing complete icon");
        assert!(output.contains("❌"), "missing failed icon");
        assert!(output.contains("○"), "missing pending icon");
    }

    // format_status_plain produces no ANSI escape codes
    #[test]
    fn plain_status_no_ansi() {
        let state = fresh_pipeline(crate::models::ExecutionMode::Safe);
        let output = format_status_plain(&state);
        assert!(
            !output.contains("\x1b["),
            "plain output should not contain ANSI codes"
        );
    }

    // --- Step 5: Multi-project CLI + project_path tests ---

    // AC3: Command::List variant exists
    #[test]
    fn parse_list() {
        let cli = Cli::parse_from(["loopy", "list"]);
        assert_eq!(cli.command, Command::List);
    }

    // AC4: project_path(None) returns .loopy/state.json
    #[test]
    fn project_path_none() {
        let p = project_path(None);
        assert_eq!(p, std::path::PathBuf::from(".loopy/state.json"));
    }

    // AC5: project_path(Some("foo")) returns .loopy/projects/foo/state.json
    #[test]
    fn project_path_some() {
        let p = project_path(Some("foo"));
        assert_eq!(
            p,
            std::path::PathBuf::from(".loopy/projects/foo/state.json")
        );
    }

    #[test]
    fn resolve_project_name_prefers_explicit_name() {
        let got = resolve_project_name(Some("explicit".into()), "some idea", Some("yml".into()));
        assert_eq!(got.as_deref(), Some("explicit"));
    }

    #[test]
    fn resolve_project_name_falls_back_to_idea_slug() {
        // loopy.yml wins over idea slug
        let got = resolve_project_name(None, "My Cool Idea", Some("yml".into()));
        assert_eq!(got.as_deref(), Some("yml"));
    }

    #[test]
    fn resolve_project_name_idea_slug_when_no_yml() {
        let got = resolve_project_name(None, "My Cool Idea", None);
        assert_eq!(got.as_deref(), Some("my-cool-idea"));
    }

    #[test]
    fn resolve_project_name_falls_back_to_loopy_yml_project() {
        let got = resolve_project_name(None, "", Some("yml-proj".into()));
        assert_eq!(got.as_deref(), Some("yml-proj"));
    }

    #[test]
    fn resolve_project_name_returns_none_when_all_empty() {
        let got = resolve_project_name(None, "", None);
        assert!(got.is_none());
    }

    #[test]
    fn resolve_project_name_loopy_yml_end_to_end() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("loopy.yml"), "project: from-yml\n").unwrap();
        let cfg = crate::config::LoopyConfig::load_or_default(dir.path());
        let got = resolve_project_name(None, "", cfg.project);
        assert_eq!(got.as_deref(), Some("from-yml"));
    }

    // format_status with color=true produces ANSI escape codes
    #[test]
    fn colorized_status_has_ansi() {
        let state = fresh_pipeline(crate::models::ExecutionMode::Safe);
        let output = format_status(&state, true);
        assert!(
            output.contains("\x1b["),
            "colorized output should contain ANSI codes"
        );
        assert!(
            output.contains("\x1b[0m"),
            "colorized output should contain reset codes"
        );
        // Content is still present
        assert!(output.contains("Idea"));
        assert!(output.contains("Orbital Lanes"));
        assert!(output.contains("Active processes:"));
    }

    // --- Step 5 task 2: slugify + list + backward compat ---

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("My Cool Project"), "my-cool-project");
    }

    #[test]
    fn slugify_special_chars() {
        assert_eq!(slugify("hello@world! 123"), "hello-world-123");
    }

    #[test]
    fn slugify_truncate() {
        let long = "a".repeat(50);
        let result = slugify(&long);
        assert!(result.len() <= 30);
    }

    #[test]
    fn slugify_empty() {
        assert_eq!(slugify(""), "");
    }

    #[test]
    fn slugify_leading_trailing_hyphens() {
        assert_eq!(slugify("--hello--"), "hello");
    }

    #[test]
    fn list_projects_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let base = dir.path().join(".loopy");
        let output = list_projects_in(&base);
        assert_eq!(output.trim(), "No projects found");
    }

    #[test]
    fn list_projects_legacy() {
        let dir = tempfile::TempDir::new().unwrap();
        let base = dir.path().join(".loopy");
        std::fs::create_dir_all(&base).unwrap();
        let state = fresh_pipeline(crate::models::ExecutionMode::Safe);
        crate::checkpoint::save(&base.join("state.json"), &state).unwrap();
        let output = list_projects_in(&base);
        assert!(
            output.contains("default"),
            "should show legacy as 'default': {output}"
        );
    }

    #[test]
    fn list_projects_multi() {
        let dir = tempfile::TempDir::new().unwrap();
        let base = dir.path().join(".loopy");
        let projects = base.join("projects");
        for name in ["foo", "bar"] {
            let pdir = projects.join(name);
            std::fs::create_dir_all(&pdir).unwrap();
            let state = fresh_pipeline(crate::models::ExecutionMode::Safe);
            crate::checkpoint::save(&pdir.join("state.json"), &state).unwrap();
        }
        let output = list_projects_in(&base);
        assert!(output.contains("foo"), "should list foo: {output}");
        assert!(output.contains("bar"), "should list bar: {output}");
    }

    // --- Step 1 Loop 11: Command::Init parse tests ---

    // AC-Init1: Parse `loopy init` with no flags
    #[test]
    fn parse_init() {
        let cli = Cli::parse_from(["loopy", "init"]);
        assert!(matches!(cli.command, Command::Init));
    }

    // AC-Init3: Existing commands still parse after adding Init
    #[test]
    fn existing_commands_unchanged_after_init() {
        let status = Cli::parse_from(["loopy", "status"]);
        assert!(matches!(status.command, Command::Status { name: None }));
        let list = Cli::parse_from(["loopy", "list"]);
        assert_eq!(list.command, Command::List);
    }

    // Step 9: dead_stage_pids returns stages with dead/missing PIDs
    #[test]
    fn stage_has_completion_event_true_when_present() {
        let dir = tempfile::TempDir::new().unwrap();
        let scan_ralph = dir.path().join("scan/.ralph");
        std::fs::create_dir_all(&scan_ralph).unwrap();
        std::fs::write(
            scan_ralph.join("events-test.jsonl"),
            "{\"topic\":\"scan.done\",\"iteration\":1}\n",
        )
        .unwrap();
        assert!(stage_has_completion_event(
            dir.path(),
            crate::models::StageId::Scan
        ));
    }

    #[test]
    fn stage_has_completion_event_false_when_missing() {
        let dir = tempfile::TempDir::new().unwrap();
        let scan_ralph = dir.path().join("scan/.ralph");
        std::fs::create_dir_all(&scan_ralph).unwrap();
        std::fs::write(
            scan_ralph.join("events-test.jsonl"),
            "{\"topic\":\"task.start\",\"iteration\":0}\n",
        )
        .unwrap();
        assert!(!stage_has_completion_event(
            dir.path(),
            crate::models::StageId::Scan
        ));
    }

    #[test]
    fn dead_stage_pids_skips_completed_stages() {
        let dir = tempfile::TempDir::new().unwrap();
        let scan_ralph = dir.path().join("scan/.ralph");
        std::fs::create_dir_all(&scan_ralph).unwrap();
        // Stage has completion event
        std::fs::write(
            scan_ralph.join("events-test.jsonl"),
            "{\"topic\":\"scan.complete\",\"iteration\":1}\n",
        )
        .unwrap();
        let state = crate::models::PipelineState {
            version: 1,
            idea_text: String::new(),
            stages: vec![crate::models::StageState {
                id: crate::models::StageId::Scan,
                status: crate::models::StageStatus::Running,
                loop_id: None,
                loop_pid: None,
                started_at: None,
                completed_at: None,
                error: None,
            }],
            tracks: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            execution_mode: crate::models::ExecutionMode::Fast,
            artifact_registry: crate::models::ArtifactRegistry {
                artifacts: std::collections::HashMap::new(),
                consumptions: vec![],
            },
            active_conflicts: vec![],
            awaiting_approval: None,
        };
        // Should NOT return Scan since it already completed
        let dead = dead_stage_pids(&state, dir.path());
        assert!(dead.is_empty(), "completed stage should not be respawned");
    }

    #[test]
    fn dead_stage_pids_returns_dead_stages() {
        use crate::models::*;
        let dir = tempfile::TempDir::new().unwrap();
        let mut state = fresh_pipeline(ExecutionMode::Safe);
        // Scan is Running with a PID that doesn't exist (99999999)
        state.stages[1].status = StageStatus::Running;
        state.stages[1].loop_pid = Some(99999999);
        // Plan is Running with no PID at all
        state.stages[2].status = StageStatus::Running;
        state.stages[2].loop_pid = None;

        let dead = dead_stage_pids(&state, dir.path());
        assert!(
            dead.contains(&StageId::Scan),
            "Scan should be dead (bogus PID)"
        );
        assert!(
            dead.contains(&StageId::Plan),
            "Plan should be dead (no PID)"
        );
    }

    // Step 9: dead_stage_pids skips non-running stages
    #[test]
    fn dead_stage_pids_skips_non_running() {
        use crate::models::*;
        let dir = tempfile::TempDir::new().unwrap();
        let mut state = fresh_pipeline(ExecutionMode::Safe);
        // Idea is Running (default), but Scan is Pending
        state.stages[1].status = StageStatus::Pending;
        state.stages[1].loop_pid = Some(99999999);

        let dead = dead_stage_pids(&state, dir.path());
        // Scan is Pending, so it should NOT appear
        assert!(!dead.contains(&StageId::Scan));
    }

    // Step 9: dead_stage_pids reads PID from file when loop_pid is None
    #[test]
    fn dead_stage_pids_reads_pid_file() {
        use crate::models::*;
        let dir = tempfile::TempDir::new().unwrap();
        let mut state = fresh_pipeline(ExecutionMode::Safe);
        state.stages[1].status = StageStatus::Running;
        state.stages[1].loop_pid = None;
        // Write a bogus PID file
        crate::orchestrator::write_pid_file(dir.path(), StageId::Scan, 99999999).unwrap();

        let dead = dead_stage_pids(&state, dir.path());
        // Should find the PID file, check it's dead, and include Scan
        assert!(dead.contains(&StageId::Scan));
    }

    #[test]
    fn list_projects_legacy_ignored_when_projects_dir_exists() {
        let dir = tempfile::TempDir::new().unwrap();
        let base = dir.path().join(".loopy");
        std::fs::create_dir_all(&base).unwrap();
        let state = fresh_pipeline(crate::models::ExecutionMode::Safe);
        crate::checkpoint::save(&base.join("state.json"), &state).unwrap();
        // Also create projects dir with one project
        let pdir = base.join("projects").join("myproj");
        std::fs::create_dir_all(&pdir).unwrap();
        crate::checkpoint::save(&pdir.join("state.json"), &state).unwrap();
        let output = list_projects_in(&base);
        assert!(output.contains("myproj"), "should list myproj: {output}");
        assert!(
            !output.contains("default"),
            "should NOT show legacy when projects/ exists: {output}"
        );
    }

    // Bug 13: alive_stage_pids returns nothing for dead PIDs
    #[test]
    fn alive_stage_pids_excludes_dead() {
        use crate::models::*;
        let dir = tempfile::TempDir::new().unwrap();
        let mut state = fresh_pipeline(ExecutionMode::Safe);
        state.stages[1].status = StageStatus::Running;
        state.stages[1].loop_pid = Some(99999999); // bogus PID
        let alive = alive_stage_pids(&state, dir.path());
        assert!(alive.is_empty(), "dead PID should not appear in alive list");
    }

    // Bug 13: alive_stage_pids skips non-running stages
    #[test]
    fn alive_stage_pids_skips_non_running() {
        use crate::models::*;
        let dir = tempfile::TempDir::new().unwrap();
        let state = fresh_pipeline(ExecutionMode::Safe);
        // Idea is Running but has no PID → not alive
        let alive = alive_stage_pids(&state, dir.path());
        assert!(alive.is_empty());
    }

    #[test]
    fn completed_running_stages_detects_completion() {
        use crate::models::{
            ArtifactRegistry, ExecutionMode, PipelineState, StageId, StageState, StageStatus,
        };
        use chrono::Utc;
        let dir = tempfile::TempDir::new().unwrap();
        // Scan is Running and has completion event
        let scan_ralph = dir.path().join("scan/.ralph");
        std::fs::create_dir_all(&scan_ralph).unwrap();
        std::fs::write(
            scan_ralph.join("events.jsonl"),
            "{\"topic\":\"scan.complete\",\"iteration\":1}\n",
        )
        .unwrap();
        // Plan is Running but has no completion event
        let plan_ralph = dir.path().join("plan/.ralph");
        std::fs::create_dir_all(&plan_ralph).unwrap();

        let now = Utc::now();
        let make_stage = |id, status| StageState {
            id,
            status,
            loop_id: None,
            loop_pid: None,
            started_at: None,
            completed_at: None,
            error: None,
        };
        let state = PipelineState {
            version: 1,
            idea_text: String::new(),
            stages: vec![
                make_stage(StageId::Scan, StageStatus::Running),
                make_stage(StageId::Plan, StageStatus::Running),
                make_stage(StageId::RequirementsAnalysis, StageStatus::Pending),
                make_stage(StageId::OrbitalLanes, StageStatus::Pending),
                make_stage(StageId::Land, StageStatus::Pending),
                make_stage(StageId::Idea, StageStatus::Complete),
            ],
            tracks: None,
            created_at: now,
            updated_at: now,
            execution_mode: ExecutionMode::Safe,
            artifact_registry: ArtifactRegistry::default(),
            active_conflicts: vec![],
            awaiting_approval: None,
        };
        let result = completed_running_stages(&state, dir.path());
        assert_eq!(result, vec![StageId::Scan]);
    }

    #[test]
    fn logs_prints_stage_and_track_headers() {
        use std::fs;
        let tmp = tempfile::tempdir().unwrap();
        let stages = tmp.path().join("stages");

        // Create scan stage log
        let scan_ralph = stages.join("scan/.ralph");
        fs::create_dir_all(&scan_ralph).unwrap();
        fs::write(scan_ralph.join("ralph-output.log"), "scan output line 1\n").unwrap();

        // Create plan stage log (missing — should be skipped)

        // Create track log
        let track_ralph = stages.join("orbital-lanes/track-abc/.ralph");
        fs::create_dir_all(&track_ralph).unwrap();
        fs::write(track_ralph.join("ralph-output.log"), "track abc output\n").unwrap();

        let tracks = vec![crate::models::TrackState {
            id: "track-abc".into(),
            name: "My Track".into(),
            status: crate::models::TrackStatus::Complete,
            loop_id: None,
            loop_pid: None,
            current_sub_stage: None,
            depends_on: vec![],
            blocking_artifact: None,
            consumed_versions: vec![],
            run_count: 0,
            medium: Default::default(),
            review_status: Default::default(),
            cr_url: None,
        }];

        let output = collect_logs(&stages, &Some(tracks));
        assert!(
            output.contains("=== Stage: scan ==="),
            "missing scan header: {output}"
        );
        assert!(
            output.contains("scan output line 1"),
            "missing scan content: {output}"
        );
        assert!(
            !output.contains("=== Stage: plan ==="),
            "plan should be skipped: {output}"
        );
        assert!(
            output.contains("=== Track: My Track (track-abc) ==="),
            "missing track header: {output}"
        );
        assert!(
            output.contains("track abc output"),
            "missing track content: {output}"
        );
    }

    #[test]
    fn tracks_add_writes_yaml() {
        let dir = tempfile::TempDir::new().unwrap();
        let tracks_dir = dir.path().join(".loopy").join("tracks");
        let result = handle_tracks_add(
            dir.path(),
            "load-testing",
            Some("Performance testing"),
            Some("builtin:code-assist"),
            &["backend".to_string(), "security".to_string()],
        );
        assert!(result.is_ok());
        let yml_path = tracks_dir.join("load-testing.yml");
        assert!(yml_path.exists());
        let content = std::fs::read_to_string(&yml_path).unwrap();
        let def: crate::models::TrackDefinition = serde_yaml::from_str(&content).unwrap();
        assert_eq!(def.name, "load-testing");
        assert_eq!(def.description, "Performance testing");
        assert_eq!(def.hat, "builtin:code-assist");
        assert_eq!(def.dependencies, vec!["backend", "security"]);
    }

    // --- Output file fallback for completion detection ---

    #[test]
    fn stage_has_completion_event_scan_output_fallback() {
        let dir = tempfile::TempDir::new().unwrap();
        let scan_dir = dir.path().join("scan");
        std::fs::create_dir_all(scan_dir.join(".ralph")).unwrap();
        // No JSONL events, but output files exist
        std::fs::write(scan_dir.join("scan-report.md"), "# Report").unwrap();
        std::fs::write(scan_dir.join("environment.json"), "{}").unwrap();
        assert!(stage_has_completion_event(
            dir.path(),
            crate::models::StageId::Scan
        ));
    }

    #[test]
    fn stage_has_completion_event_plan_needs_both_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let plan_dir = dir.path().join("plan");
        std::fs::create_dir_all(plan_dir.join(".ralph")).unwrap();
        // Only PLAN.md, no tracks.json — should NOT be complete
        std::fs::write(plan_dir.join("PLAN.md"), "# Plan").unwrap();
        assert!(!stage_has_completion_event(
            dir.path(),
            crate::models::StageId::Plan
        ));
        // Add tracks.json — now complete
        std::fs::write(plan_dir.join("tracks.json"), "{}").unwrap();
        assert!(stage_has_completion_event(
            dir.path(),
            crate::models::StageId::Plan
        ));
    }

    // --- resolve_project_name priority: yml > slug ---

    #[test]
    fn resolve_project_name_yml_wins_over_slug() {
        let got = resolve_project_name(None, "My Idea Text", Some("from-yml".into()));
        assert_eq!(got.as_deref(), Some("from-yml"));
    }

    #[test]
    fn resolve_project_name_slug_when_no_yml() {
        let got = resolve_project_name(None, "My Idea", None);
        assert_eq!(got.as_deref(), Some("my-idea"));
    }

    #[test]
    fn resolve_project_name_explicit_wins_all() {
        let got = resolve_project_name(Some("explicit".into()), "idea", Some("yml".into()));
        assert_eq!(got.as_deref(), Some("explicit"));
    }

    #[test]
    fn status_shows_next_command_after_plan() {
        use crate::models::*;
        let mut state = fresh_pipeline(ExecutionMode::Fast);
        state.stages[0].status = StageStatus::Complete; // Idea
        state.stages[1].status = StageStatus::Complete; // Scan
        state.stages[2].status = StageStatus::Complete; // Plan
        let output = format_status(&state, false);
        assert!(
            output.contains("loopy review"),
            "should suggest loopy review after plan: {output}"
        );
    }

    #[test]
    fn status_shows_next_command_after_requirements() {
        use crate::models::*;
        let mut state = fresh_pipeline(ExecutionMode::Fast);
        state.stages[0].status = StageStatus::Complete;
        state.stages[1].status = StageStatus::Complete;
        state.stages[2].status = StageStatus::Complete;
        state.stages[3].status = StageStatus::Complete; // RequirementsAnalysis
        let output = format_status(&state, false);
        assert!(
            output.contains("loopy review --approve"),
            "should suggest approve after req: {output}"
        );
    }

    #[test]
    fn status_shows_tracks_setup_when_pending() {
        use crate::models::*;
        let mut state = fresh_pipeline(ExecutionMode::Fast);
        for s in &mut state.stages {
            s.status = StageStatus::Complete;
        }
        state.stages[4].status = StageStatus::Running; // OrbitalLanes
        state.tracks = Some(vec![TrackState {
            id: "backend".into(),
            name: "Backend".into(),
            status: TrackStatus::PendingSetup,
            loop_id: None,
            loop_pid: None,
            review_status: ReviewStatus::PendingReview,
            cr_url: None,
            current_sub_stage: None,
            depends_on: vec![],
            blocking_artifact: None,
            consumed_versions: vec![],
            run_count: 0,
            medium: TrackMedium::default(),
        }]);
        let output = format_status(&state, false);
        assert!(
            output.contains("loopy tracks setup"),
            "should suggest tracks setup: {output}"
        );
    }

    #[test]
    fn status_shows_logs_hint_when_tracks_running() {
        use crate::models::*;
        let mut state = fresh_pipeline(ExecutionMode::Fast);
        for s in &mut state.stages {
            s.status = StageStatus::Complete;
        }
        state.stages[4].status = StageStatus::Running;
        state.tracks = Some(vec![TrackState {
            id: "backend".into(),
            name: "Backend".into(),
            status: TrackStatus::Running,
            loop_id: None,
            loop_pid: None,
            review_status: ReviewStatus::PendingReview,
            cr_url: None,
            current_sub_stage: None,
            depends_on: vec![],
            blocking_artifact: None,
            consumed_versions: vec![],
            run_count: 0,
            medium: TrackMedium::default(),
        }]);
        let output = format_status(&state, false);
        assert!(
            output.contains("loopy logs"),
            "should suggest loopy logs when tracks running: {output}"
        );
    }

    // ── Loop 19 Step 1: CLI Ergonomics ──

    #[test]
    fn doctor_command_parses() {
        let cli = Cli::parse_from(["loopy", "doctor"]);
        assert!(matches!(cli.command, Command::Doctor));
    }

    #[test]
    fn parse_blocks() {
        let cli = Cli::parse_from(["loopy", "blocks"]);
        assert_eq!(cli.command, Command::Blocks { check: false });
    }

    #[test]
    fn parse_blocks_check() {
        let cli = Cli::parse_from(["loopy", "blocks", "--check"]);
        assert_eq!(cli.command, Command::Blocks { check: true });
    }

    #[test]
    fn parse_start_defaults() {
        let cli = Cli::parse_from(["loopy", "start"]);
        assert!(matches!(
            cli.command,
            Command::Start {
                idea: None,
                name: None,
                port: 3000,
                no_open: false,
            }
        ));
    }
}
