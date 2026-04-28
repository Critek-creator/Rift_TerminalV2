//! Vault-walker translator — publishes `Category::Index` envelopes by walking
//! and watching the Abyssal Index vault directory.
//!
//! ## Phase 8.5
//!
//! This module is the vault-walker source deferred from Phase 8.1. It:
//!   1. Performs a boot walk of `<vault_root>/vaults/*.md`, extracting YAML
//!      frontmatter and emitting a `vault.update / Created` envelope per vault.
//!   2. Emits a `walk.complete` envelope after the boot walk finishes so the
//!      frontend can distinguish "still loading" from "no vaults found".
//!   3. Sets up a `notify::RecommendedWatcher` (recursive) on `<vault_root>`.
//!   4. Implements 100ms manual debounce — **no `notify_debouncer_*` crate** per
//!      pr003 `fs-rs-debounce-policy`.
//!
//! ## §9 boundary
//!
//! All `notify`, `std::fs`, and `serde_yaml` calls live EXCLUSIVELY in this
//! file. No cross-module leakage. The boundary checker enforces this at CI.
//!
//! ## Kind taxonomy under `Category::Index`
//!
//! | kind            | trigger                                     |
//! |-----------------|---------------------------------------------|
//! | `"vault.update"`| vault file created, modified, or deleted    |
//! | `"walk.complete"`| boot walk finished (signals load state)    |
//!
//! `"vault.update"` was already defined by the Phase 8.1 index translator
//! publish API. `"walk.complete"` is additive and does NOT bump `CURRENT_VERSION`
//! (per `envelope-version-additive-categories-no-bump`).
//!
//! ## Debounce strategy
//!
//! Manual 100ms debounce using a shared `HashMap<PathBuf, (Instant, DebouncedKind)>`
//! under a `Mutex`. The notify callback writes into the map; a separate tokio
//! task ticks every 50ms and flushes entries older than 100ms. This avoids
//! adding `notify_debouncer_full` or `notify_debouncer_mini` as dependencies.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::json;
use tokio::time::interval;

use crate::translators::index::VaultChangeKind;
use crate::{Category, Envelope, RiftBus};

// ---------------------------------------------------------------------------
// Frontmatter parsing
// ---------------------------------------------------------------------------

/// Parsed fields extracted from a vault file's YAML frontmatter.
#[derive(Debug, Clone)]
pub struct VaultMeta {
    /// Vault identifier, e.g. `"p006"` or `"pr001"`.
    pub id: String,
    /// Human-readable vault name.
    pub name: String,
    /// Abyssal Index type prefix, e.g. `"project"`, `"practices"`, `"research"`.
    pub vault_type: String,
    /// Last-updated date string (YYYY-MM-DD).
    pub updated: String,
    /// Optional repo path (absolute, Windows or Unix).
    pub repo: Option<String>,
    /// Optional source subdirectory within repo.
    pub source: Option<String>,
    /// Cross-reference vault ids.
    pub cross_refs: Vec<String>,
}

