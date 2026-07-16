use crate::models::{RalphEvent, RalphTask};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub enum FileEvent {
    EventsAppended {
        loop_id: String,
        new_events: Vec<RalphEvent>,
    },
    TasksChanged {
        tasks: Vec<RalphTask>,
    },
    ArtifactCreated {
        path: PathBuf,
    },
    StateJsonChanged {
        content: String,
    },
    LogLinesAppended {
        lines: Vec<String>,
    },
}

/// Parse a single JSONL line into a RalphEvent, returning None for corrupt lines.
pub fn parse_event_line(line: &str) -> Option<RalphEvent> {
    serde_json::from_str(line).ok()
}

/// Parse a single JSONL line into a RalphTask, returning None for corrupt lines.
pub fn parse_task_line(line: &str) -> Option<RalphTask> {
    serde_json::from_str(line).ok()
}

/// Parse all valid events from JSONL content, skipping corrupt lines.
pub fn parse_events(content: &str) -> Vec<RalphEvent> {
    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(parse_event_line)
        .collect()
}

/// Parse all valid tasks from JSONL content, skipping corrupt lines.
pub fn parse_tasks(content: &str) -> Vec<RalphTask> {
    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(parse_task_line)
        .collect()
}

/// Read new bytes from a file starting at `offset`, parse events, return new offset.
pub fn read_new_events(path: &Path, offset: u64) -> std::io::Result<(Vec<RalphEvent>, u64)> {
    let mut file = std::fs::File::open(path)?;
    let metadata = file.metadata()?;
    let file_len = metadata.len();
    if file_len <= offset {
        return Ok((vec![], offset));
    }
    file.seek(SeekFrom::Start(offset))?;
    let mut events = Vec::new();
    let reader = BufReader::new(&file);
    let mut new_offset = offset;
    for line in reader.lines() {
        let line = line?;
        new_offset += line.len() as u64 + 1; // +1 for newline
        if line.trim().is_empty() {
            continue;
        }
        if let Some(event) = parse_event_line(&line) {
            events.push(event);
        }
    }
    Ok((events, new_offset))
}

/// Read and parse the entire tasks file (full re-parse).
pub fn read_all_tasks(path: &Path) -> std::io::Result<Vec<RalphTask>> {
    let content = std::fs::read_to_string(path)?;
    Ok(parse_tasks(&content))
}

/// Read new lines from a log file starting at `offset`, return lines and new offset.
pub fn read_new_log_lines(path: &Path, offset: u64) -> std::io::Result<(Vec<String>, u64)> {
    let mut file = std::fs::File::open(path)?;
    let file_len = file.metadata()?.len();
    if file_len <= offset {
        return Ok((vec![], offset));
    }
    file.seek(SeekFrom::Start(offset))?;
    let reader = BufReader::new(&file);
    let mut lines = Vec::new();
    let mut new_offset = offset;
    for line in reader.lines() {
        let line = line?;
        new_offset += line.len() as u64 + 1;
        if !line.is_empty() {
            lines.push(line);
        }
    }
    Ok((lines, new_offset))
}

/// Read approximately the last `max_lines` lines of a (possibly large) file
/// without loading the whole thing. Reads a bounded tail window from the end.
pub fn read_last_lines(path: &Path, max_lines: usize) -> std::io::Result<Vec<String>> {
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    // Read at most the last 256 KiB — plenty for max_lines of log output.
    let window: u64 = (256 * 1024).min(len);
    file.seek(SeekFrom::Start(len - window))?;
    let reader = BufReader::new(&file);
    let mut lines: Vec<String> = reader.lines().map_while(Result::ok).collect();
    // If we started mid-file, the first (partial) line is unreliable — drop it.
    if window < len && !lines.is_empty() {
        lines.remove(0);
    }
    let start = lines.len().saturating_sub(max_lines);
    Ok(lines.split_off(start))
}

