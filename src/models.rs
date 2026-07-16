use chrono::{DateTime, FixedOffset, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

// --- Stage identifiers ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageId {
    Idea,
    Scan,
    Plan,
    RequirementsAnalysis,
    OrbitalLanes,
    TestFlight,
    Land,
}

impl StageId {
    /// All stage identifiers, in pipeline order.
    pub fn all() -> [StageId; 7] {
        [
            StageId::Idea,
            StageId::Scan,
            StageId::Plan,
            StageId::RequirementsAnalysis,
            StageId::OrbitalLanes,
            StageId::TestFlight,
            StageId::Land,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageStatus {
    Pending,
    Running,
    Complete,
    Failed,
}

// --- Track identifiers ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackId {
    Backend,
    Console,
    Frontend,
    Security,
    Auth,
    Infrastructure,
    Observability,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackStatus {
    Pending,
    PendingSetup,
    SettingUp,
    Running,
    Complete,
    Failed,
    Skipped,
    Blocked,
    Invalidated,
    ConflictPaused,
}

// --- Execution & approval modes ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum ExecutionMode {
    #[default]
    Safe,
    Fast,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum ApprovalMode {
    Required,
    #[default]
    Auto,
}

// --- Track working mediums ---

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum TrackMedium {
    #[default]
    Brazil,
    /// A docs-delivery track: writes documents into a folder rather than code
    /// into packages.
    Docs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageRef {
    pub name: String,
    pub version_set: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewPackage {
    pub name: String,
    pub template: String,
}

fn default_medium_config() -> MediumConfig {
    MediumConfig::Brazil {
        packages: vec![],
        new_packages: vec![],
        version_set: None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MediumConfig {
    Brazil {
        #[serde(default)]
        packages: Vec<PackageRef>,
        #[serde(default)]
        new_packages: Vec<NewPackage>,
        #[serde(default)]
        version_set: Option<String>,
    },
    Docs {
        folder: String,
        #[serde(default)]
        templates: Vec<String>,
    },
}

impl Default for MediumConfig {
    fn default() -> Self {
        default_medium_config()
    }
}

// --- Track definitions (config/preset format) ---

fn default_hat() -> String {
    "builtin:code-assist".into()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackDefinition {
    pub name: String,
    pub icon: String,
    pub description: String,
    #[serde(default = "default_hat")]
    pub hat: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub artifacts_produced: Vec<String>,
    #[serde(default)]
    pub approval: ApprovalMode,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub medium: TrackMedium,
    #[serde(default)]
    pub medium_config: MediumConfig,
    #[serde(default)]
    pub max_iterations: Option<u32>,
}

pub fn builtin_track_definitions() -> std::collections::BTreeMap<String, TrackDefinition> {
    let presets: &[(&str, &str, &str, &[&str])] = &[
        ("backend", "Backend", "🔧", &["api-contract"]),
        ("console", "Console", "🖥️", &["console-ui"]),
        ("frontend", "Frontend", "🌐", &["frontend-ui"]),
        ("security", "Security", "🔒", &["security-findings"]),
        ("auth", "Auth", "🔑", &["auth-flow"]),
        (
            "infrastructure",
            "Infrastructure",
            "☁️",
            &["deployment-shape"],
        ),
        (
            "observability",
            "Observability",
            "📊",
            &["observability-config"],
        ),
    ];
    presets
        .iter()
        .map(|(key, name, icon, artifacts)| {
            (
                key.to_string(),
                TrackDefinition {
                    name: name.to_string(),
                    icon: icon.to_string(),
                    description: format!("{name} track"),
                    hat: "builtin:code-assist".into(),
                    dependencies: vec![],
                    artifacts_produced: artifacts.iter().map(|s| s.to_string()).collect(),
                    approval: ApprovalMode::Auto,
                    enabled: true,
                    medium: TrackMedium::Brazil,
                    medium_config: MediumConfig::Brazil {
                        packages: vec![],
                        new_packages: vec![],
                        version_set: None,
                    },
                    max_iterations: None,
                },
            )
        })
        .collect()
}

// --- Artifact dependency ---

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactDep {
    pub artifact: String,
    #[serde(default = "default_true")]
    pub required: bool,
}

// --- Artifact system ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Artifact {
    pub artifact_name: String,
    pub version: u32,
    pub content_hash: String,
    pub produced_by: String,
    pub produced_at: DateTime<Utc>,
    pub content: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactEntry {
    pub current_version: u32,
    pub content_hash: String,
    pub produced_by: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ArtifactRegistry {
    pub artifacts: HashMap<String, ArtifactEntry>,
    pub consumptions: Vec<TrackConsumption>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackConsumption {
    pub track_id: String,
    pub consumed: Vec<(String, u32)>,
}

// --- Conflict record ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConflictRecord {
    pub id: String,
    pub affected_tracks: Vec<String>,
    pub description: String,
    pub detected_at: DateTime<Utc>,
    pub resolved: bool,
}

// --- Stage state ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StageState {
    pub id: StageId,
    pub status: StageStatus,
    pub loop_id: Option<String>,
    pub loop_pid: Option<u32>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

// --- Track state ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackState {
    pub id: String,
    pub name: String,
    pub status: TrackStatus,
    pub loop_id: Option<String>,
    pub loop_pid: Option<u32>,
    pub current_sub_stage: Option<String>,
    pub depends_on: Vec<ArtifactDep>,
    pub blocking_artifact: Option<String>,
    #[serde(default)]
    pub consumed_versions: Vec<(String, u32)>,
    #[serde(default)]
    pub run_count: u32,
    #[serde(default)]
    pub medium: TrackMedium,
    #[serde(default)]
    pub review_status: ReviewStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cr_url: Option<String>,
}

// --- Pipeline state (persisted) ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PipelineState {
    pub version: u32,
    pub idea_text: String,
    pub stages: Vec<StageState>,
    pub tracks: Option<Vec<TrackState>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub execution_mode: ExecutionMode,
    #[serde(default)]
    pub artifact_registry: ArtifactRegistry,
    #[serde(default)]
    pub active_conflicts: Vec<ConflictRecord>,
    #[serde(default)]
    pub awaiting_approval: Option<StageId>,
}

// --- Track manifest (from Plan stage) ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackDef {
    pub id: String,
    pub name: String,
    pub description: String,
    pub sub_stages: Vec<String>,
    pub hat_collection: String,
    pub prompt_file: PathBuf,
    pub depends_on: Vec<ArtifactDep>,
    #[serde(default)]
    pub produces: Vec<String>,
    #[serde(default)]
    pub consumes: Vec<ArtifactDep>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackManifest {
    pub tracks: Vec<TrackDef>,
}

// --- Ralph event (from .ralph/events-*.jsonl) ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewComment {
    pub file: String,
    pub line: usize,
    pub hunk_context: String,
    pub comment: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CrComment {
    pub file: String,
    pub line: usize,
    pub author: String,
    pub body: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    #[default]
    PendingReview,
    ChangesRequested,
    Approved,
}

// --- Collaborator (user presence) ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Collaborator {
    pub alias: String,
    pub connected_at: DateTime<Utc>,
    pub viewing_stage: Option<StageId>,
}

// --- Ralph event (from .ralph/events-*.jsonl) ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RalphEvent {
    #[serde(rename = "ts")]
    pub timestamp: DateTime<FixedOffset>,
    pub topic: String,
    pub payload: serde_json::Value,
    pub iteration: Option<u32>,
    pub hat: Option<String>,
    pub triggered: Option<String>,
}

// --- Ralph task (from .ralph/agent/tasks.jsonl) ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RalphTask {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub key: Option<String>,
    pub status: String,
    pub priority: Option<u32>,
    pub blocked_by: Vec<String>,
    pub loop_id: Option<String>,
    pub created: Option<String>,
    pub started: Option<String>,
    pub closed: Option<String>,
}

// --- Log entries (streamed to the UI) ---

/// Log severity for a streamed log entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

/// A single log entry produced by the aggregator / stream forwarders.
#[derive(Debug, Clone, PartialEq)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: LogLevel,
    pub message: String,
}

// --- Pipeline events (parsed from agent event files by the aggregator) ---

/// A high-level pipeline event derived from an agent's `.ralph` event stream.
/// The aggregator emits these; the engine runner consumes them to advance the
/// pipeline. Lean set — only the variants the runtime actually uses.
#[derive(Debug, Clone, PartialEq)]
pub enum PipelineEvent {
    StageCompleted { stage: StageId },
    StageFailed { stage: StageId, error: String },
    TrackCompleted { track: String },
    TrackFailed { track: String, error: String },
    TrackSetupComplete { track: String },
    ArtifactProduced { track: String, artifact: String },
    ArtifactVersioned { track: String, artifact: String, version: u32, content_hash: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike;

    #[test]
    fn deserialize_ralph_event_string_payload() {
        let json = r#"{"ts":"2026-03-31T19:40:40.977787752+00:00","topic":"design.start","payload":"hello","iteration":0,"hat":"loop","triggered":"planner"}"#;
        let event: RalphEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.topic, "design.start");
        assert_eq!(event.payload, serde_json::Value::String("hello".into()));
        assert_eq!(event.iteration, Some(0));
        assert_eq!(event.hat, Some("loop".into()));
        assert_eq!(event.triggered, Some("planner".into()));
        assert_eq!(event.timestamp.year(), 2026);
    }

    #[test]
    fn deserialize_ralph_event_object_payload() {
        let json = r#"{"ts":"2026-03-31T20:00:00+00:00","topic":"answer.proposed","payload":{"summary":"test","task_id":"t1"}}"#;
        let event: RalphEvent = serde_json::from_str(json).unwrap();
        assert!(event.payload.is_object());
        assert_eq!(event.payload["summary"], "test");
        assert_eq!(event.iteration, None);
        assert_eq!(event.hat, None);
        assert_eq!(event.triggered, None);
    }

    #[test]
    fn deserialize_ralph_task() {
        let json = r#"{"id":"task-123","title":"Test task","description":"desc","key":"k1","status":"closed","priority":1,"blocked_by":[],"loop_id":"primary-123","created":"2026-03-31T19:41:27+00:00"}"#;
        let task: RalphTask = serde_json::from_str(json).unwrap();
        assert_eq!(task.id, "task-123");
        assert_eq!(task.status, "closed");
    }

    // --- AC1: StageId has 6 variants, RequirementsAnalysis round-trips ---
    #[test]
    fn stage_id_requirements_analysis_round_trip() {
        let stage = StageId::RequirementsAnalysis;
        let json = serde_json::to_string(&stage).unwrap();
        assert_eq!(json, r#""requirements_analysis""#);
        let back: StageId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, StageId::RequirementsAnalysis);
    }

    #[test]
    fn stage_id_all_variants_round_trip() {
        let variants = [
            StageId::Idea,
            StageId::Scan,
            StageId::Plan,
            StageId::RequirementsAnalysis,
            StageId::OrbitalLanes,
            StageId::Land,
        ];
        for v in &variants {
            let json = serde_json::to_string(v).unwrap();
            let back: StageId = serde_json::from_str(&json).unwrap();
            assert_eq!(&back, v);
        }
    }

    // --- AC2: TrackStatus new variants ---
    #[test]
    fn track_status_invalidated_round_trip() {
        let s = TrackStatus::Invalidated;
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, r#""invalidated""#);
        let back: TrackStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back, TrackStatus::Invalidated);
    }

    #[test]
    fn track_status_conflict_paused_round_trip() {
        let s = TrackStatus::ConflictPaused;
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, r#""conflict_paused""#);
        let back: TrackStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back, TrackStatus::ConflictPaused);
    }

    // --- AC3: TrackId enum round-trip ---
    #[test]
    fn track_id_all_variants_round_trip() {
        let variants = [
            TrackId::Backend,
            TrackId::Console,
            TrackId::Frontend,
            TrackId::Security,
            TrackId::Auth,
            TrackId::Infrastructure,
            TrackId::Observability,
        ];
        for v in &variants {
            let json = serde_json::to_string(v).unwrap();
            let back: TrackId = serde_json::from_str(&json).unwrap();
            assert_eq!(&back, v);
        }
    }

    // --- AC4: ArtifactDep new schema ---
    #[test]
    fn artifact_dep_serialization() {
        let dep = ArtifactDep {
            artifact: "api-contract".into(),
            required: true,
        };
        let json = serde_json::to_string(&dep).unwrap();
        assert_eq!(json, r#"{"artifact":"api-contract","required":true}"#);
    }

    // --- AC5: ArtifactDep required defaults true ---
    #[test]
    fn artifact_dep_required_defaults_true() {
        let json = r#"{"artifact":"api-contract"}"#;
        let dep: ArtifactDep = serde_json::from_str(json).unwrap();
        assert!(dep.required);
    }

    #[test]
    fn artifact_dep_optional() {
        let json = r#"{"artifact":"security-findings","required":false}"#;
        let dep: ArtifactDep = serde_json::from_str(json).unwrap();
        assert!(!dep.required);
    }

    // --- AC6: Artifact struct round-trip ---
    #[test]
    fn artifact_round_trip() {
        let art = Artifact {
            artifact_name: "api-contract".into(),
            version: 2,
            content_hash: "sha256:abc123".into(),
            produced_by: "backend".into(),
            produced_at: Utc::now(),
            content: serde_json::json!({"endpoints": ["/api/v1/users"]}),
        };
        let json = serde_json::to_string(&art).unwrap();
        let back: Artifact = serde_json::from_str(&json).unwrap();
        assert_eq!(art, back);
    }

    // --- AC7: ArtifactRegistry round-trip ---
    #[test]
    fn artifact_registry_round_trip() {
        let reg = ArtifactRegistry {
            artifacts: HashMap::from([(
                "api-contract".into(),
                ArtifactEntry {
                    current_version: 2,
                    content_hash: "sha256:abc".into(),
                    produced_by: "backend".into(),
                },
            )]),
            consumptions: vec![TrackConsumption {
                track_id: "console".into(),
                consumed: vec![("api-contract".into(), 1)],
            }],
        };
        let json = serde_json::to_string(&reg).unwrap();
        let back: ArtifactRegistry = serde_json::from_str(&json).unwrap();
        assert_eq!(reg, back);
    }

    // --- AC8: ExecutionMode round-trip ---
    #[test]
    fn execution_mode_round_trip() {
        for (mode, expected) in [
            (ExecutionMode::Safe, "\"safe\""),
            (ExecutionMode::Fast, "\"fast\""),
        ] {
            let json = serde_json::to_string(&mode).unwrap();
            assert_eq!(json, expected);
            let back: ExecutionMode = serde_json::from_str(&json).unwrap();
            assert_eq!(back, mode);
        }
    }

    // --- AC9: PipelineState v2 fields round-trip ---
    #[test]
    fn pipeline_state_v2_round_trip() {
        let state = PipelineState {
            version: 2,
            idea_text: "Build feature X".into(),
            stages: vec![StageState {
                id: StageId::Idea,
                status: StageStatus::Complete,
                loop_id: None,
                loop_pid: None,
                started_at: Some(Utc::now()),
                completed_at: Some(Utc::now()),
                error: None,
            }],
            tracks: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            execution_mode: ExecutionMode::Fast,
            artifact_registry: ArtifactRegistry {
                artifacts: HashMap::from([(
                    "api-contract".into(),
                    ArtifactEntry {
                        current_version: 1,
                        content_hash: "sha256:xyz".into(),
                        produced_by: "backend".into(),
                    },
                )]),
                consumptions: vec![],
            },
            active_conflicts: vec![ConflictRecord {
                id: "c1".into(),
                affected_tracks: vec!["backend".into(), "security".into()],
                description: "API design conflict".into(),
                detected_at: Utc::now(),
                resolved: false,
            }],
            awaiting_approval: None,
        };
        let json = serde_json::to_string(&state).unwrap();
        let back: PipelineState = serde_json::from_str(&json).unwrap();
        assert_eq!(state, back);
    }

    // --- AC10: PipelineState v1 JSON missing v2 fields → defaults ---
    #[test]
    fn pipeline_state_v1_defaults() {
        let json = r#"{
            "version": 1,
            "idea_text": "test",
            "stages": [],
            "tracks": null,
            "created_at": "2026-04-01T00:00:00Z",
            "updated_at": "2026-04-01T00:00:00Z"
        }"#;
        let state: PipelineState = serde_json::from_str(json).unwrap();
        assert_eq!(state.execution_mode, ExecutionMode::Safe);
        assert!(state.artifact_registry.artifacts.is_empty());
        assert!(state.active_conflicts.is_empty());
    }

    // --- AC11: TrackState v2 fields ---
    #[test]
    fn track_state_v2_fields_round_trip() {
        let ts = TrackState {
            id: "backend".into(),
            name: "Backend".into(),
            status: TrackStatus::Running,
            loop_id: None,
            loop_pid: None,
            current_sub_stage: None,
            depends_on: vec![],
            blocking_artifact: None,
            consumed_versions: vec![("api-contract".into(), 1)],
            run_count: 2,
            medium: TrackMedium::default(),
            review_status: Default::default(),
            cr_url: None,
        };
        let json = serde_json::to_string(&ts).unwrap();
        let back: TrackState = serde_json::from_str(&json).unwrap();
        assert_eq!(ts, back);
    }

    #[test]
    fn track_state_v1_defaults() {
        let json = r#"{
            "id": "backend",
            "name": "Backend",
            "status": "pending",
            "loop_id": null,
            "loop_pid": null,
            "current_sub_stage": null,
            "depends_on": [],
            "blocking_artifact": null
        }"#;
        let ts: TrackState = serde_json::from_str(json).unwrap();
        assert!(ts.consumed_versions.is_empty());
        assert_eq!(ts.run_count, 0);
    }

