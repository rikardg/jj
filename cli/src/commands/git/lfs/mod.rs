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

mod track;
mod untrack;

use clap::Subcommand;

use self::track::GitLfsTrackArgs;
use self::track::cmd_git_lfs_track;
use self::untrack::GitLfsUntrackArgs;
use self::untrack::cmd_git_lfs_untrack;
use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::ui::Ui;

/// Manage Git LFS tracking patterns
///
/// These commands modify `.gitattributes` to control which files are
/// stored using Git LFS.
#[derive(Subcommand, Clone, Debug)]
pub enum LfsCommand {
    Track(GitLfsTrackArgs),
    Untrack(GitLfsUntrackArgs),
}

pub async fn cmd_git_lfs(
    ui: &mut Ui,
    command: &CommandHelper,
    subcommand: &LfsCommand,
) -> Result<(), CommandError> {
    match subcommand {
        LfsCommand::Track(args) => cmd_git_lfs_track(ui, command, args).await,
        LfsCommand::Untrack(args) => cmd_git_lfs_untrack(ui, command, args).await,
    }
}
