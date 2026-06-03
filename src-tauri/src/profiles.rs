use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

/// In-memory cache of profiles loaded from disk. Avoids re-reading
/// `profiles.toml` on every list/load call.
#[derive(Clone, Default)]
pub struct ProfileStore {
    inner: Arc<Mutex<Vec<WorkspaceProfile>>>,
    loaded: Arc<Mutex<bool>>,
}

impl ProfileStore {
    fn ensure_loaded(&self) -> Result<(), String> {
        if *self.loaded.lock() {
            return Ok(());
        }
        let path = profiles_path()?;
        let parsed = if path.exists() {
            let raw = std::fs::read_to_string(&path)
                .map_err(|e| format!("profiles: failed to read {}: {e}", path.display()))?;
            let file: ProfilesFile =
                toml::from_str(&raw).map_err(|e| format!("profiles: parse error: {e}"))?;
            file.profiles
        } else {
            Vec::new()
        };
        *self.inner.lock() = parsed;
        *self.loaded.lock() = true;
        Ok(())
    }

    fn save(&self) -> Result<(), String> {
        let path = profiles_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("profiles: create dir: {e}"))?;
        }
        let file = ProfilesFile {
            profiles: self.inner.lock().clone(),
        };
        let raw = toml::to_string_pretty(&file).map_err(|e| format!("profiles: serialize: {e}"))?;
        std::fs::write(&path, raw)
            .map_err(|e| format!("profiles: write {}: {e}", path.display()))?;
        Ok(())
    }
}

