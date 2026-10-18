//! The shim: the same binary as the CLI, linked under the name of every
//! command in every registered JDK's `bin/`.
//!
//! There is one file to install. `jenv rehash` hard-links *this* executable
//! into `shims/`, and the shim reads its own file name to decide whether it is
//! being run as `jenv` or as, say, `java`. Being the same inode is not a trick
//! to save disk space — it is what makes it impossible for a shim and the CLI
//! that manages it to be different builds.

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
/// before this. Nor is anything converted: the arguments are taken as
/// `OsString` so a filename that is not UTF-8 reaches the tool unchanged
/// instead of panicking this process.
pub fn run(tool: &str) -> ! {
    let mut argv = Vec::with_capacity(std::env::args_os().count());
    argv.push(std::ffi::OsString::from(tool));
    argv.extend(std::env::args_os().skip(1));

    let env = crate::env::Env::process();
    let layout = crate::layout::Layout::from_env(&env);
    if let Err(message) = crate::cmd::exec::exec(&layout, &env, &argv) {
        eprintln!("{message}");
        std::process::exit(1);
    }
    // `exec` replaces the process; this is only reached if it refused to.
    std::process::exit(1);
}
