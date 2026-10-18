use crate::layout::Layout;
use crate::link;
use crate::version;
use std::path::{Path, PathBuf};

/// `jenv global [version]`
pub fn global(layout: &Layout, requested: Option<String>, unset: bool) -> Result<(), String> {
    let file = layout.global_version_file();

    if unset {
        // `global` and `default` are legacy names for the same setting, so
        // clearing one of them would just resurrect the next in the chain.
        for file in [
            layout.root.join("version"),
            layout.root.join("global"),
            layout.root.join("default"),
        ] {
            match std::fs::remove_file(&file) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("jenv: {e}")),
            }
        }
        return Ok(());
    }

    let Some(requested) = requested else {
        return match version::read_name_file(&file) {
            Some(name) => {
                println!("{name}");
                Ok(())
            }
            None => Err("jenv: no global version configured".into()),
        };
    };

    validate_installed(layout, &requested)?;
    write_version_file(&file, &requested)
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
            if let Some(name) = version::read_name_file(file) {
                println!("{name}");
                return Ok(());
            }
        }
        return Err("jenv: no local version configured for this directory".into());
    };

    // Checked before anything is deleted: a rejected name used to take the
    // legacy file with it and leave the directory with no version at all.
    validate_installed(layout, &requested)?;

    // A `.jenv-version` in the way gets migrated rather than silently
    // shadowed, so a project that predates `.java-version` keeps working.
    if version::read_name_file(&legacy).is_some() {
        std::fs::remove_file(&legacy).map_err(|e| format!("jenv: {e}"))?;
        eprintln!("jenv: removed existing `.jenv-version' file and migrated");
        eprintln!("       local version specification to `.java-version' file");
    }

    write_version_file(&java_version, &requested)
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
                // PowerShell has neither `unset` nor `export`; both would
                // raise CommandNotFoundException on every shell start.
                "powershell" => {
                    "Remove-Item Env:JENV_VERSION -ErrorAction SilentlyContinue"
                }
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
    validate_installed(layout, &requested)?;

    println!(
        "{}",
        match shell.as_str() {
            "fish" => format!(
                "set -gx JENV_VERSION {}",
                super::quote::shell_quote(&shell, &requested)
            ),
            // PowerShell has no `export`; this line goes straight into
            // Invoke-Expression, so POSIX syntax would throw on every use.
            "powershell" => format!(
                "$env:JENV_VERSION = {}",
                super::quote::shell_quote(&shell, &requested)
            ),
            _ => format!(
                "export JENV_VERSION={}",
                super::quote::shell_quote(&shell, &requested)
            ),
        }
    );
    Ok(())
}

fn write_version_file(file: &Path, requested: &str) -> Result<(), String> {
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("jenv: {e}"))?;
    }
    std::fs::write(file, format!("{requested}\n")).map_err(|e| format!("jenv: {e}"))
}

/// A version a file may name: one that is registered, or `system`.
///
/// `link::exists`, not `home_of`: `system` is a version you select rather than
/// one that is registered, and jenv treats it as first-class —
/// `jenv global system` and `jenv shell system` have to agree. Nothing is
/// printed on success, because `jenv local 21` is quiet and the prefix it
/// validates against is not what the user asked to see.
fn validate_installed(layout: &Layout, name: &str) -> Result<(), String> {
    if link::exists(layout, name) {
        return Ok(());
    }
    Err(format!("jenv: version `{name}' not installed"))
}

/// `jenv options [--verbose]` — the extra arguments prepended to every run.
pub fn options(layout: &Layout, verbose: bool) {
    if let Some(from_env) = std::env::var("JENV_OPTIONS").ok().filter(|v| !v.is_empty()) {
        return print_options(&from_env, "JENV_OPTIONS environment variable", verbose);
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

/// The nearest `.java-options`, looking up from `$JENV_DIR` and then from the
/// working directory, the way `.java-version` is looked for.
///
/// Both legs are made absolute first, because `Path::parent` on a bare `.`
/// stops immediately: a relative `$JENV_DIR`, and a relative working
/// directory, would each otherwise walk no further than themselves.
fn options_file(layout: &Layout) -> PathBuf {
    let from_env = std::env::var_os("JENV_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(crate::cmd::version_cmd::jenv_dir);
    for start in [from_env, crate::layout::cwd()] {
        let mut current = Some(crate::layout::absolute(&start));
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
        // `registered`, not `home_of`: a registration whose JDK was moved away
        // is precisely the one a user needs to delete, and it used to be
        // neither listed nor removable.
        if link::registered(layout, name) {
            link::unregister(layout, name).map_err(|e| format!("jenv: {e}"))?;
            println!("JDK {name} removed");
            removed = true;
        } else {
            // Not an error when it is one of several names, but never a
            // silent success either: a script that removed nothing has to be
            // able to tell.
            println!("{name} is not a managed version of Java");
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
