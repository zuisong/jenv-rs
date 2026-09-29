use crate::layout::Layout;
use crate::link;
use crate::probe;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

/// `jenv add [<name>] <path-to-jdk-home>`
///
/// This is the one command with no equivalent in proto or mise: registering a
/// JDK that is already on the machine, under every name it plausibly answers
/// to (`temurin64-21.0.2.13`, `21.0.2.13`, `21.0`, `21`).
///
/// One JDK per call, as in jenv. A batch would have to decide whether a
/// leading argument is a name or a path, and the only signal available is
/// whether it happens to exist on disk, which makes a mistyped path silently
/// register something else. Keeping jenv's rule keeps the command honest.
pub fn add(layout: &Layout, target: &[String], force: bool) -> Result<(), String> {
    let (path, alias) = split_target(target)?;

    let jdk = probe::probe(&path)?;

    // A caller-supplied name registers exactly one alias; that is how you add
    // a JDK whose banner would otherwise be misidentified.
    let names = match alias {
        Some(name) => vec![name],
        None => jdk.aliases(),
    };

    let mut added = false;
    for name in &names {
        if link::exists(layout, name) && !override_existing(layout, name, force)? {
            println!(" {name} already present, skip installation");
            continue;
        }

        link::register(layout, name, &jdk.home).map_err(|e| format!("jenv: {name}: {e}"))?;
        println!("{name} added");
        added = true;
    }

    if added {
        crate::cmd::rehash::rehash(layout, false)?;
    }
    Ok(())
}

/// jenv documents both `jenv add <path>` and `jenv add <name> <path>`, so the
/// two arguments are told apart by which one is a directory rather than by
/// their position.
fn split_target(target: &[String]) -> Result<(PathBuf, Option<String>), String> {
    const USAGE: &str = "jenv: usage: jenv add [<name>] <path-to-jdk-home>";

    match target {
        [only] => Ok((PathBuf::from(only), None)),
        [first, second] if Path::new(first).is_dir() => {
            Ok((PathBuf::from(first), Some(second.clone())))
        }
        [first, second] if Path::new(second).is_dir() => {
            Ok((PathBuf::from(second), Some(first.clone())))
        }
        [first, second, ..] => Err(format!("{USAGE} (got {first:?} and {second:?})")),
        [] => Err(USAGE.to_string()),
    }
}

/// jenv asks before clobbering an existing registration. A non-interactive
/// shell has no way to answer, so it keeps what is already there.
fn override_existing(layout: &Layout, name: &str, force: bool) -> Result<bool, String> {
    if force {
        return Ok(true);
    }
    if !std::io::stdin().is_terminal() {
        return Ok(false);
    }

    eprint!("There is already a {name} JDK managed by jenv. Override? (y/N) ");
    let mut answer = String::new();
    if std::io::stdin().read_line(&mut answer).is_err() {
        return Ok(false);
    }
    if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        return Ok(false);
    }
    link::unregister(layout, name).map_err(|e| format!("jenv: {e}"))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::split_target;
    use std::path::PathBuf;

    /// The two-argument form can only be told apart by the filesystem, so
    /// the tests have to make one.
    fn dir(name: &str) -> String {
        let path = std::env::temp_dir().join(format!("jenv-rs-split-{name}"));
        std::fs::create_dir_all(&path).unwrap();
        path.display().to_string()
    }

    #[test]
    fn a_lone_argument_is_the_path() {
        let a = dir("lone");
        assert_eq!(
            split_target(std::slice::from_ref(&a)).unwrap(),
            (PathBuf::from(&a), None)
        );
    }

    #[test]
    fn either_order_of_name_and_path_is_accepted() {
        let a = dir("order");
        let named = split_target(&[a.clone(), "work-jdk".to_string()]).unwrap();
        let reversed = split_target(&["work-jdk".to_string(), a.clone()]).unwrap();
        assert_eq!(named, reversed);
        assert_eq!(named, (PathBuf::from(a), Some("work-jdk".to_string())));
    }

    /// jenv reads the directory as the path in both orders, so a pair of
    /// directories is a name and a path rather than two paths. This is the
    /// ambiguity that rules out batching, and it is jenv's own rule.
    #[test]
    fn a_pair_of_directories_is_read_as_jenv_reads_it() {
        let (a, b) = (dir("pair-a"), dir("pair-b"));
        assert_eq!(
            split_target(&[a.clone(), b.clone()]).unwrap(),
            (PathBuf::from(a), Some(b))
        );
    }

    #[test]
    fn no_arguments_is_a_usage_error() {
        assert!(split_target(&[]).unwrap_err().contains("usage: jenv add"));
    }

    #[test]
    fn more_than_two_arguments_is_refused() {
        let a = dir("extra");
        let err = split_target(&[a, "b".to_string(), "c".to_string()]).unwrap_err();
        assert!(err.contains("usage: jenv add"), "{err}");
    }
}
