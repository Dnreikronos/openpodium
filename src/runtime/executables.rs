use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;

pub(super) fn executable_exists(executable: &OsStr) -> bool {
    let path = Path::new(executable);
    if path.components().count() > 1 {
        return is_executable(path);
    }
    let Some(search_path) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&search_path).any(|directory| executable_candidates(&directory, executable))
}

fn executable_candidates(directory: &Path, executable: &OsStr) -> bool {
    let candidate = directory.join(executable);
    if is_executable(&candidate) {
        return true;
    }
    #[cfg(windows)]
    {
        let extensions = env::var_os("PATHEXT").unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".into());
        extensions.to_string_lossy().split(';').any(|extension| {
            is_executable(&candidate.with_extension(extension.trim_start_matches('.')))
        })
    }
    #[cfg(not(windows))]
    false
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(windows)]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}
