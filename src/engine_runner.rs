use crate::aggregator::Aggregator;
use crate::config::LoopyConfig;
use crate::engine::{Effect, Engine, EngineEvent, EngineState, Phase};
use crate::models::StageId;
use crate::orchestrator::{self, LoopConfig, Orchestrator};
use crate::watcher::{self, FileEvent, WatcherHandle};
use std::path::PathBuf;
use tokio::sync::mpsc;

/// Is `pid` a live, non-zombie process? On Linux, `/proc/<pid>` exists for
/// zombies too (a finished-but-unreaped child), so we must check the process
/// state field in `/proc/<pid>/stat` and reject `Z` (zombie) and `X` (dead).
fn pid_is_live(pid: u64) -> bool {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false; // no /proc entry → not running
    };
    // Format: "pid (comm) STATE ...". comm may contain spaces/parens, so the
    // state char is the first token after the LAST ')'.
    let Some(after) = stat.rsplit(')').next() else {
        return false;
    };
    match after.trim_start().chars().next() {
        Some('Z') | Some('X') | Some('x') => false,
        Some(_) => true,
        None => false,
    }
}

/// True if a watcher events-file `source` belongs to the given block work dir.
/// A block's events live at `<block_dir>/.ralph/events-*.jsonl`, so the block
/// identity is in the DIRECTORY, not the (Ralph-chosen) filename — attributing by
/// filename/loop_id never matched and hung every linear pipeline on block 1.
fn event_source_matches_block(source: &std::path::Path, block_dir: &std::path::Path) -> bool {
    source.starts_with(block_dir)
}

pub struct EngineRunner {
    pub engine: Engine,
    orchestrator: Orchestrator,
    aggregator: Aggregator,
    watcher_handle: Option<WatcherHandle>,
    file_rx: Option<mpsc::Receiver<FileEvent>>,
    project_root: PathBuf,
    stages_dir: PathBuf,
    log_tx: mpsc::Sender<LogLine>,
    /// Execution cursor for a generic (non-POC) pipeline. `Some` only when the
    /// active template is executable AND not the POC pipeline — the POC pipeline
    /// runs via the `engine.transition()` state machine and leaves this `None`.
    linear_run: Option<crate::pipeline::LinearRun>,
    /// The task/idea text, so the linear driver can build block prompts in poll().
    task: String,
    /// When a linear run pauses at a checkpoint, the block awaiting human review.
    /// `Some` ⇒ the run is paused; `approve_linear`/`reject_linear` resume it.
    paused_block: Option<crate::pipeline::StageBlock>,
    /// Test-only: skip actually spawning agent processes. Driver state still
    /// advances; no real `ralph` is launched. Prevents tests from leaking
    /// processes (a fork-bomb hazard). Always false in production.
    no_spawn: bool,
    /// Wall-clock of the last observed progress (file event OR a spawn). Drives
    /// the liveness fallback: if the current agent process dies and no progress
    /// is seen for `HANG_GRACE`, the pipeline is unstuck (completed-if-output, or
    /// failed) instead of hanging forever. `None` until the first spawn.
    last_progress: Option<std::time::Instant>,
}

impl EngineRunner {
    /// Whether this runner drives a generic (non-POC) pipeline via the linear
    /// driver rather than the POC state machine.
    pub fn is_linear(&self) -> bool {
        self.linear_run.is_some()
    }
}

#[derive(Debug, Clone)]
pub struct LogLine {
    pub level: String,
    pub message: String,
}

impl EngineRunner {
    pub async fn new(
        engine: Engine,
        project_root: PathBuf,
        log_tx: mpsc::Sender<LogLine>,
    ) -> anyhow::Result<Self> {
        let stages_dir = project_root.join(".loopy/stages");
        std::fs::create_dir_all(&stages_dir)?;

        let (file_tx, file_rx) = mpsc::channel(64);
        let watcher_handle = watcher::start(stages_dir.clone(), file_tx).await?;

        let mut engine = engine;
        let engine_idea = engine.state.idea.clone();
        // Restore a persisted custom pipeline from disk if the in-memory engine
        // doesn't have one (e.g. after a server restart — the checkpoint doesn't
        // carry custom_pipeline, so without this a custom run reverts to POC).
        if engine.state.custom_pipeline.is_none() {
            if let Some(cp) = Self::load_custom_pipeline(&project_root) {
                engine.state.custom_pipeline = Some(cp);
            }
        }
        // Set up a linear run only for an executable, non-POC template. The POC
        // pipeline uses the engine state machine and leaves this None.
        let template = engine.template();
        let linear_run = if template.executable && template.id != "poc-from-design-doc" {
            // Restore a persisted run (resume) if it matches this template;
            // otherwise begin a fresh run. This makes generic pipelines survive
            // a server restart instead of replaying from the first block.
            match Self::load_linear_run(&project_root) {
                Some(run) if run.template_id == template.id => Some(run),
                _ => Some(crate::pipeline::LinearRun::start(&template, true)),
            }
        } else {
            None
        };

        Ok(Self {
            engine,
            orchestrator: Orchestrator::default(),
            aggregator: Aggregator::new(),
            watcher_handle: Some(watcher_handle),
            file_rx: Some(file_rx),
            project_root,
            stages_dir,
            log_tx,
            linear_run,
            task: engine_idea,
            paused_block: None,
            no_spawn: false,
            last_progress: None,
        })
    }

    /// Grace period after the last progress + agent-process death before the
    /// liveness fallback unsticks a stalled pipeline. Generous so it never fires
    /// during normal between-iteration gaps of a live agent.
    const HANG_GRACE: std::time::Duration = std::time::Duration::from_secs(90);

    /// Liveness fallback: is the current run wedged? True when (a) we've spawned
    /// something, (b) no progress for `HANG_GRACE`, and (c) no agent loop is still
    /// alive. Used by `poll()` so a crashed/exited agent that never emitted a
    /// completion event can't hang the pipeline forever.
    fn is_stalled(&mut self) -> bool {
        let Some(since) = self.last_progress else {
            return false;
        };
        if since.elapsed() < Self::HANG_GRACE {
            return false;
        }
        // Any loop this runner spawned still running? (ralph is one long-lived
        // process per loop, so a live child means work is genuinely in progress.)
        let ids: Vec<String> = self.orchestrator.active_loops().keys().cloned().collect();
        !ids.iter().any(|id| self.orchestrator.is_alive(id))
    }

    /// Unstick a stalled run (its agent died without emitting completion). If the
    /// current stage/block produced its expected output we accept it as complete
    /// and advance; otherwise we fail the stage/run so it terminates instead of
    /// hanging. Returns true if state changed. Resets the hang timer either way so
    /// we don't re-fire every tick.
    async fn handle_stall(&mut self) -> bool {
        self.last_progress = Some(std::time::Instant::now());

        if self.is_linear() {
            let Some(current_id) = self.linear_run.as_ref().and_then(|r| r.current.clone()) else {
                return false;
            };
            if self.block_has_output(&current_id) {
                self.log("warn", &format!(
                    "Block '{current_id}' agent exited without a completion event but produced output — advancing"
                )).await;
                self.drive_linear().await;
            } else {
                self.log("error", &format!(
                    "Block '{current_id}' agent exited without completing and produced no output — stopping pipeline"
                )).await;
                orchestrator::kill_all_project_ralph(&self.project_root);
                if let Some(run) = self.linear_run.as_mut() {
                    run.done = true;
                    run.current = None;
                }
                self.paused_block = None;
                self.save_linear_run();
            }
            return true;
        }

        // POC path: map the running phase to its stage.
        let stage = match self.engine.state.phase {
            Phase::Scanning => StageId::Scan,
            Phase::Planning => StageId::Plan,
            Phase::Landing => StageId::Land,
            Phase::TestFlying => StageId::TestFlight,
            // Not in a single-agent running phase (e.g. RunningTracks is handled by
            // reconcile_tracks; review phases wait on the user) — nothing to unstick.
            _ => return false,
        };
        if self.stage_has_output(stage) {
            self.log("warn", &format!(
                "{stage:?} agent exited without a completion event but produced output — accepting"
            )).await;
            let stage_dir = self.stages_dir.join(orchestrator::stage_dir_name(stage));
            let _ = orchestrator::collect_stage_output(&stage_dir);
            if stage == StageId::Plan {
                self.load_tracks_from_plan(&stage_dir);
            }
            let event = match stage {
                StageId::Scan => EngineEvent::ScanComplete,
                StageId::Plan => EngineEvent::PlanComplete,
                StageId::Land => EngineEvent::LandComplete,
                StageId::TestFlight => EngineEvent::TestFlightComplete,
                _ => return false,
            };
            let effects = self.engine.transition(event);
            self.process_effects(effects).await;
        } else {
            self.log("error", &format!(
                "{stage:?} agent exited without completing and produced no output — failing stage"
            )).await;
            let effects = self.engine.transition(EngineEvent::StageFailed {
                stage,
                error: "agent process exited without completing".into(),
            });
            self.process_effects(effects).await;
        }
        true
    }

