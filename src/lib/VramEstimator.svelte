<script lang="ts">
  import type { LlamaServerConfig, GgufMeta } from './riftConfig';
  import { computeVram, CUDA_OVERHEAD_GB } from './vramModel';

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

  // --- Estimate --------------------------------------------------------------
  // All weights/KV/overhead math lives in vramModel.ts so this readout and the
  // "Fit to my GPU" solver compute identical numbers (candidate #789).
  let b = $derived(computeVram(config, modelName, meta));

  // Destructured aliases keep the template + tooltip readable.
  let layers = $derived(b.layers);
  let isMoe = $derived(b.isMoe);
  let bits = $derived(b.bits);
  let totalParamsB = $derived(b.totalParamsB);
  let hasMeta = $derived(b.hasMeta);
  let canEstimate = $derived(b.canEstimate);
  let autoFit = $derived(b.autoFit);
  let weightsOnGpu = $derived(b.weightsOnGpuGb);
  let kvOnGpu = $derived(b.kvOnGpuGb);
  let totalEstGb = $derived(b.totalGb);

  // True when any expert weights are pushed to CPU (drives the tooltip note).
  let expertsOffloaded = $derived(config.cpu_moe || config.n_cpu_moe != null);

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
        `${expertsOffloaded && isMoe ? ` (experts offloaded to CPU)` : ''}\n` +
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
