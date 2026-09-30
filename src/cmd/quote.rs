//! Quoting for the shell code jenv prints for the parent shell to `eval`.
//!
//! `jenv shell` and the `export-hook` prompt hook both emit a literal that the
//! user's shell evaluates, and the value is a version *name* — which arrives
//! from `.java-version` in a cloned repository, not just from argv. Wrapping it
//! in bare double quotes is not enough: a name containing `"` closes the quote
//! and everything after it runs as a command, so checking out an untrusted
//! project would be arbitrary code execution on every prompt.
//!
//! Each dialect needs its own escaping, because a backslash is not an escape
//! character inside PowerShell's single quotes and `''` is not an escape inside
//! fish's.

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

    /// The characters `link::valid_name` allows are the ones that must survive
    /// quoting; these are the injection vectors a version name can carry.
    const NASTY: &str = r#"x"; id>PWNED; #`$'"#;

    #[test]
    fn posix_quotes_neutralise_the_injection() {
        let quoted = shell_quote("bash", NASTY);
        assert_eq!(quoted, "'x\"; id>PWNED; #`$'\\'''");

        // The only real test: a shell must read the literal back unchanged,
        // with nothing interpreted.
        let out = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(format!("printf '%s' {quoted}"))
            .output()
            .expect("sh");
        assert_eq!(String::from_utf8_lossy(&out.stdout), NASTY);
    }

    #[test]
    fn fish_escapes_backslash_and_quote() {
        assert_eq!(shell_quote("fish", r"a'b\c"), r"'a\'b\\c'");
    }

    #[test]
    fn powershell_doubles_the_quote() {
        assert_eq!(shell_quote("powershell", "a'b"), "'a''b'");
    }

    #[test]
    fn a_plain_name_is_unchanged() {
        for shell in ["bash", "zsh", "fish", "powershell"] {
            assert_eq!(shell_quote(shell, "21.0.2"), "'21.0.2'");
        }
    }
}
