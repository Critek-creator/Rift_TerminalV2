//! Default-shell resolution per platform.

use std::path::PathBuf;

/// Resolves the default interactive shell for the current platform.
/// Returns `(executable_path, args)`.
///
/// Windows: `%COMSPEC%` if set (typically `cmd.exe`), else `cmd.exe`.
/// Unix:    `$SHELL` if set, else `/bin/sh`.
pub fn default_shell() -> (PathBuf, Vec<String>) {
    #[cfg(windows)]
    {
        if let Ok(comspec) = std::env::var("COMSPEC") {
            if !comspec.is_empty() {
                return (PathBuf::from(comspec), Vec::new());
            }
        }
        (PathBuf::from("cmd.exe"), Vec::new())
    }
    #[cfg(unix)]
    {
        if let Ok(shell) = std::env::var("SHELL") {
            if !shell.is_empty() {
                return (PathBuf::from(shell), Vec::new());
            }
        }
        (PathBuf::from("/bin/sh"), Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_a_nonempty_path() {
        let (path, _args) = default_shell();
        assert!(!path.as_os_str().is_empty());
    }
}
