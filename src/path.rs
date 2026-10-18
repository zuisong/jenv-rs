//! PATH manipulation and executable lookup.
//!
//! Shared by `which`, `exec` and `rehash`: every command that needs to find
//! an executable on `PATH`, or to remove the shims directory from it, goes
//! through here.

use std::path::{Path, PathBuf};

/// The platform's `PATH` separator.
pub const SEPARATOR: char = if cfg!(windows) { ';' } else { ':' };

/// The directories on `PATH`, split and ready to search.
pub fn path_dirs() -> Vec<PathBuf> {
    match std::env::var_os("PATH") {
        Some(value) => std::env::split_paths(&value).collect(),
        None => Vec::new(),
    }
}

/// The first `PATH` entry that provides `name`, with each directory expanded
/// to a real path so that symlinked prefixes (notably `/tmp` on macOS, and
/// every Windows junction) compare equal.
pub fn which_in_path(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    for dir in dirs {
        if dir.as_os_str().is_empty() {
            continue;
        }
        let real_dir = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.clone());
        for candidate in executable_names(&real_dir.join(name)) {
            if is_executable(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

/// The names `stem` can carry on this platform. Unix runs the bare name.
/// Windows runs the `.exe` and only the `.exe`: probing it directly is faster
/// than consulting `PATHEXT`, and immune to a `PATHEXT` the caller mangled.
pub fn executable_names(stem: &Path) -> Vec<PathBuf> {
    if cfg!(windows) {
        vec![stem.with_extension("exe")]
    } else {
        vec![stem.to_path_buf()]
    }
}

/// Is `path` a file this platform would execute?
pub fn is_executable(path: &Path) -> bool {
    path.is_file() && has_permission_to_run(path)
}

#[cfg(unix)]
fn has_permission_to_run(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Windows has no execute bit: what runs is decided by the file extension.
/// `executable_names` probes `.exe` and `rehash` links shims as `.exe`, so
/// that extension is the whole of jenv's definition of an executable.
#[cfg(windows)]
fn has_permission_to_run(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
}

/// The `PATH` with `victim` removed, compared by real path.
///
/// A shim on `PATH` shadows the very command being looked up, so every
/// `which` has to answer "what would this resolve to without jenv?".
pub fn path_without(victim: &Path, dirs: &[PathBuf]) -> Vec<PathBuf> {
    let real_victim = std::fs::canonicalize(victim).unwrap_or_else(|_| victim.to_path_buf());
    dirs.iter()
        .filter(|dir| {
            if dir.as_os_str().is_empty() {
                return true;
            }
            std::fs::canonicalize(dir).unwrap_or_else(|_| (*dir).clone()) != real_victim
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_without_drops_the_shims_dir() {
        let tmp = std::env::temp_dir().join(format!("jenv-rs-path-{}", std::process::id()));
        let shims = tmp.join("shims");
        let other = tmp.join("other");
        std::fs::create_dir_all(&shims).unwrap();
        std::fs::create_dir_all(&other).unwrap();

        let dirs = vec![shims.clone(), other.clone()];
        assert_eq!(path_without(&shims, &dirs), vec![other]);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn which_finds_an_executable_and_skips_non_executables() {
        let tmp = std::env::temp_dir().join(format!("jenv-rs-which-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let file = tmp.join("tool");
        std::fs::write(&file, "#!/bin/sh\n").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert!(!is_executable(&file), "a plain file must not count");
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
            assert!(is_executable(&file));
            // `which_in_path` canonicalizes the directory it searched, so a
            // `/tmp` that is really `/private/tmp` still compares equal.
            let expected = std::fs::canonicalize(&file).unwrap();
            assert_eq!(
                which_in_path("tool", std::slice::from_ref(&tmp)),
                Some(expected)
            );
        }
        #[cfg(windows)]
        {
            // Windows runs by extension: a bare `tool` is not executable, and
            // a `tool.exe` is. Only the `.exe` is probed.
            assert!(
                !is_executable(&file),
                "an extensionless file must not count"
            );
            let exe = tmp.join("tool.exe");
            std::fs::write(&exe, "").unwrap();
            assert!(is_executable(&exe));
            assert!(which_in_path("tool", std::slice::from_ref(&tmp)).is_some());
        }

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
