# Rift Local Model Metrics

Canonical performance reference for the local models in Rift's ensemble registry.
Use this to decide **which model fits a task** for co-work and grunt distribution.

**Measured:** 2026-06-04 · RTX 5080 16 GB · llama.cpp (CUDA, Blackwell FP4) ·
all models loaded identically: `--ctx-size 16384 --flash-attn on --n-gpu-layers 99
--cache-type-k q8_0 --cache-type-v q8_0`, thinking **off**.

> Re-run with `tools/`-style harness: load each server, POST `/completion`
> (`n_predict=256`, `cache_prompt:false`) and read `timings.{predicted,prompt}_per_second`
> for throughput; a 4-task chat suite (classify / extract / code / reason) for the
> capability sanity. One model resident at a time (16 GB = one large model).

---

## Throughput & footprint (measured)

Sorted by decode (generation) speed. Throughput is context-independent for short
generations; footprint is the GGUF weight size (KV cache adds on top, scaling with
configured `ctx_size` — see [the 12B note](#gemma-4-12b-the-large-context-all-rounder)).

| Model | Decode tok/s | Prompt-eval tok/s | Quick-cap (4) | Weights (GGUF) | Max ctx | Auto-start |
|---|---:|---:|:---:|---:|---:|:---:|
| **gemma-4-E4B** | **147.5** | 1,775 | 3/4 | 6.7 GB | 128K | no |
| **OmniCoder-9B** | **121.4** | 1,951 | 3/4 | 6.9 GB | 64K | no |
| **gemma-4-12b** | **98.6** | 3,165 | **4/4** | 7.1 GB | **256K** | no |
| **Qwen3.5-4B** | 77.8 | **5,076** | **4/4** | **3.1 GB** | 128K | no |
| **gpt-oss-20b** | 71.3 | 140 | 3/4 | 11.6 GB | 131K | no |
| **granite-4.1-8b** | 44.4 | **5,533** | **4/4** | 5.3 GB | 64K | **YES (resident)** |

**Quick-cap** = a 4-item sanity suite (classify-with-defs / JSON-extract / `is_palindrome` /
bat-and-ball), thinking **off**. It is a *sanity check, not a depth ranking* — the
authoritative coding-depth eval is the 2026-06-02 hard-eval (granite 5/5 top, OmniCoder &
gpt-oss 4/5). The single-task misses here (OmniCoder & E4B fail the bat-and-ball trap,
gpt-oss's classify mangles under forced no-think) are reasoning-trap / harness artifacts,
not incapability.

---

## What the numbers say

### Two speeds, not one
**Decode** (tokens generated/sec) and **prompt-eval** (tokens ingested/sec) are different
axes and they invert the ranking:

- **granite-4.1-8b** — *slowest decoder (44 t/s)* but *fastest prompt-eval (5,533 t/s)*.
  It reads input blazingly fast and emits slowly. **Ideal for short-output grunt**
  (classify, extract, score, route, digest) where you process a big prompt and emit little.
  Poor choice for long generation.
- **gemma-4-E4B (147) / OmniCoder-9B (121)** — *fastest decoders*. Best when the work is
  **generation-heavy** and short-context.

### Per-model role

#### granite-4.1-8b — the grunt default (resident)
4/4, tool-calling 5/5, hard-coding 5/5 (prior eval), top prompt-eval. The right default for
short-output mechanical work and the sole `auto_start` resident. Its only weakness is slow
*generation* — don't reach for it when the output is long.

#### gemma-4-12b — the large-context all-rounder
98.6 t/s decode (**2.2× granite**), 4/4 sanity, 3,165 t/s prompt-eval, and the **only local
model reaching 256K** (verified to load at full 256K on the 5080 — ~12.9 GB used, sliding-
window attention keeps KV cheap; ~9.4 GB at 128K). The strongest *fast* generalist:
faster-generating than the default and far roomier. **Reach for it for generation-heavy grunt
(docstrings, boilerplate, HTML render, summaries) and for any prompt that overflows the other
locals (131K–256K) but you want kept on-device/private** instead of going to Gemini cloud.
Thinking is on by default — keep it **off** for our use cases ([bench](#thinking-on-vs-off)).
Tool-calling **3/3 verified** (2026-06-04 — correct tool + argument JSON; needs `--jinja`,
which Rift always launches with), so `supports_tool_use=true`. Multimodal-capable
(`mmproj-F16.gguf` present) but wired text-only.

#### gemma-4-E4B — pure-speed grunt
Fastest decoder (147). Use for max-throughput mechanical work (classify / extract / summarize)
and vision (mmproj wired). 3/4 — weaker on reasoning traps; don't hand it multi-step logic.

#### OmniCoder-9B — fast code drafts
121 t/s decode, code/fill-in-middle. Quick code drafts where speed beats depth (4/5 prior eval).

#### Qwen3.5-4B — speed-per-VRAM champion
Only 3.1 GB, yet 77.8 decode / 5,076 prompt / 4/4 / 128K / tool-calling 5/5. The pick when you
want to **leave VRAM free** for other work, or need large context at minimal footprint.

#### gpt-oss-20b — the heavyweight reasoner
Slowest prompt-eval (140 t/s — large MoE), but the deep-reasoning + agentic large-context
specialist. Thinking model (always emits `reasoning_content`; give `max_tokens ≥ 8–16K` for
hard work or it returns empty). Escalate here when granite's depth/64K isn't enough and you
don't want cloud.

---

## Grunt / co-work distribution cheat-sheet

| Task shape | Pick | Why |
|---|---|---|
| Short-output grunt (classify, extract, score, route, dedup) | **granite-4.1-8b** (resident) | 4/4 + top prompt-eval; already loaded |
| Max-throughput mechanical (bulk classify/summarize) | **gemma-4-E4B** / **Qwen3.5-4B** | fastest decode / best per-VRAM |
| Generation-heavy grunt (docstrings, boilerplate, HTML, summaries) | **gemma-4-12b** / **OmniCoder-9B** | fast decode, granite is slow here |
| Big local context 131K–256K (private) | **gemma-4-12b** | only local model that reaches it |
| Quick code drafts | **OmniCoder-9B** | fast coder #2 |
| Deep reasoning / agentic / large-ctx think | **gpt-oss-20b** (thinking on) | reasoning specialist |
| Beyond local window / strongest available | **Gemini-2.5-pro** (cloud, 1M) | only >131K + only cloud |
| **Quality floor** (review, architecture, security judgment, synthesis) | **Claude tier** — never local | local models do bounded/checkable work + drafts; Claude owns judgment |

**VRAM rule:** 16 GB = one large local model resident. Calling a non-resident model triggers
a 10–30 s cold load that evicts the current resident. Batch work on one model; weigh a cold
swap against a Claude tier or (no-VRAM) Gemini cloud for one-shots.

---

## Thinking on vs off
For gemma-4-12b across our use cases (classify / extract / summarize / reformat / easy-code +
classic reasoning traps), thinking **off** scored identical correctness to on at ~6× less
latency / ~7× fewer tokens. **Default thinking off** for all grunt + moderate-reasoning work
(`chat_template_kwargs: {enable_thinking: false}` raw, or `enable_thinking:false` on
`llm_prompt`). Reserve thinking-on for genuinely novel multi-step problems where you can't
cheaply verify the answer — and budget ≥ 8–16K tokens or the answer lands empty (the reasoning
eats the budget).

---

*Reproducible; re-run after any model swap or llama.cpp upgrade. Pairs with the Aegis
`references/model-routing.md` doctrine (Claude-side routing) and the 2026-06-02 hard-eval
coding ranking.*
