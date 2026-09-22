use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use openpodium::git::{ChangedPath, CollisionReport, CollisionSeverity};

/// Built by the scan worker, never by the UI. Resolve each checkout only once,
/// regardless of how many changed files or nodes refer to it.
#[derive(Clone, Default)]
pub(super) struct IndexedReport {
    pub(super) report: CollisionReport,
    identities: BTreeMap<PathBuf, PathBuf>,
    severities: BTreeMap<PathBuf, CollisionSeverity>,
    inventories: BTreeMap<PathBuf, usize>,
}

impl IndexedReport {
    pub(super) fn new(report: CollisionReport, aliases: &[PathBuf]) -> Self {
        Self::resolve(report, aliases, |path| {
            dunce::canonicalize(path).unwrap_or_else(|_| dunce::simplified(path).to_owned())
        })
    }

    fn resolve(
        report: CollisionReport,
        aliases: &[PathBuf],
        mut canonicalize: impl FnMut(&Path) -> PathBuf,
    ) -> Self {
        let mut identities = BTreeMap::new();
        for path in aliases
            .iter()
            .chain(report.inventories.iter().map(|i| &i.checkout))
            .chain(report.collisions.iter().flat_map(|c| [&c.left, &c.right]))
        {
            identities
                .entry(path.clone())
                .or_insert_with(|| canonicalize(path));
        }
        let mut severities = BTreeMap::<PathBuf, CollisionSeverity>::new();
        for collision in &report.collisions {
            for path in [&collision.left, &collision.right] {
                severities
                    .entry(identities[path].clone())
                    .and_modify(|severity| *severity = (*severity).max(collision.severity))
                    .or_insert(collision.severity);
            }
        }
        let inventories = report
            .inventories
            .iter()
            .enumerate()
            .map(|(index, inventory)| (identities[&inventory.checkout].clone(), index))
            .collect();
        Self {
            report,
            identities,
            severities,
            inventories,
        }
    }

    fn identity<'a>(&'a self, path: &'a Path) -> &'a Path {
        self.identities.get(path).map_or(path, PathBuf::as_path)
    }

    pub(super) fn severity_for(&self, path: &Path) -> Option<CollisionSeverity> {
        self.severities.get(self.identity(path)).copied()
    }

    pub(super) fn changes_for(&self, path: &Path) -> &[ChangedPath] {
        self.inventories
            .get(self.identity(path))
            .map_or(&[], |index| {
                self.report.inventories[*index].paths.as_slice()
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openpodium::git::{Collision, WorktreeInventory};

    fn changed_paths() -> Vec<ChangedPath> {
        let temp = tempfile::tempdir().unwrap();
        for args in [
            vec!["init", "-b", "main"],
            vec![
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--allow-empty",
                "-m",
                "initial",
            ],
        ] {
            let result = std::process::Command::new("git")
                .current_dir(temp.path())
                .args(args)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
        std::fs::write(temp.path().join("file.rs"), "test").unwrap();
        let repo = openpodium::git::Repository::discover(temp.path()).unwrap();
        repo.collision_report(&repo.checkouts().unwrap())
            .unwrap()
            .inventories
            .remove(0)
            .paths
    }

    fn collision(path: &Path, severity: CollisionSeverity) -> Collision {
        let changed = changed_paths();
        Collision {
            left: path.to_owned(),
            right: PathBuf::from("/other"),
            path: changed[0].path.clone(),
            severity,
        }
    }

    #[test]
    fn resolution_cost_depends_on_checkouts_not_collision_count() {
        let path = PathBuf::from("/repo");
        let alias = PathBuf::from("/alias");
        let mut collisions = vec![collision(&path, CollisionSeverity::Warning); 10_000];
        collisions.push(collision(&path, CollisionSeverity::Critical));
        let report = CollisionReport {
            inventories: vec![],
            collisions,
        };
        let mut calls = 0;
        let indexed = IndexedReport::resolve(report, std::slice::from_ref(&alias), |p| {
            calls += 1;
            if p == alias {
                path.clone()
            } else {
                p.to_owned()
            }
        });
        assert_eq!(calls, 3);
        for _ in 0..100 {
            assert_eq!(
                indexed.severity_for(&alias),
                Some(CollisionSeverity::Critical)
            );
            assert_eq!(
                indexed.severity_for(&path),
                Some(CollisionSeverity::Critical)
            );
        }
        assert_eq!(indexed.severity_for(Path::new("/unrelated")), None);
    }

    #[cfg(unix)]
    #[test]
    fn aliases_and_inventories_are_resolved_before_rendering() {
        let temp = tempfile::tempdir().unwrap();
        let checkout = temp.path().join("checkout");
        let alias = temp.path().join("alias");
        std::fs::create_dir(&checkout).unwrap();
        std::os::unix::fs::symlink(&checkout, &alias).unwrap();
        let paths = changed_paths();
        let report = CollisionReport {
            inventories: vec![WorktreeInventory {
                checkout: checkout.clone(),
                paths: paths.clone(),
            }],
            collisions: vec![collision(&checkout, CollisionSeverity::Warning)],
        };
        let indexed = IndexedReport::new(report, std::slice::from_ref(&alias));
        std::fs::remove_file(&alias).unwrap();
        assert_eq!(
            indexed.severity_for(&alias),
            Some(CollisionSeverity::Warning)
        );
        assert_eq!(indexed.changes_for(&alias), paths);
        assert_eq!(indexed.changes_for(&checkout), paths);
        assert!(indexed.changes_for(Path::new("/unrelated")).is_empty());
    }
}
