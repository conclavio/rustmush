<script lang="ts">
  // The quarry screen (doc 20 cut SM.9; §10, D-20-13): the hull's rigs on the left, the map in
  // the middle — the drive view, the whole site, or the body's ground chart as a 2:1 picture
  // with a flat-map toggle — and the viewer's readings and log (or the ground chart's sites)
  // on the right. Every glyph is the frame's (`$lib/quarry`'s pure helpers, pinned by vitest);
  // the picture colours a glyph and draws a mark, never decides one. The one thing reckoned
  // here is where along its frozen leg the viewer's own moving rig stands now (the lead's 0z
  // ruling 1): a one-second tick re-draws it while a leg is in flight, and pulls nothing.
  //
  // **Click-to-prefill, never submit**: a site cell prefills `+tv/<n> drive <x>,<y>`, a reading
  // `+tv/<n> drive <target>`, a ground site `+ship/plan <venue>` — each into the composer.
  import type { GameApp } from "$lib/game.svelte";
  import {
    glyphColour,
    groundPoint,
    groundPrintRows,
    icNow,
    placedRows,
    rigPrefill,
    siteCellOf,
  } from "$lib/quarry";
  import { expandRows } from "$lib/stores/map.svelte";

  let { app }: { app: GameApp } = $props();

  const q = $derived(app.quarry);
  const site = $derived(q.site);
  const block = $derived(site?.site ?? null);
  const ground = $derived(q.ground);
  const groundBlock = $derived(ground?.ground ?? null);
  const moving = $derived(block?.rigs.some((r) => r.leg !== null) ?? false);

  /** The paint clock (ms): ticks each second while one of the viewer's rigs has a leg. */
  let now = $state(Date.now());
  $effect(() => {
    if (!moving) {
      return;
    }
    const timer = setInterval(() => {
      now = Date.now();
    }, 1000);
    return () => clearInterval(timer);
  });

  let flat = $state(false);

  /** The site rows, row 0 the south edge, the viewer's rigs placed at the reckoned instant. */
  const rows = $derived(site && block ? placedRows(site, icNow(block, q.siteAt, now)) : []);
  const CELL = 14;
  const SQUARES = new Set([",", "^", "=", "_", " ", "O"]);

  /** The ground's 2:1 picture: the 56 × 28 provinces, each as tall as it is wide in degrees,
   * north at the top (the grid's row 0 is the south edge). */
  const GW = 896;
  const GH = 448;
  const groundRows = $derived(ground ? expandRows({ ...ground.grid, units: [] }) : []);
  const flatRows = $derived(
    groundBlock && ground ? groundPrintRows(groundBlock, ground.grid.height) : [],
  );

  function prefill(line: string): void {
    app.focusComposer(line);
  }

  function clickCell(fx: number, fy: number): void {
    if (!block) {
      return;
    }
    const cell = siteCellOf(block, fx, fy);
    if (cell) {
      prefill(rigPrefill(q.selectedRig, `drive ${cell[0]},${cell[1]}`));
    }
  }

  function pickRig(slot: number): void {
    app.quarry.rig = slot;
    app.refreshQuarry();
  }
</script>