/// Extract loop_id from an events filename like `events-{loop-id}.jsonl`.
pub fn extract_loop_id(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    stem.strip_prefix("events-").map(|s| s.to_string())
}

pub struct WatcherHandle {
    watcher: RecommendedWatcher,
}

impl WatcherHandle {
    /// Add a path to the watcher (recursive).
    pub fn watch_path(&mut self, path: &Path) -> Result<(), crate::error::LoopyError> {
        self.watcher
            .watch(path, RecursiveMode::Recursive)
            .map_err(|e| crate::error::LoopyError::WatcherError(e.to_string()))
    }

    /// Remove a path from the watcher.
    pub fn unwatch_path(&mut self, path: &Path) -> Result<(), crate::error::LoopyError> {
        self.watcher
            .unwatch(path)
            .map_err(|e| crate::error::LoopyError::WatcherError(e.to_string()))
    }
}

/// Start watching the given directory for JSONL file changes.
/// Emits FileEvents through the provided channel.
pub async fn start(
    watch_dir: PathBuf,
    tx: mpsc::Sender<FileEvent>,
) -> Result<WatcherHandle, crate::error::LoopyError> {
    let offsets: std::sync::Arc<std::sync::Mutex<HashMap<PathBuf, u64>>> =
        std::sync::Arc::new(std::sync::Mutex::new(HashMap::new()));

    let offsets_clone = offsets.clone();
    let tx_clone = tx.clone();

    let mut watcher = RecommendedWatcher::new(
        move |res: Result<Event, notify::Error>| {
            let Ok(event) = res else { return };
            match event.kind {
                EventKind::Modify(_) | EventKind::Create(_) => {
                    for path in &event.paths {
                        let Some(fname) = path.file_name().and_then(|f| f.to_str()) else {
                            continue;
                        };

                        if fname == "tasks.jsonl" {
                            if let Ok(tasks) = read_all_tasks(path) {
                                let _ = tx_clone.try_send(FileEvent::TasksChanged { tasks });
                            }
                        } else if fname == "state.json" {
                            if let Ok(content) = std::fs::read_to_string(path) {
                                let _ =
                                    tx_clone.try_send(FileEvent::StateJsonChanged { content });
                            }
                        } else if fname == "ralph-output.log" {
                            let mut map = offsets_clone.lock().unwrap();
                            let offset = map.get(path).copied().unwrap_or(0);
                            if let Ok((lines, new_offset)) = read_new_log_lines(path, offset) {
                                map.insert(path.clone(), new_offset);
                                if !lines.is_empty() {
                                    let _ = tx_clone
                                        .try_send(FileEvent::LogLinesAppended { lines });
                                }
                            }
                        } else if fname.starts_with("events-") && fname.ends_with(".jsonl") {
                            let mut map = offsets_clone.lock().unwrap();
                            let offset = map.get(path).copied().unwrap_or(0);
                            if let Ok((events, new_offset)) = read_new_events(path, offset) {
                                map.insert(path.clone(), new_offset);
                                if !events.is_empty()
                                    && let Some(loop_id) = extract_loop_id(path)
                                {
                                    let _ = tx_clone.try_send(FileEvent::EventsAppended {
                                        loop_id,
                                        new_events: events,
                                    });
                                }
                            }
                        } else if matches!(event.kind, EventKind::Create(_)) {
                            let _ = tx_clone
                                .try_send(FileEvent::ArtifactCreated { path: path.clone() });
                        }
                    }
                }
                _ => {}
            }
        },
        notify::Config::default(),
    )
    .map_err(|e| crate::error::LoopyError::WatcherError(e.to_string()))?;

    watcher
        .watch(&watch_dir, RecursiveMode::Recursive)
        .map_err(|e| crate::error::LoopyError::WatcherError(e.to_string()))?;

    Ok(WatcherHandle { watcher })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    const VALID_EVENT: &str = r#"{"ts":"2026-03-31T19:40:40.977787752+00:00","topic":"design.start","payload":"hello","iteration":0,"hat":"loop","triggered":"planner"}"#;
    const VALID_EVENT_2: &str = r#"{"ts":"2026-03-31T20:00:00+00:00","topic":"answer.proposed","payload":{"summary":"test"}}"#;
    const CORRUPT_LINE: &str = r#"{"ts":"bad json"#;
    const VALID_TASK: &str = r#"{"id":"task-123","title":"Test","status":"open","blocked_by":[]}"#;

    // AC1: Parse valid event JSONL
    #[test]
    fn parse_valid_event_line() {
        let event = parse_event_line(VALID_EVENT).unwrap();
        assert_eq!(event.topic, "design.start");
        assert_eq!(event.payload, serde_json::Value::String("hello".into()));
        assert_eq!(event.triggered, Some("planner".into()));
    }

    // AC2: Skip corrupt JSONL line
    #[test]
    fn skip_corrupt_line_in_events() {
        let content = format!("{}\n{}\n{}\n", VALID_EVENT, CORRUPT_LINE, VALID_EVENT_2);
        let events = parse_events(&content);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].topic, "design.start");
        assert_eq!(events[1].topic, "answer.proposed");
    }

    // AC3: Byte-offset tailing — only new lines parsed
    #[test]
    fn byte_offset_tailing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("events-scan1.jsonl");

        // Write 3 initial lines
        {
            let mut f = std::fs::File::create(&path).unwrap();
            for _ in 0..3 {
                writeln!(f, "{}", VALID_EVENT).unwrap();
            }
        }

        // Read all — establishes offset
        let (events, offset) = read_new_events(&path, 0).unwrap();
        assert_eq!(events.len(), 3);
        assert!(offset > 0);

        // Append 2 new lines
        {
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            writeln!(f, "{}", VALID_EVENT_2).unwrap();
            writeln!(f, "{}", VALID_EVENT_2).unwrap();
        }

        // Read from saved offset — only 2 new
        let (new_events, _new_offset) = read_new_events(&path, offset).unwrap();
        assert_eq!(new_events.len(), 2);
        assert_eq!(new_events[0].topic, "answer.proposed");
    }

    // AC4: Full re-parse for tasks
    #[test]
    fn full_reparse_tasks() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("tasks.jsonl");

        // Write initial content
        std::fs::write(&path, format!("{}\n", VALID_TASK)).unwrap();
        let tasks = read_all_tasks(&path).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, "task-123");

        // Rewrite with different content
        let new_task = r#"{"id":"task-456","title":"New","status":"closed","blocked_by":[]}"#;
        std::fs::write(&path, format!("{}\n{}\n", new_task, new_task)).unwrap();
        let tasks = read_all_tasks(&path).unwrap();
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].id, "task-456");
    }

    // AC5: Empty file handling
    #[test]
    fn empty_file_no_events() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("events-empty.jsonl");
        std::fs::write(&path, "").unwrap();

        let (events, offset) = read_new_events(&path, 0).unwrap();
        assert!(events.is_empty());
        assert_eq!(offset, 0);
    }

    #[test]
    fn empty_file_no_tasks() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("tasks.jsonl");
        std::fs::write(&path, "").unwrap();

        let tasks = read_all_tasks(&path).unwrap();
        assert!(tasks.is_empty());
    }

    // AC6: Channel emission via integration test
    #[tokio::test]
    async fn watcher_emits_events_on_file_change() {
        let dir = TempDir::new().unwrap();
        let events_path = dir.path().join("events-loop1.jsonl");
        std::fs::write(&events_path, "").unwrap();

        let (tx, mut rx) = mpsc::channel(32);
        let _handle = start(dir.path().to_path_buf(), tx).await.unwrap();

        // Give watcher time to register
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Append an event
        {
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&events_path)
                .unwrap();
            writeln!(f, "{}", VALID_EVENT).unwrap();
        }

        // Wait for notification
        let file_event = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("timeout waiting for event")
            .expect("channel closed");

        match file_event {
            FileEvent::EventsAppended {
                loop_id,
                new_events,
            } => {
                assert_eq!(loop_id, "loop1");
                assert_eq!(new_events.len(), 1);
                assert_eq!(new_events[0].topic, "design.start");
            }
            other => panic!("expected EventsAppended, got {:?}", other),
        }
    }

    // AC6 continued: tasks channel emission
    #[tokio::test]
    async fn watcher_emits_tasks_on_file_change() {
        let dir = TempDir::new().unwrap();
        let tasks_path = dir.path().join("tasks.jsonl");
        std::fs::write(&tasks_path, "").unwrap();

        let (tx, mut rx) = mpsc::channel(32);
        let _handle = start(dir.path().to_path_buf(), tx).await.unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Rewrite tasks file
        std::fs::write(&tasks_path, format!("{}\n", VALID_TASK)).unwrap();

        let file_event = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("timeout waiting for event")
            .expect("channel closed");

        match file_event {
            FileEvent::TasksChanged { tasks } => {
                assert_eq!(tasks.len(), 1);
                assert_eq!(tasks[0].id, "task-123");
            }
            other => panic!("expected TasksChanged, got {:?}", other),
        }
    }

    #[test]
    fn extract_loop_id_from_filename() {
        let path = PathBuf::from("/some/dir/events-scan-loop-1.jsonl");
        assert_eq!(extract_loop_id(&path), Some("scan-loop-1".to_string()));
    }

    #[test]
    fn extract_loop_id_non_events_file() {
        let path = PathBuf::from("/some/dir/tasks.jsonl");
        assert_eq!(extract_loop_id(&path), None);
    }

    // AC: watch_path adds a new directory and receives events from it
    #[tokio::test]
    async fn watch_path_receives_events_from_new_dir() {
        let dir1 = TempDir::new().unwrap();
        let dir2 = TempDir::new().unwrap();

        // Pre-create events file in dir2
        let events_path = dir2.path().join("events-newloop.jsonl");
        std::fs::write(&events_path, "").unwrap();

        let (tx, mut rx) = mpsc::channel(32);
        let mut handle = start(dir1.path().to_path_buf(), tx).await.unwrap();

        // Add dir2 dynamically
        handle.watch_path(dir2.path()).unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Write to dir2 — should be detected
        {
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&events_path)
                .unwrap();
            writeln!(f, "{}", VALID_EVENT).unwrap();
        }

        let file_event = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("timeout waiting for event")
            .expect("channel closed");

        match file_event {
            FileEvent::EventsAppended {
                loop_id,
                new_events,
            } => {
                assert_eq!(loop_id, "newloop");
                assert_eq!(new_events.len(), 1);
            }
            other => panic!("expected EventsAppended, got {:?}", other),
        }
    }

    // AC: unwatch_path stops receiving events from a directory
    // Reproduces the real Loopy scenario: watch stages/, create scan/.ralph/, write events
    #[tokio::test]
    async fn watcher_detects_events_in_nested_ralph_dir() {
        let dir = TempDir::new().unwrap();
        let stages = dir.path().join("stages");
        std::fs::create_dir_all(&stages).unwrap();

        let (tx, mut rx) = mpsc::channel(32);
        let mut handle = start(stages.clone(), tx).await.unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Simulate loopy creating scan stage dir with .ralph subdir
        let ralph_dir = stages.join("scan").join(".ralph");
        std::fs::create_dir_all(&ralph_dir).unwrap();

        // KEY FIX: explicitly watch the new .ralph dir (macOS FSEvents doesn't auto-detect new subdirs)
        handle.watch_path(&ralph_dir).unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Write events file (like Ralph does)
        let events_file = ralph_dir.join("events-test123.jsonl");
        std::fs::write(&events_file, "").unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Append an event
        {
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&events_file)
                .unwrap();
            writeln!(f, "{}", VALID_EVENT).unwrap();
        }

        // Drain any ArtifactCreated events first
        let mut got_events_appended = false;
        for _ in 0..10 {
            match tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv()).await {
                Ok(Some(FileEvent::EventsAppended {
                    loop_id,
                    new_events,
                })) => {
                    assert_eq!(loop_id, "test123");
                    assert_eq!(new_events.len(), 1);
                    got_events_appended = true;
                    break;
                }
                Ok(Some(_)) => continue, // skip ArtifactCreated etc
                _ => break,
            }
        }
        assert!(
            got_events_appended,
            "watcher must detect events in nested .ralph dir"
        );
    }

    #[tokio::test]
    async fn unwatch_path_stops_events() {
        let dir = TempDir::new().unwrap();
        let events_path = dir.path().join("events-loop1.jsonl");
        std::fs::write(&events_path, "").unwrap();

        let (tx, mut rx) = mpsc::channel(32);
        let mut handle = start(dir.path().to_path_buf(), tx).await.unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Unwatch the directory
        handle.unwatch_path(dir.path()).unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Write — should NOT be detected
        {
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&events_path)
                .unwrap();
            writeln!(f, "{}", VALID_EVENT).unwrap();
        }

        let result = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await;
        assert!(result.is_err(), "should not receive events after unwatch");
    }

    #[test]
    fn read_new_log_lines_tails_from_offset() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("ralph-output.log");
        std::fs::write(&path, "line1\nline2\nline3\n").unwrap();

        let (lines, offset) = read_new_log_lines(&path, 0).unwrap();
        assert_eq!(lines, vec!["line1", "line2", "line3"]);
        assert!(offset > 0);

        // Append more
        {
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            writeln!(f, "line4").unwrap();
        }
        let (new_lines, _) = read_new_log_lines(&path, offset).unwrap();
        assert_eq!(new_lines, vec!["line4"]);
    }

    #[test]
    fn read_new_log_lines_empty_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("ralph-output.log");
        std::fs::write(&path, "").unwrap();
        let (lines, offset) = read_new_log_lines(&path, 0).unwrap();
        assert!(lines.is_empty());
        assert_eq!(offset, 0);
    }

    #[tokio::test]
    async fn watcher_emits_state_json_changed() {
        let dir = TempDir::new().unwrap();
        let state_path = dir.path().join("state.json");
        std::fs::write(&state_path, "{}").unwrap();

        let (tx, mut rx) = mpsc::channel(32);
        let _handle = start(dir.path().to_path_buf(), tx).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        std::fs::write(&state_path, r#"{"version":1}"#).unwrap();

        let mut got_state = false;
        for _ in 0..10 {
            match tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv()).await {
                Ok(Some(FileEvent::StateJsonChanged { content })) => {
                    assert!(content.contains("version"));
                    got_state = true;
                    break;
                }
                Ok(Some(_)) => continue,
                _ => break,
            }
        }
        assert!(
            got_state,
            "watcher must emit StateJsonChanged for state.json"
        );
    }

    #[tokio::test]
    async fn watcher_emits_log_lines_appended() {
        let dir = TempDir::new().unwrap();
        let log_path = dir.path().join("ralph-output.log");
        std::fs::write(&log_path, "").unwrap();

        let (tx, mut rx) = mpsc::channel(32);
        let _handle = start(dir.path().to_path_buf(), tx).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        {
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&log_path)
                .unwrap();
            writeln!(f, "hello world").unwrap();
        }

        let mut got_log = false;
        for _ in 0..10 {
            match tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv()).await {
                Ok(Some(FileEvent::LogLinesAppended { lines })) => {
                    assert_eq!(lines, vec!["hello world"]);
                    got_log = true;
                    break;
                }
                Ok(Some(_)) => continue,
                _ => break,
            }
        }
        assert!(
            got_log,
            "watcher must emit LogLinesAppended for ralph-output.log"
        );
    }
}
