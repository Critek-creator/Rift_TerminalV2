//! Filesystem-watcher translator — publishes `Category::Fs` envelopes.
//!
//! This translator is the **sole entry point** for OS filesystem events into
//! the Rift bus. The backend spawns one watcher per project root via
//! [`spawn_fs_watcher`]; all subsequent publish calls happen automatically on
//! the internal dispatcher thread. External code can also call
//! [`publish_fs_event`] directly (e.g. for testing or synthetic events).
//!
//! ## Design notes
//!
//! ### Kind taxonomy
//!
//! Four kinds are emitted under `Category::Fs`:
//!
//! | kind       | trigger                            |
//! |------------|------------------------------------|
//! | `"create"` | new file or directory              |
//! | `"write"`  | file contents modified             |
//! | `"delete"` | file or directory removed          |
//! | `"rename"` | file moved (both sides known); if only one side is reported, falls back to `"delete"` for the known path only (the unknown side cannot be inferred without both paths) |
//!
//! **Omitted in Phase 6.1** (each silently dropped at the dispatch layer):
//!
//! - `"read"` / Access events — `notify` 6.x does not reliably emit
//!   Access/Read events on Windows or macOS without platform-specific filter
//!   configuration; including it would violate the Completeness Principle
//!   (cross-platform stub). Design for Phase 6.x extension once read-tracking
//!   requirements are confirmed.
//! - `EventKind::Other` / `EventKind::Any` — backend-specific signals with no
//!   stable cross-platform meaning; the dispatcher's catch-all arm drops them.
//!
//! ### Path normalization
//!
//! All paths in payloads are **relative to the watcher root** and use
//! forward-slash separators on all platforms (Windows backslashes are
//! replaced). If a path falls outside the root — which should not happen in
//! practice — it is logged via `tracing::warn!` and skipped.
//!
//! ### Ignore globs
//!
//! Ignore patterns are compiled into a [`globset::GlobSet`] and matched
//! against the **relative** (post-normalization) path. Matching paths are
//! silently dropped. Default globs set by the watcher caller:
//! `.git/**`, `node_modules/**`, `target/**`, `dist/**`, `*.log`.
//!
//! ### Debouncing
//!
//! None in Phase 6.1. `notify` bare (without `notify_debouncer_*`) may emit
//! multiple `Modify` events for one logical save; the frontend activity store
//! collapses bursts visually. Not adding debouncer crates is the locked v1
//! call.
//!
//! ## Payload shapes
//!
//! ```json
//! // create
//! { "path": "src/lib.rs" }
//!
//! // write
//! { "path": "src/main.rs" }
//!
//! // delete
//! { "path": "old_file.rs" }
//!
//! // rename (both sides known)
//! { "from": "old.rs", "to": "new.rs" }
//! ```
//!
//! `kind` values under `Category::Fs` are additive and do NOT bump
//! `CURRENT_VERSION` (per `envelope-version-additive-categories-no-bump`).

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;

use globset::{Glob, GlobSet, GlobSetBuilder};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::json;
use thiserror::Error;

use crate::{Category, Envelope, RiftBus};

// ---------------------------------------------------------------------------
// FsEvent — the typed taxonomy passed to publish_fs_event
// ---------------------------------------------------------------------------

/// A filesystem event ready to publish. Constructed from a `notify::Event`
/// by the dispatcher thread and then forwarded to [`publish_fs_event`].
pub enum FsEvent {
    /// A new file or directory was created.
    Create {
        /// Relative path from the watcher root, forward-slash normalized.
        path: String,
    },
    /// A file's contents were modified.
    Write {
        /// Relative path from the watcher root, forward-slash normalized.
        path: String,
    },
    /// A file or directory was removed.
    Delete {
        /// Relative path from the watcher root, forward-slash normalized.
        path: String,
    },
    /// A file was moved and both sides are known.
    Rename {
        /// Original relative path.
        from: String,
        /// New relative path.
        to: String,
    },
}

// ---------------------------------------------------------------------------
// FsWatcherError
// ---------------------------------------------------------------------------

