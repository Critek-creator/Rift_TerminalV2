// vramModel.ts — shared VRAM math for local (llama-server) models.
//
// Single source of truth for the weights / KV-cache / overhead breakdown used
// by BOTH the live VramEstimator readout and the "Fit to my GPU" solver
// (candidate #789). Keeping the math here — not duplicated in each component —
// is what guarantees the solver's verdict and the readout never disagree.
//
// Pure functions only: no Svelte runes, no I/O. Trivially unit-testable.

import type { LlamaServerConfig, KvCacheType, GgufMeta } from './riftConfig';

// Bytes per stored element for each KV-cache quantization.
export const KV_BYTES: Record<KvCacheType, number> = {
  f32: 4.0,
  f16: 2.0,
  bf16: 2.0,
  q8_0: 1.0,
  q4_0: 0.5,
  q4_1: 0.5625,
  iq4_nl: 0.5,
  q5_0: 0.625,
  q5_1: 0.6875,
};

export const CUDA_OVERHEAD_GB = 0.6; // context, compute buffers, fragmentation
const GIB = 1024 * 1024 * 1024;
// Typical grouped-query-attention ratio (kv heads ≪ query heads). Rough —
// real value comes from GGUF metadata when present.
const GQA_FACTOR = 0.25;

/** Parse total + active params (in billions) from a model id/filename.
 *  "gemma-4-26B-A4B-it-Q4_K_M" → {total: 26, active: 4}
 *  "llama-3.3-70b-instruct"    → {total: 70, active: 70} */
export function parseParams(name: string): { total: number; active: number } | null {
  const lower = name.toLowerCase();
  const nums = [...lower.matchAll(/(\d+(?:\.\d+)?)\s*b/g)].map((m) => parseFloat(m[1]));
  if (nums.length === 0) return null;
  const total = Math.max(...nums);
  const moe = lower.match(/a(\d+(?:\.\d+)?)b/); // active-params marker, e.g. "a4b"
  const active = moe ? parseFloat(moe[1]) : total;
  return { total, active: Math.min(active, total) };
}

/** Approximate bits-per-weight from the quant tag in the filename. */
export function quantBits(name: string): number {
  const n = name.toLowerCase();
  if (n.includes('q2_k')) return 2.6;
  if (n.includes('q3_k')) return 3.4;
  if (n.includes('q4_k') || n.includes('q4_0') || n.includes('q4_1')) return 4.5;
  if (n.includes('q5_k') || n.includes('q5_0') || n.includes('q5_1')) return 5.5;
  if (n.includes('q6_k')) return 6.6;
  if (n.includes('q8')) return 8.5;
  if (n.includes('bf16') || n.includes('f16') || n.includes('fp16')) return 16;
  if (n.includes('f32') || n.includes('fp32')) return 32;
  if (n.includes('q4')) return 4.5;
  return 4.5; // assume ~Q4_K_M when unspecified
}

/** Rough (layers, hidden) for a dense model of `totalB` billion params. */
export function archEstimate(totalB: number): { layers: number; hidden: number } {
  if (totalB <= 4) return { layers: 30, hidden: 3072 };
  if (totalB <= 9) return { layers: 32, hidden: 4096 };
  if (totalB <= 16) return { layers: 40, hidden: 5120 };
  if (totalB <= 35) return { layers: 48, hidden: 5120 };
  if (totalB <= 75) return { layers: 80, hidden: 8192 };
  return { layers: 96, hidden: 12288 };
}

export interface VramBreakdown {
  /** True if we had at least the filename or some GGUF metadata to work from. */
  canEstimate: boolean;
  /** True once at least one measured fact came from the GGUF header. */
  hasMeta: boolean;
  layers: number;
  totalParamsB: number;
  bits: number;
  isMoe: boolean;
  /** Full model weights (all experts), GB. */
  weightsGb: number;
  /** Weights after the configured MoE expert offload, GB. */
  weightsAfterMoeGb: number;
  /** KV cache for the full ctx at all layers, GB. */
  kvGb: number;
  /** Fraction of layers resident on GPU (n_gpu_layers / layers; 1 for auto). */
  gpuFrac: number;
  /** Whether n_gpu_layers is the auto (-1) sentinel. */
  autoFit: boolean;
  weightsOnGpuGb: number;
  kvOnGpuGb: number;
  /** Total estimated VRAM resident on the GPU, GB. */
  totalGb: number;
}

/** Compute the full VRAM breakdown for a config. Mirrors the derived chain that
 *  used to live inside VramEstimator.svelte — the component now calls this so the
 *  two can never drift. */
