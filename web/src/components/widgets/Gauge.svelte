<script lang="ts">
  // A bounded gauge (interface plan I6): `▰▰▰▱▱▱▱▱` plus digits, coloured by
  // the frame's state. Clamps; never knows what it measures.
  import { GAUGE_CELLS, GAUGE_OFF, GAUGE_ON, gaugeFill } from "$lib/widgets";

  let {
    label,
    current,
    min,
    max,
    state = "ok",
    detail = null,
  }: {
    label: string;
    current: number;
    min: number;
    max: number;
    state?: string;
    detail?: string | null;
  } = $props();

  const filled = $derived(gaugeFill(current, min, max, GAUGE_CELLS));
</script>

<div class="gauge state-{state}" title={detail ?? label}>
  <span class="label">{label}</span>
  <span class="bar" aria-hidden="true">{GAUGE_ON.repeat(filled)}{GAUGE_OFF.repeat(Math.max(0, GAUGE_CELLS - filled))}</span>
  <span class="num">{current}/{Math.max(min, max)}</span>
  {#if detail}<span class="detail">{detail}</span>{/if}
</div>

<style>
  .gauge {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
    font-family: var(--font-mono);
    font-size: 0.9rem;
  }

  .label {
    min-width: 6rem;
    color: var(--fg-muted);
  }

  .bar {
    letter-spacing: 0.05em;
  }

  .num {
    font-variant-numeric: tabular-nums;
  }

  .detail {
    color: var(--fg-faint);
    font-size: 0.8rem;
  }

  .state-warn {
    color: var(--warn);
  }

  .state-crit,
  .state-down {
    color: var(--error);
  }
</style>