    // --- AC12: ConflictRecord round-trip ---
    #[test]
    fn conflict_record_round_trip() {
        let cr = ConflictRecord {
            id: "conflict-1".into(),
            affected_tracks: vec!["backend".into(), "security".into()],
            description: "Conflicting API design".into(),
            detected_at: Utc::now(),
            resolved: true,
        };
        let json = serde_json::to_string(&cr).unwrap();
        let back: ConflictRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(cr, back);
    }

    // --- TrackDefinition ---
    #[test]
    fn track_definition_round_trip() {
        let td = TrackDefinition {
            name: "Load Testing".into(),
            icon: "⚡".into(),
            description: "Performance testing".into(),
            hat: "builtin:code-assist".into(),
            dependencies: vec!["backend".into()],
            artifacts_produced: vec!["load-test-results".into()],
            approval: ApprovalMode::Required,
            enabled: false,
            medium: TrackMedium::Brazil,
            medium_config: MediumConfig::default(),
            max_iterations: None,
        };
        let yaml = serde_yaml::to_string(&td).unwrap();
        let back: TrackDefinition = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(td, back);
    }

    #[test]
    fn track_definition_defaults() {
        let yaml = r#"
name: "Test"
icon: "🔧"
description: "A test track"
"#;
        let td: TrackDefinition = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(td.hat, "builtin:code-assist");
        assert!(td.dependencies.is_empty());
        assert!(td.artifacts_produced.is_empty());
        assert_eq!(td.approval, ApprovalMode::Auto);
        assert!(td.enabled);
    }

