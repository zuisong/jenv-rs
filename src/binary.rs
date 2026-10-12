//! Platform-specific naming of the files jenv creates.

/// The name `stem` carries on this platform: `jenv` versus `jenv.exe`.
pub fn file_name(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_string()
    }
}
