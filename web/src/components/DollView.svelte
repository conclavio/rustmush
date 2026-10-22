<script lang="ts">
  // The armour doll in the cockpit tab (doc 22 cut U.4): the pilot's own
  // unit's look grid — the same file the terminal draws its doll from — one
  // square a cell, each coloured by what is left of its part (a facet's
  // armour, or its section's internals), touching parts in the same state a
  // lighter shade apart. Drawn from the frame's own `unit_type` and
  // `sections`, nothing else.
  import type { CockpitSection } from "$lib/cockpit";
  import { dollColour, dollCondition, dollTones, type Doll } from "$lib/look";

  let { doll, sections, label }: { doll: Doll; sections: CockpitSection[]; label: string } =
    $props();

  const tones = $derived(dollTones(doll));
  const condition = $derived(dollCondition(doll, sections));
  /** Each drawn cell: where, and its colour. */
  const squares = $derived(
    doll.cells.flatMap((part, i) => {
      if (part === null) return [];
      const left = condition[part];
      const fill = left ? dollColour(left[0], left[1], tones[part] ?? 0) : dollColour(0, 0, 0);
      const id = doll.parts[part]?.section ?? "?";
      return [{ x: i % doll.width, y: Math.floor(i / doll.width), fill, id }];
    }),
  );
</script>

<svg
  class="doll"
  viewBox="0 0 {doll.width} {doll.height}"
  role="img"
  aria-label="{label} armour"
  shape-rendering="crispEdges"
>
  {#each squares as sq (sq.y * doll.width + sq.x)}
    <rect x={sq.x} y={sq.y} width="1" height="1" fill={sq.fill}><title>{sq.id}</title></rect>
  {/each}
</svg>

<style>
  .doll {
    display: block;
    width: 100%;
    max-width: 192px;
    height: auto;
    margin: 0.2rem auto 0.4rem;
  }
</style>
