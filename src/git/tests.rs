use super::*;

fn repository() -> (tempfile::TempDir, Repository) {
    let directory = tempfile::tempdir().unwrap();
    run(directory.path(), &["init", "-b", "main"]).unwrap();
    run(
        directory.path(),
        &["config", "user.email", "test@example.com"],
    )
    .unwrap();
    run(directory.path(), &["config", "user.name", "Test"]).unwrap();
    run(
        directory.path(),
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-m",
            "initial",
        ],
    )
    .unwrap();
    let repo = Repository::discover(directory.path()).unwrap();
    (directory, repo)
}

#[test]
fn validates_before_attempting_git() {
    let repo = Repository {
        root: PathBuf::from("/nonexistent"),
        common_directory: PathBuf::new(),
    };
    for name in [
        "",
        "../escape",
        "-option",
        "HEAD",
        "x.lock",
        "a//b",
        "a@{b",
        "a b",
        "a\nb",
        "a~b",
    ] {
        assert!(validate_name(name).is_err(), "{name:?}");
        assert!(
            repo.create(name, "valid", Path::new("/nonexistent"))
                .unwrap_err()
                .contains("valid Git branch")
        );
    }
    assert!(validate_name("feature/task_15").is_ok());
}

#[test]
fn discovers_creates_and_protects_user_worktrees() {
    let (directory, repo) = repository();
    let nested = directory.path().join("nested");
    std::fs::create_dir(&nested).unwrap();
    assert_eq!(Repository::discover(&nested).unwrap(), repo);
    let parent = tempfile::tempdir().unwrap();
    let checkout = repo.create("floor", "feature/task", parent.path()).unwrap();
    assert_eq!(repo.checkouts().unwrap().len(), 2);
    assert!(!repo.dirty(&checkout).unwrap());
    assert!(repo.remove(&checkout, false, "main", true).is_err());
    assert!(checkout.path.exists());
    assert!(repo.create("floor", "other", parent.path()).is_err());
    repo.remove(&checkout, true, "main", false).unwrap();
    assert!(!checkout.path.exists());
    assert!(
        run(
            &repo.root,
            &["rev-parse", "--verify", "refs/heads/feature/task"]
        )
        .is_ok()
    );
}

#[test]
fn cleanup_requires_decision_for_dirty_and_unmerged_work() {
    let (_directory, repo) = repository();
    let parent = tempfile::tempdir().unwrap();
    let checkout = repo.create("floor", "task", parent.path()).unwrap();
    std::fs::write(checkout.path.join("untracked"), "work").unwrap();
    assert!(repo.dirty(&checkout).unwrap());
    assert!(repo.remove(&checkout, true, "main", false).is_err());
    run(&checkout.path, &["add", "untracked"]).unwrap();
    run(
        &checkout.path,
        &["-c", "commit.gpgsign=false", "commit", "-m", "work"],
    )
    .unwrap();
    assert!(!repo.dirty(&checkout).unwrap());
    assert!(repo.remove(&checkout, true, "main", false).is_err());
    repo.remove(&checkout, true, "main", true).unwrap();
}

#[test]
fn locked_and_original_checkouts_are_never_removed() {
    let (_directory, repo) = repository();
    let main = repo.checkouts().unwrap().remove(0);
    assert!(repo.remove(&main, true, "main", true).is_err());
    let parent = tempfile::tempdir().unwrap();
    let checkout = repo.create("floor", "task", parent.path()).unwrap();
    run(
        &repo.root,
        &["worktree", "lock", checkout.path.to_str().unwrap()],
    )
    .unwrap();
    assert!(repo.remove(&checkout, true, "main", true).is_err());
    assert!(checkout.path.exists());
}

#[test]
fn floor_names_are_portable_path_components() {
    for name in [
        "CON",
        "nul.txt",
        "COM1",
        "LPT9.log",
        "nested/name",
        "a|b",
        "a<b",
        "a>b",
        "a\"b",
    ] {
        assert!(validate_floor_name(name).is_err(), "{name}");
    }
    assert!(validate_floor_name("task_15-login").is_ok());
}

#[test]
fn canonicalized_paths_support_discovery_creation_and_cleanup() {
    let (directory, expected) = repository();
    let repo = Repository::discover(&directory.path().canonicalize().unwrap()).unwrap();
    assert_eq!(repo, expected);
    assert_eq!(repo.checkouts().unwrap()[0].path, repo.root);

    let parent = tempfile::tempdir().unwrap();
    let with_spaces = parent.path().join("worktree parent");
    std::fs::create_dir(&with_spaces).unwrap();
    let checkout = repo
        .create("floor", "task", &with_spaces.canonicalize().unwrap())
        .unwrap();
    assert_eq!(
        Repository::discover(&checkout.path).unwrap().root,
        checkout.path
    );
    assert!(
        repo.checkouts()
            .unwrap()
            .iter()
            .any(|c| c.path == checkout.path)
    );
    let token = repo.claim(&checkout).unwrap();
    assert!(repo.owns(&checkout, Some(&token)).unwrap());
    assert!(!repo.dirty(&checkout).unwrap());
    repo.remove(&checkout, true, "main", false).unwrap();
    assert!(!checkout.path.exists());
}

