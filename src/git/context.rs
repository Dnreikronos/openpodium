use std::fs;
use std::path::Path;

use crate::domain::ProjectPath;

use super::git_command;

const MAX_INDEXED_PROJECT_PATHS: usize = 100_000;

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

pub fn project_paths(checkout: &Path) -> Result<Vec<ProjectPath>, String> {
    let output = git_command(
        checkout,
        &[
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
        ],
    )
    .output();
    let mut paths = match output {
        Ok(output) if output.status.success() => output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|bytes| !bytes.is_empty())
            .filter_map(|bytes| std::str::from_utf8(bytes).ok())
            .filter_map(|path| ProjectPath::new(path.to_owned()).ok())
            .collect::<Vec<_>>(),
        Ok(_) | Err(_) => filesystem_project_paths(checkout)?,
    };
    paths.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    paths.dedup_by(|left, right| left.as_str() == right.as_str());
    paths.truncate(MAX_INDEXED_PROJECT_PATHS);
    Ok(paths)
}

fn filesystem_project_paths(checkout: &Path) -> Result<Vec<ProjectPath>, String> {
    let mut pending = vec![checkout.to_owned()];
    let mut paths = Vec::new();
    while let Some(path) = pending.pop() {
        let mut entries = fs::read_dir(&path)
            .map_err(|error| format!("Cannot list {}: {error}", path.display()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Cannot read {}: {error}", path.display()))?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries.into_iter().rev() {
            if entry.file_name() == ".git" {
                continue;
            }
            let entry_path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|error| format!("Cannot inspect {}: {error}", entry_path.display()))?;
            if file_type.is_dir() {
                pending.push(entry_path);
                continue;
            }
            let components = entry_path
                .strip_prefix(checkout)
                .map_err(|error| error.to_string())?
                .components()
                .map(|component| component.as_os_str().to_str())
                .collect::<Option<Vec<_>>>();
            let Some(components) = components else {
                continue;
            };
            let relative = components.join("/");
            if let Ok(path) = ProjectPath::new(relative) {
                paths.push(path);
                if paths.len() == MAX_INDEXED_PROJECT_PATHS {
                    return Ok(paths);
                }
            }
        }
    }
    Ok(paths)
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
    crate::security::redact_secrets(String::from_utf8_lossy(bytes).trim()).into_owned()
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

        fs::write(repository.path().join("untracked.txt"), "new").unwrap();
        fs::write(repository.path().join("ignored.txt"), "hidden").unwrap();
        let paths = project_paths(repository.path()).unwrap();
        assert!(paths.iter().any(|path| path.as_str() == "tracked.txt"));
        assert!(paths.iter().any(|path| path.as_str() == "untracked.txt"));
        assert!(!paths.iter().any(|path| path.as_str() == "ignored.txt"));
    }

    #[test]
    fn indexes_files_outside_a_git_repository() {
        let project = tempfile::tempdir().unwrap();
        fs::create_dir(project.path().join("nested")).unwrap();
        fs::write(project.path().join("README.md"), "read me").unwrap();
        fs::write(
            project.path().join("nested").join("main.rs"),
            "fn main() {}",
        )
        .unwrap();

        let paths = project_paths(project.path()).unwrap();

        assert_eq!(
            paths.iter().map(|path| path.as_str()).collect::<Vec<_>>(),
            vec!["README.md", "nested/main.rs"]
        );
    }
}
