//! Resolution of `$JENV_ROOT` and the fixed directory layout underneath it.

use std::path::{Path, PathBuf};

pub const SHIM_VERSION: u8 = 1;
/// Name of the file inside `shims/` that records which generation of the shim
/// binary the existing shims were generated from. Bumping `SHIM_VERSION` and
/// re-running `jenv rehash` rewrites every shim; the cost is one byte read
/// instead of diffing N binaries.
const SHIM_VERSION_FILE: &str = ".jenv-shim-version";

pub struct Layout {
    pub root: PathBuf,
}

impl Layout {
    pub fn from_env(env: &crate::env::Env) -> Self {
        let root = match env.var("JENV_ROOT") {
            Some(v) if !v.is_empty() => PathBuf::from(v),
            _ => env.home_dir().join(".jenv"),
        };
        // Absolute and lexically normal at construction, so every path derived
        // from `root` is too and no consumer has to spell out "relative to
        // what". A relative `$JENV_ROOT` is resolved against the working
        // directory once, here, rather than at each of a dozen call sites.
        Self {
            root: env.absolute(&root),
        }
    }

    /// A layout rooted somewhere other than `$JENV_ROOT`, for tests that need
    /// a directory of their own.
    #[cfg(test)]
    pub fn with_root(root: impl Into<PathBuf>) -> Self {
        Self {
            root: normalize(&root.into()),
        }
    }

    pub fn versions_dir(&self) -> PathBuf {
        self.root.join("versions")
    }

    pub fn shims_dir(&self) -> PathBuf {
        self.root.join("shims")
    }

    /// The file that pins a version for every directory without a local one.
    /// jenv also accepts `global` and `default` as legacy names for it.
    pub fn global_version_file(&self) -> PathBuf {
        let version = self.root.join("version");
        if version.exists() {
            return version;
        }
        let global = self.root.join("global");
        if global.exists() {
            return global;
        }
        let default = self.root.join("default");
        if default.exists() {
            return default;
        }
        version
    }

    /// Create the directories `jenv init` promises to the shell.
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(self.shims_dir())?;
        std::fs::create_dir_all(self.versions_dir())?;
        Ok(())
    }
}

/// Lexically remove `.` and `..` components. `std::fs::canonicalize` would do
/// this too but it requires the path to exist, and it resolves symlinks, which
/// we deliberately avoid for version directories.
pub fn normalize(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    let mut depth = 0usize;

    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                // `..` above the root is the root, exactly as the shell sees
                // it; going past a relative path's start is a real `..`.
                if depth > 0 && out.pop() {
                    depth -= 1;
                } else if !p.is_absolute() {
                    out.push("..");
                }
            }
            other => {
                out.push(other.as_os_str());
                depth += 1;
            }
        }
    }
    out
}

/// Read the shim generation marker, if one was written.
pub fn read_shim_version(shims_dir: &Path) -> Option<u8> {
    std::fs::read_to_string(shims_dir.join(SHIM_VERSION_FILE))
        .ok()?
        .trim()
        .parse()
        .ok()
}

pub fn write_shim_version(shims_dir: &Path, version: u8) -> std::io::Result<()> {
    std::fs::write(shims_dir.join(SHIM_VERSION_FILE), version.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_resolves_dot_segments() {
        assert_eq!(normalize(Path::new("/a/./b/../c")), PathBuf::from("/a/c"));
        assert_eq!(normalize(Path::new("/../a")), PathBuf::from("/a"));
    }
}