/// Parse YAML frontmatter from a vault file's text content.
///
/// Frontmatter must be delimited by `---` lines at the very start of the file.
/// Returns `None` when the file is missing frontmatter delimiters or when
/// required fields (`id`, `name`, `type`) are absent. Logs a `tracing::warn!`
/// on any parse or field error; never panics.
pub fn parse_frontmatter(content: &str) -> Option<VaultMeta> {
    // Fast path — must start with `---`
    if !content.starts_with("---") {
        return None;
    }

    // Find the closing `---` delimiter (the second occurrence after the first line).
    let after_open = content.get(3..)?;
    // Skip the optional newline immediately after `---`
    let rest = after_open.trim_start_matches(['\r', '\n']);
    let close_pos = rest.find("\n---")?;
    let yaml_body = &rest[..close_pos];

    // Parse the YAML block.
    let value: serde_yaml::Value = match serde_yaml::from_str(yaml_body) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("vault_walker: serde_yaml parse error: {e}");
            return None;
        }
    };

    let map = value.as_mapping()?;

    let get_str = |key: &str| -> Option<String> {
        map.get(key).and_then(|v| v.as_str()).map(|s| s.to_owned())
    };

    let id = get_str("id")?;
    let name = get_str("name")?;
    let vault_type = get_str("type")?;
    let updated = get_str("updated").unwrap_or_default();
    let repo = get_str("repo");
    let source = get_str("source");

    // cross_refs can be:
    //   - a YAML sequence:  [pr001, pr003]
    //   - a quoted string:  "[pr001, pr003]"   (rare, but guard it)
    //   - absent / null
    let cross_refs: Vec<String> = map
        .get("cross_refs")
        .map(|v| {
            if let Some(seq) = v.as_sequence() {
                seq.iter()
                    .filter_map(|item| item.as_str().map(|s| s.to_owned()))
                    .collect()
            } else if let Some(s) = v.as_str() {
                // Inline list as string "[a, b, c]" — parse manually.
                s.trim_matches(['[', ']', ' '])
                    .split(',')
                    .map(|p| p.trim().to_owned())
                    .filter(|s| !s.is_empty())
                    .collect()
            } else {
                Vec::new()
            }
        })
        .unwrap_or_default();

    Some(VaultMeta {
        id,
        name,
        vault_type,
        updated,
        repo,
        source,
        cross_refs,
    })
}

// ---------------------------------------------------------------------------
// walk.complete envelope helper
// ---------------------------------------------------------------------------

/// Publish a `Category::Index / kind="walk.complete"` envelope onto the bus.
///
/// Signals to the frontend that the boot walk has finished — the graph can now
/// distinguish "still loading" from "no vaults found / abyssal-index absent".
pub fn publish_walk_complete(bus: &RiftBus) {
    let mut env = Envelope::new(Category::Index, "walk.complete");
    env.payload = json!({ "source": "vault_walker" });
    bus.publish(env);
}

/// Publish a `Category::Index / kind="vault.update"` envelope with enriched
/// payload including `name` and `cross_refs` so the frontend graph can build
/// labels and edges without a separate Phase 8.6 enrichment pass.
///
/// Extends the Phase 8.1 `publish_vault_update` shape (additive fields;
/// per `envelope-version-additive-categories-no-bump` this does NOT bump
/// `CURRENT_VERSION`).
fn publish_vault_update_rich(
    bus: &RiftBus,
    meta: &VaultMeta,
    path: &std::path::Path,
    change_kind: VaultChangeKind,
) {
    let path_str = path.to_string_lossy().replace('\\', "/");
    let mut env = Envelope::new(Category::Index, "vault.update");
    env.payload = json!({
        "vault_id":    meta.id,
        "path":        path_str,
        "change_kind": change_kind,
        "name":        meta.name,
        "cross_refs":  meta.cross_refs,
    });
    bus.publish(env);
}

/// Publish a `Category::Index / kind="vault.update"` envelope with minimal
/// payload (fallback when frontmatter is absent or malformed).
fn publish_vault_update_minimal(
    bus: &RiftBus,
    vault_id: &str,
    path: &std::path::Path,
    change_kind: VaultChangeKind,
) {
    let path_str = path.to_string_lossy().replace('\\', "/");
    let mut env = Envelope::new(Category::Index, "vault.update");
    env.payload = json!({
        "vault_id":    vault_id,
        "path":        path_str,
        "change_kind": change_kind,
    });
    bus.publish(env);
}

// ---------------------------------------------------------------------------
// Debounce map entry
// ---------------------------------------------------------------------------

/// The kind of pending debounced event for a path.
#[derive(Clone, Copy, Debug)]
enum DebouncedKind {
    /// File was created or appeared.
    Created,
    /// File was modified.
    Modified,
    /// File was deleted or removed.
    Deleted,
}

