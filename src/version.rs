//! Which JDK is selected, and why.
//!
//! The precedence is fixed by `jenv version-file`:
//!
//! 1. `JENV_VERSION` in the environment
//! 2. `.java-version` (or legacy `.jenv-version`) found by walking up from
//!    `$JENV_DIR`, then from the working directory if that finds nothing
//! 3. the global version file
//!
//! An unset or empty selection, and the literal name `system`, both mean "the
//! java already on `PATH`".

use crate::layout::Layout;
use crate::link::{self, Version};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Selection {
    pub version: Version,
    /// Human-readable provenance, printed by `jenv version` and `version-origin`.
    pub origin: String,
}

/// The file that actually selects the version: `$JENV_DIR` first, then the
/// working directory, then the global file.
///
/// Both legs live here so that `jenv version-file` and `jenv version` cannot
/// disagree about which file is in charge.
pub fn active_version_file(layout: &Layout) -> PathBuf {
    let (start, cwd) = start_dirs();
    local_version_file(&start)
        .or_else(|| local_version_file(&cwd))
        .unwrap_or_else(|| layout.global_version_file())
}

/// `$JENV_DIR` made absolute — a relative value would otherwise stop the walk
/// at itself — and the working directory.
fn start_dirs() -> (PathBuf, PathBuf) {
    let cwd = crate::layout::cwd();
    let start = match std::env::var_os("JENV_DIR") {
        Some(dir) if !dir.is_empty() => crate::layout::absolute(Path::new(&dir)),
        _ => cwd.clone(),
    };
    (start, cwd)
}

/// The nearest `.java-version`, or legacy `.jenv-version`, at or above `start`.
/// Both names are checked in each directory as the walk climbs, so a nearer
/// legacy file still beats a further `.java-version`.
fn local_version_file(start: &Path) -> Option<PathBuf> {
    for dir in walk_up(start) {
        for name in [".java-version", ".jenv-version"] {
            let candidate = dir.join(name);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    None
}

fn walk_up(start: &Path) -> impl Iterator<Item = PathBuf> + '_ {
    let mut current = Some(start.to_path_buf());
    std::iter::from_fn(move || {
        let dir = current.take()?;
        if !dir.as_os_str().is_empty() {
            current = Some(dir.parent().map(Path::to_path_buf).unwrap_or_default());
            Some(dir)
        } else {
            None
        }
    })
}

pub fn read_version_file(path: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(path).ok()?;
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// The version named by a *version name* file.
///
/// A version name is one line — a second line is either a trailing newline or
/// a second command — so only the first is read, and it is rejected unless
/// `jenv add` would have accepted it as a name. That keeps a `.java-version`
/// from `git clone` out of the shell code `jenv shell` and the prompt hook
/// print.
///
/// Distinct from [`read_version_file`], which reads values that are not names
/// at all: `.java-options` holds a whitespace-separated argument list, and
/// applying the name rule to it would silently drop every option.
pub fn read_name_file(path: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(path).ok()?;
    let name = raw.lines().next().unwrap_or_default().trim();
    (!name.is_empty() && crate::link::valid_name(name)).then(|| name.to_string())
}

/// Resolve the active version. Returns `Err` with a jenv-compatible message
/// when the configured version is not registered.
pub fn select(layout: &Layout) -> Result<Selection, String> {
    if let Ok(from_env) = std::env::var("JENV_VERSION") {
        let name = from_env.trim().to_string();
        if name.is_empty() || name == "system" {
            return Ok(Selection {
                version: Version::System,
                origin: "JENV_VERSION environment variable".into(),
            });
        }
        return build(layout, &name, "JENV_VERSION environment variable");
    }

    let file = active_version_file(layout);
    let origin = file.display().to_string();
    let name = read_name_file(&file).unwrap_or_else(|| "system".into());
    if name == "system" {
        return Ok(Selection {
            version: Version::System,
            origin,
        });
    }
    build(layout, &name, &origin)
}

fn build(layout: &Layout, name: &str, origin: &str) -> Result<Selection, String> {
    // `exists` and `home_of` each stat the entry, so a concurrent
    // `jenv remove` can land between them. That is an ordinary "not installed"
    // rather than a reason to abort, so the lookup is repeated and its failure
    // reported as the error the caller already understands.
    if link::exists(layout, name) {
        let Some(home) = link::home_of(layout, name) else {
            return Err(format!("jenv: version `{name}' is not installed"));
        };
        return Ok(Selection {
            version: Version::Installed {
                name: name.to_string(),
                home,
            },
            origin: origin.to_string(),
        });
    }

    // `java-21` is a common typo/legacy spelling of `21`; jenv forgives it.
    if let Some(stripped) = name.strip_prefix("java-")
        && link::exists(layout, stripped)
    {
        let Some(home) = link::home_of(layout, stripped) else {
            return Err(format!("jenv: version `{name}' is not installed"));
        };
        eprintln!("warning: ignoring extraneous `java-' prefix in version `{name}'");
        eprintln!("         (set by {origin})");
        return Ok(Selection {
            version: Version::Installed {
                name: stripped.to_string(),
                home,
            },
            origin: origin.to_string(),
        });
    }

    Err(format!("jenv: version `{name}' is not installed"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_version_beats_legacy_jenv_version_in_the_same_dir() {
        let tmp = std::env::temp_dir().join(format!("jenv-rs-vf-{}", std::process::id()));
        let dir = tmp.join("project");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".java-version"), "21\n").unwrap();
        std::fs::write(dir.join(".jenv-version"), "17").unwrap();

        assert_eq!(
            local_version_file(&dir),
            Some(dir.join(".java-version"))
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn version_file_walks_up_to_a_parent() {
        let tmp = std::env::temp_dir().join(format!("jenv-rs-walk-{}", std::process::id()));
        let nested = tmp.join("a/b/c");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(tmp.join(".java-version"), "21").unwrap();

        assert_eq!(local_version_file(&nested), Some(tmp.join(".java-version")));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// A version file names a version, so only the first line is one. Reading
    /// the rest would put a second line into the shell code the prompt hook
    /// prints, and a name the registrar would have refused is not honoured
    /// from a file either.
    #[test]
    fn a_version_file_yields_one_name_or_nothing() {
        let tmp = std::env::temp_dir().join(format!("jenv-rs-rvf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let read = |body: &str| {
            let path = tmp.join(".java-version");
            std::fs::write(&path, body).unwrap();
            read_name_file(&path)
        };

        assert_eq!(read("21\n"), Some("21".into()));
        // A trailing newline is ordinary; a second line is not part of a name.
        assert_eq!(read("21\ntouch PWNED\n"), Some("21".into()));
        assert_eq!(read("  21  \n"), Some("21".into()));
        // Empty, whitespace, and anything a name cannot be read as nothing.
        assert_eq!(read("\n"), None);
        assert_eq!(read("   "), None);
        assert_eq!(read(r#"x"; touch PWNED; #"#), None);
        // A newline splits the file; the first line is the name, and it is
        // then refused on its own merits. Only a name containing a newline is
        // dangerous, and a name can no longer contain one.
        assert_eq!(read("21\ntouch PWNED\n"), Some("21".into()));
        assert_eq!(read("21;id\ntouch PWNED\n"), None);
        assert!(!crate::link::valid_name("21\ntouch PWNED"));

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