export function computeVram(
  config: LlamaServerConfig,
  modelName: string,
  meta: GgufMeta | null,
): VramBreakdown {
  const params = parseParams(modelName || config.model_path);
  const heuristicArch = archEstimate(params?.total ?? 8);
  const hasMeta = !!(meta && (meta.n_layers != null || meta.parameter_count != null));

  const layers = meta?.n_layers ?? heuristicArch.layers;
  const hidden = meta?.n_embd ?? heuristicArch.hidden;
  const gqa =
    meta?.n_head_kv != null && meta?.n_head != null && meta.n_head > 0
      ? meta.n_head_kv / meta.n_head
      : GQA_FACTOR;

  const totalParamsB =
    meta?.parameter_count != null ? meta.parameter_count / 1e9 : (params?.total ?? 0);
  const bits = quantBits(modelName || config.model_path);

  const weightsGb = totalParamsB > 0 ? (totalParamsB * 1e9 * (bits / 8)) / GIB : 0;

  const expertOffload = config.cpu_moe
    ? 1
    : config.n_cpu_moe != null
      ? Math.min(1, config.n_cpu_moe / layers)
      : 0;
  const isMoe =
    (meta?.expert_count != null && meta.expert_count > 0) ||
    (!!params && params.active < params.total);
  const expertShare =
    params && params.total > 0 && params.active < params.total
      ? 1 - params.active / params.total
      : 0;
  const weightsAfterMoeGb = weightsGb * (1 - expertShare * expertOffload);

  const gpuFrac = config.n_gpu_layers < 0 ? 1 : Math.min(1, config.n_gpu_layers / layers);
  const autoFit = config.n_gpu_layers < 0;

  const kvGb =
    (config.ctx_size *
      layers *
      hidden *
      gqa *
      ((KV_BYTES[config.cache_type_k] ?? 1.0) + (KV_BYTES[config.cache_type_v] ?? 1.0))) /
    GIB;

  const weightsOnGpuGb = weightsAfterMoeGb * gpuFrac;
  const kvOnGpuGb = kvGb * gpuFrac;
  const totalGb = weightsOnGpuGb + kvOnGpuGb + CUDA_OVERHEAD_GB;

  const canEstimate = !!params || hasMeta;

  return {
    canEstimate,
    hasMeta,
    layers,
    totalParamsB,
    bits,
    isMoe,
    weightsGb,
    weightsAfterMoeGb,
    kvGb,
    gpuFrac,
    autoFit,
    weightsOnGpuGb,
    kvOnGpuGb,
    totalGb,
  };
}

// ---------------------------------------------------------------------------
// "Fit to my GPU" solver (candidate #789)
// ---------------------------------------------------------------------------

export type FitVerdict = 'fits' | 'fits-experts-cpu' | 'fits-partial-layers' | 'wont-fit';

/** The subset of LlamaServerConfig the solver is allowed to change. The solver
 *  never silently touches ctx_size or KV-cache quant — those encode quality
 *  intent; when they're the only way to fit, it says so in `message` instead. */
export interface FitPatch {
  n_gpu_layers: number;
  cpu_moe: boolean;
  n_cpu_moe: number | null;
}

export interface FitResult {
  verdict: FitVerdict;
  /** Config changes to apply (null when nothing can fit). */
  patch: FitPatch | null;
  /** Estimated VRAM the patched config would use, GB (null when wont-fit). */
  estGb: number | null;
  /** Free VRAM headroom the patched config leaves, GB (null when wont-fit). */
  freeGb: number | null;
  /** Human-readable one-liner for the verdict chip. */
  message: string;
}

/** Build a candidate config = base with the solver-owned fields overridden. */
function withPatch(base: LlamaServerConfig, patch: FitPatch): LlamaServerConfig {
  return { ...base, ...patch };
}

/** Classify whether ONE given config fits `vramGb` — no searching. Powers the
 *  live pre-launch verdict chip, which reflects the current config (including
 *  hand edits) at all times, independent of whether the solver has been run. */
export function classifyConfig(
  config: LlamaServerConfig,
  modelName: string,
  meta: GgufMeta | null,
  vramGb: number,
  headroomGb = 0.8,
): FitResult {
  const b = computeVram(config, modelName, meta);
  if (!b.canEstimate || b.totalParamsB <= 0) {
    return {
      verdict: 'wont-fit',
      patch: null,
      estGb: null,
      freeGb: null,
      message: 'Set a GGUF path or a name with params + quant to estimate fit.',
    };
  }
  const fmt = (n: number) => n.toFixed(1);
  const estGb = b.totalGb;
  const freeGb = vramGb - estGb;
  const fits = estGb <= vramGb - headroomGb;
  const expertsOnCpu = config.cpu_moe || config.n_cpu_moe != null;
  const partialLayers = !b.autoFit && config.n_gpu_layers >= 0 && b.gpuFrac < 1;

  if (!fits) {
    return {
      verdict: 'wont-fit',
      patch: null,
      estGb,
      freeGb,
      message: `Over budget — ~${fmt(estGb)}GB needed vs ${fmt(vramGb)}GB. Click "Fit to my GPU".`,
    };
  }
  if (partialLayers) {
    return {
      verdict: 'fits-partial-layers',
      patch: null,
      estGb,
      freeGb,
      message: `Fits — ${config.n_gpu_layers}/${b.layers} layers on GPU (rest on CPU), ~${fmt(freeGb)}GB free.`,
    };
  }
  if (expertsOnCpu && b.isMoe) {
    return {
      verdict: 'fits-experts-cpu',
      patch: null,
      estGb,
      freeGb,
      message: `Fits with experts on CPU — ~${fmt(estGb)}GB used, ~${fmt(freeGb)}GB free.`,
    };
  }
  return {
    verdict: 'fits',
    patch: null,
    estGb,
    freeGb,
    message: `Fits fully on GPU — ~${fmt(estGb)}GB used, ~${fmt(freeGb)}GB free.`,
  };
}

