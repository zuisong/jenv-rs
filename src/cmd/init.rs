use crate::cli::Shell;
use crate::layout::Layout;
use crate::version_cmd::default_shell;
use std::path::Path;

/// `jenv init - [<shell>] [--no-export]`
///
/// The parent shell is the only process that can change its own environment,
/// so this prints code for the caller to `eval` rather than mutating anything
/// itself.
pub fn init(
    layout: &Layout,
    print: bool,
    no_rehash: bool,
    export_home: bool,
    shell: Option<String>,
) -> Result<(), String> {
    let shell = shell
        .or_else(|| std::env::var("JENV_SHELL").ok().filter(|s| !s.is_empty()))
        .unwrap_or_else(default_shell);

    if !print {
        let profile = match shell.as_str() {
            "bash" => "~/.bash_profile",
            "zsh" => "~/.zshrc",
            "fish" => "~/.config/fish/config.fish",
            "powershell" => "$PROFILE",
            _ => "your profile",
        };
        let snippet = match shell.as_str() {
            "fish" => "status --is-interactive; and jenv init - | source",
            "powershell" => "jenv init - | Invoke-Expression",
            _ => "eval \"$(jenv init -)\"",
        };
        eprintln!("# Load jenv automatically by adding");
        eprintln!("# the following to {profile}:");
        eprintln!();
        eprintln!("{snippet}");
        eprintln!();
        return Err(format!("jenv: {shell} setup required"));
    }

    layout.ensure_dirs().map_err(|e| format!("jenv: {e}"))?;

    let shims = layout.shims_dir();
    let shell = parse_shell(&shell)?;
    let code = match shell {
        Shell::Fish => fish_code(&shims, export_home),
        Shell::PowerShell => powershell_code(&shims, export_home),
        Shell::Bash | Shell::Zsh => posix_code(&shims, shell, export_home),
    };
    print!("{code}");

    // The completion script is generated from the clap definition rather than
    // shipped as a file, so it can never drift from the CLI, and it is
    // inlined because `source <(...)` silently reads nothing under bash.
    print!("{}", crate::cmd::completion_script(shell));

    if !no_rehash {
        // The shell is mid-`eval` here; a rehash that fails must not abort it.
        let _ = crate::cmd::rehash::rehash(layout, false);
    }
    Ok(())
}

fn parse_shell(name: &str) -> Result<Shell, String> {
    match name {
        "bash" => Ok(Shell::Bash),
        "zsh" => Ok(Shell::Zsh),
        "fish" => Ok(Shell::Fish),
        "powershell" | "pwsh" => Ok(Shell::PowerShell),
        other => Err(format!("jenv: unsupported shell `{other}'")),
    }
}

fn shell_name(shell: Shell) -> &'static str {
    match shell {
        Shell::Bash => "bash",
        Shell::Zsh => "zsh",
        Shell::Fish => "fish",
        Shell::PowerShell => "powershell",
    }
}

/// bash and zsh share everything but the name of the prompt hook they use to
/// install the export watcher.
fn posix_code(shims: &Path, shell: Shell, export_home: bool) -> String {
    let shims = shims.display();
    // The shell the caller asked for, not the one `$SHELL` happens to name:
    // `jenv init - zsh` from a bash login must still set up zsh.
    let name = shell_name(shell);

    let mut code = format!(
        r#"export PATH="{shims}:$PATH"
export JENV_SHELL="{name}"
export JENV_LOADED=1
"#
    );

    if export_home {
        code.push_str(&posix_export_hook(shell));
    } else {
        code.push_str("unset JAVA_HOME\nunset JDK_HOME\n");
    }

    code.push_str(
        r#"
jenv() {
  if [ "$1" = "shell" ]; then
    shift
    eval "$(command jenv shell "$@")"
  else
    command jenv "$@"
  fi
}
"#,
    );
    code
}

/// jenv's `export` plugin, inlined.
///
/// It re-runs before every prompt so that `cd`-ing into another project
/// updates `JAVA_HOME`. There is no way to do that without a hook, because only
/// the shell can observe that the directory changed.
fn posix_export_hook(shell: Shell) -> String {
    let body = "_jenv_export_hook() { eval \"$(command jenv export-hook)\"; }\n";

    match shell {
        Shell::Zsh => format!(
            "{body}\
             typeset -ag precmd_functions\n\
             [[ -z ${{precmd_functions[(r)_jenv_export_hook]}} ]] && precmd_functions+=(_jenv_export_hook)\n\
             _jenv_export_hook\n"
        ),
        // bash 5.1+ replaced PROMPT_COMMAND's string form with an array, so
        // both spellings are registered where the old one still works.
        _ => format!(
            "{body}\
             if [[ $(declare -p PROMPT_COMMAND 2>/dev/null) == declare\\ -a* ]]; then\n\
             \x20 PROMPT_COMMAND=(_jenv_export_hook \"${{PROMPT_COMMAND[@]}}\")\n\
             else\n\
             \x20 PROMPT_COMMAND=\"_jenv_export_hook;$PROMPT_COMMAND\"\n\
             fi\n\
             _jenv_export_hook\n"
        ),
    }
}

