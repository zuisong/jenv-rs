use crate::layout::Layout;
use crate::link::Version;
use crate::proc;
use crate::version;
use std::collections::HashMap;

/// `jenv exec <command> [args...]`
///
/// Prepares the environment the selected version implies and replaces this
/// process with the real tool, so the JVM inherits the terminal and the
/// process group rather than sitting behind an interpreter.
pub fn exec(layout: &Layout, argv: &[String]) -> Result<(), String> {
    let Some((command, args)) = argv.split_first() else {
        return Err("jenv: no command given to exec".into());
    };

    let selection = version::select(layout)?;
    let path = crate::cmd::which::resolve(layout, command)?;

    if !proc::is_executable(&path) {
        // Returned, not printed: the caller prints it, and printing here too
        // would say it twice.
        return Err(format!("jenv: {command}: command not found"));
    }

    let mut env: HashMap<&str, String> = HashMap::new();
    env.insert("JENV_VERSION", selection.version.name().to_string());
    env.insert("JENV_COMMAND", command.to_string());
    env.insert(
        "JENV_DIR",
        crate::cmd::version_cmd::jenv_dir().display().to_string(),
    );

    if let Version::Installed { home, .. } = &selection.version {
        env.insert("JAVA_HOME", home.display().to_string());

        // jenv prepends the version's `bin` rather than replacing PATH, so the
        // rest of the toolchain (gradle, node) stays reachable.
        let current = std::env::var("PATH").unwrap_or_default();
        let joined = std::env::join_paths(
            std::iter::once(home.join("bin")).chain(std::env::split_paths(&current)),
        )
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or(current);
        env.insert("PATH", joined);
    }

    // `JENV_OPTIONS` in the environment wins over the nearest `.java-options`
    // file, exactly as `jenv options` resolves them.
    if let Some(options) = crate::cmd::config::options_value(layout) {
        env.insert("JENV_OPTIONS", options);
    }

    let options = env.get("JENV_OPTIONS").cloned().unwrap_or_default();

    let mut process = proc::command_with_argv0(&path, command);
    for (name, value) in &env {
        process.env(name, value);
    }
    for word in split_words(&options) {
        process.arg(word);
    }
    process.args(args);

    let err = proc::exec_replace(process);
    Err(format!("jenv: {command}: {err}"))
}

/// Split a string the way a shell would, so `JENV_OPTIONS="-Xmx2g -Dfoo=a b"`
/// becomes two arguments and not four.
pub fn split_words(input: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;

    for ch in input.chars() {
        match quote {
            Some(q) if ch == q => quote = None,
            Some(_) => current.push(ch),
            None if ch == '\'' || ch == '"' => {
                quote = Some(ch);
                started = true;
            }
            None if ch.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            None => {
                current.push(ch);
                started = true;
            }
        }
    }
    if started {
        words.push(current);
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_split_like_a_shell_would() {
        assert_eq!(split_words("-Xmx2g -Dfoo=bar"), vec!["-Xmx2g", "-Dfoo=bar"]);
        assert_eq!(split_words("-Xmx2g  -Dx=1 "), vec!["-Xmx2g", "-Dx=1"]);
        assert_eq!(split_words(""), Vec::<String>::new());
        assert_eq!(
            split_words("-Da='a b' -Db=\"c d\""),
            vec!["-Da=a b", "-Db=c d"]
        );
    }
}
