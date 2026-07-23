use crate::models::{StageId, StageState, StageStatus, TrackState, TrackStatus};
use chrono::Utc;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Initializing,
    Scanning,
    Planning,
    AwaitingPlanReview,
    SettingUpWorkspaces,
    RunningTracks,
    AwaitingCodeReview,
    // Test Flight (opt-in beta test) — sits between Flight Check and Land.
    AwaitingTestPlan,    // user edits/approves the draft TESTING-PLAN.md
    TestFlying,          // the agent executes deploy-to-beta + E2E
    AwaitingTestReview,  // user reviews results → accept (land) or reject (back to tracks)
    Landing,
    Complete,
    Failed,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EngineEvent {
    Started { idea: String },
    ScanComplete,
    PlanComplete,
    UserApprovedPlan,
    UserRejectedPlan { feedback: String },
    WorkspaceSetupComplete,
    WorkspaceSetupFailed { error: String },
    TrackCompleted { track_id: String },
    TrackFailed { track_id: String, error: String },
    AllTracksComplete,
    /// Flight Check approval. `run_test_flight` opts into the Test Flight stage
    /// instead of going straight to Land.
    UserApprovedCode { run_test_flight: bool },
    UserRejectedCode { feedback: String },
    // --- Test Flight ---
    UserApprovedTestPlan,                  // approved the (possibly edited) TESTING-PLAN.md
    TestFlightComplete,                    // agent finished executing the test plan
    TestFlightFailed { error: String },    // agent errored out
    UserAcceptedTest,                      // results look good → Land
    UserRejectedTest { feedback: String }, // issues found → back to Tracks (additive)
    LandComplete,
    StageFailed { stage: StageId, error: String },
    Abort,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    SpawnScan,
    SpawnPlan,
    SpawnPlanWithFeedback { feedback: String, previous_plan: Option<String> },
    SetupWorkspaces { tracks: Vec<String> },
    SpawnTrack { track_id: String },
    SpawnTrackWithFeedback { track_id: String, feedback: String },
    SpawnLand,
    SaveCheckpoint,
    NotifyPhaseChange { phase: Phase },
    KillAll,
    PipelineDone,
    CreateCRs,
    // --- Test Flight ---
    GenerateTestPlan,                          // draft TESTING-PLAN.md from idea/plan/diff
    SpawnTestFlight { feedback: Option<String> }, // one-shot agent: deploy beta + E2E
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineState {
    pub phase: Phase,
    pub idea: String,
    pub project_name: String,
    pub stages: Vec<StageState>,
    pub tracks: Option<Vec<TrackState>>,
    pub plan_feedback_history: Vec<String>,
    pub code_feedback_history: Vec<String>,
    pub plan_iteration: u32,
    pub code_iteration: u32,
    #[serde(default)]
    pub test_feedback_history: Vec<String>,
    #[serde(default)]
    pub test_iteration: u32,
    /// Which Loopy pipeline template this run follows. Defaults to the POC
    /// pipeline ("poc-from-design-doc") for back-compat with existing runs and
    /// checkpoints. Recorded now so the engine can later drive transitions from
    /// the template; today it's informational and surfaced via the API.
    #[serde(default = "default_template_id")]
    pub template_id: String,
    /// A user-edited custom block list (the corrected Loopy flow: planner proposes
    /// → user adds/removes/reorders → run this). When set, it overrides
    /// `template_id` for execution. `None` = run the named template as-is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_pipeline: Option<crate::pipeline::PipelineTemplate>,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
}

fn default_template_id() -> String {
    "poc-from-design-doc".to_string()
}

impl EngineState {
    pub fn new(idea: &str, project_name: &str) -> Self {
        let now = Utc::now();
        Self {
            phase: Phase::Initializing,
            idea: idea.to_string(),
            project_name: project_name.to_string(),
            stages: vec![
                StageState {
                    id: StageId::Idea,
                    status: StageStatus::Complete,
                    loop_id: None,
                    loop_pid: None,
                    started_at: Some(now),
                    completed_at: Some(now),
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
                    id: StageId::OrbitalLanes,
                    status: StageStatus::Pending,
                    loop_id: None,
                    loop_pid: None,
                    started_at: None,
                    completed_at: None,
                    error: None,
                },
                StageState {
                    id: StageId::TestFlight,
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
            plan_feedback_history: vec![],
            code_feedback_history: vec![],
            plan_iteration: 0,
            code_iteration: 0,
            test_feedback_history: vec![],
            test_iteration: 0,
            template_id: default_template_id(),
            custom_pipeline: None,
            created_at: now,
            updated_at: now,
        }
    }
}

pub struct Engine {
    pub state: EngineState,
}

impl Engine {
    pub fn new(idea: &str, project_name: &str) -> Self {
        Self {
            state: EngineState::new(idea, project_name),
        }
    }

    /// The pipeline template this run follows (falls back to the POC pipeline if
    /// the recorded id is unknown — keeps old/forward checkpoints working).
    pub fn template(&self) -> crate::pipeline::PipelineTemplate {
        // A user-edited custom block list takes precedence over the named template.
        if let Some(custom) = &self.state.custom_pipeline {
            return custom.clone();
        }
        crate::pipeline::template_by_id(&self.state.template_id)
            .unwrap_or_else(crate::pipeline::poc_from_design_doc)
    }

    /// Whether this run executes via the generic linear driver (a composed/non-POC
    /// pipeline) rather than the POC `Phase` state machine. Must match the runner's
    /// `EngineRunner::is_linear` condition so HTTP handlers route actions correctly.
    pub fn is_linear(&self) -> bool {
        let t = self.template();
        t.executable && t.id != "poc-from-design-doc"
    }

    pub fn from_state(mut state: EngineState) -> Self {
        // Migrate checkpoints written before the Test Flight stage existed: ensure
        // a test_flight stage row is present (inserted just before Land) so the UI
        // and stage-status logic have it. Idempotent.
        if !state.stages.iter().any(|s| s.id == StageId::TestFlight) {
            let row = StageState {
                id: StageId::TestFlight,
                status: StageStatus::Pending,
                loop_id: None,
                loop_pid: None,
                started_at: None,
                completed_at: None,
                error: None,
            };
            match state.stages.iter().position(|s| s.id == StageId::Land) {
                Some(idx) => state.stages.insert(idx, row),
                None => state.stages.push(row),
            }
        }
        Self { state }
    }

    pub fn transition(&mut self, event: EngineEvent) -> Vec<Effect> {
        self.state.updated_at = Utc::now();
        match event {
            EngineEvent::Started { idea } => {
                self.state.idea = idea;
                self.state.phase = Phase::Scanning;
                self.set_stage_status(StageId::Scan, StageStatus::Running);
                vec![
                    Effect::SpawnScan,
                    Effect::SaveCheckpoint,
                    Effect::NotifyPhaseChange { phase: Phase::Scanning },
                ]
            }

            EngineEvent::ScanComplete => {
                if self.state.phase != Phase::Scanning {
                    return vec![];
                }
                self.set_stage_status(StageId::Scan, StageStatus::Complete);
                self.state.phase = Phase::Planning;
                self.set_stage_status(StageId::Plan, StageStatus::Running);
                vec![
                    Effect::SpawnPlan,
                    Effect::SaveCheckpoint,
                    Effect::NotifyPhaseChange { phase: Phase::Planning },
                ]
            }

            EngineEvent::PlanComplete => {
                if self.state.phase != Phase::Planning {
                    return vec![];
                }
                self.set_stage_status(StageId::Plan, StageStatus::Complete);
                self.state.phase = Phase::AwaitingPlanReview;
                self.state.plan_iteration += 1;
                vec![
                    Effect::SaveCheckpoint,
                    Effect::NotifyPhaseChange { phase: Phase::AwaitingPlanReview },
                ]
            }

            EngineEvent::UserApprovedPlan => {
                if self.state.phase != Phase::AwaitingPlanReview {
                    return vec![];
                }
                self.state.phase = Phase::SettingUpWorkspaces;
                self.set_stage_status(StageId::OrbitalLanes, StageStatus::Running);
                let track_ids = self.state.tracks.as_ref()
                    .map(|t| t.iter().map(|t| t.id.clone()).collect())
                    .unwrap_or_default();
                vec![
                    Effect::SetupWorkspaces { tracks: track_ids },
                    Effect::SaveCheckpoint,
                    Effect::NotifyPhaseChange { phase: Phase::SettingUpWorkspaces },
                ]
            }

            EngineEvent::UserRejectedPlan { feedback } => {
                if self.state.phase != Phase::AwaitingPlanReview {
                    return vec![];
                }
                self.state.plan_feedback_history.push(feedback.clone());
                self.state.phase = Phase::Planning;
                self.set_stage_status(StageId::Plan, StageStatus::Running);
                let previous_plan = None; // TODO: read from filesystem
                vec![
                    Effect::SpawnPlanWithFeedback { feedback, previous_plan },
                    Effect::SaveCheckpoint,
                    Effect::NotifyPhaseChange { phase: Phase::Planning },
                ]
            }

            EngineEvent::WorkspaceSetupComplete => {
                if self.state.phase != Phase::SettingUpWorkspaces {
                    return vec![];
                }
                self.state.phase = Phase::RunningTracks;
                let mut effects = vec![
                    Effect::NotifyPhaseChange { phase: Phase::RunningTracks },
                ];
                if let Some(tracks) = &mut self.state.tracks {
                    for t in tracks.iter_mut() {
                        if t.status == TrackStatus::Pending {
                            t.status = TrackStatus::Running;
                            effects.push(Effect::SpawnTrack { track_id: t.id.clone() });
                        }
                    }
                }
                effects.push(Effect::SaveCheckpoint);
                if self.all_tracks_terminal() {
                    effects.extend(self.enter_code_review());
                }
                effects
            }

            EngineEvent::WorkspaceSetupFailed { error: _ } => {
                if self.state.phase != Phase::SettingUpWorkspaces {
                    return vec![];
                }
                self.state.phase = Phase::Failed;
                self.set_stage_status(StageId::OrbitalLanes, StageStatus::Failed);
                vec![
                    Effect::SaveCheckpoint,
                    Effect::NotifyPhaseChange { phase: Phase::Failed },
                ]
            }

            EngineEvent::TrackCompleted { track_id } => {
                if self.state.phase != Phase::RunningTracks {
                    return vec![];
                }
                if let Some(tracks) = &mut self.state.tracks {
                    if let Some(t) = tracks.iter_mut().find(|t| t.id == track_id) {
                        t.status = TrackStatus::Complete;
                    }
                }
                if self.all_tracks_terminal() {
                    return self.enter_code_review();
                }
                vec![Effect::SaveCheckpoint]
            }

            EngineEvent::TrackFailed { track_id, error } => {
                if self.state.phase != Phase::RunningTracks {
                    return vec![];
                }
                if let Some(tracks) = &mut self.state.tracks {
                    if let Some(t) = tracks.iter_mut().find(|t| t.id == track_id) {
                        t.status = TrackStatus::Failed;
                        t.blocking_artifact = Some(error);
                    }
                }
                if self.all_tracks_terminal() {
                    return self.enter_code_review();
                }
                vec![Effect::SaveCheckpoint]
            }

            EngineEvent::AllTracksComplete => {
                self.enter_code_review()
            }

            EngineEvent::UserApprovedCode { run_test_flight } => {
                if self.state.phase != Phase::AwaitingCodeReview {
                    return vec![];
                }
                self.set_stage_status(StageId::OrbitalLanes, StageStatus::Complete);
                if run_test_flight {
                    // Opt into Test Flight: draft a TESTING-PLAN.md and let the user
                    // edit/approve it before we deploy to beta.
                    self.state.phase = Phase::AwaitingTestPlan;
                    self.set_stage_status(StageId::TestFlight, StageStatus::Running);
                    vec![
                        Effect::GenerateTestPlan,
                        Effect::SaveCheckpoint,
                        Effect::NotifyPhaseChange { phase: Phase::AwaitingTestPlan },
                    ]
                } else {
                    self.state.phase = Phase::Landing;
                    self.set_stage_status(StageId::Land, StageStatus::Running);
                    vec![
                        Effect::CreateCRs,
                        Effect::SpawnLand,
                        Effect::SaveCheckpoint,
                        Effect::NotifyPhaseChange { phase: Phase::Landing },
                    ]
                }
            }

            // --- Test Flight transitions ---
            EngineEvent::UserApprovedTestPlan => {
                if self.state.phase != Phase::AwaitingTestPlan {
                    return vec![];
                }
                self.state.phase = Phase::TestFlying;
                self.state.test_iteration += 1;
                vec![
                    Effect::SpawnTestFlight { feedback: None },
                    Effect::SaveCheckpoint,
                    Effect::NotifyPhaseChange { phase: Phase::TestFlying },
                ]
            }

            EngineEvent::TestFlightComplete => {
                if self.state.phase != Phase::TestFlying {
                    return vec![];
                }
                self.state.phase = Phase::AwaitingTestReview;
                vec![
                    Effect::SaveCheckpoint,
                    Effect::NotifyPhaseChange { phase: Phase::AwaitingTestReview },
                ]
            }

            EngineEvent::TestFlightFailed { error } => {
                if self.state.phase != Phase::TestFlying {
                    return vec![];
                }
                // A failed test run is still a result the human should review
                // (logs may show what broke); surface it for the review decision.
                self.set_stage_status(StageId::TestFlight, StageStatus::Failed);
                if let Some(s) = self.state.stages.iter_mut().find(|s| s.id == StageId::TestFlight) {
                    s.error = Some(error);
                }
                self.state.phase = Phase::AwaitingTestReview;
                vec![
                    Effect::SaveCheckpoint,
                    Effect::NotifyPhaseChange { phase: Phase::AwaitingTestReview },
                ]
            }

            EngineEvent::UserAcceptedTest => {
                if self.state.phase != Phase::AwaitingTestReview {
                    return vec![];
                }
                self.set_stage_status(StageId::TestFlight, StageStatus::Complete);
                self.state.phase = Phase::Landing;
                self.set_stage_status(StageId::Land, StageStatus::Running);
                vec![
                    Effect::CreateCRs,
                    Effect::SpawnLand,
                    Effect::SaveCheckpoint,
                    Effect::NotifyPhaseChange { phase: Phase::Landing },
                ]
            }

            EngineEvent::UserRejectedTest { feedback } => {
                if self.state.phase != Phase::AwaitingTestReview {
                    return vec![];
                }
                // Issues found in beta → route back to Tracks additively, same as a
                // Flight Check rejection. Test Flight stage resets to pending.
                self.state.test_feedback_history.push(feedback.clone());
                self.set_stage_status(StageId::TestFlight, StageStatus::Pending);
                self.state.phase = Phase::RunningTracks;
                self.set_stage_status(StageId::OrbitalLanes, StageStatus::Running);
                let mut effects = vec![
                    Effect::NotifyPhaseChange { phase: Phase::RunningTracks },
                ];
                if let Some(tracks) = &mut self.state.tracks {
                    for t in tracks.iter_mut() {
                        t.status = TrackStatus::Running;
                        effects.push(Effect::SpawnTrackWithFeedback {
                            track_id: t.id.clone(),
                            feedback: feedback.clone(),
                        });
                    }
                }
                effects.push(Effect::SaveCheckpoint);
                effects
            }

            EngineEvent::UserRejectedCode { feedback } => {
                if self.state.phase != Phase::AwaitingCodeReview {
                    return vec![];
                }
                self.state.code_feedback_history.push(feedback.clone());
                self.state.phase = Phase::RunningTracks;
                self.set_stage_status(StageId::OrbitalLanes, StageStatus::Running);
                let mut effects = vec![
                    Effect::NotifyPhaseChange { phase: Phase::RunningTracks },
                ];
                if let Some(tracks) = &mut self.state.tracks {
                    for t in tracks.iter_mut() {
                        t.status = TrackStatus::Running;
                        effects.push(Effect::SpawnTrackWithFeedback {
                            track_id: t.id.clone(),
                            feedback: feedback.clone(),
                        });
                    }
                }
                effects.push(Effect::SaveCheckpoint);
                effects
            }

            EngineEvent::LandComplete => {
                if self.state.phase != Phase::Landing {
                    return vec![];
                }
                self.set_stage_status(StageId::Land, StageStatus::Complete);
                self.state.phase = Phase::Complete;
                vec![
                    Effect::PipelineDone,
                    Effect::SaveCheckpoint,
                    Effect::NotifyPhaseChange { phase: Phase::Complete },
                ]
            }

            EngineEvent::StageFailed { stage, error } => {
                self.set_stage_status(stage, StageStatus::Failed);
                if let Some(s) = self.state.stages.iter_mut().find(|s| s.id == stage) {
                    s.error = Some(error);
                }
                self.state.phase = Phase::Failed;
                vec![
                    Effect::KillAll,
                    Effect::SaveCheckpoint,
                    Effect::NotifyPhaseChange { phase: Phase::Failed },
                ]
            }

            EngineEvent::Abort => {
                self.state.phase = Phase::Failed;
                vec![
                    Effect::KillAll,
                    Effect::SaveCheckpoint,
                    Effect::NotifyPhaseChange { phase: Phase::Failed },
                ]
            }
        }
    }

    fn enter_code_review(&mut self) -> Vec<Effect> {
        // Reflect reality: if any track failed, the stage is not a clean success.
        // Marking it Complete regardless surfaced a failed build as a green stage.
        let any_failed = self
            .state
            .tracks
            .as_ref()
            .is_some_and(|ts| ts.iter().any(|t| t.status == TrackStatus::Failed));
        let stage_status = if any_failed {
            StageStatus::Failed
        } else {
            StageStatus::Complete
        };
        self.set_stage_status(StageId::OrbitalLanes, stage_status);
        self.state.phase = Phase::AwaitingCodeReview;
        self.state.code_iteration += 1;
        vec![
            Effect::SaveCheckpoint,
            Effect::NotifyPhaseChange { phase: Phase::AwaitingCodeReview },
        ]
    }

    fn set_stage_status(&mut self, id: StageId, status: StageStatus) {
        if let Some(s) = self.state.stages.iter_mut().find(|s| s.id == id) {
            s.status = status;
            if status == StageStatus::Running {
                // Reset timing on (re)start — clear any stale completion time
                s.started_at = Some(Utc::now());
                s.completed_at = None;
            }
            if status == StageStatus::Complete {
                s.completed_at = Some(Utc::now());
            }
        }
    }

    fn all_tracks_terminal(&self) -> bool {
        match &self.state.tracks {
            None => false, // No tracks loaded = NOT done (tracks must be loaded first)
            Some(tracks) if tracks.is_empty() => true, // Explicitly empty = done
            Some(tracks) => tracks.iter().all(|t| matches!(t.status, TrackStatus::Complete | TrackStatus::Failed | TrackStatus::Skipped)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_engine_starts_in_initializing() {
        let engine = Engine::new("Add rate limiting", "my-project");
        assert_eq!(engine.state.phase, Phase::Initializing);
        assert_eq!(engine.state.idea, "Add rate limiting");
        // idea, scan, plan, orbital_lanes, test_flight, land
        assert_eq!(engine.state.stages.len(), 6);
    }

    #[test]
    fn started_transitions_to_scanning() {
        let mut engine = Engine::new("idea", "proj");
        let effects = engine.transition(EngineEvent::Started { idea: "idea".into() });
        assert_eq!(engine.state.phase, Phase::Scanning);
        assert!(effects.contains(&Effect::SpawnScan));
    }

    #[test]
    fn scan_complete_transitions_to_planning() {
        let mut engine = Engine::new("idea", "proj");
        engine.transition(EngineEvent::Started { idea: "idea".into() });
        let effects = engine.transition(EngineEvent::ScanComplete);
        assert_eq!(engine.state.phase, Phase::Planning);
        assert!(effects.contains(&Effect::SpawnPlan));
    }

    #[test]
    fn plan_complete_transitions_to_awaiting_review() {
        let mut engine = Engine::new("idea", "proj");
        engine.transition(EngineEvent::Started { idea: "idea".into() });
        engine.transition(EngineEvent::ScanComplete);
        let _ = engine.transition(EngineEvent::PlanComplete);
        assert_eq!(engine.state.phase, Phase::AwaitingPlanReview);
        assert_eq!(engine.state.plan_iteration, 1);
    }

    #[test]
    fn approve_plan_transitions_to_setup() {
        let mut engine = Engine::new("idea", "proj");
        engine.transition(EngineEvent::Started { idea: "idea".into() });
        engine.transition(EngineEvent::ScanComplete);
        engine.transition(EngineEvent::PlanComplete);
        engine.state.tracks = Some(vec![
            TrackState {
                id: "backend".into(),
                name: "Backend".into(),
                status: TrackStatus::Pending,
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
            },
        ]);
        let effects = engine.transition(EngineEvent::UserApprovedPlan);
        assert_eq!(engine.state.phase, Phase::SettingUpWorkspaces);
        assert!(effects.iter().any(|e| matches!(e, Effect::SetupWorkspaces { .. })));
    }

    #[test]
    fn reject_plan_cycles_back_to_planning() {
        let mut engine = Engine::new("idea", "proj");
        engine.transition(EngineEvent::Started { idea: "idea".into() });
        engine.transition(EngineEvent::ScanComplete);
        engine.transition(EngineEvent::PlanComplete);
        let effects = engine.transition(EngineEvent::UserRejectedPlan {
            feedback: "too many tracks, simplify".into(),
        });
        assert_eq!(engine.state.phase, Phase::Planning);
        assert_eq!(engine.state.plan_feedback_history.len(), 1);
        assert!(effects.iter().any(|e| matches!(e, Effect::SpawnPlanWithFeedback { .. })));
    }

    #[test]
    fn feedback_loop_can_cycle_multiple_times() {
        let mut engine = Engine::new("idea", "proj");
        engine.transition(EngineEvent::Started { idea: "idea".into() });
        engine.transition(EngineEvent::ScanComplete);

        // First plan attempt
        engine.transition(EngineEvent::PlanComplete);
        assert_eq!(engine.state.plan_iteration, 1);
        engine.transition(EngineEvent::UserRejectedPlan { feedback: "nope".into() });

        // Second plan attempt
        engine.transition(EngineEvent::PlanComplete);
        assert_eq!(engine.state.plan_iteration, 2);
        engine.transition(EngineEvent::UserRejectedPlan { feedback: "still nope".into() });

        // Third attempt — approve
        engine.transition(EngineEvent::PlanComplete);
        assert_eq!(engine.state.plan_iteration, 3);
        assert_eq!(engine.state.plan_feedback_history.len(), 2);
    }

    #[test]
    fn workspace_setup_complete_launches_tracks() {
        let mut engine = Engine::new("idea", "proj");
        engine.transition(EngineEvent::Started { idea: "idea".into() });
        engine.transition(EngineEvent::ScanComplete);
        engine.transition(EngineEvent::PlanComplete);
        engine.state.tracks = Some(vec![
            TrackState {
                id: "backend".into(),
                name: "Backend".into(),
                status: TrackStatus::Pending,
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
            },
        ]);
        engine.transition(EngineEvent::UserApprovedPlan);
        let effects = engine.transition(EngineEvent::WorkspaceSetupComplete);
        assert_eq!(engine.state.phase, Phase::RunningTracks);
        assert!(effects.iter().any(|e| matches!(e, Effect::SpawnTrack { .. })));
    }

    #[test]
    fn all_tracks_complete_enters_code_review() {
        let mut engine = Engine::new("idea", "proj");
        engine.transition(EngineEvent::Started { idea: "idea".into() });
        engine.transition(EngineEvent::ScanComplete);
        engine.transition(EngineEvent::PlanComplete);
        engine.state.tracks = Some(vec![
            TrackState {
                id: "backend".into(),
                name: "Backend".into(),
                status: TrackStatus::Pending,
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
            },
        ]);
        engine.transition(EngineEvent::UserApprovedPlan);
        engine.transition(EngineEvent::WorkspaceSetupComplete);
        let _ = engine.transition(EngineEvent::TrackCompleted { track_id: "backend".into() });
        assert_eq!(engine.state.phase, Phase::AwaitingCodeReview);
    }

    #[test]
    fn reject_code_restarts_tracks_with_feedback() {
        let mut engine = Engine::new("idea", "proj");
        engine.transition(EngineEvent::Started { idea: "idea".into() });
        engine.transition(EngineEvent::ScanComplete);
        engine.transition(EngineEvent::PlanComplete);
        engine.state.tracks = Some(vec![
            TrackState {
                id: "backend".into(),
                name: "Backend".into(),
                status: TrackStatus::Pending,
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
            },
        ]);
        engine.transition(EngineEvent::UserApprovedPlan);
        engine.transition(EngineEvent::WorkspaceSetupComplete);
        engine.transition(EngineEvent::TrackCompleted { track_id: "backend".into() });

        let effects = engine.transition(EngineEvent::UserRejectedCode {
            feedback: "error handling is wrong".into(),
        });
        assert_eq!(engine.state.phase, Phase::RunningTracks);
        assert_eq!(engine.state.code_feedback_history.len(), 1);
        assert!(effects.iter().any(|e| matches!(e, Effect::SpawnTrackWithFeedback { .. })));
    }

    #[test]
    fn approve_code_transitions_to_landing() {
        let mut engine = Engine::new("idea", "proj");
        engine.transition(EngineEvent::Started { idea: "idea".into() });
        engine.transition(EngineEvent::ScanComplete);
        engine.transition(EngineEvent::PlanComplete);
        engine.state.tracks = Some(vec![
            TrackState {
                id: "backend".into(),
                name: "Backend".into(),
                status: TrackStatus::Pending,
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
            },
        ]);
        engine.transition(EngineEvent::UserApprovedPlan);
        engine.transition(EngineEvent::WorkspaceSetupComplete);
        engine.transition(EngineEvent::TrackCompleted { track_id: "backend".into() });

        let effects = engine.transition(EngineEvent::UserApprovedCode { run_test_flight: false });
        assert_eq!(engine.state.phase, Phase::Landing);
        assert!(effects.contains(&Effect::CreateCRs));
        assert!(effects.contains(&Effect::SpawnLand));
    }

    #[test]
    fn land_complete_finishes_pipeline() {
        let mut engine = Engine::new("idea", "proj");
        engine.transition(EngineEvent::Started { idea: "idea".into() });
        engine.transition(EngineEvent::ScanComplete);
        engine.transition(EngineEvent::PlanComplete);
        engine.state.tracks = Some(vec![]);
        engine.transition(EngineEvent::UserApprovedPlan);
        engine.transition(EngineEvent::WorkspaceSetupComplete);
        // No tracks so AllTracksComplete fires immediately via all_tracks_terminal
        engine.transition(EngineEvent::UserApprovedCode { run_test_flight: false });
        let effects = engine.transition(EngineEvent::LandComplete);
        assert_eq!(engine.state.phase, Phase::Complete);
        assert!(effects.contains(&Effect::PipelineDone));
    }

    #[test]
    fn wrong_phase_events_are_noops() {
        let mut engine = Engine::new("idea", "proj");
        // Can't approve plan when still initializing
        let effects = engine.transition(EngineEvent::UserApprovedPlan);
        assert!(effects.is_empty());
        assert_eq!(engine.state.phase, Phase::Initializing);
    }

    // --- Test Flight ---

    fn make_track(id: &str) -> TrackState {
        TrackState {
            id: id.into(),
            name: id.into(),
            status: TrackStatus::Pending,
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
        }
    }

    #[test]
    fn new_engine_defaults_to_poc_template() {
        let engine = Engine::new("idea", "proj");
        assert_eq!(engine.state.template_id, "poc-from-design-doc");
        assert_eq!(engine.template().id, "poc-from-design-doc");
    }

    #[test]
    fn unknown_template_id_falls_back_to_poc() {
        let mut state = EngineState::new("idea", "proj");
        state.template_id = "does-not-exist".into();
        let engine = Engine::from_state(state);
        assert_eq!(engine.template().id, "poc-from-design-doc");
    }

    #[test]
    fn from_state_migrates_missing_test_flight_stage() {
        // Simulate an old checkpoint: 5 stages, no test_flight.
        let mut state = EngineState::new("idea", "proj");
        state.stages.retain(|s| s.id != StageId::TestFlight);
        assert_eq!(state.stages.len(), 5);
        let engine = Engine::from_state(state);
        // Migration inserts test_flight just before land.
        assert!(engine.state.stages.iter().any(|s| s.id == StageId::TestFlight));
        let tf = engine.state.stages.iter().position(|s| s.id == StageId::TestFlight).unwrap();
        let land = engine.state.stages.iter().position(|s| s.id == StageId::Land).unwrap();
        assert!(tf < land);
    }

    /// Drive a fresh engine to AwaitingCodeReview (Flight Check) with no tracks.
    fn at_flight_check() -> Engine {
        let mut engine = Engine::new("idea", "proj");
        engine.transition(EngineEvent::Started { idea: "idea".into() });
        engine.transition(EngineEvent::ScanComplete);
        engine.transition(EngineEvent::PlanComplete);
        engine.state.tracks = Some(vec![]);
        engine.transition(EngineEvent::UserApprovedPlan);
        engine.transition(EngineEvent::WorkspaceSetupComplete);
        assert_eq!(engine.state.phase, Phase::AwaitingCodeReview);
        engine
    }

    #[test]
    fn approve_code_without_test_flight_goes_to_landing() {
        let mut engine = at_flight_check();
        let effects = engine.transition(EngineEvent::UserApprovedCode { run_test_flight: false });
        assert_eq!(engine.state.phase, Phase::Landing);
        assert!(effects.contains(&Effect::SpawnLand));
    }

    #[test]
    fn approve_code_with_test_flight_drafts_plan() {
        let mut engine = at_flight_check();
        let effects = engine.transition(EngineEvent::UserApprovedCode { run_test_flight: true });
        assert_eq!(engine.state.phase, Phase::AwaitingTestPlan);
        assert!(effects.contains(&Effect::GenerateTestPlan));
        // Did NOT land yet
        assert!(!effects.contains(&Effect::SpawnLand));
    }

    #[test]
    fn approve_test_plan_spawns_test_flight() {
        let mut engine = at_flight_check();
        engine.transition(EngineEvent::UserApprovedCode { run_test_flight: true });
        let effects = engine.transition(EngineEvent::UserApprovedTestPlan);
        assert_eq!(engine.state.phase, Phase::TestFlying);
        assert_eq!(engine.state.test_iteration, 1);
        assert!(effects.contains(&Effect::SpawnTestFlight { feedback: None }));
    }

    #[test]
    fn test_flight_complete_awaits_review() {
        let mut engine = at_flight_check();
        engine.transition(EngineEvent::UserApprovedCode { run_test_flight: true });
        engine.transition(EngineEvent::UserApprovedTestPlan);
        engine.transition(EngineEvent::TestFlightComplete);
        assert_eq!(engine.state.phase, Phase::AwaitingTestReview);
    }

    #[test]
    fn test_flight_failure_still_awaits_review() {
        let mut engine = at_flight_check();
        engine.transition(EngineEvent::UserApprovedCode { run_test_flight: true });
        engine.transition(EngineEvent::UserApprovedTestPlan);
        engine.transition(EngineEvent::TestFlightFailed { error: "deploy failed".into() });
        // Failure surfaces for human review rather than dead-ending.
        assert_eq!(engine.state.phase, Phase::AwaitingTestReview);
    }

    #[test]
    fn engine_flight_check_branch_matches_template_next_stage() {
        // The engine's imperative Land-vs-TestFlight branch at Flight Check must
        // agree with what the POC template's next_stage() predicts. This locks
        // the template to the engine so they can't silently diverge as we move
        // toward template-driven advancement (Loopy step 3).
        let t = crate::pipeline::poc_from_design_doc();

        // Declining the opt-in (run_test_flight=false) → template skips the
        // optional test_flight block → next after flight_check is "land".
        assert_eq!(t.next_stage("flight_check", false).unwrap().id, "land");
        let mut e = at_flight_check();
        e.transition(EngineEvent::UserApprovedCode { run_test_flight: false });
        assert_eq!(e.state.phase, Phase::Landing);

        // Opting in (run_test_flight=true) → template includes test_flight →
        // next after flight_check is "test_flight".
        assert_eq!(t.next_stage("flight_check", true).unwrap().id, "test_flight");
        let mut e2 = at_flight_check();
        e2.transition(EngineEvent::UserApprovedCode { run_test_flight: true });
        assert_eq!(e2.state.phase, Phase::AwaitingTestPlan); // entering test_flight
    }

    #[test]
    fn accept_test_goes_to_landing() {
        let mut engine = at_flight_check();
        engine.transition(EngineEvent::UserApprovedCode { run_test_flight: true });
        engine.transition(EngineEvent::UserApprovedTestPlan);
        engine.transition(EngineEvent::TestFlightComplete);
        let effects = engine.transition(EngineEvent::UserAcceptedTest);
        assert_eq!(engine.state.phase, Phase::Landing);
        assert!(effects.contains(&Effect::SpawnLand));
    }

    #[test]
    fn reject_test_routes_back_to_tracks_additively() {
        let mut engine = Engine::new("idea", "proj");
        engine.transition(EngineEvent::Started { idea: "idea".into() });
        engine.transition(EngineEvent::ScanComplete);
        engine.transition(EngineEvent::PlanComplete);
        engine.state.tracks = Some(vec![make_track("backend")]);
        engine.transition(EngineEvent::UserApprovedPlan);
        engine.transition(EngineEvent::WorkspaceSetupComplete);
        engine.transition(EngineEvent::TrackCompleted { track_id: "backend".into() });
        engine.transition(EngineEvent::UserApprovedCode { run_test_flight: true });
        engine.transition(EngineEvent::UserApprovedTestPlan);
        engine.transition(EngineEvent::TestFlightComplete);

        let effects = engine.transition(EngineEvent::UserRejectedTest { feedback: "beta 500s on /generate".into() });
        assert_eq!(engine.state.phase, Phase::RunningTracks);
        assert_eq!(engine.state.test_feedback_history, vec!["beta 500s on /generate".to_string()]);
        assert!(effects.iter().any(|e| matches!(e, Effect::SpawnTrackWithFeedback { track_id, .. } if track_id == "backend")));
        // Track is running again
        assert_eq!(engine.state.tracks.as_ref().unwrap()[0].status, TrackStatus::Running);
    }

    #[test]
    fn abort_kills_everything() {
        let mut engine = Engine::new("idea", "proj");
        engine.transition(EngineEvent::Started { idea: "idea".into() });
        let effects = engine.transition(EngineEvent::Abort);
        assert_eq!(engine.state.phase, Phase::Failed);
        assert!(effects.contains(&Effect::KillAll));
    }

    #[test]
    fn full_happy_path() {
        let mut engine = Engine::new("Add rate limiting", "rate-limiter");
        engine.transition(EngineEvent::Started { idea: "Add rate limiting".into() });
        assert_eq!(engine.state.phase, Phase::Scanning);

        engine.transition(EngineEvent::ScanComplete);
        assert_eq!(engine.state.phase, Phase::Planning);

        engine.transition(EngineEvent::PlanComplete);
        assert_eq!(engine.state.phase, Phase::AwaitingPlanReview);

        // Set tracks (normally loaded from plan output)
        engine.state.tracks = Some(vec![
            TrackState {
                id: "backend".into(),
                name: "Backend".into(),
                status: TrackStatus::Pending,
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
            },
            TrackState {
                id: "infrastructure".into(),
                name: "Infrastructure".into(),
                status: TrackStatus::Pending,
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
            },
        ]);

        engine.transition(EngineEvent::UserApprovedPlan);
        assert_eq!(engine.state.phase, Phase::SettingUpWorkspaces);

        engine.transition(EngineEvent::WorkspaceSetupComplete);
        assert_eq!(engine.state.phase, Phase::RunningTracks);

        engine.transition(EngineEvent::TrackCompleted { track_id: "backend".into() });
        assert_eq!(engine.state.phase, Phase::RunningTracks); // infra still running

        engine.transition(EngineEvent::TrackCompleted { track_id: "infrastructure".into() });
        assert_eq!(engine.state.phase, Phase::AwaitingCodeReview);

        engine.transition(EngineEvent::UserApprovedCode { run_test_flight: false });
        assert_eq!(engine.state.phase, Phase::Landing);

        engine.transition(EngineEvent::LandComplete);
        assert_eq!(engine.state.phase, Phase::Complete);
    }
}
