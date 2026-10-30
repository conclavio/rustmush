<script lang="ts">
  // The tactical map (doc 14 §6, plan §3.5): the `atlas.map` frame drawn on a
  // canvas — one square per cell coloured by its glyph, the units as discs,
  // the viewer's ringed — from the same rows the text renderer prints, so
  // the picture and the text can never disagree for one viewer at one
  // instant. Nothing here knows a rule: the glyph is the fact.
  import type { GameApp } from "$lib/game.svelte";
  import { biomeTerrainColours, expandRows } from "$lib/stores/map.svelte";

  let { app }: { app: GameApp } = $props();

  const frame = $derived(app.map.frame);
  const notes = $derived(app.map.notes);

  /** Colours by glyph; an unknown glyph is the void. One table for both map
   * canvases since 16b cut Q.5 — see `TERRAIN_COLOURS` / `biomeTerrainColours`. */
  const COLOURS = $derived(biomeTerrainColours(frame?.surface));

  let canvas: HTMLCanvasElement | undefined = $state();

  const CELL_MAX = 14;
  const CELL_MIN = 3;
  const WIDTH_MAX = 480;

  const cellPx = $derived(
    frame ? Math.max(CELL_MIN, Math.min(CELL_MAX, Math.floor(WIDTH_MAX / Math.max(1, frame.width)))) : 0,
  );

  $effect(() => {
    const f = frame;
    const c = canvas;
    if (!f || !c) return;
    const px = cellPx;
    c.width = f.width * px;
    c.height = f.height * px;
    const ctx = c.getContext("2d");
    if (!ctx) return;
    ctx.clearRect(0, 0, c.width, c.height);
    const rows = expandRows({ ...f, units: [] });
    for (let y = 0; y < f.height; y += 1) {
      const row = rows[y] ?? [];
      // Row 0 is the south edge: draw it at the bottom.
      const top = (f.height - 1 - y) * px;
      for (let x = 0; x < f.width; x += 1) {
        const glyph = row[x] ?? " ";
        const colour = COLOURS[glyph] ?? "transparent";
        if (colour === "transparent") continue;
        ctx.fillStyle = colour;
        ctx.fillRect(x * px, top, px, px);
      }
    }
    for (const unit of f.units) {
      const [x, y] = unit.cell;
      const cx = x * px + px / 2;
      const cy = (f.height - 1 - y) * px + px / 2;
      ctx.beginPath();
      ctx.arc(cx, cy, Math.max(1.5, px * 0.38), 0, Math.PI * 2);
      ctx.fillStyle = unit.you ? "#f2c94c" : "#e05a5a";
      ctx.fill();
      if (unit.you) {
        ctx.lineWidth = Math.max(1, px * 0.12);
        ctx.strokeStyle = "#fff8dc";
        ctx.stroke();
      }
    }
  });
</script>

{#if frame}
  <section class="map" aria-label="map">
    <header>
      <strong>{frame.title || "Map"}</strong>
      <span class="dim">
        {frame.surface}{frame.floors.length > 1 ? ` · floors: ${frame.floors.join(", ")}` : ""}
        · {frame.cell_cm} cm cells · {frame.width}×{frame.height}
        {#if frame.you}· you at ({frame.you[0]}, {frame.you[1]}){/if}
      </span>
    </header>
    <div class="canvas-wrap">
      <canvas bind:this={canvas} style="width: {frame.width * cellPx}px; height: {frame.height * cellPx}px"></canvas>
    </div>
    <ul class="legend">
      {#each frame.legend as [glyph, label] (glyph + label)}
        <li><span class="swatch" style="background: {COLOURS[glyph] ?? 'transparent'}"></span><code>{glyph}</code> {label}</li>
      {/each}
      {#each frame.units as unit (unit.id)}
        <li><code>{unit.glyph}</code> {unit.name}{unit.you ? " (you)" : ""} ({unit.cell[0]}, {unit.cell[1]})</li>
      {/each}
    </ul>
    {#if notes.length > 0}
      <ul class="notes">
        {#each notes.slice(-4) as note, i (i)}
          <li>{note}</li>
        {/each}
      </ul>
    {/if}
  </section>
{/if}

<style>
  .map {
    border-top: 1px solid var(--border);
    background: var(--bg-panel);
    padding: 0.35rem 0.75rem;
    font-family: var(--font-mono);
    font-size: 0.8rem;
  }

  header {
    display: flex;
    gap: 0.6rem;
    align-items: baseline;
    flex-wrap: wrap;
  }

  .dim {
    color: var(--fg-dim, #999);
  }

  .canvas-wrap {
    overflow: auto;
    max-height: 40vh;
    margin: 0.3rem 0;
  }

  canvas {
    image-rendering: pixelated;
    display: block;
  }

  .legend,
  .notes {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 0.25rem 0.9rem;
  }

  .swatch {
    display: inline-block;
    width: 0.7em;
    height: 0.7em;
    margin-right: 0.3em;
    border: 1px solid var(--border);
    vertical-align: middle;
  }

  .notes {
    flex-direction: column;
    gap: 0;
    color: var(--fg-dim, #999);
  }
</style>
