<script lang="ts">
  // The section map (interface cut I8): one block per section — the glyph
  // from the section's `state`, its id, and the ruleset's own condition
  // word printed verbatim — with one gauge per cell addressed under it.
  //
  // Rule 1 (docs/design/10b §4, "rulesets judge, clients render"): the
  // glyph and every colour come from a `state` word alone — the four the
  // widgets already know, with anything else falling back to neutral. A
  // `condition` is the ruleset's own judgement: it is shown as text and is
  // never mapped to a colour, an icon or an ordering here.
  //
  // `compact` is the unit layout's inline list (facet-scope cells only);
  // the full map is the damage layout's, with every cell under the section
  // and its address as the gauge's detail.
  import { cellsIn, facetCells, type UnitCell, type UnitSection } from "$lib/stores/unit.svelte";
  import { conditionGlyph } from "$lib/widgets";

  import Gauge from "./widgets/Gauge.svelte";

  let {
    sections,
    cells,
    compact = false,
  }: { sections: UnitSection[]; cells: UnitCell[]; compact?: boolean } = $props();
</script>

<div class="map" class:compact>
  {#if sections.length === 0}
    <p class="muted">Nothing here.</p>
  {/if}
  {#each sections as s (s.id)}
    <section class="sec" aria-label={s.id}>
      <h5 class="state-{s.state}">
        <span class="glyph" aria-hidden="true">{conditionGlyph(s.state)}</span>
        <span class="id">{s.id}</span>
        <span class="cond">{s.condition}</span>
      </h5>
      {#each compact ? facetCells(cells, s.id) : cellsIn(cells, "section", s.id) as c (c.addr)}
        <Gauge
          label={c.label}
          current={c.current}
          min={c.min}
          max={c.max}
          state={c.state}
          detail={compact ? null : c.addr}
        />
      {/each}
    </section>
  {/each}
</div>

<style>
  .map {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .map.compact {
    gap: 0.3rem;
  }

  .sec {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
  }

  h5 {
    margin: 0;
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
    font-family: var(--font-mono);
    font-size: 0.85rem;
    font-weight: 600;
  }

  .glyph {
    width: 1rem;
    text-align: center;
  }

  .cond {
    color: var(--fg-muted);
    font-weight: 400;
    font-size: 0.78rem;
  }

  .state-warn {
    color: var(--warn);
  }

  .state-crit,
  .state-down {
    color: var(--error);
  }

  .muted {
    color: var(--fg-muted);
    font-size: 0.85rem;
    margin: 0;
  }
</style>
