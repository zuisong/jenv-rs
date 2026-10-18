use crate::env::Env;
use crate::layout::Layout;
use crate::link::Version;
use crate::process;
use crate::version;
use std::collections::HashMap;
use std::ffi::OsString;

/// `jenv exec <command> [args...]`
///
/// Prepares the environment the selected version implies and replaces this
/// process with the real tool, so the JVM inherits the terminal and the
/// process group rather than sitting behind an interpreter.
pub fn exec(layout: &Layout, env: &Env, argv: &[OsString]) -> Result<(), String> {
    let Some((command, args)) = argv.split_first() else {
        return Err("jenv: no command given to exec".into());
    };
    // Only the command is looked up and reported; the arguments after it are
    // passed through untouched, so a filename that is not UTF-8 reaches the
    // tool exactly as the shell spelled it.
    let command = command.to_string_lossy().into_owned();

    let selection = version::select(layout, env)?;
    let Some(path) = crate::cmd::which::resolve(layout, env, &command) else {
        return Err(format!("jenv: {command}: command not found"));
    };

    let mut vars: HashMap<&str, String> = HashMap::new();
    vars.insert("JENV_VERSION", selection.version.name().to_string());
    vars.insert("JENV_COMMAND", command.clone());
    vars.insert("JENV_DIR", env.jenv_dir().display().to_string());

    if let Version::Installed { name, .. } = &selection.version {
        // The registration path, not the JDK's real home: it is what
        // `jenv javahome`, `jenv prefix` and the export hook all publish, and
        // a JAVA_HOME that survives the JDK being re-registered is the point.
        let prefix = crate::link::registered_path(layout, name);
        vars.insert("JAVA_HOME", prefix.display().to_string());

        // jenv prepends the version's `bin` rather than replacing PATH, so the
        // rest of the toolchain (gradle, node) stays reachable. `var_os` and
        // `OsString`: a PATH entry that is not UTF-8 is still a valid entry,
        // and reading it as a `String` silently dropped every other one.
        let joined =
            std::env::join_paths(std::iter::once(prefix.join("bin")).chain(env.path_dirs()));
        if let Ok(joined) = joined {
            vars.insert("PATH", joined.to_string_lossy().into_owned());
        }
    }

    // `JENV_OPTIONS` in the environment wins over the nearest `.java-options`
    // file, exactly as `jenv options` resolves them.
    if let Some(options) = crate::cmd::config::options_value(layout, env) {
        vars.insert("JENV_OPTIONS", options);
    }

    let options = vars.get("JENV_OPTIONS").cloned().unwrap_or_default();

    let mut process = process::command_with_argv0(&path, &command);
    for (name, value) in &vars {
        process.env(name, value);
    }
    for word in split_words(&options) {
        process.arg(word);
    }
    process.args(args);

    let err = process::exec_replace(process);
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
