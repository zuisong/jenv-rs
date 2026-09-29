use crate::layout::{self, Layout, SHIM_VERSION};
use crate::link;
use crate::proc;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const LOCK_FILE: &str = ".jenv-shim-lock";

/// `jenv rehash [--force]`
///
/// Regenerates one shim per executable across every registered version. A
/// shim is not a script: it is this same binary, hard-linked under each tool's
/// own file name, so it reads that name to decide what to dispatch and a
/// `java` invocation costs one process instead of a chain of a dozen shell
/// interpreters.
pub fn rehash(layout: &Layout, force: bool) -> Result<(), String> {
    let shims_dir = layout.shims_dir();
    std::fs::create_dir_all(&shims_dir).map_err(|e| format!("jenv: {e}"))?;

    let _lock = RehashLock::acquire(&shims_dir, force)?;

    let shim_binary = shim_binary()?;

    let mut wanted: BTreeSet<String> = BTreeSet::new();
    for name in link::list(layout) {
        let Some(home) = link::home_of(layout, &name) else {
            continue;
        };
        wanted.extend(executables_in(&home.join("bin")));
    }

    // A shim binary from another generation cannot be trusted, and neither can
    // shims a crashed run left half-written. Wiping is cheap next to being
    // wrong, so the version marker alone decides.
    let outdated = layout::read_shim_version(&shims_dir) != Some(SHIM_VERSION);
    if force || outdated {
        remove_all_shims(&shims_dir);
    }

    let mut created = 0;
    for name in &wanted {
        let path = shim_path(&shims_dir, name);
        if path.exists() {
            continue;
        }
        write_shim(&shim_binary, &path).map_err(|e| format!("jenv: {}: {e}", path.display()))?;
        created += 1;
    }

    let kept: BTreeSet<String> = wanted
        .iter()
        .map(|name| crate::proc::file_name(name))
        .collect();
    for stale in existing_shim_files(&shims_dir) {
        if !kept.contains(&stale) {
            let _ = std::fs::remove_file(shims_dir.join(&stale));
        }
    }

    layout::write_shim_version(&shims_dir, SHIM_VERSION).map_err(|e| format!("jenv: {e}"))?;

    if std::env::var_os("JENV_DEBUG").is_some() {
        eprintln!(
            "jenv: rehash created {created} shim(s), {} wanted",
            wanted.len()
        );
    }
    Ok(())
}

fn shim_path(shims_dir: &Path, name: &str) -> PathBuf {
    shims_dir.join(crate::proc::file_name(name))
}

/// Every executable in a JDK's `bin/`, keyed by the name a shim will carry.
fn executables_in(bin: &Path) -> BTreeSet<String> {
    let Ok(entries) = std::fs::read_dir(bin) else {
        return BTreeSet::new();
    };

    entries
        .flatten()
        .filter(|entry| proc::is_executable(&entry.path()))
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            let stem = if cfg!(windows) {
                name.strip_suffix(".exe").map(str::to_string)
            } else {
                Some(name)
            }?;
            (!stem.is_empty() && !stem.starts_with('.')).then_some(stem)
        })
        .collect()
}

fn existing_shim_files(shims_dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(shims_dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| !name.starts_with('.'))
        .collect()
}

fn remove_all_shims(shims_dir: &Path) {
    for name in existing_shim_files(shims_dir) {
        let _ = std::fs::remove_file(shims_dir.join(name));
    }
}

/// Hard-link when the filesystem allows it, copy otherwise.
///
/// A link keeps a hundred shims to one inode; a copy still works, so a
/// filesystem without hard links (some FUSE and network mounts) degrades in
/// disk usage rather than in behaviour.
fn write_shim(source: &Path, target: &Path) -> std::io::Result<()> {
    match std::fs::hard_link(source, target) {
        Ok(()) => return Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Ok(()),
        Err(_) => {}
    }

    // Stage under a temporary name and rename, so a concurrent `java` never
    // observes a half-written executable.
    let staging = target.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::copy(source, &staging)?;
    if let Err(e) = std::fs::rename(&staging, target) {
        let _ = std::fs::remove_file(&staging);
        return Err(e);
    }
    Ok(())
}

/// The binary that gets linked into every shim name: this one.
///
/// The shim and the CLI are the same file, so there is nothing to locate and
/// no way for a shim to end up being a different build from the `jenv` that
/// manages it.
fn shim_binary() -> Result<PathBuf, String> {
    std::env::current_exe()
        .map_err(|e| format!("jenv: cannot find my own executable to build shims from: {e}"))
}

/// Mutual exclusion between concurrent rehashes.
///
/// jenv used `set -o noclobber` against its prototype shim, which is a lock
/// that only works because the prototype happens to live in the shims
/// directory. An explicit file is clearer, and `--force` still overrides it.
struct RehashLock {
    path: PathBuf,
}

impl RehashLock {
    fn acquire(shims_dir: &Path, force: bool) -> Result<Self, String> {
        let path = shims_dir.join(LOCK_FILE);

        if !force {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => {
                    return Ok(Self { path });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    return Err(format!(
                        "jenv: cannot rehash: {} exists, try 'jenv rehash --force' to override",
                        path.display()
                    ));
                }
                Err(e) => return Err(format!("jenv: {e}")),
            }
        }

        // Forced: the previous run is presumed dead, so its lock is ours to
        // take. Taking it means removing it — `--force` is advertised as the
        // way out of a poisoned lock, so it has to actually leave no lock
        // behind, or every later rehash keeps failing.
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("jenv: cannot rehash: {e}")),
        }
        // Claim the lock like the ordinary path does, so a concurrent rehash
        // still sees it while this one runs.
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(_) => Ok(Self { path }),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(format!(
                "jenv: cannot rehash: {} exists, try 'jenv rehash --force' to override",
                path.display()
            )),
            Err(e) => Err(format!("jenv: {e}")),
        }
    }
}

impl Drop for RehashLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
