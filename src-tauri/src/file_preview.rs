use serde::Serialize;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::ProjectRoot;

#[derive(Clone, Debug, Default, Serialize)]
pub struct FilePreviewResult {
    pub exists: bool,
    pub size_bytes: u64,
    pub modified_iso: String,
    pub language_hint: String,
    pub preview_lines: Vec<String>,
    pub is_binary: bool,
}

#[tauri::command]
pub async fn file_preview(
    path: String,
    project_root: tauri::State<'_, ProjectRoot>,
) -> Result<FilePreviewResult, String> {
    let root = project_root.get();
    tokio::task::spawn_blocking(move || {
        let resolved = resolve_path(&path, &root);

        if !resolved.exists() {
            return Ok(FilePreviewResult::default());
        }

        let meta =
            std::fs::metadata(&resolved).map_err(|e| format!("file_preview: metadata: {e}"))?;

        if meta.is_dir() {
            return Ok(FilePreviewResult {
                exists: true,
                size_bytes: 0,
                modified_iso: format_modified(&meta),
                language_hint: "directory".to_string(),
                preview_lines: Vec::new(),
                is_binary: false,
            });
        }

        let size_bytes = meta.len();
        let modified_iso = format_modified(&meta);
        let language_hint = language_from_ext(&resolved);

        let is_binary = check_binary(&resolved);
        let preview_lines = if is_binary {
            Vec::new()
        } else {
            read_preview_lines(&resolved, 10)
        };

        Ok(FilePreviewResult {
            exists: true,
            size_bytes,
            modified_iso,
            language_hint,
            preview_lines,
            is_binary,
        })
    })
    .await
    .map_err(|e| format!("file_preview: join error: {e}"))?
}

fn resolve_path(path: &str, project_root: &Path) -> PathBuf {
    let p = PathBuf::from(path);
    // Always resolve relative to project root — even absolute paths must be
    // confined within it (prevents path-traversal via absolute path bypass).
    let candidate = if p.is_absolute() {
        p
    } else {
        project_root.join(&p)
    };
    let root_canonical = match std::fs::canonicalize(project_root) {
        Ok(r) => r,
        Err(_) => return project_root.to_path_buf(),
    };
    let canonical = match std::fs::canonicalize(&candidate) {
        Ok(c) => c,
        Err(_) => return project_root.to_path_buf(),
    };
    if !canonical.starts_with(&root_canonical) {
        return project_root.to_path_buf();
    }
    candidate
}

fn format_modified(meta: &std::fs::Metadata) -> String {
    meta.modified()
        .ok()
        .and_then(|t| {
            let dur = t.duration_since(std::time::UNIX_EPOCH).ok()?;
            let secs = dur.as_secs();
            let days = secs / 86400;
            let remaining = secs % 86400;
            let hours = remaining / 3600;
            let mins = (remaining % 3600) / 60;
            let s = remaining % 60;

            let (year, month, day) = epoch_days_to_ymd(days);
            Some(format!(
                "{year:04}-{month:02}-{day:02}T{hours:02}:{mins:02}:{s:02}Z"
            ))
        })
        .unwrap_or_default()
}

fn epoch_days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    // Simplified civil date from epoch days (good enough for display).
    let mut year = 1970u64;
    loop {
        let ydays = if is_leap(year) { 366 } else { 365 };
        if days < ydays {
            break;
        }
        days -= ydays;
        year += 1;
    }
    let leap = is_leap(year);
    let months: [u64; 12] = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1u64;
    for m in months {
        if days < m {
            break;
        }
        days -= m;
        month += 1;
    }
    (year, month, days + 1)
}

fn is_leap(y: u64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn check_binary(path: &Path) -> bool {
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut buf = [0u8; 512];
    let Ok(n) = file.read(&mut buf) else {
        return false;
    };
    buf[..n].contains(&0)
}

fn read_preview_lines(path: &Path, max_lines: usize) -> Vec<String> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    content
        .lines()
        .take(max_lines)
        .map(|l| l.to_string())
        .collect()
}

fn language_from_ext(path: &Path) -> String {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "rs" => "rust",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "svelte" => "svelte",
        "html" | "htm" => "html",
        "css" | "scss" | "sass" => "css",
        "json" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "md" | "markdown" => "markdown",
        "py" => "python",
        "go" => "go",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" => "cpp",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "sh" | "bash" | "zsh" => "shell",
        "sql" => "sql",
        "xml" => "xml",
        "txt" | "text" => "plaintext",
        "lock" => "lockfile",
        "" => "unknown",
        other => other,
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_detection() {
        assert_eq!(language_from_ext(Path::new("foo.rs")), "rust");
        assert_eq!(language_from_ext(Path::new("bar.tsx")), "typescript");
        assert_eq!(language_from_ext(Path::new("baz.svelte")), "svelte");
        assert_eq!(language_from_ext(Path::new("no_ext")), "unknown");
    }

    #[test]
    fn resolve_absolute_outside_root_is_blocked() {
        // An absolute path outside the project root must be rejected —
        // resolve_path falls back to project_root (which then fails exists()).
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir_all(&root).unwrap();
        let outside = dir.path().join("outside.txt");
        std::fs::write(&outside, "secret").unwrap();
        let resolved = resolve_path(outside.to_str().unwrap(), &root);
        // Must NOT equal the outside path — should fall back to root.
        assert_ne!(resolved, outside);
    }

    #[test]
    fn resolve_absolute_inside_root_allowed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let inside = root.join("child.txt");
        std::fs::write(&inside, "ok").unwrap();
        let resolved = resolve_path(inside.to_str().unwrap(), &root);
        assert_eq!(resolved, inside);
    }

    #[test]
    fn resolve_relative_joins_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let child = root.join("src");
        std::fs::create_dir_all(&child).unwrap();
        std::fs::write(child.join("main.rs"), "fn main(){}").unwrap();
        let resolved = resolve_path("src/main.rs", &root);
        assert_eq!(resolved, root.join("src/main.rs"));
    }

    #[test]
    fn resolve_traversal_blocked() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir_all(&root).unwrap();
        let secret = dir.path().join("secret.txt");
        std::fs::write(&secret, "nope").unwrap();
        let resolved = resolve_path("../secret.txt", &root);
        // Must not resolve to the actual secret file.
        assert_ne!(
            std::fs::canonicalize(&resolved).ok(),
            std::fs::canonicalize(&secret).ok()
        );
    }
}
