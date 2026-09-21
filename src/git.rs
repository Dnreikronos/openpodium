//! Conservative Git CLI boundary. Ownership is supplied by durable floor records.

use std::path::{Path, PathBuf};
use std::process::Command;

mod changes;
mod context;
mod integration;

pub use changes::{
    ChangeKind, ChangedPath, Collision, CollisionReport, CollisionSeverity, RepoPath,
    WorktreeInventory,
};
pub use context::{PathDiff, path_diff, path_is_ignored, path_is_tracked, project_paths};
pub use integration::{
    CommitPreview, IntegrationAction, IntegrationError, IntegrationOutcome, IntegrationPreview,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkout {
    pub path: PathBuf,
    pub head: String,
    pub branch: Option<String>,
    pub locked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repository {
    pub root: PathBuf,
    pub common_directory: PathBuf,
}

pub fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 200
        || name.starts_with('-')
        || name == "HEAD"
        || name.contains("..")
        || name.contains("@{")
        || name
            .bytes()
            .any(|b| b <= 32 || b >= 127 || b"~^:?*[\\".contains(&b))
        || name.split('/').any(|part| {
            part.is_empty()
                || part.starts_with('.')
                || part.ends_with('.')
                || part.ends_with(".lock")
        })
    {
        return Err("Use a valid Git branch name: no spaces, control characters, traversal, or revision syntax".to_owned());
    }
    Ok(())
}

pub fn validate_floor_name(name: &str) -> Result<(), String> {
    validate_name(name)?;
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
            })
        });
    if reserved
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return Err("Use a portable floor name with letters, numbers, dots, underscores, or hyphens; reserved device names are not allowed".to_owned());
    }
    Ok(())
}

impl Repository {
    pub fn discover(directory: &Path) -> Result<Self, String> {
        let root = run(directory, &["rev-parse", "--show-toplevel"])?;
        let common = run(
            directory,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )?;
        Ok(Self {
            root: dunce::canonicalize(root.trim_end_matches('\n')).map_err(|e| e.to_string())?,
            common_directory: dunce::canonicalize(common.trim_end_matches('\n'))
                .map_err(|e| e.to_string())?,
        })
    }

    pub fn checkouts(&self) -> Result<Vec<Checkout>, String> {
        let output = run(&self.root, &["worktree", "list", "--porcelain", "-z"])?;
        let mut checkouts = Vec::new();
        let mut current: Option<Checkout> = None;
        for field in output.split('\0') {
            if let Some(path) = field.strip_prefix("worktree ") {
                if let Some(checkout) = current.take() {
                    checkouts.push(checkout);
                }
                current = Some(Checkout {
                    path: dunce::simplified(Path::new(path)).to_owned(),
                    head: String::new(),
                    branch: None,
                    locked: false,
                });
            } else if let Some(checkout) = current.as_mut() {
                if let Some(head) = field.strip_prefix("HEAD ") {
                    checkout.head = head.to_owned();
                }
                if let Some(branch) = field.strip_prefix("branch refs/heads/") {
                    checkout.branch = Some(branch.to_owned());
                }
                if field == "locked" || field.starts_with("locked ") {
                    checkout.locked = true;
                }
            }
        }
        if let Some(checkout) = current {
            checkouts.push(checkout);
        }
        Ok(checkouts)
    }

    pub fn create(&self, name: &str, branch: &str, parent: &Path) -> Result<Checkout, String> {
        validate_floor_name(name)?;
        validate_name(branch)?;
        let parent = dunce::canonicalize(parent).map_err(|e| e.to_string())?;
        let path = parent.join(name);
        if path.exists() {
            return Err("The floor directory already exists".to_owned());
        }
        let base = run(&self.root, &["rev-parse", "--verify", "HEAD^{commit}"])?;
        let path_text = path
            .to_str()
            .ok_or("The worktree path must be valid Unicode")?;
        run(
            &self.root,
            &[
                "worktree",
                "add",
                "-b",
                branch,
                "--",
                path_text,
                base.trim(),
            ],
        )?;
        Ok(Checkout {
            path,
            head: base.trim().to_owned(),
            branch: Some(branch.to_owned()),
            locked: false,
        })
    }