fn fish_code(shims: &Path, export_home: bool) -> String {
    let shims = shims.display();
    let mut code = format!(
        r#"set -gx PATH '{shims}' $PATH
set -gx JENV_SHELL fish
set -gx JENV_LOADED 1
"#
    );

    if export_home {
        code.push_str(
            r#"
function __jenv_export_hook --on-event fish_prompt
  command jenv export-hook | source
end
__jenv_export_hook
"#,
        );
    } else {
        code.push_str("set -e JAVA_HOME\nset -e JDK_HOME\n");
    }

    code.push_str(
        r#"
function jenv
  if test "$argv[1]" = "shell"
    command jenv shell $argv[2..-1] | source
  else
    command jenv $argv
  end
end
"#,
    );
    code
}

fn powershell_code(shims: &Path, export_home: bool) -> String {
    let shims = shims.display();
    // PowerShell is not only a Windows shell: `jenv init - powershell` works
    // wherever pwsh is installed, and the binary there is `jenv`, not
    // `jenv.exe`. Naming the wrong one makes every hook call fail.
    let exe = format!("jenv{}", std::env::consts::EXE_SUFFIX);
    let mut code = format!(
        r#"$env:PATH = "{shims};$env:PATH"
$env:JENV_SHELL = "powershell"
$env:JENV_LOADED = "1"
"#
    );

    if export_home {
        // PowerShell has no precmd hook, so the prompt function itself is
        // wrapped, keeping whatever the user already had.
        code.push_str(
            r#"
if (-not (Test-Path function:__jenv_original_prompt)) {
    Copy-Item function:prompt function:__jenv_original_prompt
}
function global:prompt {
    __JENV_EXE__ export-hook | Invoke-Expression
    __jenv_original_prompt
}
__JENV_EXE__ export-hook | Invoke-Expression
"#,
        );
    } else {
        code.push_str(
            "Remove-Item Env:JAVA_HOME -ErrorAction SilentlyContinue\n\
             Remove-Item Env:JDK_HOME -ErrorAction SilentlyContinue\n",
        );
    }

    code.push_str(
        r#"
function global:jenv {
  if ($args.Count -gt 0 -and $args[0] -eq "shell") {
    $rest = @()
    if ($args.Count -gt 1) { $rest = $args[1..($args.Count - 1)] }
    Invoke-Expression ((__JENV_EXE__ shell @rest) | Out-String)
  } else {
    & __JENV_EXE__ @args
  }
}
"#,
    );
    code = code.replace("__JENV_EXE__", &exe);
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posix_setup_names_the_requested_shell_not_the_login_one() {
        let code = posix_code(Path::new("/root/.jenv/shims"), Shell::Zsh, false);
        assert!(code.contains(r#"export JENV_SHELL="zsh""#), "{code}");
        assert!(code.contains(r#"export PATH="/root/.jenv/shims:$PATH""#));
        assert!(code.contains("unset JAVA_HOME"), "{code}");
    }

    #[test]
    fn the_export_hook_is_installed_per_shell_and_absent_otherwise() {
        let zsh = posix_code(Path::new("/s"), Shell::Zsh, true);
        assert!(zsh.contains("precmd_functions"), "{zsh}");
        assert!(zsh.contains("jenv export-hook"), "{zsh}");
        assert!(!zsh.contains("unset JAVA_HOME"), "{zsh}");

        // bash 5.1 turned PROMPT_COMMAND into an array, so both shapes matter.
        let bash = posix_code(Path::new("/s"), Shell::Bash, true);
        assert!(bash.contains("declare -p PROMPT_COMMAND"), "{bash}");
        assert!(!bash.contains("unset JAVA_HOME"), "{bash}");

        let fish = fish_code(Path::new("/s"), true);
        assert!(fish.contains("--on-event fish_prompt"), "{fish}");
        assert!(!fish.contains("set -e JAVA_HOME"), "{fish}");

        let pwsh = powershell_code(Path::new(r"C:\s"), true);
        assert!(pwsh.contains("function global:prompt"), "{pwsh}");
        assert!(!pwsh.contains("Remove-Item Env:JAVA_HOME"), "{pwsh}");
    }

    #[test]
    fn powershell_setup_uses_the_windows_path_separator() {
        let code = powershell_code(Path::new(r"C:\Users\dev\.jenv\shims"), false);
        assert!(code.contains(r#"$env:PATH = "C:\Users\dev\.jenv\shims;$env:PATH""#));
    }

    #[test]
    fn an_unknown_shell_is_named_in_the_error() {
        assert!(
            parse_shell("csh")
                .unwrap_err()
                .contains("unsupported shell")
        );
        assert_eq!(parse_shell("pwsh").unwrap(), Shell::PowerShell);
    }
}
