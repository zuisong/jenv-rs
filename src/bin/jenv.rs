use clap::Parser;
use jenv::cli::{Cli, Command, Failure};
use jenv::cmd;
use jenv::layout::Layout;
use jenv::shim;

/// The internal subcommand the installed prompt hook evaluates.
const EXPORT_HOOK: &str = "export-hook";

fn main() {
    // One binary, two jobs. `jenv rehash` hard-links this executable into
    // `shims/` under every command name, so the file name is what says
    // whether this is the CLI or a shim standing in for `java`.
    if let Some(tool) = shim::tool_name() {
        shim::run(&tool);
    }

    // A tool whose output is piped into `head` has to die quietly, the way
    // every other Unix program does. Rust ignores SIGPIPE by default, which
    // would turn a closed pipe into a panic.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    let layout = Layout::from_env();

    // The prompt hook that `jenv init` installs calls this. It is handled
    // before clap rather than declared as a subcommand because `hide = true`
    // is not honoured by every completion generator, and an internal entry
    // point should never turn up in a user's tab menu.
    if std::env::args().nth(1).as_deref() == Some(EXPORT_HOOK) {
        return match cmd::export_hook(&layout) {
            Ok(()) => {}
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1);
            }
        };
    }

    // `jenv init -` is the documented spelling and every existing dotfile and
    // completion uses it, but clap has no way to declare a bare `-` flag.
    let raw = normalize_init_dash(&std::env::args().collect::<Vec<String>>());

    let command = match Cli::parse_from(raw).command {
        Some(command) => command,
        None => {
            // A bare `jenv` is how a user asks what jenv can do.
            let _ = <Cli as clap::CommandFactory>::command().print_help();
            println!();
            return;
        }
    };

    match run(&layout, command) {
        Ok(()) => {}
        Err(failure) => {
            if !failure.message.is_empty() {
                eprintln!("{}", failure.message);
            }
            std::process::exit(failure.code);
        }
    }
}

fn normalize_init_dash(argv: &[String]) -> Vec<String> {
    argv.iter()
        .enumerate()
        .map(|(index, arg)| {
            if index >= 2 && argv[index - 1] == "init" && arg == "-" {
                "--print".to_string()
            } else {
                arg.clone()
            }
        })
        .collect()
}

/// `jenv exec` and `jenv which` both resolve through the shim path, and both
/// answer 127 — what a shell reports for a missing executable — rather than
/// the generic 1. Everything else is an ordinary failure.
fn run(layout: &Layout, command: Command) -> Result<(), Failure> {
    match command {
        Command::Which { command } => cmd::which(layout, &command).map_err(Failure::not_found),
        Command::Exec { args } => match cmd::exec::exec(layout, &args) {
            Err(message) if message.ends_with("command not found") => {
                Err(Failure::not_found(message))
            }
            other => other.map_err(Failure::exit),
        },
        other => dispatch(layout, other).map_err(Failure::exit),
    }
}

fn dispatch(layout: &Layout, command: Command) -> Result<(), String> {
    match command {
        Command::Add { target, force } => cmd::add(layout, &target, force),
        Command::Commands => {
            cmd::commands();
            Ok(())
        }
        Command::Completions { shell } => cmd::completions(shell),
        Command::Complete { field } => {
            cmd::complete(layout, &field);
            Ok(())
        }
        Command::Doctor => cmd::doctor(layout),
        Command::Global { version, unset } => cmd::global(layout, version, unset),
        Command::JavaHome => cmd::javahome(layout),
        Command::Info => {
            cmd::info(layout);
            Ok(())
        }
        Command::Init {
            print,
            no_rehash,
            export,
            shell,
        } => cmd::init(layout, print, no_rehash, !export, shell),
        Command::Local { version, unset } => cmd::local(layout, version, unset),
        Command::Options { verbose } => {
            cmd::options(layout, verbose);
            Ok(())
        }
        Command::Rehash { force } => cmd::rehash(layout, force),
        Command::Remove { versions } => cmd::remove(layout, &versions),
        Command::Skill => {
            cmd::skill();
            Ok(())
        }
        Command::Shims { short } => {
            cmd::shims(layout, short);
            Ok(())
        }
        Command::Shell { version, unset } => cmd::shell(layout, version, unset),
        Command::Whence { command } => {
            for name in cmd::whence(layout, &command) {
                println!("{name}");
            }
            Ok(())
        }
        // `Which` and `Exec` are handled by `run`, for their exit status.
        other => cmd::dispatch(layout, other),
    }
}
