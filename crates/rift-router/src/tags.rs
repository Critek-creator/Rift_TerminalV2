//! @model tag parser — extracts model override directives from prompt text.
//!
//! Syntax: `@<model_short_id>` or `@<model_id>` at the start of the prompt
//! or anywhere preceded by whitespace. The tag is stripped from the prompt
//! text so the downstream model sees clean input.
//!
//! Examples: `@local lint this file`, `@CLD explain this code`, `@gemini-pro translate`

/// Result of parsing a prompt for @model tags.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedPrompt {
    /// The model identifier extracted from the tag (if any).
    pub model_tag: Option<String>,
    /// The prompt text with the @model tag stripped.
    pub clean_prompt: String,
}

/// Parse a prompt for `@<model>` tags. Returns the extracted model
/// identifier and the cleaned prompt text.
///
/// Rules:
/// - Tag must be at the very start of the prompt (optionally preceded by whitespace)
/// - Tag is the first `@`-prefixed word (alphanumeric + hyphen + underscore)
/// - Only one tag is extracted (the first one)
/// - Tag is case-insensitive for matching but preserved as-is
pub fn parse_model_tag(prompt: &str) -> ParsedPrompt {
    let trimmed = prompt.trim_start();

    if !trimmed.starts_with('@') {
        return ParsedPrompt {
            model_tag: None,
            clean_prompt: prompt.to_string(),
        };
    }

    let tag_text = &trimmed[1..];
    let tag_end = tag_text
        .find(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
        .unwrap_or(tag_text.len());

    if tag_end == 0 {
        return ParsedPrompt {
            model_tag: None,
            clean_prompt: prompt.to_string(),
        };
    }

    let tag = &tag_text[..tag_end];
    let rest = tag_text[tag_end..].trim_start();

    ParsedPrompt {
        model_tag: Some(tag.to_string()),
        clean_prompt: rest.to_string(),
    }
}

/// Resolve a tag string against configured models. Matches by:
/// 1. Exact model ID match (case-sensitive)
/// 2. Exact short_id match (case-insensitive)
/// 3. Provider name prefix match (e.g. "local" → first LlamaServer model)
pub fn resolve_tag(tag: &str, models: &[rift_bus::config::ModelConfig]) -> Option<String> {
    // 1. Exact ID match
    if let Some(m) = models.iter().find(|m| m.id == tag) {
        return Some(m.id.clone());
    }

    // 2. Short ID match (case-insensitive)
    let tag_lower = tag.to_lowercase();
    if let Some(m) = models
        .iter()
        .find(|m| m.short_id.to_lowercase() == tag_lower)
    {
        return Some(m.id.clone());
    }

    // 3. Provider alias match
    let alias_provider = match tag_lower.as_str() {
        "local" => Some(rift_bus::config::ProviderType::LlamaServer),
        "claude" | "anthropic" => Some(rift_bus::config::ProviderType::Anthropic),
        "gemini" | "google" => Some(rift_bus::config::ProviderType::Google),
        "server" | "remote" => Some(rift_bus::config::ProviderType::OpenAiCompat),
        _ => None,
    };

    if let Some(provider) = alias_provider {
        if let Some(m) = models.iter().find(|m| m.provider == provider) {
            return Some(m.id.clone());
        }
    }

    // 4. Display name prefix (case-insensitive)
    if let Some(m) = models
        .iter()
        .find(|m| m.display_name.to_lowercase().starts_with(&tag_lower))
    {
        return Some(m.id.clone());
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_tag() {
        let p = parse_model_tag("hello world");
        assert_eq!(p.model_tag, None);
        assert_eq!(p.clean_prompt, "hello world");
    }

    #[test]
    fn tag_at_start() {
        let p = parse_model_tag("@local lint this file");
        assert_eq!(p.model_tag, Some("local".into()));
        assert_eq!(p.clean_prompt, "lint this file");
    }

    #[test]
    fn tag_with_leading_whitespace() {
        let p = parse_model_tag("  @CLD explain this");
        assert_eq!(p.model_tag, Some("CLD".into()));
        assert_eq!(p.clean_prompt, "explain this");
    }

    #[test]
    fn tag_with_hyphens() {
        let p = parse_model_tag("@gemini-pro translate this");
        assert_eq!(p.model_tag, Some("gemini-pro".into()));
        assert_eq!(p.clean_prompt, "translate this");
    }

    #[test]
    fn tag_only_no_prompt() {
        let p = parse_model_tag("@local");
        assert_eq!(p.model_tag, Some("local".into()));
        assert_eq!(p.clean_prompt, "");
    }

    #[test]
    fn bare_at_sign() {
        let p = parse_model_tag("@ something");
        assert_eq!(p.model_tag, None);
        assert_eq!(p.clean_prompt, "@ something");
    }

    #[test]
    fn at_in_middle_not_extracted() {
        let p = parse_model_tag("send to @user");
        assert_eq!(p.model_tag, None);
        assert_eq!(p.clean_prompt, "send to @user");
    }

    #[test]
    fn resolve_by_exact_id() {
        let models = test_models();
        assert_eq!(resolve_tag("local-1", &models), Some("local-1".into()));
    }

    #[test]
    fn resolve_by_short_id_case_insensitive() {
        let models = test_models();
        assert_eq!(resolve_tag("cld", &models), Some("cloud-1".into()));
        assert_eq!(resolve_tag("CLD", &models), Some("cloud-1".into()));
    }

    #[test]
    fn resolve_by_provider_alias() {
        let models = test_models();
        assert_eq!(resolve_tag("local", &models), Some("local-1".into()));
        assert_eq!(resolve_tag("claude", &models), Some("cloud-1".into()));
    }

    #[test]
    fn resolve_unknown_returns_none() {
        let models = test_models();
        assert_eq!(resolve_tag("nonexistent", &models), None);
    }

    fn test_models() -> Vec<rift_bus::config::ModelConfig> {
        use rift_bus::config::*;
        vec![
            ModelConfig {
                id: "local-1".into(),
                display_name: "Local Llama".into(),
                provider: ProviderType::LlamaServer,
                model_identifier: "test.gguf".into(),
                hosting: HostingMode::Local {
                    process_config: LlamaServerConfig::default(),
                },
                endpoint: "http://127.0.0.1:8081".into(),
                api_key_ref: None,
                color: "--model-local".into(),
                short_id: "LOC".into(),
                capabilities: ModelCapabilities::default(),
            },
            ModelConfig {
                id: "cloud-1".into(),
                display_name: "Claude Sonnet".into(),
                provider: ProviderType::Anthropic,
                model_identifier: "claude-sonnet".into(),
                hosting: HostingMode::Cloud,
                endpoint: "https://api.anthropic.com/v1/messages".into(),
                api_key_ref: Some("key".into()),
                color: "--model-claude".into(),
                short_id: "CLD".into(),
                capabilities: ModelCapabilities {
                    cost_per_1m_input: 3.0,
                    cost_per_1m_output: 15.0,
                    strength_tags: vec!["code".into()],
                    ..Default::default()
                },
            },
        ]
    }
}
