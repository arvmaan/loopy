use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use chrono::{DateTime, FixedOffset};

use crate::models::{LogEntry, PipelineEvent, StageId};
use crate::watcher::FileEvent;

/// Per-loop state tracked for the TUI view model.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct LoopState {
    #[serde(default)]
    pub latest_event_ts: Option<DateTime<FixedOffset>>,
    #[serde(default)]
    pub latest_hat: Option<String>,
    #[serde(default)]
    pub task_count: usize,
    #[serde(default)]
    pub latest_topic: Option<String>,
    #[serde(default)]
    pub latest_iteration: Option<u32>,
}

/// Result of applying a file event — may produce a pipeline event and/or log entries.
#[derive(Debug, PartialEq)]
pub struct ApplyResult {
    pub pipeline_event: Option<PipelineEvent>,
    pub log_entries: Vec<LogEntry>,
}

/// Bridges FileEvents from the watcher to PipelineEvents for the state machine.
#[derive(Default)]
pub struct Aggregator {
    pub loops: HashMap<String, LoopState>,
    pub last_event_time: Option<Instant>,
}

impl Aggregator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a formatted status string from the most recently updated loop.
    pub fn latest_ralph_status(&self) -> Option<String> {
        self.loops
            .values()
            .filter(|s| s.latest_event_ts.is_some())
            .max_by_key(|s| s.latest_event_ts)
            .map(|s| {
                let iter = s.latest_iteration.unwrap_or(0);
                let hat = s.latest_hat.as_deref().unwrap_or("?");
                let topic = s.latest_topic.as_deref().unwrap_or("?");
                format!("Iteration {} | {} → {}", iter, hat, topic)
            })
    }

    pub fn clear(&mut self) {
        for state in self.loops.values_mut() {
            state.latest_hat = None;
            state.latest_topic = None;
            state.latest_iteration = None;
        }
    }

    /// Serialize loop states to a JSON file (atomic temp+rename).
    pub fn save_state(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_string_pretty(&self.loops).map_err(std::io::Error::other)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, &data)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Restore loop states from a JSON file.
    pub fn load_state(&mut self, path: &Path) -> std::io::Result<()> {
        let data = std::fs::read_to_string(path)?;
        let loops: HashMap<String, LoopState> = serde_json::from_str(&data)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        self.loops = loops;
        Ok(())
    }

    pub fn apply(&mut self, event: FileEvent) -> ApplyResult {
        self.last_event_time = Some(Instant::now());
        match event {
            FileEvent::EventsAppended {
                loop_id,
                new_events,
                ..
            } => {
                // The loop_id encodes which stage emitted the event
                // (e.g. "Scan-123", "Plan-456", "OrbitalLanes-789"), so a
                // generic loop.terminate can be attributed to the right stage.
                let stage_from_loop = stage_from_loop_id(&loop_id);
                let loop_state = self.loops.entry(loop_id).or_default();
                let mut result = None;
                let mut log_entries = Vec::new();
                for ev in &new_events {
                    loop_state.latest_event_ts = Some(ev.timestamp);
                    if let Some(ref hat) = ev.hat {
                        loop_state.latest_hat = Some(hat.clone());
                    }
                    loop_state.latest_topic = Some(ev.topic.clone());
                    loop_state.latest_iteration = ev.iteration;
                    result = map_topic(&ev.topic, &ev.payload, stage_from_loop);

                    // Produce a log entry summary for each JSONL event
                    let iter_str = ev
                        .iteration
                        .map(|i| format!("iter {i}"))
                        .unwrap_or_default();
                    let hat_str = ev.hat.as_deref().unwrap_or("?");
                    let ts = ev.timestamp.format("%H:%M:%S").to_string();
                    let msg = format!("[{hat_str}] {iter_str} {}", ev.topic);
                    log_entries.push(LogEntry {
                        timestamp: ts,
                        level: crate::models::LogLevel::Info,
                        message: msg,
                    });
                }
                ApplyResult {
                    pipeline_event: result,
                    log_entries,
                }
            }
            FileEvent::TasksChanged { tasks } => {
                // Zero all existing counts — tasks.jsonl is a full rewrite
                for state in self.loops.values_mut() {
                    state.task_count = 0;
                }
                for t in &tasks {
                    if let Some(ref lid) = t.loop_id {
                        self.loops.entry(lid.clone()).or_default().task_count += 1;
                    }
                }
                ApplyResult {
                    pipeline_event: None,
                    log_entries: vec![],
                }
            }
            FileEvent::ArtifactCreated { path } => ApplyResult {
                pipeline_event: map_artifact(&path),
                log_entries: vec![],
            },
            FileEvent::StateJsonChanged { .. } | FileEvent::LogLinesAppended { .. } => {
                // These are forwarded directly via WebSocket, not through the state machine
                ApplyResult {
                    pipeline_event: None,
                    log_entries: vec![],
                }
            }
        }
    }
}

