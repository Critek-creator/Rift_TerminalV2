//! Ensemble Router — LLM routing logic.
//!
//! This crate contains the routing rules engine, task classifier,
//! profile management, and @model tag parser. It has NO external calls
//! — all provider interaction goes through `rift-bus` translators
//! (§9 compliant).

use std::collections::HashSet;

use rift_bus::config::{EnsembleConfig, ModelConfig, RoutingProfile};

pub mod classifier;
pub mod profiles;
pub mod tags;

pub use classifier::TaskType;
pub use profiles::RoutingDecision;
pub use tags::{parse_model_tag, resolve_tag, ParsedPrompt};

/// The core routing service. Holds the active configuration and
/// resolves which model should handle a given prompt.
pub struct RouterService {
    config: EnsembleConfig,
    /// Model IDs currently known to be unavailable (health check failed
    /// or recent error). Callers update this via `mark_unavailable` /
    /// `mark_available`. The router skips these during auto-routing.
    unavailable: HashSet<String>,
}

impl RouterService {
    pub fn new(mut config: EnsembleConfig) -> Self {
        // Disabled models are invisible to routing (tags, profiles, default,
        // and `models()`). The Settings UI keeps the full list separately.
        config.models.retain(|m| m.enabled);
        Self {
            config,
            unavailable: HashSet::new(),
        }
    }

    pub fn reload(&mut self, mut config: EnsembleConfig) {
        config.models.retain(|m| m.enabled);
        self.config = config;
        self.unavailable.clear();
    }

    pub fn mark_unavailable(&mut self, model_id: &str) {
        self.unavailable.insert(model_id.to_string());
    }

    pub fn mark_available(&mut self, model_id: &str) {
        self.unavailable.remove(model_id);
    }

    pub fn is_available(&self, model_id: &str) -> bool {
        !self.unavailable.contains(model_id)
    }

    /// Route a prompt to the appropriate model.
    ///
    /// Full Phase 2 routing pipeline:
    /// 1. Parse @model tags from the prompt
    /// 2. If tag found, resolve to model ID (override)
    /// 3. If explicit override_model_id, use that
    /// 4. Otherwise, use active profile (Manual → default, auto → classifier + rules)
    /// 5. Skip unavailable models in auto-routing
    pub fn route(
        &self,
        prompt: &str,
        override_model_id: Option<&str>,
    ) -> Result<RoutingDecision, RoutingError> {
        // Phase 2: Parse @model tags from prompt
        let parsed = tags::parse_model_tag(prompt);
        let effective_prompt = &parsed.clean_prompt;

        // Resolve tag to model ID if present
        let tag_override = parsed
            .model_tag
            .as_ref()
            .and_then(|tag| tags::resolve_tag(tag, &self.config.models));

        // Priority: explicit override > @tag override > profile routing
        let effective_override = override_model_id.map(|s| s.to_string()).or(tag_override);

        if let Some(ref id) = effective_override {
            let model = self.find_model(id)?;
            let was_tag = parsed.model_tag.is_some() && override_model_id.is_none();
            return Ok(RoutingDecision {
                model_id: model.id.clone(),
                task_type: classifier::classify(effective_prompt),
                profile: self.config.active_profile.clone(),
                reason: if was_tag {
                    format!(
                        "@{} tag override",
                        parsed.model_tag.as_deref().unwrap_or("")
                    )
                } else {
                    "explicit override".to_string()
                },
                was_overridden: true,
                fallback_chain: vec![],
            });
        }

        match self.config.active_profile {
            RoutingProfile::Manual => {
                if self.config.default_model.is_empty() {
                    return Err(RoutingError::NoDefaultModel);
                }
                let model = self.find_model(&self.config.default_model)?;
                Ok(RoutingDecision {
                    model_id: model.id.clone(),
                    task_type: classifier::classify(effective_prompt),
                    profile: RoutingProfile::Manual,
                    reason: "manual profile — using default model".to_string(),
                    was_overridden: false,
                    fallback_chain: vec![],
                })
            }
            _ => {
                let task_type = classifier::classify(effective_prompt);

                // Filter to available models only
                let available_models: Vec<&ModelConfig> = self
                    .config
                    .models
                    .iter()
                    .filter(|m| self.is_available(&m.id))
                    .collect();

                if available_models.is_empty() {
                    return Err(RoutingError::AllModelsUnavailable);
                }

                let model_id = profiles::select_model(
                    &available_models
                        .iter()
                        .map(|m| (*m).clone())
                        .collect::<Vec<_>>(),
                    &self.config.active_profile,
                    &task_type,
                    effective_prompt.len(),
                );

                match model_id {
                    Some(id) => Ok(RoutingDecision {
                        model_id: id.clone(),
                        task_type,
                        profile: self.config.active_profile.clone(),
                        reason: format!(
                            "{:?} profile — matched task type {:?}",
                            self.config.active_profile, task_type
                        ),
                        was_overridden: false,
                        fallback_chain: self.build_fallback_chain(&id, &task_type),
                    }),
                    None => {
                        if self.config.default_model.is_empty() {
                            Err(RoutingError::NoDefaultModel)
                        } else if !self.is_available(&self.config.default_model) {
                            Err(RoutingError::AllModelsUnavailable)
                        } else {
                            Ok(RoutingDecision {
                                model_id: self.config.default_model.clone(),
                                task_type,
                                profile: self.config.active_profile.clone(),
                                reason: "no rule matched — fallback to default".to_string(),
                                was_overridden: false,
                                fallback_chain: vec![],
                            })
                        }
                    }
                }
            }
        }
    }