impl From<DebouncedKind> for VaultChangeKind {
    fn from(k: DebouncedKind) -> Self {
        match k {
            DebouncedKind::Created => VaultChangeKind::Created,
            DebouncedKind::Modified => VaultChangeKind::Modified,
            DebouncedKind::Deleted => VaultChangeKind::Deleted,
        }
    }
}

/// Entry in the debounce map: the most recent event kind for a path, plus the
/// timestamp of the FIRST event in the current debounce window.
#[derive(Clone, Debug)]
struct DebounceEntry {
    kind: DebouncedKind,
    /// When the first event in this window arrived. The flush task uses this to
    /// decide when to emit (first + 100ms ≤ now).
    first_seen: Instant,
}

type DebounceMap = Arc<Mutex<HashMap<PathBuf, DebounceEntry>>>;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Returns `true` if the path is a `.md` file (case-insensitive extension
/// check) and is NOT `.autoindex-state.json` or any other non-md file.
fn is_md_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("md"))
        .unwrap_or(false)
}

/// Read a vault file and extract its frontmatter.
///
/// Returns `None` on any I/O error or if frontmatter is absent/malformed.
/// Logs warnings but does NOT panic.
fn read_vault_meta(path: &Path) -> Option<VaultMeta> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("vault_walker: could not read '{}': {e}", path.display());
            return None;
        }
    };
    parse_frontmatter(&content)
}

/// Forward-slash normalize a path string (Windows backslash → forward slash).
fn normalize_path_str(raw: &str) -> String {
    raw.replace('\\', "/")
}

// ---------------------------------------------------------------------------
// spawn_vault_walker — public entry point
// ---------------------------------------------------------------------------

/// Spawn the vault-walker on a separate tokio task.
///
/// Performs a boot walk of `<vault_root>/vaults/*.md` immediately, then sets
/// up a `notify` watcher for live changes. Events are debounced at 100ms via a
/// manual flush loop (pr003 `fs-rs-debounce-policy`).
///
/// Mirrors [`crate::translators::fs::spawn_fs_watcher`]'s Phase 7.1 setup()
/// pattern: `tauri::async_runtime::spawn(async move { ... })`.
///
/// Returns a [`JoinHandle`] for the outer async task. The outer task exits
/// once the notify watcher is set up and the boot walk is done; the live
/// watcher runs for the process lifetime via the inner dispatcher thread.
///
/// # Missing vault root
///
/// If `vault_root` does not exist on disk the walker logs a warning and
/// returns without spawning a watcher or crashing the app. The frontend
/// receives a `walk.complete` envelope so it can show the empty-state.
///
/// # Calling convention (Phase 7.1 setup() pattern)
///
/// This is an `async fn` — callers wrap it in `tauri::async_runtime::spawn`:
/// ```ignore
/// tauri::async_runtime::spawn(async move {
///     spawn_vault_walker(bus, vault_root).await;
/// });
/// ```
pub async fn spawn_vault_walker(bus: RiftBus, vault_root: PathBuf) {
    run_vault_walker(bus, vault_root).await;
}

