use clap::{Parser, Subcommand, ValueEnum};

/// The shells jenv can configure and generate completions for.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
    #[value(name = "powershell")]
    PowerShell,
}

/// A command that failed, and the status the shell should see for it.
///
/// jenv uses 127 for "no such command", which is what a shell reports for a
/// missing executable, and 1 for everything else. Carrying the number here is
/// what keeps a command from printing its own error and then having `main`
/// print it a second time.
pub struct Failure {
    pub message: String,
    pub code: i32,
}

impl Failure {
    /// The ordinary case: a message on stderr and a non-zero status.
    pub fn exit(message: String) -> Self {
        Self { message, code: 1 }
    }

    /// 127, as `command not found` means to a shell.
    pub fn not_found(message: String) -> Self {
        Self { message, code: 127 }
    }
}

#[derive(Parser, Debug)]
#[command(
    name = "jenv",
    about = "Manage multiple Java installations",
    version,
    disable_help_subcommand = true
)]
pub struct Cli {
    // `jenv` on its own is a request for help, not a missing argument.
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Register a JDK that is already installed on this machine
    Add {
        /// The JDK home and, optionally, the single name to register it under.
        ///
        /// jenv accepts these in either order, choosing by which argument is a
        /// directory, so `jenv add <path>` and `jenv add <alias> <path>` both
        /// work. Anything that is not a directory is taken as the name.
        target: Vec<String>,
        /// Replace an existing registration without asking.
        #[arg(long)]
        force: bool,
    },

    /// List the subcommands jenv dispatches to
    Commands,

    /// Print a completion script for the given shell
    Completions { shell: Shell },

    /// Print completion candidates for a field
    ///
    /// Shell completion scripts cannot know which JDKs are installed, so they
    /// shell back out to this. Not meant to be typed by hand.
    #[command(hide = true)]
    Complete { field: String },

    /// Check the installation for problems
    Doctor,

    /// Run a command against the selected version
    Exec {
        #[arg(
            required = true,
            num_args = 1..,
            trailing_var_arg = true,
            allow_hyphen_values = true
        )]
        args: Vec<String>,
    },

    /// Set or show the version used outside any project
    Global {
        version: Option<String>,
        #[arg(long)]
        unset: bool,
    },

    /// Print `JAVA_HOME` for the active version
    ///
    /// Exits non-zero when the active version is `system`, which is what jenv
    /// does and what the `export` hook relies on to clear the variable.
    #[command(name = "javahome")]
    JavaHome,

    /// Show the resolved configuration
    Info,

    /// Configure the shell environment for jenv
    Init {
        /// Print the shell code instead of writing instructions to stderr.
        #[arg(long = "print")]
        print: bool,
        /// Skip regenerating shims, which matters on a slow filesystem.
        #[arg(long)]
        no_rehash: bool,
        /// Leave `JAVA_HOME` and `JDK_HOME` alone instead of keeping them in
        /// sync with the selected version. Without this, jenv-rs installs a
        /// prompt hook that re-exports them, which is jenv's `export` plugin
        /// and the only way they can follow a `cd` into another project.
        #[arg(long = "no-export")]
        export: bool,
        shell: Option<String>,
    },

    /// Set or show the version for the current directory
    Local {
        version: Option<String>,
        #[arg(long)]
        unset: bool,
    },

    /// Show the options applied to the selected version
    Options {
        #[arg(long)]
        verbose: bool,
    },

    /// Print the prefix of a version
    Prefix { version: Option<String> },

    /// Regenerate the shims in the shims directory
    Rehash {
        /// Replace shims left behind by a crashed or concurrent rehash.
        #[arg(long)]
        force: bool,
    },

    /// Unregister installed versions
    Remove { versions: Vec<String> },

    /// Print the jenv root directory
    Root,

    /// List the commands that have a shim
    Shims {
        #[arg(long)]
        short: bool,
    },

    /// Set or show the version for the current shell session
    Shell {
        version: Option<String>,
        #[arg(long)]
        unset: bool,
    },

    /// Show the active version and where it came from
    Version,

    /// Print the file that determines the active version
    VersionFile,

    /// Print the name of the active version
    VersionName,

    /// Print where the active version came from
    VersionOrigin,

    /// List the installed versions
    Versions {
        #[arg(long)]
        bare: bool,
        #[arg(long)]
        verbose: bool,
    },

    /// List the versions that provide a command
    Whence { command: String },

    /// Print the path a command resolves to
    Which { command: String },
}
