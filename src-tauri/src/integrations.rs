use std::path::PathBuf;
use std::process::Command;

use serde::Serialize;

fn claude_dir() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|b| b.home_dir().join(".claude"))
}

#[derive(Serialize)]
pub struct IntegrationDetail {
    pub installed: bool,
    pub enabled: bool,
    pub path: String,
}

#[derive(Serialize)]
pub struct IntegrationStatus {
    pub claude_dir_exists: bool,
    pub node_available: bool,
    pub node_version: Option<String>,
    pub aegis: IntegrationDetail,
    pub index: IntegrationDetail,
}

#[tauri::command]
pub fn integration_detect(cached: tauri::State<'_, crate::CachedConfig>) -> IntegrationStatus {
    let claude = claude_dir();
    let claude_exists = claude.as_ref().is_some_and(|p| p.exists());

    let aegis_path = claude
        .as_ref()
        .map(|d| d.join("skills/aegis/SKILL.md"))
        .unwrap_or_default();
    let aegis_installed = aegis_path.exists();

    let index_path = claude
        .as_ref()
        .map(|d| d.join("abyssal-index/MAIN_INDEX.md"))
        .unwrap_or_default();
    let index_installed = index_path.exists();

    let cfg = cached.get();

    let (node_available, node_version) = check_node();

    IntegrationStatus {
        claude_dir_exists: claude_exists,
        node_available,
        node_version,
        aegis: IntegrationDetail {
            installed: aegis_installed,
            enabled: cfg.integrations.aegis_enabled,
            path: aegis_path.to_string_lossy().into_owned(),
        },
        index: IntegrationDetail {
            installed: index_installed,
            enabled: cfg.integrations.index_enabled,
            path: index_path.to_string_lossy().into_owned(),
        },
    }
}

fn check_node() -> (bool, Option<String>) {
    match Command::new("node").arg("--version").output() {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            (true, Some(version))
        }
        _ => (false, None),
    }
}