/** Solve for the offload layout that fits `vramGb` with `headroomGb` to spare.
 *
 *  Ladder (priority order, per candidate #789):
 *   1. all layers on GPU, no expert offload          → 'fits' (green)
 *   2. MoE: raise n_cpu_moe until it fits            → 'fits-experts-cpu' (amber)
 *   3. MoE: all experts on CPU (cpu_moe)             → 'fits-experts-cpu' (amber)
 *   4. reduce GPU layers (rest run on CPU, slower)   → 'fits-partial-layers' (amber)
 *   5. nothing fits                                  → 'wont-fit' (red)
 *
 *  ctx_size and KV-cache quant are left untouched; step 5's message points at
 *  them as the remaining levers. */
export function fitToGpu(
  base: LlamaServerConfig,
  modelName: string,
  meta: GgufMeta | null,
  vramGb: number,
  headroomGb = 0.8,
): FitResult {
  const probe = computeVram(base, modelName, meta);
  if (!probe.canEstimate || probe.totalParamsB <= 0) {
    return {
      verdict: 'wont-fit',
      patch: null,
      estGb: null,
      freeGb: null,
      message: 'Set a GGUF path or a name with params + quant to compute a fit.',
    };
  }

  const budget = vramGb - headroomGb;
  const { layers, isMoe } = probe;
  const fmt = (n: number) => n.toFixed(1);

  const est = (patch: FitPatch) => computeVram(withPatch(base, patch), modelName, meta).totalGb;

  // Step 1 — all layers on GPU, no expert offload.
  const allOnGpu: FitPatch = { n_gpu_layers: 99, cpu_moe: false, n_cpu_moe: null };
  const allGpuGb = est(allOnGpu);
  if (allGpuGb <= budget) {
    return {
      verdict: 'fits',
      patch: allOnGpu,
      estGb: allGpuGb,
      freeGb: vramGb - allGpuGb,
      message: `Fits fully on GPU — ~${fmt(allGpuGb)}GB used, ~${fmt(vramGb - allGpuGb)}GB free.`,
    };
  }

  // Steps 2-3 — MoE: push experts to CPU. Keep all layers on GPU; raise the
  // count of layers whose experts live on CPU until it fits, then fall to all.
  if (isMoe) {
    for (let k = 1; k < layers; k++) {
      const patch: FitPatch = { n_gpu_layers: 99, cpu_moe: false, n_cpu_moe: k };
      const g = est(patch);
      if (g <= budget) {
        return {
          verdict: 'fits-experts-cpu',
          patch,
          estGb: g,
          freeGb: vramGb - g,
          message: `Fits with experts of ${k}/${layers} layers on CPU — ~${fmt(g)}GB used, ~${fmt(vramGb - g)}GB free.`,
        };
      }
    }
    const allExperts: FitPatch = { n_gpu_layers: 99, cpu_moe: true, n_cpu_moe: null };
    const allExpGb = est(allExperts);
    if (allExpGb <= budget) {
      return {
        verdict: 'fits-experts-cpu',
        patch: allExperts,
        estGb: allExpGb,
        freeGb: vramGb - allExpGb,
        message: `Fits with ALL experts on CPU — ~${fmt(allExpGb)}GB used, ~${fmt(vramGb - allExpGb)}GB free.`,
      };
    }
  }

  // Step 4 — reduce GPU layers (remainder runs on CPU, slower). For MoE keep all
  // experts on CPU first since that frees the most VRAM per layer kept on GPU.
  const cpuMoeBase = isMoe;
  for (let n = layers - 1; n >= 1; n--) {
    const patch: FitPatch = { n_gpu_layers: n, cpu_moe: cpuMoeBase, n_cpu_moe: null };
    const g = est(patch);
    if (g <= budget) {
      const expertsNote = cpuMoeBase ? ', all experts on CPU' : '';
      return {
        verdict: 'fits-partial-layers',
        patch,
        estGb: g,
        freeGb: vramGb - g,
        message: `Fits with ${n}/${layers} layers on GPU${expertsNote} (rest on CPU — slower) — ~${fmt(g)}GB used.`,
      };
    }
  }

  // Step 5 — nothing fits even at minimum residency.
  return {
    verdict: 'wont-fit',
    patch: null,
    estGb: null,
    freeGb: null,
    message: `Won't fit in ${fmt(vramGb)}GB even with all experts + most layers on CPU. Reduce context size or pick a smaller quant.`,
  };
}
