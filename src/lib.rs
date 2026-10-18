pub mod binary;
pub mod cli;
pub mod cmd;
pub mod env;
pub mod layout;
pub mod link;
pub mod path;
pub mod probe;
pub mod process;
pub mod shim;
pub mod version;

/// Re-exported because version resolution is reached from everywhere, and
/// spelling out `cmd::version_cmd` at each call site is noise.
pub use cmd::version_cmd;
