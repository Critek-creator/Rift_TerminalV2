//! Ensemble Router — LLM routing logic.
//!
//! This crate contains the routing rules engine, task classifier, and
//! profile management. It has NO external calls — all provider
//! interaction goes through `rift-bus` translators (§9 compliant).
//!
//! Phase 1: manual routing only (user selects model explicitly).
//! Phase 2: automatic routing via rules engine + task classifier.

use rift_bus::config::{EnsembleConfig, ModelConfig, RoutingProfile};

mod classifier;
mod profiles;

pub use classifier::TaskType;
pub use profiles::RoutingDecision;

/// The core routing service. Holds the active configuration and
/// resolves which model should handle a given prompt.
pub struct RouterService {
    config: EnsembleConfig,
}

impl RouterService {
    /// Create a new router from config.
    pub fn new(config: EnsembleConfig) -> Self {
        Self { config }
    }

    /// Reload configuration (e.g., after settings change).
    pub fn reload(&mut self, config: EnsembleConfig) {
        self.config = config;
    }

    /// Route a prompt to the appropriate model. Returns the model config
    /// and the routing decision metadata.
    ///
    /// Phase 1: Manual profile always returns `default_model` or `override_id`.
    /// Phase 2: Rules engine evaluates task type, token estimate, availability.
    pub fn route(
        &self,
        prompt: &str,
        override_model_id: Option<&str>,
    ) -> Result<RoutingDecision, RoutingError> {
        if let Some(id) = override_model_id {
            let model = self.find_model(id)?;
            return Ok(RoutingDecision {
                model_id: model.id.clone(),
                task_type: classifier::classify(prompt),
                profile: self.config.active_profile.clone(),
                reason: "explicit override".to_string(),
                was_overridden: true,
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
                    task_type: classifier::classify(prompt),
                    profile: RoutingProfile::Manual,
                    reason: "manual profile — using default model".to_string(),
                    was_overridden: false,
                })
            }
            _ => {
                let task_type = classifier::classify(prompt);
                let model_id = profiles::select_model(
                    &self.config.models,
                    &self.config.active_profile,
                    &task_type,
                    prompt.len(),
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
                    }),
                    None => {
                        if self.config.default_model.is_empty() {
                            Err(RoutingError::NoDefaultModel)
                        } else {
                            Ok(RoutingDecision {
                                model_id: self.config.default_model.clone(),
                                task_type,
                                profile: self.config.active_profile.clone(),
                                reason: "no rule matched — fallback to default".to_string(),
                                was_overridden: false,
                            })
                        }
                    }
                }
            }
        }
    }

    /// Look up a model by ID.
    pub fn find_model(&self, id: &str) -> Result<&ModelConfig, RoutingError> {
        self.config
            .models
            .iter()
            .find(|m| m.id == id)
            .ok_or_else(|| RoutingError::ModelNotFound(id.to_string()))
    }

    /// List all configured models.
    pub fn models(&self) -> &[ModelConfig] {
        &self.config.models
    }
}

/// Errors from the routing engine.
#[derive(Debug, thiserror::Error)]
pub enum RoutingError {
    /// No default model configured and no override specified.
    #[error("no default model configured — select one in Settings → Models")]
    NoDefaultModel,

    /// Requested model ID not found in config.
    #[error("model not found: {0}")]
    ModelNotFound(String),
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
}
