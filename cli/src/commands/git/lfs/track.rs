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
    let workspace_command = command.workspace_helper(ui)?;
    let workspace_root = workspace_command.workspace_root().to_owned();
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
    }

    Ok(())
}

fn matches_pattern(file_name: &str, pattern: &str) -> bool {
    if let Some(ext) = pattern.strip_prefix("*.") {
        file_name
            .rsplit_once('.')
            .is_some_and(|(_, file_ext)| file_ext == ext)
    } else {
        file_name == pattern
    }
}

fn touch_matching_files(workspace_root: &Path, patterns: &[&str]) -> Result<u64, CommandError> {
    let now = SystemTime::now();
    let mut count = 0u64;
    let mut dirs = vec![workspace_root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str == ".jj" || name_str == ".git" {
                continue;
            }
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                dirs.push(entry.path());
            } else if file_type.is_file()
                && patterns.iter().any(|p| matches_pattern(&name_str, p))
            {
                let file = std::fs::File::options().write(true).open(entry.path())?;
                file.set_modified(now)?;
                count += 1;
            }
        }
    }
    Ok(count)
}