/// Derive the pipeline stage from a loop_id like "Scan-1781...", "Plan-...",
/// "Land-...", or "OrbitalLanes-...". Returns None for track loops or unknown.
fn stage_from_loop_id(loop_id: &str) -> Option<StageId> {
    let prefix = loop_id.split('-').next().unwrap_or("");
    match prefix {
        "Scan" => Some(StageId::Scan),
        "Plan" => Some(StageId::Plan),
        "Land" => Some(StageId::Land),
        "RequirementsAnalysis" => Some(StageId::RequirementsAnalysis),
        "OrbitalLanes" => Some(StageId::OrbitalLanes),
        _ => None,
    }
}

fn map_topic(
    topic: &str,
    payload: &serde_json::Value,
    stage_from_loop: Option<StageId>,
) -> Option<PipelineEvent> {
    match topic {
        "scan.complete" | "scan.done" => Some(PipelineEvent::StageCompleted {
            stage: StageId::Scan,
        }),
        "land.complete" | "land.done" => Some(PipelineEvent::StageCompleted {
            stage: StageId::Land,
        }),
        "plan.complete" | "plan.done" => Some(PipelineEvent::StageCompleted {
            stage: StageId::Plan,
        }),
        "requirements.complete" | "requirements.done" => Some(PipelineEvent::StageCompleted {
            stage: StageId::RequirementsAnalysis,
        }),
        "loop.terminate" => {
            // Ralph emits loop.terminate when the completion promise is satisfied.
            // Ralph's loop_id is its own timestamp id (e.g. "primary-20260616-..."),
            // NOT Loopy's stage id, so stage_from_loop is usually None. We emit a
            // generic StageCompleted and let the engine_runner attribute it to the
            // currently-running stage (with a phase-match guard against duplicates).
            let payload_str = payload.as_str().unwrap_or("");
            let completed = payload_str.contains("completed")
                || payload_str.contains("Completion promise");
            if completed {
                // Use the loop-derived stage if known, else Scan as a placeholder
                // (engine_runner overrides by current phase).
                let stage = stage_from_loop.unwrap_or(StageId::Scan);
                Some(PipelineEvent::StageCompleted { stage })
            } else {
                None
            }
        }
        other => {
            let parts: Vec<&str> = other.splitn(4, '.').collect();
            if parts.len() >= 3 && parts[0] == "track" {
                let id = parts[1].to_string();
                match (parts.get(2).copied(), parts.get(3)) {
                    (Some("complete"), None) => Some(PipelineEvent::TrackCompleted { track: id }),
                    (Some("setup"), Some(&"complete")) => {
                        Some(PipelineEvent::TrackSetupComplete { track: id })
                    }
                    (Some("failed"), _) => {
                        let error = payload["error"].as_str().unwrap_or("").to_string();
                        Some(PipelineEvent::TrackFailed { track: id, error })
                    }
                    (Some("artifact"), Some(name)) => {
                        let version = payload["version"].as_u64().unwrap_or(0) as u32;
                        let content_hash =
                            payload["content_hash"].as_str().unwrap_or("").to_string();
                        Some(PipelineEvent::ArtifactVersioned {
                            track: id,
                            artifact: name.to_string(),
                            version,
                            content_hash,
                        })
                    }
                    _ => None,
                }
            } else {
                None
            }
        }
    }
}

