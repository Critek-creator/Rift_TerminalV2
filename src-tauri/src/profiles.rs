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
pub fn profile_list(store: tauri::State<'_, ProfileStore>) -> Result<Vec<ProfileSummary>, String> {
    store.ensure_loaded()?;
    let profiles = store.inner.lock();
    Ok(profiles
        .iter()
        .map(|p| ProfileSummary {
            name: p.name.clone(),
            project_filter: p.project_filter.clone(),
        })
        .collect())
}

#[tauri::command]
pub fn profile_save(
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
    store.ensure_loaded()?;
    let mut profiles = store.inner.lock();
    const MAX_PROFILES: usize = 100;
    // Only enforce the cap when creating a new profile, not when updating.
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
}

#[tauri::command]
pub fn profile_load(
    name: String,
    store: tauri::State<'_, ProfileStore>,
) -> Result<WorkspaceProfileState, String> {
    store.ensure_loaded()?;
    let profiles = store.inner.lock();
    profiles
        .iter()
        .find(|p| p.name == name)
        .map(|p| p.state.clone())
        .ok_or_else(|| format!("profile_load: no profile named '{name}'"))
}

#[tauri::command]
pub fn profile_delete(name: String, store: tauri::State<'_, ProfileStore>) -> Result<(), String> {
    store.ensure_loaded()?;
    let mut profiles = store.inner.lock();
    let before = profiles.len();
    profiles.retain(|p| p.name != name);
    if profiles.len() == before {
        return Err(format!("profile_delete: no profile named '{name}'"));
    }
    drop(profiles);
    store.save()
}