    #[test]
    fn builtin_track_definitions_count() {
        let builtins = builtin_track_definitions();
        assert_eq!(builtins.len(), 7);
    }

    #[test]
    fn builtin_track_definitions_all_enabled() {
        for (_, td) in builtin_track_definitions() {
            assert!(td.enabled);
            assert_eq!(td.approval, ApprovalMode::Auto);
            assert_eq!(td.hat, "builtin:code-assist");
        }
    }

    #[test]
    fn builtin_track_definitions_keys_match_track_ids() {
        let builtins = builtin_track_definitions();
        let keys: Vec<&String> = builtins.keys().collect();
        for name in &[
            "backend",
            "console",
            "frontend",
            "security",
            "auth",
            "infrastructure",
            "observability",
        ] {
            assert!(keys.contains(&&name.to_string()), "missing key: {name}");
        }
    }

    // --- TrackDef v2 fields ---
    #[test]
    fn track_def_v2_fields_default() {
        let json = r#"{
            "id": "backend",
            "name": "Backend",
            "description": "desc",
            "sub_stages": ["code"],
            "hat_collection": "default",
            "prompt_file": "backend.md",
            "depends_on": []
        }"#;
        let td: TrackDef = serde_json::from_str(json).unwrap();
        assert!(td.produces.is_empty());
        assert!(td.consumes.is_empty());
    }

    // --- Step 2: TrackMedium, PackageRef, NewPackage, MediumConfig ---

    #[test]
    fn track_medium_default_is_brazil() {
        let m: TrackMedium = Default::default();
        assert_eq!(m, TrackMedium::Brazil);
    }

    #[test]
    fn track_medium_round_trip_serde() {
        for m in [TrackMedium::Brazil, TrackMedium::Docs] {
            let json = serde_json::to_string(&m).unwrap();
            let back: TrackMedium = serde_json::from_str(&json).unwrap();
            assert_eq!(back, m);
        }
    }

    #[test]
    fn package_ref_round_trip_serde() {
        let pr = PackageRef {
            name: "MyService".into(),
            version_set: "MyService/development".into(),
        };
        let json = serde_json::to_string(&pr).unwrap();
        let back: PackageRef = serde_json::from_str(&json).unwrap();
        assert_eq!(back, pr);
    }

    #[test]
    fn new_package_round_trip_serde() {
        let np = NewPackage {
            name: "MyInfra".into(),
            template: "cdk-app".into(),
        };
        let json = serde_json::to_string(&np).unwrap();
        let back: NewPackage = serde_json::from_str(&json).unwrap();
        assert_eq!(back, np);
    }

    #[test]
    fn medium_config_brazil_round_trip() {
        let mc = MediumConfig::Brazil {
            packages: vec![PackageRef {
                name: "Svc".into(),
                version_set: "Svc/dev".into(),
            }],
            new_packages: vec![],
            version_set: Some("Svc/dev".into()),
        };
        let json = serde_json::to_string(&mc).unwrap();
        let back: MediumConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back, mc);
    }

    #[test]
    fn medium_config_docs_round_trip() {
        let mc = MediumConfig::Docs {
            folder: "folder-id".into(),
            templates: vec!["ThreatModel".into()],
        };
        let json = serde_json::to_string(&mc).unwrap();
        let back: MediumConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back, mc);
    }

    #[test]
    fn builtin_track_definitions_all_have_brazil_medium() {
        let defs = builtin_track_definitions();
        for (key, def) in &defs {
            assert_eq!(
                def.medium,
                TrackMedium::Brazil,
                "track {key} should have Brazil medium"
            );
            assert!(
                matches!(&def.medium_config, MediumConfig::Brazil { packages, .. } if packages.is_empty()),
                "track {key} should have empty Brazil medium_config"
            );
        }
    }

    #[test]
    fn track_definition_with_medium_deserializes_from_yaml() {
        let yaml = r#"
name: Test Track
icon: "🧪"
description: A test track
medium: docs
medium_config:
  type: docs
  folder: "abc123"
  templates:
    - ThreatModel
"#;
        let td: TrackDefinition = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(td.medium, TrackMedium::Docs);
        assert!(
            matches!(&td.medium_config, MediumConfig::Docs { folder, .. } if folder == "abc123")
        );
    }

    #[test]
    fn track_definition_without_medium_defaults_to_brazil() {
        let yaml = r#"
name: Minimal Track
icon: "📦"
description: No medium specified
"#;
        let td: TrackDefinition = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(td.medium, TrackMedium::Brazil);
        assert!(
            matches!(&td.medium_config, MediumConfig::Brazil { packages, .. } if packages.is_empty())
        );
    }

    #[test]
    fn track_definition_brazil_medium_config_from_yaml() {
        let yaml = r#"
name: Backend
icon: "🔧"
description: Backend track
medium: brazil
medium_config:
  type: brazil
  packages:
    - name: MyService
      version_set: MyService/development
"#;
        let td: TrackDefinition = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(td.medium, TrackMedium::Brazil);
        match &td.medium_config {
            MediumConfig::Brazil { packages, .. } => {
                assert_eq!(packages.len(), 1);
                assert_eq!(packages[0].name, "MyService");
            }
            _ => panic!("expected Brazil medium_config"),
        }
    }

    // --- ReviewComment / ReviewStatus tests ---

    #[test]
    fn review_comment_round_trip() {
        let comment = ReviewComment {
            file: "src/main.rs".into(),
            line: 42,
            hunk_context: "fn main() {".into(),
            comment: "Handle the error case".into(),
            timestamp: Utc::now(),
        };
        let json = serde_json::to_string(&comment).unwrap();
        let back: ReviewComment = serde_json::from_str(&json).unwrap();
        assert_eq!(comment, back);
    }

    #[test]
    fn review_status_round_trip() {
        for status in [
            ReviewStatus::PendingReview,
            ReviewStatus::ChangesRequested,
            ReviewStatus::Approved,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let back: ReviewStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(back, status);
        }
    }

    #[test]
    fn review_status_default_is_pending() {
        let status: ReviewStatus = Default::default();
        assert_eq!(status, ReviewStatus::PendingReview);
    }

    #[test]
    fn collaborator_serde_round_trip() {
        let c = Collaborator {
            alias: "arvinmaa".into(),
            connected_at: Utc::now(),
            viewing_stage: Some(StageId::Plan),
        };
        let json = serde_json::to_string(&c).unwrap();
        let back: Collaborator = serde_json::from_str(&json).unwrap();
        assert_eq!(back.alias, "arvinmaa");
        assert_eq!(back.viewing_stage, Some(StageId::Plan));
    }

    #[test]
    fn collaborator_serde_no_stage() {
        let c = Collaborator {
            alias: "teammate".into(),
            connected_at: Utc::now(),
            viewing_stage: None,
        };
        let json = serde_json::to_string(&c).unwrap();
        let back: Collaborator = serde_json::from_str(&json).unwrap();
        assert_eq!(back.viewing_stage, None);
    }

    #[test]
    fn cr_comment_serde_round_trip() {
        let c = CrComment {
            file: "src/main.rs".into(),
            line: 42,
            author: "reviewer".into(),
            body: "Handle the error case".into(),
            timestamp: Utc::now(),
        };
        let json = serde_json::to_string(&c).unwrap();
        let back: CrComment = serde_json::from_str(&json).unwrap();
        assert_eq!(back.file, "src/main.rs");
        assert_eq!(back.line, 42);
        assert_eq!(back.author, "reviewer");
        assert_eq!(back.body, "Handle the error case");
    }

}
