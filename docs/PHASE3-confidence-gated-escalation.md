# Phase 3 — Confidence-Gated Escalation in rift-router

> Status: **IMPLEMENTED — shipped OFF (`confidence_threshold: None`); calibration says KEEP it off for grunt work** · Created 2026-06-04 · Scope: `rift-router`, `rift-bus` translators, `src-tauri/src/llm_commands.rs`
>
> Origin: surfaced during the 2026-06-04 local-LLM offload audit (`~/.claude` grunt stack). After fixing the dead-Ollama grunt path and mapping rift-router, this is the **one genuine "routing intelligence" gap** that remains — confirmed by both external research (RouteLLM/C3PO/cascade literature) and a direct code-read of this crate.

## Calibration result (2026-06-04) — empirical decision: do NOT enable for grunt work

Built across 9 files (commit `ef0dd22`) behind `confidence_threshold: None`, then calibrated empirically per the Test Plan before considering enabling it. Harness: `docs/phase3-calibration.py` — hits the resident granite-4.1-8b exactly as the grunt path does (`/v1/chat/completions`, `logprobs:true`, `top_logprobs:1`, temperature omitted), reduces logprobs with the same mean→`exp` formula as `compute_confidence`, over a 26-item labeled grunt set (classify / extract / arithmetic / factual / code) mixed with hard traps.

