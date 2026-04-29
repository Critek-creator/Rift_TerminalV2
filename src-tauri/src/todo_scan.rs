// Phase 8.7i — TODO scanner for the new TODO notif tab.
//
// Walks the project root looking for `TODO`, `FIXME`, `XXX`, `HACK` markers
// in source/text files. Returns up to `MAX_RESULTS` entries so the frontend
// payload stays bounded for very large repos.
//
// Filtering:
//   - Honors the same ignore-globs as `fs_tree` (caller passes the GlobSet).
//   - Only opens files whose extension is in `SCANNABLE_EXTENSIONS`.
//   - Skips files larger than `MAX_FILE_BYTES` to avoid stalling on
//     accidentally-checked-in binaries / lockfiles.

use globset::GlobSet;
use serde::Serialize;
use std::path::Path;

/// Hard cap so a runaway match doesn't flood the IPC channel.
const MAX_RESULTS: usize = 1000;

/// Per-file size cap. Anything larger is treated as binary/generated.
const MAX_FILE_BYTES: u64 = 1_048_576; // 1 MiB

/// Recursion depth — matches `FS_TREE_DEFAULT_MAX_DEPTH` semantically.
const MAX_DEPTH: u32 = 16;

/// File extensions we consider "scannable" — i.e. text source/markup. Kept
/// conservative; extending later is cheaper than scanning gigabytes of
/// `node_modules` indexes.
const SCANNABLE_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "svelte", "vue", "py", "go", "java", "kt", "kts",
    "cpp", "cc", "cxx", "hpp", "hxx", "c", "h", "m", "mm", "swift", "rb", "php", "cs", "scala",
    "lua", "sh", "bash", "zsh", "fish", "ps1", "psm1", "md", "mdx", "txt", "toml", "yaml", "yml",
    "json", "css", "scss", "sass", "less", "html", "xml", "sql",
];

/// Markers we recognise. Order matters for tie-breaking: the first that
/// matches wins (so `TODO` beats `HACK` when both somehow appear on a line).
const MARKERS: &[&str] = &["TODO", "FIXME", "XXX", "HACK"];

#[derive(Debug, Clone, Serialize)]
pub struct TodoEntry {
    /// Project-relative path with forward-slash separators.
    pub path: String,
    /// 1-based line number.
    pub line: u32,
    /// One of `TODO` / `FIXME` / `XXX` / `HACK`.
    pub marker: String,
    /// Trimmed text after the marker — comment body without leading colon /
    /// dash / bracket / spaces. Capped to 200 chars.
    pub message: String,
}

/// Scan `root` recursively for TODO-style markers. Returns at most
/// [`MAX_RESULTS`] entries, sorted by `(path, line)` for stable display.
pub fn scan_todos(root: &Path, ignore_globs: &GlobSet) -> Vec<TodoEntry> {
    let canon = match dunce::canonicalize(root) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("todo_scan: canonicalize failed for {}: {e}", root.display());
            return Vec::new();
        }
    };

    let mut out: Vec<TodoEntry> = Vec::new();
    walk(&canon, &canon, 1, ignore_globs, &mut out);
    out.sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
    out
}

fn walk(root: &Path, dir: &Path, depth: u32, ignore_globs: &GlobSet, out: &mut Vec<TodoEntry>) {
    if depth > MAX_DEPTH || out.len() >= MAX_RESULTS {
        return;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(it) => it,
        Err(e) => {
            tracing::debug!("todo_scan: read_dir failed for {}: {e}", dir.display());
            return;
        }
    };

    for entry_result in entries {
        if out.len() >= MAX_RESULTS {
            return;
        }
        let entry = match entry_result {
            Ok(e) => e,
            Err(_) => continue,
        };
        let full_path = entry.path();

        let rel = match full_path.strip_prefix(root) {
            Ok(r) => r.to_path_buf(),
            Err(_) => continue,
        };
        let rel_str = forward_slash(&rel);

        // Probe ignore globs — directories also test a synthetic child entry
        // so patterns like `target/**` suppress the whole subtree.
        if ignore_globs.is_match(&rel_str) {
            continue;
        }

        let file_type = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };

        if file_type.is_dir() {
            // Skip whole subtree if a probe path is matched.
            let probe = format!("{rel_str}/_probe");
            if ignore_globs.is_match(&probe) {
                continue;
            }
            walk(root, &full_path, depth + 1, ignore_globs, out);
            continue;
        }

        if !file_type.is_file() {
            continue;
        }

        if !is_scannable_extension(&full_path) {
            continue;
        }

        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if metadata.len() > MAX_FILE_BYTES {
            continue;
        }

        scan_file(&full_path, &rel_str, out);
    }
}

fn forward_slash(rel: &Path) -> String {
    rel.to_string_lossy().replace('\\', "/")
}

fn is_scannable_extension(path: &Path) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => SCANNABLE_EXTENSIONS
            .iter()
            .any(|&w| w.eq_ignore_ascii_case(ext)),
        None => false,
    }
}

fn scan_file(full_path: &Path, rel_str: &str, out: &mut Vec<TodoEntry>) {
    let content = match std::fs::read_to_string(full_path) {
        Ok(c) => c,
        Err(_) => return, // probably non-UTF8 or transient — skip silently
    };

    for (idx, line) in content.lines().enumerate() {
        if out.len() >= MAX_RESULTS {
            return;
        }
        if let Some((marker, message)) = extract_marker(line) {
            // Filter trivially obvious self-references like the constant
            // `MARKERS` array above — only flag lines that look like a
            // comment (contain `//`, `#`, `/*`, `<!--`, `--`, or `;`).
            if !looks_like_comment(line) {
                continue;
            }
            out.push(TodoEntry {
                path: rel_str.to_string(),
                line: (idx as u32) + 1,
                marker: marker.to_string(),
                message: trim_message(message),
            });
        }
    }
}

fn extract_marker(line: &str) -> Option<(&'static str, &str)> {
    for marker in MARKERS {
        if let Some(pos) = line.find(marker) {
            // Word-boundary check: the byte before must not be alphanumeric
            // (so `MASTODON` doesn't match `TODO`).
            let before_ok = pos == 0 || !line.as_bytes()[pos - 1].is_ascii_alphanumeric();
            let after_idx = pos + marker.len();
            let after_ok =
                after_idx == line.len() || !line.as_bytes()[after_idx].is_ascii_alphanumeric();
            if before_ok && after_ok {
                let rest = &line[after_idx..];
                return Some((marker, rest));
            }
        }
    }
    None
}

fn looks_like_comment(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("//")
        || trimmed.starts_with('#')
        || trimmed.starts_with("/*")
        || trimmed.starts_with('*')
        || trimmed.starts_with("<!--")
        || trimmed.starts_with("--")
        || trimmed.starts_with(';')
        // Inline comments (e.g. `let x = 1; // TODO ...`) — line must contain
        // a comment opener anywhere before the marker. Approximate by
        // requiring `//`, `#`, or `/*` in the line at all.
        || line.contains("//")
        || line.contains("/*")
        || line.contains('#')
}

fn trim_message(rest: &str) -> String {
    let s = rest.trim_start_matches(|c: char| {
        c == ':' || c == '-' || c == ']' || c == '(' || c == ')' || c.is_whitespace()
    });
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i >= 200 {
            out.push('…');
            return out;
        }
        out.push(ch);
    }
    out
}
