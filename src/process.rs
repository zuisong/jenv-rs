//! Process execution: replace or spawn, with platform-specific signal handling.

use std::path::Path;

/// Restore the default `SIGPIPE` behaviour, which Rust's runtime turns off.
///
/// Rust ignores `SIGPIPE` so that a write to a closed pipe returns `EPIPE`
/// instead of killing the process, which turns `jenv version-name | head -1`
/// into a panic. Every other Unix tool dies quietly on that pipe, so restore
/// the default before either the CLI or a shim can write to one.
#[cfg(unix)]
pub fn restore_default_sigpipe() {
    // SAFETY: a valid signal number and the constant `SIG_DFL` disposition;
    // no handler is installed and no Rust state is touched.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

/// Nothing to restore: Windows has no `SIGPIPE`.
#[cfg(not(unix))]
pub fn restore_default_sigpipe() {}

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
