use super::*;
use crate::git::run;

fn repository() -> (tempfile::TempDir, Repository) {
    let directory = tempfile::tempdir().unwrap();
    run(directory.path(), &["init", "-b", "main"]).unwrap();
    run(
        directory.path(),
        &["config", "user.email", "test@example.com"],
    )
    .unwrap();
    run(directory.path(), &["config", "user.name", "Test"]).unwrap();
    std::fs::write(directory.path().join("base"), "base\n").unwrap();
    commit(directory.path(), "base");
    let repo = Repository::discover(directory.path()).unwrap();
    (directory, repo)
}

fn commit(directory: &Path, message: &str) {
    run(directory, &["add", "."]).unwrap();
    run(
        directory,
        &["-c", "commit.gpgsign=false", "commit", "-m", message],
    )
    .unwrap();
}

#[test]
fn merge_applies_the_source_head_to_the_target() {
    let (directory, repo) = repository();
    let parent = tempfile::tempdir().unwrap();
    let source = repo.create("source", "source", parent.path()).unwrap();
    std::fs::write(source.path.join("source-file"), "source\n").unwrap();
    commit(&source.path, "source");
    let target = repo.checkouts().unwrap().remove(0);

    let preview = repo.preview_integration(&source, &target).unwrap();
    let outcome = repo.integrate(&preview, IntegrationAction::Merge).unwrap();

    assert_eq!(outcome.action, IntegrationAction::Merge);
    assert!(directory.path().join("source-file").exists());
}

#[test]
fn rebase_updates_only_the_source_checkout() {
    let (directory, repo) = repository();
    let parent = tempfile::tempdir().unwrap();
    let source = repo.create("source", "source", parent.path()).unwrap();
    std::fs::write(source.path.join("source-file"), "source\n").unwrap();
    commit(&source.path, "source");
    std::fs::write(directory.path().join("target-file"), "target\n").unwrap();
    commit(directory.path(), "target");
    let target = repo.checkouts().unwrap().remove(0);
    let target_head = target.head.clone();

    let preview = repo.preview_integration(&source, &target).unwrap();
    let outcome = repo.integrate(&preview, IntegrationAction::Rebase).unwrap();

    assert_eq!(outcome.checkout, source.path);
    assert!(!directory.path().join("source-file").exists());
    assert!(source.path.join("target-file").exists());
    let source_after = repo
        .checkouts()
        .unwrap()
        .into_iter()
        .find(|checkout| checkout.path == source.path)
        .unwrap();
    assert!(
        run(
            &repo.root,
            &[
                "merge-base",
                "--is-ancestor",
                &target_head,
                &source_after.head
            ]
        )
        .is_ok()
    );
}