    /// Test-only: disable real process spawning so driver-logic tests never
    /// launch `ralph` (avoids leaking processes / fork bombs).
    #[cfg(test)]
    pub fn set_no_spawn(&mut self, v: bool) {
        self.no_spawn = v;
    }

    pub fn state(&self) -> &EngineState {
        &self.engine.state
    }

    pub fn phase(&self) -> Phase {
        self.engine.state.phase
    }

    pub async fn start(&mut self, idea: String) {
        // Clean stale stage CONTENTS (but not the watched stages_dir itself —
        // removing the watched dir invalidates the inotify watch on Linux).
        if let Ok(entries) = std::fs::read_dir(&self.stages_dir) {
            for entry in entries.flatten() {
                let _ = std::fs::remove_dir_all(entry.path())
                    .or_else(|_| std::fs::remove_file(entry.path()));
            }
        }
        // Generic (non-POC) pipelines run via the linear driver, not the POC
        // state machine. The POC path is untouched.
        if self.is_linear() {
            self.task = idea;
            self.start_linear().await;
            return;
        }
        let effects = self.engine.transition(EngineEvent::Started { idea });
        self.process_effects(effects).await;
    }

    /// Begin a generic (non-POC) pipeline run FRESH: reset the cursor to the
    /// first block and spawn it. No-op for POC runs. (Resume uses
    /// resume_linear() instead, which keeps persisted progress.)
    pub async fn start_linear(&mut self) -> bool {
        let template = self.engine.template();
        if self.linear_run.is_none() {
            return false;
        }
        // Fresh start: ignore any stale persisted run.
        let run = crate::pipeline::LinearRun::start(&template, true);
        self.linear_run = Some(run.clone());
        self.paused_block = None;
        // Persist the custom pipeline + run immediately so a restart resumes the
        // right blocks instead of reverting to POC.
        self.save_custom_pipeline();
        self.save_linear_run();
        let Some(block) = run.current_block(&template).cloned() else {
            return false;
        };
        let task = self.task.clone();
        self.log("info", &format!("Starting pipeline '{}' at block '{}'", template.id, block.id)).await;
        self.spawn_block(&block, &task, &[], None).await;
        self.save_linear_run();
        true
    }

    /// Resume a generic pipeline after a restart: re-spawn the persisted current
    /// block (or do nothing if paused at a checkpoint / already done). No-op for
    /// POC runs. Keeps progress, unlike start_linear().
    pub async fn resume_linear(&mut self) -> bool {
        let template = self.engine.template();
        let Some(run) = self.linear_run.clone() else {
            return false;
        };
        if run.done {
            self.log("info", "Pipeline already complete on resume").await;
            return true;
        }
        if self.paused_block.is_some() {
            // Was awaiting review — leave it paused for the user.
            return true;
        }
        let Some(block) = run.current_block(&template).cloned() else {
            return false;
        };
        let task = self.task.clone();
        let completed = run.completed.clone();
        self.log("info", &format!("Resuming pipeline '{}' at block '{}'", template.id, block.id)).await;
        self.spawn_block(&block, &task, &completed, None).await;
        true
    }

    /// Handle completion of the current block in a generic pipeline. Called when
    /// the watcher observes the active block's completion promise. Advances the
    /// LinearRun and spawns the next block (or stops at a checkpoint / finishes).
    /// No-op for POC runs. Returns true if it acted.
    pub async fn drive_linear(&mut self) -> bool {
        use crate::pipeline::DriverAction;
        let template = self.engine.template();
        let Some(mut run) = self.linear_run.clone() else {
            return false;
        };
        let completed: Vec<String> = run.completed.clone();
        let action = crate::pipeline::driver_next(&mut run, &template);
        self.linear_run = Some(run);
        let task = self.task.clone();
        match action {
            DriverAction::Spawn(block) => {
                self.log("info", &format!("Pipeline → spawning '{}'", block.id)).await;
                self.spawn_block(&block, &task, &completed, None).await;
            }
            DriverAction::PauseForCheckpoint(block) => {
                self.log("info", &format!("Pipeline → '{}' needs review (paused)", block.id)).await;
                // Record the paused block; approve_linear/reject_linear resume it.
                self.paused_block = Some(block);
            }
            DriverAction::Done => {
                self.log("info", "Pipeline complete").await;
            }
        }
        // Persist advanced progress so a restart resumes here, not from block 1.
        self.save_linear_run();
        true
    }

    /// Is a linear run paused at a checkpoint awaiting review? Returns the block.
    pub fn paused_block(&self) -> Option<&crate::pipeline::StageBlock> {
        self.paused_block.as_ref()
    }

    /// Snapshot of the linear run for the UI: (template_id, stages, current,
    /// completed, paused_block_id, done). `None` for POC runs.
    pub fn linear_snapshot(&self) -> Option<(
        String,
        Vec<crate::pipeline::StageBlock>,
        Option<String>,
        Vec<String>,
        Option<String>,
        bool,
    )> {
        let run = self.linear_run.as_ref()?;
        let template = self.engine.template();
        Some((
            template.id.clone(),
            template.stages.clone(),
            run.current.clone(),
            run.completed.clone(),
            self.paused_block.as_ref().map(|b| b.id.clone()),
            run.done,
        ))
    }

    /// Approve a paused linear checkpoint: spawn the awaited block and resume.
    /// No-op if not paused. Returns true if it resumed.
    pub async fn approve_linear(&mut self) -> bool {
        let Some(block) = self.paused_block.take() else {
            return false;
        };
        let task = self.task.clone();
        let completed = self.linear_run.as_ref().map(|r| r.completed.clone()).unwrap_or_default();
        self.log("info", &format!("Checkpoint approved → spawning '{}'", block.id)).await;
        self.spawn_block(&block, &task, &completed, None).await;
        self.save_linear_run();
        true
    }

    /// Reject a paused linear checkpoint: route feedback back to the block's
    /// `on_reject` target (additive) and re-run from there. No-op if not paused.
    pub async fn reject_linear(&mut self, feedback: String) -> bool {
        use crate::pipeline::RerouteTarget;
        let Some(block) = self.paused_block.take() else {
            return false;
        };
        let template = self.engine.template();
        let task = self.task.clone();

        // Resolve the reroute target block id.
        let target_id = match block.on_reject {
            Some(RerouteTarget::SelfStage) | None => block.id.clone(),
            Some(RerouteTarget::Implement) => template
                .stages
                .iter()
                .find(|s| s.kind == crate::pipeline::StageKind::Implement)
                .map(|s| s.id.clone())
                .unwrap_or_else(|| block.id.clone()),
            Some(RerouteTarget::Plan) => template
                .stages
                .iter()
                .find(|s| s.kind == crate::pipeline::StageKind::Plan)
                .map(|s| s.id.clone())
                .unwrap_or_else(|| block.id.clone()),
        };

        // Reset the run cursor to the target and re-spawn it with the feedback.
        if let (Some(run), Some(target)) = (self.linear_run.as_mut(), template.stage(&target_id)) {
            run.current = Some(target_id.clone());
            run.done = false;
            let completed = run.completed.clone();
            let target = target.clone();
            self.log("info", &format!("Checkpoint changes requested → re-running '{}'", target_id)).await;
            self.spawn_block(&target, &task, &completed, Some(&feedback)).await;
            self.save_linear_run();
            return true;
        }
        false
    }

    /// Whether a block actually produced output — used to reject a spurious
    /// completion before the block did real work. A block has output if its
    /// `.ralph` dir shows a landed/completion signal in the agent log or events.
    fn block_has_output(&self, block_id: &str) -> bool {
        let ralph = self.stages_dir.join(block_id).join(".ralph");
        // Landing line in the raw log is the strongest "the agent finished" signal.
        if let Ok(lines) = crate::watcher::read_last_lines(&ralph.join("ralph-output.log"), 200) {
            if lines.iter().any(|l|
                l.contains("Primary loop landed successfully")
                || l.contains("Completion promise detected")
                || l.contains("LOOP_COMPLETE"))
            {
                return true;
            }
        }
        false
    }

