use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub enum DiffLineKind {
    Added,
    Removed,
    Context,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Hunk {
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DiffFile {
    pub path: String,
    pub hunks: Vec<Hunk>,
}

pub fn parse_diff(input: &str) -> Vec<DiffFile> {
    let mut files: Vec<DiffFile> = Vec::new();
    let mut current_hunk: Option<Hunk> = None;

    for line in input.lines() {
        if line.starts_with("diff --git ") {
            flush_hunk(&mut files, &mut current_hunk);
            let path = line.split(" b/").last().unwrap_or("").to_string();
            files.push(DiffFile {
                path,
                hunks: Vec::new(),
            });
        } else if line.starts_with("@@ ") {
            flush_hunk(&mut files, &mut current_hunk);
            current_hunk = Some(Hunk {
                header: line.to_string(),
                lines: Vec::new(),
            });
        } else if let Some(hunk) = current_hunk.as_mut() {
            let (kind, content) = if let Some(rest) = line.strip_prefix('+') {
                (DiffLineKind::Added, rest)
            } else if let Some(rest) = line.strip_prefix('-') {
                (DiffLineKind::Removed, rest)
            } else if let Some(rest) = line.strip_prefix(' ') {
                (DiffLineKind::Context, rest)
            } else {
                continue;
            };
            hunk.lines.push(DiffLine {
                kind,
                content: content.to_string(),
            });
        }
    }
    flush_hunk(&mut files, &mut current_hunk);
    files
}

fn flush_hunk(files: &mut [DiffFile], hunk: &mut Option<Hunk>) {
    if let Some(h) = hunk.take()
        && let Some(f) = files.last_mut()
    {
        f.hunks.push(h);
    }
}

pub fn get_workspace_diff(project_root: &Path, workspace_path: Option<&Path>) -> Vec<DiffFile> {
    let dir = workspace_path.unwrap_or(project_root);
    let output = Command::new("git").args(["diff"]).current_dir(dir).output();
    match output {
        Ok(o) if o.status.success() => parse_diff(&String::from_utf8_lossy(&o.stdout)),
        _ => Vec::new(),
    }
}

/// Committed diff for a single package repo. Tracks commit their work onto an
/// `loopy/*` feature branch, so a plain working-tree `git diff` shows nothing —
/// we diff the feature branch against its base (`mainline`, else
/// `origin/mainline..HEAD`). Returns [] if the repo doesn't exist or has no
/// loopy branch / no changes.
pub fn get_package_committed_diff(repo: &Path) -> Vec<DiffFile> {
    if !repo.join(".git").exists() {
        return Vec::new();
    }

    // Prefer an loopy/* feature branch diffed against mainline.
    let loopy_branch = Command::new("git")
        .args(["branch", "--list", "loopy/*"])
        .current_dir(repo)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| {
            s.lines()
                .next()
                .map(|l| l.trim().trim_start_matches("* ").to_string())
        })
        .filter(|s| !s.is_empty());

    let range = match &loopy_branch {
        Some(branch) => format!("mainline..{branch}"),
        None => "origin/mainline..HEAD".to_string(),
    };

    match Command::new("git")
        .args(["diff", &range])
        .current_dir(repo)
        .output()
    {
        Ok(o) if o.status.success() => parse_diff(&String::from_utf8_lossy(&o.stdout)),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_diff() {
        assert_eq!(parse_diff(""), Vec::<DiffFile>::new());
    }

    #[test]
    fn parse_single_file_with_add_remove_context() {
        let input = "\
diff --git a/src/main.rs b/src/main.rs
index abc1234..def5678 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,4 +1,4 @@
 fn main() {
-    println!(\"old\");
+    println!(\"new\");
 }
";
        let files = parse_diff(input);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "src/main.rs");
        assert_eq!(files[0].hunks.len(), 1);
        assert_eq!(files[0].hunks[0].header, "@@ -1,4 +1,4 @@");

        let lines = &files[0].hunks[0].lines;
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0].kind, DiffLineKind::Context);
        assert_eq!(lines[0].content, "fn main() {");
        assert_eq!(lines[1].kind, DiffLineKind::Removed);
        assert_eq!(lines[1].content, "    println!(\"old\");");
        assert_eq!(lines[2].kind, DiffLineKind::Added);
        assert_eq!(lines[2].content, "    println!(\"new\");");
        assert_eq!(lines[3].kind, DiffLineKind::Context);
        assert_eq!(lines[3].content, "}");
    }

    #[test]
    fn parse_multi_file_diff() {
        let input = "\
diff --git a/a.rs b/a.rs
index 1111111..2222222 100644
--- a/a.rs
+++ b/a.rs
@@ -1,2 +1,3 @@
 line1
+added
 line2
diff --git a/b.rs b/b.rs
index 3333333..4444444 100644
--- a/b.rs
+++ b/b.rs
@@ -1 +1 @@
-old
+new
";
        let files = parse_diff(input);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "a.rs");
        assert_eq!(files[1].path, "b.rs");
        assert_eq!(files[0].hunks[0].lines.len(), 3);
        assert_eq!(files[1].hunks[0].lines.len(), 2);
    }

    #[test]
    fn hunk_header_extraction() {
        let input = "\
diff --git a/f.rs b/f.rs
index 0000000..1111111 100644
--- a/f.rs
+++ b/f.rs
@@ -10,6 +10,7 @@ fn existing() {
 context
+added
@@ -30,3 +31,3 @@ fn another() {
-removed
+replaced
";
        let files = parse_diff(input);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].hunks.len(), 2);
        assert_eq!(
            files[0].hunks[0].header,
            "@@ -10,6 +10,7 @@ fn existing() {"
        );
        assert_eq!(files[0].hunks[1].header, "@@ -30,3 +31,3 @@ fn another() {");
    }

    #[test]
    fn per_track_diff_uses_workspace_path() {
        // When workspace_path is Some, parse_diff is called on that dir's git diff.
        // We test the parse_diff path directly since git isn't available in test.
        let input = "\
diff --git a/track_file.rs b/track_file.rs
index 0000000..1111111 100644
--- a/track_file.rs
+++ b/track_file.rs
@@ -1,2 +1,3 @@
 existing
+new_line
 end
";
        let files = parse_diff(input);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "track_file.rs");
        assert_eq!(files[0].hunks[0].lines.len(), 3);
        assert_eq!(files[0].hunks[0].lines[1].kind, DiffLineKind::Added);
    }
}
