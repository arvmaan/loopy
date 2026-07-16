//! Deterministic per-track status inspector.
//!
//! Reads a track's on-disk artifacts (`.ralph/agent/tasks.jsonl`, the newest
//! `events-*.jsonl`, and the raw `ralph-output.log`) and derives an
//! interpretable status WITHOUT an LLM: current task, phase, iteration/budget,
//! and — crucially — whether the loop has reached a terminal state.
//!
//! This serves two purposes:
//!   1. Drives the friendly per-track activity shown in the UI.
//!   2. Detects track completion/failure so the engine can advance past
//!      `running_tracks` (Ralph signals completion via a generic `loop.terminate`
//!      / "Primary loop landed successfully", NOT a `track.<id>.complete` topic,
//!      so the runner cannot rely on the aggregator alone).

use crate::watcher;
use std::path::Path;

/// Terminal disposition of a track's Ralph loop, derived from its log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// Still running (or not yet started).
    Active,
    /// Loop landed successfully — completion promise satisfied.
    Completed,
    /// Loop stopped without landing (hit max iterations / errored out).
    Failed,
}

/// A structured, human-interpretable snapshot of a track's progress.
#[derive(Debug, Clone)]
pub struct TrackInspection {
    pub disposition: Disposition,
    pub tasks_done: u32,
    pub tasks_total: u32,
    /// Short phase label: "building", "reviewing", "landed", etc.
    pub phase: String,
    /// The current/most-recent task description, if known.
    pub task: Option<String>,
    /// "iter 24/200" if the log exposes it.
    pub iteration: Option<String>,
    /// Budget percentage from the iteration marker, e.g. 12.
    pub budget_pct: Option<u32>,
    /// One-line interpretable activity summary for the UI.
    pub activity: String,
}

/// Inspect a single track's work directory.
pub fn inspect(work_dir: &Path) -> TrackInspection {
    let ralph_dir = work_dir.join(".ralph");

    // --- Task counts -------------------------------------------------------
    let tasks =
        watcher::read_all_tasks(&ralph_dir.join("agent/tasks.jsonl")).unwrap_or_default();
    let tasks_total = tasks.len() as u32;
    let tasks_done = tasks.iter().filter(|t| t.status == "closed").count() as u32;
    // Current task = first non-closed, else the most recently touched.
    let task = tasks
        .iter()
        .find(|t| t.status == "in_progress")
        .or_else(|| tasks.iter().find(|t| t.status == "open"))
        .map(|t| t.title.clone());

    // --- Raw log tail: disposition + iteration/budget + phase --------------
    let log_path = ralph_dir.join("ralph-output.log");
    let tail = watcher::read_last_lines(&log_path, 400).unwrap_or_default();

    let disposition = derive_disposition(&tail);
    let (iteration, budget_pct) = parse_iter_marker(&tail);
    let phase = derive_phase(disposition, &ralph_dir);

    let activity = build_activity(disposition, &phase, task.as_deref(), &iteration, budget_pct);

    TrackInspection {
        disposition,
        tasks_done,
        tasks_total,
        phase,
        task,
        iteration,
        budget_pct,
        activity,
    }
}

/// Determine terminal state from the log tail.
///
/// "Primary loop landed successfully" is the authoritative completion marker
/// (emitted exactly once when the completion promise is satisfied and changes
/// are committed). A loop that stopped at max iterations without landing is a
/// failure.
fn derive_disposition(tail: &[String]) -> Disposition {
    let mut landed = false;
    let mut max_iter = false;
    for line in tail {
        if line.contains("Primary loop landed successfully")
            || line.contains("Landing completed with auto-commit")
        {
            landed = true;
        }
        if line.contains("reason=max_iterations") || line.contains("Maximum iterations reached") {
            max_iter = true;
        }
    }
    if landed {
        Disposition::Completed
    } else if max_iter {
        Disposition::Failed
    } else {
        Disposition::Active
    }
}

/// Parse the most recent `[iter N/M done] ... budget=P%` marker.
fn parse_iter_marker(tail: &[String]) -> (Option<String>, Option<u32>) {
    let mut iteration = None;
    let mut budget = None;
    for line in tail {
        let Some(start) = line.find("[iter ") else {
            continue;
        };
        let rest = &line[start + 6..];
        if let Some(done_pos) = rest.find(" done]") {
            // "N/M"
            iteration = Some(format!("iter {}", &rest[..done_pos]));
        }
        if let Some(bpos) = line.find("budget=") {
            let num: String = line[bpos + 7..]
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if let Ok(n) = num.parse::<u32>() {
                budget = Some(n);
            }
        }
    }
    (iteration, budget)
}