    /// Re-spawn the Ralph process for the currently-running stage after a restart.
    /// Used on resume when a stage was interrupted (SIGTERM) mid-run.
    pub async fn respawn_active_stage(&mut self) {
        // Generic (non-POC) pipelines resume via the persisted LinearRun, not the
        // POC phase machine.
        if self.is_linear() {
            self.resume_linear().await;
            return;
        }
        match self.engine.state.phase {
            Phase::Scanning => {
                self.log("info", "Resuming scan (re-spawning Ralph)").await;
                self.spawn_stage(StageId::Scan, None).await;
            }
            Phase::Planning => {
                self.log("info", "Resuming plan (re-spawning Ralph)").await;
                self.spawn_stage(StageId::Plan, None).await;
            }
            Phase::Landing => {
                self.log("info", "Resuming land (re-spawning Ralph)").await;
                self.spawn_stage(StageId::Land, None).await;
            }
            Phase::TestFlying => {
                // Test Flight was interrupted mid-run — re-spawn the agent instance
                // against the already-approved TESTING-PLAN.md.
                self.log("info", "Resuming test flight (re-spawning agent)").await;
                self.spawn_test_flight(None).await;
            }
            Phase::RunningTracks => {
                // First, reconcile from disk: tracks that already landed while the
                // server was down must be marked complete BEFORE we decide what to
                // re-spawn — otherwise we'd launch fresh Ralph loops redoing work
                // that's already done (and possibly advance straight to code review).
                if self.reconcile_tracks().await {
                    self.log("info", "Reconciled completed tracks from disk on resume").await;
                }
                // If reconciliation advanced us out of RunningTracks, nothing to respawn.
                if self.engine.state.phase != Phase::RunningTracks {
                    return;
                }
                // Re-spawn only tracks that are still genuinely incomplete AND whose
                // loop is not already running on disk (avoid duplicate loops).
                let track_ids: Vec<String> = self.engine.state.tracks.as_ref()
                    .map(|tracks| tracks.iter()
                        .filter(|t| !matches!(t.status, crate::models::TrackStatus::Complete | crate::models::TrackStatus::Failed | crate::models::TrackStatus::Skipped))
                        .filter(|t| !self.track_loop_alive(&t.id))
                        .map(|t| t.id.clone())
                        .collect())
                    .unwrap_or_default();
                for track_id in track_ids {
                    self.log("info", &format!("Resuming track {} (re-spawning)", track_id)).await;
                    let more = self.spawn_track(&track_id, None).await;
                    self.process_effects(more).await;
                }
            }
            _ => {}
        }
    }

    pub async fn approve(&mut self) {
        let event = match self.engine.state.phase {
            Phase::AwaitingPlanReview => EngineEvent::UserApprovedPlan,
            Phase::AwaitingCodeReview => EngineEvent::UserApprovedCode { run_test_flight: false },
            Phase::AwaitingTestPlan => EngineEvent::UserApprovedTestPlan,
            Phase::AwaitingTestReview => EngineEvent::UserAcceptedTest,
            _ => return,
        };
        let effects = self.engine.transition(event);
        self.process_effects(effects).await;
    }

    pub async fn reject(&mut self, feedback: String) {
        let event = match self.engine.state.phase {
            Phase::AwaitingPlanReview => EngineEvent::UserRejectedPlan { feedback },
            Phase::AwaitingCodeReview => EngineEvent::UserRejectedCode { feedback },
            Phase::AwaitingTestReview => EngineEvent::UserRejectedTest { feedback },
            _ => return,
        };
        let effects = self.engine.transition(event);
        self.process_effects(effects).await;
    }

    pub async fn abort(&mut self) {
        let effects = self.engine.transition(EngineEvent::Abort);
        self.process_effects(effects).await;
    }

    /// Poll for file system events and advance the engine. Returns true if phase changed.
    pub async fn poll(&mut self) -> bool {
        // Drain all file events into a buffer first to avoid borrow conflicts
        let mut file_events = Vec::new();
        if let Some(rx) = &mut self.file_rx {
            while let Ok(event) = rx.try_recv() {
                file_events.push(event);
            }
        }

        if !file_events.is_empty() {
            // Any activity counts as progress and resets the hang timer.
            self.last_progress = Some(std::time::Instant::now());
        } else {
            // No events this tick. Completion is normally detected via watcher
            // events, but a crashed/OOM-killed/exited agent may never emit one —
            // which previously hung the pipeline forever. Liveness fallback: if the
            // agent process is dead and we've seen no progress for the grace
            // period, unstick the run (accept output-based completion, else fail).
            if self.is_stalled() {
                return self.handle_stall().await;
            }
            return false;
        }

        // Generic (non-POC) pipeline: drive blocks via the linear driver. This is
        // a SEPARATE path — the POC state machine below is never entered when
        // is_linear() is true.
        if self.is_linear() {
            // Drive blocks, but ONLY advance when the CURRENTLY-running block's own
            // ralph emits completion. Every block dir is watched, so we attribute
            // each event to a block by the DIRECTORY its events file lives in
            // (`<stages_dir>/<block_id>/.ralph/events-*.jsonl`) — NOT by loop_id,
            // which Ralph derives from its own internal id and never matches the
            // block id (that mismatch previously left every linear pipeline hung on
            // block 1 forever). A stale/late completion from an earlier block is
            // ignored because its source dir is a different block_id.
            let Some(current_id) = self.linear_run.as_ref().and_then(|r| r.current.clone()) else {
                return false;
            };
            let block_dir = self.stages_dir.join(&current_id);
            let mut completed = false;
            for file_event in file_events {
                let from_current = match &file_event {
                    crate::watcher::FileEvent::EventsAppended { source, .. } => {
                        event_source_matches_block(source, &block_dir)
                    }
                    _ => false,
                };
                let result = self.aggregator.apply(file_event);
                if !from_current {
                    continue; // ignore events from other blocks' dirs
                }
                if let Some(crate::models::PipelineEvent::StageCompleted { .. }) = result.pipeline_event {
                    completed = true;
                }
            }
            // Require the current block's output to actually exist before advancing
            // (guards against a spurious terminate before the block did its work).
            if completed && self.block_has_output(&current_id) {
                // Kill the completed block's Ralph loop before advancing so it
                // can't linger and keep emitting (the POC path already does this).
                let killed = self.orchestrator.kill_loops_for_block(&current_id).await;
                if killed > 0 {
                    self.log("info", &format!("Stopped {killed} Ralph loop(s) for block '{current_id}'")).await;
                }
                self.drive_linear().await;
                return true;
            }
            return false;
        }

        let old_phase = self.engine.state.phase;
        let mut stage_completed_this_batch = false;

        for file_event in file_events {
            let result = self.aggregator.apply(file_event);
            if let Some(pipeline_event) = result.pipeline_event {
                let engine_event = match pipeline_event {
                    crate::models::PipelineEvent::StageCompleted { stage: _ } => {
                        // Only advance one stage per poll batch — guards against a
                        // stale/duplicate terminate completing the next stage too.
                        if stage_completed_this_batch {
                            continue;
                        }
                        stage_completed_this_batch = true;
                        // Ralph's loop_id doesn't carry the Loopy stage, so the
                        // aggregator's `stage` is just a placeholder. The real stage
                        // is whatever is currently running — map from the phase.
                        // If we're not in a running-stage phase (e.g. already at a
                        // review checkpoint), ignore the event as a stale duplicate.
                        let actual_stage = match self.engine.state.phase {
                            Phase::Scanning => StageId::Scan,
                            Phase::Planning => StageId::Plan,
                            Phase::Landing => StageId::Land,
                            Phase::TestFlying => StageId::TestFlight,
                            _ => continue,
                        };
                        // Verify the stage actually produced its expected output before
                        // accepting completion. A terminate event can fire spuriously /
                        // get re-read; without real output it's a false completion that
                        // would skip the stage's work (e.g. Plan completing with no
                        // tracks.json, which prevented tracks from ever launching).
                        if !self.stage_has_output(actual_stage) {
                            self.log("warn", &format!(
                                "{:?} reported complete but produced no output yet — ignoring",
                                actual_stage
                            )).await;
                            continue;
                        }
                        // Kill the completed stage's Ralph loop so it doesn't linger
                        let killed = self.orchestrator.kill_loops_for_stage(actual_stage).await;
                        if killed > 0 {
                            self.log("info", &format!("Stopped {} Ralph loop(s) for {:?}", killed, actual_stage)).await;
                        }
                        // Collect stage outputs before transitioning
                        let stage_dir = self.stages_dir.join(orchestrator::stage_dir_name(actual_stage));
                        let _ = orchestrator::collect_stage_output(&stage_dir);
                        // Load tracks from Plan output
                        if actual_stage == StageId::Plan {
                            let plan_dir = self.stages_dir.join(orchestrator::stage_dir_name(StageId::Plan));
                            self.load_tracks_from_plan(&plan_dir);
                        }
                        match actual_stage {
                            StageId::Scan => Some(EngineEvent::ScanComplete),
                            StageId::Plan => Some(EngineEvent::PlanComplete),
                            StageId::Land => Some(EngineEvent::LandComplete),
                            StageId::TestFlight => Some(EngineEvent::TestFlightComplete),
                            _ => None,
                        }
                    }
                    crate::models::PipelineEvent::StageFailed { stage, error } => {
                        Some(EngineEvent::StageFailed { stage, error })
                    }
                    crate::models::PipelineEvent::TrackCompleted { track } => {
                        Some(EngineEvent::TrackCompleted { track_id: track })
                    }
                    crate::models::PipelineEvent::TrackFailed { track, error } => {
                        Some(EngineEvent::TrackFailed { track_id: track, error })
                    }
                    crate::models::PipelineEvent::TrackSetupComplete { .. } => {
                        Some(EngineEvent::WorkspaceSetupComplete)
                    }
                    _ => None,
                };

                if let Some(event) = engine_event {
                    let effects = self.engine.transition(event);
                    self.process_effects(effects).await;
                }
            }
        }

        self.engine.state.phase != old_phase
    }

