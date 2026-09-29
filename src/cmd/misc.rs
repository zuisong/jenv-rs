use crate::cli::{Cli, Shell};
use crate::layout::Layout;
use std::io::Write;

/// `jenv commands` — the subcommands the wrapper dispatches to.
pub fn commands() {
    for name in subcommand_names() {
        println!("{name}");
    }
}

/// Derived from the same clap definition the parser uses, so a new subcommand
/// cannot be half-registered.
fn subcommand_names() -> Vec<String> {
    use clap::CommandFactory;
    Cli::command()
        .get_subcommands()
        .filter(|c| !c.is_hide_set())
        .map(|c| c.get_name().to_string())
        .collect()
}

/// `jenv completions <shell>`
pub fn completions(shell: Shell) -> Result<(), String> {
    // Generated into memory first: clap_complete panics if the writer fails,
    // and a completion script piped into `head` closes the pipe early.
    let mut out = std::io::stdout().lock();
    out.write_all(completion_script(shell).as_bytes())
        .map_err(|e| format!("jenv: {e}"))?;
    out.flush().map_err(|e| format!("jenv: {e}"))
}

/// The full completion script, static and dynamic halves together.
///
/// `jenv init -` inlines this rather than sourcing it from a process
/// substitution: `source <(...)` silently reads nothing under bash, and
/// `eval`ing the whole thing is both simpler and one less subprocess at shell
/// startup.
pub fn completion_script(shell: Shell) -> String {
    use clap::CommandFactory;

    let generator = match shell {
        Shell::Bash => clap_complete::Shell::Bash,
        Shell::Zsh => clap_complete::Shell::Zsh,
        Shell::Fish => clap_complete::Shell::Fish,
        Shell::PowerShell => clap_complete::Shell::PowerShell,
    };

    let mut script = Vec::new();
    clap_complete::generate(generator, &mut Cli::command(), "jenv", &mut script);
    script.extend_from_slice(dynamic_completions(shell).as_bytes());
    String::from_utf8_lossy(&script).into_owned()
}

/// Candidate lists that depend on what is installed, which no static script
/// can contain. `jenv complete <field>` is what these call back into.
fn dynamic_completions(shell: Shell) -> &'static str {
    match shell {
        Shell::Bash => {
            // `complete -F` replaces the registration rather than adding to
            // it, so this has to call clap's `_jenv` itself; registering a
            // second function for the same command would silently delete
            // every static answer.
            //
            // `-o default` restores bash's filename completion for the
            // commands that take a path, which a function that returns
            // nothing would otherwise suppress.
            //
            // `r##` because the script itself contains `"#`.
            r##"# jenv-rs: dynamic candidates
_jenv_dynamic() {
    _jenv "$@"
    local cur="${COMP_WORDS[COMP_CWORD]}"
    case "${COMP_WORDS[1]}" in
        local|global|shell|prefix|remove)
            COMPREPLY+=( $(compgen -W "$(command jenv complete versions 2>/dev/null)" -- "$cur") ) ;;
        which|whence|exec|options)
            COMPREPLY+=( $(compgen -W "$(command jenv complete shims 2>/dev/null)" -- "$cur") ) ;;
    esac
}
complete -F _jenv_dynamic -o default jenv
"##
        }
        Shell::Fish => {
            r##"# jenv-rs: dynamic candidates
complete -c jenv -f -n '__fish_seen_subcommand_from local global shell prefix remove' -a '(jenv complete versions)'
complete -c jenv -f -n '__fish_seen_subcommand_from which whence exec options' -a '(jenv complete shims)'
"##
        }
        // zsh and PowerShell need a full completion function to merge dynamic
        // values, which is more machinery than the static script is worth.
        // `jenv complete <field>` is still there for anyone who wants to wire
        // it into their own setup.
        Shell::Zsh | Shell::PowerShell => "",
    }
}

/// `jenv complete <field>` — candidates a shell cannot know on its own.
pub fn complete(layout: &Layout, field: &str) {
    match field {
        "versions" => complete_versions(layout),
        "shims" => crate::cmd::which::shims(layout, true),
        "subcommands" => {
            for name in subcommand_names() {
                println!("{name}");
            }
        }
        other => eprintln!("jenv: no completion field `{other}'"),
    }
}

fn complete_versions(layout: &Layout) {
    println!("system");
    for name in crate::link::list(layout) {
        println!("{name}");
    }
}

/// `jenv info` — enough to diagnose a broken install over a bug report.
pub fn info(layout: &Layout) {
    let version = match crate::version::select(layout) {
        Ok(selection) => format!("{} (set by {})", selection.version.name(), selection.origin),
        Err(e) => e,
    };

    let entries: [(&str, String); 7] = [
        ("JENV_ROOT", layout.root.display().to_string()),
        ("JENV_SHELL", crate::version_cmd::jenv_shell()),
        (
            "JENV_VERSION",
            std::env::var("JENV_VERSION").unwrap_or_default(),
        ),
        (
            "JENV_DIR",
            crate::version_cmd::jenv_dir().display().to_string(),
        ),
        ("active version", version),
        ("versions", crate::link::list(layout).join(", ")),
        (
            "shims",
            std::fs::read_dir(layout.shims_dir())
                .map(|entries| {
                    entries
                        .flatten()
                        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
                        .count()
                        .to_string()
                })
                .unwrap_or_else(|_| "0".into()),
        ),
    ];

    let width = entries.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
    for (key, value) in entries {
        println!("{key:<width$}  {value}", width = width);
    }
}

/// `jenv doctor` — checks that would otherwise fail confusingly later.
pub fn doctor(layout: &Layout) -> Result<(), String> {
    let mut problems = Vec::new();

    for dir in [
        layout.root.clone(),
        layout.versions_dir(),
        layout.shims_dir(),
    ] {
        if !dir.is_dir() {
            problems.push(format!(
                "{} does not exist (run `jenv init -`)",
                dir.display()
            ));
        }
    }

    if let Ok(selection) = crate::version::select(layout)
        && let crate::link::Version::Installed { name, home } = &selection.version
        && !home.join("bin").join("java").exists()
        && !home.join("bin").join("java.exe").exists()
    {
        problems.push(format!(
            "version `{name}` has no java in {}",
            home.display()
        ));
    }

    // Not setting JENV_ROOT is the normal case, not a problem: the default is
    // a perfectly good location. Only an override that points nowhere is.

    if problems.is_empty() {
        println!("jenv: no problems detected");
        return Ok(());
    }
    for problem in &problems {
        eprintln!("jenv: {problem}");
    }
    Err(format!("jenv: {} problem(s) found", problems.len()))
}
