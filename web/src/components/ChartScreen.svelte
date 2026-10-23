<script lang="ts">
  // The sky chart screen (doc 19 cut A.3b; doc 19 §5): the `sky.chart` panel
  // drawn as a picture — the Sun at the centre, every body's orbit as a ring
  // at the server's own logarithmic radial scale, the bodies as discs with
  // their letters, the listed routes as lines from the port the chart is read
  // from, that port ringed, and (doc 21 cut G.5c) rings of 3, 6, 12 and 24
  // light-minutes about it, as the terminal's picture draws them — with the sky table, the passages and the window
  // beside it, and the server's flat character map under a toggle, so the
  // picture a web player sees and the map a TUI or line player sees are two
  // renders of ONE frame.
  //
  // Every string printed here comes out of `$lib/chart`'s pure helpers, which
  // vitest pins; nothing here decides a figure.
  import type { GameApp } from "$lib/game.svelte";
  import {
    bodyLines,
    bodyPoints,
    chartHeading,
    LAG_RING_MINUTES,
    lagRing,
    orbitRadius,
    routeLines,
    routePolyline,
    scaleCaption,
    windowLines,
    type ChartPoint,
  } from "$lib/chart";
  import { expandRows } from "$lib/stores/map.svelte";

  let { app }: { app: GameApp } = $props();

  const frame = $derived(app.chart.frame);
  const chart = $derived(frame?.chart ?? null);

  /** The picture's size in its own units; it scales to the pane. */
  const SIZE = 640;
  let flat = $state(false);

  const points = $derived<ChartPoint[]>(chart ? bodyPoints(chart, SIZE) : []);
  const pointOf = $derived(new Map(points.map((p) => [p.id, p])));
  const heading = $derived(chart ? chartHeading(chart) : "");
  const bodies = $derived(chart ? bodyLines(chart) : []);
  const routes = $derived(chart ? routeLines(chart) : []);
  const windows = $derived(chart ? windowLines(chart) : []);
  /** The flat map's rows, north up (row 0 of the grid is the south edge). */
  const flatRows = $derived(frame ? expandRows({ ...frame.grid, units: [] }).map((r) => r.join("")).reverse() : []);

  const RING = "#3a3f4a";
  const ROUTE = "#c2a86b";
  const HERE = "#f2c94c";
  const BODY = "#9fc4d8";
  const MOON = "#6b7a88";
  const LAG = "#46788f";
  /** Doc 21 cut G.5c: the light-lag rings about the viewer's body. */
  const rings = $derived(
    chart ? LAG_RING_MINUTES.map((minutes) => ({ minutes, points: lagRing(chart, minutes, SIZE) })) : [],
  );
</script>

