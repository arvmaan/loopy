use std::path::Path;
use std::process::Command;

use crate::models::{CrComment, PipelineState, TrackMedium};
use chrono::Utc;

/// Result of attempting pull request creation for a single track.
pub struct CrResult {
    pub track_id: String,
    pub cr_url: Option<String>,
    pub error: Option<String>,
}

/// Create pull requests for all Brazil tracks that have workspaces.
/// Returns a `CrResult` per track. Skips non-Brazil tracks.
pub fn create_cr_for_tracks(state: &PipelineState, project_root: &Path) -> Vec<CrResult> {
    let tracks = match &state.tracks {
        Some(t) => t,
        None => return Vec::new(),
    };
    tracks
        .iter()
        .filter(|t| t.medium == TrackMedium::Brazil)
        .map(|t| {
            let ws = project_root.join("workspaces").join(&t.id);
            if !ws.exists() {
                return CrResult {
                    track_id: t.id.clone(),
                    cr_url: None,
                    error: Some("workspace not found".into()),
                };
            }
            match run_cr_cli(&ws) {
                Ok(url) => CrResult {
                    track_id: t.id.clone(),
                    cr_url: Some(url),
                    error: None,
                },
                Err(e) => CrResult {
                    track_id: t.id.clone(),
                    cr_url: None,
                    error: Some(e),
                },
            }
        })
        .collect()
}

/// Run `gh pr create --fill` in the given workspace directory and extract the
/// pull request URL from output.
fn run_cr_cli(workspace: &Path) -> Result<String, String> {
    let output = Command::new("gh")
        .args(["pr", "create", "--fill"])
        .current_dir(workspace)
        .output()
        .map_err(|e| {
            format!("gh (GitHub CLI) not found; install from https://cli.github.com ({e})")
        })?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    // `gh pr create` prints the PR URL on success.
    extract_cr_url(&stdout)
        .or_else(|| extract_cr_url(&stderr))
        .ok_or_else(|| format!("no PR URL in output. stdout: {stdout}, stderr: {stderr}"))
}

/// Extract a GitHub pull request URL from text.
pub fn extract_cr_url(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        line.split_whitespace()
            .find(|w| w.contains("github.com/") && w.contains("/pull/"))
            .map(|w| {
                // Strip surrounding punctuation
                w.trim_matches(|c: char| !c.is_alphanumeric() && c != ':' && c != '/' && c != '-')
                    .to_string()
            })
    })
}

/// Fetch pull request comments by running `gh pr view <url> --comments`.
/// Parses output lines in the format: `file:line: [author] comment body`
pub fn fetch_cr_comments(cr_url: &str) -> Vec<CrComment> {
    let output = Command::new("gh")
        .args(["pr", "view", cr_url, "--comments"])
        .output();
    let output = match output {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };
    let text = String::from_utf8_lossy(&output.stdout);
    parse_cr_comment_output(&text)
}

/// Parse review comment output lines: `file:line: [author] body`
pub fn parse_cr_comment_output(text: &str) -> Vec<CrComment> {
    let now = Utc::now();
    text.lines()
        .filter_map(|line| {
            // Expected format: src/main.rs:42: [author] comment body
            let (file_line, rest) = line.split_once(": [")?;
            let (author, body) = rest.split_once("] ")?;
            let (file, line_str) = file_line.rsplit_once(':')?;
            let line_num = line_str.trim().parse::<usize>().ok()?;
            Some(CrComment {
                file: file.to_string(),
                line: line_num,
                author: author.to_string(),
                body: body.to_string(),
                timestamp: now,
            })
        })
        .collect()
}

