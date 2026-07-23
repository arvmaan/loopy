use clap::Parser;

use loopy::cli::{Cli, Command, kill_stale_ralph, load_or_fresh, print_status, project_path};

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    /// If --name not given, read the project name from loopy.yml.
    fn resolve_name_from_config(name: Option<String>) -> Option<String> {
        name.or_else(|| {
            loopy::config::LoopyConfig::load_or_default(&std::env::current_dir().unwrap_or_default())
                .project
        })
    }

    match cli.command {
        Command::Start {
            idea,
            name,
            port,
            no_open,
        } => {
            // Kill any stale agent loops from a previous run BEFORE starting/resuming,
            // so restarting doesn't spawn fresh loops on top of orphaned ones.
            kill_stale_ralph(&std::env::current_dir().unwrap_or_default());

            let loopy_config = loopy::config::LoopyConfig::load_or_default(
                &std::env::current_dir().unwrap_or_default(),
            );
            let resolved_name = name.or(loopy_config.project);
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(run_server(idea, resolved_name, port, no_open))
        }
        Command::Status { name } => {
            let resolved = resolve_name_from_config(name);
            let state = load_or_fresh(&project_path(resolved.as_deref()));
            print!("{}", print_status(&state));
            Ok(())
        }
        Command::List => {
            print!("{}", loopy::cli::list_projects());
            Ok(())
        }
        Command::Blocks { check } => {
            use loopy::pipeline::{StageBlock, StageKind};
            let project_root = std::env::current_dir().unwrap_or_default();
            let config = loopy::config::LoopyConfig::load_or_default(&project_root);
            if !check {
                // List every block: name, kind, category, description.
                println!("Pipeline blocks ({} total):\n", StageKind::all().len());
                for k in StageKind::all() {
                    println!("  {:<14} [{}]  {}", k.display_name(), k.category(), k.id());
                    println!("                 {}", k.description());
                }
                return Ok(());
            }
            // --check: validate each block produces a non-empty ralph.yml + PROMPT.md.
            println!("Smoke-testing every block (dry-run, no processes spawned):\n");
            let mut failures = 0;
            for k in StageKind::all() {
                let block = StageBlock::simple(k.id(), k.display_name(), k);
                let yml = loopy::orchestrator::ralph_yml_for_block(&block, &config, &project_root);
                let prompt =
                    loopy::orchestrator::prompt_for_block(&block, "smoke test task", &[], None);
                let promise = k.completion_promise();
                let ok = yml.contains("completion_promise:")
                    && yml.contains(promise)
                    && prompt.contains(k.role_line())
                    && prompt.contains(promise);
                println!(
                    "  {} {:<14} promise={}",
                    if ok { "✅" } else { "❌" },
                    k.display_name(),
                    promise
                );
                if !ok {
                    failures += 1;
                    println!("       yml/prompt missing expected fields");
                }
            }
            if failures == 0 {
                println!(
                    "\nAll {} blocks generate valid config + prompt ✅",
                    StageKind::all().len()
                );
                Ok(())
            } else {
                Err(anyhow::anyhow!("{failures} block(s) failed validation"))
            }
        }
        Command::Clean { keep_state, stage } => {
            kill_stale_ralph(&std::env::current_dir().unwrap_or_default());
            if let Some(stage_name) = stage {
                let checkpoint = project_path(None);
                let mut state = load_or_fresh(&checkpoint);
                let target = state
                    .stages
                    .iter_mut()
                    .find(|s| loopy::orchestrator::stage_dir_name(s.id) == stage_name);
                if let Some(s) = target {
                    let stages_base = loopy::orchestrator::stages_dir(&checkpoint);
                    let dir = stages_base.join(&stage_name);
                    if dir.exists() {
                        std::fs::remove_dir_all(&dir)?;
                    }
                    s.status = loopy::models::StageStatus::Pending;
                    s.started_at = None;
                    s.completed_at = None;
                    s.error = None;
                    if let Err(e) = loopy::checkpoint::save(&checkpoint, &state) {
                        eprintln!("checkpoint save error: {e}");
                    }
                    println!("Cleaned stage: {stage_name}");
                } else {
                    let valid: Vec<&str> = state
                        .stages
                        .iter()
                        .map(|s| loopy::orchestrator::stage_dir_name(s.id))
                        .collect();
                    eprintln!("Unknown stage: {stage_name}");
                    eprintln!("Valid stages: {}", valid.join(", "));
                }
            } else {
                let loopy_dir = std::path::Path::new(".loopy");
                if keep_state {
                    let stages = loopy_dir.join("stages");
                    if stages.exists() {
                        std::fs::remove_dir_all(&stages)?;
                    }
                } else if loopy_dir.exists() {
                    std::fs::remove_dir_all(loopy_dir)?;
                }
                println!("Cleaned.");
            }
            Ok(())
        }
        Command::Init => {
            let project_root = std::env::current_dir()?;
            let created = loopy::orchestrator::loopy_init(&project_root, None, true)?;
            println!("✅ Loopy project initialized:");
            for item in &created {
                println!("  {item}");
            }
            Ok(())
        }
        Command::Doctor => {
            let config = loopy::config::LoopyConfig::load_or_default(
                &std::env::current_dir().unwrap_or_default(),
            );
            let backend = loopy::orchestrator::resolve_backend(&config);
            let mut passed = 0u32;
            let total = 2u32;

            let agent_ok = std::process::Command::new(backend)
                .arg("--version")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if agent_ok {
                passed += 1;
                println!("✅ Agent backend ({backend})");
            } else {
                println!("❌ Agent backend ({backend}) — not found on PATH");
            }

            if check_disk_space() {
                passed += 1;
                println!("✅ Disk space (≥1 GB free)");
            } else {
                println!("❌ Disk space (<1 GB free)");
            }

            println!("\n{passed}/{total} checks passed");
            if passed < total {
                std::process::exit(1);
            }
            Ok(())
        }
    }
}