<div class="chart">
  {#if !chart}
    <p class="muted pad">
      {app.chart.refused ? "The sky chart the server sent did not read." : "Waiting for the sky chart (r asks again)."}
    </p>
  {:else}
    <h2>{heading}</h2>
    <div class="row">
      <div class="picture">
        {#if flat}
          <pre class="flat" aria-label="the flat sky map">{flatRows.join("\n")}</pre>
        {:else}
          <svg viewBox="0 0 {SIZE} {SIZE}" role="img" aria-label="the sky chart">
            <rect x="0" y="0" width={SIZE} height={SIZE} fill="#0e1116" />
            {#each chart.bodies.filter((b) => b.parent === null) as b (b.id)}
              <circle cx={SIZE / 2} cy={SIZE / 2} r={orbitRadius(b, chart, SIZE)} fill="none" stroke={RING} stroke-width="1" />
            {/each}
            {#each rings as ring (ring.minutes)}
              {#if ring.points.length > 1}
                <polyline
                  points={ring.points.map((p) => `${p.x.toFixed(1)},${p.y.toFixed(1)}`).join(" ")}
                  fill="none"
                  stroke={LAG}
                  stroke-width="1"
                  stroke-dasharray="2 4"
                />
                {@const top = ring.points[Math.floor(ring.points.length / 4)]}
                {#if top}
                  <text x={top.x} y={top.y - 3} fill={LAG} font-size="10" text-anchor="middle">{ring.minutes} min</text>
                {/if}
              {/if}
            {/each}
            {#each chart.routes as r (r.to)}
              {@const line = routePolyline(r, points)}
              {#if line.length > 1}
                <polyline
                  points={line.map((p) => `${p.x.toFixed(1)},${p.y.toFixed(1)}`).join(" ")}
                  fill="none"
                  stroke={ROUTE}
                  stroke-width="1.5"
                  stroke-dasharray="4 3"
                />
              {/if}
            {/each}
            <circle cx={SIZE / 2} cy={SIZE / 2} r="5" fill="#f5d36b" />
            {#each chart.bodies as b (b.id)}
              {@const p = pointOf.get(b.id)}
              {#if p && b.parent === null}
                <circle cx={p.x} cy={p.y} r={b.here ? 6 : 4} fill={b.here ? HERE : BODY} />
                {#if b.here}
                  <circle cx={p.x} cy={p.y} r="11" fill="none" stroke={HERE} stroke-width="1.5" />
                {/if}
                <text x={p.x + 8} y={p.y - 6} fill={b.here ? HERE : BODY} font-size="13">{b.label}</text>
              {:else if p}
                <text x={p.x + 8} y={p.y + 12} fill={MOON} font-size="10">{b.label}</text>
              {/if}
            {/each}
          </svg>
        {/if}
        <div class="toggles">
          <button type="button" class:on={!flat} onclick={() => (flat = false)}>picture</button>
          <button type="button" class:on={flat} onclick={() => (flat = true)}>flat map</button>
          <span class="muted">{scaleCaption(chart)}</span>
        </div>
      </div>
      <div class="tables">
        <h3>the sky</h3>
        <pre>{bodies.join("\n")}</pre>
        {#if routes.length > 0}
          <h3>passages now</h3>
          <ul>
            {#each routes as line, i (i)}
              <li>{line}</li>
            {/each}
          </ul>
        {/if}
        {#if windows.length > 0}
          <h3>the window</h3>
          <ul>
            {#each windows as line, i (i)}
              <li>{line}</li>
            {/each}
          </ul>
        {/if}
      </div>
    </div>
  {/if}
</div>

<style>
  .chart {
    padding: 0.5rem 0.75rem;
    overflow: auto;
    height: 100%;
    box-sizing: border-box;
  }
  h2 {
    font-size: 1rem;
    margin: 0 0 0.5rem;
  }
  h3 {
    font-size: 0.85rem;
    margin: 0.75rem 0 0.25rem;
    color: var(--muted, #9aa);
  }
  .row {
    display: flex;
    gap: 1rem;
    flex-wrap: wrap;
    align-items: flex-start;
  }
  .picture {
    flex: 0 1 480px;
    min-width: 280px;
  }
  svg {
    width: 100%;
    height: auto;
    display: block;
    border-radius: 4px;
  }
  .flat {
    background: #0e1116;
    color: #cfd8e3;
    font-size: 0.7rem;
    line-height: 1.05;
    padding: 0.5rem;
    overflow: auto;
    border-radius: 4px;
    margin: 0;
  }
  .toggles {
    display: flex;
    gap: 0.5rem;
    align-items: center;
    margin-top: 0.35rem;
    font-size: 0.8rem;
  }
  .toggles button {
    background: transparent;
    border: 1px solid #555;
    color: inherit;
    border-radius: 3px;
    padding: 0.1rem 0.5rem;
    cursor: pointer;
  }
  .toggles button.on {
    border-color: #c2a86b;
    color: #c2a86b;
  }
  .tables {
    flex: 1 1 320px;
    min-width: 280px;
  }
  .tables pre {
    font-size: 0.78rem;
    margin: 0;
    white-space: pre;
    overflow: auto;
  }
  .tables ul {
    margin: 0;
    padding-left: 1.1rem;
    font-size: 0.85rem;
  }
  .tables li {
    margin-bottom: 0.25rem;
  }
  .muted {
    color: var(--muted, #9aa);
  }
  .pad {
    padding: 0.5rem;
  }
</style>
