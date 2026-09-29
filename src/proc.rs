//! Process and `PATH` helpers shared by `which`, `exec` and `rehash`.

use std::path::{Path, PathBuf};

pub const SEPARATOR: char = if cfg!(windows) { ';' } else { ':' };

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

pub fn which(name: &str) -> Option<PathBuf> {
    which_in_path(name, &path_dirs())
}

/// The other jenv-rs binary that goes with the running one: the file sitting
/// next to `current_exe`, or failing that whatever `PATH` offers.
///
/// jenv-rs imposes no install layout. The two binaries only have to be
/// findable, and a sibling is preferred over `PATH` so that a developer
/// running `target/release/jenv` rehashes from that same build rather than
/// from whatever happens to be installed.
pub fn sibling_binary(name: &str) -> Option<PathBuf> {
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        && let Some(found) = which_in_path(name, std::slice::from_ref(&dir))
    {
        return Some(found);
    }
    which(name)
}

/// The name `stem` carries on this platform: `jenv` versus `jenv.exe`.
pub fn file_name(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_string()
    }
}

/// `java` on unix, `java.exe` then `java` on Windows. Windows resolves
/// executables through `PATHEXT`, but probing the extensions directly is both
/// faster and immune to a `PATHEXT` the caller has mangled.
pub fn executable_names(stem: &Path) -> Vec<PathBuf> {
    if cfg!(windows) {
        vec![stem.with_extension("exe"), stem.to_path_buf()]
    } else {
        vec![stem.to_path_buf()]
    }
}

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

/// Windows has no execute bit: `CreateProcess` decides what runs from the file
/// extension, and everything jenv hands it already has one.
#[cfg(not(unix))]
fn has_permission_to_run(_path: &Path) -> bool {
    true
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

/// Replace this process with `command`, forwarding signals to it.
///
/// On unix `execvp` hands the process over outright, so the JVM inherits
/// jenv's process group and terminal. Windows has no equivalent, and sends
/// CTRL_C to every process attached to the console, so the shim must opt out
/// of the default handler or the tool is killed the moment the user hits
/// Ctrl-C.
pub fn exec_replace(mut command: std::process::Command) -> std::io::Error {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.exec()
    }

    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;
        use windows_sys::core::BOOL;

        unsafe extern "system" fn passthrough(_: u32) -> BOOL {
            // TRUE: we handled it, i.e. deliberately ignored it.
            1
        }

        unsafe {
            SetConsoleCtrlHandler(Some(passthrough), 1);
        }

        match command.spawn() {
            Ok(mut child) => {
                let status = child.wait();
                unsafe {
                    SetConsoleCtrlHandler(Some(passthrough), 0);
                }
                match status {
                    Ok(status) => std::process::exit(status.code().unwrap_or(1)),
                    Err(e) => e,
                }
            }
            Err(e) => {
                unsafe {
                    SetConsoleCtrlHandler(Some(passthrough), 0);
                }
                e
            }
        }
    }
}

/// Build a `Command` that runs `program` under the display name `argv0`.
///
/// `jenv exec` re-labels the tool so that `ps` and the JVM's own diagnostics
/// show `java`, not a path ending in `/versions/temurin64-21.0.2.13/bin/java`.
pub fn command_with_argv0(program: &Path, argv0: &str) -> std::process::Command {
    let mut command = std::process::Command::new(program);
    set_argv0(&mut command, argv0);
    command
}

#[cfg(unix)]
fn set_argv0(command: &mut std::process::Command, argv0: &str) {
    use std::os::unix::process::CommandExt;
    command.arg0(argv0);
}

/// Windows has no settable argv[0]; the process reports the path it was
/// launched with, which is already the tool name.
#[cfg(not(unix))]
fn set_argv0(_command: &mut std::process::Command, _argv0: &str) {}

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
        #[cfg(not(unix))]
        assert!(is_executable(&file));

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
