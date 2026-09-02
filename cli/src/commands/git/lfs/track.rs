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

use std::io::Write as _;
use std::path::Path;
use std::time::SystemTime;

use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::ui::Ui;

/// Start tracking file patterns with Git LFS
///
/// Adds entries to `.gitattributes` so that matching files are stored
/// using Git LFS. Existing files that match the new patterns will be
/// converted to LFS on the next snapshot (any jj command that reads
/// the working copy).
///
/// Patterns use gitattributes syntax (e.g. `*.bin`, `assets/**`).
///
/// Example:
///
///     jj git lfs track "*.bin" "*.dat"
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct GitLfsTrackArgs {
    /// Patterns to track with LFS (gitattributes syntax)
    #[arg(required = true, value_name = "PATTERNS")]
    patterns: Vec<String>,
}

#[instrument(skip_all)]
pub(crate) async fn cmd_git_lfs_track(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &GitLfsTrackArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper(ui).await?;
    let workspace_root = workspace_command.workspace_root().to_owned();
    let repo_path = workspace_command.repo_path().to_owned();
    drop(workspace_command);

    let gitattributes_path = workspace_root.join(".gitattributes");
    let existing = std::fs::read_to_string(&gitattributes_path).unwrap_or_default();

    let mut lines: Vec<String> = existing.lines().map(String::from).collect();
    let mut added = Vec::new();

    for pattern in &args.patterns {
        if pattern.is_empty() {
            return Err(user_error("Empty pattern is not allowed"));
        }
        let attr_line = format!("{pattern} filter=lfs diff=lfs merge=lfs -text");
        let already_tracked = lines.iter().any(|line| {
            line.split_whitespace().next() == Some(pattern.as_str())
                && line.contains("filter=lfs")
        });
        if already_tracked {
            writeln!(ui.status(), "Pattern \"{pattern}\" is already tracked by LFS")?;
        } else {
            lines.push(attr_line);
            added.push(pattern.as_str());
        }
    }

    if !added.is_empty() {
        let mut content = lines.join("\n");
        if !content.ends_with('\n') {
            content.push('\n');
        }
        std::fs::write(&gitattributes_path, content)?;
        for pattern in &added {
            writeln!(ui.status(), "Tracking \"{pattern}\" with LFS")?;
        }

        let touched = touch_matching_files(&workspace_root, &added)?;
        if touched > 0 {
            writeln!(
                ui.status(),
                "Touched {touched} existing file(s) for LFS conversion on next snapshot"
            )?;
        }
        ensure_lfs_enabled(&repo_path)?;
    }

    Ok(())
}

fn ensure_lfs_enabled(repo_path: &Path) -> Result<(), CommandError> {
    let config_path = repo_path.join("config.toml");
    let content = std::fs::read_to_string(&config_path).unwrap_or_default();
    if content.contains("git.lfs") || content.contains("[git]") && content.contains("lfs") {
        return Ok(());
    }
    let mut doc: toml_edit::DocumentMut = content.parse().unwrap_or_default();
    doc["git"]["lfs"] = toml_edit::value(true);
    std::fs::write(&config_path, doc.to_string())?;
    Ok(())
}

fn touch_matching_files(root: &Path, patterns: &[&str]) -> Result<u64, CommandError> {
    let matchers: Vec<_> = patterns
        .iter()
        .filter_map(|p| {
            globset::Glob::new(p)
                .ok()
                .map(|g| (p.contains('/'), g.compile_matcher()))
        })
        .collect();
    if matchers.is_empty() {
        return Ok(0);
    }
    let mut count = 0u64;
    walk_and_touch(root, root, &matchers, &mut count);
    Ok(count)
}

fn walk_and_touch(
    root: &Path,
    dir: &Path,
    matchers: &[(bool, globset::GlobMatcher)],
    count: &mut u64,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().unwrap_or_default();
        if name == ".jj" || name == ".git" {
            continue;
        }
        if path.is_dir() {
            walk_and_touch(root, &path, matchers, count);
        } else if path.is_file() {
            let matches = matchers.iter().any(|(has_slash, matcher)| {
                if *has_slash {
                    path.strip_prefix(root)
                        .is_ok_and(|rel| matcher.is_match(rel))
                } else {
                    matcher.is_match(name)
                }
            });
            if matches {
                let times = std::fs::FileTimes::new().set_modified(now);
                if let Ok(f) = std::fs::File::options().write(true).open(&path) {
                    drop(f.set_times(times));
                }
                *count += 1;
            }
        }
    }
}
