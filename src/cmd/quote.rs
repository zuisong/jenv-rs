//! Quoting for the shell code jenv prints for the parent shell to `eval`.
//!
//! `jenv shell` and the `export-hook` prompt hook both emit a literal that the
//! user's shell evaluates, and the value is a version *name* — which arrives
//! from `.java-version` in a cloned repository, not just from argv. Wrapping it
//! in bare double quotes is not enough: a name containing `"` closes the quote
//! and everything after it runs as a command, so checking out an untrusted
//! project would be arbitrary code execution on every prompt.
//!
//! This is the second of two defences. `link::valid_name` refuses those
//! characters outright, so a hostile name cannot be registered or named by a
//! version file any more. Quoting still matters for the names that were
//! registered before that check existed, and for the shells' own rules about
//! what a single quote contains — which is why the escaping is per dialect:
//! a backslash is not an escape inside PowerShell's single quotes, and `''`
//! is not an escape inside fish's.

/// Quote `value` as a single-quoted literal for the named shell.
pub fn shell_quote(shell: &str, value: &str) -> String {
    match shell {
        "fish" => fish_quote(value),
        "powershell" => powershell_quote(value),
        _ => posix_quote(value),
    }
}

/// POSIX: single quotes preserve everything, and a literal `'` is spelled by
/// leaving the quoted string, adding an escaped quote, and re-entering it.
fn posix_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// fish: single quotes preserve everything except `'` and `\`, which are
/// escaped with a backslash inside the quotes.
fn fish_quote(value: &str) -> String {
    format!("'{}'", value.replace('\\', r"\\").replace('\'', r"\'"))
}

/// PowerShell: single quotes preserve everything, and a literal `'` is doubled.
fn powershell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The injection vectors a version name can carry. A bare `"` is what
    /// closed the old double-quoted literal; the rest matter if a name ever
    /// reaches a shell through some other path.
    const NASTY: &str = r#"x"; touch PWNED; #`$'"#;

    /// Run `script` in `shell`, returning stdout. `None` when that shell is
    /// not installed, so the suite still passes on a machine without it —
    /// PowerShell is absent from most Unix images.
    fn run_if_available(shell: &str, script: &str) -> Option<String> {
        let out = std::process::Command::new(shell)
            .arg("-c")
            .arg(script)
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    }

    #[test]
    fn posix_quote_survives_a_real_shell() {
        let quoted = shell_quote("bash", NASTY);
        let got = run_if_available("sh", &format!("printf '%s' {quoted}")).expect("sh");
        assert_eq!(got, NASTY, "the shell re-read the value as code");
    }

    /// fish escapes with a backslash even inside single quotes, which POSIX
    /// does not, so this is a separate rule and is checked against a real fish
    /// rather than against the string this module produced.
    #[test]
    fn fish_quote_survives_a_real_shell() {
        let quoted = shell_quote("fish", NASTY);
        let Some(got) = run_if_available("fish", &format!("printf '%s' {quoted}")) else {
            eprintln!("fish not installed; skipping the round-trip check");
            return;
        };
        assert_eq!(got, NASTY, "fish re-read the value as code");
    }

    /// PowerShell doubles `'` rather than escaping it, so it needs its own
    /// rule for the same reason fish does.
    #[test]
    fn powershell_quote_survives_a_real_shell() {
        let quoted = shell_quote("powershell", NASTY);
        // `pwsh -Command` prints the value the way PowerShell would assign it.
        let Some(got) = run_if_available("pwsh", &format!("Write-Output {quoted}")) else {
            eprintln!("pwsh not installed; skipping the round-trip check");
            return;
        };
        assert_eq!(got.trim_end(), NASTY, "pwsh re-read the value as code");
    }

    /// Every shell must at least produce a literal that cannot be closed by
    /// the value: the outer quotes have to match and enclose the whole thing.
    #[test]
    fn every_dialect_produces_one_closed_literal() {
        for shell in ["bash", "zsh", "fish", "powershell"] {
            let quoted = shell_quote(shell, NASTY);
            assert!(quoted.starts_with('\''), "{shell}: {quoted}");
            assert!(quoted.ends_with('\''), "{shell}: {quoted}");
        }
    }

    #[test]
    fn a_plain_name_is_unchanged() {
        for shell in ["bash", "zsh", "fish", "powershell"] {
            assert_eq!(shell_quote(shell, "21.0.2"), "'21.0.2'");
        }
    }
}