async fn run_vault_walker(bus: RiftBus, vault_root: PathBuf) {
    // --- Guard: vault root must exist ---
    if !vault_root.exists() {
        tracing::warn!(
            "vault_walker: '{}' does not exist — walker skipped",
            vault_root.display()
        );
        publish_walk_complete(&bus);
        return;
    }

    // --- Boot walk ---
    let vaults_dir = vault_root.join("vaults");
    if vaults_dir.is_dir() {
        match std::fs::read_dir(&vaults_dir) {
            Ok(entries) => {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if !is_md_file(&path) {
                        continue;
                    }
                    // Re-use the same read_vault_meta + publish pattern as the
                    // live watcher flush.
                    if let Some(meta) = read_vault_meta(&path) {
                        publish_vault_update_rich(&bus, &meta, &path, VaultChangeKind::Created);
                    } else {
                        // Malformed frontmatter — still emit with path-derived id
                        // so the graph can show the node at minimum.
                        let fallback_id = path
                            .file_stem()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "unknown".to_owned());
                        tracing::warn!(
                            "vault_walker: malformed frontmatter in '{}'; using fallback id '{}'",
                            path.display(),
                            fallback_id,
                        );
                        publish_vault_update_minimal(
                            &bus,
                            &fallback_id,
                            &path,
                            VaultChangeKind::Created,
                        );
                    }
                }
            }
            Err(e) => {
                tracing::warn!("vault_walker: failed to read vaults/ dir: {e}");
            }
        }
    } else {
        tracing::warn!(
            "vault_walker: vaults/ subdir not found under '{}'",
            vault_root.display()
        );
    }

    // Emit walk.complete — frontend can now distinguish "loading" from "empty".
    publish_walk_complete(&bus);

    // --- Set up notify watcher + debounce map ---
    let debounce_map: DebounceMap = Arc::new(Mutex::new(HashMap::new()));
    let debounce_map_for_flush = Arc::clone(&debounce_map);

    // Bounded channel: notify callback → dispatcher.  256 matches fs.rs.
    let (tx, rx) = mpsc::sync_channel::<Result<notify::Event, notify::Error>>(256);

    let mut watcher = match RecommendedWatcher::new(
        move |res| {
            if tx.send(res).is_err() {
                // Watcher dropped — silently stop.
            }
        },
        notify::Config::default(),
    ) {
        Ok(w) => w,
        Err(e) => {
            tracing::warn!("vault_walker: failed to create notify watcher: {e}");
            return;
        }
    };

    if let Err(e) = watcher.watch(&vault_root, RecursiveMode::Recursive) {
        tracing::warn!(
            "vault_walker: failed to watch '{}': {e}",
            vault_root.display()
        );
        return;
    }

    // --- Notify dispatcher thread ---
    // This mirrors the fs.rs dispatcher: a std::thread that recvs from the
    // channel and writes into the debounce map.
    let debounce_map_for_dispatcher = Arc::clone(&debounce_map);
    let _dispatcher = std::thread::Builder::new()
        .name("rift-vault-dispatcher".into())
        .spawn(move || {
            while let Ok(result) = rx.recv() {
                match result {
                    Err(e) => {
                        tracing::warn!("rift-vault-dispatcher: notify error: {e}");
                    }
                    Ok(event) => {
                        handle_notify_event(&debounce_map_for_dispatcher, event);
                    }
                }
            }
            // rx closed → watcher dropped → exit.
        });

    // --- Flush task (50ms tick; emits events older than 100ms) ---
    // Runs as a separate tokio task alongside the dispatcher.
    // The JoinHandle is stored and then dropped via std::mem::drop to avoid the
    // clippy::let_underscore_future lint while still making the fire-and-forget
    // intent clear.
    let bus_for_flush = bus.clone();
    let flush_handle = tokio::spawn(async move {
        let mut ticker = interval(Duration::from_millis(50));
        loop {
            ticker.tick().await;
            flush_debounce(&debounce_map_for_flush, &bus_for_flush);
        }
    });
    // Detach: the flush task runs for the process lifetime alongside the
    // notify dispatcher. We intentionally do not await it — drop the handle.
    drop(flush_handle);

    // Hold the watcher alive for the process lifetime.
    // The async task parks here indefinitely; the watcher is dropped when the
    // process exits.
    std::future::pending::<()>().await;

    // Ensure the watcher is kept alive until the pending resolves (never in
    // practice, but prevents the compiler from dropping it early).
    drop(watcher);
}

