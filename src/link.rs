//! The registry of installed versions.
//!
//! `jenv` stores each version as a symlink at `versions/<name>`; a registration
//! that cannot be a symlink is an error. [`home_of`] also reads an older form:
//! a regular UTF-8 file whose whole content is the absolute path of the JDK
//! home. Nothing writes that form any more, but registrations made by hand or
//! by an earlier build still resolve through it.

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
}

/// The path a version is registered at: `$JENV_ROOT/versions/<name>`.
///
/// This is what `jenv prefix`, `jenv javahome` and `jenv which` print, and
/// what `JAVA_HOME` is set to. It is deliberately *not* resolved to the JDK's
/// real location: a path pinned into an IDE configuration or a CI job should
/// keep naming the same thing after the JDK is re-registered somewhere else.
///
/// The exception is a text-file registration, which has no directory of its
/// own: naming the file would put a file where `JAVA_HOME` needs a JDK home, so
/// the home it records is the only path that can be published.
pub fn registered_path(layout: &Layout, name: &str) -> PathBuf {
    let entry = absolute(&layout.versions_dir().join(name));
    let is_text_link = std::fs::symlink_metadata(&entry).is_ok_and(|meta| meta.is_file());
    if is_text_link {
        return home_of(layout, name).unwrap_or(entry);
    }
    entry
}

/// Reject anything that is not a plain entry name inside `versions/`.
///
/// `Path::join("")` yields the directory itself and `join("../x")` escapes it,
/// so an unchecked name reaches `remove_dir_all` and can take the whole tree
/// with it. `jenv remove "$V"` in a script with `V` unset is the ordinary way
/// to get there.
///
/// The name also ends up inside shell code: `jenv shell` and the `export-hook`
/// prompt hook print a line the parent shell evaluates, and the value is a
/// version name. `cmd::quote` escapes it for whichever shell will read it, but
/// this is the cheaper of the two defences — it stops a hostile name from ever
/// being registered, and the quoting covers the names that already are.
/// Refusing the quoting characters outright is right for a version name: no
/// JDK is called `21"; rm -rf ~`.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains('\0')
        && !name.contains('"')
        && !name.contains('\'')
        && !name.contains('`')
        && !name.contains('$')
        && !name.contains(';')
        && !name.contains('&')
        && !name.contains('|')
        && !name.contains('<')
        && !name.contains('>')
        && !name.contains('\n')
        && !name.contains('\r')
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
    // A text-file registration from the Windows form. Nothing writes this any
    // more, but one left behind by an older build still resolves.
    let contents = std::fs::read_to_string(&entry).ok()?;
    let target = contents.trim();
    if target.is_empty() {
        return None;
    }
    let target = PathBuf::from(target);
    target.is_dir().then_some(absolute(&target))
}

/// Is there a registration under `name`, whether or not its target still
/// exists? A symlink whose JDK was moved or deleted still has to be removable.
pub fn registered(layout: &Layout, name: &str) -> bool {
    valid_name(name) && std::fs::symlink_metadata(layout.versions_dir().join(name)).is_ok()
}

pub fn exists(layout: &Layout, name: &str) -> bool {
    name == "system" || home_of(layout, name).is_some()
}

/// Point `versions/<name>` at `home`, replacing whatever was there.
pub fn register(layout: &Layout, name: &str, home: &Path) -> std::io::Result<()> {
    // `system` names the ambient java rather than a registration, so an entry
    // under that name could never be listed, selected or removed: it would be
    // born broken.
    if !valid_name(name) || name == "system" {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("`{name}` is not a valid version name"),
        ));
    }
    std::fs::create_dir_all(layout.versions_dir())?;
    let entry = layout.versions_dir().join(name);
    remove(&entry)?;
    symlink::symlink_dir(home, &entry)
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
///
/// A registration whose JDK has gone is listed rather than hidden: it is still
/// an entry under `versions/`, `jenv remove` can delete it, and a listing that
/// omits it leaves a user with no way to find out what is wrong.
pub fn list(layout: &Layout) -> Vec<String> {
    let mut names = Vec::new();
    if let Ok(entries) = std::fs::read_dir(layout.versions_dir()) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if registered(layout, &name) {
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
        // And every command that publishes a prefix has to publish the JDK
        // home, not the text file that names it.
        assert_eq!(registered_path(&layout, "21"), jdk);

        std::fs::remove_dir_all(&tmp).unwrap();
    }

    /// A version name is pasted into shell code by `jenv shell` and the prompt
    /// hook, so the characters that could close a string are refused here —
    /// the cheaper of the two defences, since it stops the name existing at
    /// all. Everything a real JDK is called must still be accepted.
    #[test]
    fn valid_name_refuses_shell_metacharacters() {
        for hostile in [
            r#"x"; touch PWNED; #"#,
            "x'; touch PWNED; #",
            "x`id`",
            "x$(id)",
            "x;id",
            "x&&id",
            "x|id",
            "x>PWNED",
            "x<PWNED",
            "x\ny",
            "x\ry",
        ] {
            assert!(!valid_name(hostile), "should be refused: {hostile:?}");
        }

        // And the names jenv actually registers still pass.
        for ok in ["21", "21.0.2.13", "temurin64-21.0.2", "1.8.0.292", "my-jdk"] {
            assert!(valid_name(ok), "should be allowed: {ok:?}");
        }
    }
}
