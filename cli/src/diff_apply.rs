// Copyright 2025 The Jujutsu Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Minimal unified diff parser and applicator for non-interactive split.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum DiffApplyError {
    #[error("Failed to parse diff: {0}")]
    Parse(String),
    #[error("Failed to apply hunk for `{path}` at line {line}: {reason}")]
    Apply {
        path: String,
        line: usize,
        reason: String,
    },
}

pub struct FileDiff {
    pub path: String,
    pub hunks: Vec<Hunk>,
    pub is_new: bool,
    pub is_deleted: bool,
}

pub struct Hunk {
    pub old_start: usize,
    pub old_count: usize,
    pub lines: Vec<HunkLine>,
}

pub enum HunkLine {
    Context(String),
    Added(String),
    Removed(String),
}

pub fn parse_unified_diff(input: &str) -> Result<Vec<FileDiff>, DiffApplyError> {
    let mut files = Vec::new();
    let mut lines = input.lines().peekable();

    while let Some(&line) = lines.peek() {
        if !line.starts_with("diff --git ") {
            lines.next();
            continue;
        }
        lines.next();

        let mut old_path = None;
        let mut new_path = None;
        let mut is_new = false;
        let mut is_deleted = false;

        while let Some(&line) = lines.peek() {
            if let Some(path) = line.strip_prefix("--- ") {
                if path == "/dev/null" {
                    is_new = true;
                } else {
                    old_path = Some(path.strip_prefix("a/").unwrap_or(path).to_string());
                }
                lines.next();
            } else if let Some(path) = line.strip_prefix("+++ ") {
                if path == "/dev/null" {
                    is_deleted = true;
                } else {
                    new_path = Some(path.strip_prefix("b/").unwrap_or(path).to_string());
                }
                lines.next();
                break;
            } else if line.starts_with("@@ ") || line.starts_with("diff --git ") {
                break;
            } else {
                lines.next();
            }
        }

        let path = new_path.or(old_path).ok_or_else(|| {
            DiffApplyError::Parse("Missing file path in diff header".to_string())
        })?;

        let mut hunks = Vec::new();
        while let Some(&line) = lines.peek() {
            if line.starts_with("diff --git ") {
                break;
            }
            if !line.starts_with("@@ ") {
                lines.next();
                continue;
            }
            let (old_start, old_count) = parse_hunk_header(line)?;
            lines.next();

            let mut hunk_lines = Vec::new();
            while let Some(&line) = lines.peek() {
                if line.starts_with("@@ ") || line.starts_with("diff --git ") {
                    break;
                }
                if let Some(content) = line.strip_prefix('+') {
                    hunk_lines.push(HunkLine::Added(content.to_string()));
                } else if let Some(content) = line.strip_prefix('-') {
                    hunk_lines.push(HunkLine::Removed(content.to_string()));
                } else if let Some(content) = line.strip_prefix(' ') {
                    hunk_lines.push(HunkLine::Context(content.to_string()));
                } else if line.starts_with('\\') {
                    // "\ No newline at end of file"
                } else {
                    hunk_lines.push(HunkLine::Context(line.to_string()));
                }
                lines.next();
            }

            hunks.push(Hunk {
                old_start,
                old_count,
                lines: hunk_lines,
            });
        }

        files.push(FileDiff {
            path,
            hunks,
            is_new,
            is_deleted,
        });
    }

    Ok(files)
}

fn parse_hunk_header(line: &str) -> Result<(usize, usize), DiffApplyError> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 3 || parts[0] != "@@" {
        return Err(DiffApplyError::Parse(format!(
            "Invalid hunk header: {line}"
        )));
    }
    let old_range = parts[1].strip_prefix('-').ok_or_else(|| {
        DiffApplyError::Parse(format!("Invalid old range in hunk header: {line}"))
    })?;
    let (start, count) = if let Some((s, c)) = old_range.split_once(',') {
        (parse_usize(s)?, parse_usize(c)?)
    } else {
        (parse_usize(old_range)?, 1)
    };
    Ok((start, count))
}

fn parse_usize(s: &str) -> Result<usize, DiffApplyError> {
    s.parse()
        .map_err(|_| DiffApplyError::Parse(format!("Invalid number: {s}")))
}

