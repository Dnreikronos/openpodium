//! Safe filesystem access for file-backed canvas nodes.

mod files;
pub use files::*;

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use crate::domain::ProjectPath;

pub const MAX_TEXT_PREVIEW_BYTES: usize = 1_048_576;
pub const CONNECTED_NOTES_ENV: &str = "OPENPODIUM_CONNECTED_NOTES";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileRevision {
    Missing,
    Present([u8; 32]),
}

impl FileRevision {
    fn for_bytes(bytes: &[u8]) -> Self {
        Self::Present(*blake3::hash(bytes).as_bytes())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilePreview {
    Missing,
    Text {
        content: String,
        revision: FileRevision,
    },
    Binary {
        size: u64,
        revision: FileRevision,
    },
    TooLarge {
        size: u64,
        max_bytes: usize,
    },
}

pub fn preview_file(
    checkout: &Path,
    path: &ProjectPath,
    max_bytes: usize,
) -> Result<FilePreview, ContextFileError> {
    let resolved = resolve_project_path(checkout, path)?;
    let mut file = match File::open(&resolved) {
        Ok(file) => file,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(FilePreview::Missing),
        Err(source) => return Err(io_error("open", &resolved, source)),
    };
    let size = file
        .metadata()
        .map_err(|source| io_error("inspect", &resolved, source))?
        .len();
    if size > max_bytes as u64 {
        return Ok(FilePreview::TooLarge { size, max_bytes });
    }

    let mut bytes = Vec::with_capacity(size as usize);
    Read::by_ref(&mut file)
        .take(max_bytes.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|source| io_error("read", &resolved, source))?;
    if bytes.len() > max_bytes {
        return Ok(FilePreview::TooLarge {
            size: bytes.len() as u64,
            max_bytes,
        });
    }
    let revision = FileRevision::for_bytes(&bytes);
    if bytes.contains(&0) {
        return Ok(FilePreview::Binary { size, revision });
    }
    match String::from_utf8(bytes) {
        Ok(content) => Ok(FilePreview::Text { content, revision }),
        Err(error) => Ok(FilePreview::Binary {
            size: error.as_bytes().len() as u64,
            revision,
        }),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteBuffer {
    path: ProjectPath,
    text: String,
    observed: FileRevision,
    dirty: bool,
    external_change: Option<FileRevision>,
}

impl NoteBuffer {
    pub fn load(checkout: &Path, path: ProjectPath) -> Result<Self, ContextFileError> {
        let (text, observed) = read_note(checkout, &path)?;
        Ok(Self {
            path,
            text,
            observed,
            dirty: false,
            external_change: None,
        })
    }

    pub fn new(path: ProjectPath) -> Self {
        Self {
            path,
            text: String::new(),
            observed: FileRevision::Missing,
            dirty: true,
            external_change: None,
        }
    }

    pub const fn path(&self) -> &ProjectPath {
        &self.path
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub const fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub const fn has_external_change(&self) -> bool {
        self.external_change.is_some()
    }

    pub fn edit(&mut self, text: impl Into<String>) {
        let text = text.into();
        if self.text != text {
            self.text = text;
            self.dirty = true;
        }
    }

    pub fn refresh(&mut self, checkout: &Path) -> Result<bool, ContextFileError> {
        let (text, revision) = read_note(checkout, &self.path)?;
        if revision == self.observed {
            return Ok(false);
        }
        if self.dirty {
            self.external_change = Some(revision);
            return Ok(false);
        }
        self.text = text;
        self.observed = revision;
        self.external_change = None;
        Ok(true)
    }

    pub fn reload_discarding_edits(&mut self, checkout: &Path) -> Result<(), ContextFileError> {
        let (text, revision) = read_note(checkout, &self.path)?;
        self.text = text;
        self.observed = revision;
        self.dirty = false;
        self.external_change = None;
        Ok(())
    }

    pub fn save(&mut self, checkout: &Path, overwrite: bool) -> Result<(), ContextFileError> {
        let (_, current) = read_note(checkout, &self.path)?;
        if !overwrite && current != self.observed {
            self.external_change = Some(current);
            return Err(ContextFileError::Conflict {
                expected: self.observed,
                actual: current,
            });
        }
        atomic_write(checkout, &self.path, self.text.as_bytes())?;
        self.observed = FileRevision::for_bytes(self.text.as_bytes());
        self.dirty = false;
        self.external_change = None;
        Ok(())
    }
}

fn read_note(
    checkout: &Path,
    path: &ProjectPath,
) -> Result<(String, FileRevision), ContextFileError> {
    match preview_file(checkout, path, MAX_TEXT_PREVIEW_BYTES)? {
        FilePreview::Missing => Ok((String::new(), FileRevision::Missing)),
        FilePreview::Text { content, revision } => Ok((content, revision)),
        FilePreview::Binary { .. } => Err(ContextFileError::BinaryNote),
        FilePreview::TooLarge { size, max_bytes } => {
            Err(ContextFileError::TooLarge { size, max_bytes })
        }
    }
}

fn atomic_write(checkout: &Path, path: &ProjectPath, bytes: &[u8]) -> Result<(), ContextFileError> {
    let destination = resolve_project_path(checkout, path)?;
    let parent = destination
        .parent()
        .ok_or_else(|| ContextFileError::EscapesCheckout(destination.clone()))?;
    fs::create_dir_all(parent).map_err(|source| io_error("create directory", parent, source))?;
    ensure_inside_checkout(checkout, parent)?;

    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|source| io_error("create temporary note", parent, source))?;
    temporary
        .write_all(bytes)
        .map_err(|source| io_error("write temporary note", temporary.path(), source))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|source| io_error("sync temporary note", temporary.path(), source))?;
    temporary
        .persist(&destination)
        .map_err(|error| io_error("replace", &destination, error.error))?;
    Ok(())
}

pub fn resolve_project_path(
    checkout: &Path,
    path: &ProjectPath,
) -> Result<PathBuf, ContextFileError> {
    let checkout = dunce::canonicalize(checkout)
        .map_err(|source| io_error("resolve checkout", checkout, source))?;
    let candidate = checkout.join(path.as_str());
    let existing = nearest_existing_ancestor(&candidate)?;
    let canonical = dunce::canonicalize(existing)
        .map_err(|source| io_error("resolve path", existing, source))?;
    if !canonical.starts_with(&checkout) {
        return Err(ContextFileError::EscapesCheckout(candidate));
    }
    if candidate.exists() {
        let canonical = dunce::canonicalize(&candidate)
            .map_err(|source| io_error("resolve path", &candidate, source))?;
        if !canonical.starts_with(&checkout) {
            return Err(ContextFileError::EscapesCheckout(candidate));
        }
        Ok(canonical)
    } else {
        Ok(candidate)
    }
}

fn nearest_existing_ancestor(path: &Path) -> Result<&Path, ContextFileError> {
    let mut current = path;
    loop {
        if current.exists() {
            return Ok(current);
        }
        current = current
            .parent()
            .ok_or_else(|| ContextFileError::EscapesCheckout(path.to_owned()))?;
    }
}

fn ensure_inside_checkout(checkout: &Path, path: &Path) -> Result<(), ContextFileError> {
    let checkout = dunce::canonicalize(checkout)
        .map_err(|source| io_error("resolve checkout", checkout, source))?;
    let path =
        dunce::canonicalize(path).map_err(|source| io_error("resolve path", path, source))?;
    if path.starts_with(checkout) {
        Ok(())
    } else {
        Err(ContextFileError::EscapesCheckout(path))
    }
}

fn io_error(operation: &'static str, path: &Path, source: io::Error) -> ContextFileError {
    ContextFileError::Io {
        operation,
        path: path.to_owned(),
        source: source.to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextFileError {
    EscapesCheckout(PathBuf),
    BinaryNote,
    TooLarge {
        size: u64,
        max_bytes: usize,
    },
    Conflict {
        expected: FileRevision,
        actual: FileRevision,
    },
    Io {
        operation: &'static str,
        path: PathBuf,
        source: String,
    },
}

impl Display for ContextFileError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::EscapesCheckout(path) => {
                write!(formatter, "path escapes the checkout: {}", path.display())
            }
            Self::BinaryNote => formatter.write_str("note file is binary or is not valid UTF-8"),
            Self::TooLarge { size, max_bytes } => {
                write!(formatter, "file has {size} bytes; limit is {max_bytes}")
            }
            Self::Conflict { .. } => {
                formatter.write_str("file changed externally; reload or overwrite explicitly")
            }
            Self::Io {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "failed to {operation} {}: {source}",
                path.display()
            ),
        }
    }
}

impl Error for ContextFileError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn note_path() -> ProjectPath {
        ProjectPath::new(".openpodium/notes/7.md").unwrap()
    }

    #[test]
    fn previews_text_without_decoding_binary_or_oversized_files() {
        let checkout = tempfile::tempdir().unwrap();
        fs::write(checkout.path().join("text.md"), "hello").unwrap();
        fs::write(checkout.path().join("binary"), b"hello\0world").unwrap();
        fs::write(checkout.path().join("invalid"), [0xff, 0xfe]).unwrap();

        assert!(matches!(
            preview_file(
                checkout.path(),
                &ProjectPath::new("text.md").unwrap(),
                32
            )
            .unwrap(),
            FilePreview::Text { content, .. } if content == "hello"
        ));
        for name in ["binary", "invalid"] {
            assert!(matches!(
                preview_file(checkout.path(), &ProjectPath::new(name).unwrap(), 32).unwrap(),
                FilePreview::Binary { .. }
            ));
        }
        assert!(matches!(
            preview_file(checkout.path(), &ProjectPath::new("text.md").unwrap(), 2).unwrap(),
            FilePreview::TooLarge { .. }
        ));
    }

    #[test]
    fn dirty_note_buffers_keep_edits_when_the_file_changes() {
        let checkout = tempfile::tempdir().unwrap();
        let mut note = NoteBuffer::new(note_path());
        note.edit("first");
        note.save(checkout.path(), false).unwrap();
        note.edit("local edit");

        fs::write(
            checkout.path().join(".openpodium/notes/7.md"),
            "external edit",
        )
        .unwrap();
        assert!(!note.refresh(checkout.path()).unwrap());
        assert_eq!(note.text(), "local edit");
        assert!(note.has_external_change());
        assert!(matches!(
            note.save(checkout.path(), false),
            Err(ContextFileError::Conflict { .. })
        ));

        note.save(checkout.path(), true).unwrap();
        assert_eq!(
            fs::read_to_string(checkout.path().join(".openpodium/notes/7.md")).unwrap(),
            "local edit"
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_cannot_escape_the_checkout() {
        use std::os::unix::fs::symlink;

        let checkout = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), checkout.path().join("escape")).unwrap();
        let path = ProjectPath::new("escape/note.md").unwrap();

        assert!(matches!(
            resolve_project_path(checkout.path(), &path),
            Err(ContextFileError::EscapesCheckout(_))
        ));
    }
}
