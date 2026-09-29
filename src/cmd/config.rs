use crate::layout::Layout;
use crate::link;
use crate::version;
use std::path::{Path, PathBuf};

/// `jenv global [version]`
pub fn global(layout: &Layout, requested: Option<String>, unset: bool) -> Result<(), String> {
    let file = layout.global_version_file();

    if unset {
        return match std::fs::remove_file(&file) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("jenv: {e}")),
        };
    }

    let Some(requested) = requested else {
        return match version::read_version_file(&file) {
            Some(name) => {
                println!("{name}");
                Ok(())
            }
            None => Err("jenv: no global version configured".into()),
        };
    };

    write_version_file(layout, &file, &requested)
}

/// `jenv local [version]`
pub fn local(layout: &Layout, requested: Option<String>, unset: bool) -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|e| format!("jenv: {e}"))?;
    let java_version = cwd.join(".java-version");
    let legacy = cwd.join(".jenv-version");

    if unset {
        for file in [&java_version, &legacy] {
            let _ = std::fs::remove_file(file);
        }
        return Ok(());
    }

    let Some(requested) = requested else {
        for file in [&java_version, &legacy] {
            if let Some(name) = version::read_version_file(file) {
                println!("{name}");
                return Ok(());
            }
        }
        return Err("jenv: no local version configured for this directory".into());
    };

    // A `.jenv-version` in the way gets migrated rather than silently
    // shadowed, so a project that predates `.java-version` keeps working.
    if version::read_version_file(&legacy).is_some() {
        std::fs::remove_file(&legacy).map_err(|e| format!("jenv: {e}"))?;
        eprintln!("jenv: removed existing `.jenv-version' file and migrated");
        eprintln!("       local version specification to `.java-version' file");
    }

    write_version_file(layout, &java_version, &requested)
}

/// `jenv shell [version]`
///
/// The parent shell is the only process that can change its own environment,
/// so this prints the line for the wrapper to `eval`.
pub fn shell(layout: &Layout, requested: Option<String>, unset: bool) -> Result<(), String> {
    let shell = crate::cmd::version_cmd::jenv_shell();

    if unset {
        println!(
            "{}",
            match shell.as_str() {
                "fish" => "set -e JENV_VERSION; or true",
                _ => "unset JENV_VERSION",
            }
        );
        return Ok(());
    }

    let Some(requested) = requested else {
        return match std::env::var("JENV_VERSION").ok().filter(|v| !v.is_empty()) {
            Some(current) => {
                println!("{current}");
                Ok(())
            }
            None => Err("jenv: no shell-specific version configured".into()),
        };
    };

    // Refuse to pin a shell session to a version that is not there, which is
    // what makes `jenv shell` fail loudly instead of at the next `java`.
    if !link::exists(layout, &requested) {
        return Err(format!("jenv: version `{requested}' not installed"));
    }

    println!(
        "{}",
        match shell.as_str() {
            "fish" => format!("set -gx JENV_VERSION \"{requested}\""),
            _ => format!("export JENV_VERSION=\"{requested}\""),
        }
    );
    Ok(())
}

fn write_version_file(layout: &Layout, file: &Path, requested: &str) -> Result<(), String> {
    // Check the name is real, but print nothing: `jenv local 21` is quiet on
    // success, and the prefix it validates against is not what the user asked
    // to see.
    if link::home_of(layout, requested).is_none() {
        return Err(format!("jenv: version `{requested}' not installed"));
    }
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("jenv: {e}"))?;
    }
    std::fs::write(file, format!("{requested}\n")).map_err(|e| format!("jenv: {e}"))
}

/// `jenv options [--verbose]` — the extra arguments prepended to every run.
pub fn options(layout: &Layout, verbose: bool) {
    if let Some(from_env) = std::env::var("JENV_OPTIONS").ok().filter(|v| !v.is_empty()) {
        return print_options(
            &from_env,
            "JENV_VERSION_OPTIONS environment variable",
            verbose,
        );
    }

    let file = options_file(layout);
    match version::read_version_file(&file) {
        Some(value) => {
            let origin = file.display().to_string();
            print_options(&value, &origin, verbose)
        }
        None => println!(),
    }
}

/// The resolved options, for `jenv exec` to prepend to the tool's argv.
pub fn options_value(layout: &Layout) -> Option<String> {
    if let Some(from_env) = std::env::var("JENV_OPTIONS").ok().filter(|v| !v.is_empty()) {
        return Some(from_env);
    }
    version::read_version_file(&options_file(layout))
}

fn print_options(value: &str, origin: &str, verbose: bool) {
    if verbose {
        println!("{value}  (set by {origin})");
    } else {
        println!("{value}");
    }
}

fn options_file(layout: &Layout) -> PathBuf {
    for start in [crate::cmd::version_cmd::jenv_dir(), PathBuf::from(".")] {
        let mut current = Some(start);
        while let Some(dir) = current {
            let candidate = dir.join(".java-options");
            if candidate.exists() {
                return candidate;
            }
            current = dir.parent().map(Path::to_path_buf);
        }
    }
    layout.root.join("options")
}

/// `jenv remove <version>...`
pub fn remove(layout: &Layout, versions: &[String]) -> Result<(), String> {
    let mut removed = false;
    for name in versions {
        if link::home_of(layout, name).is_some() {
            link::unregister(layout, name).map_err(|e| format!("jenv: {e}"))?;
            println!("JDK {name} removed");
            removed = true;
        } else {
            // Not an error when it is one of several names, but never a
            // silent success either: a script that removed nothing has to be
            // able to tell.
            println!("{name} is not a managed version of Java ");
        }
    }
    if !removed && !versions.is_empty() {
        return Err("jenv: no such version".into());
    }
    crate::cmd::rehash::rehash(layout, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_walk_stops_at_the_nearest_file() {
        let tmp = std::env::temp_dir().join(format!("jenv-rs-opts-{}", std::process::id()));
        let nested = tmp.join("a/b");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(tmp.join(".java-options"), "-Xmx4g").unwrap();

        assert_eq!(
            options_file(&Layout::with_root(&tmp)).parent(),
            Some(tmp.as_path())
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