async fn run_server(
    idea: Option<String>,
    name: Option<String>,
    port: u16,
    no_open: bool,
) -> anyhow::Result<()> {
    use loopy::engine::Engine;
    use loopy::web_v2::{self, AppState, spawn_engine_runner};
    use std::sync::Arc;

    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("loopy=info"))
        .format_timestamp_secs()
        .try_init();

    // Pre-flight: warn if the filesystem is critically low on inodes. Loopy (via
    // the agent loop) creates many small files; if inodes are exhausted, stages
    // fail silently and the pipeline appears to hang.
    if let Some(free) = free_inodes() {
        if free < 50_000 {
            eprintln!("⚠️  WARNING: only {free} free inodes on this filesystem.");
            eprintln!("   Loopy creates many small files per stage — low inodes cause");
            eprintln!("   silent failures. Free some up before running a pipeline.\n");
        }
    }

    let project_root = std::env::current_dir()?;
    let state = Arc::new(AppState::new(project_root.clone()));

    // Load existing project state from checkpoint if available
    let checkpoint_path = project_root.join(".loopy/state.json");
    if checkpoint_path.exists() {
        if let Ok(Some(pipeline_state)) = loopy::checkpoint::load(&checkpoint_path) {
            let proj_name = name
                .clone()
                .or_else(|| loopy::config::LoopyConfig::load_or_default(&project_root).project)
                .unwrap_or_else(|| "project".to_string());
            use loopy::engine::Phase;
            use loopy::models::{StageId, StageStatus};
            let mut engine = Engine::new(&pipeline_state.idea_text, &proj_name);

            // Restore the saved stages/tracks/idea if anything has progressed
            // beyond a fresh start (any stage running or complete).
            let has_progress = pipeline_state.stages.iter().any(|s| {
                matches!(
                    s.status,
                    StageStatus::Complete | StageStatus::Running | StageStatus::Failed
                )
            });
            if has_progress {
                engine.state.stages = pipeline_state.stages.clone();
                engine.state.tracks = pipeline_state.tracks.clone();
                engine.state.idea = pipeline_state.idea_text.clone();

                // A RUNNING stage means we were interrupted mid-work and should
                // resume it. Otherwise fall back to the last completed stage's
                // checkpoint.
                let running = pipeline_state
                    .stages
                    .iter()
                    .find(|s| s.status == StageStatus::Running);
                let last_complete = pipeline_state
                    .stages
                    .iter()
                    .rev()
                    .find(|s| s.status == StageStatus::Complete);

                engine.state.phase = if let Some(s) = running {
                    match s.id {
                        StageId::Scan => Phase::Scanning,
                        StageId::Plan => Phase::Planning,
                        StageId::OrbitalLanes => Phase::RunningTracks,
                        StageId::TestFlight => Phase::AwaitingTestPlan,
                        StageId::Land => Phase::Landing,
                        _ => Phase::Scanning,
                    }
                } else if let Some(s) = last_complete {
                    match s.id {
                        StageId::Land => Phase::Complete,
                        StageId::TestFlight => Phase::Landing,
                        StageId::OrbitalLanes => Phase::AwaitingCodeReview,
                        StageId::Plan => Phase::AwaitingPlanReview,
                        StageId::Scan => Phase::Planning,
                        _ => Phase::Scanning,
                    }
                } else {
                    engine.state.phase
                };
            }
            let loaded_phase = engine.state.phase;
            let loaded_idea = engine.state.idea.clone();
            state.engines.write().await.insert(proj_name.clone(), engine);
            log::info!(
                "Loaded existing project: {} (phase: {:?})",
                proj_name,
                loaded_phase
            );

            // Spawn a runner in resume mode for any non-fresh phase: checkpoints
            // (so approve/reject works) AND running stages (re-spawns the stage's
            // agent so an interrupted scan/plan/track continues).
            if !matches!(
                loaded_phase,
                Phase::Initializing | Phase::Complete | Phase::Failed
            ) {
                loopy::web_v2::spawn_engine_runner_resume(
                    state.clone(),
                    proj_name.clone(),
                    loaded_idea,
                );
            }
        }
    }

    // Determine URL: if we loaded a project and no new idea is being started, point to it.
    let loaded_project = state.engines.read().await.keys().next().cloned();
    let mut open_url = if let Some(ref proj) = loaded_project {
        format!("http://localhost:{port}/projects/{proj}")
    } else {
        format!("http://localhost:{port}")
    };

    // If an idea was provided on the CLI, create and start the project (or resume).
    if let Some(idea_text) = idea {
        let project_name = name.unwrap_or_else(|| {
            idea_text
                .chars()
                .take(40)
                .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
                .collect::<String>()
                .trim_matches('-')
                .to_string()
        });

        let already_loaded = state.engines.read().await.contains_key(&project_name);
        if !already_loaded {
            let mut engine = Engine::new(&idea_text, &project_name);
            engine.transition(loopy::engine::EngineEvent::Started {
                idea: idea_text.clone(),
            });
            state
                .engines
                .write()
                .await
                .insert(project_name.clone(), engine);
            spawn_engine_runner(state.clone(), project_name.clone(), idea_text);
        } else {
            // Resume existing project — don't wipe stages.
            let engines = state.engines.read().await;
            let phase = engines.get(&project_name).map(|e| e.state.phase);
            drop(engines);
            log::info!("Resuming existing project: {} (phase: {:?})", project_name, phase);
            if let Some(p) = phase {
                use loopy::engine::Phase;
                if matches!(
                    p,
                    Phase::AwaitingPlanReview
                        | Phase::AwaitingCodeReview
                        | Phase::Scanning
                        | Phase::Planning
                ) {
                    loopy::web_v2::spawn_engine_runner_resume(
                        state.clone(),
                        project_name.clone(),
                        idea_text,
                    );
                }
            }
        }
        open_url = format!("http://localhost:{port}/projects/{project_name}");
    }

    // Build router and start server.
    let app = web_v2::router(state.clone());
    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("Loopy running at {open_url}");

    if let Some(ref proj) = loaded_project {
        let engines = state.engines.read().await;
        if let Some(engine) = engines.get(proj) {
            let phase_hint = match engine.state.phase {
                loopy::engine::Phase::AwaitingPlanReview => {
                    " (awaiting plan review — approve or reject in browser)"
                }
                loopy::engine::Phase::AwaitingCodeReview => {
                    " (awaiting code review — approve or reject in browser)"
                }
                loopy::engine::Phase::Complete => " (pipeline complete)",
                loopy::engine::Phase::Failed => " (pipeline failed — check logs)",
                _ => "",
            };
            if !phase_hint.is_empty() {
                println!("  {}{}", proj, phase_hint);
            }
        }
    }

    if !no_open {
        let _ = std::process::Command::new("open")
            .arg(&open_url)
            .spawn()
            .or_else(|_| std::process::Command::new("xdg-open").arg(&open_url).spawn());
    }

    // Serve with graceful shutdown: on Ctrl+C (SIGINT) OR SIGTERM (`kill <pid>`),
    // kill the detached agent loops + their descendants so nothing lingers as an
    // orphan. SIGTERM matters because that's how the server is usually killed.
    let shutdown_root = project_root.clone();
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let ctrl_c = async {
                let _ = tokio::signal::ctrl_c().await;
            };
            #[cfg(unix)]
            let term = async {
                if let Ok(mut s) =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                {
                    s.recv().await;
                }
            };
            #[cfg(not(unix))]
            let term = std::future::pending::<()>();
            tokio::select! { _ = ctrl_c => {}, _ = term => {} }
            println!("\nShutting down — stopping agent loops...");
            let n = loopy::orchestrator::kill_all_project_ralph(&shutdown_root);
            if n > 0 {
                println!("Stopped {n} agent process group(s).");
            }
        })
        .await?;
    // Belt-and-suspenders: also kill on normal exit.
    let _ = loopy::orchestrator::kill_all_project_ralph(&project_root);
    Ok(())
}

fn check_disk_space() -> bool {
    std::process::Command::new("df")
        .args(["-k", "."])
        .output()
        .ok()
        .and_then(|o| {
            let out = String::from_utf8_lossy(&o.stdout);
            let line = out.lines().nth(1)?;
            // df -k columns: Filesystem 1K-blocks Used Available ...
            let avail_kb: u64 = line.split_whitespace().nth(3)?.parse().ok()?;
            Some(avail_kb >= 1_048_576) // 1 GB in KB
        })
        .unwrap_or(false)
}

/// Free inodes on the current filesystem (via `df -i .`), or None if unknown.
fn free_inodes() -> Option<u64> {
    let out = std::process::Command::new("df").args(["-i", "."]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().nth(1)?;
    // df -i columns: Filesystem Inodes IUsed IFree IUse% Mounted
    line.split_whitespace().nth(3)?.parse().ok()
}
