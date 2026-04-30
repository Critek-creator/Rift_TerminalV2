//! Default-shell resolution per platform.
//!
//! Three resolution strategies:
//!
//! * [`default_shell`] — legacy zero-discovery resolver. Windows = `%COMSPEC%`
//!   or `cmd.exe`; Unix = `$SHELL` or `/bin/sh`. Kept for callers that don't
//!   plumb a config-driven shell preference (CLI tools, tests).
//! * [`resolve_auto_shell`] — Auto-discovery walk preferred by config-aware
//!   callers. Windows order: `pwsh` → `powershell` → `%COMSPEC%` → `cmd.exe`.
//!   Unix order: `$SHELL` → `zsh` → `bash` → `sh`.
//! * [`resolve_named_shell`] — explicit name lookup ("pwsh", "powershell",
//!   "cmd", "bash", "zsh", "sh"). Returns `None` if the named binary is not
//!   on `PATH`.
//!
//! Shell discovery walks `$PATH` directly via `std::env::var_os`. We avoid
//! the `which` crate to keep this crate dependency-light — PATH walking is
//! 20 lines of std and the same trade rift-bus's `directories` dep makes.

use std::path::{Path, PathBuf};

/// Resolves the default interactive shell for the current platform.
/// Returns `(executable_path, args)`.
///
/// Windows: `%COMSPEC%` if set (typically `cmd.exe`), else `cmd.exe`.
/// Unix:    `$SHELL` if set, else `/bin/sh`.
///
/// This is the legacy zero-discovery path. Config-aware callers should
/// prefer [`resolve_auto_shell`] or [`resolve_named_shell`].
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

/// Walk `$PATH` looking for an executable named `binary`. On Windows, the
/// caller is responsible for passing a name including the `.exe` extension
/// (or for accepting the no-extension form if `PATHEXT` matches). Returns
/// the first match or `None`.
fn find_on_path(binary: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(binary);
        if candidate.is_file() {
            return Some(candidate);
        }
        // On Windows, also try the bare name with PATHEXT-style suffixes
        // commonly omitted by callers.
        #[cfg(windows)]
        {
            for ext in [".exe", ".cmd", ".bat"] {
                let mut with_ext = candidate.clone().into_os_string();
                with_ext.push(ext);
                let p = PathBuf::from(with_ext);
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// Resolve an explicitly-named shell to its absolute path + default args.
///
/// Recognized names: `"pwsh"`, `"powershell"`, `"cmd"`, `"bash"`, `"zsh"`,
/// `"sh"`. Returns `None` if the binary is not on `PATH`. Names are lowercase
/// and platform-agnostic — e.g. `"cmd"` works on Windows even though the
/// binary is `cmd.exe`.
pub fn resolve_named_shell(name: &str) -> Option<(PathBuf, Vec<String>)> {
    let binary: &str = match name {
        "pwsh" => {
            #[cfg(windows)]
            {
                "pwsh.exe"
            }
            #[cfg(unix)]
            {
                "pwsh"
            }
        }
        "powershell" => {
            #[cfg(windows)]
            {
                "powershell.exe"
            }
            #[cfg(unix)]
            {
                return None;
            }
        }
        "cmd" => {
            #[cfg(windows)]
            {
                "cmd.exe"
            }
            #[cfg(unix)]
            {
                return None;
            }
        }
        "bash" => "bash",
        "zsh" => "zsh",
        "sh" => "sh",
        _ => return None,
    };
    find_on_path(binary).map(|p| (p, Vec::new()))
}

/// Auto-discovery walk preferred by config-aware callers.
///
/// Windows order: `pwsh` → `powershell` → `%COMSPEC%` → `cmd.exe` fallback.
/// Unix order:    `$SHELL` (if set) → `zsh` → `bash` → `sh`.
///
/// Falls through to [`default_shell`] when no preferred binary is found,
/// guaranteeing a non-empty result.
pub fn resolve_auto_shell() -> (PathBuf, Vec<String>) {
    #[cfg(windows)]
    {
        if let Some(found) = resolve_named_shell("pwsh") {
            return found;
        }
        if let Some(found) = resolve_named_shell("powershell") {
            return found;
        }
        // Fall through to %COMSPEC%/cmd.exe.
        default_shell()
    }
    #[cfg(unix)]
    {
        if let Ok(shell) = std::env::var("SHELL") {
            if !shell.is_empty() {
                let p = PathBuf::from(&shell);
                if p.is_file() {
                    return (p, Vec::new());
                }
            }
        }
        for name in ["zsh", "bash", "sh"] {
            if let Some(found) = resolve_named_shell(name) {
                return found;
            }
        }
        default_shell()
    }
}

/// Resolve an explicit shell binary path supplied by config (`ShellPref::Custom`).
///
/// Returns `Some` if the path exists and is a file, else `None` so the caller
/// can surface a config-error envelope without crashing the PTY spawn.
pub fn resolve_custom_shell(path: &Path) -> Option<(PathBuf, Vec<String>)> {
    if path.is_file() {
        Some((path.to_path_buf(), Vec::new()))
    } else {
        None
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

    #[test]
    fn auto_returns_a_nonempty_path() {
        // resolve_auto_shell must always return SOME path — it falls through
        // to default_shell when nothing preferred is on PATH.
        let (path, _args) = resolve_auto_shell();
        assert!(!path.as_os_str().is_empty());
    }

    #[test]
    fn unknown_named_shell_returns_none() {
        assert!(resolve_named_shell("definitely-not-a-real-shell-xyz").is_none());
    }

    #[test]
    fn custom_nonexistent_path_returns_none() {
        let bogus = PathBuf::from("/nonexistent/shell/binary/xyz");
        assert!(resolve_custom_shell(&bogus).is_none());
    }
}