fn map_artifact(path: &Path) -> Option<PipelineEvent> {
    // Expected: .loopy/artifacts/{track}/{artifact_name}
    let components: Vec<&str> = path.iter().filter_map(|c| c.to_str()).collect();
    // Find "artifacts" segment, then track and filename follow
    let artifacts_idx = components.iter().position(|c| *c == "artifacts")?;
    let track = components.get(artifacts_idx + 1)?;
    let artifact = components.get(artifacts_idx + 2)?;
    Some(PipelineEvent::ArtifactProduced {
        track: track.to_string(),
        artifact: artifact.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{RalphEvent, RalphTask};
    use std::path::PathBuf;

    fn make_event(topic: &str) -> RalphEvent {
        RalphEvent {
            timestamp: DateTime::parse_from_rfc3339("2026-03-31T20:00:00+00:00").unwrap(),
            topic: topic.to_string(),
            payload: serde_json::Value::Null,
            iteration: Some(1),
            hat: Some("builder".into()),
            triggered: None,
        }
    }

    fn make_events_appended(topic: &str) -> FileEvent {
        FileEvent::EventsAppended {
            loop_id: "loop-1".into(),
            source: PathBuf::from("loop-1/.ralph/events-loop-1.jsonl"),
            new_events: vec![make_event(topic)],
        }
    }

    // AC1: scan.complete → StageCompleted { Scan }
    #[test]
    fn scan_complete_maps_to_stage_completed() {
        let mut agg = Aggregator::new();
        let result = agg
            .apply(make_events_appended("scan.complete"))
            .pipeline_event;
        assert_eq!(
            result,
            Some(PipelineEvent::StageCompleted {
                stage: StageId::Scan
            })
        );
    }

    // AC2: land.complete → StageCompleted { Land }
    #[test]
    fn land_complete_maps_to_stage_completed() {
        let mut agg = Aggregator::new();
        let result = agg
            .apply(make_events_appended("land.complete"))
            .pipeline_event;
        assert_eq!(
            result,
            Some(PipelineEvent::StageCompleted {
                stage: StageId::Land
            })
        );
    }

    // AC3: track.{id}.complete → TrackCompleted { track: id }
    #[test]
    fn track_completion_maps_to_track_completed() {
        let mut agg = Aggregator::new();
        let result = agg
            .apply(make_events_appended("track.backend.complete"))
            .pipeline_event;
        assert_eq!(
            result,
            Some(PipelineEvent::TrackCompleted {
                track: "backend".into()
            })
        );
    }

    #[test]
    fn track_completion_with_different_id() {
        let mut agg = Aggregator::new();
        let result = agg
            .apply(make_events_appended("track.frontend.complete"))
            .pipeline_event;
        assert_eq!(
            result,
            Some(PipelineEvent::TrackCompleted {
                track: "frontend".into()
            })
        );
    }

    // AC4: unknown topic → None
    #[test]
    fn unknown_topic_returns_none() {
        let mut agg = Aggregator::new();
        let result = agg
            .apply(make_events_appended("some.random.topic"))
            .pipeline_event;
        assert_eq!(result, None);
    }

    #[test]
    fn design_start_returns_none() {
        let mut agg = Aggregator::new();
        let result = agg
            .apply(make_events_appended("design.start"))
            .pipeline_event;
        assert_eq!(result, None);
    }

    // AC5: ArtifactCreated → ArtifactProduced
    #[test]
    fn artifact_created_maps_to_artifact_produced() {
        let mut agg = Aggregator::new();
        let result = agg
            .apply(FileEvent::ArtifactCreated {
                path: PathBuf::from(".loopy/artifacts/backend/api-contracts.json"),
            })
            .pipeline_event;
        assert_eq!(
            result,
            Some(PipelineEvent::ArtifactProduced {
                track: "backend".into(),
                artifact: "api-contracts.json".into()
            })
        );
    }

    #[test]
    fn artifact_without_artifacts_segment_returns_none() {
        let mut agg = Aggregator::new();
        let result = agg
            .apply(FileEvent::ArtifactCreated {
                path: PathBuf::from("some/random/file.txt"),
            })
            .pipeline_event;
        assert_eq!(result, None);
    }

    // AC6: per-loop state tracking
    #[test]
    fn tracks_latest_event_and_hat_per_loop() {
        let mut agg = Aggregator::new();
        let ts1 = DateTime::parse_from_rfc3339("2026-03-31T20:00:00+00:00").unwrap();
        let ts2 = DateTime::parse_from_rfc3339("2026-03-31T20:05:00+00:00").unwrap();

        agg.apply(FileEvent::EventsAppended {
            loop_id: "loop-1".into(),
            source: PathBuf::from("loop-1/.ralph/events-loop-1.jsonl"),
            new_events: vec![RalphEvent {
                timestamp: ts1,
                topic: "scan.complete".into(),
                payload: serde_json::Value::Null,
                iteration: Some(1),
                hat: Some("scanner".into()),
                triggered: None,
            }],
        });

        agg.apply(FileEvent::EventsAppended {
            loop_id: "loop-1".into(),
            source: PathBuf::from("loop-1/.ralph/events-loop-1.jsonl"),
            new_events: vec![RalphEvent {
                timestamp: ts2,
                topic: "some.event".into(),
                payload: serde_json::Value::Null,
                iteration: Some(2),
                hat: Some("builder".into()),
                triggered: None,
            }],
        });

        let state = &agg.loops["loop-1"];
        assert_eq!(state.latest_event_ts, Some(ts2));
        assert_eq!(state.latest_hat, Some("builder".into()));
    }

    #[test]
    fn tasks_changed_updates_task_count() {
        let mut agg = Aggregator::new();
        agg.apply(FileEvent::TasksChanged {
            tasks: vec![
                RalphTask {
                    id: "t1".into(),
                    title: "Task 1".into(),
                    description: None,
                    key: None,
                    status: "open".into(),
                    priority: None,
                    blocked_by: vec![],
                    loop_id: Some("loop-1".into()),
                    created: None,
                    started: None,
                    closed: None,
                },
                RalphTask {
                    id: "t2".into(),
                    title: "Task 2".into(),
                    description: None,
                    key: None,
                    status: "open".into(),
                    priority: None,
                    blocked_by: vec![],
                    loop_id: Some("loop-1".into()),
                    created: None,
                    started: None,
                    closed: None,
                },
            ],
        });
        assert_eq!(agg.loops["loop-1"].task_count, 2);
    }

    // TasksChanged returns None (no PipelineEvent)
    #[test]
    fn tasks_changed_returns_none() {
        let mut agg = Aggregator::new();
        let result = agg
            .apply(FileEvent::TasksChanged { tasks: vec![] })
            .pipeline_event;
        assert_eq!(result, None);
    }

    // --- Step 9: New event mappings ---

    fn make_events_appended_with_payload(topic: &str, payload: serde_json::Value) -> FileEvent {
        FileEvent::EventsAppended {
            loop_id: "loop-1".into(),
            source: PathBuf::from("loop-1/.ralph/events-loop-1.jsonl"),
            new_events: vec![RalphEvent {
                timestamp: DateTime::parse_from_rfc3339("2026-03-31T20:00:00+00:00").unwrap(),
                topic: topic.to_string(),
                payload,
                iteration: Some(1),
                hat: Some("builder".into()),
                triggered: None,
            }],
        }
    }

    // AC1: requirements.complete → StageCompleted { RequirementsAnalysis }
    #[test]
    fn requirements_complete_maps_to_stage_completed() {
        let mut agg = Aggregator::new();
        let result = agg
            .apply(make_events_appended("requirements.complete"))
            .pipeline_event;
        assert_eq!(
            result,
            Some(PipelineEvent::StageCompleted {
                stage: StageId::RequirementsAnalysis
            })
        );
    }

    #[test]
    fn plan_complete_maps_to_stage_completed() {
        let mut agg = Aggregator::new();
        let result = agg
            .apply(make_events_appended("plan.complete"))
            .pipeline_event;
        assert_eq!(
            result,
            Some(PipelineEvent::StageCompleted {
                stage: StageId::Plan
            })
        );
    }

    #[test]
    fn plan_done_maps_to_stage_completed() {
        let mut agg = Aggregator::new();
        let result = agg.apply(make_events_appended("plan.done")).pipeline_event;
        assert_eq!(
            result,
            Some(PipelineEvent::StageCompleted {
                stage: StageId::Plan
            })
        );
    }

    // AC2: track.backend.artifact.api-contract → ArtifactVersioned
    #[test]
    fn track_artifact_maps_to_artifact_versioned() {
        let mut agg = Aggregator::new();
        let payload = serde_json::json!({"version": 1, "content_hash": "abc123"});
        let result = agg.apply(make_events_appended_with_payload(
            "track.backend.artifact.api-contract",
            payload,
        ));
        assert_eq!(
            result.pipeline_event,
            Some(PipelineEvent::ArtifactVersioned {
                track: "backend".into(),
                artifact: "api-contract".into(),
                version: 1,
                content_hash: "abc123".into(),
            })
        );
    }

    // AC3: track.backend.artifact.api-contract with null payload → defaults
    #[test]
    fn track_artifact_with_null_payload_defaults_gracefully() {
        let mut agg = Aggregator::new();
        let result = agg.apply(make_events_appended_with_payload(
            "track.backend.artifact.api-contract",
            serde_json::Value::Null,
        ));
        assert_eq!(
            result.pipeline_event,
            Some(PipelineEvent::ArtifactVersioned {
                track: "backend".into(),
                artifact: "api-contract".into(),
                version: 0,
                content_hash: "".into(),
            })
        );
    }

    // AC4: track.backend.failed → TrackFailed
    #[test]
    fn track_failed_maps_to_track_failed() {
        let mut agg = Aggregator::new();
        let payload = serde_json::json!({"error": "Build failed"});
        let result = agg.apply(make_events_appended_with_payload(
            "track.backend.failed",
            payload,
        ));
        assert_eq!(
            result.pipeline_event,
            Some(PipelineEvent::TrackFailed {
                track: "backend".into(),
                error: "Build failed".into(),
            })
        );
    }

    // AC5: track.security.failed with null payload → defaults
    #[test]
    fn track_failed_with_null_payload_defaults_gracefully() {
        let mut agg = Aggregator::new();
        let result = agg.apply(make_events_appended_with_payload(
            "track.security.failed",
            serde_json::Value::Null,
        ));
        assert_eq!(
            result.pipeline_event,
            Some(PipelineEvent::TrackFailed {
                track: "security".into(),
                error: "".into(),
            })
        );
    }

    // --- latest_ralph_status tests ---

    #[test]
    fn latest_ralph_status_returns_none_when_no_events() {
        let agg = Aggregator::new();
        assert_eq!(agg.latest_ralph_status(), None);
    }

    #[test]
    fn latest_ralph_status_returns_formatted_string_after_events() {
        let mut agg = Aggregator::new();
        agg.apply(FileEvent::EventsAppended {
            loop_id: "loop-1".into(),
            source: PathBuf::from("loop-1/.ralph/events-loop-1.jsonl"),
            new_events: vec![RalphEvent {
                timestamp: DateTime::parse_from_rfc3339("2026-03-31T20:00:00+00:00").unwrap(),
                topic: "design.start".into(),
                payload: serde_json::Value::Null,
                iteration: Some(3),
                hat: Some("builder".into()),
                triggered: None,
            }],
        });
        let status = agg.latest_ralph_status().unwrap();
        assert!(status.contains("Iteration 3"));
        assert!(status.contains("builder"));
        assert!(status.contains("design.start"));
    }

    #[test]
    fn latest_ralph_status_picks_most_recent_loop() {
        let mut agg = Aggregator::new();
        let ts1 = DateTime::parse_from_rfc3339("2026-03-31T20:00:00+00:00").unwrap();
        let ts2 = DateTime::parse_from_rfc3339("2026-03-31T21:00:00+00:00").unwrap();
        agg.apply(FileEvent::EventsAppended {
            loop_id: "loop-old".into(),
            source: PathBuf::from("loop-old/.ralph/events-loop-old.jsonl"),
            new_events: vec![RalphEvent {
                timestamp: ts1,
                topic: "old.topic".into(),
                payload: serde_json::Value::Null,
                iteration: Some(1),
                hat: Some("planner".into()),
                triggered: None,
            }],
        });
        agg.apply(FileEvent::EventsAppended {
            loop_id: "loop-new".into(),
            source: PathBuf::from("loop-new/.ralph/events-loop-new.jsonl"),
            new_events: vec![RalphEvent {
                timestamp: ts2,
                topic: "new.topic".into(),
                payload: serde_json::Value::Null,
                iteration: Some(5),
                hat: Some("builder".into()),
                triggered: None,
            }],
        });
        let status = agg.latest_ralph_status().unwrap();
        assert!(status.contains("Iteration 5"));
        assert!(status.contains("builder"));
        assert!(status.contains("new.topic"));
    }

    // Regression: stale task_count when tasks removed from a loop
    #[test]
    fn tasks_changed_zeros_stale_counts() {
        let mut agg = Aggregator::new();
        // First update: loop-1 has 2 tasks
        agg.apply(FileEvent::TasksChanged {
            tasks: vec![
                RalphTask {
                    id: "t1".into(),
                    title: "Task 1".into(),
                    description: None,
                    key: None,
                    status: "open".into(),
                    priority: None,
                    blocked_by: vec![],
                    loop_id: Some("loop-1".into()),
                    created: None,
                    started: None,
                    closed: None,
                },
                RalphTask {
                    id: "t2".into(),
                    title: "Task 2".into(),
                    description: None,
                    key: None,
                    status: "open".into(),
                    priority: None,
                    blocked_by: vec![],
                    loop_id: Some("loop-1".into()),
                    created: None,
                    started: None,
                    closed: None,
                },
            ],
        });
        assert_eq!(agg.loops["loop-1"].task_count, 2);

        // Second update: loop-1 has NO tasks (all removed)
        agg.apply(FileEvent::TasksChanged { tasks: vec![] });
        assert_eq!(agg.loops["loop-1"].task_count, 0);
    }

    // Step 3b: track.{id}.setup.complete → TrackSetupComplete
    #[test]
    fn track_setup_complete_maps_to_track_setup_complete() {
        let mut agg = Aggregator::new();
        let result = agg.apply(make_events_appended("track.backend.setup.complete"));
        assert_eq!(
            result.pipeline_event,
            Some(PipelineEvent::TrackSetupComplete {
                track: "backend".into()
            })
        );
    }

    #[test]
    fn track_setup_complete_with_different_id() {
        let mut agg = Aggregator::new();
        let result = agg.apply(make_events_appended("track.infra.setup.complete"));
        assert_eq!(
            result.pipeline_event,
            Some(PipelineEvent::TrackSetupComplete {
                track: "infra".into()
            })
        );
    }

    #[test]
    fn clear_resets_loop_state_fields() {
        let mut agg = Aggregator::new();
        // Populate some state
        agg.apply(make_events_appended("scan.complete"));
        assert!(agg.loops.get("loop-1").is_some());
        let state = &agg.loops["loop-1"];
        assert!(state.latest_hat.is_some());
        assert!(state.latest_topic.is_some());
        assert!(state.latest_iteration.is_some());

        // Clear should zero the loop state fields
        agg.clear();

        let state = &agg.loops["loop-1"];
        assert_eq!(state.latest_hat, None);
        assert_eq!(state.latest_topic, None);
        assert_eq!(state.latest_iteration, None);
    }

    // --- Hang detection tests ---

    #[test]
    fn last_event_time_updates_on_file_event() {
        let mut agg = Aggregator::new();
        assert!(agg.last_event_time.is_none());
        agg.apply(make_events_appended("scan.complete"));
        assert!(agg.last_event_time.is_some());
    }

    #[test]
    fn last_event_time_advances_on_subsequent_events() {
        let mut agg = Aggregator::new();
        agg.apply(make_events_appended("scan.complete"));
        let t1 = agg.last_event_time.unwrap();
        // Apply another event — time should be >= t1
        agg.apply(make_events_appended("design.start"));
        let t2 = agg.last_event_time.unwrap();
        assert!(t2 >= t1);
    }

    #[test]
    fn last_event_time_updates_on_tasks_changed() {
        let mut agg = Aggregator::new();
        agg.apply(FileEvent::TasksChanged { tasks: vec![] });
        assert!(agg.last_event_time.is_some());
    }

    #[test]
    fn last_event_time_updates_on_artifact_created() {
        let mut agg = Aggregator::new();
        agg.apply(FileEvent::ArtifactCreated {
            path: PathBuf::from(".loopy/artifacts/backend/api.json"),
        });
        assert!(agg.last_event_time.is_some());
    }

    // --- Step 9: Aggregator save_state / load_state ---

    #[test]
    fn aggregator_save_load_round_trip() {
        let mut agg = Aggregator::new();
        // Populate some loop state
        agg.apply(FileEvent::EventsAppended {
            loop_id: "loop-1".into(),
            source: PathBuf::from("loop-1/.ralph/events-loop-1.jsonl"),
            new_events: vec![RalphEvent {
                timestamp: DateTime::parse_from_rfc3339("2026-03-31T20:00:00+00:00").unwrap(),
                topic: "scan.complete".into(),
                payload: serde_json::Value::Null,
                iteration: Some(3),
                hat: Some("builder".into()),
                triggered: None,
            }],
        });
        assert_eq!(agg.loops.len(), 1);

        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("aggregator-state.json");
        agg.save_state(&path).unwrap();

        let mut agg2 = Aggregator::new();
        agg2.load_state(&path).unwrap();
        assert_eq!(agg2.loops.len(), 1);
        let state = &agg2.loops["loop-1"];
        assert_eq!(state.latest_hat, Some("builder".into()));
        assert_eq!(state.latest_topic, Some("scan.complete".into()));
        assert_eq!(state.latest_iteration, Some(3));
    }

    #[test]
    fn aggregator_load_state_missing_file_errors() {
        let mut agg = Aggregator::new();
        let result = agg.load_state(Path::new("/nonexistent/aggregator-state.json"));
        assert!(result.is_err());
    }

    #[test]
    fn aggregator_save_state_creates_parent_dirs() {
        let agg = Aggregator::new();
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir
            .path()
            .join("nested")
            .join("dir")
            .join("aggregator-state.json");
        agg.save_state(&path).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn aggregator_event_creates_log_entry() {
        let mut agg = Aggregator::new();
        let result = agg.apply(FileEvent::EventsAppended {
            loop_id: "loop-1".into(),
            source: PathBuf::from("loop-1/.ralph/events-loop-1.jsonl"),
            new_events: vec![RalphEvent {
                timestamp: DateTime::parse_from_rfc3339("2026-04-01T10:30:00+00:00").unwrap(),
                topic: "build.done".into(),
                payload: serde_json::Value::Null,
                iteration: Some(2),
                hat: Some("builder".into()),
                triggered: None,
            }],
        });
        assert_eq!(result.log_entries.len(), 1);
        let entry = &result.log_entries[0];
        assert!(entry.message.contains("builder"), "should contain hat");
        assert!(entry.message.contains("iter 2"), "should contain iteration");
        assert!(entry.message.contains("build.done"), "should contain topic");
        assert_eq!(entry.level, crate::models::LogLevel::Info);
    }
}
