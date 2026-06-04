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
    /// Ordered fallback models for escalation on retryable failure.
    #[serde(default)]
    pub fallback_chain: Vec<String>,
}

/// Select the best model for a task given a profile.
/// Returns `None` if no model matches (caller should fall back to default).
///
/// All non-Manual profiles are **context-aware**: a prompt is first filtered to
/// the models whose context window can actually hold it (see [`context_fit`]),
/// and only then ranked by the profile's cost/tag strategy. This stops oversized
/// prompts from routing to a model guaranteed to truncate, and lets a
/// large-context model win precisely when the prompt needs it.
pub fn select_model(
    models: &[ModelConfig],
    profile: &RoutingProfile,
    task_type: &TaskType,
    prompt_len: usize,
) -> Option<String> {
    match profile {
        RoutingProfile::Manual => None,
        RoutingProfile::CostOptimized => select_cheapest(models, prompt_len),
        RoutingProfile::QualityFirst => select_highest_quality(models, task_type, prompt_len),
        RoutingProfile::Balanced => select_balanced(models, task_type, prompt_len),
    }
}

/// Rough chars-per-token ratio for prompt-size estimation. The router has no
/// tokenizer, so 4 chars/token (typical for English prose; code/markup runs
/// denser) combined with [`OUTPUT_HEADROOM_TOKENS`] deliberately OVER-estimates
/// the context a prompt needs — the safe direction, since over-estimating picks
/// a roomier model while under-estimating routes to one that truncates.
const CHARS_PER_TOKEN: usize = 4;

/// Tokens reserved for the model's response on top of the estimated prompt, so a
/// context-fit selection leaves room to actually answer.
const OUTPUT_HEADROOM_TOKENS: u64 = 2048;

/// Estimate the context (prompt + response headroom), in tokens, that a prompt
/// of `prompt_len` characters requires.
fn estimate_context_tokens(prompt_len: usize) -> u64 {
    (prompt_len / CHARS_PER_TOKEN) as u64 + OUTPUT_HEADROOM_TOKENS
}

/// Filter `models` to those whose context window can hold `est_tokens`. If none
/// fit (the prompt exceeds every model's window), returns the single
/// largest-context model as a best-effort fallback — an oversized prompt should
/// route to the roomiest model rather than one guaranteed to truncate, or to
/// nothing at all.
fn context_fit(models: &[ModelConfig], est_tokens: u64) -> Vec<&ModelConfig> {
    let fitting: Vec<&ModelConfig> = models
        .iter()
        .filter(|m| m.capabilities.max_context_tokens >= est_tokens)
        .collect();
    if !fitting.is_empty() {
        return fitting;
    }
    models
        .iter()
        .max_by_key(|m| m.capabilities.max_context_tokens)
        .into_iter()
        .collect()
}

fn select_cheapest(models: &[ModelConfig], prompt_len: usize) -> Option<String> {
    let fit = context_fit(models, estimate_context_tokens(prompt_len));
    fit.iter()
        .min_by(|a, b| {
            a.capabilities
                .cost_per_1m_input
                .partial_cmp(&b.capabilities.cost_per_1m_input)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|m| m.id.clone())
}

fn select_highest_quality(
    models: &[ModelConfig],
    task_type: &TaskType,
    prompt_len: usize,
) -> Option<String> {
    let fit = context_fit(models, estimate_context_tokens(prompt_len));
    let tag = task_type_tag(task_type);
    fit.iter()
        .filter(|m| m.capabilities.strength_tags.iter().any(|t| t == tag))
        .max_by(|a, b| {
            a.capabilities
                .cost_per_1m_input
                .partial_cmp(&b.capabilities.cost_per_1m_input)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .or_else(|| {
            fit.iter().max_by(|a, b| {
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
    let fit = context_fit(models, estimate_context_tokens(prompt_len));

    // Small prompt → cheapest local model that still fits the context window.
    if prompt_len < 500 {
        if let Some(local) = fit.iter().find(|m| m.capabilities.cost_per_1m_input == 0.0) {
            return Some(local.id.clone());
        }
    }

    let tag = task_type_tag(task_type);
    if let Some(matched) = fit
        .iter()
        .find(|m| m.capabilities.strength_tags.iter().any(|t| t == tag))
    {
        return Some(matched.id.clone());
    }

    fit.first().map(|m| m.id.clone())
}

pub fn task_type_tag(task_type: &TaskType) -> &'static str {
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
                enabled: true,
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
                enabled: true,
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

    /// Three local models with distinct context windows, all zero-cost and all
    /// tagged `large-context`, so only the context-fit filter (not cost or tag)
    /// can differentiate them.
    fn ctx_models() -> Vec<ModelConfig> {
        let mk = |id: &str, ctx: u64| ModelConfig {
            id: id.to_string(),
            enabled: true,
            display_name: id.to_string(),
            provider: ProviderType::LlamaServer,
            model_identifier: "x.gguf".to_string(),
            hosting: HostingMode::Local {
                process_config: LlamaServerConfig::default(),
            },
            endpoint: "http://127.0.0.1:8081".to_string(),
            api_key_ref: None,
            color: "--model-local".to_string(),
            short_id: id.to_string(),
            capabilities: ModelCapabilities {
                cost_per_1m_input: 0.0,
                cost_per_1m_output: 0.0,
                max_context_tokens: ctx,
                supports_streaming: true,
                supports_tool_use: false,
                strength_tags: vec!["large-context".to_string()],
            },
        };
        // small 64K, mid 131K, big 256K — in config order.
        vec![mk("small", 65_536), mk("mid", 131_072), mk("big", 262_144)]
    }

    #[test]
    fn small_prompt_fits_all_picks_first_local() {
        let m = ctx_models();
        // Tiny prompt → all fit → cheapest/first zero-cost local.
        let r = select_model(
            &m,
            &RoutingProfile::Balanced,
            &TaskType::LargeContextAnalysis,
            100,
        );
        assert_eq!(r, Some("small".to_string()));
    }

    #[test]
    fn large_prompt_excludes_models_that_cannot_hold_it() {
        let m = ctx_models();
        // ~300K chars ≈ 75K+2K tokens → excludes small(64K); first fitting = mid.
        let r = select_model(
            &m,
            &RoutingProfile::Balanced,
            &TaskType::LargeContextAnalysis,
            300_000,
        );
        assert_eq!(r, Some("mid".to_string()));
    }

    #[test]
    fn huge_prompt_routes_to_only_model_that_fits() {
        let m = ctx_models();
        // ~600K chars ≈ 150K+2K tokens → excludes small(64K) + mid(131K) → big.
        let r = select_model(
            &m,
            &RoutingProfile::Balanced,
            &TaskType::LargeContextAnalysis,
            600_000,
        );
        assert_eq!(r, Some("big".to_string()));
    }

    #[test]
    fn oversized_prompt_falls_back_to_largest_context() {
        let m = ctx_models();
        // ~2M chars ≈ 500K tokens → exceeds every window → largest-context model.
        let r = select_model(&m, &RoutingProfile::Balanced, &TaskType::Other, 2_000_000);
        assert_eq!(r, Some("big".to_string()));
    }

    #[test]
    fn cost_optimized_respects_context_fit() {
        let m = ctx_models();
        // ~400K chars ≈ 100K+2K tokens drops only small(64K) from the pool;
        // cheapest of the fitting (mid 131K, big 256K — both free) is the
        // first, mid.
        let r = select_model(
            &m,
            &RoutingProfile::CostOptimized,
            &TaskType::Other,
            400_000,
        );
        assert_eq!(r, Some("mid".to_string()));
    }
}
