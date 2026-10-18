//! The environment jenv reads: variables, the working directory and `PATH`.
//!
//! Everything jenv decides — which version a directory selects, which shell
//! dialect a hook speaks, which `java` is on the ambient `PATH` — is a
//! question about this environment. It used to be asked by reading
//! `std::env` at a dozen call sites, which has two costs: every reader grew
//! its own rule for empty values and non-UTF-8, and none of it could be tested
//! without spawning a process with a cleared environment.
//!
//! So the questions live here, and the process is one adapter behind them.
//! [`Env::process`] reads the real thing; [`Env::bare`] builds one for a test.
//! Two adapters make the seam real rather than hypothetical.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

pub struct Env {
    vars: BTreeMap<OsString, OsString>,
    /// The working directory with every symlink resolved, as the filesystem
    /// reports it. [`Env::cwd`] may prefer the shell's spelling of it.
    cwd: PathBuf,
}

impl Env {
    /// The environment of this process.
    pub fn process() -> Self {
        Self {
            vars: std::env::vars_os().collect(),
            cwd: std::env::current_dir().unwrap_or_default(),
        }
    }

    /// An environment with no variables, standing in `cwd`. For tests: this is
    /// what makes a decision reachable without a subprocess.
    pub fn bare(cwd: impl Into<PathBuf>) -> Self {
        Self {
            vars: BTreeMap::new(),
            cwd: cwd.into(),
        }
    }

    /// The same environment with `key` set. Builder, so a test can state only
    /// the variables its decision depends on.
    pub fn with(mut self, key: &str, value: impl Into<OsString>) -> Self {
        self.vars.insert(key.into(), value.into());
        self
    }

    /// A variable's value, empty values included.
    pub fn var(&self, key: &str) -> Option<&OsStr> {
        self.vars.get(OsStr::new(key)).map(OsString::as_os_str)
    }

    /// A variable's value as text, or `None` when it is unset or not UTF-8.
    ///
    /// Only for values jenv itself defines and writes — a version name, a
    /// shell name. A `PATH` entry or a directory is an `OsString` and goes
    /// through [`Env::var`], because a path that is not UTF-8 is still a path.
    pub fn text(&self, key: &str) -> Option<&str> {
        self.var(key)?.to_str()
    }

    /// A variable's value, treating empty as unset. An exported-but-empty
    /// variable is how a shell says "cleared", and it must not win over a file.
    pub fn non_empty(&self, key: &str) -> Option<&str> {
        self.text(key).filter(|v| !v.is_empty())
    }

    /// The working directory, spelled the way a user would.
    ///
    /// `current_dir` resolves every symlink, so a shell sitting in a symlinked
    /// directory names it differently. The shell's `pwd` reads `$PWD` instead,
    /// so that is what is reported here — but only when it still names this
    /// directory, because `$PWD` is inherited and can be stale. A wrong `$PWD`
    /// falls back to the resolved path rather than inventing a file that is not
    /// there.
    pub fn cwd(&self) -> PathBuf {
        if let Some(pwd) = self.var("PWD").filter(|v| !v.is_empty()) {
            let logical = PathBuf::from(pwd);
            if std::fs::canonicalize(&logical).ok().as_deref() == Some(self.cwd.as_path()) {
                return logical;
            }
        }
        self.cwd.clone()
    }

    /// The working directory with symlinks resolved, for the rare decision that
    /// must agree with the filesystem rather than with the shell.
    pub fn physical_cwd(&self) -> &Path {
        &self.cwd
    }

    /// Make `p` absolute against the working directory and lexically normalise
    /// it, without requiring it to exist.
    pub fn absolute(&self, p: &Path) -> PathBuf {
        if p.is_absolute() {
            return crate::layout::normalize(p);
        }
        crate::layout::normalize(&self.cwd.join(p))
    }

    /// `$JENV_DIR`, or the physical working directory when it is unset. This is
    /// the directory a hook script treats as the project root.
    pub fn jenv_dir(&self) -> PathBuf {
        match self.var("JENV_DIR") {
            Some(dir) if !dir.is_empty() => PathBuf::from(dir),
            _ => self.cwd.clone(),
        }
    }

    /// The directories on `PATH`, split and ready to search.
    pub fn path_dirs(&self) -> Vec<PathBuf> {
        match self.var("PATH") {
            Some(value) => std::env::split_paths(value).collect(),
            None => Vec::new(),
        }
    }

    /// The shell whose dialect a hook or a `shell` line has to be written in.
    pub fn shell(&self) -> String {
        self.non_empty("JENV_SHELL")
            .map(str::to_string)
            .unwrap_or_else(|| self.default_shell())
    }

    /// Best-effort detection of the shell that invoked us, used only to pick a
    /// hook-script dialect; an explicit `jenv init <shell>` always wins.
    pub fn default_shell(&self) -> String {
        #[cfg(windows)]
        {
            if self.var("PSModulePath").is_some() {
                return "powershell".into();
            }
        }

        self.non_empty("SHELL")
            .map(|s| s.rsplit('/').next().unwrap_or("bash").to_string())
            .unwrap_or_else(|| "bash".into())
    }

    /// The user's home directory, by the platform's own rules.
    pub fn home_dir(&self) -> PathBuf {
        #[cfg(windows)]
        {
            if let Some(p) = self.var("USERPROFILE").filter(|v| !v.is_empty()) {
                return PathBuf::from(p);
            }
            let drive = self.var("HOMEDRIVE").unwrap_or_default();
            let path = self.var("HOMEPATH").unwrap_or_default();
            if !drive.is_empty() || !path.is_empty() {
                return PathBuf::from(format!(
                    "{}{}",
                    drive.to_string_lossy(),
                    path.to_string_lossy()
                ));
            }
        }
        self.var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_variable_is_treated_as_unset() {
        // A shell exports a cleared variable as empty rather than removing it.
        let env = Env::bare("/tmp").with("JENV_VERSION", "").with("SHELL", "");
        assert_eq!(env.non_empty("JENV_VERSION"), None);
        // Nothing set, so the fallback is the documented default.
        assert_eq!(env.shell(), "bash");
    }

    #[test]
    fn the_shell_is_named_from_shell_or_jenv_shell() {
        let login = Env::bare("/tmp").with("SHELL", "/usr/local/bin/fish");
        assert_eq!(login.default_shell(), "fish");

        // `JENV_SHELL` is what `jenv init` writes, and it wins.
        let configured = Env::bare("/tmp")
            .with("SHELL", "/bin/bash")
            .with("JENV_SHELL", "zsh");
        assert_eq!(configured.shell(), "zsh");
    }

    #[test]
    fn a_stale_pwd_does_not_win_over_the_real_directory() {
        let dir = std::env::temp_dir().join(format!("jenv-rs-env-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        // `$PWD` inherits; only the shell that set it can be trusted, so it is
        // used only when it still names this directory.
        let here = Env::bare(&dir).with("PWD", &dir);
        assert_eq!(here.cwd(), here.physical_cwd().to_path_buf());

        let stale = Env::bare(&dir).with("PWD", "/somewhere/else");
        assert_eq!(stale.cwd(), dir);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_relative_path_is_made_absolute_against_the_working_directory() {
        let env = Env::bare("/tmp/base");
        assert_eq!(
            env.absolute(Path::new("sub/./x")),
            PathBuf::from("/tmp/base/sub/x")
        );
        assert_eq!(env.absolute(Path::new("/abs/../y")), PathBuf::from("/y"));
    }
}