/// Errors that can occur when spawning the filesystem watcher.
#[derive(Debug, Error)]
pub enum FsWatcherError {
    /// The root path could not be canonicalized.
    #[error("failed to canonicalize root path '{path}': {source}")]
    CanonicalizeFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// `notify` failed to initialize or watch the root.
    #[error("notify watcher error: {0}")]
    Notify(#[from] notify::Error),
    /// A glob pattern in the ignore list was invalid.
    #[error("invalid ignore glob '{pattern}': {source}")]
    InvalidGlob {
        pattern: String,
        #[source]
        source: globset::Error,
    },
    /// The OS rejected the dispatcher thread spawn (thread limit, OOM, etc.).
    #[error("failed to spawn rift-fs-dispatcher thread: {0}")]
    DispatcherSpawnFailed(#[source] std::io::Error),
}

// ---------------------------------------------------------------------------
// FsWatcher — opaque handle
// ---------------------------------------------------------------------------

/// Opaque handle returned by [`spawn_fs_watcher`].
///
/// Dropping this handle stops the watcher cleanly: the inner
/// [`RecommendedWatcher`] is dropped, which closes the `notify` OS thread,
/// which closes the channel sender, which causes the dispatcher thread to
/// exit its `recv` loop.
pub struct FsWatcher {
    // Held alive so it watches for the duration. Drop closes the OS thread.
    // Field order matters: `watcher` is declared FIRST so it drops FIRST,
    // closing the channel sender, which lets the dispatcher exit cleanly.
    #[allow(dead_code)]
    watcher: RecommendedWatcher,
    // Held alive so the dispatcher thread exits when this is dropped.
    #[allow(dead_code)]
    dispatcher: thread::JoinHandle<()>,
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

/// Normalize a path string to forward-slash separators.
///
/// On Windows, `\\` path separators are replaced with `/`. On all platforms
/// the input string is returned unchanged if no backslashes are present.
pub(crate) fn normalize_path(raw: &str) -> String {
    raw.replace('\\', "/")
}

/// Strip `root` from `full_path`, normalize separators, and return the
/// resulting relative string.
///
/// Returns `None` when `full_path` is not under `root` (should not happen in
/// practice; caller logs and skips).
fn relative_path(root: &Path, full_path: &Path) -> Option<String> {
    full_path
        .strip_prefix(root)
        .ok()
        .map(|rel| normalize_path(&rel.to_string_lossy()))
}

// ---------------------------------------------------------------------------
// Glob helper
// ---------------------------------------------------------------------------

/// Returns `true` when the (already-normalized, relative) path should be
/// forwarded to the bus, `false` when a glob in `globs` matches it.
pub(crate) fn should_publish(rel_path: &str, globs: &GlobSet) -> bool {
    !globs.is_match(rel_path)
}

fn compile_globs(patterns: &[String]) -> Result<GlobSet, FsWatcherError> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let glob = Glob::new(pattern).map_err(|source| FsWatcherError::InvalidGlob {
            pattern: pattern.clone(),
            source,
        })?;
        builder.add(glob);
    }
    Ok(builder
        .build()
        .expect("GlobSetBuilder::build never fails after valid Glob::new"))
}

// ---------------------------------------------------------------------------
// publish_fs_event
// ---------------------------------------------------------------------------

/// Publish a `Category::Fs` envelope onto the bus.
///
/// Fire-and-forget: if the bus publish itself fails (bus closed, zero
/// capacity) the error is logged to stderr and the function returns normally.
/// Callers are already on a background thread and must not be disrupted by a
/// secondary bus failure.
///
/// # Arguments
///
/// * `bus`   — the shared [`RiftBus`] instance.
/// * `event` — the typed filesystem event to publish.
pub fn publish_fs_event(bus: &RiftBus, event: FsEvent) {
    let (kind, payload) = match event {
        FsEvent::Create { path } => ("create", json!({ "path": path })),
        FsEvent::Write { path } => ("write", json!({ "path": path })),
        FsEvent::Delete { path } => ("delete", json!({ "path": path })),
        FsEvent::Rename { from, to } => ("rename", json!({ "from": from, "to": to })),
    };
    let mut env = Envelope::new(Category::Fs, kind);
    env.payload = payload;
    bus.publish(env);
}

// ---------------------------------------------------------------------------
// spawn_fs_watcher
// ---------------------------------------------------------------------------

/// Spawn a filesystem watcher on `root`, publishing events to `bus`.
///
/// Events matching any pattern in `ignore_globs` (matched against the
/// relative path) are silently dropped. Errors received from `notify`
/// on the dispatcher channel are logged via `tracing::warn!` and the
/// dispatcher continues running.
///
/// Returns an opaque [`FsWatcher`] handle. Dropping the handle stops the
/// watcher cleanly.
///
/// # Errors
///
/// Returns [`FsWatcherError`] if the root cannot be canonicalized, if
/// any ignore glob is invalid, or if `notify` fails to initialize.
pub fn spawn_fs_watcher(
    bus: RiftBus,
    root: PathBuf,
    ignore_globs: Vec<String>,
) -> Result<FsWatcher, FsWatcherError> {
    // dunce::canonicalize avoids Windows `\\?\` UNC prefixes that can prevent
    // strip_prefix from matching `notify` callback paths (which on Windows
    // typically lack the prefix). On non-Windows this is a passthrough to
    // std::fs::canonicalize.
    let root = dunce::canonicalize(&root).map_err(|source| FsWatcherError::CanonicalizeFailed {
        path: root.clone(),
        source,
    })?;

    let globs = compile_globs(&ignore_globs)?;

    // Channel: notify callback → dispatcher thread (bounded to avoid OOM on
    // burst; 256 is generous for typical save operations).
    let (tx, rx) = mpsc::sync_channel::<Result<notify::Event, notify::Error>>(256);

    let mut watcher = RecommendedWatcher::new(
        move |res| {
            // IMPORTANT: do minimal work here — just forward over the channel.
            // The dispatcher thread does path normalization, glob matching,
            // and publishing.
            if tx.send(res).is_err() {
                // Dispatcher exited (FsWatcher dropped). Silently stop.
            }
        },
        notify::Config::default(),
    )?;

    watcher.watch(&root, RecursiveMode::Recursive)?;

    let dispatcher = thread::Builder::new()
        .name("rift-fs-dispatcher".into())
        .spawn(move || {
            // Loop until the sender half is dropped (FsWatcher dropped → watcher
            // dropped → tx closed → rx.recv() returns Err).
            while let Ok(result) = rx.recv() {
                match result {
                    Err(e) => {
                        tracing::warn!("rift-fs-dispatcher: notify error: {e}");
                    }
                    Ok(event) => {
                        dispatch_event(&bus, &root, &globs, event);
                    }
                }
            }
        })
        .map_err(FsWatcherError::DispatcherSpawnFailed)?;

    Ok(FsWatcher {
        watcher,
        dispatcher,
    })
}

/// Translate a single `notify::Event` into zero or more [`FsEvent`]s and
/// publish them. Called exclusively from the dispatcher thread.
fn dispatch_event(bus: &RiftBus, root: &Path, globs: &GlobSet, event: notify::Event) {
    use notify::event::{ModifyKind, RenameMode};

    match event.kind {
        EventKind::Create(_) => {
            for path in &event.paths {
                if let Some(rel) = relative_path(root, path) {
                    if should_publish(&rel, globs) {
                        publish_fs_event(bus, FsEvent::Create { path: rel });
                    }
                } else {
                    tracing::warn!(
                        "rift-fs-dispatcher: path outside root skipped: {}",
                        path.display()
                    );
                }
            }
        }
        // notify provides both paths in order [from, to] for a complete rename.
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)) if event.paths.len() >= 2 => {
            let from_path = &event.paths[0];
            let to_path = &event.paths[1];
            match (relative_path(root, from_path), relative_path(root, to_path)) {
                (Some(from), Some(to)) => {
                    // Apply glob check on both sides — if either is ignored,
                    // skip the rename envelope.
                    if should_publish(&from, globs) && should_publish(&to, globs) {
                        publish_fs_event(bus, FsEvent::Rename { from, to });
                    }
                }
                _ => {
                    tracing::warn!(
                        "rift-fs-dispatcher: rename path(s) outside root skipped: {:?}",
                        event.paths
                    );
                }
            }
        }
        EventKind::Modify(ModifyKind::Name(_)) => {
            // Partial rename (only one side known) — fall back to delete + create
            // per notify 6.x debouncer convention.
            for path in &event.paths {
                if let Some(rel) = relative_path(root, path) {
                    if should_publish(&rel, globs) {
                        // Without both sides we cannot know if this is the
                        // source or destination; emit delete for the known path
                        // so the frontend can decay it from the graph.
                        publish_fs_event(bus, FsEvent::Delete { path: rel });
                    }
                } else {
                    tracing::warn!(
                        "rift-fs-dispatcher: partial rename path outside root skipped: {}",
                        path.display()
                    );
                }
            }
        }
        EventKind::Modify(_) => {
            for path in &event.paths {
                if let Some(rel) = relative_path(root, path) {
                    if should_publish(&rel, globs) {
                        publish_fs_event(bus, FsEvent::Write { path: rel });
                    }
                } else {
                    tracing::warn!(
                        "rift-fs-dispatcher: modify path outside root skipped: {}",
                        path.display()
                    );
                }
            }
        }
        EventKind::Remove(_) => {
            for path in &event.paths {
                if let Some(rel) = relative_path(root, path) {
                    if should_publish(&rel, globs) {
                        publish_fs_event(bus, FsEvent::Delete { path: rel });
                    }
                } else {
                    tracing::warn!(
                        "rift-fs-dispatcher: remove path outside root skipped: {}",
                        path.display()
                    );
                }
            }
        }
        // Access, Other, Any — not published in Phase 6.1.
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RiftBus, SubscribeFilter};

    fn make_globset(patterns: &[&str]) -> GlobSet {
        let strs: Vec<String> = patterns.iter().map(|s| s.to_string()).collect();
        compile_globs(&strs).expect("test globs compile")
    }

    // T1 — publish_create_shape
    #[test]
    fn publish_create_shape() {
        let bus = RiftBus::default();
        publish_fs_event(
            &bus,
            FsEvent::Create {
                path: "src/lib.rs".into(),
            },
        );

        let (snapshot, _sub) = bus.subscribe(SubscribeFilter::All);
        assert_eq!(snapshot.len(), 1, "expected exactly one envelope");
        let env = &snapshot[0];
        assert_eq!(env.category, Category::Fs);
        assert_eq!(env.kind, "create");
        assert_eq!(env.payload["path"], "src/lib.rs");
        assert!(
            env.payload.get("from").is_none(),
            "from must be absent for create"
        );
    }

    // T2 — publish_write_shape
    #[test]
    fn publish_write_shape() {
        let bus = RiftBus::default();
        publish_fs_event(
            &bus,
            FsEvent::Write {
                path: "src/main.rs".into(),
            },
        );

        let (snapshot, _sub) = bus.subscribe(SubscribeFilter::All);
        assert_eq!(snapshot.len(), 1);
        let env = &snapshot[0];
        assert_eq!(env.category, Category::Fs);
        assert_eq!(env.kind, "write");
        assert_eq!(env.payload["path"], "src/main.rs");
    }

    // T3 — publish_delete_shape
    #[test]
    fn publish_delete_shape() {
        let bus = RiftBus::default();
        publish_fs_event(
            &bus,
            FsEvent::Delete {
                path: "old_file.rs".into(),
            },
        );

        let (snapshot, _sub) = bus.subscribe(SubscribeFilter::All);
        assert_eq!(snapshot.len(), 1);
        let env = &snapshot[0];
        assert_eq!(env.category, Category::Fs);
        assert_eq!(env.kind, "delete");
        assert_eq!(env.payload["path"], "old_file.rs");
    }

    // T4 — publish_rename_shape
    #[test]
    fn publish_rename_shape() {
        let bus = RiftBus::default();
        publish_fs_event(
            &bus,
            FsEvent::Rename {
                from: "old.rs".into(),
                to: "new.rs".into(),
            },
        );

        let (snapshot, _sub) = bus.subscribe(SubscribeFilter::All);
        assert_eq!(snapshot.len(), 1);
        let env = &snapshot[0];
        assert_eq!(env.category, Category::Fs);
        assert_eq!(env.kind, "rename");
        assert_eq!(env.payload["from"], "old.rs");
        assert_eq!(env.payload["to"], "new.rs");
        assert!(
            env.payload.get("path").is_none(),
            "path must be absent for rename"
        );
    }

    // T5 — version_not_bumped
    #[test]
    fn version_not_bumped() {
        let bus = RiftBus::default();
        publish_fs_event(
            &bus,
            FsEvent::Create {
                path: "any.rs".into(),
            },
        );
        let (snapshot, _) = bus.subscribe(SubscribeFilter::All);
        assert_eq!(snapshot[0].version, crate::CURRENT_VERSION);
    }

    // T6 — should_publish_filters_globs
    #[test]
    fn should_publish_filters_globs() {
        let globs = make_globset(&[".git/**", "target/**"]);
        assert!(
            !should_publish(".git/HEAD", &globs),
            ".git/HEAD should be filtered"
        );
        assert!(
            !should_publish("target/debug/foo", &globs),
            "target/debug/foo should be filtered"
        );
        assert!(
            should_publish("src/lib.rs", &globs),
            "src/lib.rs should pass through"
        );
    }

    // T7 — path_normalize_windows
    #[test]
    fn path_normalize_windows() {
        assert_eq!(normalize_path(r"src\lib.rs"), "src/lib.rs");
        assert_eq!(normalize_path("src/lib.rs"), "src/lib.rs");
        assert_eq!(normalize_path(r"a\b\c.txt"), "a/b/c.txt");
    }

    // T8 — relative_path strips root + normalizes to forward-slash form.
    // Defensive test against the Windows UNC-prefix bug surfaced in Phase 6.1
    // BV review (`windows-canonicalize-unc-prefix-vs-notify-callback-paths`):
    // dunce::canonicalize avoids the bug at root storage; this test asserts
    // strip_prefix + normalize_path produces the expected forward-slash output
    // regardless of native separator on the input.
    #[test]
    fn relative_path_strips_and_normalizes() {
        use std::path::Path;
        let root = Path::new("/proj/root");
        let full = Path::new("/proj/root/src/lib.rs");
        assert_eq!(
            relative_path(root, full).as_deref(),
            Some("src/lib.rs"),
            "unix-style strip_prefix"
        );

        // A path NOT under root returns None (caller logs + skips).
        let outside = Path::new("/elsewhere/file.rs");
        assert!(
            relative_path(root, outside).is_none(),
            "outside-root path returns None"
        );
    }
}