    pub async fn process_effects(&mut self, effects: Vec<Effect>) {
        let mut pending: Vec<Effect> = effects;
        while !pending.is_empty() {
            let batch = std::mem::take(&mut pending);
            for effect in batch {
                match effect {
                    Effect::SpawnScan => {
                        self.spawn_stage(StageId::Scan, None).await;
                    }
                    Effect::SpawnPlan => {
                        self.spawn_stage(StageId::Plan, None).await;
                    }
                    Effect::SpawnPlanWithFeedback { feedback, .. } => {
                        // Read previous plan from disk
                        let plan_dir = self.stages_dir.join("plan");
                        let previous_plan = [
                            plan_dir.join("PLAN.md"),
                            plan_dir.join("output/PLAN.md"),
                            plan_dir.join("output/scratchpad.md"),
                            plan_dir.join(".ralph/agent/scratchpad.md"),
                        ].iter()
                            .find_map(|p| std::fs::read_to_string(p).ok());

                        let mut extra = format!(
                            "## User Feedback\n\nThe user rejected your previous plan. Here is their feedback:\n\n> {}\n\nPlease revise the plan to address this feedback.\n",
                            feedback
                        );
                        if let Some(prev) = previous_plan {
                            extra.push_str(&format!("\n## Your Previous Plan (for reference)\n\n{prev}\n"));
                        }
                        self.spawn_stage(StageId::Plan, Some(&extra)).await;
                    }
                    Effect::SetupWorkspaces { tracks } => {
                        self.log("info", &format!("Setting up workspaces for {} tracks", tracks.len())).await;
                        // Create workspace directories for each track
                        let workspaces_dir = self.project_root.join(".loopy/workspaces");
                        for track_id in &tracks {
                            let ws_dir = workspaces_dir.join(track_id);
                            if let Err(e) = std::fs::create_dir_all(&ws_dir) {
                                self.log("error", &format!("Failed to create workspace for {}: {}", track_id, e)).await;
                            } else {
                                // Initialize git repo (off the async runtime to avoid blocking)
                                if !ws_dir.join(".git").exists() {
                                    let dir = ws_dir.clone();
                                    let _ = tokio::task::spawn_blocking(move || {
                                        std::process::Command::new("git")
                                            .args(["init"])
                                            .current_dir(&dir)
                                            .output()
                                    }).await;
                                }
                                self.log("info", &format!("Workspace ready: {}", track_id)).await;
                            }
                        }
                        let inner = self.engine.transition(EngineEvent::WorkspaceSetupComplete);
                        pending.extend(inner);
                    }
                    Effect::SpawnTrack { track_id } => {
                        let more = self.spawn_track(&track_id, None).await;
                        pending.extend(more);
                    }
                    Effect::SpawnTrackWithFeedback { track_id, feedback } => {
                        let more = self.spawn_track(&track_id, Some(&feedback)).await;
                        pending.extend(more);
                    }
                    Effect::SpawnLand => {
                        self.spawn_stage(StageId::Land, None).await;
                    }
                    Effect::SaveCheckpoint => {
                        self.save_checkpoint();
                    }
                    Effect::NotifyPhaseChange { phase } => {
                        self.log("info", &format!("Phase: {:?}", phase)).await;
                    }
                    Effect::KillAll => {
                        // 1) Kill loops this orchestrator spawned (graceful TERM + reap).
                        let ids: Vec<String> = self.orchestrator.active_loops().keys().cloned().collect();
                        for id in ids {
                            let _ = self.orchestrator.kill(&id).await;
                        }
                        // 2) Disk sweep: SIGKILL every ralph process GROUP recorded in
                        //    this project's loop.lock files — reaps orphans this process
                        //    never spawned (previous-server leftovers). Hard requirement:
                        //    abort must never leave tasks filling the dev desktop.
                        let n = orchestrator::kill_all_project_ralph(&self.project_root);
                        if n > 0 {
                            self.log("warn", &format!("Abort: force-killed {n} stray Ralph process group(s)")).await;
                        }
                    }
                    Effect::PipelineDone => {
                        self.log("info", "Pipeline complete").await;
                    }
                    Effect::CreateCRs => {
                        self.log("info", "Creating CRs...").await;
                        let results = crate::cr::create_cr_for_tracks(
                            &self.to_v1_state(),
                            &self.project_root,
                        );
                        for r in &results {
                            if let Some(url) = &r.cr_url {
                                self.log("info", &format!("CR created for {}: {}", r.track_id, url)).await;
                            }
                        }
                    }
                    Effect::GenerateTestPlan => {
                        self.generate_test_plan().await;
                    }
                    Effect::SpawnTestFlight { feedback } => {
                        self.spawn_test_flight(feedback.as_deref()).await;
                    }
                }
            }
        }
    }

    async fn spawn_stage(&mut self, stage: StageId, extra_context: Option<&str>) {
        self.last_progress = Some(std::time::Instant::now());
        let config = LoopyConfig::load_or_default(&self.project_root);
        let idea = &self.engine.state.idea;
        let kb_path = self.project_root.join(".loopy/knowledge-base.md");
        let kb_content = std::fs::read_to_string(&kb_path).ok();

        match orchestrator::setup_stage_dir(
            stage,
            idea,
            kb_content.as_deref(),
            &self.project_root,
            &config,
            &self.stages_dir,
        ) {
            Ok(stage_dir) => {
                if let Some(extra) = extra_context {
                    let prompt_path = stage_dir.join("PROMPT.md");
                    if let Ok(existing) = std::fs::read_to_string(&prompt_path) {
                        let _ = std::fs::write(&prompt_path, format!("{existing}\n\n{extra}"));
                    }
                }

                let ralph_dir = stage_dir.join(".ralph");
                std::fs::create_dir_all(&ralph_dir).ok();
                // Remove stale lock from previous failed run
                let _ = std::fs::remove_file(ralph_dir.join("loop.lock"));
                if let Some(wh) = &mut self.watcher_handle {
                    let _ = wh.watch_path(&ralph_dir);
                }

                let hat = orchestrator::hat_collection_for_stage(stage).map(|raw| {
                    if raw.starts_with("builtin:") {
                        raw.to_string()
                    } else {
                        self.project_root.join(raw).display().to_string()
                    }
                });

                let loop_config = LoopConfig {
                    stage,
                    track: None,
                    hat_collection: hat,
                    prompt_file: None,
                    completion_promise: None,
                    knowledge_base: None,
                    mcp_servers: None,
                    execution_mode: None,
                    work_dir: Some(stage_dir),
                    block_id: None,
                };

                let spawn_ok = match self.orchestrator.spawn(loop_config).await {
                    Ok(_) => true,
                    Err(e) => {
                        self.log("error", &format!("Failed to spawn {:?}: {}", stage, e)).await;
                        let _ = self.engine.transition(EngineEvent::StageFailed {
                            stage,
                            error: format!("{e}"),
                        });
                        false
                    }
                };
                if spawn_ok {
                    self.log("info", &format!("Spawned Ralph for {:?}", stage)).await;
                    // Tail the Ralph log file to stream output to web UI
                    let log_path = ralph_dir.join("ralph-output.log");
                    let log_fwd = self.log_tx.clone();
                    tokio::spawn(async move {
                        use tokio::io::{AsyncBufReadExt, BufReader};
                        // Wait for file to exist
                        for _ in 0..50 {
                            if log_path.exists() { break; }
                            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        }
                        let file = match tokio::fs::File::open(&log_path).await {
                            Ok(f) => f,
                            Err(_) => return,
                        };
                        let reader = BufReader::new(file);
                        let mut lines = reader.lines();
                        loop {
                            match lines.next_line().await {
                                Ok(Some(line)) => {
                                    let clean = orchestrator::strip_ansi(&line);
                                    if !clean.trim().is_empty() && !clean.starts_with('[') {
                                        // Non-blocking: drop log lines if the buffer is
                                        // full rather than blocking on a slow consumer.
                                        let _ = log_fwd.try_send(LogLine {
                                            level: "info".to_string(),
                                            message: clean,
                                        });
                                    }
                                }
                                Ok(None) => {
                                    // EOF — wait and retry (file still being written)
                                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                }
                                Err(_) => break,
                            }
                        }
                    });
                }
            }
            Err(e) => {
                self.log("error", &format!("Stage dir setup failed for {:?}: {}", stage, e)).await;
            }
        }
    }