/// Compile review comments into the same feedback format used by inline review comments.
pub fn compile_cr_feedback(comments: &[CrComment]) -> String {
    let mut out = String::from("PR reviewer feedback on your changes:\n");
    for c in comments {
        out.push_str(&format!(
            "\n## {}:{}\n> [{}]\nComment: {}\n",
            c.file, c.line, c.author, c.body
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::*;
    use chrono::Utc;

    fn minimal_state(tracks: Vec<TrackState>) -> PipelineState {
        PipelineState {
            version: 1,
            idea_text: "test".into(),
            stages: vec![],
            tracks: Some(tracks),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            execution_mode: ExecutionMode::Safe,
            artifact_registry: ArtifactRegistry::default(),
            active_conflicts: vec![],
            awaiting_approval: None,
        }
    }

    fn brazil_track(id: &str) -> TrackState {
        TrackState {
            id: id.into(),
            name: id.into(),
            status: TrackStatus::Complete,
            loop_id: None,
            loop_pid: None,
            current_sub_stage: None,
            depends_on: vec![],
            blocking_artifact: None,
            consumed_versions: vec![],
            run_count: 0,
            medium: TrackMedium::Brazil,
            review_status: ReviewStatus::PendingReview,
            cr_url: None,
        }
    }

    fn docs_track(id: &str) -> TrackState {
        TrackState {
            medium: TrackMedium::Docs,
            ..brazil_track(id)
        }
    }

    #[test]
    fn cr_url_serde_round_trip() {
        let mut t = brazil_track("backend");
        t.cr_url = Some("https://github.com/owner/repo/pull/123".into());
        let json = serde_json::to_string(&t).unwrap();
        assert!(json.contains("cr_url"));
        let back: TrackState = serde_json::from_str(&json).unwrap();
        assert_eq!(
            back.cr_url.as_deref(),
            Some("https://github.com/owner/repo/pull/123")
        );
    }

    #[test]
    fn cr_url_serde_none_omitted() {
        let t = brazil_track("backend");
        let json = serde_json::to_string(&t).unwrap();
        assert!(!json.contains("cr_url"));
        let back: TrackState = serde_json::from_str(&json).unwrap();
        assert_eq!(back.cr_url, None);
    }

    #[test]
    fn create_cr_skips_non_brazil_tracks() {
        let state = minimal_state(vec![docs_track("docs")]);
        let tmp = tempfile::tempdir().unwrap();
        let results = create_cr_for_tracks(&state, tmp.path());
        assert!(results.is_empty());
    }

    #[test]
    fn create_cr_skips_missing_workspace() {
        let state = minimal_state(vec![brazil_track("backend")]);
        let tmp = tempfile::tempdir().unwrap();
        let results = create_cr_for_tracks(&state, tmp.path());
        assert_eq!(results.len(), 1);
        assert!(results[0].cr_url.is_none());
        assert!(
            results[0]
                .error
                .as_ref()
                .unwrap()
                .contains("workspace not found")
        );
    }

    #[test]
    fn create_cr_graceful_when_gh_not_found() {
        // Create workspace dir but the gh CLI may not exist / may not have a repo here.
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("workspaces").join("backend");
        std::fs::create_dir_all(&ws).unwrap();
        let state = minimal_state(vec![brazil_track("backend")]);
        let results = create_cr_for_tracks(&state, tmp.path());
        assert_eq!(results.len(), 1);
        assert!(results[0].cr_url.is_none());
        // Should have an error (gh not found or no PR URL in output)
        assert!(results[0].error.is_some());
    }

    #[test]
    fn extract_cr_url_from_output() {
        let output = "Creating pull request...\nhttps://github.com/owner/repo/pull/123\nDone.";
        assert_eq!(
            extract_cr_url(output),
            Some("https://github.com/owner/repo/pull/123".into())
        );
    }

    #[test]
    fn extract_cr_url_none_when_absent() {
        assert_eq!(extract_cr_url("no url here"), None);
    }

    #[test]
    fn create_cr_no_tracks() {
        let state = minimal_state(vec![]);
        let tmp = tempfile::tempdir().unwrap();
        let results = create_cr_for_tracks(&state, tmp.path());
        assert!(results.is_empty());
    }

    #[test]
    fn parse_cr_comment_output_valid() {
        let text =
            "src/main.rs:42: [alice] Handle the error case\nsrc/lib.rs:10: [bob] Missing derive\n";
        let comments = parse_cr_comment_output(text);
        assert_eq!(comments.len(), 2);
        assert_eq!(comments[0].file, "src/main.rs");
        assert_eq!(comments[0].line, 42);
        assert_eq!(comments[0].author, "alice");
        assert_eq!(comments[0].body, "Handle the error case");
        assert_eq!(comments[1].file, "src/lib.rs");
        assert_eq!(comments[1].line, 10);
        assert_eq!(comments[1].author, "bob");
    }

    #[test]
    fn parse_cr_comment_output_skips_invalid() {
        let text = "not a comment line\nsrc/main.rs:42: [alice] Valid comment\ngarbage";
        let comments = parse_cr_comment_output(text);
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].author, "alice");
    }

    #[test]
    fn compile_cr_feedback_format() {
        let comments = vec![
            CrComment {
                file: "src/main.rs".into(),
                line: 42,
                author: "alice".into(),
                body: "Handle the error case".into(),
                timestamp: Utc::now(),
            },
            CrComment {
                file: "src/lib.rs".into(),
                line: 10,
                author: "bob".into(),
                body: "Missing derive".into(),
                timestamp: Utc::now(),
            },
        ];
        let feedback = compile_cr_feedback(&comments);
        assert!(feedback.starts_with("PR reviewer feedback"));
        assert!(feedback.contains("## src/main.rs:42"));
        assert!(feedback.contains("> [alice]"));
        assert!(feedback.contains("Comment: Handle the error case"));
        assert!(feedback.contains("## src/lib.rs:10"));
        assert!(feedback.contains("Comment: Missing derive"));
    }
}
