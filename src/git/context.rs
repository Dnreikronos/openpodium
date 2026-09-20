use std::path::Path;

use super::git_command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathDiff {
    pub text: String,
    pub binary: bool,
}

pub fn path_is_ignored(checkout: &Path, path: &str) -> Result<bool, String> {
    let output = git_command(checkout, &["check-ignore", "--quiet", "--", path])
        .output()
        .map_err(|error| format!("Cannot run Git: {error}"))?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(stderr(&output.stderr)),
    }
}

pub fn path_is_tracked(checkout: &Path, path: &str) -> Result<bool, String> {
    let output = git_command(checkout, &["ls-files", "--error-unmatch", "--", path])
        .output()
        .map_err(|error| format!("Cannot run Git: {error}"))?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(stderr(&output.stderr)),
    }
}

pub fn path_diff(checkout: &Path, path: &str) -> Result<PathDiff, String> {
    let output = git_command(
        checkout,
        &[
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "HEAD",
            "--",
            path,
        ],
    )
    .output()
    .map_err(|error| format!("Cannot run Git: {error}"))?;
    if !output.status.success() {
        return Err(stderr(&output.stderr));
    }
    let text = String::from_utf8(output.stdout)
        .map_err(|_| "Git returned a diff that is not valid UTF-8".to_owned())?;
    let binary = text
        .lines()
        .any(|line| line.starts_with("Binary files ") || line.starts_with("GIT binary patch"));
    Ok(PathDiff { text, binary })
}

fn stderr(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim().to_owned()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::process::Command;

    use super::*;

    fn git(directory: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(directory)
            .args(args)
            .env("GIT_AUTHOR_NAME", "OpenPodium")
            .env("GIT_AUTHOR_EMAIL", "test@openpodium.invalid")
            .env("GIT_COMMITTER_NAME", "OpenPodium")
            .env("GIT_COMMITTER_EMAIL", "test@openpodium.invalid")
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed");
    }

    #[test]
    fn reports_ignore_tracking_and_text_diff_without_shell_interpolation() {
        let repository = tempfile::tempdir().unwrap();
        git(repository.path(), &["init", "--quiet"]);
        fs::write(repository.path().join("tracked.txt"), "before\n").unwrap();
        fs::write(repository.path().join("tracked.bin"), b"before\0bytes").unwrap();
        fs::write(repository.path().join(".gitignore"), "ignored.txt\n").unwrap();
        git(repository.path(), &["add", "."]);
        git(repository.path(), &["commit", "--quiet", "-m", "initial"]);
        fs::write(repository.path().join("tracked.txt"), "after\n").unwrap();
        fs::write(repository.path().join("tracked.bin"), b"after\0bytes").unwrap();

        assert!(path_is_tracked(repository.path(), "tracked.txt").unwrap());
        assert!(!path_is_tracked(repository.path(), "untracked.txt").unwrap());
        assert!(path_is_ignored(repository.path(), "ignored.txt").unwrap());
        let diff = path_diff(repository.path(), "tracked.txt").unwrap();
        assert!(!diff.binary);
        assert!(diff.text.contains("-before"));
        assert!(diff.text.contains("+after"));
        let binary = path_diff(repository.path(), "tracked.bin").unwrap();
        assert!(binary.binary);
        assert!(binary.text.contains("Binary files"));
    }
}
