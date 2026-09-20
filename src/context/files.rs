use std::fs;
use std::path::Path;

use crate::domain::ProjectPath;
use crate::git::{ChangeKind, ChangedPath, path_diff, path_is_ignored, path_is_tracked};

use super::{ContextFileError, FilePreview, MAX_TEXT_PREVIEW_BYTES, preview_file};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileTreeEntryKind {
    Directory,
    File,
    Symlink,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileTreeEntry {
    pub path: ProjectPath,
    pub name: String,
    pub kind: FileTreeEntryKind,
    pub size: u64,
    pub change: Option<ChangeKind>,
}

pub fn list_directory(
    checkout: &Path,
    root: &ProjectPath,
    changes: &[ChangedPath],
) -> Result<Vec<FileTreeEntry>, ContextFileError> {
    let directory = super::resolve_project_path(checkout, root)?;
    let entries = fs::read_dir(&directory).map_err(|source| ContextFileError::Io {
        operation: "list directory",
        path: directory.clone(),
        source: source.to_string(),
    })?;
    let mut result = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| ContextFileError::Io {
            operation: "read directory entry",
            path: directory.clone(),
            source: source.to_string(),
        })?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| ContextFileError::Io {
                operation: "read directory entry",
                path: entry.path(),
                source: "file name is not valid Unicode".to_owned(),
            })?;
        if name == ".git" {
            continue;
        }
        let path_text = if root.as_str() == "." {
            name.clone()
        } else {
            format!("{}/{name}", root.as_str())
        };
        let path = ProjectPath::new(path_text).map_err(|error| ContextFileError::Io {
            operation: "validate directory entry",
            path: entry.path(),
            source: error.to_string(),
        })?;
        if path_is_ignored(checkout, path.as_str()).unwrap_or(false) {
            continue;
        }
        let metadata =
            fs::symlink_metadata(entry.path()).map_err(|source| ContextFileError::Io {
                operation: "inspect directory entry",
                path: entry.path(),
                source: source.to_string(),
            })?;
        let kind = if metadata.file_type().is_symlink() {
            FileTreeEntryKind::Symlink
        } else if metadata.is_dir() {
            FileTreeEntryKind::Directory
        } else {
            FileTreeEntryKind::File
        };
        result.push(FileTreeEntry {
            change: change_for(path.as_str(), kind, changes),
            path,
            name,
            kind,
            size: metadata.len(),
        });
    }
    result.sort_by(|left, right| {
        entry_rank(left.kind)
            .cmp(&entry_rank(right.kind))
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(result)
}

fn entry_rank(kind: FileTreeEntryKind) -> u8 {
    match kind {
        FileTreeEntryKind::Directory => 0,
        FileTreeEntryKind::File => 1,
        FileTreeEntryKind::Symlink => 2,
    }
}

fn change_for(path: &str, kind: FileTreeEntryKind, changes: &[ChangedPath]) -> Option<ChangeKind> {
    changes.iter().find_map(|change| {
        let changed = change.path.display();
        (changed == path
            || (kind == FileTreeEntryKind::Directory
                && changed
                    .strip_prefix(path)
                    .is_some_and(|suffix| suffix.starts_with('/'))))
        .then(|| change.kind.clone())
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedDiff {
    pub text: String,
    pub binary: bool,
}

pub fn render_diff(checkout: &Path, path: &ProjectPath) -> Result<RenderedDiff, ContextFileError> {
    if path_is_tracked(checkout, path.as_str()).unwrap_or(false) {
        return path_diff(checkout, path.as_str())
            .map(|diff| RenderedDiff {
                text: diff.text,
                binary: diff.binary,
            })
            .map_err(|source| ContextFileError::Io {
                operation: "render Git diff",
                path: checkout.join(path.as_str()),
                source,
            });
    }

    match preview_file(checkout, path, MAX_TEXT_PREVIEW_BYTES)? {
        FilePreview::Missing => Ok(RenderedDiff {
            text: format!("File is unavailable: {}", path.as_str()),
            binary: false,
        }),
        FilePreview::Text { content, .. } => Ok(RenderedDiff {
            text: untracked_text_diff(path, &content),
            binary: false,
        }),
        FilePreview::Binary { size, .. } => Ok(RenderedDiff {
            text: format!("Untracked binary file: {} ({size} bytes)", path.as_str()),
            binary: true,
        }),
        FilePreview::TooLarge { size, .. } => Ok(RenderedDiff {
            text: format!(
                "Untracked file is too large to preview: {} ({size} bytes)",
                path.as_str()
            ),
            binary: true,
        }),
    }
}

fn untracked_text_diff(path: &ProjectPath, content: &str) -> String {
    let mut diff = format!(
        "diff --git a/{0} b/{0}\nnew file mode 100644\n--- /dev/null\n+++ b/{0}\n",
        path.as_str()
    );
    for line in content.split_inclusive('\n') {
        diff.push('+');
        diff.push_str(line);
    }
    if !content.is_empty() && !content.ends_with('\n') {
        diff.push_str("\n\\ No newline at end of file\n");
    }
    diff
}

#[cfg(test)]
mod tests {
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
    fn tree_hides_git_and_ignored_entries_and_sorts_directories_first() {
        let checkout = tempfile::tempdir().unwrap();
        git(checkout.path(), &["init", "--quiet"]);
        fs::create_dir(checkout.path().join("src")).unwrap();
        fs::write(checkout.path().join("README.md"), "read me").unwrap();
        fs::write(checkout.path().join("ignored.log"), "ignored").unwrap();
        fs::write(checkout.path().join(".gitignore"), "*.log\n").unwrap();

        let entries =
            list_directory(checkout.path(), &ProjectPath::new(".").unwrap(), &[]).unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            vec!["src", ".gitignore", "README.md"]
        );
    }

    #[test]
    fn renders_untracked_text_and_binary_without_decoding_binary() {
        let checkout = tempfile::tempdir().unwrap();
        git(checkout.path(), &["init", "--quiet"]);
        fs::write(checkout.path().join("new.txt"), "one\ntwo\n").unwrap();
        fs::write(checkout.path().join("new.bin"), b"one\0two").unwrap();

        let text = render_diff(checkout.path(), &ProjectPath::new("new.txt").unwrap()).unwrap();
        assert!(!text.binary);
        assert!(text.text.contains("+one\n+two"));
        let binary = render_diff(checkout.path(), &ProjectPath::new("new.bin").unwrap()).unwrap();
        assert!(binary.binary);
        assert!(binary.text.contains("Untracked binary file"));
    }
}