pub fn apply_hunks(
    path: &str,
    old_content: &str,
    hunks: &[Hunk],
) -> Result<String, DiffApplyError> {
    let old_lines: Vec<&str> = if old_content.is_empty() {
        Vec::new()
    } else {
        old_content.lines().collect()
    };
    let mut result = Vec::new();
    let mut old_pos: usize = 0;

    for hunk in hunks {
        let hunk_start = if hunk.old_start == 0 {
            0
        } else {
            hunk.old_start - 1
        };

        if hunk_start < old_pos {
            return Err(DiffApplyError::Apply {
                path: path.to_string(),
                line: hunk.old_start,
                reason: "overlapping hunks".to_string(),
            });
        }

        while old_pos < hunk_start {
            if old_pos >= old_lines.len() {
                return Err(DiffApplyError::Apply {
                    path: path.to_string(),
                    line: hunk.old_start,
                    reason: format!(
                        "hunk starts at line {} but file only has {} lines",
                        hunk.old_start,
                        old_lines.len()
                    ),
                });
            }
            result.push(old_lines[old_pos].to_string());
            old_pos += 1;
        }

        for line in &hunk.lines {
            match line {
                HunkLine::Context(_) | HunkLine::Removed(_) => {
                    old_pos += 1;
                }
                HunkLine::Added(_) => {}
            }
            match line {
                HunkLine::Context(text) | HunkLine::Added(text) => {
                    result.push(text.clone());
                }
                HunkLine::Removed(_) => {}
            }
        }
    }

    while old_pos < old_lines.len() {
        result.push(old_lines[old_pos].to_string());
        old_pos += 1;
    }

    let mut output = result.join("\n");
    if !output.is_empty() && (old_content.ends_with('\n') || old_content.is_empty()) {
        output.push('\n');
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_diff() {
        let diff = "\
diff --git a/file.rs b/file.rs
index abc..def 100644
--- a/file.rs
+++ b/file.rs
@@ -1,3 +1,3 @@
 line 1
-line 2
+line 2 modified
 line 3
";
        let files = parse_unified_diff(diff).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "file.rs");
        assert_eq!(files[0].hunks.len(), 1);
        assert_eq!(files[0].hunks[0].old_start, 1);
        assert_eq!(files[0].hunks[0].old_count, 3);
    }

    #[test]
    fn test_parse_new_file() {
        let diff = "\
diff --git a/new.txt b/new.txt
new file mode 100644
--- /dev/null
+++ b/new.txt
@@ -0,0 +1,2 @@
+hello
+world
";
        let files = parse_unified_diff(diff).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].is_new);
        assert_eq!(files[0].path, "new.txt");
    }

    #[test]
    fn test_parse_deleted_file() {
        let diff = "\
diff --git a/old.txt b/old.txt
deleted file mode 100644
--- a/old.txt
+++ /dev/null
@@ -1,2 +0,0 @@
-hello
-world
";
        let files = parse_unified_diff(diff).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].is_deleted);
        assert_eq!(files[0].path, "old.txt");
    }

    #[test]
    fn test_parse_multiple_files() {
        let diff = "\
diff --git a/a.txt b/a.txt
--- a/a.txt
+++ b/a.txt
@@ -1,1 +1,1 @@
-old
+new
diff --git a/b.txt b/b.txt
--- a/b.txt
+++ b/b.txt
@@ -1,1 +1,1 @@
-foo
+bar
";
        let files = parse_unified_diff(diff).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "a.txt");
        assert_eq!(files[1].path, "b.txt");
    }

    #[test]
    fn test_apply_simple_modification() {
        let old = "line 1\nline 2\nline 3\n";
        let hunks = vec![Hunk {
            old_start: 2,
            old_count: 1,
            lines: vec![
                HunkLine::Removed("line 2".to_string()),
                HunkLine::Added("line 2 modified".to_string()),
            ],
        }];
        let result = apply_hunks("test", old, &hunks).unwrap();
        assert_eq!(result, "line 1\nline 2 modified\nline 3\n");
    }

    #[test]
    fn test_apply_addition() {
        let old = "line 1\nline 3\n";
        let hunks = vec![Hunk {
            old_start: 1,
            old_count: 2,
            lines: vec![
                HunkLine::Context("line 1".to_string()),
                HunkLine::Added("line 2".to_string()),
                HunkLine::Context("line 3".to_string()),
            ],
        }];
        let result = apply_hunks("test", old, &hunks).unwrap();
        assert_eq!(result, "line 1\nline 2\nline 3\n");
    }

    #[test]
    fn test_apply_new_file() {
        let hunks = vec![Hunk {
            old_start: 0,
            old_count: 0,
            lines: vec![
                HunkLine::Added("hello".to_string()),
                HunkLine::Added("world".to_string()),
            ],
        }];
        let result = apply_hunks("test", "", &hunks).unwrap();
        assert_eq!(result, "hello\nworld\n");
    }

    #[test]
    fn test_apply_multiple_hunks() {
        let old = "a\nb\nc\nd\ne\nf\n";
        let hunks = vec![
            Hunk {
                old_start: 2,
                old_count: 1,
                lines: vec![
                    HunkLine::Removed("b".to_string()),
                    HunkLine::Added("B".to_string()),
                ],
            },
            Hunk {
                old_start: 5,
                old_count: 1,
                lines: vec![
                    HunkLine::Removed("e".to_string()),
                    HunkLine::Added("E".to_string()),
                ],
            },
        ];
        let result = apply_hunks("test", old, &hunks).unwrap();
        assert_eq!(result, "a\nB\nc\nd\nE\nf\n");
    }

    #[test]
    fn test_roundtrip_parse_and_apply() {
        let old = "line 1\nline 2\nline 3\nline 4\n";
        let diff = "\
diff --git a/file.txt b/file.txt
--- a/file.txt
+++ b/file.txt
@@ -2,2 +2,2 @@
-line 2
-line 3
+LINE 2
+LINE 3
";
        let files = parse_unified_diff(diff).unwrap();
        let result = apply_hunks("file.txt", old, &files[0].hunks).unwrap();
        assert_eq!(result, "line 1\nLINE 2\nLINE 3\nline 4\n");
    }
}
