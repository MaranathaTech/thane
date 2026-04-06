use std::path::{Path, PathBuf};
use std::process::Command;

/// Git repository state for a workspace.
#[derive(Debug, Clone, Default)]
pub struct GitInfo {
    /// Path to the git repository root.
    pub repo_root: Option<PathBuf>,
    /// Current branch name.
    pub branch: Option<String>,
    /// Whether the working tree has uncommitted changes.
    pub dirty: bool,
}

impl GitInfo {
    /// Detect git info for the given working directory.
    pub fn detect(cwd: &Path) -> Self {
        let repo_root = detect_repo_root(cwd);
        let branch = repo_root.as_ref().and_then(|root| detect_branch(root));
        let dirty = repo_root
            .as_ref()
            .map(|root| detect_dirty(root))
            .unwrap_or(false);

        Self {
            repo_root,
            branch,
            dirty,
        }
    }
}

/// Find the git repository root by running `git rev-parse --show-toplevel`.
fn detect_repo_root(cwd: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(cwd)
        .output()
        .ok()?;

    if output.status.success() {
        let path = String::from_utf8(output.stdout).ok()?;
        Some(PathBuf::from(path.trim()))
    } else {
        None
    }
}

/// Detect the current git branch name.
fn detect_branch(repo_root: &Path) -> Option<String> {
    // Try reading .git/HEAD directly first (faster than spawning git).
    let head_path = repo_root.join(".git").join("HEAD");
    if let Ok(content) = std::fs::read_to_string(&head_path) {
        let content = content.trim();
        if let Some(branch) = content.strip_prefix("ref: refs/heads/") {
            return Some(branch.to_string());
        }
        // Detached HEAD — return short hash.
        if content.len() >= 8 {
            return Some(content[..8].to_string());
        }
    }

    // Fallback to git command.
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(repo_root)
        .output()
        .ok()?;

    if output.status.success() {
        let branch = String::from_utf8(output.stdout).ok()?;
        Some(branch.trim().to_string())
    } else {
        None
    }
}

/// Check if the working tree has uncommitted changes.
fn detect_dirty(repo_root: &Path) -> bool {
    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repo_root)
        .output();

    match output {
        Ok(out) => !out.stdout.is_empty(),
        Err(_) => false,
    }
}

/// A changed file in the git diff.
#[derive(Debug, Clone)]
pub struct DiffFile {
    /// The file path relative to the repo root.
    pub path: String,
    /// Status character from git status --porcelain (M, A, D, R, ?, etc.).
    pub status: char,
    /// Unified diff hunks for this file (empty if binary or new untracked file).
    pub hunks: Vec<DiffHunk>,
}

/// A single hunk from a unified diff.
#[derive(Debug, Clone)]
pub struct DiffHunk {
    /// The hunk header line (e.g. "@@ -1,5 +1,7 @@").
    pub header: String,
    /// Lines in this hunk with their type.
    pub lines: Vec<DiffLine>,
}

/// A single line in a diff hunk.
#[derive(Debug, Clone)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub content: String,
}

/// Type of a diff line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    Context,
    Added,
    Removed,
}

/// Get the list of changed files with their diffs for a working directory.
pub fn get_diff(cwd: &Path) -> Vec<DiffFile> {
    let mut files = Vec::new();

    // Get the list of changed files via git status --porcelain.
    let status_output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(cwd)
        .output();

    let status_text = match status_output {
        Ok(out) if out.status.success() => {
            String::from_utf8_lossy(&out.stdout).to_string()
        }
        _ => return files,
    };

    // Parse status lines to get file paths and statuses.
    let mut file_statuses: Vec<(char, String)> = Vec::new();
    for line in status_text.lines() {
        if line.len() < 4 {
            continue;
        }
        // Format: XY filename
        // X = index status, Y = working tree status
        let index_status = line.as_bytes()[0] as char;
        let wt_status = line.as_bytes()[1] as char;
        let path = line[3..].to_string();

        // Use the most significant status.
        let status = if index_status != ' ' && index_status != '?' {
            index_status
        } else {
            wt_status
        };

        file_statuses.push((status, path));
    }

    // Get unified diff for tracked files.
    let diff_output = Command::new("git")
        .args(["diff", "HEAD", "--unified=3"])
        .current_dir(cwd)
        .output();

    let diff_text = match diff_output {
        Ok(out) => String::from_utf8_lossy(&out.stdout).to_string(),
        Err(_) => String::new(),
    };

    // Parse the unified diff to extract per-file hunks.
    let parsed = parse_unified_diff(&diff_text);

    // Build the result, matching status info with diff hunks.
    for (status, path) in file_statuses {
        let hunks = parsed
            .iter()
            .find(|(p, _)| *p == path)
            .map(|(_, h)| h.clone())
            .unwrap_or_default();

        files.push(DiffFile {
            path,
            status,
            hunks,
        });
    }

    files
}