<div class="quarry">
  <div class="rigs">
    <h3>rigs</h3>
    {#if !block}
      <p class="muted">{site?.grid.title || (q.refused ? "The map the server sent did not read." : "No map yet: r asks the server.")}</p>
    {:else}
      <ul>
        {#each block.rigs as r (r.slot)}
          <li>
            <button type="button" class="rig" class:on={q.selectedRig === r.slot} onclick={() => pickRig(r.slot)}>
              {r.line}
            </button>
            {#if r.run}<div class="sub">{r.run.text}</div>{/if}
            {#if r.queued}<div class="sub">{r.queued}</div>{/if}
            {#if r.plan}<div class="sub">{r.plan.text}</div>{/if}
          </li>
        {/each}
      </ul>
      {#if block.hold}<p class="muted">{block.hold.line}</p>{/if}
    {/if}
  </div>

  <div class="map">
    {#if q.mode === "ground"}
      {#if !ground || !groundBlock}
        <p class="muted pad">{ground?.grid.title || "No ground chart yet: r asks the server."}</p>
      {:else}
        <h2>{ground.grid.title}</h2>
        {#if flat}
          <pre class="flat" aria-label="the flat ground chart">{flatRows.join("\n")}</pre>
        {:else}
          <svg viewBox="0 0 {GW} {GH}" role="img" aria-label="the ground chart">
            {#each groundRows as row, y (y)}
              {#each row as glyph, x (x)}
                <rect
                  x={(x * GW) / ground.grid.width}
                  y={((ground.grid.height - 1 - y) * GH) / ground.grid.height}
                  width={GW / ground.grid.width}
                  height={GH / ground.grid.height}
                  fill={glyphColour(glyph)}
                />
              {/each}
            {/each}
            {#each groundBlock.ports as p (p.glyph)}
              {@const at = groundPoint(p, GW, GH)}
              <circle cx={at.x} cy={at.y} r="6" fill="#f2c94c" />
              <text x={at.x + 8} y={at.y - 6} fill="#f2c94c" font-size="13">{p.glyph} {p.port}</text>
            {/each}
            {#each groundBlock.sites as s (s.letter)}
              {@const at = groundPoint(s, GW, GH)}
              <circle cx={at.x} cy={at.y} r="4" fill={s.here ? "#ffffff" : "#9fc4d8"} />
              <text
                x={at.x + 6}
                y={at.y + 14}
                fill="#9fc4d8"
                font-size="12"
                role="button"
                tabindex="0"
                onclick={() => prefill(`+ship/plan ${s.venue}`)}
                onkeydown={(e) => e.key === "Enter" && prefill(`+ship/plan ${s.venue}`)}
              >{s.letter} {s.site}</text>
            {/each}
          </svg>
        {/if}
        <div class="toggles">
          <button type="button" class:on={!flat} onclick={() => (flat = false)}>picture</button>
          <button type="button" class:on={flat} onclick={() => (flat = true)}>flat map</button>
          <span class="muted">{groundBlock.footer}</span>
        </div>
        <p class="legend muted">{ground.grid.legend.map(([g, l]) => `${g} ${l}`).join("  ")}</p>
      {/if}
    {:else if !site || !block}
      <p class="muted pad">{site?.grid.title || "No map yet: r asks the server."}</p>
      {#if site}<p class="muted pad">z: the ground chart</p>{/if}
    {:else}
      <h2>{site.grid.title}</h2>
      <svg
        viewBox="0 0 {site.grid.width * CELL} {site.grid.height * CELL}"
        role="img"
        aria-label="the site map"
      >
        {#each rows as row, y (y)}
          {#each row as glyph, x (x)}
            {@const top = (site.grid.height - 1 - y) * CELL}
            <rect
              x={x * CELL}
              y={top}
              width={CELL}
              height={CELL}
              fill={glyphColour(SQUARES.has(glyph) ? glyph : ",")}
              role="button"
              tabindex="-1"
              onclick={() => clickCell(x, y)}
              onkeydown={(e) => e.key === "Enter" && clickCell(x, y)}
            />
            {#if !SQUARES.has(glyph)}
              <text
                x={x * CELL + CELL / 2}
                y={top + CELL - 3}
                text-anchor="middle"
                font-size="11"
                fill={glyphColour(glyph)}
                pointer-events="none">{glyph}</text>
            {/if}
          {/each}
        {/each}
        {#each block.marks.letters as [fx, fy, letter] (`${fx},${fy}`)}
          <rect
            x={fx * CELL}
            y={(site.grid.height - 1 - fy) * CELL}
            width={CELL}
            height={CELL}
            fill="none"
            stroke="#5c6670"
            stroke-dasharray="2 2"
            pointer-events="none"
          ><title>block {letter}</title></rect>
        {/each}
        {#if block.marks.hull}
          <rect
            x={block.marks.hull[0] * CELL - 2}
            y={(site.grid.height - 1 - block.marks.hull[1]) * CELL - 2}
            width={CELL + 4}
            height={CELL + 4}
            fill="none"
            stroke="#f2c94c"
            stroke-width="2"
            pointer-events="none"
          ><title>your hull</title></rect>
        {/if}
      </svg>
      <p class="legend muted">{site.grid.legend.map(([g, l]) => `${g} ${l}`).join("  ")}</p>
    {/if}
  </div>

  <div class="side">
    {#if q.mode === "ground"}
      <h3>sites</h3>
      {#if groundBlock}
        <ul>
          {#each groundBlock.sites as s (s.letter)}
            <li>
              <button type="button" class="row" onclick={() => prefill(`+ship/plan ${s.venue}`)}>
                {s.letter} {s.site}{s.passage ? `: ${s.passage}` : ""}
              </button>
              <div class="sub">{s.best_line}</div>
            </li>
          {/each}
        </ul>
        <pre class="lines">{groundBlock.lines.join("\n")}</pre>
      {/if}
    {:else if block}
      <h3>readings, {block.family}</h3>
      {#if block.readings.length === 0}
        {#if block.no_readings}<p class="muted">{block.no_readings}</p>{/if}
      {:else}
        <ul>
          {#each block.readings as r, i (i)}
            <li>
              <button type="button" class="row" onclick={() => prefill(rigPrefill(q.selectedRig, `drive ${r.target}`))}>
                {r.line}
              </button>
            </li>
          {/each}
        </ul>
      {/if}
      {#if block.other_rigs}<p class="muted other-rigs">{block.other_rigs}</p>{/if}
      <h3>log</h3>
      <ul>
        {#each block.log as l, i (i)}
          <li class="log">{l.text}</li>
        {/each}
      </ul>
    {/if}
  </div>
</div>

<style>
  .quarry {
    display: flex;
    gap: 0.75rem;
    padding: 0.5rem 0.75rem;
    height: 100%;
    box-sizing: border-box;
    overflow: auto;
  }
  .rigs {
    flex: 0 0 16rem;
  }
  .map {
    flex: 1 1 32rem;
    min-width: 18rem;
  }
  .side {
    flex: 0 0 18rem;
  }
  h2 {
    font-size: 1rem;
    margin: 0 0 0.5rem;
  }
  h3 {
    font-size: 0.85rem;
    margin: 0.5rem 0 0.25rem;
    color: var(--muted, #9aa);
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  li {
    margin: 0.15rem 0;
  }
  button.rig,
  button.row {
    background: none;
    border: 1px solid transparent;
    color: inherit;
    text-align: left;
    font: inherit;
    cursor: pointer;
    padding: 0.1rem 0.25rem;
    width: 100%;
  }
  button.rig.on {
    border-color: var(--accent, #6b9fd4);
    font-weight: 600;
  }
  .sub {
    padding-left: 1rem;
    font-size: 0.85rem;
    color: var(--muted, #9aa);
  }
  .muted {
    color: var(--muted, #9aa);
  }
  .pad {
    padding: 0.5rem;
  }
  svg {
    width: 100%;
    height: auto;
    background: #0e1116;
  }
  .flat,
  .lines {
    font-family: var(--mono, monospace);
    font-size: 0.75rem;
    white-space: pre;
    overflow: auto;
  }
  .legend {
    font-size: 0.8rem;
  }
  .toggles {
    display: flex;
    gap: 0.5rem;
    align-items: center;
    margin-top: 0.25rem;
  }
  .toggles button.on {
    font-weight: 600;
  }
  .log {
    font-size: 0.85rem;
  }
</style>
