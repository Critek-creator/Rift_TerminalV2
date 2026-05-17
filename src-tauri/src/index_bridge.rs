use std::path::PathBuf;
use std::sync::OnceLock;

use index_core::{Index, TagOperator};
use serde::Serialize;

static INDEX_VAULT_PATH: OnceLock<PathBuf> = OnceLock::new();

fn vault_path() -> &'static PathBuf {
    INDEX_VAULT_PATH.get_or_init(|| {
        if let Ok(p) = std::env::var("INDEX_VAULT_PATH") {
            return PathBuf::from(p);
        }

        let candidates = [
            directories::BaseDirs::new().map(|b| {
                b.home_dir()
                    .join("Documents/Abyssal_Arts_main/Projects/abyssal-index/vault")
            }),
            std::env::current_dir()
                .ok()
                .map(|d| d.join("../abyssal-index/vault")),
        ];

        for candidate in candidates.into_iter().flatten() {
            if candidate.exists() {
                return candidate;
            }
        }

        PathBuf::from("vault")
    })
}

fn open_index() -> Result<Index, String> {
    Index::open(vault_path()).map_err(|e| format!("Index unavailable: {e}"))
}

#[derive(Serialize)]
pub struct IndexNode {
    pub id: String,
    pub title: String,
    pub domain: String,
    pub floor: String,
    pub tags: Vec<String>,
    pub summary: Option<String>,
    pub status: String,
    pub modified: String,
}

#[derive(Serialize)]
pub struct IndexNodeFull {
    pub id: String,
    pub title: String,
    pub domain: String,
    pub floor: String,
    pub tags: Vec<String>,
    pub summary: Option<String>,
    pub status: String,
    pub body: String,
    pub created: String,
    pub modified: String,
    pub links: Vec<String>,
}

#[derive(Serialize)]
pub struct IndexConnection {
    pub source: String,
    pub target: String,
}

#[derive(Serialize)]
pub struct IndexStats {
    pub total_nodes: usize,
    pub total_links: usize,
    pub unique_tags: usize,
    pub by_domain: Vec<(String, usize)>,
    pub by_floor: Vec<(String, usize)>,
}

#[tauri::command]
pub fn index_list_nodes(
    domain: Option<String>,
    floor: Option<String>,
    tags: Option<String>,
) -> Result<Vec<IndexNode>, String> {
    let index = open_index()?;

    let records = if let Some(tag_str) = tags {
        let tag_list: Vec<String> = tag_str.split(',').map(|t| t.trim().to_string()).collect();
        index
            .by_tags(&tag_list, TagOperator::Or, None)
            .map_err(|e| e.to_string())?
    } else {
        index
            .db()
            .query_nodes(domain.as_deref(), floor.as_deref(), None, None)
            .map_err(|e| e.to_string())?
    };

    Ok(records
        .into_iter()
        .map(|r| IndexNode {
            id: r.id.to_string(),
            title: r.title,
            domain: r.domain.dir_name().to_string(),
            floor: r.floor.to_string(),
            tags: r.tags,
            summary: r.summary,
            status: format!("{:?}", r.status).to_lowercase(),
            modified: r.modified.to_rfc3339(),
        })
        .collect())
}

#[tauri::command]
pub fn index_search_nodes(query: String, limit: Option<usize>) -> Result<Vec<IndexNode>, String> {
    let index = open_index()?;
    let results = index
        .search(&query, limit.unwrap_or(50))
        .map_err(|e| e.to_string())?;

    Ok(results
        .into_iter()
        .map(|r| IndexNode {
            id: r.record.id.to_string(),
            title: r.record.title,
            domain: r.record.domain.dir_name().to_string(),
            floor: r.record.floor.to_string(),
            tags: r.record.tags,
            summary: r.record.summary,
            status: format!("{:?}", r.record.status).to_lowercase(),
            modified: r.record.modified.to_rfc3339(),
        })
        .collect())
}

#[tauri::command]
pub fn index_get_node(id: String) -> Result<IndexNodeFull, String> {
    let index = open_index()?;
    let uuid = uuid::Uuid::parse_str(&id).map_err(|e| format!("Invalid UUID: {e}"))?;
    let node = index.read(uuid).map_err(|e| e.to_string())?;

    let links: Vec<String> = index_core::wikilink::extract_wikilinks(&node.body)
        .into_iter()
        .map(|l| l.target)
        .collect();

    Ok(IndexNodeFull {
        id: node.metadata.id.to_string(),
        title: node.metadata.title,
        domain: node.metadata.domain.dir_name().to_string(),
        floor: node.metadata.floor.to_string(),
        tags: node.metadata.tags,
        summary: node.metadata.summary,
        status: format!("{:?}", node.metadata.status).to_lowercase(),
        body: node.body,
        created: node.metadata.created.to_rfc3339(),
        modified: node.metadata.modified.to_rfc3339(),
        links,
    })
}

#[tauri::command]
pub fn index_get_connections(id: String, depth: Option<u32>) -> Result<Vec<IndexNode>, String> {
    let index = open_index()?;
    let uuid = uuid::Uuid::parse_str(&id).map_err(|e| format!("Invalid UUID: {e}"))?;
    let records = index
        .connections(uuid, depth.unwrap_or(1))
        .map_err(|e| e.to_string())?;

    Ok(records
        .into_iter()
        .map(|r| IndexNode {
            id: r.id.to_string(),
            title: r.title,
            domain: r.domain.dir_name().to_string(),
            floor: r.floor.to_string(),
            tags: r.tags,
            summary: r.summary,
            status: format!("{:?}", r.status).to_lowercase(),
            modified: r.modified.to_rfc3339(),
        })
        .collect())
}

#[tauri::command]
pub fn index_get_stats() -> Result<IndexStats, String> {
    let index = open_index()?;
    let stats = index.stats().map_err(|e| e.to_string())?;

    Ok(IndexStats {
        total_nodes: stats.total_nodes,
        total_links: stats.total_links,
        unique_tags: stats.unique_tags,
        by_domain: stats.by_domain.into_iter().collect(),
        by_floor: stats.by_floor.into_iter().collect(),
    })
}