/// Record a notify event into the debounce map.
///
/// Strategy:
/// - Create/Modify → record with the corresponding kind.
/// - Delete/Remove → record as Deleted.
/// - Rename: emit both Deleted (from) and Created (to) immediately by
///   inserting both into the debounce map with a `first_seen` that ensures
///   they flush on the next tick (set to `Instant::now() - 200ms`).
/// - Only `.md` files pass through; `.autoindex-state.json` and others are
///   dropped silently.
fn handle_notify_event(debounce_map: &DebounceMap, event: notify::Event) {
    use notify::event::{ModifyKind, RenameMode};

    let now = Instant::now();
    let already_due = now.checked_sub(Duration::from_millis(200)).unwrap_or(now);

    match event.kind {
        EventKind::Create(_) => {
            for path in event.paths {
                if is_md_file(&path) {
                    upsert_debounce(debounce_map, path, DebouncedKind::Created, now);
                }
            }
        }
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)) if event.paths.len() >= 2 => {
            let from = &event.paths[0];
            let to = &event.paths[1];
            if is_md_file(from) {
                upsert_debounce(
                    debounce_map,
                    from.clone(),
                    DebouncedKind::Deleted,
                    already_due,
                );
            }
            if is_md_file(to) {
                upsert_debounce(
                    debounce_map,
                    to.clone(),
                    DebouncedKind::Created,
                    already_due,
                );
            }
        }
        EventKind::Modify(ModifyKind::Name(_)) => {
            // Partial rename — treat as delete (same as fs.rs).
            for path in event.paths {
                if is_md_file(&path) {
                    upsert_debounce(debounce_map, path, DebouncedKind::Deleted, already_due);
                }
            }
        }
        EventKind::Modify(_) => {
            for path in event.paths {
                if is_md_file(&path) {
                    upsert_debounce(debounce_map, path, DebouncedKind::Modified, now);
                }
            }
        }
        EventKind::Remove(_) => {
            for path in event.paths {
                if is_md_file(&path) {
                    upsert_debounce(debounce_map, path, DebouncedKind::Deleted, now);
                }
            }
        }
        _ => {}
    }
}

/// Insert or update an entry in the debounce map.
///
/// If an entry already exists for `path`:
/// - `first_seen` is preserved (window starts at the first event).
/// - `kind` is updated to the latest event (last-write-wins within the window).
///
/// If the entry is new, `first_seen` is set to `first_seen_ts`.
fn upsert_debounce(
    debounce_map: &DebounceMap,
    path: PathBuf,
    kind: DebouncedKind,
    first_seen_ts: Instant,
) {
    let mut guard = debounce_map.lock().expect("debounce map poisoned");
    guard
        .entry(path)
        .and_modify(|e| e.kind = kind)
        .or_insert(DebounceEntry {
            kind,
            first_seen: first_seen_ts,
        });
}

