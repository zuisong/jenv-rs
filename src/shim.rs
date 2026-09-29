//! The shim: the same binary as the CLI, linked under the name of every
//! command in every registered JDK's `bin/`.
//!
//! There is one file to install. `jenv rehash` hard-links *this* executable
//! into `shims/`, and the shim reads its own file name to decide whether it is
//! being run as `jenv` or as, say, `java`. Being the same inode is not a trick
//! to save disk space — it is what makes it impossible for a shim and the CLI
//! that manages it to be different builds.

use std::path::{Path, PathBuf};

/// The command this executable stands for, taken from the name it was invoked
/// as, or `None` when it is being run as the CLI.
///
/// `argv[0]`'s file name is the only thing that distinguishes `java` from
/// `javac` at runtime, and the only thing that distinguishes either from
/// `jenv`.
pub fn tool_name() -> Option<String> {
    let name = std::env::current_exe()
        .ok()?
        .file_name()?
        .to_string_lossy()
        .to_string();

    let stem = if cfg!(windows) {
        name.strip_suffix(".exe").unwrap_or(&name)
    } else {
        &name
    };

    if stem.is_empty() || stem == "jenv" || stem.starts_with('.') || stem.contains(".tmp-") {
        return None;
    }
    Some(stem.to_string())
}

/// Run as a shim: hand this process straight to `tool`.
///
/// Nothing is interpreted on the way, which is the whole point — the shell
/// implementation spent a dozen bash interpreters per `java` invocation
/// before this.
pub fn run(tool: &str) -> ! {
    // The shim inherits stdout from the tool it dispatches to, so a tool piped
    // into `head` must still get the normal Unix behaviour.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    let mut argv = Vec::with_capacity(std::env::args().count());
    argv.push(tool.to_string());
    argv.extend(std::env::args().skip(1));

    if let Some(dir) = jar_directory(tool) {
        // SAFETY: nothing else is running; the shim has not started a
        // thread, and `exec` below replaces the process outright.
        unsafe { std::env::set_var("JENV_DIR", dir) };
    }

    let layout = crate::layout::Layout::from_env();
    if let Err(message) = crate::cmd::exec::exec(&layout, &argv) {
        eprintln!("{message}");
        std::process::exit(1);
    }
    // `exec` replaces the process; this is only reached if it refused to.
    std::process::exit(1);
}

/// `java -jar path/to/app.jar` runs with a working directory the user chose,
/// not one under `$JAVA_HOME`. jenv exposes that directory as `$JENV_DIR` so
/// project-local configuration can be found.
///
/// Stops at the first argument that is not an existing path, and never past
/// the first flag, so `java -version` does not pick up the current directory.
fn jar_directory(tool: &str) -> Option<PathBuf> {
    if tool != "java" {
        return None;
    }
    for arg in std::env::args_os().skip(1) {
        let text = arg.to_string_lossy();
        if text.starts_with('-') {
            return None;
        }
        let path = Path::new(&arg);
        if text.contains('/') || text.contains('\\') {
            if path.is_file() {
                return path.parent().map(Path::to_path_buf);
            }
            return None;
        }
    }
    None
}
