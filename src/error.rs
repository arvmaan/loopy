use thiserror::Error;

#[derive(Debug, Error)]
pub enum LoopyError {
    #[error("environment error: {0}")]
    EnvironmentError(String),
    #[error("spawn error: {0}")]
    SpawnError(String),
    #[error("watcher error: {0}")]
    WatcherError(String),
    #[error("checkpoint error: {0}")]
    CheckpointError(#[from] std::io::Error),
    #[error("parse error: {0}")]
    ParseError(String),
    #[error("manifest error: {0}")]
    ManifestError(String),
    #[error("pipeline error: {0}")]
    PipelineError(String),
    #[error("credential error: {0}")]
    CredentialError(String),
    #[error("artifact error: {0}")]
    ArtifactError(String),
    #[error("conflict check error: {0}")]
    ConflictCheckError(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_messages() {
        let cases: Vec<(LoopyError, &str)> = vec![
            (
                LoopyError::CredentialError("missing token".into()),
                "credential error: missing token",
            ),
            (
                LoopyError::ArtifactError("hash mismatch".into()),
                "artifact error: hash mismatch",
            ),
            (
                LoopyError::ConflictCheckError("llm timeout".into()),
                "conflict check error: llm timeout",
            ),
        ];
        for (err, expected) in cases {
            assert_eq!(err.to_string(), expected);
        }
    }
}
