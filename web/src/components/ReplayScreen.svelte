<script lang="ts">
  // The replay screen (16b cut I.2; 16c N9, Q37): a finished match on screen 0,
  // both sides, scrubbed at the arena's own clock — the second layout of the
  // cockpit's digit, mounted by `Game.svelte` while a replay is open and no
  // seat is held.
  //
  // It renders ONLY the replay store: the document the client holds, the
  // ground it was fought on, and where the viewer is in it. There is no lookup
  // into any other store — not who, not presence, not the unit feed — so no
  // name, object number or pilot can reach this screen that the document did
  // not carry, and the document carries none (Q37: every tank by the callsign
  // the arena gave it). Every string comes out of `replayView`, and the canvas
  // draws `composeReplayWindow` — both pure, both pinned by vitest against the
  // committed fixtures (`fixtures/replay/`).
  //
  // **Interpolation is the renderer's drawing, and the screen says so**: off a
  // keyframe, `frameAt` moves every unit in a straight line and turns it along
  // the shorter arc, and the label under the canvas reads `Between keyframes 19
  // and 20: drawn, not recorded.`
  //
  // The keys (`,` `.` `u` `x` `p`) are the cockpit's second table, dispatched
  // by the shell (`GameApp.localAction`); the scrubber, play and the units'
  // buttons below drive the same store.
  //
  // **The live picture (16b cut I.4; 16c N10)** is the same screen in LIVE
  // mode (`app.replay.live`): a staffer's `@arena/watch` picture, pushed at
  // most once an arena second and held by the store as a one-keyframe
  // document, drawn by the same `composeReplayWindow` and worded by
  // `liveView` — no scrubber and no play (nothing steps it), the label `Live:
  // the arena's own positions, at most once an arena second.`, a status pane
  // in place of the shots and the wrecks, and every unit's object number on
  // purpose (a staff surface; the picture carries it).
  import type { GameApp } from "$lib/game.svelte";
  import {
    COCKPIT_VIEW_CELLS,
    labelPlan,
    markerOwners,
    terrainCacheKey,
    WAITING_FOR_THE_MAP,
    type ArenaMap,
    type ComposedWindow,
    type Marker,
  } from "$lib/cockpit";
  import {
    composeReplayWindow,
    frameAt,
    liveView,
    replayView,
    WAITING_FOR_THE_REPLAY,
  } from "$lib/replay";
  import {
    FRAME_H,
    FRAME_W,
    LocalReplayPicture,
    localCapable,
    pixelModule,
    type ReplayCameraMode,
  } from "$lib/pixel/local";
  import { TERRAIN_COLOURS } from "$lib/stores/map.svelte";

  let { app }: { app: GameApp } = $props();

  let camera = $state<ReplayCameraMode>("overview");
  let zoomPermille = $state(1000);
  let localReplay = $state<LocalReplayPicture | null>(null);

  $effect(() => {
    if (!localCapable()) return;
    let cancelled = false;
    void pixelModule()
      .then((m) => {
        if (!cancelled) {
          localReplay = new LocalReplayPicture(m);
        }
      })
      .catch(() => {
        // Fallback 2D canvas drawing handles environments without WASM.
      });
    return () => {
      cancelled = true;
    };
  });

  function cycleCamera(): void {
    camera =
      camera === "overview"
        ? "iso25d"
        : camera === "iso25d"
          ? "first_person"
          : "overview";
    localReplay?.forget();
  }

  function onWindowKeydown(ev: KeyboardEvent): void {
    const tag = (ev.target as HTMLElement | null)?.tagName;
    if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;
    if (ev.ctrlKey || ev.altKey || ev.metaKey) return;
    if (ev.key === "v") {
      ev.preventDefault();
      cycleCamera();
    } else if (ev.key === "z" && camera === "iso25d") {
      ev.preventDefault();
      zoomPermille = Math.min(2000, zoomPermille + 200);
      localReplay?.forget();
    } else if (ev.key === "Z" && camera === "iso25d") {
      ev.preventDefault();
      zoomPermille = Math.max(500, zoomPermille - 200);
      localReplay?.forget();
    }
  }

  /** What a replay's markers are painted in: red, blue, a wreck's grey, and a
   * named spot in the cockpit's own sand tone. */
  const MARKER_COLOURS: Record<Marker["kind"], string> = {
    red: "#e05a5a",
    blue: "#4f7fd1",
    wreck: "#777",
    spot: "#d8c8a0",
    you: "#f2c94c",
    unknown: "#b9a23a",
    friendly: "#4fa86a",
    hostile: "#e05a5a",
    // Doc 32 cut CF.4: a cockpit kind a replay never composes.
    campaign: "#c8b88a",
  };

  /** Play steps the cursor by this much every this many milliseconds: 1×
   * arena time. */
  const PLAY_STEP_MS = 100;

  let canvas: HTMLCanvasElement | undefined = $state();

  const doc = $derived(app.replay.doc);
  const ground = $derived(app.replay.holdsTheGround ? app.replay.map : null);
  const frame = $derived(doc === null ? null : frameAt(doc, app.replay.cursorMs));
  const win = $derived<ComposedWindow | null>(
    doc !== null && frame !== null && ground !== null
      ? composeReplayWindow(ground, doc, frame, app.replay.focus, COCKPIT_VIEW_CELLS, COCKPIT_VIEW_CELLS)
      : null,
  );
  // 16b cut I.4: a live picture is worded by `liveView` from the picture itself.
  const live = $derived(app.replay.live);
  const observer = $derived(app.replay.observer);
  const view = $derived(
    live && observer !== null
      ? liveView(observer, win)
      : doc !== null && frame !== null
        ? replayView(doc, frame, win, app.replay.cursorMs)
        : null,
  );
  const focusCallsign = $derived(doc?.roster[app.replay.focus]?.callsign ?? null);

  const CANVAS_MAX = 560;
  const cellPx = $derived(
    win === null ? 0 : Math.max(2, Math.min(12, Math.floor(CANVAS_MAX / Math.max(win.w, win.h, 1)))),
  );

  // Play at 1× arena time: one step of `PLAY_STEP_MS` of arena time every
  // `PLAY_STEP_MS` of wall time, stopping at the end.
  $effect(() => {
    if (!app.replay.playing) return;
    const timer = setInterval(() => {
      const next = app.replay.cursorMs + PLAY_STEP_MS;
      app.replay.seek(next);
      if (next >= app.replay.endMs) {
        app.replay.playing = false;
      }
    }, PLAY_STEP_MS);
    return () => clearInterval(timer);
  });

  function togglePlay(): void {
    if (app.replay.playing) {
      app.replay.playing = false;
      return;
    }
    if (app.replay.cursorMs >= app.replay.endMs) {
      app.replay.seek(0);
    }
    app.replay.playing = true;
  }

  function onScrub(ev: Event): void {
    const value = Number((ev.currentTarget as HTMLInputElement).value);
    app.replay.seek(Number.isFinite(value) ? value : 0);
  }

  // The terrain cache, as the cockpit's: painted once per window and map.
  let terrainCanvas: HTMLCanvasElement | null = null;
  let terrainKey = "";
  let terrainGround: ArenaMap | null = null;

  $effect(() => {
    const w = win;
    const c = canvas;
    const g = ground;
    const d = doc;
    const mode = camera;
    const z = zoomPermille;
    const cursorMs = app.replay.cursorMs;
    const focusIdx = app.replay.focus;
    if (!w || !c || !g || !d) return;
    const ctx = c.getContext("2d");
    if (!ctx) return;

    if (mode !== "overview") {
      c.width = FRAME_W;
      c.height = FRAME_H;
      if (localReplay !== null && typeof ctx.putImageData === "function") {
        localReplay.forget();
        if (localReplay.draw(d, g, d.match, cursorMs, focusIdx, mode, z)) {
          ctx.putImageData(localReplay.image(), 0, 0);
          return;
        }
      }
      // 2D follow-cam fallback when WASM is unavailable (e.g., jsdom tests).
      ctx.clearRect(0, 0, c.width, c.height);
      ctx.fillStyle = mode === "first_person" ? "#1c2434" : "#121820";
      ctx.fillRect(0, 0, c.width, c.height);
      ctx.fillStyle = "#e6edf3";
      ctx.font = "14px ui-monospace, monospace";
      ctx.fillText(
        `${mode === "iso25d" ? "TACTICAL FOLLOW" : "COCKPIT FOLLOW"} ${focusCallsign ?? ""}`,
        16,
        28,
      );
      return;
    }

    const px = cellPx;
    c.width = w.w * px;
    c.height = w.h * px;
    ctx.clearRect(0, 0, c.width, c.height);
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
    // The focus is labelled at every zoom (it takes the lock's place in the
    // plan); every other label as the cockpit's rule places it.
    const labels = labelPlan(w.markers, focusCallsign, px);
    const owns = markerOwners(w.markers);
    for (const [i, marker] of w.markers.entries()) {
      const cx = marker.x * px + px / 2;
      // `marker.y` counts from the SOUTH; the canvas paints north-up.
      const cy = (w.h - 1 - marker.y) * px + px / 2;
      const colour = MARKER_COLOURS[marker.kind];
      if (marker.kind === "spot") {
        if (owns[i] !== true) continue;
        const side = Math.max(4, px * 0.9);
        ctx.lineWidth = 1;
        ctx.strokeStyle = colour;
        ctx.strokeRect(cx - side / 2, cy - side / 2, side, side);
      } else {
        ctx.fillStyle = colour;
        ctx.beginPath();
        ctx.arc(cx, cy, Math.max(2, px * 0.45), 0, Math.PI * 2);
        ctx.fill();
        if (marker.designator === focusCallsign) {
          ctx.lineWidth = Math.max(1, px * 0.2);
          ctx.strokeStyle = "#fff8dc";
          ctx.beginPath();
          ctx.arc(cx, cy, Math.max(4, px * 0.85), 0, Math.PI * 2);
          ctx.stroke();
        }
      }
      if (labels[i] === true) {
        ctx.fillStyle = colour;
        ctx.textBaseline = "middle";
        ctx.font = `${Math.max(8, px)}px ui-monospace, monospace`;
        ctx.fillText(marker.designator, cx + px * 0.8, cy);
      }
    }
  });