    /// Spawn a track's Ralph loop. On spawn/setup failure, returns the engine
    /// effects from marking the track Failed (so the caller's effect loop can drain
    /// them, e.g. advancing to code review) — returning them instead of calling
    /// process_effects recursively avoids an async recursion cycle.
    async fn spawn_track(&mut self, track_id: &str, feedback: Option<&str>) -> Vec<Effect> {
        self.last_progress = Some(std::time::Instant::now());
        let config = LoopyConfig::load_or_default(&self.project_root);
        let track_defs = crate::config::load_track_definitions(&self.project_root, &config);
        let (description, dep_tracks) = track_defs
            .get(track_id)
            .map(|d| (d.description.as_str(), d.dependencies.clone()))
            .unwrap_or(("", vec![]));

        match orchestrator::setup_track_dir(track_id, description, &dep_tracks, &config, &self.stages_dir) {
            Ok(stage_track_dir) => {
                // Use workspace dir if it exists (for Brazil workspace tracks)
                let ws_dir = self.project_root.join(".loopy/workspaces").join(track_id);
                let track_dir = if ws_dir.exists() {
                    // Copy ralph.yml and PROMPT.md to workspace
                    let _ = std::fs::copy(stage_track_dir.join("ralph.yml"), ws_dir.join("ralph.yml"));
                    let _ = std::fs::copy(stage_track_dir.join("PROMPT.md"), ws_dir.join("PROMPT.md"));
                    ws_dir
                } else {
                    stage_track_dir
                };
                if let Some(fb) = feedback {
                    let prompt_path = track_dir.join("PROMPT.md");
                    if let Ok(existing) = std::fs::read_to_string(&prompt_path) {
                        let _ = std::fs::write(
                            &prompt_path,
                            format!("{existing}\n\n## User Feedback\n\n{fb}"),
                        );
                    }
                }

                let ralph_dir = track_dir.join(".ralph");
                std::fs::create_dir_all(&ralph_dir).ok();
                if let Some(wh) = &mut self.watcher_handle {
                    let _ = wh.watch_path(&ralph_dir);
                }

                let loop_config = LoopConfig {
                    stage: StageId::OrbitalLanes,
                    track: Some(track_id.to_string()),
                    hat_collection: Some("builtin:code-assist".into()),
                    prompt_file: Some(PathBuf::from("PROMPT.md")),
                    completion_promise: Some(format!("track.{track_id}.complete")),
                    knowledge_base: None,
                    mcp_servers: None,
                    execution_mode: None,
                    work_dir: Some(track_dir),
                    block_id: None,
                };

                match self.orchestrator.spawn(loop_config).await {
                    Ok(_) => {
                        self.log("info", &format!("Spawned Ralph for track {}", track_id)).await;
                    }
                    Err(e) => {
                        // Must mark the track Failed — otherwise it stays Running,
                        // all_tracks_terminal() is never true, and the pipeline is
                        // stuck in RunningTracks forever.
                        self.log("error", &format!("Failed to spawn track {}: {}", track_id, e)).await;
                        return self.engine.transition(EngineEvent::TrackFailed {
                            track_id: track_id.to_string(),
                            error: format!("{e}"),
                        });
                    }
                }
            }
            Err(e) => {
                self.log("error", &format!("Track dir setup failed for {}: {}", track_id, e)).await;
                return self.engine.transition(EngineEvent::TrackFailed {
                    track_id: track_id.to_string(),
                    error: format!("{e}"),
                });
            }
        }
        Vec::new()
    }