/// Derive a short phase label from disposition + the latest event topic.
fn derive_phase(disposition: Disposition, ralph_dir: &Path) -> String {
    match disposition {
        Disposition::Completed => return "landed".into(),
        Disposition::Failed => return "stopped".into(),
        Disposition::Active => {}
    }
    match latest_event_topic(ralph_dir).as_deref() {
        Some("build.start") => "building".into(),
        Some("build.done") => "built".into(),
        Some("review.ready") | Some("review.passed") => "reviewing".into(),
        Some("review.rejected") => "fixing review".into(),
        Some("tasks.ready") => "planning tasks".into(),
        Some("queue.advance") => "advancing".into(),
        Some(other) => other.replace('.', " "),
        None => "working".into(),
    }
}

/// Read the most-recently-modified events-*.jsonl file's latest topic.
fn latest_event_topic(ralph_dir: &Path) -> Option<String> {
    let mut newest: Option<(std::time::SystemTime, std::path::PathBuf)> = None;
    for entry in std::fs::read_dir(ralph_dir).ok()?.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with("events-") && name.ends_with(".jsonl")) {
            continue;
        }
        if let Ok(mtime) = entry.metadata().and_then(|m| m.modified()) {
            if newest.as_ref().map(|(t, _)| mtime > *t).unwrap_or(true) {
                newest = Some((mtime, entry.path()));
            }
        }
    }
    let (_, path) = newest?;
    let content = std::fs::read_to_string(&path).ok()?;
    watcher::parse_events(&content).last().map(|e| e.topic.clone())
}

/// Compose the one-line activity summary shown in the UI.
fn build_activity(
    disposition: Disposition,
    phase: &str,
    task: Option<&str>,
    iteration: &Option<String>,
    budget_pct: Option<u32>,
) -> String {
    match disposition {
        Disposition::Completed => "completed · landed".into(),
        Disposition::Failed => "stopped without landing".into(),
        Disposition::Active => {
            let mut parts = vec![phase.to_string()];
            if let Some(t) = task {
                parts.push(format!("· {}", truncate(t, 60)));
            }
            if let Some(it) = iteration {
                let mut tail = it.clone();
                if let Some(b) = budget_pct {
                    tail.push_str(&format!(" · {b}% budget"));
                }
                parts.push(format!("· {tail}"));
            }
            parts.join(" ")
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{t}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disposition_landed() {
        let tail = vec![
            "some work".to_string(),
            "INFO ralph::loop_runner: Primary loop landed successfully committed=true".to_string(),
        ];
        assert_eq!(derive_disposition(&tail), Disposition::Completed);
    }

    #[test]
    fn disposition_max_iter_is_failure() {
        let tail = vec!["Wrapping up: reason=max_iterations iterations=200".to_string()];
        assert_eq!(derive_disposition(&tail), Disposition::Failed);
    }

    #[test]
    fn disposition_active_when_no_markers() {
        let tail = vec!["[iter 4/200 done] dur=2m total=10m budget=2%".to_string()];
        assert_eq!(derive_disposition(&tail), Disposition::Active);
    }

    #[test]
    fn parses_iter_and_budget() {
        let tail = vec!["[iter 24/200 done] dur=4m 15s total=1h 36m budget=12%".to_string()];
        let (iter, budget) = parse_iter_marker(&tail);
        assert_eq!(iter.as_deref(), Some("iter 24/200"));
        assert_eq!(budget, Some(12));
    }

    #[test]
    fn parses_latest_iter_when_multiple() {
        let tail = vec![
            "[iter 1/200 done] budget=1%".to_string(),
            "[iter 5/200 done] budget=8%".to_string(),
        ];
        let (iter, budget) = parse_iter_marker(&tail);
        assert_eq!(iter.as_deref(), Some("iter 5/200"));
        assert_eq!(budget, Some(8));
    }

    #[test]
    fn activity_for_completed_track() {
        let a = build_activity(Disposition::Completed, "landed", None, &None, None);
        assert_eq!(a, "completed · landed");
    }

    #[test]
    fn activity_for_active_track_includes_task_and_iter() {
        let a = build_activity(
            Disposition::Active,
            "building",
            Some("Add GuardrailService dependency"),
            &Some("iter 8/200".to_string()),
            Some(4),
        );
        assert!(a.contains("building"));
        assert!(a.contains("GuardrailService"));
        assert!(a.contains("iter 8/200"));
        assert!(a.contains("4% budget"));
    }
}
