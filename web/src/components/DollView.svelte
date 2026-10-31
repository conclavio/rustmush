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
  /** Each drawn cell: where, its colour, and its tooltip label. */
  const squares = $derived(
    doll.cells.flatMap((part, i) => {
      if (part === null) return [];
      const left = condition[part];
      const fill = left ? dollColour(left[0], left[1], tones[part] ?? 0) : dollColour(0, 0, 0);
      const spec = doll.parts[part];
      const baseId = spec ? (spec.facet ? `${spec.section} (${spec.facet})` : spec.section) : "?";
      const id = left ? `${baseId}: ${left[0]}/${left[1]}` : baseId;
      return [{ x: i % doll.width, y: Math.floor(i / doll.width), fill, id }];
    }),
  );

  /** Dark seam lines between distinct parts and around the silhouette, matching
   * `mush-pixel`'s `doll.rs` so turret, barrel, engine, hull facets, and tracks
   * read as distinct components rather than a single flat block. */
  const edgePath = $derived.by(() => {
    const at = (x: number, y: number): number | null =>
      x >= 0 && y >= 0 && x < doll.width && y < doll.height
        ? (doll.cells[y * doll.width + x] ?? null)
        : null;
    const segs: string[] = [];
    for (let y = 0; y <= doll.height; y += 1) {
      for (let x = 0; x <= doll.width; x += 1) {
        const here = at(x, y);
        const left = at(x - 1, y);
        const up = at(x, y - 1);
        if (y < doll.height && here !== left && (here !== null || left !== null)) {
          segs.push(`M${x},${y}V${y + 1}`);
        }
        if (x < doll.width && here !== up && (here !== null || up !== null)) {
          segs.push(`M${x},${y}H${x + 1}`);
        }
      }
    }
    return segs.join("");
  });
</script>

<svg
  class="doll"
  viewBox="0 0 {doll.width} {doll.height}"
  style="aspect-ratio: {doll.width} / {doll.height}"
  role="img"
  aria-label="{label} armour"
  shape-rendering="crispEdges"
>
  {#each squares as sq (sq.y * doll.width + sq.x)}
    <rect x={sq.x} y={sq.y} width="1" height="1" fill={sq.fill}><title>{sq.id}</title></rect>
  {/each}
  {#if edgePath.length > 0}
    <path
      d={edgePath}
      stroke="#0e1216"
      stroke-width="0.16"
      fill="none"
      stroke-linecap="square"
      pointer-events="none"
    />
  {/if}
</svg>

<style>
  .doll {
    display: block;
    flex-shrink: 0;
    width: 100%;
    max-width: 184px;
    min-height: 76px;
    height: auto;
    margin: 0.15rem auto 0.35rem;
  }
</style>
