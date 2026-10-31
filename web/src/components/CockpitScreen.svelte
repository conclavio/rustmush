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
  // Doc 21 cut G.3c adds the first-person picture above the map
  // (`PictureView.svelte`): drawn by the gateway from this pilot's own feeds,
  // so it shows exactly what this screen does, and its keys are the one
  // place this tab SENDS an order (`HEADING`, as the SSH TUI's view does;
  // doc 21 cut G.12c adds §7.3's `SPEED`, `FIRE` and `LOCK` keys, stepped
  // from the frame this screen hands it). Its `F1`–`F6` prefill `ORDER`
  // through this screen's own `prefill`, never sent.
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
    type CockpitWeapon,
    type ComposedWindow,
    type Marker,
  } from "$lib/cockpit";
  import { biomeTerrainColours } from "$lib/stores/map.svelte";
  import {
    DEFAULT_MAX_SPEED_KPH,
    DEFAULT_REVERSE_KPH,
    gateWord,
    speedLine,
  } from "$lib/picture";
  import PictureView from "./PictureView.svelte";
  import DollView from "./DollView.svelte";
  import { lookFor } from "$lib/look";
  import { detachmentLines } from "$lib/detachment";

  let { app }: { app: GameApp } = $props();

  /** What a marker is painted in. The five kinds are the whole vocabulary:
   * by the viewer's relation (doc 32 cut CF.0, Q-CF-12; `contactKind`) —
   * neutral or not yet classified, own or friend, hostile — you, and (16b
   * cut V.1) a named spot of the ground, in a sand tone none of the four is. */
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
    // Doc 32 cut CF.4: the campaign layer's glyphs, ground-toned.
    campaign: "#c8b88a",
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
  /** Doc 22 cut U.4: the pilot's own unit's look — static art keyed by the
   * frame's own `unit_type`, never a lookup into another store. */
  const look = $derived(frame === null ? null : lookFor(frame.unit_type));

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

  /** Prefill the composer with an order; the player presses Enter. */
  function prefill(line: string): void {
    app.focusComposer(line);
  }

  // -- speed drag slider, auto-target, and weapons-free rule controls --------

  const maxKph = $derived(frame?.max_speed_kph ?? DEFAULT_MAX_SPEED_KPH);
  const reverseKph = $derived(Math.abs(frame?.reverse_kph ?? DEFAULT_REVERSE_KPH));
  let sliderKph = $state<number | null>(null);
  const currentSliderKph = $derived(sliderKph ?? frame?.speed_kph ?? 0);

  $effect(() => {
    const f = frame;
    if (!f) {
      sliderKph = null;
      return;
    }
    if (sliderKph !== null && f.speed_kph === sliderKph) {
      sliderKph = null;
    }
  });

  function onSpeedSlider(ev: Event): void {
    const raw = Number((ev.currentTarget as HTMLInputElement).value);
    if (!Number.isFinite(raw)) return;
    const clamped = Math.max(-reverseKph, Math.min(maxKph, Math.round(raw)));
    sliderKph = clamped;
    app.helm(speedLine(clamped));
  }

  function stopSpeed(): void {
    sliderKph = 0;
    app.helm(speedLine(0));
  }

  let autoTargetArmed = $state(false);

  $effect(() => {
    const f = frame;
    if (!f) {
      autoTargetArmed = false;
      return;
    }
    if (f.rules.some((r) => r.action === "lock")) {
      autoTargetArmed = true;
    }
  });

  function toggleAutoTarget(): void {
    const lockRule = frame?.rules.find((r) => r.action === "lock") ?? null;
    if (autoTargetArmed || lockRule !== null) {
      autoTargetArmed = false;
      app.helm(`MIND RULE ${lockRule?.slot ?? 1} OFF`);
    } else {
      autoTargetArmed = true;
      app.helm("LOCK nearest WHEN nolock");
    }
  }

  let fireRuleWeapon = $state("ALL");
  let fireRuleOdds = $state(60);
  let fireRuleArmed = $state(false);

  $effect(() => {
    const f = frame;
    if (!f) {
      fireRuleArmed = false;
      return;
    }
    if (fireRuleWeapon !== "ALL" && !f.weapons.some((w) => w.id === fireRuleWeapon)) {
      fireRuleWeapon = "ALL";
    }
  });

  function sendFireRule(): void {
    fireRuleArmed = true;
    if (frame?.weapons_free === false) {
      app.helm("FIRE FREE");
    }
    app.helm(`FIRE ${fireRuleWeapon} WHEN odds >= ${fireRuleOdds}`);
  }

  function onFireWeaponChange(ev: Event): void {
    fireRuleWeapon = (ev.currentTarget as HTMLSelectElement).value || "ALL";
    sendFireRule();
  }

  function onFireOddsChange(ev: Event): void {
    const raw = Number((ev.currentTarget as HTMLInputElement).value);
    if (Number.isFinite(raw)) {
      fireRuleOdds = Math.max(10, Math.min(100, Math.round(raw)));
    }
    sendFireRule();
  }

  function toggleFireRule(): void {
    if (fireRuleArmed) {
      fireRuleArmed = false;
      app.helm(`FIRE ${fireRuleWeapon} OFF`);
    } else {
      sendFireRule();
    }
  }

  function weaponForLine(line: string): CockpitWeapon | null {
    if (!frame) return null;
    return frame.weapons.find((w) => line === w.id || line.startsWith(`${w.id}  `)) ?? null;
  }

  function legendParts(
    legend: string,
    w: ComposedWindow | null,
  ): { text: string; cmd: string | null; kind: "spot" | "contact" | "plain" }[] {
    if (!legend) return [];
    return legend.split("  ").map((part) => {
      const marker = w?.markers.find((m) => `${m.glyph} ${m.designator}` === part);
      if (marker?.kind === "spot") {
        return { text: part, cmd: `DRIVE ${marker.glyph}`, kind: "spot" };
      }
      if (marker && marker.kind !== "you" && marker.kind !== "campaign") {
        return { text: part, cmd: `LOCK ${marker.designator}`, kind: "contact" };
      }
      return { text: part, cmd: null, kind: "plain" };
    });
  }

  /** Clicking a contact marker on the arena window locks that contact. */
  function onMapClick(ev: MouseEvent): void {
    const w = win;
    const c = canvas;
    const px = cellPx;
    if (!w || !c || px <= 0) return;
    const rect = c.getBoundingClientRect();
    const scaleX = rect.width > 0 ? c.width / rect.width : 1;
    const scaleY = rect.height > 0 ? c.height / rect.height : 1;
    const mx = (ev.clientX - rect.left) * scaleX;
    const my = (ev.clientY - rect.top) * scaleY;
    const maxDist = Math.max(8, px * 1.8);
    let best: Marker | null = null;
    let bestDistSq = maxDist * maxDist;
    for (const marker of w.markers) {
      // Doc 32 cut CF.4 (Q-CF-50): click-to-lock is the field layer's alone.
      if (marker.kind === "you" || marker.kind === "spot" || marker.kind === "campaign") continue;
      const cx = marker.x * px + px / 2;
      const cy = (w.h - 1 - marker.y) * px + px / 2;
      const d2 = (mx - cx) * (mx - cx) + (my - cy) * (my - cy);
      if (d2 <= bestDistSq) {
        bestDistSq = d2;
        best = marker;
      }
    }
    if (best !== null) {
      app.helm(`LOCK ${best.designator}`);
    }
  }

  /** Double-clicking a cell on the arena window drives to that arena coordinate. */
  function onMapDblClick(ev: MouseEvent): void {
    const w = win;
    const c = canvas;
    const px = cellPx;
    if (!w || !c || px <= 0) return;
    const rect = c.getBoundingClientRect();
    const scaleX = rect.width > 0 ? c.width / rect.width : 1;
    const scaleY = rect.height > 0 ? c.height / rect.height : 1;
    const mx = (ev.clientX - rect.left) * scaleX;
    const my = (ev.clientY - rect.top) * scaleY;
    const col = Math.max(0, Math.min(w.w - 1, Math.floor(mx / px)));
    const row = Math.max(0, Math.min(w.h - 1, Math.floor(my / px)));
    const cellX = w.x0 + col;
    const cellY = w.y0 + (w.h - 1 - row);
    app.helm(`DRIVE ${cellX},${cellY}`);
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
      const palette = biomeTerrainColours(g.surface);
      for (let row = 0; row < w.h; row += 1) {
        const line = w.terrain[row] ?? "";
        for (let col = 0; col < w.w; col += 1) {
          const colour = palette[line.charAt(col)] ?? "transparent";
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
      if (marker.kind === "campaign") {
        // Doc 32 cut CF.4: the server's glyph itself, small — ground, not a unit.
        if (owns[i] !== true) continue;
        ctx.font = `${Math.max(7, Math.floor(px * 0.9))}px ui-monospace, monospace`;
        ctx.textAlign = "center";
        ctx.textBaseline = "middle";
        ctx.fillText(marker.glyph, cx, cy);
        ctx.textAlign = "start";
        continue;
      }
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
    <!-- Doc 21 cut G.3c: the first-person picture top-left, the arena map and contacts stacked beside it. -->
    <div class="top-deck">
      <PictureView {app} headingDeg={frame.heading_deg} {frame} {prefill} />
      <div class="side-column">
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
                style="max-width: min(100%, {win.w * cellPx}px); max-height: 240px; aspect-ratio: {win.w} / {win.h}"
                onclick={onMapClick}
                ondblclick={onMapDblClick}
              ></canvas>
            </div>
            {#if view.legend.length > 0}
              <p class="legendline">{#each legendParts(view.legend, win) as item, idx (idx)}{#if idx > 0}{"  "}{/if}{#if item.cmd}<button
                      type="button"
                      class="legend-btn"
                      data-kind={item.kind}
                      title={item.cmd}
                      onclick={() => app.helm(item.cmd!)}>{item.text}</button>{:else}<span>{item.text}</span>{/if}{/each}</p>
            {/if}
          {/if}
        </section>

        <section class="pane contacts-pane" aria-label="contacts">
          <h4 class="pt">contacts</h4>
          {#each view.contacts as row, i (i)}
            <div
              class="contact"
              class:locked={row.designator === frame.lock_designator}
              data-locked={row.designator === frame.lock_designator ? "true" : undefined}
            >
              <button
                type="button"
                class="desig"
                title={contactText(row)}
                onclick={() => app.helm(`LOCK ${row.designator}`)}>{row.designator}</button
              >
              <span class="detail">{row.detail}</span>
              <button
                type="button"
                class="scanbtn"
                title={`SCAN ${row.designator}`}
                onclick={() => app.helm(`SCAN ${row.designator}`)}>scan</button
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
      </div>
    </div>

    <div class="panes">
      {#if frame?.detachment}
        <section class="pane full" aria-label="detachment">
          <h4 class="pt">detachment</h4>
          {#each detachmentLines(frame.detachment) as line, i (i)}
            <p class="line">{line}</p>
          {/each}
          <div class="detachment-actions">
            <button type="button" onclick={() => prefill(frame!.detachment!.all_order_prefill)}>Order ALL</button>
            {#each frame.detachment.slots as slot (slot.slot)}
              <button type="button" onclick={() => prefill(slot.order_prefill)}>Order {slot.slot}</button>
            {/each}
            {#if frame.detachment.wilco_command}
              <button type="button" onclick={() => app.helm(frame!.detachment!.wilco_command!)}>WILCO</button>
            {/if}
            {#if frame.detachment.negative_command}
              <button type="button" onclick={() => app.helm(frame!.detachment!.negative_command!)}>NEGATIVE</button>
            {/if}
          </div>
        </section>
      {/if}
      <section class="pane" aria-label="status">
        <h4 class="pt">status</h4>
        {#each view.status as line, i (i)}
          <div class="status-row">
            <p class="line">{line}</p>
            {#if i === 1 && frame.power === "off" && !frame.wreck}
              <button
                type="button"
                class="inline-btn"
                onclick={() => app.helm("STARTUP")}>Startup</button
              >
            {:else if i === view.status.length - 2 && frame.ecm !== null}
              <button
                type="button"
                class="inline-btn"
                class:active={frame.ecm === "on"}
                aria-pressed={frame.ecm === "on"}
                title={`EMCON ${frame.ecm === "on" ? "off" : "on"}`}
                onclick={() => app.helm(`EMCON ${frame.ecm === "on" ? "off" : "on"}`)}
              >EMCON</button>
            {:else if i === view.status.length - 1}
              <button
                type="button"
                class="inline-btn"
                class:active={autoTargetArmed}
                aria-pressed={autoTargetArmed}
                title={autoTargetArmed ? "Disable auto-target" : "LOCK nearest WHEN nolock"}
                onclick={toggleAutoTarget}>Auto-target</button
              >
            {/if}
          </div>
          {#if i === 2}
            <div class="speed-control">
              <input
                type="range"
                class="speed-slider"
                aria-label="speed slider"
                min={-reverseKph}
                max={maxKph}
                step="1"
                value={currentSliderKph}
                oninput={onSpeedSlider}
                onchange={onSpeedSlider}
              />
              <button
                type="button"
                class="inline-btn stop-btn"
                title="SPEED 0 QUIET"
                onclick={stopSpeed}>Stop</button
              >
            </div>
          {/if}
        {/each}
      </section>

      <section class="pane damage-pane" aria-label="damage">
        <h4 class="pt">damage</h4>
        {#if look?.doll && frame}
          <DollView doll={look.doll} sections={frame.sections} label={look.label} />
        {/if}
        <div class="damage-grid">
          {#each view.damage as row, i (i)}
            <div class="damage-section">
              <p class="line damage-head" title={damageText(row)}>
                <span class="rowid">{row.id}</span>
                <span class="cond">{row.condition}</span>
                <span class="frac">{row.internals}</span>
              </p>
              <div class="facets">
                {#each row.facets as facet, f (f)}
                  <div class="facet">
                    <span class="barlabel">{facet.id} {facet.armor}</span>
                  </div>
                {/each}
              </div>
            </div>
          {/each}
        </div>
        {#if view.damage.length === 0}
          <p class="muted">Nothing here.</p>
        {/if}
      </section>

      <section class="pane" aria-label="weapons">
        <h4 class="pt">weapons</h4>
        {#if frame?.mind}<button type="button" onclick={() => { app.mindReadoutOpen = true; app.refreshMindTrace(); }}>Card &amp; trace</button>{/if}
        {#if frame.weapons.length > 0}
          <div class="wf-controls" aria-label="weapons free controls">
            <button
              type="button"
              class="inline-btn wf-toggle"
              class:active={fireRuleArmed}
              aria-pressed={fireRuleArmed}
              aria-label="weapons free toggle"
              title={fireRuleArmed
                ? `FIRE ${fireRuleWeapon} OFF`
                : `FIRE ${fireRuleWeapon} WHEN odds >= ${fireRuleOdds}`}
              onclick={toggleFireRule}>{gateWord(frame.weapons_free)}</button
            >
            <select
              class="wf-select"
              aria-label="weapons free weapon"
              value={fireRuleWeapon}
              onchange={onFireWeaponChange}
            >
              <option value="ALL">ALL</option>
              {#each frame.weapons as w (w.id)}
                <option value={w.id}>{w.id}</option>
              {/each}
            </select>
            <input
              type="range"
              class="wf-slider"
              aria-label="weapons free odds"
              min="10"
              max="100"
              step="5"
              value={fireRuleOdds}
              oninput={onFireOddsChange}
              onchange={onFireOddsChange}
            />
            <span class="wf-odds">&gt;={fireRuleOdds}%</span>
            <button
              type="button"
              class="inline-btn volley-btn"
              title="FIRE ALL"
              onclick={() => app.helm("FIRE ALL")}>Volley</button
            >
          </div>
        {/if}
        {#if frame.seated && frame.rules.length > 0}
          <div class="rule-pills" aria-label="active rules">
            {#each frame.rules as rule (rule.slot)}
              <button
                type="button"
                class="inline-btn rule-pill"
                title={`MIND RULE ${rule.slot} OFF`}
                onclick={() => app.helm(`MIND RULE ${rule.slot} OFF`)}
              >S{rule.slot} {rule.action} ×</button>
            {/each}
          </div>
        {/if}
        {#each view.weapons as line, i (i)}
          {@const weapon = weaponForLine(line)}
          {#if weapon !== null}
            <p class="line"><button
                type="button"
                class="weapon-btn"
                class:ready={weapon.ready}
                class:cycling={!weapon.ready}
                data-ready={weapon.ready ? "true" : "false"}
                style={!weapon.ready && weapon.ready_in_ms > 0
                  ? `--cooldown-pct: ${Math.min(100, Math.max(10, Math.round((weapon.ready_in_ms / 5000) * 100)))}%`
                  : undefined}
                title={`FIRE ${weapon.id}`}
                onclick={() => app.helm(`FIRE ${weapon.id}`)}>{line}</button></p>
          {:else}
            <p class="line">{line}</p>
          {/if}
        {/each}
        <!-- 17b cut M5.5: the mind's four lines, the section's tail (in
             `(no weapon)`'s place on an unarmed hull). On the profile row each
             sealed profile the mind is NOT flying is a button that switches to
             `MIND PROFILE <name>`; the flown one is plain text, since the
             cockpit refuses a switch to it (the review's D4); the row's text
             is `mindLines`' own. -->
        {#each view.mind as line, i (i)}
          {#if i === 1 && view.profiles.length > 0}
            <p class="line mind profiles" title={line}>{#each view.profiles as profile, p (p)}<span
                  class="lead">{profile.lead}</span>{#if profile.active}<span class="flown"
                    >{profile.name}</span
                  >{:else}<button
                    type="button"
                    class="profile"
                    title={`MIND PROFILE ${profile.name}`}
                    onclick={() => app.helm(`MIND PROFILE ${profile.name}`)}>{profile.name}</button
                  >{/if}{/each}</p>
          {:else}
            <p class="line mind">{line}</p>
          {/if}
        {/each}
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
    padding: 0.5rem 0.65rem;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .top-deck {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-start;
    gap: 0.5rem;
  }

  .side-column {
    flex: 1 1 18rem;
    min-width: 15rem;
    max-width: 28rem;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .mapbox {
    min-width: 0;
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
    max-height: 260px;
    margin: 0.3rem 0;
    display: flex;
    justify-content: center;
  }

  canvas {
    image-rendering: pixelated;
    display: block;
    background: var(--bg);
    cursor: pointer;
    width: auto;
    height: auto;
  }

  .legendline {
    margin: 0;
    font-family: var(--font-mono);
    font-size: 0.78rem;
    color: var(--fg-muted);
    white-space: pre-wrap;
  }

  .legend-btn {
    background: none;
    border: none;
    border-bottom: 1px dotted var(--border);
    color: var(--fg-muted);
    cursor: pointer;
    font: inherit;
    padding: 0;
  }

  .legend-btn:hover {
    color: var(--accent, #6aa0ff);
  }

  .panes {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(14rem, 1fr));
    gap: 0.5rem;
    align-items: start;
  }

  .pane {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    padding: 0.45rem 0.6rem;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-panel);
    min-width: 0;
    max-height: 38vh;
    overflow-y: auto;
    overflow-x: auto;
  }

  .contacts-pane {
    max-height: 24vh;
  }

  .pane.full {
    grid-column: 1 / -1;
  }

  .pane.wide {
    grid-column: span 2;
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

  .status-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.35rem;
  }

  .inline-btn {
    background: none;
    border: 1px solid var(--border);
    border-radius: 4px;
    color: var(--fg-muted);
    cursor: pointer;
    font-family: var(--font-mono);
    font-size: 0.72rem;
    padding: 0.05rem 0.4rem;
    line-height: 1.2;
    white-space: nowrap;
  }

  .inline-btn:hover {
    color: var(--fg);
    border-color: var(--fg-muted);
  }

  .inline-btn.active {
    color: var(--fg);
    border-color: var(--accent, #6aa0ff);
    background: var(--bg-input);
  }

  .speed-control {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    margin: 0.05rem 0 0.1rem;
  }

  .speed-slider {
    width: 100%;
    accent-color: var(--accent, #6aa0ff);
    cursor: pointer;
  }

  .wf-controls {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.25rem;
    margin: 0.1rem 0 0.2rem;
    font-family: var(--font-mono);
    font-size: 0.75rem;
  }

  .rule-pills {
    display: flex;
    flex-wrap: wrap;
    gap: 0.25rem;
    margin: 0.05rem 0 0.15rem;
  }

  .rule-pill:hover {
    color: var(--error);
    border-color: var(--error);
  }

  .wf-select {
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 4px;
    color: var(--fg);
    font: inherit;
    padding: 0.05rem 0.25rem;
  }

  .wf-slider {
    flex: 1 1 3.5rem;
    min-width: 3.5rem;
    accent-color: var(--accent, #6aa0ff);
    cursor: pointer;
  }

  .wf-odds {
    color: var(--fg-muted);
    min-width: 2.6rem;
  }

  .weapon-btn {
    background: none;
    border: none;
    border-bottom: 1px dotted var(--border);
    color: var(--fg);
    cursor: pointer;
    font: inherit;
    padding: 0.05rem 0.2rem;
    border-radius: 3px;
    text-align: left;
    white-space: pre;
    display: inline-block;
  }

  .weapon-btn.ready {
    border-left: 2px solid var(--success);
    padding-left: 0.3rem;
  }

  .weapon-btn.cycling {
    border-left: 2px solid var(--warn);
    padding-left: 0.3rem;
    background: linear-gradient(
      90deg,
      rgba(232, 194, 104, 0.18) var(--cooldown-pct, 30%),
      transparent var(--cooldown-pct, 30%)
    );
  }

  .weapon-btn:hover {
    color: var(--accent, #6aa0ff);
  }

  .damage-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(7.2rem, 1fr));
    gap: 0.2rem 0.45rem;
  }

  .damage-section {
    min-width: 0;
  }

  .damage-head {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 0.2rem;
    font-size: 0.75rem;
  }

  .rowid {
    display: inline-block;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .detachment-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
    margin-top: 0.3rem;
  }

  .cond {
    color: var(--fg-muted);
    display: inline-block;
    min-width: 0;
  }

  .frac {
    color: var(--fg-muted);
  }

  .facets {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 0.04rem 0.3rem;
    margin-left: 0.15rem;
  }

  .facet {
    min-width: 0;
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
    padding: 0.05rem 0.2rem;
    border-radius: 3px;
  }

  .contact.locked {
    border-left: 2px solid var(--accent, #6aa0ff);
    background: rgba(110, 168, 254, 0.12);
    padding-left: 0.35rem;
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
