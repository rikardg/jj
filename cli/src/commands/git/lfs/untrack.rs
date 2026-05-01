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

use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::ui::Ui;

/// Stop tracking file patterns with Git LFS
///
/// Removes matching entries from `.gitattributes`. Files that were
/// previously stored as LFS pointers will be stored as regular content
/// on the next snapshot.
///
/// Example:
///
///     jj git lfs untrack "*.bin"
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct GitLfsUntrackArgs {
    /// Patterns to stop tracking with LFS (gitattributes syntax)
    #[arg(required = true, value_name = "PATTERNS")]
    patterns: Vec<String>,
}

#[instrument(skip_all)]
pub(crate) async fn cmd_git_lfs_untrack(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &GitLfsUntrackArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper(ui)?;
    let workspace_root = workspace_command.workspace_root().to_owned();
    drop(workspace_command);

    let gitattributes_path = workspace_root.join(".gitattributes");
    let existing = match std::fs::read_to_string(&gitattributes_path) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            writeln!(ui.status(), "No .gitattributes file found")?;
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };

    let patterns_set: std::collections::HashSet<&str> =
        args.patterns.iter().map(String::as_str).collect();

    let mut removed = Vec::new();
    let lines: Vec<&str> = existing
        .lines()
        .filter(|line| {
            let first_token = line.split_whitespace().next().unwrap_or("");
            if patterns_set.contains(first_token) && line.contains("filter=lfs") {
                removed.push(first_token.to_string());
                false
            } else {
                true
            }
        })
        .collect();

    if removed.is_empty() {
        for pattern in &args.patterns {
            writeln!(ui.status(), "Pattern \"{pattern}\" is not tracked by LFS")?;
        }
    } else {
        let mut content = lines.join("\n");
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        std::fs::write(&gitattributes_path, content)?;
        for pattern in &removed {
            writeln!(ui.status(), "Untracking \"{pattern}\" from LFS")?;
        }
    }

    Ok(())
}
