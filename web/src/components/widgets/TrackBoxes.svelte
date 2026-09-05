<script lang="ts">
  // A grouped track (interface plan I6, the "two-track" monitor): boxes in
  // rows of three with the deepest level named and the frame's detail —
  // the wound penalty, the "next box" warning — beside it. Empty tracks
  // render nothing but the label.
  import { boxGlyph } from "$lib/stores/vitals.svelte";
  import { groupBoxes } from "$lib/widgets";

  let {
    label,
    boxes,
    level = null,
    state = "ok",
    detail = null,
  }: {
    label: string;
    boxes: (string | null)[];
    level?: string | null;
    state?: string;
    detail?: string | null;
  } = $props();

  const groups = $derived(groupBoxes(boxes, 3));
</script>

<div class="track state-{state}" title={detail ?? label}>
  <span class="label">{label}</span>
  <span class="groups" aria-hidden="true">
    {#each groups as g, gi (gi)}
      <span class="group">
        {#each g as b, i (i)}
          <span class="box" class:filled={b !== null}>{boxGlyph(b)}</span>
        {/each}
      </span>
    {/each}
  </span>
  {#if level}<span class="level">{level}</span>{/if}
  {#if detail}<span class="detail">{detail}</span>{/if}
</div>

<style>
  .track {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.5rem;
    font-family: var(--font-mono);
    font-size: 0.9rem;
  }

  .label {
    min-width: 6rem;
    color: var(--fg-muted);
  }

  .groups {
    display: inline-flex;
    gap: 0.4rem;
  }

  .group {
    display: inline-flex;
    gap: 2px;
  }

  .box {
    display: inline-block;
    width: 1.1rem;
    height: 1.2rem;
    line-height: 1.2rem;
    text-align: center;
    border: 1px solid var(--border);
    border-radius: 3px;
    font-size: 0.8rem;
  }

  .box.filled {
    border-color: currentcolor;
  }

  .level {
    font-weight: 600;
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