#[test]
fn collision_report_handles_ignored_renamed_and_deleted_paths() {
    let (directory, repo) = repository();
    std::fs::write(directory.path().join("shared"), "base\n").unwrap();
    std::fs::write(directory.path().join("removed"), "base\n").unwrap();
    std::fs::write(directory.path().join(".gitignore"), "generated\n").unwrap();
    run(directory.path(), &["add", "."]).unwrap();
    run(
        directory.path(),
        &["-c", "commit.gpgsign=false", "commit", "-m", "files"],
    )
    .unwrap();

    let parent = tempfile::tempdir().unwrap();
    let left = repo.create("left", "left", parent.path()).unwrap();
    let right = repo.create("right", "right", parent.path()).unwrap();
    std::fs::write(left.path.join("shared"), "left\n").unwrap();
    std::fs::rename(left.path.join("removed"), left.path.join("renamed")).unwrap();
    std::fs::write(left.path.join("generated"), "ignored\n").unwrap();
    std::fs::write(right.path.join("shared"), "right\n").unwrap();
    std::fs::remove_file(right.path.join("removed")).unwrap();

    let report = repo
        .collision_report(&[left.clone(), right.clone()])
        .unwrap();
    let paths = report
        .collisions
        .iter()
        .map(|collision| collision.path.display())
        .collect::<Vec<_>>();
    assert_eq!(paths, vec!["removed", "shared"]);
    assert!(
        report
            .inventories
            .iter()
            .flat_map(|inventory| &inventory.paths)
            .all(|change| change.path.display() != "generated")
    );
    assert_eq!(
        report.severity_for(&left.path),
        Some(CollisionSeverity::Warning)
    );
}

#[test]
fn previews_reject_stale_state_and_integrate_explicit_choices() {
    let (directory, repo) = repository();
    std::fs::write(directory.path().join("file"), "base\n").unwrap();
    run(directory.path(), &["add", "file"]).unwrap();
    run(
        directory.path(),
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    )
    .unwrap();
    let target = repo.checkouts().unwrap().remove(0);
    let parent = tempfile::tempdir().unwrap();
    let source = repo.create("source", "source", parent.path()).unwrap();
    std::fs::write(source.path.join("source-file"), "source\n").unwrap();
    run(&source.path, &["add", "source-file"]).unwrap();
    run(
        &source.path,
        &["-c", "commit.gpgsign=false", "commit", "-m", "source"],
    )
    .unwrap();

    let preview = repo.preview_integration(&source, &target).unwrap();
    assert_eq!(preview.commits.len(), 1);
    assert!(preview.blocker(IntegrationAction::Merge).is_none());
    std::fs::write(directory.path().join("untracked"), "changed\n").unwrap();
    let error = repo
        .integrate(&preview, IntegrationAction::Merge)
        .unwrap_err();
    assert!(error.message.contains("stale"));
    std::fs::remove_file(directory.path().join("untracked")).unwrap();

    let preview = repo.preview_integration(&source, &target).unwrap();
    let outcome = repo
        .integrate(&preview, IntegrationAction::CherryPick)
        .unwrap();
    assert_eq!(outcome.checkout, target.path);
    assert!(directory.path().join("source-file").exists());

    let preview = repo
        .preview_integration(&source, &repo.checkouts().unwrap()[0])
        .unwrap();
    let outcome = repo
        .integrate(&preview, IntegrationAction::LeaveAlone)
        .unwrap();
    assert!(outcome.message.contains("unchanged"));
}

#[test]
fn failed_merge_keeps_recoverable_git_state() {
    let (directory, repo) = repository();
    std::fs::write(directory.path().join("shared"), "base\n").unwrap();
    run(directory.path(), &["add", "shared"]).unwrap();
    run(
        directory.path(),
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    )
    .unwrap();
    let parent = tempfile::tempdir().unwrap();
    let source = repo.create("source", "source", parent.path()).unwrap();
    std::fs::write(source.path.join("shared"), "source\n").unwrap();
    run(&source.path, &["add", "shared"]).unwrap();
    run(
        &source.path,
        &["-c", "commit.gpgsign=false", "commit", "-m", "source"],
    )
    .unwrap();
    std::fs::write(directory.path().join("shared"), "target\n").unwrap();
    run(directory.path(), &["add", "shared"]).unwrap();
    run(
        directory.path(),
        &["-c", "commit.gpgsign=false", "commit", "-m", "target"],
    )
    .unwrap();
    let target = repo.checkouts().unwrap().remove(0);
    let preview = repo.preview_integration(&source, &target).unwrap();
    assert_eq!(preview.likely_conflicts[0].display(), "shared");
    let error = repo
        .integrate(&preview, IntegrationAction::Merge)
        .unwrap_err();
    assert!(error.guidance.contains("git merge --abort"));
    assert!(directory.path().join(".git/MERGE_HEAD").exists());
    run(directory.path(), &["merge", "--abort"]).unwrap();
}