    /// Draft a bare-bones TESTING-PLAN.md for the Test Flight stage. Deterministic
    /// (no LLM) — scaffolds deploy + functional-check sections from the idea and
    /// the track→package map so the user has something concrete to edit/approve.
    async fn generate_test_plan(&mut self) {
        let stage_dir = self.stages_dir.join(orchestrator::stage_dir_name(StageId::TestFlight));
        if let Err(e) = std::fs::create_dir_all(&stage_dir) {
            self.log("error", &format!("Failed to create test-flight dir: {e}")).await;
            return;
        }
        let plan_path = stage_dir.join("TESTING-PLAN.md");
        // Don't clobber an edited plan if one already exists (e.g. on resume).
        if plan_path.exists() {
            self.log("info", "TESTING-PLAN.md already exists — keeping it").await;
            return;
        }

        // Collect packages from tracks.json for the deploy/test scaffold.
        let mut pkg_lines = String::new();
        let tracks_json = self.stages_dir.join("plan/tracks.json");
        if let Ok(content) = std::fs::read_to_string(&tracks_json) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(tracks) = json.get("tracks").and_then(|t| t.as_array()) {
                    for t in tracks {
                        let id = t.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                        let pkgs: Vec<&str> = t.get("packages")
                            .and_then(|p| p.as_array())
                            .map(|a| a.iter().filter_map(|p| p.get("name").and_then(|n| n.as_str())).collect())
                            .unwrap_or_default();
                        pkg_lines.push_str(&format!("- **{id}**: {}\n", pkgs.join(", ")));
                    }
                }
            }
        }

        let idea = &self.engine.state.idea;
        let feedback_note = if self.engine.state.test_feedback_history.is_empty() {
            String::new()
        } else {
            format!(
                "\n> Re-running after earlier test feedback ({} round(s)). Prior notes:\n{}\n",
                self.engine.state.test_feedback_history.len(),
                self.engine.state.test_feedback_history.iter()
                    .map(|f| format!("> - {f}")).collect::<Vec<_>>().join("\n"),
            )
        };

        let draft = format!(
            "# Test Flight Plan\n\n\
            > **Draft** — edit this before approving. Loopy deploys to **beta** and \
            runs these checks; it does not touch production.\n{feedback_note}\n\
            ## What we built\n\n{idea}\n\n\
            ## Components changed (by track)\n\n{pkg_lines}\n\
            ## 1. Deploy to a test environment\n\n\
            Fill in the exact commands / pipeline / stack to deploy these changes to a \
            beta or test environment. Examples:\n\n\
            ```bash\n\
            # e.g. build + deploy the changed components\n\
            # <your build command>\n\
            # <your deploy/promote command>\n\
            ```\n\n\
            ## 2. Functional checks\n\n\
            List what to verify once deployed (endpoints, behaviors, acceptance criteria). \
            Be specific enough that someone could run them:\n\n\
            - [ ] \n- [ ] \n- [ ] \n\n\
            ## 3. How to report issues\n\n\
            Anything that fails here can be sent back to the tracks as feedback — they will \
            iterate on top of the existing work.\n"
        );

        match std::fs::write(&plan_path, draft) {
            Ok(_) => self.log("info", &format!("Drafted test plan: {}", plan_path.display())).await,
            Err(e) => self.log("error", &format!("Failed to write test plan: {e}")).await,
        }
    }

    /// Spawn the one-shot agent instance for Test Flight. It deploys to beta and
    /// runs the (user-approved) TESTING-PLAN.md as its prompt. Reuses the stage
    /// spawn machinery; completion is detected the same way as other stages.
    async fn spawn_test_flight(&mut self, feedback: Option<&str>) {
        let stage_dir = self.stages_dir.join(orchestrator::stage_dir_name(StageId::TestFlight));
        let plan_path = stage_dir.join("TESTING-PLAN.md");
        let plan = std::fs::read_to_string(&plan_path).unwrap_or_default();

        // Build the agent prompt from the approved plan + any prior test feedback.
        let mut prompt = format!(
            "# Test Flight\n\nYou are running the test plan below against a BETA/TEST \
            environment. Deploy as described, run the functional checks, and report \
            results clearly. Do NOT deploy to production.\n\n{plan}\n"
        );
        if let Some(fb) = feedback {
            prompt.push_str(&format!("\n## Additional notes\n\n{fb}\n"));
        }

        // spawn_stage sets up the dir + PROMPT; pass the plan-derived prompt as
        // extra context so the agent sees the approved plan.
        self.spawn_stage(StageId::TestFlight, Some(&prompt)).await;
    }

    /// Spawn a generic Loopy [`StageBlock`](crate::pipeline::StageBlock) agent
    /// for non-POC pipeline execution. Composes the block's ralph.yml + PROMPT.md
    /// (from its kind metadata) and spawns via a block_id-labeled LoopConfig.
    /// Returns the work dir on success. POC stages do NOT use this — it's the
    /// generic executor's spawn primitive.
    pub async fn spawn_block(
        &mut self,
        block: &crate::pipeline::StageBlock,
        task: &str,
        prior_stage_ids: &[String],
        feedback: Option<&str>,
    ) -> Option<std::path::PathBuf> {
        self.last_progress = Some(std::time::Instant::now());
        let config = LoopyConfig::load_or_default(&self.project_root);
        // Work dir: .loopy/stages/<block_id>/ (parallel to POC stage dirs).
        let work_dir = self.stages_dir.join(&block.id);
        if let Err(e) = std::fs::create_dir_all(&work_dir) {
            self.log("error", &format!("Failed to create block dir {}: {e}", block.id)).await;
            return None;
        }

        // Write agent config + prompt from the block's metadata.
        let yml = orchestrator::ralph_yml_for_block(block, &config, &self.project_root);
        let prompt = orchestrator::prompt_for_block(block, task, prior_stage_ids, feedback);
        if std::fs::write(work_dir.join("ralph.yml"), &yml).is_err()
            || std::fs::write(work_dir.join("PROMPT.md"), &prompt).is_err()
        {
            self.log("error", &format!("Failed to write block config for {}", block.id)).await;
            return None;
        }

        let ralph_dir = work_dir.join(".ralph");
        std::fs::create_dir_all(&ralph_dir).ok();
        let _ = std::fs::remove_file(ralph_dir.join("loop.lock"));
        if let Some(wh) = &mut self.watcher_handle {
            let _ = wh.watch_path(&ralph_dir);
        }

        let hat = block.kind.default_hat().map(|raw| {
            if raw.starts_with("builtin:") {
                raw.to_string()
            } else {
                self.project_root.join(raw).display().to_string()
            }
        });

        let loop_config = LoopConfig {
            // `stage` is unused for labeling when block_id is set; OrbitalLanes is
            // a harmless placeholder (most permissive setup).
            stage: StageId::OrbitalLanes,
            track: None,
            hat_collection: hat,
            prompt_file: Some(std::path::PathBuf::from("PROMPT.md")),
            completion_promise: Some(block.kind.completion_promise().to_string()),
            knowledge_base: None,
            mcp_servers: None,
            execution_mode: None,
            work_dir: Some(work_dir.clone()),
            block_id: Some(block.id.clone()),
        };

        // Test guard: skip the real process spawn (driver state already advanced).
        if self.no_spawn {
            return Some(work_dir);
        }

        match self.orchestrator.spawn(loop_config).await {
            Ok(_) => {
                self.log("info", &format!("Spawned block '{}' ({})", block.id, block.label)).await;
                // Tail the block's Ralph log → log_tx so the UI streams its output
                // (same as POC stages; without this, linear runs show no logs).
                let log_path = ralph_dir.join("ralph-output.log");
                let log_fwd = self.log_tx.clone();
                tokio::spawn(async move {
                    use tokio::io::{AsyncBufReadExt, BufReader};
                    for _ in 0..50 {
                        if log_path.exists() { break; }
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                    let file = match tokio::fs::File::open(&log_path).await {
                        Ok(f) => f,
                        Err(_) => return,
                    };
                    let reader = BufReader::new(file);
                    let mut lines = reader.lines();
                    loop {
                        match lines.next_line().await {
                            Ok(Some(line)) => {
                                let clean = orchestrator::strip_ansi(&line);
                                if !clean.trim().is_empty() && !clean.starts_with('[') {
                                    let _ = log_fwd.try_send(LogLine {
                                        level: "info".to_string(),
                                        message: clean,
                                    });
                                }
                            }
                            Ok(None) => tokio::time::sleep(std::time::Duration::from_millis(500)).await,
                            Err(_) => break,
                        }
                    }
                });
                Some(work_dir)
            }
            Err(e) => {
                self.log("error", &format!("Failed to spawn block {}: {e}", block.id)).await;
                None
            }
        }
    }

    /// Whether a stage has produced its expected output artifact yet. Used to
    /// reject spurious/duplicate completion events that would skip a stage's work.
    fn stage_has_output(&self, stage: StageId) -> bool {
        let dir = self.stages_dir.join(orchestrator::stage_dir_name(stage));
        let exists = |rels: &[&str]| rels.iter().any(|r| dir.join(r).exists());
        // NOTE: the agent scratchpad (output/scratchpad.md, .ralph/agent/scratchpad.md)
        // is created for EVERY run before any real work, so it must NOT count as
        // output — accepting it let a Plan that never wrote tracks.json report
        // "complete", so tracks never launched. Require a real artifact instead.
        match stage {
            StageId::Scan => exists(&[
                "scan-report.md", "report.md", "output/scan-report.md",
                "environment.json", "output/environment.json",
            ]),
            StageId::Plan => exists(&[
                "tracks.json", "output/tracks.json", "PLAN.md", "output/PLAN.md",
            ]),
            // Land has no strict required artifact; accept its completion.
            _ => true,
        }
    }

    pub fn load_tracks_from_plan(&mut self, plan_dir: &std::path::Path) {
        // Try tracks.json first
        let tracks_path = plan_dir.join("tracks.json");
        log::info!("Looking for tracks at: {} (exists: {})", tracks_path.display(), tracks_path.exists());
        let tracks_json = std::fs::read_to_string(&tracks_path)
            .or_else(|_| std::fs::read_to_string(plan_dir.join("output/tracks.json")));

        if let Ok(contents) = tracks_json {
            if let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&contents) {
                let tracks_arr = manifest.get("tracks").and_then(|t| t.as_array())
                    .or_else(|| manifest.as_array());

                if let Some(arr) = tracks_arr {
                    let track_states: Vec<crate::models::TrackState> = arr.iter().filter_map(|t| {
                        let id = t.get("id").and_then(|v| v.as_str())?.to_string();
                        Some(crate::models::TrackState {
                            id: id.clone(),
                            name: t.get("name").and_then(|v| v.as_str()).unwrap_or(&id).to_string(),
                            status: crate::models::TrackStatus::Pending,
                            loop_id: None,
                            loop_pid: None,
                            current_sub_stage: None,
                            depends_on: vec![],
                            blocking_artifact: None,
                            consumed_versions: vec![],
                            run_count: 0,
                            medium: crate::models::TrackMedium::Brazil,
                            review_status: crate::models::ReviewStatus::PendingReview,
                            cr_url: None,
                        })
                    }).collect();

                    if !track_states.is_empty() {
                        log::info!("Loaded {} tracks from plan output", track_states.len());
                        self.engine.state.tracks = Some(track_states);
                    }
                }
            }
        }
    }

    pub fn save_checkpoint(&self) {
        let state = self.to_v1_state();
        let checkpoint_path = self.project_root.join(".loopy/state.json");
        let _ = crate::checkpoint::save(&checkpoint_path, &state);
        self.save_linear_run();
        self.save_custom_pipeline();
    }

    /// Path of the linear-run sidecar checkpoint (generic pipeline progress).
    fn linear_run_path(project_root: &std::path::Path) -> std::path::PathBuf {
        project_root.join(".loopy/linear-run.json")
    }

    fn custom_pipeline_path(project_root: &std::path::Path) -> std::path::PathBuf {
        project_root.join(".loopy/custom-pipeline.json")
    }

    /// Persist the custom pipeline definition so a custom (linear) run survives a
    /// restart. The main checkpoint (PipelineState) doesn't carry it, so without
    /// this a restart loses the block list and reverts to the POC pipeline.
    fn save_custom_pipeline(&self) {
        let path = Self::custom_pipeline_path(&self.project_root);
        match &self.engine.state.custom_pipeline {
            Some(cp) => {
                if let Ok(json) = serde_json::to_string_pretty(cp) {
                    let _ = std::fs::write(&path, json);
                }
            }
            None => { let _ = std::fs::remove_file(&path); }
        }
    }

    /// Load a persisted custom pipeline definition, if present.
    fn load_custom_pipeline(project_root: &std::path::Path) -> Option<crate::pipeline::PipelineTemplate> {
        let content = std::fs::read_to_string(Self::custom_pipeline_path(project_root)).ok()?;
        serde_json::from_str(&content).ok()
    }

    /// Persist the LinearRun so a non-POC pipeline survives a server restart
    /// (otherwise resume would restart it from block 1). No-op for POC runs.
    fn save_linear_run(&self) {
        let path = Self::linear_run_path(&self.project_root);
        match &self.linear_run {
            Some(run) => {
                if let Ok(json) = serde_json::to_string_pretty(run) {
                    let _ = std::fs::write(&path, json);
                }
            }
            // POC run: ensure no stale sidecar lingers.
            None => { let _ = std::fs::remove_file(&path); }
        }
    }

    /// Load a persisted LinearRun for a project, if one exists and parses.
    fn load_linear_run(project_root: &std::path::Path) -> Option<crate::pipeline::LinearRun> {
        let content = std::fs::read_to_string(Self::linear_run_path(project_root)).ok()?;
        serde_json::from_str(&content).ok()
    }

    /// Finalize an aborted linear run: mark it done and remove the sidecar so a
    /// restart does NOT resume the aborted pipeline. (KillAll already reaped the
    /// processes.) No-op for POC runs.
    pub fn abort_linear(&mut self) {
        if let Some(run) = self.linear_run.as_mut() {
            run.done = true;
            run.current = None;
        }
        self.paused_block = None;
        let _ = std::fs::remove_file(Self::linear_run_path(&self.project_root));
    }

    fn to_v1_state(&self) -> crate::models::PipelineState {
        crate::models::PipelineState {
            version: 2,
            idea_text: self.engine.state.idea.clone(),
            stages: self.engine.state.stages.clone(),
            tracks: self.engine.state.tracks.clone().map(|tracks| {
                tracks.into_iter().map(|t| crate::models::TrackState {
                    id: t.id,
                    name: t.name,
                    status: t.status,
                    loop_id: t.loop_id,
                    loop_pid: t.loop_pid,
                    current_sub_stage: t.current_sub_stage,
                    depends_on: vec![],
                    blocking_artifact: None,
                    consumed_versions: vec![],
                    run_count: 0,
                    medium: crate::models::TrackMedium::Brazil,
                    review_status: crate::models::ReviewStatus::PendingReview,
                    cr_url: None,
                }).collect()
            }),
            created_at: self.engine.state.created_at,
            updated_at: self.engine.state.updated_at,
            execution_mode: crate::models::ExecutionMode::Fast,
            artifact_registry: crate::models::ArtifactRegistry::default(),
            active_conflicts: vec![],
            awaiting_approval: None,
        }
    }

    /// Is a Ralph loop actually alive for this track? Checks the track's
    /// `.ralph/loop.lock` pid against /proc. Disk-based (not via the in-process
    /// orchestrator) so it works on resume, when the loops were spawned by a
    /// previous server process. A stale lock whose pid is gone reads as dead.
    pub fn track_loop_alive(&self, track_id: &str) -> bool {
        let lock = self.track_work_dir(track_id).join(".ralph/loop.lock");
        let Ok(content) = std::fs::read_to_string(&lock) else {
            return false;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) else {
            return false;
        };
        let Some(pid) = json.get("pid").and_then(|p| p.as_u64()) else {
            return false;
        };
        pid_is_live(pid)
    }

    /// True only when we can CONFIRM a track's Ralph is dead: its `loop.lock`
    /// exists (so the loop actually started and recorded a PID) but that PID is no
    /// longer live. Distinct from `!track_loop_alive` (which is also true in the
    /// spawn window before the lock is written) — used by reconcile so a
    /// just-spawned track isn't prematurely failed.
    fn track_loop_confirmed_dead(&self, track_id: &str) -> bool {
        let lock = self.track_work_dir(track_id).join(".ralph/loop.lock");
        let Ok(content) = std::fs::read_to_string(&lock) else {
            return false; // no lock yet → could be starting; not confirmed dead
        };
        let Some(pid) = serde_json::from_str::<serde_json::Value>(&content)
            .ok()
            .and_then(|j| j.get("pid").and_then(|p| p.as_u64()))
        else {
            return false;
        };
        !pid_is_live(pid)
    }

    /// Resolve the on-disk work directory for a track (workspace dir if present,
    /// otherwise the stage track dir).
    pub fn track_work_dir(&self, track_id: &str) -> PathBuf {
        let ws_dir = self.project_root.join(".loopy/workspaces").join(track_id);
        if ws_dir.exists() {
            ws_dir
        } else {
            self.stages_dir.join("orbital-lanes").join(track_id)
        }
    }

    /// Compute live progress for every running track via the deterministic
    /// track inspector. Returns (track_id, tasks_done, tasks_total, activity).
    pub fn track_progress(&self) -> Vec<(String, u32, u32, String)> {
        let Some(tracks) = &self.engine.state.tracks else {
            return vec![];
        };
        tracks
            .iter()
            .filter(|t| t.status == crate::models::TrackStatus::Running)
            .map(|track| {
                let insp = crate::track_inspector::inspect(&self.track_work_dir(&track.id));
                (track.id.clone(), insp.tasks_done, insp.tasks_total, insp.activity)
            })
            .collect()
    }

    /// Detect tracks whose Ralph loop has reached a terminal state on disk and
    /// fire the matching engine event. Ralph signals completion with a generic
    /// "Primary loop landed successfully" log line (NOT a `track.<id>.complete`
    /// topic), so the file-event aggregator never sees a track completion — we
    /// must poll the inspector here, otherwise the engine wedges in
    /// `running_tracks` forever and Land never fires.
    ///
    /// Returns true if any track transitioned (i.e. an engine event fired).
    pub async fn reconcile_tracks(&mut self) -> bool {
        use crate::models::TrackStatus;
        use crate::track_inspector::Disposition;

        if self.engine.state.phase != Phase::RunningTracks {
            return false;
        }
        let Some(tracks) = &self.engine.state.tracks else {
            return false;
        };

        // Snapshot terminal transitions first (avoid borrowing self mutably
        // while iterating its state).
        let mut events: Vec<EngineEvent> = Vec::new();
        for track in tracks {
            if track.status != TrackStatus::Running {
                continue;
            }
            let insp = crate::track_inspector::inspect(&self.track_work_dir(&track.id));
            match insp.disposition {
                Disposition::Completed => {
                    events.push(EngineEvent::TrackCompleted { track_id: track.id.clone() });
                }
                Disposition::Failed => {
                    events.push(EngineEvent::TrackFailed {
                        track_id: track.id.clone(),
                        error: "Ralph loop stopped without landing (max iterations or error)".into(),
                    });
                }
                Disposition::Active => {
                    // "Active" only means the inspector saw no terminal marker. If
                    // the track's Ralph process is actually dead (crashed / killed
                    // without writing a landed/max-iterations marker), it would
                    // otherwise stay Running forever and wedge RunningTracks. Treat
                    // a dead loop with no terminal marker as Failed.
                    if self.track_loop_confirmed_dead(&track.id) {
                        events.push(EngineEvent::TrackFailed {
                            track_id: track.id.clone(),
                            error: "Ralph process is no longer running and produced no completion marker".into(),
                        });
                    }
                }
            }
        }

        if events.is_empty() {
            return false;
        }
        for event in events {
            match &event {
                EngineEvent::TrackCompleted { track_id } => {
                    self.log("info", &format!("Track {track_id} completed (landed)")).await;
                }
                EngineEvent::TrackFailed { track_id, .. } => {
                    self.log("warn", &format!("Track {track_id} stopped without landing")).await;
                }
                _ => {}
            }
            let effects = self.engine.transition(event);
            self.process_effects(effects).await;
        }
        true
    }

    async fn log(&self, level: &str, message: &str) {
        // Non-blocking: logs are best-effort. If the channel is full (slow/absent
        // WebSocket consumer), DROP the line rather than blocking the runner — a
        // full log buffer must never freeze the pipeline.
        let _ = self.log_tx.try_send(LogLine {
            level: level.to_string(),
            message: message.to_string(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pid_is_live_true_for_self() {
        let me = std::process::id() as u64;
        assert!(pid_is_live(me));
    }

    #[test]
    fn pid_is_live_false_for_nonexistent() {
        // PID 0 has no /proc entry; an absurd pid is also safe.
        assert!(!pid_is_live(0));
        assert!(!pid_is_live(4_000_000_000));
    }

    /// Build a runner for the coding-task pipeline in a temp project dir.
    async fn coding_task_runner() -> (EngineRunner, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().unwrap();
        let mut engine = Engine::new("fix the auth bug", "t");
        engine.state.template_id = "coding-task".into();
        let (tx, _rx) = mpsc::channel(64);
        let mut runner = EngineRunner::new(engine, dir.path().to_path_buf(), tx).await.unwrap();
        runner.set_no_spawn(true); // never launch a real ralph in tests
        (runner, dir)
    }

    #[tokio::test]
    async fn arbitrary_custom_pipeline_executes_through_the_driver() {
        // The flexibility requirement: a totally custom block arrangement (one that
        // is not any built-in template) must run through the linear driver. Here:
        // scan → design → team_review(checkpoint) → verify → submit.
        let dir = tempfile::TempDir::new().unwrap();
        let mut engine = Engine::new("custom flow", "c");
        engine.state.custom_pipeline = Some(crate::pipeline::PipelineTemplate::from_blocks(&[
            ("scan".into(), "scan".into(), None),
            ("design".into(), "design".into(), None),
            ("review".into(), "team_review".into(), None),
            ("verify".into(), "verify".into(), None),
            ("ship".into(), "submit".into(), None),
        ]));
        let (tx, _rx) = mpsc::channel(64);
        let mut r = EngineRunner::new(engine, dir.path().to_path_buf(), tx).await.unwrap();
        r.set_no_spawn(true);
        assert!(r.is_linear(), "custom pipeline must use the linear driver");

        assert_eq!(r.linear_snapshot().unwrap().2.as_deref(), Some("scan"));
        r.drive_linear().await; // scan → design
        assert_eq!(r.linear_snapshot().unwrap().2.as_deref(), Some("design"));
        r.drive_linear().await; // design → team_review (has default checkpoint) → pause
        assert_eq!(r.paused_block().map(|b| b.id.as_str()), Some("review"));
        r.approve_linear().await; // resume team_review
        r.drive_linear().await; // team_review → verify
        assert_eq!(r.linear_snapshot().unwrap().2.as_deref(), Some("verify"));
        r.drive_linear().await; // verify → ship
        assert_eq!(r.linear_snapshot().unwrap().2.as_deref(), Some("ship"));
        r.drive_linear().await; // ship → done
        assert!(r.linear_snapshot().unwrap().5);
    }

    #[tokio::test]
    async fn linear_runner_advances_pauses_and_resumes() {
        // Validates the LIVE driver path the poll loop uses (drive_linear mutates
        // the LinearRun/paused state regardless of whether the agent spawn — which
        // needs the agent — succeeds). This is the strongest end-to-end check possible
        // without an agent-authed environment.
        let (mut r, _dir) = coding_task_runner().await;
        assert!(r.is_linear());

        // Starts on implement.
        let (_, _, current, _, _, _) = r.linear_snapshot().unwrap();
        assert_eq!(current.as_deref(), Some("implement"));

        // implement completes → beta_test (no checkpoint).
        r.drive_linear().await;
        assert_eq!(r.linear_snapshot().unwrap().2.as_deref(), Some("beta_test"));
        assert!(r.paused_block().is_none());

        // beta_test completes → adversarial_review HAS a checkpoint → pause.
        r.drive_linear().await;
        assert_eq!(r.paused_block().map(|b| b.id.as_str()), Some("adversarial_review"));

        // Approve the checkpoint → resumes (un-pauses), still on adversarial_review.
        r.approve_linear().await;
        assert!(r.paused_block().is_none());
        assert_eq!(r.linear_snapshot().unwrap().2.as_deref(), Some("adversarial_review"));

        // adversarial_review completes → submit_cr (no checkpoint).
        r.drive_linear().await;
        assert_eq!(r.linear_snapshot().unwrap().2.as_deref(), Some("submit_cr"));

        // submit_cr completes → done.
        r.drive_linear().await;
        assert!(r.linear_snapshot().unwrap().5); // done == true
    }

    #[test]
    fn event_attributed_by_directory_not_ralph_filename() {
        // Regression for the #1 bug: linear pipelines hung forever because
        // completion was attributed by loop_id (Ralph's own filename id), which
        // never equals the block id. Attribution must be by the events file's
        // DIRECTORY (<stages_dir>/<block_id>/.ralph/events-*.jsonl).
        let stages = std::path::Path::new("/proj/.loopy/stages");
        let block_dir = stages.join("implement");

        // Ralph names its events file after its OWN internal id — a bare
        // timestamp that shares nothing with the block id. This MUST still match
        // because it lives under the block's dir.
        let ralph_named = block_dir.join(".ralph/events-primary-20260616-2254.jsonl");
        assert!(
            event_source_matches_block(&ralph_named, &block_dir),
            "event under the block dir must be attributed to it regardless of filename"
        );

        // A completion from a DIFFERENT block (e.g. an earlier scan) must NOT be
        // attributed to the current block — that was the "fell through" bug.
        let other = stages.join("scan/.ralph/events-primary-20260616-2250.jsonl");
        assert!(
            !event_source_matches_block(&other, &block_dir),
            "event from another block's dir must be ignored"
        );
    }

    #[tokio::test]
    async fn linear_reject_reroutes_to_implement() {
        let (mut r, _dir) = coding_task_runner().await;
        // Advance to the adversarial_review checkpoint.
        r.drive_linear().await; // → beta_test
        r.drive_linear().await; // → pause at adversarial_review
        assert_eq!(r.paused_block().map(|b| b.id.as_str()), Some("adversarial_review"));

        // Reject → reroutes to the Implement-kind block and clears the pause.
        r.reject_linear("tests fail on edge case".into()).await;
        assert!(r.paused_block().is_none());
        assert_eq!(r.linear_snapshot().unwrap().2.as_deref(), Some("implement"));
    }

    #[tokio::test]
    async fn linear_run_persists_and_resumes_across_restart() {
        let dir = tempfile::TempDir::new().unwrap();
        // First runner: advance partway, then it persists on each drive.
        {
            let mut engine = Engine::new("fix the auth bug", "t");
            engine.state.template_id = "coding-task".into();
            let (tx, _rx) = mpsc::channel(64);
            let mut r = EngineRunner::new(engine, dir.path().to_path_buf(), tx).await.unwrap();
            r.set_no_spawn(true);
            r.drive_linear().await; // implement → beta_test (persisted)
            assert_eq!(r.linear_snapshot().unwrap().2.as_deref(), Some("beta_test"));
        }
        // The sidecar exists on disk.
        assert!(dir.path().join(".loopy/linear-run.json").exists());

        // Second runner over the same dir: must restore at beta_test, NOT implement.
        {
            let mut engine = Engine::new("fix the auth bug", "t");
            engine.state.template_id = "coding-task".into();
            let (tx, _rx) = mpsc::channel(64);
            let r = EngineRunner::new(engine, dir.path().to_path_buf(), tx).await.unwrap();
            assert!(r.is_linear());
            assert_eq!(r.linear_snapshot().unwrap().2.as_deref(), Some("beta_test"),
                "resume must restore persisted progress, not restart from block 1");
            assert_eq!(r.linear_snapshot().unwrap().3, vec!["implement".to_string()]); // completed
        }
    }

    #[tokio::test]
    async fn abort_linear_clears_sidecar_so_restart_does_not_resume() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut engine = Engine::new("fix the auth bug", "t");
        engine.state.template_id = "coding-task".into();
        let (tx, _rx) = mpsc::channel(64);
        let mut r = EngineRunner::new(engine, dir.path().to_path_buf(), tx).await.unwrap();
        r.set_no_spawn(true);
        r.drive_linear().await; // advances + persists the sidecar
        assert!(dir.path().join(".loopy/linear-run.json").exists());

        r.abort_linear();
        assert!(!dir.path().join(".loopy/linear-run.json").exists(),
            "abort must remove the sidecar so a restart doesn't resume the aborted run");

        // A fresh runner over the same dir starts clean (block 1), not resumed.
        let mut e2 = Engine::new("fix the auth bug", "t");
        e2.state.template_id = "coding-task".into();
        let (tx2, _rx2) = mpsc::channel(64);
        let r2 = EngineRunner::new(e2, dir.path().to_path_buf(), tx2).await.unwrap();
        assert_eq!(r2.linear_snapshot().unwrap().2.as_deref(), Some("implement"));
    }

    #[tokio::test]
    async fn poc_runner_is_not_linear() {
        // The POC pipeline must NOT use the linear path.
        let dir = tempfile::TempDir::new().unwrap();
        let engine = Engine::new("build a poc", "p"); // defaults to POC template
        let (tx, _rx) = mpsc::channel(64);
        let r = EngineRunner::new(engine, dir.path().to_path_buf(), tx).await.unwrap();
        assert!(!r.is_linear());
        assert!(r.linear_snapshot().is_none());
    }
}
