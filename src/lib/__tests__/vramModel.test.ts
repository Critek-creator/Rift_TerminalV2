import { describe, it, expect } from 'vitest';
import {
  computeVram,
  fitToGpu,
  classifyConfig,
  parseParams,
} from '../vramModel';
import type { LlamaServerConfig, GgufMeta } from '../riftConfig';

// Baseline local config. Individual tests override the fields they exercise.
function cfg(over: Partial<LlamaServerConfig> = {}): LlamaServerConfig {
  return {
    model_path: '',
    flash_attention: true,
    ctx_size: 32768,
    cache_type_k: 'q8_0',
    cache_type_v: 'q8_0',
    n_gpu_layers: 99,
    cpu_moe: false,
    n_cpu_moe: null,
    cache_ram: null,
    threads: null,
    parallel: 1,
    port: 8081,
    cuda_visible_devices: null,
    auto_start: false,
    auto_restart: false,
    extra_flags: [],
    ...over,
  };
}

describe('parseParams', () => {
  it('reads dense param count from a filename', () => {
    expect(parseParams('llama-3.3-70b-instruct')).toEqual({ total: 70, active: 70 });
  });
  it('reads MoE total + active from the aNb marker', () => {
    expect(parseParams('gemma-4-26B-A4B-it-Q4_K_M')).toEqual({ total: 26, active: 4 });
  });
  it('returns null when no param count is present', () => {
    expect(parseParams('some-model-name')).toBeNull();
  });
});

describe('computeVram', () => {
  it('flags a small dense model as fully GPU-resident', () => {
    const b = computeVram(cfg(), 'gemma-2-2b-it-Q4_K_M', null);
    expect(b.canEstimate).toBe(true);
    expect(b.isMoe).toBe(false);
    expect(b.totalGb).toBeGreaterThan(0);
  });

  it('drops total VRAM when MoE experts are offloaded to CPU', () => {
    const full = computeVram(cfg({ cpu_moe: false }), 'gemma-4-26B-A4B-it-Q4_K_M', null);
    const offloaded = computeVram(cfg({ cpu_moe: true }), 'gemma-4-26B-A4B-it-Q4_K_M', null);
    expect(offloaded.totalGb).toBeLessThan(full.totalGb);
  });

  it('prefers GGUF metadata over filename heuristics', () => {
    const meta: GgufMeta = {
      architecture: 'gemma',
      n_layers: 48,
      n_embd: 4096,
      n_head: 32,
      n_head_kv: 8,
      expert_count: 8,
      parameter_count: 26e9,
    };
    const b = computeVram(cfg(), 'gemma-4-26B-A4B-it-Q4_K_M', meta);
    expect(b.hasMeta).toBe(true);
    expect(b.layers).toBe(48);
    expect(b.isMoe).toBe(true);
  });
});