    pub(crate) fn claim(&self, checkout: &Checkout) -> Result<String, String> {
        use std::io::Write;
        self.verify_checkout(checkout)?;
        let mut bytes = [0_u8; 32];
        getrandom::fill(&mut bytes).map_err(|e| e.to_string())?;
        let token = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let marker = self.ownership_marker(checkout)?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(marker)
            .map_err(|e| e.to_string())?;
        file.write_all(token.as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        Ok(token)
    }

    pub(crate) fn owns(&self, checkout: &Checkout, token: Option<&str>) -> Result<bool, String> {
        let Some(token) = token else {
            return Ok(false);
        };
        let marker = self.ownership_marker(checkout)?;
        match std::fs::read_to_string(marker) {
            Ok(actual) => Ok(actual == token),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.to_string()),
        }
    }

    fn ownership_marker(&self, checkout: &Checkout) -> Result<PathBuf, String> {
        self.verify_checkout(checkout)?;
        let directory = run(&checkout.path, &["rev-parse", "--absolute-git-dir"])?;
        Ok(PathBuf::from(directory.trim_end_matches('\n')).join("openpodium-owner"))
    }

    pub fn dirty(&self, checkout: &Checkout) -> Result<bool, String> {
        self.verify_checkout(checkout)?;
        Ok(!run(
            &checkout.path,
            &[
                "status",
                "--porcelain=v1",
                "--untracked-files=all",
                "--ignore-submodules=none",
            ],
        )?
        .is_empty())
    }

    pub fn remove(
        &self,
        checkout: &Checkout,
        managed: bool,
        base_branch: &str,
        discard: bool,
    ) -> Result<(), String> {
        if !managed {
            return Err("User-owned worktrees cannot be removed".to_owned());
        }
        validate_name(base_branch)?;
        let actual = self.verify_checkout(checkout)?;
        if actual.locked {
            return Err("Unlock this worktree in Git before removing it".to_owned());
        }
        let checkouts = self.checkouts()?;
        if checkouts
            .first()
            .is_some_and(|main| main.path == checkout.path)
        {
            return Err("The original checkout cannot be removed".to_owned());
        }
        let dirty = self.dirty(checkout)?;
        let target = format!("refs/heads/{base_branch}");
        let merged = is_ancestor(&self.root, &actual.head, &target)?;
        if !discard && (dirty || !merged) {
            return Err("Cleanup refused: this floor has dirty or unmerged work; keep it or explicitly discard it".to_owned());
        }
        let path = checkout
            .path
            .to_str()
            .ok_or("The worktree path must be valid Unicode")?;
        let mut args = vec!["worktree", "remove"];
        if discard {
            args.push("--force");
        }
        args.extend(["--", path]);
        run(&self.root, &args)?;
        Ok(())
    }

    fn verify_checkout(&self, expected: &Checkout) -> Result<Checkout, String> {
        let actual = self
            .checkouts()?
            .into_iter()
            .find(|c| c.path == expected.path)
            .ok_or("The checkout is no longer registered in this repository")?;
        if actual.branch != expected.branch {
            return Err("The checkout branch changed; refresh before continuing".to_owned());
        }
        let repo = Self::discover(&expected.path)?;
        if repo.common_directory != self.common_directory || repo.root != expected.path {
            return Err("The checkout repository identity changed".to_owned());
        }
        Ok(actual)
    }
}

fn is_ancestor(root: &Path, head: &str, target: &str) -> Result<bool, String> {
    let output = git_command(root, &["merge-base", "--is-ancestor", head, target])
        .output()
        .map_err(|e| e.to_string())?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(
            crate::security::redact_secrets(String::from_utf8_lossy(&output.stderr).trim())
                .into_owned(),
        ),
    }
}

fn run(directory: &Path, args: &[&str]) -> Result<String, String> {
    let output = git_command(directory, args)
        .output()
        .map_err(|e| format!("Cannot run Git: {e}"))?;
    if !output.status.success() {
        return Err(crate::security::redact_secrets(
            String::from_utf8_lossy(&output.stderr).trim(),
        )
        .into_owned());
    }
    String::from_utf8(output.stdout).map_err(|_| "Git returned a non-Unicode path".to_owned())
}

pub(super) fn git_command(directory: &Path, args: &[&str]) -> Command {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(dunce::simplified(directory))
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0");
    command
}

#[cfg(test)]
mod tests;
