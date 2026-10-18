mod add;
mod config;
pub mod exec;
mod export;
mod init;
mod misc;
pub mod quote;
mod rehash;
pub mod version_cmd;
mod which;

// Explicit, not `pub use add::*`. A wildcard makes every `pub` in every
// submodule part of the crate's surface, which is two problems at once: the
// compiler can no longer see a function nobody calls, and a rename inside a
// submodule silently breaks a caller in another. Only the names `bin/jenv.rs`
// dispatches to are re-exported here; a command that is not listed is a
// command the binary cannot reach, and that is worth finding out at compile
// time.
pub use add::add;
pub use config::{global, local, options, remove, shell};
pub use exec::exec;
pub use export::{export_hook, javahome};
pub use init::init;
pub use misc::{commands, complete, completion_script, completions, doctor, info, skill};
pub use quote::shell_quote;
pub use rehash::rehash;
pub use version_cmd::dispatch;
pub use which::{shims, whence, which};
