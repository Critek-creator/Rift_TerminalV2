//! Ensemble Router — LLM routing logic.
//!
//! This crate contains the routing rules engine, task classifier,
//! profile management, and @model tag parser. It has NO external calls
//! — all provider interaction goes through `rift-bus` translators
//! (§9 compliant).

use std::collections::HashSet;

use rift_bus::config::{EnsembleConfig, HostingMode, ModelConfig, RoutingProfile};

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

    /// Seed availability from the set of LOCAL models whose llama-server is
    /// currently running. Local models NOT in `running_ids` are marked
    /// unavailable so auto-routing and fallback chains never select a stopped
    /// server (the connection-fail cascade); local models that ARE running are
    /// marked available. Cloud and Remote models have no Rift-managed process,
    /// so they are always left available (never marked down by this sync).
    ///
    /// Explicit `@tag` / override routes bypass the availability filter (see
    /// [`route_with_hint`](Self::route_with_hint)), so a deliberate swap to a
    /// not-yet-loaded model still resolves — the caller is expected to load it
    /// first (load → call). This only constrains AUTO routing to what is
    /// actually serving, which is the fix for routing to stopped servers on a
    /// one-resident-at-a-time (VRAM-bound) host.
    pub fn sync_local_availability(&mut self, running_ids: &[String]) {
        let running: HashSet<&str> = running_ids.iter().map(String::as_str).collect();
        // Snapshot (id, is_up) first — can't borrow self.config.models while
        // mutating self.unavailable in the loop.
        let updates: Vec<(String, bool)> = self
            .config
            .models
            .iter()
            .filter(|m| matches!(m.hosting, HostingMode::Local { .. }))
            .map(|m| (m.id.clone(), running.contains(m.id.as_str())))
            .collect();
        for (id, is_up) in updates {
            if is_up {
                self.mark_available(&id);
            } else {
                self.mark_unavailable(&id);
            }
        }
    }

    /// Whether `model_id` is the configured task classifier. The classifier is
    /// a special-purpose refiner for the `Other` bucket — it must never be
    /// selected as a routing target or fallback for actual prompts.
    pub fn is_classifier(&self, model_id: &str) -> bool {
        self.config.classifier_model_id.as_deref() == Some(model_id)
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
        self.route_with_hint(prompt, override_model_id, None)
    }

    /// Like [`route`](Self::route) but accepts an optional pre-computed
    /// `TaskType`. When `Some`, it overrides the internal keyword classifier
    /// (e.g. a refinement from an LLM classifier the caller ran for the
    /// ambiguous `Other` bucket); when `None`, behavior is identical to
    /// `route`. §9 stays intact — the router still makes no external calls;
    /// the LLM classification happens in the caller/translator layer and is
    /// passed in here as data.
    pub fn route_with_hint(
        &self,
        prompt: &str,
        override_model_id: Option<&str>,
        task_hint: Option<TaskType>,
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
                task_type: task_hint.unwrap_or_else(|| classifier::classify(effective_prompt)),
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
                    task_type: task_hint.unwrap_or_else(|| classifier::classify(effective_prompt)),
                    profile: RoutingProfile::Manual,
                    reason: "manual profile — using default model".to_string(),
                    was_overridden: false,
                    fallback_chain: vec![],
                })
            }
            _ => {
                let task_type = task_hint.unwrap_or_else(|| classifier::classify(effective_prompt));

                // Filter to available models only, excluding the classifier
                // (it's a special-purpose refiner, never a routing target).
                let available_models: Vec<&ModelConfig> = self
                    .config
                    .models
                    .iter()
                    .filter(|m| self.is_available(&m.id) && !self.is_classifier(&m.id))
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
            .filter(|m| {
                m.id != primary_id && self.is_available(&m.id) && !self.is_classifier(&m.id)
            })
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

    /// Returns `true` when a SUCCESSFUL completion should be re-executed on a
    /// better model because the local model's confidence was too low.
    ///
    /// All of the following must hold:
    /// 1. `config.confidence_threshold` is `Some(t)` — feature is enabled.
    /// 2. `confidence` is `Some(c)` and `c < t` — signal is present and low.
    /// 3. `task_type` is one where token-logprob confidence is a meaningful
    ///    signal (bounded, checkable work). Explicitly NOT `Architecture`,
    ///    `Debug`, or `LargeContextAnalysis` where a fluent-but-wrong answer
    ///    can produce a deceptively high logprob score.
    ///
    /// When `confidence_threshold` is `None` (the default), returns `false`
    /// unconditionally — behavior is identical to before Phase 3.
    ///
    /// This method is §9-pure: no external calls, no side effects.
    pub fn should_escalate_on_confidence(
        &self,
        confidence: Option<f32>,
        task_type: &TaskType,
    ) -> bool {
        let Some(threshold) = self.config.confidence_threshold else {
            return false;
        };
        let Some(c) = confidence else {
            return false;
        };
        if c >= threshold {
            return false;
        }
        // Only escalate for task types where logprob confidence is reliable.
        // Architecture, Debug, and LargeContextAnalysis are explicitly excluded:
        // a model can confidently produce a wrong architectural decision or a
        // plausible-sounding but incorrect fix.
        matches!(
            task_type,
            TaskType::QuickQuery
                | TaskType::LintFormat
                | TaskType::Documentation
                | TaskType::CodeGeneration
                | TaskType::CodeRefactoring
        )
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
            classifier_model_id: None,
            confidence_threshold: None,
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

    #[test]
    fn route_with_hint_overrides_keyword_classifier() {
        let mut cfg = test_config();
        cfg.active_profile = RoutingProfile::Balanced;
        let svc = RouterService::new(cfg);

        // A long, keyword-less prompt keyword-classifies as `Other` → tag
        // "code" → the cloud model. This is the exact bucket the LLM
        // classifier exists to rescue.
        let long = "x ".repeat(300); // 600 chars, no keywords
        let baseline = svc.route(&long, None).unwrap();
        assert_eq!(baseline.task_type, TaskType::Other);
        assert_eq!(baseline.model_id, "cloud-test");

        // A `QuickQuery` hint flips the same prompt to the local model,
        // keeping it off the paid API.
        let hinted = svc
            .route_with_hint(&long, None, Some(TaskType::QuickQuery))
            .unwrap();
        assert_eq!(hinted.task_type, TaskType::QuickQuery);
        assert_eq!(hinted.model_id, "local-test");

        // A code hint still routes to cloud — the hint is honored, not ignored.
        let code = svc
            .route_with_hint(&long, None, Some(TaskType::CodeGeneration))
            .unwrap();
        assert_eq!(code.model_id, "cloud-test");
    }

    #[test]
    fn classifier_excluded_from_routing() {
        let mut cfg = test_config();
        cfg.active_profile = RoutingProfile::CostOptimized;
        // local-test is the cheapest model, but marking it the classifier must
        // remove it from the routing pool — real prompts go to cloud-test.
        cfg.classifier_model_id = Some("local-test".to_string());
        let svc = RouterService::new(cfg);
        let decision = svc.route("hello world", None).unwrap();
        assert_eq!(decision.model_id, "cloud-test");
    }

    #[test]
    fn sync_local_availability_avoids_stopped_local_keeps_cloud() {
        let mut cfg = test_config();
        cfg.active_profile = RoutingProfile::CostOptimized;
        let mut svc = RouterService::new(cfg);

        // Nothing running → the LOCAL model is marked unavailable, but the
        // CLOUD model stays available (no Rift-managed process). Auto-routing
        // must skip the stopped local server and land on cloud — this is the
        // fix for the connection-fail cascade.
        svc.sync_local_availability(&[]);
        assert!(!svc.is_available("local-test"));
        assert!(svc.is_available("cloud-test"));
        let decision = svc.route("hello world", None).unwrap();
        assert_eq!(decision.model_id, "cloud-test");

        // Once the local server is reported running, it becomes available
        // again — sync is idempotent and reversible.
        svc.sync_local_availability(&["local-test".to_string()]);
        assert!(svc.is_available("local-test"));
        assert!(svc.is_available("cloud-test"));
    }

    // -----------------------------------------------------------------------
    // should_escalate_on_confidence truth table (Phase 3)
    // -----------------------------------------------------------------------

    fn config_with_threshold(threshold: Option<f32>) -> EnsembleConfig {
        let mut cfg = test_config();
        cfg.confidence_threshold = threshold;
        cfg
    }

    /// None threshold (default) → always false regardless of confidence/type.
    #[test]
    fn escalate_confidence_none_threshold_always_false() {
        let svc = RouterService::new(config_with_threshold(None));
        // Below threshold, meaningful type — still false because feature is OFF.
        assert!(!svc.should_escalate_on_confidence(Some(0.3), &TaskType::QuickQuery));
        assert!(!svc.should_escalate_on_confidence(Some(0.3), &TaskType::CodeGeneration));
        assert!(!svc.should_escalate_on_confidence(Some(0.3), &TaskType::LintFormat));
        assert!(!svc.should_escalate_on_confidence(None, &TaskType::QuickQuery));
    }

    /// None confidence (provider didn't return logprobs) → always false.
    #[test]
    fn escalate_confidence_none_confidence_always_false() {
        let svc = RouterService::new(config_with_threshold(Some(0.7)));
        assert!(!svc.should_escalate_on_confidence(None, &TaskType::QuickQuery));
        assert!(!svc.should_escalate_on_confidence(None, &TaskType::CodeGeneration));
        assert!(!svc.should_escalate_on_confidence(None, &TaskType::Architecture));
    }

    /// Confidence above (or at) threshold → false even for meaningful types.
    #[test]
    fn escalate_confidence_above_threshold_false() {
        let svc = RouterService::new(config_with_threshold(Some(0.5)));
        assert!(!svc.should_escalate_on_confidence(Some(0.5), &TaskType::QuickQuery));
        assert!(!svc.should_escalate_on_confidence(Some(0.8), &TaskType::LintFormat));
        assert!(!svc.should_escalate_on_confidence(Some(0.99), &TaskType::Documentation));
    }

    /// Confidence below threshold, meaningful task types → true.
    #[test]
    fn escalate_confidence_below_threshold_meaningful_types_true() {
        let svc = RouterService::new(config_with_threshold(Some(0.6)));
        assert!(svc.should_escalate_on_confidence(Some(0.3), &TaskType::QuickQuery));
        assert!(svc.should_escalate_on_confidence(Some(0.3), &TaskType::LintFormat));
        assert!(svc.should_escalate_on_confidence(Some(0.3), &TaskType::Documentation));
        assert!(svc.should_escalate_on_confidence(Some(0.3), &TaskType::CodeGeneration));
        assert!(svc.should_escalate_on_confidence(Some(0.3), &TaskType::CodeRefactoring));
    }

    /// Confidence below threshold, NON-meaningful task types → false.
    /// These types produce deceptively high logprob scores for wrong answers.
    #[test]
    fn escalate_confidence_below_threshold_non_meaningful_types_false() {
        let svc = RouterService::new(config_with_threshold(Some(0.6)));
        assert!(!svc.should_escalate_on_confidence(Some(0.3), &TaskType::Architecture));
        assert!(!svc.should_escalate_on_confidence(Some(0.3), &TaskType::Debug));
        assert!(!svc.should_escalate_on_confidence(Some(0.3), &TaskType::LargeContextAnalysis));
        // Other is also excluded (catch-all bucket with unknown semantics).
        assert!(!svc.should_escalate_on_confidence(Some(0.3), &TaskType::Other));
    }

    /// Edge: confidence just barely below threshold → true for meaningful types.
    #[test]
    fn escalate_confidence_epsilon_below_threshold() {
        let svc = RouterService::new(config_with_threshold(Some(0.5)));
        // 0.4999 < 0.5 → true for a meaningful type.
        assert!(svc.should_escalate_on_confidence(Some(0.4999), &TaskType::LintFormat));
        // 0.5 == 0.5 → false (at-threshold is NOT below).
        assert!(!svc.should_escalate_on_confidence(Some(0.5), &TaskType::LintFormat));
    }
}
