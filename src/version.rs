//! Which JDK is selected, and why.
//!
//! The precedence is fixed by `jenv version-file`:
//!
//! 1. `JENV_VERSION` in the environment
//! 2. `.java-version` (or legacy `.jenv-version`) found by walking up from
//!    `$JENV_DIR`, then from the working directory if that is different
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

/// The file that pins the version, following `jenv version-file` exactly,
/// including the `global`/`default` legacy names. Returns a path that may not
/// exist, which callers must tolerate.
pub fn version_file(layout: &Layout, start_dir: &Path) -> PathBuf {
    for dir in walk_up(start_dir) {
        let java_version = dir.join(".java-version");
        if java_version.exists() {
            return java_version;
        }
        let jenv_version = dir.join(".jenv-version");
        if jenv_version.exists() {
            return jenv_version;
        }
    }
    layout.global_version_file()
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

    let start = std::env::var_os("JENV_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(crate::layout::cwd);
    let cwd = crate::layout::cwd();

    let mut file = version_file(layout, &start);
    if start != cwd {
        // `$JENV_DIR` wins, but a `.java-version` above the *working*
        // directory is the next place worth looking before going global.
        let from_cwd = version_file(layout, &cwd);
        if !from_cwd.exists() {
            file = from_cwd;
        }
    }

    let origin = file.display().to_string();
    let name = read_version_file(&file).unwrap_or_else(|| "system".into());
    if name == "system" {
        return Ok(Selection {
            version: Version::System,
            origin,
        });
    }
    build(layout, &name, &origin)
}

fn build(layout: &Layout, name: &str, origin: &str) -> Result<Selection, String> {
    if link::exists(layout, name) {
        let home = link::home_of(layout, name).expect("exists implies a resolvable home");
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
        let home = link::home_of(layout, stripped).expect("checked above");
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

        let layout = Layout::with_root(&tmp);
        assert_eq!(version_file(&layout, &dir), dir.join(".java-version"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn version_file_walks_up_to_a_parent() {
        let tmp = std::env::temp_dir().join(format!("jenv-rs-walk-{}", std::process::id()));
        let nested = tmp.join("a/b/c");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(tmp.join(".java-version"), "21").unwrap();

        let layout = Layout::with_root(&tmp);
        assert_eq!(version_file(&layout, &nested), tmp.join(".java-version"));

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
