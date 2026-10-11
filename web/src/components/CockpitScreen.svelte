<script lang="ts">
  // The cockpit screen (16b cut Q.5; doc 16 §5): the whole of what the arena
  // tells one pilot, updating by itself as the frames arrive — the ground
  // around the tank with the contacts on it, what the platform is doing, what
  // is left of it, what it can shoot with (and, 17b cut M5.5, the mind sealed
  // to fly it when the pilot is not in the seat), what it holds, and the
  // arena's own notices.
  //
  // It renders ONLY what the payload carries. There is no lookup by id into
  // any other store — not who, not presence, not the unit feed, not the atlas
  // map — because the observer rule (doc 16 §1) is that nothing tells a pilot
  // anything about a unit their platform has not detected, and a client that
  // joined a contact to a name it happened to know would break it without the
  // server ever sending a wrong byte. `$lib/cockpit`'s decoder does not even
  // keep the fields that could leak: no contact `id`, no `lock` digits, no
  // `unit` object number, no notice `text`. (The notices pane also shows the
  // pilot's own `Sys` replies while seated — 16b cut B.1 — which are lines
  // the server sent this pilot, never a lookup; the notice sentences' own
  // copies are kept out of it.)
  //
  // Every string printed here comes out of `cockpitView`, and the canvas draws
  // from `composeWindow` — both pure, both pinned by vitest against the
  // committed fixtures, so what the test reads is what the player sees.
  import type { GameApp } from "$lib/game.svelte";
  import {
    COCKPIT_VIEW_CELLS,
    COCKPIT_ZOOM_CELLS,
    cockpitView,
    composeWindow,
    contactText,
    damageText,
    labelPlan,
    markerOwners,
    NOT_IN_A_COCKPIT,
    scanText,
    terrainCacheKey,
    WAITING_FOR_THE_MAP,
    type ArenaMap,
    type ComposedWindow,
    type Marker,
  } from "$lib/cockpit";
  import { TERRAIN_COLOURS } from "$lib/stores/map.svelte";

  let { app }: { app: GameApp } = $props();

  /** What a marker is painted in. The five kinds are the whole vocabulary:
   * nothing has classified it, your side, the other side, you — and (16b cut
   * V.1) a named spot of the ground, in a sand tone none of the four is. */
  const MARKER_COLOURS: Record<Marker["kind"], string> = {
    you: "#f2c94c",
    unknown: "#b9a23a",
    friendly: "#4fa86a",
    hostile: "#e05a5a",
    spot: "#d8c8a0",
    // 16b cut I.2: a replay's kinds, which `composeWindow` never produces —
    // named here only because the record is total over `MarkerKind`; the
    // replay screen paints them (`ReplayScreen.svelte`).
    red: "#e05a5a",
    blue: "#4f7fd1",
    wreck: "#777",
  };

  /** The viewport, in arena cells; `+`/`−` step through `COCKPIT_ZOOM_CELLS`. */
  let cells = $state(COCKPIT_VIEW_CELLS);
  let canvas: HTMLCanvasElement | undefined = $state();

  const frame = $derived(app.cockpit.frame);
  const ground = $derived(app.cockpit.holdsTheGround ? app.cockpit.map : null);

  const win = $derived<ComposedWindow | null>(
    frame !== null && ground !== null ? composeWindow(ground, frame, cells, cells) : null,
  );
  const view = $derived(frame === null ? null : cockpitView(frame, win, app.cockpit.notices));

  /** The drawn size of one cell: big enough to see, small enough that a 256
   * cell window still fits a phone. */
  const CANVAS_MAX = 560;
  const cellPx = $derived(
    win === null ? 0 : Math.max(2, Math.min(12, Math.floor(CANVAS_MAX / Math.max(win.w, win.h, 1)))),
  );

  // **The terrain cache** (16b cut B.2, `T-33`). The ground is painted once
  // per WINDOW into an offscreen canvas and every frame copies it with one
  // `drawImage` before the markers go on top; before this, every keyframe —
  // up to two a second while a fight is on — repainted every cell, 6,144
  // `fillRect`s for the fixture's ground and 65,536 at 256 cells of a full
  // arena. The cache is keyed on the window (`terrainCacheKey`: its corner,
  // its size and the cell size in pixels) and on the map OBJECT, compared by
  // identity — a new map is new ground — and NEVER on the frame: a contact
  // that moves, a notice, a new heading repaint the markers and nothing else.
  // Plain `let`s, not `$state`: the effect writes them, and must not re-run
  // because it did.
  let terrainCanvas: HTMLCanvasElement | null = null;
  let terrainKey = "";
  let terrainGround: ArenaMap | null = null;

  function zoom(delta: number): void {
    const at = COCKPIT_ZOOM_CELLS.indexOf(cells);
    const next = COCKPIT_ZOOM_CELLS[Math.min(COCKPIT_ZOOM_CELLS.length - 1, Math.max(0, at + delta))];
    if (next !== undefined) {
      cells = next;
    }
  }

  /** Prefill the composer with an order; the player presses Enter. None of the
   * five ordering verbs — nor (17b cut M5.5) `MIND PROFILE` — is ever SENT by
   * this screen. */
  function prefill(line: string): void {
    app.focusComposer(line);
  }

  $effect(() => {
    const w = win;
    const c = canvas;
    const f = frame;
    const g = ground;
    if (!w || !c || !f || !g) return;
    const px = cellPx;
    c.width = w.w * px;
    c.height = w.h * px;
    const ctx = c.getContext("2d");
    if (!ctx) return;
    ctx.clearRect(0, 0, c.width, c.height);
    // The ground first, from the marker-free rows: a contact standing on a
    // wall must not repaint the wall as whatever its own glyph means. Painted
    // into the cache only when the window or the map changed (see above).
    const key = terrainCacheKey(w, px);
    if (terrainCanvas === null || key !== terrainKey || g !== terrainGround) {
      const off = terrainCanvas ?? document.createElement("canvas");
      off.width = c.width;
      off.height = c.height;
      const tctx = off.getContext("2d");
      if (!tctx) return;
      tctx.clearRect(0, 0, off.width, off.height);
      for (let row = 0; row < w.h; row += 1) {
        const line = w.terrain[row] ?? "";
        for (let col = 0; col < w.w; col += 1) {
          const colour = TERRAIN_COLOURS[line.charAt(col)] ?? "transparent";
          if (colour === "transparent") continue;
          tctx.fillStyle = colour;
          tctx.fillRect(col * px, row * px, px, px);
        }
      }
      terrainCanvas = off;
      terrainKey = key;
      terrainGround = g;
    }
    ctx.drawImage(terrainCanvas, 0, 0);
    // Which labels are painted (16b cut B.1, `B-09`): a hostile and the lock at
    // every zoom, the rest from 6 px, an overlapping label skipped in frame
    // order and the lock's never — decided once, by the pure `labelPlan`.
    const labels = labelPlan(w.markers, f.lock_designator, px);
    // Under 6 px a label is a fixed 8 px font with a one-pixel halo in the
    // canvas's own background, so it reads over terrain and over other dots.
    const halo = px < 6 ? getComputedStyle(c).backgroundColor || "#000" : null;
    // 16b cut V.1: which markers own their cell, once per paint — the owner rule
    // `markerLegend` names by. A spot a later marker landed on is drawn nowhere.
    const owns = markerOwners(w.markers);
    const centre = (marker: Marker): [number, number] => [
      marker.x * px + px / 2,
      // `marker.y` counts from the SOUTH; the canvas paints north-up.
      (w.h - 1 - marker.y) * px + px / 2,
    ];
    const label = (marker: Marker, cx: number, cy: number, colour: string): void => {
      ctx.fillStyle = colour;
      ctx.textBaseline = "middle";
      if (halo !== null) {
        ctx.font = "8px ui-monospace, monospace";
        ctx.lineWidth = 2;
        ctx.strokeStyle = halo;
        ctx.strokeText(marker.designator, cx + px * 0.8, cy);
      } else {
        ctx.font = `${Math.max(8, px)}px ui-monospace, monospace`;
      }
      ctx.fillText(marker.designator, cx + px * 0.8, cy);
    };
    // Then the markers, in the order they were composed — the spots first — so
    // a later one covers an earlier one here exactly as it does in the glyph
    // rows.
    for (const [i, marker] of w.markers.entries()) {
      const [cx, cy] = centre(marker);
      const colour = MARKER_COLOURS[marker.kind];
      ctx.fillStyle = colour;
      if (marker.kind === "spot") {
        // A hollow square, never a dot, a wedge or a ring: ground, not a unit.
        // `strokeRect` and not a path, so the lock ring stays the one `stroke`.
        if (owns[i] !== true) continue;
        const side = Math.max(4, px * 0.9);
        ctx.lineWidth = 1;
        ctx.strokeStyle = colour;
        ctx.strokeRect(cx - side / 2, cy - side / 2, side, side);
        continue;
      }
      if (marker.kind === "you") {
        // A wedge, pointing the way the tank is pointing (0° is north).
        const r = Math.max(3, px * 0.9);
        const a = (f.heading_deg * Math.PI) / 180;
        ctx.beginPath();
        ctx.moveTo(cx + r * Math.sin(a), cy - r * Math.cos(a));
        ctx.lineTo(cx + r * 0.7 * Math.sin(a + 2.5), cy - r * 0.7 * Math.cos(a + 2.5));
        ctx.lineTo(cx + r * 0.7 * Math.sin(a - 2.5), cy - r * 0.7 * Math.cos(a - 2.5));
        ctx.closePath();
        ctx.fill();
      } else {
        ctx.beginPath();
        ctx.arc(cx, cy, Math.max(2, px * 0.45), 0, Math.PI * 2);
        ctx.fill();
        if (marker.designator === f.lock_designator) {
          ctx.lineWidth = Math.max(1, px * 0.2);
          ctx.strokeStyle = "#fff8dc";
          ctx.beginPath();
          ctx.arc(cx, cy, Math.max(4, px * 0.85), 0, Math.PI * 2);
          ctx.stroke();
        }
      }
      if (labels[i] === true) {
        label(marker, cx, cy, colour);
      }
    }
    // 16b cut V.1: the spots' names last, in authored order, under the same
    // owner gate as their squares — `labelPlan` placed them after every
    // contact, so a field's name never displaces a contact's label.
    for (const [i, marker] of w.markers.entries()) {
      if (marker.kind !== "spot" || owns[i] !== true || labels[i] !== true) continue;
      const [cx, cy] = centre(marker);
      label(marker, cx, cy, MARKER_COLOURS.spot);
    }
  });
