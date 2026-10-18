use crate::layout::Layout;
use crate::link::Version;
use crate::path;
use crate::version;
use std::path::PathBuf;

/// Where `command` resolves on the ambient `PATH`, with the shims directory
/// removed so a shim cannot resolve itself.
///
/// This deliberately ignores the selected version. Anything answering "what
/// would the system JDK do" has to ask the system, and `resolve` would
/// otherwise hand back the selected version's `bin/`.
pub fn resolve_system(layout: &Layout, command: &str) -> Option<PathBuf> {
    let dirs = path::path_without(&layout.shims_dir(), &path::path_dirs());
    path::which_in_path(command, &dirs)
}

/// Is there a java reachable without going through jenv's shims?
pub fn has_system_java(layout: &Layout) -> bool {
    resolve_system(layout, "java").is_some()
}

/// Where `command` resolves for the active version.
///
/// jenv prefers the active version's own `bin/`, and falls back to `PATH` with
/// the shims directory removed so that a shim cannot resolve itself. The
/// version's own path is left unresolved: `jenv which` prints what it will
/// invoke, and that is a path under `$JENV_ROOT/versions/`.
///
/// `None` means "nowhere". Answering with the bare command name instead — a
/// relative path that `is_executable` then judges against the working
/// directory — made `jenv which` report success for a local file that would
/// not run, and gave `jenv exec` the wrong exit status.
pub fn resolve(layout: &Layout, command: &str) -> Option<PathBuf> {
    let selection = version::select(layout).ok()?;

    if let Version::Installed { name, .. } = &selection.version {
        let bin = crate::link::registered_path(layout, name).join("bin");
        if let Some(found) = path::executable_names(&bin.join(command))
            .into_iter()
            .find(|candidate| path::is_executable(candidate))
        {
            return Some(found);
        }
    }

    let dirs = path::path_without(&layout.shims_dir(), &path::path_dirs());
    path::which_in_path(command, &dirs)
}

pub fn which(layout: &Layout, command: &str) -> Result<(), String> {
    if let Some(path) = resolve(layout, command) {
        println!("{}", path.display());
        return Ok(());
    }

    // The hint is what makes this actionable, so it goes in the message
    // rather than straight to stderr: `main` is what prints.
    let mut message = format!("jenv: {command}: command not found");
    let elsewhere = whence(layout, command);
    if !elsewhere.is_empty() {
        message.push_str("\n\n");
        message.push_str(&format!(
            "The `{command}' command exists in these Java versions:"
        ));
        for name in elsewhere {
            message.push_str(&format!("\n   {name}"));
        }
    }
    Err(message)
}

pub fn whence(layout: &Layout, command: &str) -> Vec<String> {
    let mut names = Vec::new();
    for name in crate::link::list(layout) {
        let Some(home) = crate::link::home_of(layout, &name) else {
            continue;
        };
        let found = path::executable_names(&home.join("bin").join(command))
            .into_iter()
            .any(|candidate| path::is_executable(&candidate));
        if found {
            names.push(name);
        }
    }
    names
}

pub fn shims(layout: &Layout, short: bool) {
    let Ok(entries) = std::fs::read_dir(layout.shims_dir()) else {
        return;
    };

    let mut names: Vec<String> = entries
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| !name.starts_with('.'))
        .map(|name| {
            if short {
                name
            } else {
                format!("{}/{name}", layout.shims_dir().display())
            }
        })
        .collect();
    names.sort();

    for name in names {
        println!("{name}");
    }
}
