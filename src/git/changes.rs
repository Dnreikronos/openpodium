use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use super::{Checkout, Repository, git_command};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RepoPath(Vec<u8>);

impl RepoPath {
    pub fn display(&self) -> String {
        String::from_utf8_lossy(&self.0).into_owned()
    }

    fn new(bytes: &[u8]) -> Self {
        Self(bytes.to_vec())
    }
}

impl fmt::Display for RepoPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.display())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed { from: RepoPath },
    Untracked,
    Unmerged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedPath {
    pub path: RepoPath,
    pub kind: ChangeKind,
}

impl ChangedPath {
    fn touched_paths(&self) -> impl Iterator<Item = &RepoPath> {
        std::iter::once(&self.path).chain(match &self.kind {
            ChangeKind::Renamed { from } => Some(from),
            _ => None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeInventory {
    pub checkout: PathBuf,
    pub paths: Vec<ChangedPath>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CollisionSeverity {
    Warning,
    Critical,
}

impl fmt::Display for CollisionSeverity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Warning => "warning",
            Self::Critical => "critical",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collision {
    pub left: PathBuf,
    pub right: PathBuf,
    pub path: RepoPath,
    pub severity: CollisionSeverity,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CollisionReport {
    pub inventories: Vec<WorktreeInventory>,
    pub collisions: Vec<Collision>,
}

impl CollisionReport {
    pub fn severity_for(&self, checkout: &Path) -> Option<CollisionSeverity> {
        self.collisions
            .iter()
            .filter(|collision| {
                same_path(&collision.left, checkout) || same_path(&collision.right, checkout)
            })
            .map(|collision| collision.severity)
            .max()
    }
}

impl Repository {
    pub fn change_signature(&self, checkouts: &[Checkout]) -> Result<String, String> {
        let mut state = blake3::Hasher::new();
        for checkout in checkouts {
            let actual = self.verify_checkout(checkout)?;
            state.update(actual.path.to_string_lossy().as_bytes());
            state.update(actual.head.as_bytes());
            state.update(&output(
                &actual.path,
                &[
                    "status",
                    "--porcelain=v2",
                    "-z",
                    "--untracked-files=all",
                    "--ignore-submodules=none",
                ],
            )?);
        }
        Ok(state.finalize().to_hex().to_string())
    }

    pub fn collision_report(&self, checkouts: &[Checkout]) -> Result<CollisionReport, String> {
        for checkout in checkouts {
            self.verify_checkout(checkout)?;
        }

        let mut inventories = checkouts
            .iter()
            .map(|checkout| (checkout.path.clone(), BTreeMap::new()))
            .collect::<BTreeMap<_, BTreeMap<RepoPath, ChangeKind>>>();
        if checkouts.len() == 1 {
            let checkout = &checkouts[0];
            merge_changes(
                inventories
                    .get_mut(&checkout.path)
                    .expect("checkout inventory exists"),
                changed_paths(&checkout.path, &checkout.head)?,
            );
        }

        let mut collisions = Vec::new();
        for (left_index, left) in checkouts.iter().enumerate() {
            for right in &checkouts[left_index + 1..] {
                let base = merge_base(&self.root, &left.head, &right.head)?;
                let left_changes = changed_paths(&left.path, &base)?;
                let right_changes = changed_paths(&right.path, &base)?;
                merge_changes(
                    inventories
                        .get_mut(&left.path)
                        .expect("checkout inventory exists"),
                    left_changes.clone(),
                );
                merge_changes(
                    inventories
                        .get_mut(&right.path)
                        .expect("checkout inventory exists"),
                    right_changes.clone(),
                );

                let likely_conflicts = likely_conflicts(&self.root, &base, left, right)?;
                let right_paths = touched(&right_changes);
                let overlaps = touched(&left_changes)
                    .intersection(&right_paths)
                    .cloned()
                    .collect::<BTreeSet<_>>();
                collisions.extend(overlaps.into_iter().map(|path| Collision {
                    left: left.path.clone(),
                    right: right.path.clone(),
                    severity: if likely_conflicts.contains(&path) {
                        CollisionSeverity::Critical
                    } else {
                        CollisionSeverity::Warning
                    },
                    path,
                }));
            }
        }

        Ok(CollisionReport {
            inventories: inventories
                .into_iter()
                .map(|(checkout, paths)| WorktreeInventory {
                    checkout,
                    paths: paths
                        .into_iter()
                        .map(|(path, kind)| ChangedPath { path, kind })
                        .collect(),
                })
                .collect(),
            collisions,
        })
    }
}

pub(super) fn changed_paths(checkout: &Path, base: &str) -> Result<Vec<ChangedPath>, String> {
    let diff = output(
        checkout,
        &["diff", "--name-status", "-z", "--find-renames", base, "--"],
    )?;
    let mut changes = parse_name_status(&diff)?;
    let untracked = output(
        checkout,
        &["ls-files", "--others", "--exclude-standard", "-z", "--"],
    )?;
    changes.extend(
        fields(&untracked)
            .filter(|path| !path.is_empty())
            .map(|path| ChangedPath {
                path: RepoPath::new(path),
                kind: ChangeKind::Untracked,
            }),
    );
    Ok(changes)
}

pub(super) fn merge_base(root: &Path, left: &str, right: &str) -> Result<String, String> {
    let output = git_command(root, &["merge-base", left, right])
        .output()
        .map_err(|error| format!("Cannot run Git: {error}"))?;
    if !output.status.success() {
        return Err(stderr(&output.stderr));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_owned())
        .map_err(|_| "Git returned a non-Unicode revision".to_owned())
}

pub(super) fn likely_conflicts(
    root: &Path,
    base: &str,
    left: &Checkout,
    right: &Checkout,
) -> Result<BTreeSet<RepoPath>, String> {
    let index = TemporaryIndex::new()?;
    let index_text = index
        .path
        .to_str()
        .ok_or("The temporary Git index path must be valid Unicode")?;
    let read = git_command(root, &["read-tree", "-m", base, &left.head, &right.head])
        .env("GIT_INDEX_FILE", index_text)
        .output()
        .map_err(|error| format!("Cannot run Git: {error}"))?;
    if !read.status.success() {
        return Err(stderr(&read.stderr));
    }
    let unmerged = git_command(root, &["ls-files", "-u", "-z"])
        .env("GIT_INDEX_FILE", index_text)
        .output()
        .map_err(|error| format!("Cannot run Git: {error}"))?;
    if !unmerged.status.success() {
        return Err(stderr(&unmerged.stderr));
    }
    Ok(fields(&unmerged.stdout)
        .filter_map(|record| {
            record
                .iter()
                .position(|byte| *byte == b'\t')
                .map(|tab| &record[tab + 1..])
        })
        .map(RepoPath::new)
        .collect())
}

fn parse_name_status(bytes: &[u8]) -> Result<Vec<ChangedPath>, String> {
    let mut fields = fields(bytes);
    let mut changes = Vec::new();
    while let Some(status) = fields.next() {
        if status.is_empty() {
            continue;
        }
        let code = status[0];
        let first = fields
            .next()
            .ok_or("Git returned an incomplete changed-path record")?;
        let (path, kind) = match code {
            b'A' => (first, ChangeKind::Added),
            b'M' | b'T' => (first, ChangeKind::Modified),
            b'D' => (first, ChangeKind::Deleted),
            b'R' | b'C' => {
                let destination = fields
                    .next()
                    .ok_or("Git returned an incomplete rename record")?;
                (
                    destination,
                    ChangeKind::Renamed {
                        from: RepoPath::new(first),
                    },
                )
            }
            b'U' => (first, ChangeKind::Unmerged),
            _ => return Err(format!("Git returned unsupported change status {code}")),
        };
        changes.push(ChangedPath {
            path: RepoPath::new(path),
            kind,
        });
    }
    Ok(changes)
}

fn touched(changes: &[ChangedPath]) -> BTreeSet<RepoPath> {
    changes
        .iter()
        .flat_map(ChangedPath::touched_paths)
        .cloned()
        .collect()
}

fn merge_changes(target: &mut BTreeMap<RepoPath, ChangeKind>, changes: Vec<ChangedPath>) {
    for change in changes {
        target.entry(change.path).or_insert(change.kind);
    }
}

fn output(directory: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = git_command(directory, args)
        .output()
        .map_err(|error| format!("Cannot run Git: {error}"))?;
    if !output.status.success() {
        return Err(stderr(&output.stderr));
    }
    Ok(output.stdout)
}

fn fields(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    bytes.split(|byte| *byte == 0)
}

fn stderr(bytes: &[u8]) -> String {
    crate::security::redact_secrets(String::from_utf8_lossy(bytes).trim()).into_owned()
}

fn same_path(left: &Path, right: &Path) -> bool {
    match (dunce::canonicalize(left), dunce::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => dunce::simplified(left) == dunce::simplified(right),
    }
}

struct TemporaryIndex {
    path: PathBuf,
}

impl TemporaryIndex {
    fn new() -> Result<Self, String> {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).map_err(|error| error.to_string())?;
        let suffix = random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        Ok(Self {
            path: std::env::temp_dir().join(format!("openpodium-index-{suffix}")),
        })
    }
}

impl Drop for TemporaryIndex {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_deletes_and_renames_without_losing_the_old_path() {
        let changes = parse_name_status(b"D\0gone.rs\0R100\0old.rs\0new.rs\0").unwrap();
        assert_eq!(
            changes,
            vec![
                ChangedPath {
                    path: RepoPath::new(b"gone.rs"),
                    kind: ChangeKind::Deleted,
                },
                ChangedPath {
                    path: RepoPath::new(b"new.rs"),
                    kind: ChangeKind::Renamed {
                        from: RepoPath::new(b"old.rs"),
                    },
                },
            ]
        );
    }
}
