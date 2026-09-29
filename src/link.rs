//! The registry of installed versions.
//!
//! `jenv` stores each version as a symlink at `versions/<name>`. Windows
//! cannot create symlinks without an elevated token or developer mode, so
//! there a version is stored as a regular UTF-8 file whose sole content is the
//! absolute path of the JDK home. Both forms resolve through [`Version::home`],
//! which is the only place that knows about the difference.

use crate::layout::{Layout, absolute};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Version {
    /// A JDK registered with `jenv add`.
    Installed { name: String, home: PathBuf },
    /// The JDK on `PATH`, selected by writing the literal name `system`.
    System,
}

impl Version {
    pub fn name(&self) -> &str {
        match self {
            Version::Installed { name, .. } => name,
            Version::System => "system",
        }
    }

    /// The directory to put at the front of `PATH` and to export as
    /// `JAVA_HOME`. `None` for the system version, which is whatever the
    /// ambient environment already points at.
    pub fn prefix(&self) -> Option<&Path> {
        match self {
            Version::Installed { home, .. } => Some(home),
            Version::System => None,
        }
    }
}

/// The path a version is registered at: `$JENV_ROOT/versions/<name>`.
///
/// This is what `jenv prefix`, `jenv javahome` and `jenv which` print, and
/// what `JAVA_HOME` is set to. It is deliberately *not* resolved to the JDK's
/// real location: a path pinned into an IDE configuration or a CI job should
/// keep naming the same thing after the JDK is re-registered somewhere else.
pub fn registered_path(layout: &Layout, name: &str) -> PathBuf {
    absolute(&layout.versions_dir().join(name))
}

/// Reject anything that is not a plain entry name inside `versions/`.
///
/// `Path::join("")` yields the directory itself and `join("../x")` escapes it,
/// so an unchecked name reaches `remove_dir_all` and can take the whole tree
/// with it. `jenv remove "$V"` in a script with `V` unset is the ordinary way
/// to get there.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains('\0')
}

/// Resolve `versions/<name>` to the JDK home directory it ultimately points at.
///
/// Only for internal work — finding an executable, walking a `bin/`. Anything
/// the user reads should go through [`registered_path`].
pub fn home_of(layout: &Layout, name: &str) -> Option<PathBuf> {
    if name == "system" || !valid_name(name) {
        return None;
    }
    let entry = layout.versions_dir().join(name);
    if !entry.exists() {
        return None;
    }
    if entry.is_dir() {
        // A symlink on unix, or a real directory if someone copied one in.
        return Some(absolute(&std::fs::canonicalize(&entry).unwrap_or(entry)));
    }
    // Windows text-file link.
    let contents = std::fs::read_to_string(&entry).ok()?;
    let target = contents.trim();
    if target.is_empty() {
        return None;
    }
    let target = PathBuf::from(target);
    target.is_dir().then_some(absolute(&target))
}

pub fn exists(layout: &Layout, name: &str) -> bool {
    name == "system" || home_of(layout, name).is_some()
}

/// Point `versions/<name>` at `home`, replacing whatever was there.
pub fn register(layout: &Layout, name: &str, home: &Path) -> std::io::Result<()> {
    if !valid_name(name) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("`{name}` is not a valid version name"),
        ));
    }
    std::fs::create_dir_all(layout.versions_dir())?;
    let entry = layout.versions_dir().join(name);
    remove(&entry)?;
    link_entry(home, &entry)
}

#[cfg(unix)]
fn link_entry(home: &Path, entry: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(home, entry)
}

#[cfg(windows)]
fn link_entry(home: &Path, entry: &Path) -> std::io::Result<()> {
    // A symlink keeps the two platforms byte-identical, but creating one needs
    // developer mode or an elevated token. Falling back to a file whose whole
    // content is the path needs neither, and `home_of` reads both.
    std::os::windows::fs::symlink_dir(home, entry)
        .or_else(|_| std::fs::write(entry, home.to_string_lossy().as_bytes()))
}

pub fn unregister(layout: &Layout, name: &str) -> std::io::Result<()> {
    remove(&layout.versions_dir().join(name))
}

/// Delete one registration.
///
/// A real directory here means someone put a JDK there by hand rather than
/// registering it, and jenv's own `jenv-remove` uses `rm -f` for exactly that
/// reason: unregistering must never recurse. Only the symlink and the Windows
/// text file are ours to delete.
fn remove(entry: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(entry) {
        Ok(meta) if meta.file_type().is_symlink() => std::fs::remove_file(entry),
        Ok(meta) if meta.is_dir() => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "{} is a directory, not a registration; refusing to remove it",
                entry.display()
            ),
        )),
        Ok(_) => std::fs::remove_file(entry),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Every registered version, in the order `jenv versions` prints them:
/// alphabetical, with `system` first when the ambient environment has a java.
pub fn list(layout: &Layout) -> Vec<String> {
    let mut names = Vec::new();
    if let Ok(entries) = std::fs::read_dir(layout.versions_dir()) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if home_of(layout, &name).is_some() {
                names.push(name);
            }
        }
    }
    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_file_links_resolve_on_any_platform() {
        let tmp = std::env::temp_dir().join(format!("jenv-rs-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let layout = Layout::with_root(&tmp);
        let jdk = tmp.join("fake-jdk");
        std::fs::create_dir_all(&jdk).unwrap();
        std::fs::create_dir_all(layout.versions_dir()).unwrap();

        // Write the Windows form directly; the resolver must not care.
        std::fs::write(
            layout.versions_dir().join("21"),
            jdk.to_string_lossy().as_bytes(),
        )
        .unwrap();
        assert_eq!(home_of(&layout, "21"), Some(jdk.clone()));
        assert!(exists(&layout, "21"));
        assert_eq!(list(&layout), vec!["21".to_string()]);

        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
