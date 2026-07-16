use crate::engine::{Engine, EngineEvent, EngineState, Phase};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, RwLock};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsServerMsg {
    State { data: EngineState },
    PhaseChange { phase: Phase },
    Log { line: String, level: String },
    TrackProgress { track: String, tasks_done: u32, tasks_total: u32, current: String },
    /// Progress of a generic (non-POC) linear pipeline run, for the UI to render
    /// its blocks. `current`/`paused` drive the active + review states.
    LinearProgress {
        template_id: String,
        stages: Vec<LinearStageView>,
        current: Option<String>,
        completed: Vec<String>,
        paused: Option<String>,
        done: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinearStageView {
    pub id: String,
    pub label: String,
    pub description: String,
    pub optional: bool,
    pub checkpoint: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsClientMsg {
    Approve,
    Reject { feedback: String },
    Abort,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateProjectRequest {
    pub idea: String,
    pub name: Option<String>,
    /// Loopy pipeline template to run. Omitted → POC pipeline (today's default).
    #[serde(default)]
    pub template_id: Option<String>,
    /// A user-edited custom block list (overrides template_id). Each entry is a
    /// block the user confirmed after editing the planner's proposal. Built into
    /// a custom pipeline executed via the linear driver.
    #[serde(default)]
    pub blocks: Option<Vec<BlockSpec>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockSpec {
    pub id: String,
    /// Block kind, e.g. "scan", "plan", "design", "team_review", "implement",
    /// "review", "beta_test", "verify", "submit".
    pub kind: String,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RejectRequest {
    pub feedback: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ApproveRequest {
    /// At Flight Check, opt into the Test Flight (beta test) stage instead of
    /// going straight to Land. Ignored at other checkpoints.
    #[serde(default)]
    pub test_flight: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSummary {
    pub name: String,
    pub phase: Phase,
    pub idea: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

pub struct AppState {
    pub engines: RwLock<HashMap<String, Engine>>,
    pub broadcast: broadcast::Sender<(String, WsServerMsg)>,
    pub action_channels: RwLock<HashMap<String, mpsc::Sender<EngineEvent>>>,
    pub project_dir: PathBuf,
}

impl AppState {
    pub fn new(project_dir: PathBuf) -> Self {
        let (tx, _) = broadcast::channel(256);
        Self {
            engines: RwLock::new(HashMap::new()),
            broadcast: tx,
            action_channels: RwLock::new(HashMap::new()),
            project_dir,
        }
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/projects", get(list_projects).post(create_project))
        .route("/api/projects/{name}", get(get_project).delete(delete_project))
        .route("/api/projects/{name}/approve", post(approve))
        .route("/api/projects/{name}/reject", post(reject))
        .route("/api/projects/{name}/abort", post(abort))
        .route("/api/projects/{name}/retry", post(retry))
        .route("/api/projects/{name}/diff", get(get_diff))
        .route("/api/projects/{name}/review-diff", get(get_review_diff))
        .route("/api/projects/{name}/plan", get(get_plan))
        .route("/api/projects/{name}/test-plan", get(get_test_plan).post(save_test_plan))
        .route("/api/projects/{name}/tracks/{track}/log", get(get_track_log))
        .route("/api/projects/{name}/blocks/{block}/output", get(get_block_output))
        .route("/api/config", get(get_config))
        .route("/api/templates", get(get_templates))
        .route("/api/plan-template", post(plan_template))
        .route("/api/plan-blocks", post(plan_blocks))
        .route("/api/enrich-prompt", post(enrich_prompt))
        .route("/api/block-kinds", get(get_block_kinds))
        .route("/ws/{name}", get(ws_handler))
        .fallback(get(serve_spa))
        .with_state(state)
}

mod assets {
    include!(concat!(env!("OUT_DIR"), "/web_assets.rs"));
}

async fn serve_spa(req: axum::extract::Request) -> impl IntoResponse {
    use axum::http::header;
    let path = req.uri().path();

    // Serve JS bundle
    if path.ends_with(".js") {
        return (
            [(header::CONTENT_TYPE, "application/javascript")],
            assets::JS_CONTENT,
        ).into_response();
    }
    // Serve CSS bundle
    if path.ends_with(".css") {
        return (
            [(header::CONTENT_TYPE, "text/css")],
            assets::CSS_CONTENT,
        ).into_response();
    }

    // SPA fallback — rewrite asset paths to match our embedded filenames
    let html = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/web/dist/index.html"));
    ([(header::CONTENT_TYPE, "text/html")], html).into_response()
}

async fn list_projects(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let engines = state.engines.read().await;
    let projects: Vec<ProjectSummary> = engines
        .iter()
        .map(|(name, engine)| ProjectSummary {
            name: name.clone(),
            phase: engine.state.phase,
            idea: engine.state.idea.clone(),
            created_at: engine.state.created_at,
            updated_at: engine.state.updated_at,
        })
        .collect();
    axum::Json(projects)
}

async fn get_project(
    State(state): State<Arc<AppState>>,
    AxumPath(name): AxumPath<String>,
) -> impl IntoResponse {
    let engines = state.engines.read().await;
    match engines.get(&name) {
        Some(engine) => axum::Json(serde_json::to_value(&engine.state).unwrap()).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn create_project(
    State(state): State<Arc<AppState>>,
    axum::Json(req): axum::Json<CreateProjectRequest>,
) -> impl IntoResponse {
    let name = req.name.unwrap_or_else(|| slug_from_idea(&req.idea));

    // Create engine already in Scanning phase for immediate API consistency
    let mut engine = Engine::new(&req.idea, &name);
    // Apply the requested pipeline template. Unknown → POC default. A KNOWN but
    // not-yet-executable template is rejected, so we never silently run the POC
    // sequence when a different pipeline was asked for.
    if let Some(tid) = req.template_id.as_ref() {
        match crate::pipeline::template_by_id(tid) {
            Some(t) if t.executable => engine.state.template_id = tid.clone(),
            Some(t) => {
                return (
                    StatusCode::BAD_REQUEST,
                    axum::Json(serde_json::json!({
                        "error": format!("Template '{}' is defined but not yet executable. Only the POC pipeline runs today.", t.id),
                    })),
                ).into_response();
            }
            None => {} // unknown id → fall through to POC default
        }
    }
    // A user-edited custom block list overrides the template (corrected Loopy flow).
    if let Some(blocks) = req.blocks.as_ref() {
        let tuples: Vec<(String, String, Option<String>)> = blocks
            .iter()
            .map(|b| (b.id.clone(), b.kind.clone(), b.label.clone()))
            .collect();
        let custom = crate::pipeline::PipelineTemplate::from_blocks(&tuples);
        if custom.stages.is_empty() {
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(serde_json::json!({ "error": "Custom pipeline has no valid blocks." })),
            ).into_response();
        }
        engine.state.custom_pipeline = Some(custom);
    }
    // Only kick the POC state machine for POC runs. For a custom (linear) pipeline,
    // the runner drives it via start_linear() — running the POC `Started` here would
    // spawn the POC scan stage and flash the default config.
    if !engine.is_linear() {
        engine.transition(EngineEvent::Started { idea: req.idea.clone() });
    }
    state.engines.write().await.insert(name.clone(), engine);

    // Spawn engine runner in background
    spawn_engine_runner(state.clone(), name.clone(), req.idea.clone());

    (StatusCode::CREATED, axum::Json(serde_json::json!({ "name": name, "phase": "scanning" }))).into_response()
}

pub fn spawn_engine_runner(state: Arc<AppState>, project_name: String, idea: String) {
    spawn_engine_runner_inner(state, project_name, idea, false);
}

pub fn spawn_engine_runner_resume(state: Arc<AppState>, project_name: String, idea: String) {
    spawn_engine_runner_inner(state, project_name, idea, true);
}

fn spawn_engine_runner_inner(state: Arc<AppState>, project_name: String, idea: String, resume: bool) {
    use crate::engine_runner::{EngineRunner, LogLine};

    let (log_tx, mut log_rx) = tokio::sync::mpsc::channel::<LogLine>(256);
    let (action_tx, mut action_rx) = tokio::sync::mpsc::channel::<EngineEvent>(16);

    // Register action channel inside the runner task BEFORE entering the loop
    // The tx is moved into the runner task which registers it first thing

    let runner_state = state.clone();
    let runner_project = project_name.clone();
    let runner_idea = idea.clone();
    let project_root = state.project_dir.clone();

    // Engine runner task
    tokio::spawn(async move {
        // Register action channel FIRST — must be visible before entering the loop
        runner_state.action_channels.write().await.insert(runner_project.clone(), action_tx);

        let engine_state = {
            let engines = runner_state.engines.read().await;
            engines.get(&runner_project).map(|e| e.state.clone())
        };

        let engine = if resume {
            if let Some(s) = engine_state {
                Engine::from_state(s)
            } else {
                Engine::new(&runner_idea, &runner_project)
            }
        } else {
            // Fresh run: carry over the template the create-project call selected
            // (the in-memory engine state was inserted before spawning us). Defaults
            // to the POC pipeline when absent.
            let mut e = Engine::new(&runner_idea, &runner_project);
            if let Some(s) = &engine_state {
                e.state.template_id = s.template_id.clone();
                e.state.custom_pipeline = s.custom_pipeline.clone();
            }
            e
        };

        let mut runner = match EngineRunner::new(engine, project_root.clone(), log_tx).await {
            Ok(r) => r,
            Err(e) => {
                log::error!("Failed to create engine runner for {}: {e}", runner_project);
                return;
            }
        };

        if !resume {
            runner.start(runner_idea).await;
        } else {
            // On resume, load tracks if missing
            if runner.engine.state.tracks.is_none() {
                let plan_dir = project_root.join(".loopy/stages/plan");
                if plan_dir.exists() {
                    runner.load_tracks_from_plan(&plan_dir);
                }
            }
            // If a stage was running when interrupted, re-spawn its Ralph process.
            // (Checkpoint phases like AwaitingPlanReview don't need re-spawning.)
            runner.respawn_active_stage().await;
        }

        // Sync state to web
        let sync_state = |runner: &EngineRunner, state: &Arc<AppState>, name: &str| {
            let engine_state = runner.state().clone();
            let state = state.clone();
            let name = name.to_string();
            tokio::spawn(async move {
                state.engines.write().await.insert(name.clone(), Engine::from_state(engine_state.clone()));
                let _ = state.broadcast.send((name, WsServerMsg::State { data: engine_state }));
            });
        };

        sync_state(&runner, &runner_state, &runner_project);
        let mut poll_count: u32 = 0;

        loop {
            let phase = runner.phase();

            // Generic (non-POC) pipeline loop: driven by LinearRun via poll(),
            // with approve/reject routed to the linear checkpoint handlers. This
            // is a SEPARATE path — the POC phase logic below is never entered.
            if runner.is_linear() {
                // Handle any pending user actions against a linear checkpoint.
                while let Ok(event) = action_rx.try_recv() {
                    match event {
                        EngineEvent::UserApprovedCode { .. } | EngineEvent::UserApprovedTestPlan | EngineEvent::UserAcceptedTest | EngineEvent::UserApprovedPlan => {
                            runner.approve_linear().await;
                            sync_state(&runner, &runner_state, &runner_project);
                        }
                        EngineEvent::UserRejectedCode { feedback } | EngineEvent::UserRejectedTest { feedback } | EngineEvent::UserRejectedPlan { feedback } => {
                            runner.reject_linear(feedback).await;
                            sync_state(&runner, &runner_state, &runner_project);
                        }
                        EngineEvent::Abort => {
                            // KillAll reaps the process groups; abort_linear clears the
                            // sidecar so a restart won't resume the aborted pipeline.
                            let effects = runner.engine.transition(EngineEvent::Abort);
                            runner.process_effects(effects).await;
                            runner.abort_linear();
                            sync_state(&runner, &runner_state, &runner_project);
                            break;
                        }
                        _ => {}
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                if runner.poll().await {
                    sync_state(&runner, &runner_state, &runner_project);
                }
                // Broadcast linear pipeline progress for the UI (~1s cadence), or
                // immediately once the run is done.
                let is_done = runner.linear_snapshot().map(|s| s.5).unwrap_or(false);
                if poll_count % 5 == 0 || is_done {
                    if let Some((template_id, stages, current, completed, paused, done)) = runner.linear_snapshot() {
                        let stage_views: Vec<LinearStageView> = stages.iter().map(|s| LinearStageView {
                            id: s.id.clone(),
                            label: s.kind.display_name().to_string(),
                            description: s.kind.description().to_string(),
                            optional: s.optional,
                            checkpoint: s.checkpoint.is_some(),
                        }).collect();
                        let _ = runner_state.broadcast.send((runner_project.clone(), WsServerMsg::LinearProgress {
                            template_id, stages: stage_views, current, completed, paused, done,
                        }));
                    }
                }
                // Exit the loop when the pipeline finishes — otherwise this busy-
                // waits forever (mirrors the POC Complete/Failed break below).
                if is_done {
                    break;
                }
                poll_count += 1;
                continue;
            }

            if phase == Phase::Complete || phase == Phase::Failed {
                sync_state(&runner, &runner_state, &runner_project);
                break;
            }

            // At a checkpoint — wait for user action
            if phase == Phase::AwaitingPlanReview || phase == Phase::AwaitingCodeReview {
                sync_state(&runner, &runner_state, &runner_project);

                // Block until we receive an approve/reject event
                match action_rx.recv().await {
                    Some(event) => {
                        let effects = runner.engine.transition(event);
                        runner.process_effects(effects).await;
                        sync_state(&runner, &runner_state, &runner_project);
                    }
                    None => break,
                }
                continue;
            }

            // During a running stage, also drain pending actions (e.g. Abort) so
            // the user can abort mid-stage instead of only at checkpoints.
            while let Ok(event) = action_rx.try_recv() {
                let is_abort = matches!(event, EngineEvent::Abort);
                let effects = runner.engine.transition(event);
                runner.process_effects(effects).await;
                sync_state(&runner, &runner_state, &runner_project);
                if is_abort {
                    break;
                }
            }

            // Normal polling (200ms)
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            poll_count += 1;
            let phase_changed = runner.poll().await;
            if phase_changed {
                sync_state(&runner, &runner_state, &runner_project);
            }
            // While tracks are running, broadcast per-track progress (~1s cadence)
            // so the UI can show real activity instead of a generic "working...".
            if phase == Phase::RunningTracks && poll_count % 5 == 0 {
                for (track, done, total, current) in runner.track_progress() {
                    let _ = runner_state.broadcast.send((
                        runner_project.clone(),
                        WsServerMsg::TrackProgress { track, tasks_done: done, tasks_total: total, current },
                    ));
                }
                // Reconcile terminal tracks from disk. Ralph signals completion via
                // a log line, not a file-watcher topic, so this is the only path
                // that advances the engine past running_tracks → Land.
                if runner.reconcile_tracks().await {
                    sync_state(&runner, &runner_state, &runner_project);
                }
            }
            // Save checkpoint to disk every 5s (no broadcast — avoids UI flicker)
            if poll_count % 25 == 0 {
                runner.save_checkpoint();
            }
        }

        // Cleanup action channel
        runner_state.action_channels.write().await.remove(&runner_project);
    });

    // Log forwarder task
    let log_state = state;
    let log_project = project_name;
    tokio::spawn(async move {
        while let Some(log_line) = log_rx.recv().await {
            let _ = log_state.broadcast.send((
                log_project.clone(),
                WsServerMsg::Log { line: log_line.message, level: log_line.level },
            ));
        }
    });
}

async fn approve(
    State(state): State<Arc<AppState>>,
    AxumPath(name): AxumPath<String>,
    body: Option<axum::Json<ApproveRequest>>,
) -> impl IntoResponse {
    let req = body.map(|axum::Json(r)| r).unwrap_or_default();
    let engines = state.engines.read().await;
    let Some(engine) = engines.get(&name) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    // For a linear (composed) run, the POC phase isn't a review phase — the
    // checkpoint lives in the runner's LinearRun. Forward a generic approve event;
    // the runner's linear branch routes it to approve_linear().
    let event = if engine.is_linear() {
        EngineEvent::UserApprovedCode { run_test_flight: req.test_flight }
    } else {
        match engine.state.phase {
            Phase::AwaitingPlanReview => EngineEvent::UserApprovedPlan,
            Phase::AwaitingCodeReview => EngineEvent::UserApprovedCode { run_test_flight: req.test_flight },
            Phase::AwaitingTestPlan => EngineEvent::UserApprovedTestPlan,
            Phase::AwaitingTestReview => EngineEvent::UserAcceptedTest,
            _ => return (StatusCode::CONFLICT, "not at a review checkpoint").into_response(),
        }
    };
    drop(engines);

    // Notify the running runner WITHOUT blocking the HTTP request. try_send never
    // waits — if the runner is busy/wedged or its buffer is full, we fall through
    // and transition the engine directly so the UI always advances.
    let channels = state.action_channels.read().await;
    let sent = if let Some(tx) = channels.get(&name) {
        tx.try_send(event.clone()).is_ok()
    } else {
        false
    };
    drop(channels);

    if !sent {
        let mut engines = state.engines.write().await;
        if let Some(engine) = engines.get_mut(&name) {
            engine.transition(event);
            let new_state = engine.state.clone();
            let _ = state.broadcast.send((name.clone(), WsServerMsg::State { data: new_state }));
        }
    }

    axum::Json(serde_json::json!({ "status": "ok" })).into_response()
}

async fn reject(
    State(state): State<Arc<AppState>>,
    AxumPath(name): AxumPath<String>,
    axum::Json(req): axum::Json<RejectRequest>,
) -> impl IntoResponse {
    let engines = state.engines.read().await;
    let Some(engine) = engines.get(&name) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    // Linear runs: forward a generic reject; the runner routes it to reject_linear().
    let event = if engine.is_linear() {
        EngineEvent::UserRejectedCode { feedback: req.feedback }
    } else {
        match engine.state.phase {
            Phase::AwaitingPlanReview => EngineEvent::UserRejectedPlan { feedback: req.feedback },
            Phase::AwaitingCodeReview => EngineEvent::UserRejectedCode { feedback: req.feedback },
            Phase::AwaitingTestReview => EngineEvent::UserRejectedTest { feedback: req.feedback },
            _ => return (StatusCode::CONFLICT, "not at a review checkpoint").into_response(),
        }
    };
    drop(engines);

    // Non-blocking notify; fall through to direct transition if runner is busy.
    let channels = state.action_channels.read().await;
    let sent = if let Some(tx) = channels.get(&name) {
        tx.try_send(event.clone()).is_ok()
    } else {
        false
    };
    drop(channels);

    if !sent {
        let mut engines = state.engines.write().await;
        if let Some(engine) = engines.get_mut(&name) {
            engine.transition(event);
            let new_state = engine.state.clone();
            let _ = state.broadcast.send((name.clone(), WsServerMsg::State { data: new_state }));
        }
    }

    axum::Json(serde_json::json!({ "status": "ok" })).into_response()
}

async fn delete_project(
    State(state): State<Arc<AppState>>,
    AxumPath(name): AxumPath<String>,
) -> axum::response::Response {
    // Send abort to running engine
    let channels = state.action_channels.read().await;
    if let Some(tx) = channels.get(&name) {
        let _ = tx.send(EngineEvent::Abort).await;
    }
    drop(channels);

    state.engines.write().await.remove(&name);
    state.action_channels.write().await.remove(&name);
    StatusCode::OK.into_response()
}

async fn retry(
    State(state): State<Arc<AppState>>,
    AxumPath(name): AxumPath<String>,
) -> axum::response::Response {
    let engines = state.engines.read().await;
    let Some(engine) = engines.get(&name) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if engine.state.phase != Phase::Failed {
        return (StatusCode::CONFLICT, "project is not in failed state").into_response();
    }
    let idea = engine.state.idea.clone();
    drop(engines);

    // Remove old engine, create fresh one, restart
    state.engines.write().await.remove(&name);
    let mut engine = Engine::new(&idea, &name);
    engine.transition(EngineEvent::Started { idea: idea.clone() });
    state.engines.write().await.insert(name.clone(), engine);
    spawn_engine_runner(state.clone(), name.clone(), idea);

    let _ = state.broadcast.send((name.clone(), WsServerMsg::PhaseChange { phase: Phase::Scanning }));
    StatusCode::OK.into_response()
}

async fn abort(
    State(state): State<Arc<AppState>>,
    AxumPath(name): AxumPath<String>,
) -> axum::response::Response {
    // Non-blocking: signal the runner to abort (it will kill all Ralph loops).
    let channels = state.action_channels.read().await;
    if let Some(tx) = channels.get(&name) {
        let _ = tx.try_send(EngineEvent::Abort);
    }
    drop(channels);

    // Update state directly for immediate UI feedback
    let mut engines = state.engines.write().await;
    if let Some(engine) = engines.get_mut(&name) {
        engine.transition(EngineEvent::Abort);
    }
    drop(engines);

    let _ = state.broadcast.send((name.clone(), WsServerMsg::PhaseChange { phase: Phase::Failed }));
    StatusCode::OK.into_response()
}

/// Map of track id → declared package names, read from the Plan stage's
/// tracks.json. This is the authoritative track→package ownership map (each
/// track committed to an loopy/* branch in these sibling Brazil packages).
fn track_package_map(project_dir: &std::path::Path) -> Vec<(String, Vec<String>)> {
    let path = project_dir.join(".loopy/stages/plan/tracks.json");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return vec![];
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) else {
        return vec![];
    };
    let Some(tracks) = json.get("tracks").and_then(|t| t.as_array()) else {
        return vec![];
    };
    tracks
        .iter()
        .filter_map(|t| {
            let id = t.get("id")?.as_str()?.to_string();
            let pkgs = t
                .get("packages")?
                .as_array()?
                .iter()
                .filter_map(|p| p.get("name").and_then(|n| n.as_str()).map(String::from))
                .collect::<Vec<_>>();
            Some((id, pkgs))
        })
        .collect()
}

/// Resolve a package's repo dir. Tracks operate on the SHARED Brazil workspace
/// packages, which are siblings of the Loopy package dir (project_dir/..),
/// not copies inside the track workspace.
fn package_repo_dir(project_dir: &std::path::Path, package: &str) -> std::path::PathBuf {
    project_dir
        .parent()
        .map(|src| src.join(package))
        .unwrap_or_else(|| project_dir.join(package))
}

async fn get_diff(
    State(state): State<Arc<AppState>>,
    AxumPath(name): AxumPath<String>,
) -> axum::response::Response {
    let engines = state.engines.read().await;
    if engines.get(&name).is_none() {
        return StatusCode::NOT_FOUND.into_response();
    };
    drop(engines);

    // Flat list (back-compat with the simple panel): committed diff per package,
    // file paths namespaced by track/package so nothing collides.
    let mut diff_files = Vec::new();
    for (track, packages) in track_package_map(&state.project_dir) {
        for pkg in packages {
            let repo = package_repo_dir(&state.project_dir, &pkg);
            for mut f in crate::diff::get_package_committed_diff(&repo) {
                f.path = format!("{track}/{pkg}/{}", f.path);
                diff_files.push(f);
            }
        }
    }
    axum::Json(diff_files).into_response()
}

/// Structured review diff: grouped track → package → files. This is what the
/// Flight Check UI consumes to render a navigable, by-package review surface.
async fn get_review_diff(
    State(state): State<Arc<AppState>>,
    AxumPath(name): AxumPath<String>,
) -> axum::response::Response {
    let engines = state.engines.read().await;
    if engines.get(&name).is_none() {
        return StatusCode::NOT_FOUND.into_response();
    };
    drop(engines);

    let groups: Vec<serde_json::Value> = track_package_map(&state.project_dir)
        .into_iter()
        .map(|(track, packages)| {
            let pkgs: Vec<serde_json::Value> = packages
                .into_iter()
                .map(|pkg| {
                    let repo = package_repo_dir(&state.project_dir, &pkg);
                    let files = crate::diff::get_package_committed_diff(&repo);
                    serde_json::json!({
                        "package": pkg,
                        "files": files,
                    })
                })
                .collect();
            let total: usize = pkgs
                .iter()
                .map(|p| p.get("files").and_then(|f| f.as_array()).map_or(0, |a| a.len()))
                .sum();
            serde_json::json!({
                "track": track,
                "packages": pkgs,
                "file_count": total,
            })
        })
        .collect();

    axum::Json(serde_json::json!({ "groups": groups })).into_response()
}

fn test_plan_path(project_dir: &std::path::Path) -> std::path::PathBuf {
    project_dir.join(".loopy/stages/test-flight/TESTING-PLAN.md")
}

async fn get_test_plan(
    State(state): State<Arc<AppState>>,
    AxumPath(name): AxumPath<String>,
) -> impl IntoResponse {
    let engines = state.engines.read().await;
    if engines.get(&name).is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    drop(engines);
    let content = std::fs::read_to_string(test_plan_path(&state.project_dir)).ok();
    axum::Json(serde_json::json!({ "content": content })).into_response()
}

#[derive(serde::Deserialize)]
struct SaveTestPlanRequest {
    content: String,
}

async fn save_test_plan(
    State(state): State<Arc<AppState>>,
    AxumPath(name): AxumPath<String>,
    axum::Json(req): axum::Json<SaveTestPlanRequest>,
) -> impl IntoResponse {
    let engines = state.engines.read().await;
    if engines.get(&name).is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    drop(engines);
    let path = test_plan_path(&state.project_dir);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::write(&path, req.content) {
        Ok(_) => axum::Json(serde_json::json!({ "status": "ok" })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    }
}

async fn get_plan(
    State(state): State<Arc<AppState>>,
    AxumPath(name): AxumPath<String>,
) -> impl IntoResponse {
    let engines = state.engines.read().await;
    let Some(_engine) = engines.get(&name) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    // Try multiple possible plan locations
    let candidates = [
        state.project_dir.join(".loopy/stages/plan/PLAN.md"),
        state.project_dir.join(".loopy/stages/plan/output/PLAN.md"),
        state.project_dir.join(".loopy/stages/plan/output/scratchpad.md"),
        state.project_dir.join(".loopy/stages/plan/.ralph/agent/scratchpad.md"),
        state.project_dir.join(".loopy/stages/plan/tracks.json"),
    ];

    for path in &candidates {
        if let Ok(content) = std::fs::read_to_string(path) {
            return axum::Json(serde_json::json!({ "content": content, "path": path.display().to_string() })).into_response();
        }
    }
    axum::Json(serde_json::json!({ "content": null })).into_response()
}

/// Return the main markdown artifact a block produced (design-spec.md, PLAN.md,
/// scratchpad, or any *.md in the block's stage dir) so the UI can show it for
/// review at a checkpoint. `block` is the block id (e.g. "design", "plan").
async fn get_block_output(
    State(state): State<Arc<AppState>>,
    AxumPath((name, block)): AxumPath<(String, String)>,
) -> impl IntoResponse {
    let engines = state.engines.read().await;
    if engines.get(&name).is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    drop(engines);
    // Reject path-traversal in the block id.
    if block.contains('/') || block.contains("..") {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let dir = state.project_dir.join(".loopy/stages").join(&block);
    // Preferred well-known artifact names, then fall back to any *.md (not PROMPT).
    let candidates = [
        "design-spec.md", "PLAN.md", "output/PLAN.md", "scratchpad.md",
        ".ralph/agent/scratchpad.md", "output/scratchpad.md", "report.md", "scan-report.md",
    ];
    for c in candidates {
        if let Ok(content) = std::fs::read_to_string(dir.join(c)) {
            if !content.trim().is_empty() {
                return axum::Json(serde_json::json!({ "content": content, "file": c })).into_response();
            }
        }
    }
    // Fallback: first non-PROMPT .md directly in the block dir.
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let p = e.path();
            let is_md = p.extension().and_then(|x| x.to_str()) == Some("md");
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if is_md && name != "PROMPT.md" {
                if let Ok(content) = std::fs::read_to_string(&p) {
                    if !content.trim().is_empty() {
                        return axum::Json(serde_json::json!({ "content": content, "file": name })).into_response();
                    }
                }
            }
        }
    }
    axum::Json(serde_json::json!({ "content": null })).into_response()
}

async fn get_track_log(
    State(state): State<Arc<AppState>>,
    AxumPath((name, track)): AxumPath<(String, String)>,
) -> impl IntoResponse {
    let engines = state.engines.read().await;
    if engines.get(&name).is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    drop(engines);

    // Resolve the track work dir: workspace dir if present, else stage track dir.
    let ws_dir = state.project_dir.join(".loopy/workspaces").join(&track);
    let work_dir = if ws_dir.exists() {
        ws_dir
    } else {
        state.project_dir.join(".loopy/stages/orbital-lanes").join(&track)
    };
    let log_path = work_dir.join(".ralph/ralph-output.log");

    // Return the last ~500 lines so the client can render a tail without a huge payload.
    let content = std::fs::read_to_string(&log_path).unwrap_or_default();
    let lines: Vec<&str> = content.lines().collect();
    let tail: String = lines
        .iter()
        .rev()
        .take(500)
        .rev()
        .copied()
        .collect::<Vec<_>>()
        .join("\n");

    axum::Json(serde_json::json!({ "content": tail })).into_response()
}

#[derive(Debug, Deserialize)]
struct PlanTemplateRequest {
    prompt: String,
}

/// Suggest which pipeline template fits a freeform task prompt (the Loopy
/// "identify which loop steps are needed" step). Returns the chosen template id,
/// a short rationale, and the template's stages so the UI can preview the loop.
async fn plan_template(
    axum::Json(req): axum::Json<PlanTemplateRequest>,
) -> impl IntoResponse {
    let choice = crate::pipeline::plan_template_for_prompt(&req.prompt);
    let template = crate::pipeline::template_by_id(&choice.template_id)
        .unwrap_or_else(crate::pipeline::poc_from_design_doc);
    axum::Json(serde_json::json!({
        "template_id": choice.template_id,
        "reason": choice.reason,
        "name": template.name,
        "stages": template.stages.iter().map(|s| serde_json::json!({
            "id": s.id, "label": s.label, "kind": s.kind, "optional": s.optional,
        })).collect::<Vec<_>>(),
    }))
}

/// Enrich a rough task prompt into a clearer one via the agent backend (one-shot).
/// Returns the enriched prompt for the user to review/approve. On failure (backend
/// unavailable, timeout) returns 503 so the UI can fall back to the original.
async fn enrich_prompt(
    State(state): State<Arc<AppState>>,
    axum::Json(req): axum::Json<PlanTemplateRequest>,
) -> impl IntoResponse {
    if req.prompt.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "empty prompt").into_response();
    }
    let config = crate::config::LoopyConfig::load_or_default(&state.project_dir);
    match crate::orchestrator::enrich_prompt(&config, &req.prompt).await {
        Ok(enriched) => axum::Json(serde_json::json!({ "enriched": enriched })).into_response(),
        Err(e) => (StatusCode::SERVICE_UNAVAILABLE, format!("enrichment failed: {e}")).into_response(),
    }
}

/// All block kinds for the add-block palette, with flight-themed display names
/// and hover descriptions. UI-driven so adding a kind in the backend surfaces it.
async fn get_block_kinds() -> impl IntoResponse {
    let kinds: Vec<serde_json::Value> = crate::pipeline::StageKind::all()
        .iter()
        .map(|k| serde_json::json!({
            "kind": k.id(),
            "label": k.display_name(),
            "description": k.description(),
            "category": k.category(),
        }))
        .collect();
    axum::Json(serde_json::json!({ "kinds": kinds }))
}

/// Propose an editable block list for a task (the corrected Loopy flow): the UI
/// shows these blocks and lets the user add/remove/reorder before executing.
async fn plan_blocks(
    State(state): State<Arc<AppState>>,
    axum::Json(req): axum::Json<PlanTemplateRequest>,
) -> impl IntoResponse {
    // Try the LLM planner first (smarter for complex prompts); fall back to the
    // deterministic keyword heuristic if the backend is unavailable or misbehaves.
    let catalog: Vec<(&str, &str)> = crate::pipeline::StageKind::all()
        .iter()
        .map(|k| (k.id(), k.description()))
        .collect();
    let config = crate::config::LoopyConfig::load_or_default(&state.project_dir);
    let proposal = match crate::orchestrator::plan_blocks_llm(&config, &req.prompt, &catalog).await {
        Ok(ids) => crate::pipeline::compose_from_kinds(
            &ids,
            format!("AI planner chose {} block(s) for this task.", ids.len()),
        ),
        Err(_) => crate::pipeline::plan_blocks_for_task(&req.prompt),
    };
    axum::Json(serde_json::json!({
        "reason": proposal.reason,
        "blocks": proposal.blocks.iter().map(|b| serde_json::json!({
            "id": b.id,
            "label": b.kind.display_name(),
            "description": b.kind.description(),
            "kind": b.kind,
            "optional": b.optional,
            "checkpoint": b.checkpoint.is_some(),
            "locked": b.locked,
        })).collect::<Vec<_>>(),
    }))
}

/// List the available pipeline templates (Loopy presets). Read-only; lets the UI
/// show a preset picker. The first entry is today's Loopy pipeline.
async fn get_templates() -> impl IntoResponse {
    let templates: Vec<serde_json::Value> = crate::pipeline::builtin_templates()
        .into_iter()
        .map(|t| {
            serde_json::json!({
                "id": t.id,
                "name": t.name,
                "description": t.description,
                "executable": t.executable,
                "stages": t.stages.iter().map(|s| serde_json::json!({
                    "id": s.id,
                    "label": s.label,
                    "kind": s.kind,
                    "optional": s.optional,
                    "checkpoint": s.checkpoint,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    axum::Json(serde_json::json!({ "templates": templates }))
}

async fn get_config(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let config = crate::config::LoopyConfig::load_or_default(&state.project_dir);
    let context_sources: Vec<serde_json::Value> = config.context.iter().map(|c| {
        serde_json::json!({
            "type": format!("{:?}", c.source_type).to_lowercase(),
            "path": c.path,
            "description": c.description,
        })
    }).collect();
    let tracks: Vec<String> = crate::config::load_track_definitions(&state.project_dir, &config)
        .keys().cloned().collect();

    axum::Json(serde_json::json!({
        "project": config.project,
        "backend": config.backend,
        "max_iterations": config.max_iterations,
        "context_sources": context_sources,
        "tracks_available": tracks,
        "idea_doc": config.idea_doc,
    }))
}

async fn ws_handler(
    State(state): State<Arc<AppState>>,
    AxumPath(name): AxumPath<String>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state, name))
}

async fn handle_ws(mut socket: WebSocket, state: Arc<AppState>, project_name: String) {
    // Send initial state
    {
        let engines = state.engines.read().await;
        if let Some(engine) = engines.get(&project_name) {
            let msg = WsServerMsg::State { data: engine.state.clone() };
            if let Ok(json) = serde_json::to_string(&msg) {
                let _ = socket.send(Message::Text(json.into())).await;
            }
        }
    }

    let mut rx = state.broadcast.subscribe();
    let (mut ws_tx, mut ws_rx) = socket.split();

    loop {
        tokio::select! {
            msg = ws_rx.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(client_msg) = serde_json::from_str::<WsClientMsg>(&text) {
                            let event = match client_msg {
                                WsClientMsg::Approve => {
                                    let engines = state.engines.read().await;
                                    match engines.get(&project_name).map(|e| e.state.phase) {
                                        Some(Phase::AwaitingPlanReview) => Some(EngineEvent::UserApprovedPlan),
                                        // WS approve has no payload; default to no test flight.
                                        Some(Phase::AwaitingCodeReview) => Some(EngineEvent::UserApprovedCode { run_test_flight: false }),
                                        Some(Phase::AwaitingTestPlan) => Some(EngineEvent::UserApprovedTestPlan),
                                        Some(Phase::AwaitingTestReview) => Some(EngineEvent::UserAcceptedTest),
                                        _ => None,
                                    }
                                }
                                WsClientMsg::Reject { feedback } => {
                                    let engines = state.engines.read().await;
                                    match engines.get(&project_name).map(|e| e.state.phase) {
                                        Some(Phase::AwaitingPlanReview) => Some(EngineEvent::UserRejectedPlan { feedback }),
                                        Some(Phase::AwaitingCodeReview) => Some(EngineEvent::UserRejectedCode { feedback }),
                                        Some(Phase::AwaitingTestReview) => Some(EngineEvent::UserRejectedTest { feedback }),
                                        _ => None,
                                    }
                                }
                                WsClientMsg::Abort => Some(EngineEvent::Abort),
                            };

                            if let Some(event) = event {
                                // Non-blocking send so a busy runner can't wedge the WS task.
                                let channels = state.action_channels.read().await;
                                if let Some(tx) = channels.get(&project_name) {
                                    let _ = tx.try_send(event);
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
            broadcast_msg = rx.recv() => {
                if let Ok((name, msg)) = broadcast_msg {
                    if name == project_name {
                        if let Ok(json) = serde_json::to_string(&msg) {
                            if ws_tx.send(Message::Text(json.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
}

fn slug_from_idea(idea: &str) -> String {
    idea.chars()
        .take(40)
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn test_app() -> Router {
        let state = Arc::new(AppState::new(PathBuf::from("/tmp/loopy-test")));
        router(state)
    }

    #[tokio::test]
    async fn list_projects_empty() {
        let app = test_app();
        let resp = app
            .oneshot(Request::builder().uri("/api/projects").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let projects: Vec<ProjectSummary> = serde_json::from_slice(&body).unwrap();
        assert!(projects.is_empty());
    }

    #[tokio::test]
    async fn create_and_get_project() {
        let state = Arc::new(AppState::new(PathBuf::from("/tmp/loopy-test")));
        let app = router(state.clone());

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/projects")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"idea":"Add rate limiting"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/projects/add-rate-limiting")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn approve_at_wrong_phase_returns_conflict() {
        let state = Arc::new(AppState::new(PathBuf::from("/tmp/loopy-test")));
        let app = router(state.clone());

        // Create project (starts in Scanning phase)
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/projects")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"idea":"test"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Try to approve while scanning (not at a checkpoint)
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/projects/test/approve")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn approve_plan_advances_to_setup() {
        let state = Arc::new(AppState::new(PathBuf::from("/tmp/loopy-test")));

        // Manually set up engine at AwaitingPlanReview
        let mut engine = Engine::new("test", "test-project");
        engine.transition(EngineEvent::Started { idea: "test".into() });
        engine.transition(EngineEvent::ScanComplete);
        engine.transition(EngineEvent::PlanComplete);
        assert_eq!(engine.state.phase, Phase::AwaitingPlanReview);

        state.engines.write().await.insert("test-project".into(), engine);

        // Register a fake action channel
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        state.action_channels.write().await.insert("test-project".into(), tx);

        let app = router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/projects/test-project/approve")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Verify the event was sent to the channel
        let event = rx.recv().await.unwrap();
        assert_eq!(event, EngineEvent::UserApprovedPlan);
    }

    #[tokio::test]
    async fn reject_plan_cycles_back() {
        let state = Arc::new(AppState::new(PathBuf::from("/tmp/loopy-test")));

        let mut engine = Engine::new("test", "test-project");
        engine.transition(EngineEvent::Started { idea: "test".into() });
        engine.transition(EngineEvent::ScanComplete);
        engine.transition(EngineEvent::PlanComplete);

        state.engines.write().await.insert("test-project".into(), engine);

        // Register a fake action channel
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        state.action_channels.write().await.insert("test-project".into(), tx);

        let app = router(state.clone());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/projects/test-project/reject")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"feedback":"too complex, simplify"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Verify the event was sent to the channel
        let event = rx.recv().await.unwrap();
        assert!(matches!(event, EngineEvent::UserRejectedPlan { feedback } if feedback == "too complex, simplify"));
    }

    #[test]
    fn slug_from_idea_works() {
        assert_eq!(slug_from_idea("Add rate limiting"), "add-rate-limiting");
        assert_eq!(slug_from_idea("Fix the bug!"), "fix-the-bug");
        assert_eq!(slug_from_idea(""), "");
    }
}
