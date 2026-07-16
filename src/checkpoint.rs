use crate::error::LoopyError;
use crate::models::PipelineState;
use std::path::Path;

/// Load pipeline state from a checkpoint file.
/// Returns `Ok(None)` if the file doesn't exist, is corrupt, or has version < 2.
pub fn load(path: &Path) -> Result<Option<PipelineState>, LoopyError> {
    let data = match std::fs::read_to_string(path) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(LoopyError::CheckpointError(e)),
    };
    match serde_json::from_str::<PipelineState>(&data) {
        Ok(state) if state.version < 2 => Ok(None),
        Ok(state) => Ok(Some(state)),
        Err(e) => {
            eprintln!("checkpoint parse error: {e}");
            Ok(None)
        } // corrupt file treated as fresh
    }
}

/// Save pipeline state atomically via temp file + rename.
pub fn save(path: &Path, state: &PipelineState) -> Result<(), LoopyError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let data =
        serde_json::to_string_pretty(state).map_err(|e| LoopyError::ParseError(e.to_string()))?;
    std::fs::write(&tmp, &data)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::*;
    use chrono::Utc;
    use std::fs;
    use tempfile::TempDir;

    fn sample_state() -> PipelineState {
        PipelineState {
            version: 2,
            idea_text: "test idea".into(),
            stages: vec![StageState {
                id: StageId::Idea,
                status: StageStatus::Complete,
                loop_id: None,
                loop_pid: None,
                started_at: None,
                completed_at: None,
                error: None,
            }],
            tracks: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            execution_mode: ExecutionMode::Safe,
            artifact_registry: ArtifactRegistry::default(),
            active_conflicts: vec![],
            awaiting_approval: None,
        }
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".loopy").join("state.json");
        let state = sample_state();
        save(&path, &state).unwrap();
        let loaded = load(&path).unwrap().expect("should load state");
        assert_eq!(state, loaded);
    }

    #[test]
    fn load_missing_file_returns_none() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.json");
        let result = load(&path).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn load_corrupt_file_returns_none() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.json");
        fs::write(&path, "not valid json {{{").unwrap();
        let result = load(&path).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn load_v1_checkpoint_returns_none() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.json");
        // Write a valid v1 checkpoint
        let mut state = sample_state();
        state.version = 1;
        let data = serde_json::to_string_pretty(&state).unwrap();
        fs::write(&path, &data).unwrap();
        // v1 should be treated as incompatible → None
        let result = load(&path).unwrap();
        assert!(
            result.is_none(),
            "v1 checkpoint should return None (fresh start)"
        );
    }

    #[test]
    fn save_uses_temp_file_atomically() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.json");
        let state = sample_state();
        save(&path, &state).unwrap();
        // After save, the .tmp file should NOT exist (renamed away)
        let tmp = path.with_extension("json.tmp");
        assert!(!tmp.exists());
        // The target file should exist
        assert!(path.exists());
    }
}