/// Flush all debounce entries older than 100ms and publish their envelopes.
///
/// Called every 50ms from the flush tokio task. Entries that are not yet due
/// are left in the map for the next tick.
fn flush_debounce(debounce_map: &DebounceMap, bus: &RiftBus) {
    let now = Instant::now();
    let debounce_window = Duration::from_millis(100);

    let mut guard = debounce_map.lock().expect("debounce map poisoned");

    let due: Vec<(PathBuf, DebounceEntry)> = guard
        .iter()
        .filter(|(_, entry)| now.duration_since(entry.first_seen) >= debounce_window)
        .map(|(p, e)| (p.clone(), e.clone()))
        .collect();

    for (path, entry) in due {
        guard.remove(&path);
        drop(guard); // Release lock before doing I/O.

        let change_kind: VaultChangeKind = entry.kind.into();

        // For Deleted envelopes we don't try to re-read the file — it's gone.
        // For Created/Modified: re-read frontmatter to get the rich payload.
        // On read failure fall back to file-stem id with minimal payload.
        match change_kind {
            VaultChangeKind::Deleted => {
                let fallback_id = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| normalize_path_str(&path.to_string_lossy()));
                publish_vault_update_minimal(bus, &fallback_id, &path, VaultChangeKind::Deleted);
            }
            _ => match read_vault_meta(&path) {
                Some(meta) => {
                    publish_vault_update_rich(bus, &meta, &path, change_kind);
                }
                None => {
                    let fallback_id = path
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "unknown".to_owned());
                    publish_vault_update_minimal(bus, &fallback_id, &path, change_kind);
                }
            },
        }

        // Re-acquire lock for the next iteration.
        guard = debounce_map.lock().expect("debounce map poisoned");
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Category, RiftBus, SubscribeFilter};

    // -------------------------------------------------------------------------
    // T1 — frontmatter_parse_valid
    //
    // Given a synthetic vault file with full valid YAML frontmatter, all fields
    // are extracted correctly.
    // -------------------------------------------------------------------------
    #[test]
    fn frontmatter_parse_valid() {
        let content = r#"---
id: p006
name: rift-terminal
type: project
updated: 2026-04-27
repo: C:\Users\Critek\Projects\Rift
source: src/
cross_refs: [pr001, pr003, r004]
---
Body text here.
"#;
        let meta = parse_frontmatter(content).expect("should parse valid frontmatter");
        assert_eq!(meta.id, "p006");
        assert_eq!(meta.name, "rift-terminal");
        assert_eq!(meta.vault_type, "project");
        assert_eq!(meta.updated, "2026-04-27");
        assert_eq!(meta.repo.as_deref(), Some(r"C:\Users\Critek\Projects\Rift"));
        assert_eq!(meta.source.as_deref(), Some("src/"));
        assert_eq!(meta.cross_refs, vec!["pr001", "pr003", "r004"]);
    }

    // -------------------------------------------------------------------------
    // T2 — frontmatter_parse_malformed
    //
    // Missing `---` delimiters, missing required fields, and invalid YAML all
    // return None gracefully — no panic.
    // -------------------------------------------------------------------------
    #[test]
    fn frontmatter_parse_malformed() {
        // Case A: no frontmatter at all.
        let result = parse_frontmatter("Just some body text\nwith no frontmatter.");
        assert!(result.is_none(), "plain text must return None");

        // Case B: only the opening `---`, no closing delimiter.
        let result = parse_frontmatter("---\nid: p006\nname: foo\n");
        assert!(result.is_none(), "unclosed frontmatter must return None");

        // Case C: valid delimiters but missing required field `name`.
        let result = parse_frontmatter("---\nid: p006\ntype: project\n\n---\nbody\n");
        assert!(
            result.is_none(),
            "missing required `name` field must return None"
        );

        // Case D: invalid YAML inside delimiters.
        let result = parse_frontmatter("---\n: : :\n\n---\nbody\n");
        assert!(result.is_none(), "invalid YAML must return None");
    }

    // -------------------------------------------------------------------------
    // T3 — boot_walk_emits_envelopes
    //
    // Given a tempdir with 3 vault files, the walker emits 3 vault.update
    // envelopes (kind="vault.update", category=Index) + 1 walk.complete
    // (kind="walk.complete", category=Index).
    //
    // Uses tokio::time::sleep to let the async walker complete.
    // -------------------------------------------------------------------------
    #[tokio::test]
    async fn boot_walk_emits_envelopes() {
        use tempfile::tempdir;

        let dir = tempdir().expect("tempdir");
        let vault_root = dir.path().to_path_buf();
        let vaults_dir = vault_root.join("vaults");
        std::fs::create_dir_all(&vaults_dir).unwrap();

        // Write 3 vault files with valid frontmatter.
        let vault_template = |id: &str, name: &str| {
            format!("---\nid: {id}\nname: {name}\ntype: project\nupdated: 2026-04-27\n---\nBody.\n")
        };

        std::fs::write(
            vaults_dir.join("p001.md"),
            vault_template("p001", "Project One"),
        )
        .unwrap();
        std::fs::write(
            vaults_dir.join("pr001.md"),
            vault_template("pr001", "Global Practices"),
        )
        .unwrap();
        std::fs::write(
            vaults_dir.join("r004.md"),
            vault_template("r004", "Tauri Research"),
        )
        .unwrap();

        let bus = RiftBus::default();
        // Subscribe BEFORE spawning so we capture the replay.
        let (snapshot_before, mut sub) = bus.subscribe(SubscribeFilter::Category(Category::Index));
        assert!(
            snapshot_before.is_empty(),
            "no envelopes before walker spawns"
        );

        // spawn_vault_walker is async — drive it on a separate tokio task.
        tokio::spawn(spawn_vault_walker(bus.clone(), vault_root));

        // Allow the async boot walk + walk.complete to complete.
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Drain live envelopes via timeout-recv loop (Subscription has no
        // try_recv — only async recv; use a short timeout to drain the queue).
        let mut envelopes = snapshot_before;
        while let Ok(Ok(env)) = tokio::time::timeout(Duration::from_millis(50), sub.recv()).await {
            envelopes.push(env);
        }

        // Expect 3 vault.update + 1 walk.complete = 4 total.
        let vault_updates: Vec<_> = envelopes
            .iter()
            .filter(|e| e.kind == "vault.update")
            .collect();
        let walk_completes: Vec<_> = envelopes
            .iter()
            .filter(|e| e.kind == "walk.complete")
            .collect();

        assert_eq!(
            vault_updates.len(),
            3,
            "expected 3 vault.update envelopes; got {}. All envelopes: {:?}",
            vault_updates.len(),
            envelopes.iter().map(|e| &e.kind).collect::<Vec<_>>()
        );
        assert_eq!(
            walk_completes.len(),
            1,
            "expected 1 walk.complete envelope; got {}",
            walk_completes.len()
        );

        // All envelopes must be Category::Index.
        for env in &envelopes {
            assert_eq!(env.category, Category::Index, "all envelopes must be Index");
        }
    }

    // -------------------------------------------------------------------------
    // T4 — debounce_collapses_burst
    //
    // Insert 5 modify events for the same path within a very short window;
    // after 150ms the debounce map should have flushed exactly ONE vault.update.
    //
    // We test the debounce map + flush_debounce directly (no tokio::spawn).
    // -------------------------------------------------------------------------
    #[tokio::test]
    async fn debounce_collapses_burst() {
        use tempfile::tempdir;

        let dir = tempdir().expect("tempdir");
        let vault_path = dir.path().join("p006.md");
        std::fs::write(
            &vault_path,
            "---\nid: p006\nname: Rift\ntype: project\n\n---\nBody.\n",
        )
        .unwrap();

        let debounce_map: DebounceMap = Arc::new(Mutex::new(HashMap::new()));
        let bus = RiftBus::default();

        // Simulate 5 rapid modify events within <5ms window.
        for _ in 0..5 {
            upsert_debounce(
                &debounce_map,
                vault_path.clone(),
                DebouncedKind::Modified,
                Instant::now(),
            );
        }

        // After 5 inserts the map should have exactly 1 entry (same path → collapsed).
        {
            let guard = debounce_map.lock().unwrap();
            assert_eq!(
                guard.len(),
                1,
                "burst of 5 events on same path should collapse to 1 map entry"
            );
        }

        // Wait for the debounce window to expire (>100ms).
        tokio::time::sleep(Duration::from_millis(150)).await;

        // Subscribe BEFORE flush so we capture what flush emits.
        let (snapshot, _sub) = bus.subscribe(SubscribeFilter::Category(Category::Index));
        assert!(snapshot.is_empty(), "nothing published yet before flush");

        // Manual flush — simulates what the 50ms ticker does.
        flush_debounce(&debounce_map, &bus);

        // Map should be empty after flush.
        {
            let guard = debounce_map.lock().unwrap();
            assert!(guard.is_empty(), "debounce map must be empty after flush");
        }

        // Exactly 1 vault.update should have been published.
        let (all, _) = bus.subscribe(SubscribeFilter::Category(Category::Index));
        let updates: Vec<_> = all.iter().filter(|e| e.kind == "vault.update").collect();
        assert_eq!(
            updates.len(),
            1,
            "burst of 5 events should yield exactly 1 vault.update after debounce; got {}",
            updates.len()
        );
        assert_eq!(
            updates[0].payload["change_kind"], "modified",
            "change_kind must be 'modified'"
        );
    }
}
