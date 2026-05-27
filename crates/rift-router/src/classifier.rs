//! Task type classifier — keyword/pattern matching on prompts.
//!
//! Phase 1: simple keyword detection.
//! Phase 2: configurable rules, possibly local LLM classification.

use serde::{Deserialize, Serialize};

/// Detected task type — used by the routing rules engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskType {
    CodeGeneration,
    CodeRefactoring,
    LintFormat,
    LargeContextAnalysis,
    Documentation,
    QuickQuery,
    Architecture,
    Debug,
    Other,
}

/// Classify a prompt into a task type via keyword matching.
pub fn classify(prompt: &str) -> TaskType {
    let lower = prompt.to_lowercase();

    if matches_any(
        &lower,
        &["lint", "format", "prettier", "eslint", "clippy", "rustfmt"],
    ) {
        return TaskType::LintFormat;
    }
    if matches_any(
        &lower,
        &["refactor", "rename", "extract", "move method", "inline"],
    ) {
        return TaskType::CodeRefactoring;
    }
    if matches_any(
        &lower,
        &["architecture", "design", "system design", "tradeoff"],
    ) {
        return TaskType::Architecture;
    }
    if matches_any(
        &lower,
        &["debug", "fix bug", "stack trace", "error", "crash", "panic"],
    ) {
        return TaskType::Debug;
    }
    if matches_any(
        &lower,
        &[
            "analyze",
            "review",
            "audit",
            "scan",
            "codebase",
            "entire project",
        ],
    ) {
        return TaskType::LargeContextAnalysis;
    }
    if matches_any(
        &lower,
        &["document", "readme", "docstring", "jsdoc", "comment"],
    ) {
        return TaskType::Documentation;
    }
    if matches_any(
        &lower,
        &[
            "generate",
            "implement",
            "create",
            "build",
            "write code",
            "add feature",
        ],
    ) {
        return TaskType::CodeGeneration;
    }
    if lower.split_whitespace().count() < 10 {
        return TaskType::QuickQuery;
    }

    TaskType::Other
}

fn matches_any(text: &str, keywords: &[&str]) -> bool {
    keywords.iter().any(|kw| text.contains(kw))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_lint() {
        assert_eq!(classify("run clippy on this file"), TaskType::LintFormat);
        assert_eq!(classify("format with prettier"), TaskType::LintFormat);
    }

    #[test]
    fn classify_refactor() {
        assert_eq!(
            classify("refactor the auth module"),
            TaskType::CodeRefactoring
        );
        assert_eq!(classify("rename this function"), TaskType::CodeRefactoring);
    }

    #[test]
    fn classify_code_gen() {
        assert_eq!(
            classify("implement a new login page"),
            TaskType::CodeGeneration
        );
        assert_eq!(
            classify("generate a REST API handler"),
            TaskType::CodeGeneration
        );
    }

    #[test]
    fn classify_large_context() {
        assert_eq!(
            classify("analyze the entire codebase for issues"),
            TaskType::LargeContextAnalysis
        );
    }

    #[test]
    fn classify_debug() {
        assert_eq!(classify("fix bug in the parser"), TaskType::Debug);
        assert_eq!(classify("this stack trace shows a panic"), TaskType::Debug);
    }

    #[test]
    fn classify_quick_query() {
        assert_eq!(classify("what is a mutex?"), TaskType::QuickQuery);
    }

    #[test]
    fn classify_other() {
        assert_eq!(
            classify("this is a longer prompt that doesn't match any specific keyword pattern and has many words"),
            TaskType::Other,
        );
    }
}