</script>

<svelte:window onkeydown={onWindowKeydown} />

<div class="replay">
  {#if doc === null || view === null}
    <p class="muted pad">{WAITING_FOR_THE_REPLAY}</p>
  {:else}
    <section class="mapbox" aria-label="replay map">
      <header class="map-head">
        <strong class="head">{view.header}</strong>
        <div class="view-bar" role="group" aria-label="replay camera">
          <button
            type="button"
            class="view-btn"
            class:active={camera === "overview"}
            onclick={() => (camera = "overview")}>overview</button
          >
          <button
            type="button"
            class="view-btn"
            class:active={camera === "iso25d"}
            onclick={() => (camera = "iso25d")}>tactical</button
          >
          <button
            type="button"
            class="view-btn"
            class:active={camera === "first_person"}
            onclick={() => (camera = "first_person")}>cockpit</button
          >
        </div>
      </header>
      {#if win === null}
        <p class="muted">{WAITING_FOR_THE_MAP}</p>
      {:else}
        <div class="canvas-wrap">
          <canvas
            bind:this={canvas}
            aria-label="replay window"
            style="width: {camera === 'overview' ? win.w * cellPx : FRAME_W}px; height: {camera === 'overview' ? win.h * cellPx : FRAME_H}px"
          ></canvas>
        </div>
      {/if}
      <p class="label">{view.label}</p>
      {#if live}
        <p class="clock">{view.clock}</p>
      {:else}
        <div class="scrub">
          <button type="button" class="play" aria-label={app.replay.playing ? "pause" : "play"} onclick={togglePlay}
            >{app.replay.playing ? "pause" : "play"}</button
          >
          <input
            type="range"
            aria-label="arena time"
            min="0"
            max={doc.end_ms}
            step="100"
            value={app.replay.cursorMs}
            oninput={onScrub}
          />
          <span class="clock">{view.clock}</span>
        </div>
      {/if}
      {#if view.legend.length > 0}
        <p class="legendline">{view.legend}</p>
      {/if}
    </section>

    <div class="panes">
      <section class="pane" aria-label="units">
        <h4 class="pt">units</h4>
        {#each view.units as row, i (i)}
          <p class="line">
            <button
              type="button"
              class="unit"
              class:focus={i === app.replay.focus}
              onclick={() => (app.replay.focus = i)}>{row.callsign}</button
            >
            <span class="side" data-side={row.side}>{row.side}</span>
            <span class="where">{row.where}</span>
          </p>
        {/each}
      </section>

      {#if live}
        <section class="pane" aria-label="status">
          <h4 class="pt">status</h4>
          {#each view.status as line, i (i)}
            <p class="line">{line}</p>
          {/each}
        </section>
        {:else}
        <section class="pane" aria-label="shots">
          <h4 class="pt">shots</h4>
          {#each view.shots as line, i (i)}
            <p class="line">{line}</p>
          {/each}
          {#if view.shots.length === 0}
            <p class="muted">No shots yet.</p>
          {/if}
        </section>

        <section class="pane" aria-label="wrecks">
          <h4 class="pt">wrecks</h4>
          {#each view.wrecks as line, i (i)}
            <p class="line">{line}</p>
          {/each}
          {#if view.wrecks.length === 0}
            <p class="muted">No wrecks yet.</p>
          {/if}
        </section>
      {/if}
    </div>

    {#each view.truncation as line, i (i)}
      <p class="cut">{line}</p>
    {/each}
  {/if}
</div>

<style>
  .replay {
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
    font-family: var(--font-mono);
    font-size: 0.85rem;
  }

  .map-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.6rem;
    flex-wrap: wrap;
  }

  .view-bar {
    display: inline-flex;
    gap: 0.25rem;
  }

  .view-btn {
    background: none;
    border: 1px solid var(--border);
    border-radius: 4px;
    color: var(--fg-muted);
    cursor: pointer;
    font-family: var(--font-mono);
    font-size: 0.72rem;
    padding: 0.05rem 0.45rem;
  }

  .view-btn.active {
    border-color: var(--accent);
    color: var(--accent);
    font-weight: 600;
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

  .label,
  .legendline,
  .cut {
    margin: 0;
    font-family: var(--font-mono);
    font-size: 0.78rem;
    color: var(--fg-muted);
  }

  .scrub {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin: 0.3rem 0;
  }

  .scrub input {
    flex: 1;
  }

  .play {
    background: none;
    border: 1px solid var(--border);
    border-radius: 4px;
    color: var(--fg-muted);
    cursor: pointer;
    font: inherit;
    font-size: 0.8rem;
    padding: 0.05rem 0.5rem;
  }

  .clock {
    font-family: var(--font-mono);
    font-size: 0.8rem;
    color: var(--fg-muted);
  }

  .panes {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(16rem, 1fr));
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

  .unit {
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

  .unit.focus {
    font-weight: 700;
  }

  .side {
    display: inline-block;
    min-width: 3rem;
  }

  .side[data-side="red"] {
    color: #e05a5a;
  }

  .side[data-side="blue"] {
    color: #4f7fd1;
  }

  .where {
    color: var(--fg-muted);
  }

  .muted {
    color: var(--fg-muted);
    font-size: 0.85rem;
    margin: 0;
  }

  .pad {
    padding: 0.8rem;
  }
</style>