    /// Build a ranked fallback chain for escalation. Excludes the primary
    /// model and any unavailable models. Used when the primary model fails
    /// with a retryable error.
    fn build_fallback_chain(&self, primary_id: &str, task_type: &TaskType) -> Vec<String> {
        let tag = profiles::task_type_tag(task_type);
        let mut candidates: Vec<&ModelConfig> = self
            .config
            .models
            .iter()
            .filter(|m| m.id != primary_id && self.is_available(&m.id))
            .collect();

        // Sort: tag-matched first, then by cost (cheapest first)
        candidates.sort_by(|a, b| {
            let a_match = a.capabilities.strength_tags.iter().any(|t| t == tag);
            let b_match = b.capabilities.strength_tags.iter().any(|t| t == tag);
            b_match.cmp(&a_match).then_with(|| {
                a.capabilities
                    .cost_per_1m_input
                    .partial_cmp(&b.capabilities.cost_per_1m_input)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        });

        candidates.iter().map(|m| m.id.clone()).collect()
    }

    /// Route to the next model in the fallback chain after a failure.
    /// Returns None if no more fallbacks are available.
    pub fn escalate(
        &self,
        failed_model_id: &str,
        fallback_chain: &[String],
        prompt: &str,
    ) -> Option<RoutingDecision> {
        let task_type = classifier::classify(prompt);

        for candidate_id in fallback_chain {
            if candidate_id == failed_model_id || !self.is_available(candidate_id) {
                continue;
            }
            if self.find_model(candidate_id).is_ok() {
                return Some(RoutingDecision {
                    model_id: candidate_id.clone(),
                    task_type,
                    profile: self.config.active_profile.clone(),
                    reason: format!("escalation — {} failed", failed_model_id),
                    was_overridden: false,
                    fallback_chain: fallback_chain
                        .iter()
                        .filter(|id| id.as_str() != candidate_id && id.as_str() != failed_model_id)
                        .cloned()
                        .collect(),
                });
            }
        }

        None
    }

    pub fn find_model(&self, id: &str) -> Result<&ModelConfig, RoutingError> {
        self.config
            .models
            .iter()
            .find(|m| m.id == id)
            .ok_or_else(|| RoutingError::ModelNotFound(id.to_string()))
    }

    pub fn models(&self) -> &[ModelConfig] {
        &self.config.models
    }

    pub fn config(&self) -> &EnsembleConfig {
        &self.config
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RoutingError {
    #[error("no default model configured — select one in Settings → Models")]
    NoDefaultModel,

    #[error("model not found: {0}")]
    ModelNotFound(String),

    #[error("all configured models are unavailable")]
    AllModelsUnavailable,

    #[error("@model tag '{0}' did not match any configured model")]
    TagNotResolved(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use rift_bus::config::*;

    fn test_config() -> EnsembleConfig {
        EnsembleConfig {
            enabled: true,
            active_profile: RoutingProfile::Manual,
            default_model: "local-test".to_string(),
            models: vec![
                ModelConfig {
                    id: "local-test".to_string(),
                    enabled: true,
                    display_name: "Test Local".to_string(),
                    provider: ProviderType::LlamaServer,
                    model_identifier: "test.gguf".to_string(),
                    hosting: HostingMode::Local {
                        process_config: LlamaServerConfig::default(),
                    },
                    endpoint: "http://127.0.0.1:8081".to_string(),
                    api_key_ref: None,
                    color: "--model-local".to_string(),
                    short_id: "TST".to_string(),
                    capabilities: ModelCapabilities::default(),
                },
                ModelConfig {
                    id: "cloud-test".to_string(),
                    enabled: true,
                    display_name: "Test Cloud".to_string(),
                    provider: ProviderType::Anthropic,
                    model_identifier: "claude-test".to_string(),
                    hosting: HostingMode::Cloud,
                    endpoint: "https://api.anthropic.com/v1/messages".to_string(),
                    api_key_ref: Some("test-key".to_string()),
                    color: "--model-claude".to_string(),
                    short_id: "CLD".to_string(),
                    capabilities: ModelCapabilities {
                        cost_per_1m_input: 15.0,
                        cost_per_1m_output: 75.0,
                        strength_tags: vec!["code".to_string(), "refactor".to_string()],
                        ..Default::default()
                    },
                },
            ],
        }
    }

    #[test]
    fn manual_route_returns_default() {
        let svc = RouterService::new(test_config());
        let decision = svc.route("hello", None).unwrap();
        assert_eq!(decision.model_id, "local-test");
        assert!(!decision.was_overridden);
        assert_eq!(decision.reason, "manual profile — using default model");
    }

    #[test]
    fn override_route() {
        let svc = RouterService::new(test_config());
        let decision = svc.route("hello", Some("cloud-test")).unwrap();
        assert_eq!(decision.model_id, "cloud-test");
        assert!(decision.was_overridden);
    }

    #[test]
    fn override_nonexistent_model_errors() {
        let svc = RouterService::new(test_config());
        let err = svc.route("hello", Some("nonexistent")).unwrap_err();
        assert!(matches!(err, RoutingError::ModelNotFound(_)));
    }

    #[test]
    fn no_default_model_errors() {
        let mut cfg = test_config();
        cfg.default_model = String::new();
        let svc = RouterService::new(cfg);
        let err = svc.route("hello", None).unwrap_err();
        assert!(matches!(err, RoutingError::NoDefaultModel));
    }

    #[test]
    fn find_model_by_id() {
        let svc = RouterService::new(test_config());
        let m = svc.find_model("cloud-test").unwrap();
        assert_eq!(m.display_name, "Test Cloud");
    }

    #[test]
    fn reload_replaces_config() {
        let mut svc = RouterService::new(test_config());
        let mut new_cfg = test_config();
        new_cfg.default_model = "cloud-test".to_string();
        svc.reload(new_cfg);
        let decision = svc.route("hello", None).unwrap();
        assert_eq!(decision.model_id, "cloud-test");
    }

    #[test]
    fn tag_override_routes_to_tagged_model() {
        let svc = RouterService::new(test_config());
        let decision = svc.route("@CLD explain this code", None).unwrap();
        assert_eq!(decision.model_id, "cloud-test");
        assert!(decision.was_overridden);
        assert!(decision.reason.contains("@CLD"));
    }

    #[test]
    fn unavailable_model_skipped_in_auto_routing() {
        let mut cfg = test_config();
        cfg.active_profile = RoutingProfile::CostOptimized;
        let mut svc = RouterService::new(cfg);
        svc.mark_unavailable("local-test");
        let decision = svc.route("hello world", None).unwrap();
        assert_eq!(decision.model_id, "cloud-test");
    }

    #[test]
    fn all_unavailable_returns_error() {
        let mut cfg = test_config();
        cfg.active_profile = RoutingProfile::CostOptimized;
        let mut svc = RouterService::new(cfg);
        svc.mark_unavailable("local-test");
        svc.mark_unavailable("cloud-test");
        let err = svc.route("hello", None).unwrap_err();
        assert!(matches!(err, RoutingError::AllModelsUnavailable));
    }

    #[test]
    fn escalate_picks_next_in_chain() {
        let svc = RouterService::new(test_config());
        let fallback = vec!["cloud-test".to_string()];
        let decision = svc.escalate("local-test", &fallback, "hello").unwrap();
        assert_eq!(decision.model_id, "cloud-test");
        assert!(decision.reason.contains("escalation"));
    }

    #[test]
    fn escalate_returns_none_when_exhausted() {
        let mut svc = RouterService::new(test_config());
        svc.mark_unavailable("cloud-test");
        let fallback = vec!["cloud-test".to_string()];
        assert!(svc.escalate("local-test", &fallback, "hello").is_none());
    }

    #[test]
    fn fallback_chain_excludes_primary() {
        let mut cfg = test_config();
        cfg.active_profile = RoutingProfile::Balanced;
        let svc = RouterService::new(cfg);
        let decision = svc
            .route("implement a new feature with lots of code that goes beyond the short prompt threshold", None)
            .unwrap();
        assert!(!decision.fallback_chain.contains(&decision.model_id));
    }
}