**Finding (`--runs 3`):**
- granite is **96% accurate (25/26)** on bounded grunt work.
- The persistent error — "how many r's in strawberry" → "1" — scored **mean 0.9905 / min-token 0.9892**. The model is *confidently wrong*.
- **Threshold sweep precision = 0.00 at every threshold, on both mean AND min per-token probability** (spec open-decision #1 tested). No cut catches the error before escalating only-correct answers; to reach the error's 0.99 confidence you escalate 4–5 correct grunt answers first.

**Decision: keep `confidence_threshold: None`.** The dangerous grunt errors are confidently-wrong (high logprob), so a logprob gate cannot catch them, while any threshold high enough to try taxes correct grunt work with paid cloud escalations — net cost, no accuracy gain. This is the spec's load-bearing miscalibration caveat, now empirical.

**What stays / what's deferred:** the machinery ships off-by-default (current behavior byte-identical) and the `confidence`/`mean_logprob` fields are now surfaced on the `llm.response` bus envelope and the `rift llm --json` output (`tool_llm_prompt`, takes effect on next app rebuild+restart) for observability and future recalibration. Re-evaluate only if (a) a *different* resident model is used, (b) the goal shifts to catching catastrophic low-confidence failures (truncation/garbage) rather than subtle wrong answers — though those are already covered by the failure-gated path — or (c) a calibrated, label-free method (C3PO conformal) replaces raw logprobs. Re-run `phase3-calibration.py` with a larger error sample before revisiting.

## Problem

`RouterService::escalate()` exists, is wired into the host loop, and is unit-tested — but escalation is **failure-gated only**. The loop in `src-tauri/src/llm_commands.rs`:

1. `router.route()` / `route_with_hint()` → picks a model + builds `fallback_chain`
2. executes the prompt (via `crates/rift-bus/src/translators/llm_server.rs`)
3. calls `router.escalate(current_model, fallback_chain, prompt)` **only on a hard error**

A call that **succeeds with a confidently-wrong answer is accepted as-is.** There is currently **no logprob/confidence signal anywhere in the stack** (verified: zero `logprob`/`n_probs`/`top_logprobs` hits across `crates/`). The router cannot see low confidence, so it cannot act on it.

This is the same class of gap as the literature's "confidence-gated cascade" (vs. the failure-gated cascade we already have). See `C3PO`, `RouteLLM`, bi-directional cascade work.

## Design — 4 touch points

### 1. `crates/rift-bus/src/translators/llm_server.rs` — capture confidence (net-new)

The llama-server HTTP call lives here. Request token probabilities and reduce to one scalar:

- `/v1/chat/completions` path → add `"logprobs": true, "top_logprobs": 1` to the request body.
- `/completion` path → add `"n_probs": 1`.
- Parse per-token logprobs from the response. **Reduce to a metric — recommended: mean token logprob → `exp()` → mean per-token probability** (0–1, human-readable). Retain the raw mean logprob too.

**Caveat (load-bearing — must stay in the doc):** logprob confidence is **miscalibrated** — fluent-but-wrong answers score high. This is therefore a *coarse* signal, trustworthy only for **bounded, checkable grunt work** (classify / extract / score / lint), NOT for reasoning (Architecture / Debug), where a confident-wrong logprob is a trap. The threshold MUST be calibrated empirically (see Test Plan), never guessed.

### 2. `llm.response` envelope (rift-bus) — surface it

Add to the response/envelope struct:

```rust
confidence: Option<f32>,     // mean per-token probability, 0..1
mean_logprob: Option<f32>,   // raw, for debugging/calibration
```

Two payoffs:
- The **cockpit llm-activity tab** can display it (observability — consistent with the rest of the stack's bus-tagging).
- The host caller can branch on it.

`Option` so providers that don't return logprobs (cloud CLI / Anthropic) are simply `None` → never confidence-escalate → safe default.

### 3. `crates/rift-router` (`lib.rs` + `profiles.rs` + `EnsembleConfig`) — threshold + pure helper

Router stays §9-pure (no external calls). Add:

- `confidence_threshold: Option<f32>` on `EnsembleConfig` (could become per-profile later). **`None` = feature off (default).**
- A pure helper:

```rust
/// True when a SUCCESSFUL completion's confidence is low enough to warrant
/// escalation — but only for task types where token-logprob confidence is a
/// meaningful signal (bounded/checkable work). Returns false for reasoning
/// types (logprob lies there) and when confidence is None or threshold unset.
pub fn should_escalate_on_confidence(
    &self,
    confidence: Option<f32>,
    task_type: &TaskType,
) -> bool
```

Meaningful types: `QuickQuery`, `LintFormat`, `Documentation`, `CodeGeneration` (bounded) — reuse the `profiles::task_type_tag` taxonomy. Explicitly NOT `Architecture` / `Debug` / `LargeContextAnalysis`.

### 4. `src-tauri/src/llm_commands.rs` — the gated loop (reuses existing `escalate()`)

After a **successful** execution:

```rust
if router.should_escalate_on_confidence(resp.confidence, &decision.task_type)
    && !fallback_chain.is_empty()
    && confidence_escalations_used == 0          // bound to ONE — cost guard
{
    if let Some(next) = router.escalate(&current_model_id, &fallback_chain, &clean_prompt) {
        // re-execute on `next`; KEEP the higher-confidence of the two answers
        confidence_escalations_used += 1;
    }
}
```

- **Bounded to one** confidence-escalation per call — a low-confidence answer must not thrash VRAM / spend cloud tokens in a loop.
- **Cost gate:** the escalation target is a partner/cloud model (cold-load eviction or paid call). Default OFF; when on, fire only for the grunt-eligible task types where the local draft is cheap and the win is real.
- Reuses the existing `escalate()` — no new routing machinery.

## Test Plan

- **Router unit tests** (`crates/rift-router`): `should_escalate_on_confidence` truth table — below/above threshold × meaningful/non-meaningful task type × `None` confidence × `None` threshold. Mirror the existing `escalate_*` tests.
- **Translator test** (`rift-bus`): canned llama-server response with known logprobs → assert computed `confidence`.
- **Calibration pass** (the real effort): run ~100–200 representative grunt prompts through the resident grunt model (`granite-4.1-8b`), record confidence vs. known-correct outcome, pick the threshold from the curve. **Ship behind `None` until this is done — do not ship a guessed number.**

## Open Decisions

1. **Metric:** ~~mean-token-probability (recommended, simple) · min-token-probability (stricter)~~ — **RESOLVED (calibration): neither separates correct from wrong on granite grunt work** (both gave precision 0.00; see Calibration result). C3PO conformal (label-free, calibrated) remains the only metric with a theoretical shot, but is more work and unbuilt.
2. **Escalation target policy:** local partner (cold-load/evict the grunt resident) vs. straight to **cloud Gemini** (no VRAM, costs latency). Lean **cloud** — confidence-escalation is rare; it shouldn't evict the grunt resident mid-session. (Moot while the feature stays off.)
3. **Rollout:** ~~ship behind `None` → calibrate → enable~~ — **RESOLVED: shipped behind `None`, calibrated, decision is to STAY off for grunt work** (see Calibration result). Not enabling.

## Why this is the last research item

The original 2026-06-04 research framed a broad "routing intelligence" axis (learned routers, cascades, cost-aware routing). The code-read showed rift-router **already has**: keyword task-classifier, 4 profiles, context-fit filtering, cost ranking, and wired error-gated escalation. The single piece that does **not** exist is gating escalation on a *successful-but-low-confidence* result. This spec is that piece. Learned routing (RouteLLM) was assessed as marginal-gain over the existing heuristics and is deliberately out of scope.

## References

- `crates/rift-router/src/lib.rs` — `RouterService`, `route_with_hint`, `build_fallback_chain`, `escalate`
- `crates/rift-router/src/profiles.rs` — `select_model`, `task_type_tag`, context-fit
- `crates/rift-router/src/classifier.rs` — keyword `TaskType` classifier
- `crates/rift-bus/src/translators/llm_server.rs` — llama-server HTTP call (logprob source)
- `src-tauri/src/llm_commands.rs` — host loop that calls `route()`/`escalate()`
- `~/.claude/skills/abyssal-engine/references/grunt-dispatch.md` — grunt-tier path (consumer of this router)
- `~/.claude/skills/aegis/references/model-routing.md` — partner-tier doctrine