</script>

<div class="cockpit">
  {#if frame === null || view === null}
    <p class="muted pad">{NOT_IN_A_COCKPIT}</p>
  {:else}
    <section class="mapbox" aria-label="arena map">
      <header>
        <strong>{view.heading}</strong>
        <span class="grow"></span>
        <span class="dim">{cells} cells</span>
        <!-- 16b cut B.1 (`B-60`): only the glyphs moved. The labels and the
             handlers were always true — fewer cells IS zooming in — but the
             button that zooms in wore a minus sign. -->
        <button type="button" class="zoom" aria-label="zoom in" onclick={() => zoom(-1)}>+</button>
        <button type="button" class="zoom" aria-label="zoom out" onclick={() => zoom(1)}>−</button>
      </header>
      {#if win === null}
        <p class="muted">{WAITING_FOR_THE_MAP}</p>
      {:else}
        <div class="canvas-wrap">
          <canvas
            bind:this={canvas}
            aria-label="arena window"
            style="width: {win.w * cellPx}px; height: {win.h * cellPx}px"
          ></canvas>
        </div>
        {#if view.legend.length > 0}
          <p class="legendline">{view.legend}</p>
        {/if}
      {/if}
    </section>

    <div class="panes">
      <section class="pane" aria-label="status">
        <h4 class="pt">status</h4>
        {#each view.status as line, i (i)}
          <p class="line">{line}</p>
        {/each}
      </section>

      <section class="pane" aria-label="damage">
        <h4 class="pt">damage</h4>
        {#each view.damage as row, i (i)}
          <p class="line" title={damageText(row)}>
            <span class="rowid">{row.id}</span>
            <span class="cond">{row.condition}</span>
            <span class="frac">{row.internals}</span>
          </p>
          <div class="bar" data-kind="inside">
            <span class="fill" style="width: {row.pct}%"></span>
          </div>
          {#each row.facets as facet, f (f)}
            <div class="facet">
              <span class="barlabel">{facet.id} {facet.armor}</span>
              <div class="bar" data-kind="armour">
                <span class="fill" style="width: {facet.pct}%"></span>
              </div>
            </div>
          {/each}
        {/each}
        {#if view.damage.length === 0}
          <p class="muted">Nothing here.</p>
        {/if}
      </section>

      <section class="pane" aria-label="weapons">
        <h4 class="pt">weapons</h4>
        {#each view.weapons as line, i (i)}
          <p class="line">{line}</p>
        {/each}
        <!-- 17b cut M5.5: the mind's four lines, the section's tail (in
             `(no weapon)`'s place on an unarmed hull). On the profile row each
             sealed profile the mind is NOT flying is a button that puts `MIND
             PROFILE <name>` in the box and sends nothing (its title says so);
             the flown one is plain text, since the cockpit refuses a switch to
             it (the review's D4); the row's text is `mindLines`' own. -->
        {#each view.mind as line, i (i)}
          {#if i === 1 && view.profiles.length > 0}
            <p class="line mind profiles" title={line}>{#each view.profiles as profile, p (p)}<span
                  class="lead">{profile.lead}</span>{#if profile.active}<span class="flown"
                    >{profile.name}</span
                  >{:else}<button
                    type="button"
                    class="profile"
                    title={`MIND PROFILE ${profile.name}`}
                    onclick={() => prefill(`MIND PROFILE ${profile.name}`)}>{profile.name}</button
                  >{/if}{/each}</p>
          {:else}
            <p class="line mind">{line}</p>
          {/if}
        {/each}
      </section>

      <section class="pane wide" aria-label="contacts">
        <h4 class="pt">contacts</h4>
        {#each view.contacts as row, i (i)}
          <div class="contact">
            <button
              type="button"
              class="desig"
              title={contactText(row)}
              onclick={() => prefill(`lock ${row.designator}`)}>{row.designator}</button
            >
            <span class="detail">{row.detail}</span>
            <button type="button" class="scanbtn" onclick={() => prefill(`scan ${row.designator}`)}
              >scan</button
            >
          </div>
          {#if row.chips.length > 0}
            <p class="chips" title={scanText(row.chips)}>
              {#each row.chips as chip, i (i)}
                <span class="chip" data-band={chip.band}>{chip.word}</span>
              {/each}
            </p>
          {/if}
        {/each}
        {#if view.contacts.length === 0}
          <p class="line">(no contacts)</p>
        {/if}
      </section>

      <section class="pane wide" aria-label="notices">
        <h4 class="pt">notices</h4>
        {#each view.notices.slice(-12) as line, i (i)}
          <p class="line dim">{line}</p>
        {/each}
        {#if view.notices.length === 0}
          <p class="muted">Nothing yet.</p>
        {/if}
      </section>
    </div>
  {/if}
</div>

<style>
  .cockpit {
    height: 100%;
    overflow-y: auto;
    padding: 0.6rem 0.75rem;
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
  }

  .mapbox {
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-panel);
    padding: 0.4rem 0.6rem;
  }

  .mapbox header {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-family: var(--font-mono);
    font-size: 0.85rem;
  }

  .grow {
    flex: 1;
  }

  .zoom {
    background: none;
    border: 1px solid var(--border);
    border-radius: 4px;
    color: var(--fg-muted);
    cursor: pointer;
    font: inherit;
    line-height: 1;
    padding: 0.05rem 0.4rem;
  }

  .zoom:hover {
    color: var(--fg);
  }

  .canvas-wrap {
    overflow: auto;
    max-height: 52vh;
    margin: 0.3rem 0;
  }

  canvas {
    image-rendering: pixelated;
    display: block;
    background: var(--bg);
  }

  .legendline {
    margin: 0;
    font-family: var(--font-mono);
    font-size: 0.78rem;
    color: var(--fg-muted);
  }

  .panes {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(18rem, 1fr));
    gap: 0.6rem;
    align-items: start;
  }

  .pane {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    padding: 0.5rem 0.65rem;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-panel);
    min-width: 0;
    overflow-x: auto;
  }

  .pane.wide {
    grid-column: span 2;
  }

  @media (max-width: 800px) {
    .pane.wide {
      grid-column: auto;
    }
  }

  .pt {
    margin: 0 0 0.1rem;
    font-size: 0.72rem;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--fg-muted);
    font-weight: 600;
  }

  .line {
    margin: 0;
    font-family: var(--font-mono);
    font-size: 0.82rem;
    white-space: pre;
  }

  .rowid {
    display: inline-block;
    min-width: 7rem;
  }

  .cond {
    color: var(--fg-muted);
    display: inline-block;
    min-width: 6rem;
  }

  .frac {
    color: var(--fg-muted);
  }

  .bar {
    height: 0.4rem;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 2px;
    overflow: hidden;
    margin: 0 0 0.25rem;
  }

  .fill {
    display: block;
    height: 100%;
    background: var(--accent);
  }

  .bar[data-kind="armour"] .fill {
    background: var(--accent-dim);
  }

  .facet {
    margin-left: 0.8rem;
  }

  .barlabel {
    font-family: var(--font-mono);
    font-size: 0.72rem;
    color: var(--fg-muted);
  }

  .contact {
    display: flex;
    align-items: baseline;
    gap: 0.4rem;
  }

  .desig {
    background: none;
    border: none;
    border-bottom: 1px dotted var(--border);
    color: var(--accent);
    cursor: pointer;
    font-family: var(--font-mono);
    font-size: 0.82rem;
    padding: 0;
    min-width: 2.5rem;
    text-align: left;
  }

  /* 17b cut M5.5's review: a little air between the last weapon line and the
     mind's first, so the tail does not read as one more weapon (on an unarmed
     hull it follows the heading and takes none). */
  .line:not(.mind) + .line.mind {
    margin-top: 0.35rem;
  }

  /* 17b cut M5.5: a sealed profile on the mind tail's profile row. */
  .profile {
    background: none;
    border: none;
    border-bottom: 1px dotted var(--border);
    color: var(--accent);
    cursor: pointer;
    font: inherit;
    padding: 0;
  }

  /* The profile the mind flies: named, not offered (a switch to it is refused). */
  .flown {
    font-weight: 600;
  }

  .detail {
    font-family: var(--font-mono);
    font-size: 0.82rem;
    flex: 1;
    white-space: pre;
  }

  .scanbtn {
    background: none;
    border: 1px solid var(--border);
    border-radius: 999px;
    color: var(--fg-muted);
    cursor: pointer;
    font: inherit;
    font-size: 0.7rem;
    padding: 0 0.45rem;
  }

  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 0.2rem;
    margin: 0 0 0.25rem 0.5rem;
  }

  .chip {
    border-radius: 3px;
    font-family: var(--font-mono);
    font-size: 0.72rem;
    padding: 0 0.3rem;
    border: 1px solid var(--border);
  }

  .chip[data-band="green"] {
    color: var(--success);
  }

  .chip[data-band="yellow"] {
    color: var(--warn);
  }

  .chip[data-band="red"] {
    color: var(--error);
  }

  .chip[data-band="destroyed"] {
    color: var(--error);
    opacity: 0.6;
    text-decoration: line-through;
  }

  .muted {
    color: var(--fg-muted);
    font-size: 0.85rem;
    margin: 0;
  }

  .dim {
    color: var(--fg-muted);
  }

  .pad {
    padding: 0.8rem;
  }
</style>