fn profiles_path() -> Result<PathBuf, String> {
    let dirs = directories::ProjectDirs::from("com", "abyssal", "rift")
        .ok_or_else(|| "profiles: unable to resolve config directory".to_string())?;
    Ok(dirs.config_dir().join("profiles.toml"))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ProfilesFile {
    #[serde(default)]
    profiles: Vec<WorkspaceProfile>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkspaceProfile {
    pub name: String,
    #[serde(default)]
    pub project_filter: Option<String>,
    pub state: WorkspaceProfileState,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkspaceProfileState {
    pub tabs: Vec<ProfileTabState>,
    #[serde(default)]
    pub cockpit_visible: bool,
    #[serde(default)]
    pub cockpit_panels: Vec<String>,
    #[serde(default)]
    pub notification_filters: ProfileNotifFilters,
    /// Opaque JSON blob capturing a snapshot of the LLM routing ledger at save
    /// time (session cost, token counts, routing history, etc.).  The field is
    /// intentionally `Option<String>` so:
    ///   – old profiles serialised before this field existed parse without
    ///     error (`serde(default)` materialises `None`).
    ///   – a `null` / absent value never panics on load.
    ///   – the backend does NOT interpret or validate the contents; that is the
    ///     frontend's responsibility.
    /// Restore semantics are DESIGN-DEFERRED — loading a workspace should not
    /// automatically overwrite the live session ledger without explicit user
    /// opt-in.  The field is read back by the frontend `profile_load` response
    /// so it CAN act on it when the design is settled.
    #[serde(default)]
    pub analytics_snapshot: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProfileTabState {
    pub label: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub splits: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProfileNotifFilters {
    #[serde(default)]
    pub default_threshold: Option<String>,
    #[serde(default)]
    pub per_tab: std::collections::HashMap<String, String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProfileSummary {
    pub name: String,
    pub project_filter: Option<String>,
}

#[tauri::command]
pub async fn profile_list(
    store: tauri::State<'_, ProfileStore>,
) -> Result<Vec<ProfileSummary>, String> {
    let store = store.inner().clone();
    tokio::task::spawn_blocking(move || {
        store.ensure_loaded()?;
        let profiles = store.inner.lock();
        Ok(profiles
            .iter()
            .map(|p| ProfileSummary {
                name: p.name.clone(),
                project_filter: p.project_filter.clone(),
            })
            .collect())
    })
    .await
    .map_err(|e| format!("profile_list: join error: {e}"))?
}

#[tauri::command]
pub async fn profile_save(
    name: String,
    state: WorkspaceProfileState,
    project_filter: Option<String>,
    store: tauri::State<'_, ProfileStore>,
) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("profile_save: name cannot be empty".to_string());
    }
    const MAX_NAME_LEN: usize = 200;
    if name.len() > MAX_NAME_LEN {
        return Err(format!(
            "profile_save: name exceeds {MAX_NAME_LEN} characters"
        ));
    }
    let store = store.inner().clone();
    tokio::task::spawn_blocking(move || {
        store.ensure_loaded()?;
        let mut profiles = store.inner.lock();
        const MAX_PROFILES: usize = 100;
        if !profiles.iter().any(|p| p.name == name) && profiles.len() >= MAX_PROFILES {
            return Err(format!(
                "profile_save: maximum of {MAX_PROFILES} profiles reached"
            ));
        }
        if let Some(existing) = profiles.iter_mut().find(|p| p.name == name) {
            existing.state = state;
            existing.project_filter = project_filter;
        } else {
            profiles.push(WorkspaceProfile {
                name,
                project_filter,
                state,
            });
        }
        drop(profiles);
        store.save()
    })
    .await
    .map_err(|e| format!("profile_save: join error: {e}"))?
}

#[tauri::command]
pub async fn profile_load(
    name: String,
    store: tauri::State<'_, ProfileStore>,
) -> Result<WorkspaceProfileState, String> {
    let store = store.inner().clone();
    tokio::task::spawn_blocking(move || {
        store.ensure_loaded()?;
        let profiles = store.inner.lock();
        profiles
            .iter()
            .find(|p| p.name == name)
            .map(|p| p.state.clone())
            .ok_or_else(|| format!("profile_load: no profile named '{name}'"))
    })
    .await
    .map_err(|e| format!("profile_load: join error: {e}"))?
}

#[tauri::command]
pub async fn profile_delete(
    name: String,
    store: tauri::State<'_, ProfileStore>,
) -> Result<(), String> {
    let store = store.inner().clone();
    tokio::task::spawn_blocking(move || {
        store.ensure_loaded()?;
        let mut profiles = store.inner.lock();
        let before = profiles.len();
        profiles.retain(|p| p.name != name);
        if profiles.len() == before {
            return Err(format!("profile_delete: no profile named '{name}'"));
        }
        drop(profiles);
        store.save()
    })
    .await
    .map_err(|e| format!("profile_delete: join error: {e}"))?
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_state(analytics: Option<&str>) -> WorkspaceProfileState {
        WorkspaceProfileState {
            tabs: vec![ProfileTabState {
                label: "main".to_string(),
                cwd: Some("/tmp".to_string()),
                splits: vec![],
            }],
            cockpit_visible: true,
            cockpit_panels: vec!["tree".to_string()],
            notification_filters: ProfileNotifFilters {
                default_threshold: Some("info".to_string()),
                per_tab: std::collections::HashMap::new(),
            },
            analytics_snapshot: analytics.map(|s| s.to_string()),
        }
    }

    /// Round-trip: WorkspaceProfileState → TOML → WorkspaceProfileState preserves
    /// the analytics_snapshot field (present case).
    #[test]
    fn analytics_snapshot_round_trips_through_toml_present() {
        let blob = r#"{"sessionCostUsd":0.042,"requestCount":7}"#;
        let state = make_state(Some(blob));

        let profile = WorkspaceProfile {
            name: "test".to_string(),
            project_filter: None,
            state,
        };
        let file = ProfilesFile {
            profiles: vec![profile],
        };

        let serialised = toml::to_string_pretty(&file).expect("serialise ok");
        let restored: ProfilesFile = toml::from_str(&serialised).expect("deserialise ok");

        let restored_state = &restored.profiles[0].state;
        assert_eq!(
            restored_state.analytics_snapshot.as_deref(),
            Some(blob),
            "analytics_snapshot should survive a TOML round-trip"
        );
    }

    /// Round-trip: when analytics_snapshot is None the field is absent in TOML
    /// and parses back as None without error.
    #[test]
    fn analytics_snapshot_round_trips_through_toml_absent() {
        let state = make_state(None);
        let profile = WorkspaceProfile {
            name: "no-analytics".to_string(),
            project_filter: None,
            state,
        };
        let file = ProfilesFile {
            profiles: vec![profile],
        };

        let serialised = toml::to_string_pretty(&file).expect("serialise ok");
        let restored: ProfilesFile = toml::from_str(&serialised).expect("deserialise ok");

        assert_eq!(
            restored.profiles[0].state.analytics_snapshot, None,
            "absent analytics_snapshot should round-trip as None"
        );
    }

    /// Backward-compat: a TOML blob that pre-dates the analytics_snapshot field
    /// (i.e. it is absent from the serialised form) should parse without error
    /// and yield None for the field.
    #[test]
    fn analytics_snapshot_missing_from_old_toml_parses_as_none() {
        let old_toml = r#"
[[profiles]]
name = "legacy"

[profiles.state]
cockpit_visible = false
cockpit_panels = []

[profiles.state.notification_filters]
"#;
        let file: ProfilesFile = toml::from_str(old_toml).expect("old TOML should parse");
        assert_eq!(
            file.profiles[0].state.analytics_snapshot, None,
            "old TOML without analytics_snapshot should yield None"
        );
    }
}