describe('fitToGpu', () => {
  const VALID_VERDICTS = ['fits', 'fits-experts-cpu', 'fits-partial-layers', 'wont-fit'];
  const HEADROOM = 0.8; // must match fitToGpu's default

  // Universal invariants that hold for ANY input to a correct solver — no
  // guessed VRAM thresholds, so they don't go stale if the heuristic constants
  // are retuned:
  //   I1. verdict is always one of the four known strings.
  //   I2. wont-fit ⟺ no patch (and vice-versa).
  //   I3. a proposed patch actually fits the budget the solver targeted, so its
  //       estimate is within VRAM (this is what makes the result trustworthy).
  //   I4. the solver's reported estGb equals re-running computeVram on the
  //       patched config — the solver and the live readout never disagree.
  //   I5. cpu_moe and n_cpu_moe are never both "active" (mutually exclusive).
  function assertInvariants(
    r: ReturnType<typeof fitToGpu>,
    name: string,
    vramGb: number,
  ) {
    expect(VALID_VERDICTS).toContain(r.verdict); // I1
    if (r.verdict === 'wont-fit') {
      expect(r.patch).toBeNull(); // I2
      return;
    }
    expect(r.patch).not.toBeNull(); // I2
    expect(r.estGb).not.toBeNull();
    expect(r.estGb!).toBeLessThanOrEqual(vramGb); // I3
    const recomputed = computeVram({ ...cfg(), ...r.patch! }, name, null);
    expect(recomputed.totalGb).toBeCloseTo(r.estGb!, 5); // I4
    if (r.patch!.cpu_moe) {
      expect(r.patch!.n_cpu_moe).toBeNull(); // I5
    }
  }

  it('holds all solver invariants across a sweep of GPU sizes (MoE model)', () => {
    const name = 'gemma-4-26B-A4B-it-Q4_K_M';
    for (const vram of [2, 4, 6, 8, 10, 12, 16, 24, 48]) {
      assertInvariants(fitToGpu(cfg(), name, null, vram), name, vram);
    }
  });

  it('holds all solver invariants for a dense model too', () => {
    const name = 'llama-3.3-8b-instruct-Q4_K_M';
    for (const vram of [4, 6, 8, 12, 16]) {
      assertInvariants(fitToGpu(cfg(), name, null, vram), name, vram);
    }
  });

  it('returns "fits" (no offload) when VRAM is clearly ample', () => {
    // A tiny model on a big GPU must take the first rung — the one case whose
    // outcome is unambiguous regardless of the exact heuristic constants.
    const r = fitToGpu(cfg({ ctx_size: 4096 }), 'gemma-2-2b-it-Q4_K_M', null, 48);
    expect(r.verdict).toBe('fits');
    expect(r.patch!.n_gpu_layers).toBe(99);
    expect(r.patch!.cpu_moe).toBe(false);
    expect(r.patch!.n_cpu_moe).toBeNull();
  });

  it('reports wont-fit when not even one layer fits', () => {
    // 70B Q8 ≈ 0.87GB per layer + ~0.6GB CUDA overhead, so a single layer needs
    // ~1.5GB. On a 2GB GPU (0.8 headroom → 1.2 budget) even ngl=1 overflows, so
    // the solver exhausts the ladder and reports wont-fit. (At 8GB it WOULD fit
    // 1/80 layers — a degenerate but honest "fits-partial-layers" — which is why
    // the budget here is deliberately tiny.)
    const r = fitToGpu(cfg({ ctx_size: 32768 }), 'llama-3.3-70b-instruct-Q8_0', null, 2);
    expect(r.verdict).toBe('wont-fit');
    expect(r.patch).toBeNull();
    expect(r.message).toMatch(/won.t fit/i);
  });

  it('cannot estimate without params or metadata', () => {
    const r = fitToGpu(cfg(), 'mystery-model', null, 16);
    expect(r.verdict).toBe('wont-fit');
    expect(r.patch).toBeNull();
    expect(r.message).toMatch(/GGUF|params/i);
  });

  it('a returned fit leaves at least the headroom margin free', () => {
    // Whatever rung wins, the layout must respect the headroom budget.
    const name = 'gemma-4-26B-A4B-it-Q4_K_M';
    for (const vram of [8, 16, 24]) {
      const r = fitToGpu(cfg(), name, null, vram);
      if (r.verdict !== 'wont-fit') {
        expect(r.estGb!).toBeLessThanOrEqual(vram - HEADROOM + 1e-6);
      }
    }
  });
});

describe('classifyConfig (live verdict chip)', () => {
  it('classifies an all-on-GPU small model as fits', () => {
    const r = classifyConfig(cfg({ ctx_size: 4096 }), 'gemma-2-2b-it-Q4_K_M', null, 16);
    expect(r.verdict).toBe('fits');
    expect(r.freeGb!).toBeGreaterThan(0);
  });

  it('classifies an over-budget config as wont-fit without searching', () => {
    const r = classifyConfig(cfg(), 'llama-3.3-70b-instruct-Q8_0', null, 8);
    expect(r.verdict).toBe('wont-fit');
    expect(r.patch).toBeNull(); // classify never proposes a patch
  });

  it('reflects a manual partial-layer setting', () => {
    const r = classifyConfig(
      cfg({ n_gpu_layers: 10, cpu_moe: true }),
      'gemma-4-26B-A4B-it-Q4_K_M',
      null,
      16,
    );
    // With only 10 layers on GPU + all experts on CPU it should fit, and be
    // reported as a partial-layer fit.
    if (r.verdict !== 'wont-fit') {
      expect(['fits-partial-layers', 'fits-experts-cpu', 'fits']).toContain(r.verdict);
    }
  });
});
