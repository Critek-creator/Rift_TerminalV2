<script lang="ts">
  import type { LlamaServerConfig, KvCacheType } from './riftConfig';

  interface Props {
    config: LlamaServerConfig;
    gpuVramGb?: number;
  }

  let { config, gpuVramGb = 16 }: Props = $props();

  const KV_BYTES_PER_TOKEN: Record<KvCacheType, number> = {
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

  const CUDA_OVERHEAD_GB = 0.5;
  const ASSUMED_NUM_LAYERS = 32;

  let kvBytesK = $derived(KV_BYTES_PER_TOKEN[config.cache_type_k] ?? 1.0);
  let kvBytesV = $derived(KV_BYTES_PER_TOKEN[config.cache_type_v] ?? 1.0);

  let kvCacheGb = $derived(
    (config.ctx_size * (kvBytesK + kvBytesV) * ASSUMED_NUM_LAYERS) / (1024 * 1024 * 1024),
  );

  let totalEstGb = $derived(kvCacheGb + CUDA_OVERHEAD_GB);

  let pct = $derived(Math.min(100, (totalEstGb / gpuVramGb) * 100));

  let barColor = $derived(
    pct > 90 ? 'var(--term-red)' : pct > 70 ? 'var(--amber-bright)' : 'var(--term-green)',
  );
</script>

<div class="vram-est" title="Estimated VRAM: KV cache {kvCacheGb.toFixed(1)}GB + {CUDA_OVERHEAD_GB}GB CUDA overhead (model weights not included — depends on GGUF quantization)">
  <div class="label">VRAM est.</div>
  <div class="bar-bg">
    <div class="bar-fill" style="width: {pct.toFixed(0)}%; background: {barColor}"></div>
  </div>
  <div class="value">~{totalEstGb.toFixed(1)} / {gpuVramGb}GB</div>
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
</style>
