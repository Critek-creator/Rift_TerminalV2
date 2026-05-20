<script lang="ts">
  interface Props {
    data: number[];
  }

  let { data }: Props = $props();

  const WIDTH = 100;
  const HEIGHT = 16;
  const PADDING_Y = 2;

  const total = $derived(data.reduce((a, b) => a + b, 0));

  const chart = $derived.by(() => {
    const max = Math.max(1, ...data);
    const usable = HEIGHT - PADDING_Y * 2;

    let peakIdx = 0;
    let peakVal = data[0] ?? 0;

    const pts: string[] = [];
    for (let i = 0; i < data.length; i++) {
      const v = data[i] ?? 0;
      if (v > peakVal) {
        peakVal = v;
        peakIdx = i;
      }
      const x = (i / (data.length - 1)) * WIDTH;
      const y = HEIGHT - PADDING_Y - (v / max) * usable;
      pts.push(`${x.toFixed(1)},${y.toFixed(1)}`);
    }

    const peakX = (peakIdx / (data.length - 1)) * WIDTH;
    const peakY = HEIGHT - PADDING_Y - (peakVal / max) * usable;

    return {
      points: pts.join(' '),
      peakX: +peakX.toFixed(1),
      peakY: +peakY.toFixed(1),
      hasPeak: peakVal > 0,
    };
  });
</script>

<div class="sparkline-wrap" title={total > 0 ? `${total} events/min` : 'idle'}>
  <svg
    class="sparkline"
    viewBox="0 0 {WIDTH} {HEIGHT}"
    aria-hidden="true"
  >
    {#if total === 0}
      <line x1="0" y1={HEIGHT / 2} x2={WIDTH} y2={HEIGHT / 2}
        stroke="var(--amber-faint, #A87830)" stroke-width="1" stroke-dasharray="3,3" opacity="0.5" />
    {:else}
      <polyline
        points={chart.points}
        fill="none"
        stroke="var(--amber-bright, #FFC840)"
        stroke-width="1.5"
        stroke-linejoin="round"
        stroke-linecap="round"
      />
      {#if chart.hasPeak}
        <circle
          cx={chart.peakX}
          cy={chart.peakY}
          r="2"
          fill="var(--term-green, #4FE855)"
        />
      {/if}
    {/if}
  </svg>
</div>

<style>
  .sparkline-wrap {
    display: inline-flex;
    align-items: center;
    flex-shrink: 0;
    height: 18px;
    max-height: 18px;
    overflow: hidden;
  }
  .sparkline {
    display: block;
    width: 80px;
    height: 14px;
    background: var(--bg-amber-tint);
    border: 1px solid var(--border-amber-tint);
    border-radius: 3px;
  }
</style>
