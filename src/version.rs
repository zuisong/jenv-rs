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

use crate::env::Env;
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
pub fn active_version_file(layout: &Layout, env: &Env) -> PathBuf {
    let (start, cwd) = start_dirs(env);
    local_version_file(&start)
        .or_else(|| local_version_file(&cwd))
        .unwrap_or_else(|| layout.global_version_file())
}

/// `$JENV_DIR` made absolute — a relative value would otherwise stop the walk
/// at itself — and the working directory.
fn start_dirs(env: &Env) -> (PathBuf, PathBuf) {
    let cwd = env.cwd();
    let start = env.absolute(&env.jenv_dir());
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
pub fn select(layout: &Layout, env: &Env) -> Result<Selection, String> {
    if let Some(from_env) = env.text("JENV_VERSION") {
        let name = from_env.trim().to_string();
        if name.is_empty() || name == "system" {
            return Ok(Selection {
                version: Version::System,
                origin: "JENV_VERSION environment variable".into(),
            });
        }
        return build(layout, &name, "JENV_VERSION environment variable");
    }

    let file = active_version_file(layout, env);
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

    /// The two-leg walk, as a decision rather than a subprocess. This is the
    /// case that was broken in the shipped code and could not be reached by a
    /// unit test before the environment became a parameter: `$JENV_DIR` and the
    /// working directory are different, and the working directory still wins
    /// when `$JENV_DIR` finds nothing.
    #[test]
    fn the_walk_prefers_jenv_dir_then_the_working_directory() {
        let tmp = std::env::temp_dir().join(format!("jenv-rs-twolegs-{}", std::process::id()));
        let project = tmp.join("project");
        let elsewhere = tmp.join("elsewhere");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(project.join(".java-version"), "21").unwrap();

        let layout = Layout::with_root(&tmp);

        // Leg one: `$JENV_DIR` is inside the project.
        let env = Env::bare(&elsewhere).with("JENV_DIR", &project);
        assert_eq!(
            active_version_file(&layout, &env),
            project.join(".java-version")
        );

        // Leg two: `$JENV_DIR` points nowhere interesting, so the working
        // directory's own file is next — it used to lose to the global file.
        let env = Env::bare(&project).with("JENV_DIR", &elsewhere);
        assert_eq!(
            active_version_file(&layout, &env),
            project.join(".java-version")
        );

        // And with neither leg finding one, the global file is the answer.
        let env = Env::bare(&elsewhere).with("JENV_DIR", &elsewhere);
        assert_eq!(
            active_version_file(&layout, &env),
            layout.global_version_file()
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// A relative `$JENV_DIR` is relative to the working directory and must be
    /// made absolute before the walk, or `Path::parent` stops at itself.
    #[test]
    fn a_relative_jenv_dir_is_resolved_before_the_walk() {
        let tmp = std::env::temp_dir().join(format!("jenv-rs-rel-{}", std::process::id()));
        let deep = tmp.join("a/b");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(tmp.join(".java-version"), "21").unwrap();

        let env = Env::bare(&deep).with("JENV_DIR", "..");
        assert_eq!(
            active_version_file(&Layout::with_root(&tmp), &env),
            tmp.join(".java-version")
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// `$JENV_VERSION` outranks every file, and an empty one is how a shell
    /// says "cleared" rather than "set to the empty version name".
    #[test]
    fn the_environment_variable_outranks_the_files() {
        let tmp = std::env::temp_dir().join(format!("jenv-rs-envsel-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let layout = Layout::with_root(&tmp);

        // The variable wins over the global file regardless of the file.
        std::fs::write(tmp.join("version"), "21\n").unwrap();
        let empty = Env::bare(&tmp).with("JENV_VERSION", "");
        assert_eq!(
            select(&layout, &empty).unwrap().version.name(),
            "system",
            "an empty JENV_VERSION must not be read as a version name"
        );

        // And a name it holds is attempted before any file is consulted.
        let pinned = Env::bare(&tmp).with("JENV_VERSION", "99");
        assert!(select(&layout, &pinned).unwrap_err().contains("not installed"));

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
