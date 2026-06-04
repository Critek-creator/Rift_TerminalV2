//! Integration test guarding the *intended* routing behavior of the real Rift
//! roster (mirrored here so the test is deterministic and CI-safe — it does not
//! read the user's config). Captures three decisions that regressed or were
//! added on 2026-06-04:
//!   1. code tasks route to granite (the `"Coding"` → `"code"` tag fix),
//!   2. normal large-context work stays on gpt-oss (gemma-12b does not steal it),
//!   3. oversized prompts route to gemma-12b via context-fit (256K reachable),
//!      skipping the tag-less cloud model.

use rift_bus::config::*;
use rift_router::{RouterService, TaskType};

fn local(id: &str, ctx: u64, tags: &[&str]) -> ModelConfig {
    ModelConfig {
        id: id.to_string(),
        enabled: true,
        display_name: id.to_string(),
        provider: ProviderType::LlamaServer,
        model_identifier: "x.gguf".to_string(),
        hosting: HostingMode::Local {
            process_config: LlamaServerConfig::default(),
        },
        endpoint: "http://127.0.0.1:8080".to_string(),
        api_key_ref: None,
        color: "--model-local".to_string(),
        short_id: id.to_string(),
        capabilities: ModelCapabilities {
            cost_per_1m_input: 0.0,
            cost_per_1m_output: 0.0,
            max_context_tokens: ctx,
            supports_streaming: true,
            supports_tool_use: true,
            strength_tags: tags.iter().map(|s| s.to_string()).collect(),
        },
    }
}

fn cloud(id: &str, ctx: u64) -> ModelConfig {
    ModelConfig {
        id: id.to_string(),
        enabled: true,
        display_name: id.to_string(),
        provider: ProviderType::Cli,
        model_identifier: "gemini".to_string(),
        hosting: HostingMode::Cloud,
        endpoint: "gemini -p {prompt}".to_string(),
        api_key_ref: None,
        color: "--model-custom".to_string(),
        short_id: id.to_string(),
        capabilities: ModelCapabilities {
            cost_per_1m_input: 0.0,
            cost_per_1m_output: 0.0,
            max_context_tokens: ctx,
            supports_streaming: true,
            supports_tool_use: true,
            strength_tags: Vec::new(), // cloud has no routing tags (mirrors live config)
        },
    }
}

/// Mirror of the live roster's relevant subset, in config order.
fn roster() -> EnsembleConfig {
    EnsembleConfig {
        enabled: true,
        active_profile: RoutingProfile::Balanced,
        default_model: "granite".to_string(),
        models: vec![
            local("granite", 65_536, &["classification", "code"]),
            local("gpt-oss", 131_072, &["large-context", "code"]),
            cloud("gemini", 1_000_000),
            local("gemma-12b", 262_144, &["large-context", "tool-calling"]),
        ],
        classifier_model_id: None,
    }
}

#[test]
fn short_grunt_goes_to_resident_default() {
    let svc = RouterService::new(roster());
    let d = svc.route("classify this task", None).unwrap();
    assert_eq!(d.task_type, TaskType::QuickQuery);
    assert_eq!(d.model_id, "granite");
}

#[test]
fn code_task_routes_to_granite_after_tag_fix() {
    let svc = RouterService::new(roster());
    // Long enough to skip the <500-char short-circuit; "implement" → CodeGeneration.
    let prompt = format!(
        "Please implement a new REST API handler for user login. {}",
        "context ".repeat(80)
    );
    let d = svc.route(&prompt, None).unwrap();
    assert_eq!(d.task_type, TaskType::CodeGeneration);
    // granite carries the (corrected) lowercase "code" tag and is first in
    // order, so it wins — NOT gpt-oss. This is the regression guard for the
    // "Coding" → "code" config fix.
    assert_eq!(d.model_id, "granite");
}

#[test]
fn normal_large_context_stays_on_gpt_oss() {
    let svc = RouterService::new(roster());
    let prompt = format!(
        "analyze and review this module for issues. {}",
        "x ".repeat(300)
    );
    let d = svc.route(&prompt, None).unwrap();
    assert_eq!(d.task_type, TaskType::LargeContextAnalysis);
    // Fits every model → first with the large-context tag = gpt-oss. gemma-12b
    // must NOT steal ordinary large-context work.
    assert_eq!(d.model_id, "gpt-oss");
}

#[test]
fn oversized_prompt_routes_to_gemma_12b_via_context_fit() {
    let svc = RouterService::new(roster());
    // ~600K chars ≈ 152K tokens: excludes granite(64K) and gpt-oss(131K). Of the
    // models that fit (gemini 1M, gemma-12b 256K), the tag-less cloud model is
    // skipped and gemma-12b — carrying large-context — wins.
    let prompt = "analyze the entire codebase for issues. ".repeat(15_000);
    let d = svc.route(&prompt, None).unwrap();
    assert_eq!(d.task_type, TaskType::LargeContextAnalysis);
    assert_eq!(d.model_id, "gemma-12b");
}
