use std::fmt;
use std::path::{Path, PathBuf};

use super::changes::{RepoPath, changed_paths, likely_conflicts, merge_base};
use super::{Checkout, Repository, git_command};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitPreview {
    pub id: String,
    pub subject: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegrationAction {
    Merge,
    Rebase,
    CherryPick,
    LeaveAlone,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationPreview {
    pub source: Checkout,
    pub target: Checkout,
    pub commits: Vec<CommitPreview>,
    pub diff_summary: String,
    pub likely_conflicts: Vec<RepoPath>,
    pub source_dirty: bool,
    pub target_dirty: bool,
    source_signature: String,
    target_signature: String,
}

impl IntegrationPreview {
    pub fn blocker(&self, action: IntegrationAction) -> Option<String> {
        match action {
            IntegrationAction::Merge | IntegrationAction::CherryPick if self.target_dirty => {
                Some("Commit or stash target checkout changes before integration".to_owned())
            }
            IntegrationAction::Rebase if self.source_dirty => {
                Some("Commit or stash source checkout changes before rebasing".to_owned())
            }
            IntegrationAction::Merge | IntegrationAction::Rebase if self.commits.is_empty() => {
                Some("The source has no commits that are absent from the target".to_owned())
            }
            IntegrationAction::CherryPick if self.commits.is_empty() => {
                Some("There are no source commits to cherry-pick".to_owned())
            }
            IntegrationAction::LeaveAlone => None,
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationOutcome {
    pub action: IntegrationAction,
    pub checkout: PathBuf,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationError {
    pub message: String,
    pub checkout: PathBuf,
    pub guidance: String,
}

impl fmt::Display for IntegrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}. {}", self.message, self.guidance)
    }
}

impl std::error::Error for IntegrationError {}

impl Repository {
    pub fn preview_integration(
        &self,
        source: &Checkout,
        target: &Checkout,
    ) -> Result<IntegrationPreview, String> {
        let source = self.verify_checkout(source)?;
        let target = self.verify_checkout(target)?;
        if source.path == target.path {
            return Err("Choose different source and target checkouts".to_owned());
        }
        let base = merge_base(&self.root, &source.head, &target.head)?;
        let commits = commits(&target.path, &target.head, &source.head)?;
        let diff_summary = text_output(
            &self.root,
            &["diff", "--stat", "--summary", &base, &source.head, "--"],
        )?;
        let likely_conflicts = likely_conflicts(&self.root, &base, &target, &source)?
            .into_iter()
            .collect();
        let source_signature = status_signature(&source.path)?;
        let target_signature = status_signature(&target.path)?;
        let source_dirty = !changed_paths(&source.path, &source.head)?.is_empty();
        let target_dirty = !changed_paths(&target.path, &target.head)?.is_empty();
        Ok(IntegrationPreview {
            source,
            target,
            commits,
            diff_summary,
            likely_conflicts,
            source_dirty,
            target_dirty,
            source_signature,
            target_signature,
        })
    }

    pub fn integrate(
        &self,
        preview: &IntegrationPreview,
        action: IntegrationAction,
    ) -> Result<IntegrationOutcome, IntegrationError> {
        if action == IntegrationAction::LeaveAlone {
            return Ok(IntegrationOutcome {
                action,
                checkout: preview.target.path.clone(),
                message: "Left both checkouts unchanged".to_owned(),
            });
        }
        self.verify_preview(preview)?;
        if let Some(message) = preview.blocker(action) {
            return Err(IntegrationError {
                message,
                checkout: affected_checkout(preview, action).to_owned(),
                guidance: "Refresh the preview after preparing the checkout".to_owned(),
            });
        }

        let (checkout, args, success, recovery) = match action {
            IntegrationAction::Merge => (
                preview.target.path.as_path(),
                vec!["merge", "--no-edit", preview.source.head.as_str()],
                "Merged the source into the target checkout",
                "Resolve the files, then run `git merge --continue`, or run `git merge --abort`",
            ),
            IntegrationAction::Rebase => (
                preview.source.path.as_path(),
                vec!["rebase", preview.target.head.as_str()],
                "Rebased the source checkout; preview again before updating the target",
                "Resolve the files, then run `git rebase --continue`, or run `git rebase --abort`",
            ),
            IntegrationAction::CherryPick => {
                let mut args = vec!["cherry-pick"];
                args.extend(preview.commits.iter().map(|commit| commit.id.as_str()));
                (
                    preview.target.path.as_path(),
                    args,
                    "Cherry-picked the source commits into the target checkout",
                    "Resolve the files, then run `git cherry-pick --continue`, or run `git cherry-pick --abort`",
                )
            }
            IntegrationAction::LeaveAlone => unreachable!("handled before Git validation"),
        };
        let output = git_command(checkout, &args)
            .output()
            .map_err(|error| IntegrationError {
                message: format!("Cannot run Git: {error}"),
                checkout: checkout.to_owned(),
                guidance: "Inspect the checkout with `git status` and retry from a new preview"
                    .to_owned(),
            })?;
        if !output.status.success() {
            let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            return Err(IntegrationError {
                message: if detail.is_empty() {
                    format!("Git {:?} failed", action)
                } else {
                    detail
                },
                checkout: checkout.to_owned(),
                guidance: format!(
                    "The Git state was kept in {}. {recovery}",
                    checkout.display()
                ),
            });
        }
        Ok(IntegrationOutcome {
            action,
            checkout: checkout.to_owned(),
            message: success.to_owned(),
        })
    }

    fn verify_preview(&self, preview: &IntegrationPreview) -> Result<(), IntegrationError> {
        let stale = || IntegrationError {
            message: "The integration preview is stale".to_owned(),
            checkout: preview.target.path.clone(),
            guidance: "Refresh the preview before choosing an integration action".to_owned(),
        };
        let source = self.verify_checkout(&preview.source).map_err(|_| stale())?;
        let target = self.verify_checkout(&preview.target).map_err(|_| stale())?;
        if source.head != preview.source.head
            || target.head != preview.target.head
            || status_signature(&source.path).map_err(|_| stale())? != preview.source_signature
            || status_signature(&target.path).map_err(|_| stale())? != preview.target_signature
        {
            return Err(stale());
        }
        Ok(())
    }
}

fn affected_checkout(preview: &IntegrationPreview, action: IntegrationAction) -> &Path {
    if action == IntegrationAction::Rebase {
        &preview.source.path
    } else {
        &preview.target.path
    }
}

fn commits(directory: &Path, target: &str, source: &str) -> Result<Vec<CommitPreview>, String> {
    let range = format!("{target}..{source}");
    let output = byte_output(
        directory,
        &["log", "-z", "--reverse", "--format=%H%x00%s", &range, "--"],
    )?;
    let fields = output
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .collect::<Vec<_>>();
    if fields.len() % 2 != 0 {
        return Err("Git returned an incomplete commit preview".to_owned());
    }
    fields
        .chunks_exact(2)
        .map(|pair| {
            Ok(CommitPreview {
                id: String::from_utf8(pair[0].to_vec())
                    .map_err(|_| "Git returned a non-Unicode commit ID")?,
                subject: String::from_utf8_lossy(pair[1]).into_owned(),
            })
        })
        .collect()
}

fn status_signature(directory: &Path) -> Result<String, String> {
    let output = byte_output(
        directory,
        &[
            "status",
            "--porcelain=v2",
            "-z",
            "--untracked-files=all",
            "--ignore-submodules=none",
        ],
    )?;
    Ok(blake3::hash(&output).to_hex().to_string())
}

fn text_output(directory: &Path, args: &[&str]) -> Result<String, String> {
    String::from_utf8(byte_output(directory, args)?)
        .map_err(|_| "Git returned non-Unicode preview text".to_owned())
}

fn byte_output(directory: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = git_command(directory, args)
        .output()
        .map_err(|error| format!("Cannot run Git: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    Ok(output.stdout)
}

#[cfg(test)]
mod tests;
