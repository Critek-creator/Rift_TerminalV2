//! Routing profiles — built-in model selection strategies.
//!
//! Phase 1: Manual (user picks) + Balanced/CostOptimized/QualityFirst
//! as basic heuristics. Phase 2: configurable rule chains.

use rift_bus::config::{ModelConfig, RoutingProfile};
use serde::{Deserialize, Serialize};

use crate::classifier::TaskType;

/// The outcome of a routing decision.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoutingDecision {
    /// Which model was selected.
    pub model_id: String,
    /// Detected task type.
    pub task_type: TaskType,
    /// Which profile made the decision.
    pub profile: RoutingProfile,
    /// Human-readable reason for the choice.
    pub reason: String,
    /// Whether the user explicitly overrode routing.
    pub was_overridden: bool,
}

/// Select the best model for a task given a profile.
/// Returns `None` if no model matches (caller should fall back to default).
pub fn select_model(
    models: &[ModelConfig],
    profile: &RoutingProfile,
    task_type: &TaskType,
    prompt_len: usize,
) -> Option<String> {
    match profile {
        RoutingProfile::Manual => None,
        RoutingProfile::CostOptimized => select_cheapest(models),
        RoutingProfile::QualityFirst => select_highest_quality(models, task_type),
        RoutingProfile::Balanced => select_balanced(models, task_type, prompt_len),
    }
}

fn select_cheapest(models: &[ModelConfig]) -> Option<String> {
    models
        .iter()
        .min_by(|a, b| {
            a.capabilities
                .cost_per_1m_input
                .partial_cmp(&b.capabilities.cost_per_1m_input)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|m| m.id.clone())
}

fn select_highest_quality(models: &[ModelConfig], task_type: &TaskType) -> Option<String> {
    let tag = task_type_tag(task_type);
    models
        .iter()
        .filter(|m| m.capabilities.strength_tags.iter().any(|t| t == tag))
        .max_by(|a, b| {
            a.capabilities
                .cost_per_1m_input
                .partial_cmp(&b.capabilities.cost_per_1m_input)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .or_else(|| {
            models.iter().max_by(|a, b| {
                a.capabilities
                    .cost_per_1m_input
                    .partial_cmp(&b.capabilities.cost_per_1m_input)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        })
        .map(|m| m.id.clone())
}

fn select_balanced(
    models: &[ModelConfig],
    task_type: &TaskType,
    prompt_len: usize,
) -> Option<String> {
    if prompt_len < 500 {
        if let Some(local) = models
            .iter()
            .find(|m| m.capabilities.cost_per_1m_input == 0.0)
        {
            return Some(local.id.clone());
        }
    }

    let tag = task_type_tag(task_type);
    if let Some(matched) = models
        .iter()
        .find(|m| m.capabilities.strength_tags.iter().any(|t| t == tag))
    {
        return Some(matched.id.clone());
    }

    models.first().map(|m| m.id.clone())
}

fn task_type_tag(task_type: &TaskType) -> &'static str {
    match task_type {
        TaskType::CodeGeneration | TaskType::CodeRefactoring => "code",
        TaskType::LintFormat => "lint",
        TaskType::LargeContextAnalysis => "large-context",
        TaskType::Documentation => "code",
        TaskType::QuickQuery => "fast",
        TaskType::Architecture => "code",
        TaskType::Debug => "code",
        TaskType::Other => "code",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rift_bus::config::*;

    fn test_models() -> Vec<ModelConfig> {
        vec![
            ModelConfig {
                id: "local".to_string(),
                display_name: "Local".to_string(),
                provider: ProviderType::LlamaServer,
                model_identifier: "test.gguf".to_string(),
                hosting: HostingMode::Local {
                    process_config: LlamaServerConfig::default(),
                },
                endpoint: "http://127.0.0.1:8081".to_string(),
                api_key_ref: None,
                color: "--model-local".to_string(),
                short_id: "LOC".to_string(),
                capabilities: ModelCapabilities {
                    cost_per_1m_input: 0.0,
                    cost_per_1m_output: 0.0,
                    strength_tags: vec!["fast".to_string(), "lint".to_string()],
                    ..Default::default()
                },
            },
            ModelConfig {
                id: "cloud".to_string(),
                display_name: "Cloud".to_string(),
                provider: ProviderType::Anthropic,
                model_identifier: "claude".to_string(),
                hosting: HostingMode::Cloud,
                endpoint: "https://api.anthropic.com".to_string(),
                api_key_ref: Some("key".to_string()),
                color: "--model-claude".to_string(),
                short_id: "CLD".to_string(),
                capabilities: ModelCapabilities {
                    cost_per_1m_input: 15.0,
                    cost_per_1m_output: 75.0,
                    strength_tags: vec!["code".to_string(), "refactor".to_string()],
                    ..Default::default()
                },
            },
        ]
    }

    #[test]
    fn cost_optimized_picks_cheapest() {
        let models = test_models();
        let result = select_model(
            &models,
            &RoutingProfile::CostOptimized,
            &TaskType::Other,
            100,
        );
        assert_eq!(result, Some("local".to_string()));
    }

    #[test]
    fn quality_first_picks_most_expensive_with_tag() {
        let models = test_models();
        let result = select_model(
            &models,
            &RoutingProfile::QualityFirst,
            &TaskType::CodeRefactoring,
            100,
        );
        assert_eq!(result, Some("cloud".to_string()));
    }

    #[test]
    fn balanced_short_prompt_picks_local() {
        let models = test_models();
        let result = select_model(
            &models,
            &RoutingProfile::Balanced,
            &TaskType::QuickQuery,
            50,
        );
        assert_eq!(result, Some("local".to_string()));
    }

    #[test]
    fn balanced_code_task_picks_tag_match() {
        let models = test_models();
        let result = select_model(
            &models,
            &RoutingProfile::Balanced,
            &TaskType::CodeGeneration,
            1000,
        );
        assert_eq!(result, Some("cloud".to_string()));
    }

    #[test]
    fn manual_returns_none() {
        let models = test_models();
        let result = select_model(&models, &RoutingProfile::Manual, &TaskType::Other, 100);
        assert_eq!(result, None);
    }
}
