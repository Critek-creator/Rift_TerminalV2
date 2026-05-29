<script lang="ts">
  import type { LlamaServerConfig, KvCacheType, GgufMeta } from './riftConfig';

  interface Props {
    config: LlamaServerConfig;
    /** Model id / filename — used to estimate weights (params + quant) when
     *  no GGUF metadata is available. */
    modelName?: string;
    /** Measured architecture facts from the GGUF header (gguf_inspect). When
     *  present these replace the filename/arch-table heuristics field by field. */
    meta?: GgufMeta | null;
    gpuVramGb?: number;
  }

  let { config, modelName = '', meta = null, gpuVramGb = 16 }: Props = $props();

  // Bytes per stored element for each KV-cache quantization.
  const KV_BYTES: Record<KvCacheType, number> = {
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

  const CUDA_OVERHEAD_GB = 0.6; // context, compute buffers, fragmentation
  const GIB = 1024 * 1024 * 1024;
  // Typical grouped-query-attention ratio (kv heads ≪ query heads). Rough —
  // real value comes from GGUF metadata we don't have at config-edit time.
  const GQA_FACTOR = 0.25;

  // --- Heuristic model introspection (no GGUF metadata available here) -------

  /** Parse total + active params (in billions) from a model id/filename.
   *  "gemma-4-26B-A4B-it-Q4_K_M" → {total: 26, active: 4}
   *  "llama-3.3-70b-instruct"    → {total: 70, active: 70} */
  function parseParams(name: string): { total: number; active: number } | null {
    const lower = name.toLowerCase();
    const nums = [...lower.matchAll(/(\d+(?:\.\d+)?)\s*b/g)].map((m) => parseFloat(m[1]));
    if (nums.length === 0) return null;
    const total = Math.max(...nums);
    const moe = lower.match(/a(\d+(?:\.\d+)?)b/); // active-params marker, e.g. "a4b"
    const active = moe ? parseFloat(moe[1]) : total;
    return { total, active: Math.min(active, total) };
  }

  /** Approximate bits-per-weight from the quant tag in the filename. */
  function quantBits(name: string): number {
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
  function archEstimate(totalB: number): { layers: number; hidden: number } {
    if (totalB <= 4) return { layers: 30, hidden: 3072 };
    if (totalB <= 9) return { layers: 32, hidden: 4096 };
    if (totalB <= 16) return { layers: 40, hidden: 5120 };
    if (totalB <= 35) return { layers: 48, hidden: 5120 };
    if (totalB <= 75) return { layers: 80, hidden: 8192 };
    return { layers: 96, hidden: 12288 };
  }

  // --- Estimate --------------------------------------------------------------

  let params = $derived(parseParams(modelName || config.model_path));
  let heuristicArch = $derived(archEstimate(params?.total ?? 8));

  // True once we have at least one measured fact from the GGUF header. Drives
  // both the per-field source selection below and the "measured vs estimated"
  // tooltip language.
  let hasMeta = $derived(!!(meta && (meta.n_layers != null || meta.parameter_count != null)));

  // Prefer measured GGUF values; fall back to the filename/arch-table heuristic
  // per field so a partial header still sharpens whatever it covers.
  let layers = $derived(meta?.n_layers ?? heuristicArch.layers);
  let hidden = $derived(meta?.n_embd ?? heuristicArch.hidden);
  // Exact grouped-query-attention ratio when both head counts are known,
  // otherwise the rough 0.25 estimate.
  let gqa = $derived(
    meta?.n_head_kv != null && meta?.n_head != null && meta.n_head > 0
      ? meta.n_head_kv / meta.n_head
      : GQA_FACTOR,
  );

  // Total params (billions): the GGUF's own count when present, else parsed
  // from the filename.
  let totalParamsB = $derived(
    meta?.parameter_count != null ? meta.parameter_count / 1e9 : (params?.total ?? 0),
  );
  let bits = $derived(quantBits(modelName || config.model_path));

  // Total model weights loaded (all experts included).
  let weightsGb = $derived(totalParamsB > 0 ? (totalParamsB * 1e9 * (bits / 8)) / GIB : 0);

  // Fraction of expert weights pushed to CPU. cpu_moe = all; n_cpu_moe = first
  // N layers' experts; neither = none. Only meaningful for MoE models.
  let expertOffload = $derived(
    config.cpu_moe ? 1 : config.n_cpu_moe != null ? Math.min(1, config.n_cpu_moe / layers) : 0,
  );
  let isMoe = $derived(
    (meta?.expert_count != null && meta.expert_count > 0) ||
      (!!params && params.active < params.total),
  );
  // Expert share of weights ≈ everything beyond the active path. Needs the
  // active/total split, which only the filename carries — a GGUF that reports
  // expert_count but no filename "aNb" marker still flags MoE but can't size
  // the offload saving, so expertShare stays 0 (no double-counting).
  let expertShare = $derived(
    params && params.total > 0 && params.active < params.total
      ? 1 - params.active / params.total
      : 0,
  );
  let weightsAfterMoe = $derived(weightsGb * (1 - expertShare * expertOffload));

  // GPU-layer fraction. -1 (auto) or ≥ layer count ⇒ fully resident.
  let gpuFrac = $derived(config.n_gpu_layers < 0 ? 1 : Math.min(1, config.n_gpu_layers / layers));
  let autoFit = $derived(config.n_gpu_layers < 0);

  // KV cache: ctx × layers × hidden × gqa × (K bytes + V bytes).
  let kvGb = $derived(
    (config.ctx_size *
      layers *
      hidden *
      gqa *
      ((KV_BYTES[config.cache_type_k] ?? 1.0) + (KV_BYTES[config.cache_type_v] ?? 1.0))) /
      GIB,
  );

  let weightsOnGpu = $derived(weightsAfterMoe * gpuFrac);
  let kvOnGpu = $derived(kvGb * gpuFrac);
  let totalEstGb = $derived(weightsOnGpu + kvOnGpu + CUDA_OVERHEAD_GB);

  // We can produce an estimate from either a parsed filename or GGUF metadata.
  let canEstimate = $derived(!!params || hasMeta);

  let pct = $derived(Math.min(100, (totalEstGb / gpuVramGb) * 100));
  let barColor = $derived(
    pct > 90 ? 'var(--term-red)' : pct > 70 ? 'var(--amber-bright)' : 'var(--term-green)',
  );

  let sizeLabel = $derived(totalParamsB > 0 ? `~${totalParamsB.toFixed(totalParamsB < 10 ? 1 : 0)}B` : '');

  let tooltip = $derived(
    canEstimate
      ? `${hasMeta ? 'Measured from GGUF' : 'Rough estimate'} (${sizeLabel}${isMoe ? ', MoE' : ''}, ` +
        `${bits}-bit):\n` +
        `• weights on GPU ~${weightsOnGpu.toFixed(1)}GB` +
        `${expertOffload > 0 && isMoe ? ` (experts offloaded to CPU)` : ''}\n` +
        `• KV cache ~${kvOnGpu.toFixed(1)}GB (${config.ctx_size} ctx, ${layers} layers)\n` +
        `• CUDA overhead ~${CUDA_OVERHEAD_GB}GB\n` +
        `${autoFit ? 'GPU layers: auto-fit' : `GPU layers: ${config.n_gpu_layers}/${layers}`}\n` +
        `${hasMeta ? 'Architecture read from the GGUF header.' : 'Approximate — set a model path to read exact GGUF metadata.'}`
      : `Set a model path (GGUF) or a name with params + quant (e.g. "...-26B-...-Q4_K_M.gguf") for a full estimate. Showing KV cache + overhead only.`,
  );
</script>

<div class="vram-est" title={tooltip}>
  <div class="label">VRAM est.</div>
  <div class="bar-bg">
    <div class="bar-fill" style="width: {pct.toFixed(0)}%; background: {barColor}"></div>
  </div>
  <div class="value">
    ~{totalEstGb.toFixed(1)} / {gpuVramGb}GB{#if autoFit}<span class="tag"> auto</span>{/if}
  </div>
</div>

<style>
  .vram-est {
    display: flex;
    align-items: center;
    gap: 6px;
    font-family: 'JetBrains Mono', monospace;
    font-size: 9px;
    margin-top: 8px;
  }

  .label {
    color: var(--amber-faint, #A87830);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    flex-shrink: 0;
  }

  .bar-bg {
    flex: 1;
    height: 6px;
    background: rgba(0, 0, 0, 0.4);
    border: 1px solid rgba(168, 120, 48, 0.2);
    border-radius: var(--radius-sm);
    overflow: hidden;
  }

  .bar-fill {
    height: 100%;
    border-radius: var(--radius-sm);
    transition: width var(--duration-med) var(--ease-out);
  }

  .value {
    color: var(--term-white, #E8E4D8);
    flex-shrink: 0;
    min-width: 70px;
    text-align: right;
  }

  .tag {
    color: var(--amber-faint, #A87830);
  }
</style>
