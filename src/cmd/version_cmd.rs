use crate::cli::Command;
use crate::layout::Layout;
use crate::link;

use crate::version;
use std::path::PathBuf;

pub fn root(layout: &Layout) {
    println!("{}", layout.root.display());
}

pub fn version_file(layout: &Layout) {
    println!("{}", version::version_file(layout, &jenv_dir()).display());
}

pub fn version_origin(layout: &Layout) {
    match version::select(layout) {
        Ok(selection) => println!("{}", selection.origin),
        Err(e) => eprintln!("{e}"),
    }
}

pub fn version_name(layout: &Layout) -> Result<(), String> {
    println!("{}", version::select(layout)?.version.name());
    Ok(())
}

pub fn version(layout: &Layout) -> Result<(), String> {
    // One line, like jenv: the prefix has its own command.
    let selection = version::select(layout)?;
    println!("{} (set by {})", selection.version.name(), selection.origin);
    Ok(())
}

pub fn versions(layout: &Layout, bare: bool, verbose: bool) -> Result<(), String> {
    let current = if bare {
        String::new()
    } else {
        version::select(layout)
            .map(|s| s.version.name().to_string())
            .unwrap_or_default()
    };
    let current_origin = if current.is_empty() {
        None
    } else {
        version::select(layout).ok().map(|s| s.origin)
    };

    // In bare mode the output is a plain list for scripts to consume, so the
    // `*` marker and its padding both have to go.
    let marker = |name: &str| -> String {
        if bare {
            name.to_string()
        } else if name == current {
            // jenv annotates only the active version, and this is the one
            // place `jenv version` is echoed into a listing.
            match &current_origin {
                Some(origin) => format!("* {name} (set by {origin})"),
                None => format!("* {name}"),
            }
        } else {
            format!("  {name}")
        }
    };

    if !bare && crate::cmd::which::has_system_java(layout) {
        println!("{}", marker("system"));
    }

    for name in link::list(layout) {
        println!("{}", marker(&name));

        if verbose {
            let entry = layout.versions_dir().join(&name);
            println!("         {}", entry.display());
            if let Some(home) = link::home_of(layout, &name) {
                println!("         --> {}", home.display());
            }
            println!();
        }
    }
    Ok(())
}

pub fn prefix(layout: &Layout, requested: Option<String>) -> Result<(), String> {
    let name = match requested {
        Some(name) => name,
        None => version::select(layout)?.version.name().to_string(),
    };

    if name == "system" {
        // The system java's home is wherever `java` points on the ambient
        // PATH, minus /bin. Asking `which::resolve` would consult the selected
        // version first and answer with that instead, which is the opposite of
        // what `system` means.
        let java = crate::cmd::which::resolve_system(layout, "java")
            .ok_or_else(|| "jenv: no system java on PATH".to_string())?;
        let Some(bin) = java.parent() else {
            return Err("jenv: could not derive a prefix from the system java".into());
        };
        println!("{}", bin.parent().unwrap_or(bin).display());
        return Ok(());
    }

    // The registration path, not the JDK's real location: jenv prints this
    // and exports it as JAVA_HOME so that a path pinned elsewhere keeps
    // naming the same thing when the JDK is re-registered.
    let entry = link::registered_path(layout, &name);
    if link::home_of(layout, &name).is_none() {
        return Err(format!("jenv: version `{name}' not installed"));
    }
    println!("{}", entry.display());
    Ok(())
}

/// The directory a hook script should treat as the project root, matching
/// jenv's own `$JENV_DIR`.
pub fn jenv_dir() -> PathBuf {
    std::env::var_os("JENV_DIR")
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir().unwrap_or_default())
}

pub fn jenv_shell() -> String {
    std::env::var("JENV_SHELL").unwrap_or_else(|_| default_shell())
}

/// Best-effort detection of the shell that invoked us, used only to pick a
/// hook-script dialect; an explicit `jenv init <shell>` always wins.
pub fn default_shell() -> String {
    #[cfg(windows)]
    {
        if std::env::var_os("PSModulePath").is_some() {
            return "powershell".into();
        }
    }

    std::env::var("SHELL")
        .map(|s| s.rsplit('/').next().unwrap_or("bash").to_string())
        .unwrap_or_else(|_| "bash".into())
}

pub fn dispatch(layout: &Layout, command: Command) -> Result<(), String> {
    match command {
        Command::Root => root(layout),
        Command::Version => version(layout)?,
        Command::VersionName => version_name(layout)?,
        Command::VersionOrigin => version_origin(layout),
        Command::VersionFile => version_file(layout),
        Command::Versions { bare, verbose } => versions(layout, bare, verbose)?,
        Command::Prefix { version } => prefix(layout, version)?,
        _ => unreachable!("dispatch is only reached for the commands above"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_global_file_name_follows_jenvs_legacy_precedence() {
        let sandbox = std::env::temp_dir().join(format!("jenv-rs-gv-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&sandbox);
        let layout = Layout::with_root(&sandbox);
        let from = sandbox.join("project");

        // With nothing configured the chain still names the file it would create.
        assert_eq!(layout.global_version_file(), sandbox.join("version"));

        // `default` is the oldest name and is only consulted last.
        std::fs::create_dir_all(&sandbox).unwrap();
        std::fs::write(sandbox.join("default"), "17").unwrap();
        assert_eq!(layout.global_version_file(), sandbox.join("default"));

        std::fs::write(sandbox.join("global"), "21").unwrap();
        assert_eq!(layout.global_version_file(), sandbox.join("global"));

        std::fs::write(sandbox.join("version"), "21.0.2").unwrap();
        assert_eq!(layout.global_version_file(), sandbox.join("version"));

        // A local file anywhere above the start directory beats all of them.
        std::fs::create_dir_all(from.join("a/b")).unwrap();
        std::fs::write(from.join(".java-version"), "21").unwrap();
        assert_eq!(
            version::version_file(&layout, &from.join("a/b")),
            from.join(".java-version")
        );

        let _ = std::fs::remove_dir_all(&sandbox);
    }
}