/// Parse a unified diff string into per-file hunk lists.
/// Returns Vec<(file_path, Vec<DiffHunk>)>.
fn parse_unified_diff(diff: &str) -> Vec<(String, Vec<DiffHunk>)> {
    let mut result: Vec<(String, Vec<DiffHunk>)> = Vec::new();
    let mut current_file: Option<String> = None;
    let mut current_hunks: Vec<DiffHunk> = Vec::new();
    let mut current_hunk: Option<DiffHunk> = None;

    for line in diff.lines() {
        if line.starts_with("diff --git") {
            // Flush previous file.
            if let Some(hunk) = current_hunk.take() {
                current_hunks.push(hunk);
            }
            if let Some(file) = current_file.take() {
                result.push((file, std::mem::take(&mut current_hunks)));
            }
        } else if let Some(path) = line.strip_prefix("+++ b/") {
            current_file = Some(path.to_string());
        } else if line.starts_with("@@") {
            // New hunk.
            if let Some(hunk) = current_hunk.take() {
                current_hunks.push(hunk);
            }
            current_hunk = Some(DiffHunk {
                header: line.to_string(),
                lines: Vec::new(),
            });
        } else if let Some(ref mut hunk) = current_hunk {
            if let Some(content) = line.strip_prefix('+') {
                hunk.lines.push(DiffLine {
                    kind: DiffLineKind::Added,
                    content: content.to_string(),
                });
            } else if let Some(content) = line.strip_prefix('-') {
                hunk.lines.push(DiffLine {
                    kind: DiffLineKind::Removed,
                    content: content.to_string(),
                });
            } else if let Some(content) = line.strip_prefix(' ') {
                hunk.lines.push(DiffLine {
                    kind: DiffLineKind::Context,
                    content: content.to_string(),
                });
            }
        }
    }

    // Flush the last file.
    if let Some(hunk) = current_hunk {
        current_hunks.push(hunk);
    }
    if let Some(file) = current_file {
        result.push((file, current_hunks));
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_unified_diff_single_file() {
        let diff = "\
diff --git a/foo.rs b/foo.rs
index 1234567..abcdefg 100644
--- a/foo.rs
+++ b/foo.rs
@@ -1,3 +1,4 @@
 fn main() {
+    println!(\"hello\");
     let x = 1;
 }";

        let result = parse_unified_diff(diff);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "foo.rs");
        assert_eq!(result[0].1.len(), 1);

        let hunk = &result[0].1[0];
        assert!(hunk.header.starts_with("@@"));
        assert_eq!(hunk.lines.len(), 4);
        assert_eq!(hunk.lines[0].kind, DiffLineKind::Context);
        assert_eq!(hunk.lines[1].kind, DiffLineKind::Added);
        assert_eq!(hunk.lines[1].content, "    println!(\"hello\");");
        assert_eq!(hunk.lines[2].kind, DiffLineKind::Context);
        assert_eq!(hunk.lines[3].kind, DiffLineKind::Context);
    }

    #[test]
    fn test_parse_unified_diff_multiple_files() {
        let diff = "\
diff --git a/a.rs b/a.rs
--- a/a.rs
+++ b/a.rs
@@ -1 +1 @@
-old
+new
diff --git a/b.rs b/b.rs
--- a/b.rs
+++ b/b.rs
@@ -1 +1,2 @@
 existing
+added";

        let result = parse_unified_diff(diff);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].0, "a.rs");
        assert_eq!(result[1].0, "b.rs");
        assert_eq!(result[0].1[0].lines.len(), 2); // -old +new
        assert_eq!(result[1].1[0].lines.len(), 2); // existing, +added
    }

    #[test]
    fn test_parse_unified_diff_empty() {
        let result = parse_unified_diff("");
        assert!(result.is_empty());
    }
}
