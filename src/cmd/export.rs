use crate::layout::Layout;
use crate::link::Version;
use crate::version;

/// `jenv javahome`
///
/// The path the active version is registered under, not the resolved JDK home:
/// tools that write `JAVA_HOME` into a config file should see the stable
/// `versions/<name>` path, the same way jenv's own `javahome` does.
pub fn javahome(layout: &Layout) -> Result<(), String> {
    match version::select(layout)?.version {
        // The registration path, like `jenv prefix` and jenv's own
        // `jenv javahome`: the export plugin puts this straight into
        // JAVA_HOME, and a value that survives the JDK being re-registered
        // is the whole point of having a registration at all.
        Version::Installed { name, .. } => {
            println!("{}", crate::link::registered_path(layout, &name).display());
            Ok(())
        }
        Version::System => Err("jenv: using system JDK, no JAVA_HOME set".into()),
    }
}

/// `jenv export-hook`
///
/// The body of the prompt hook that `jenv init -` installs, in the syntax of
/// the shell being set up. This is jenv's `export` plugin, inlined: the thing
/// it does is keep `JAVA_HOME` and `JDK_HOME` pointing at whichever version
/// the current directory selects, so an IDE or a build tool that reads
/// `JAVA_HOME` instead of going through a shim still gets the right JDK.
///
/// Printing nothing when nothing changed keeps the per-prompt cost to the
/// subprocess this binary already costs, and makes `eval` a no-op.
pub fn export_hook(layout: &Layout) -> Result<(), String> {
    let shell = crate::cmd::version_cmd::jenv_shell();

    // A version that cannot be resolved leaves the environment alone rather
    // than clearing JAVA_HOME, which would be worse than being stale.
    let selected = match version::select(layout).ok() {
        Some(selection) => selection.version,
        None => return Ok(()),
    };

    let java_home = match &selected {
        // What `jenv javahome` would print, because that is what the export
        // plugin puts here: the registration path, not the resolved JDK home.
        Version::Installed { name, .. } => Some(
            crate::link::registered_path(layout, name)
                .display()
                .to_string(),
        ),
        Version::System => None,
    };
    // `javac` is what distinguishes a JDK from a JRE; only a JDK has a home
    // worth publishing as JDK_HOME.
    let jdk_home = java_home
        .as_deref()
        .filter(|home| has_javac(std::path::Path::new(home)))
        .map(str::to_string);

    let mut out = String::new();
    for (var, value) in [
        ("JAVA_HOME", java_home.as_deref()),
        ("JENV_FORCEJAVAHOME", java_home.as_deref().map(|_| "true")),
        ("JDK_HOME", jdk_home.as_deref()),
        ("JENV_FORCEJDKHOME", jdk_home.as_deref().map(|_| "true")),
    ] {
        if let Some(code) = render(&shell, var, value) {
            out.push_str(&code);
        }
    }

    print!("{out}");
    Ok(())
}

/// `None` when the environment already agrees, so the caller evals nothing.
fn render(shell: &str, var: &str, value: Option<&str>) -> Option<String> {
    if std::env::var_os(var).map(|v| v.to_string_lossy().into_owned()) == value.map(str::to_string)
    {
        return None;
    }

    let line = match shell {
        "fish" => match value {
            Some(value) => format!("set -gx {var} \"{value}\"\n"),
            None => format!("set -e {var}\n"),
        },
        _ => match value {
            Some(value) => format!("export {var}=\"{value}\"\n"),
            None => format!("unset {var}\n"),
        },
    };
    // PowerShell cannot be eval'd from a posix hook body, and the prompt hook
    // it installs calls this with `pwsh -NoProfile -Command`, so the same
    // syntax works there.
    Some(line)
}

fn has_javac(home: &std::path::Path) -> bool {
    crate::proc::executable_names(&home.join("bin").join("javac"))
        .iter()
        .any(|candidate| crate::proc::is_executable(candidate))
}
