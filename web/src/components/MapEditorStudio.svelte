<script lang="ts">
  // Unity-style Terrain & Arena Map Studio: interactive 2D grid with brush
  // footprints, elevation sculpting (raise, lower, flatten, smooth, linear ramp),
  // line/rectangle/flood-fill tools, muster & spot placement, undo/redo stack,
  // live 3-view `mush-pixel-wasm` (1st-person 3D, 2.5D, overhead) preview, and
  // `ArenaDef` / `mush_atlas::Cells` base64 import & export.
  import { onMount } from "svelte";
  import type { GameApp } from "$lib/game.svelte";
  import {
    applyBrushDab,
    applyFloodFill,
    applyLineStroke,
    applyRampStroke,
    applyRectStroke,
    bresenhamCells,
    brushFootprint,
    cloneMapState,
    EDITOR_TERRAINS,
    editorToPictureFeeds,
    encodeTileCellsBase64,
    exportArenaDefJson,
    generatePresetMap,
    importArenaDefJson,
    terrainByCode,
    type BiomeId,
    type BrushShape,
    type EditorMapState,
    type EditorTool,
    type PresetTemplateId,
  } from "$lib/map-editor";
  import { biomeTerrainColours } from "$lib/stores/map.svelte";
  import type { PictureKind } from "$lib/picture";
  import {
    FRAME_H,
    FRAME_W,
    LocalPicture,
    localCapable,
    pixelModule,
  } from "$lib/pixel/local";

  let { app }: { app: GameApp } = $props();

  const MAX_UNDO = 35;

  let mapState = $state<EditorMapState>(generatePresetMap("proving_ground", 64, 64, 42));
  let undoStack = $state<EditorMapState[]>([]);
  let redoStack = $state<EditorMapState[]>([]);

  let preset = $state<PresetTemplateId>("proving_ground");
  let presetSize = $state<number>(64);
  let seed = $state<number>(42);

  let tool = $state<EditorTool>("paint");
  let brushShape = $state<BrushShape>("circle");
  let brushRadius = $state<number>(2);
  let terrainCode = $state<number>(3); // Grass
  let densityPct = $state<number>(100);
  let elevStepCm = $state<number>(35);
  let targetElevCm = $state<number>(200);
  let hollowRect = $state<boolean>(false);
  let rampPavesRoad = $state<boolean>(true);
  let spotLetter = $state<string>("C");
  let spotName = $state<string>("High Ridge");

  let showGridLines = $state<boolean>(true);
  let showContours = $state<boolean>(true);
  let showExportDrawer = $state<boolean>(false);
  let jsonBuffer = $state<string>("");
  let statusMsg = $state<string>("");

  let hoverCell = $state<[number, number] | null>(null);
  let isDragging = $state<boolean>(false);
  let dragStart = $state<[number, number] | null>(null);
  let strokeSeed = $state<number>(101);

  let gridCanvas: HTMLCanvasElement | undefined = $state();
  let previewCanvas: HTMLCanvasElement | undefined = $state();
  let renderer = $state<LocalPicture | null>(null);
  let previewView = $state<PictureKind>("cockpit");

  const GRID_MAX_PX = 540;

  const cellPx = $derived(
    Math.max(4, Math.min(14, Math.floor(GRID_MAX_PX / Math.max(mapState.width, mapState.height, 1)))),
  );

  const hoveredInfo = $derived.by(() => {
    if (!hoverCell) return null;
    const [x, y] = hoverCell;
    if (x < 0 || x >= mapState.width || y < 0 || y >= mapState.height) return null;
    const idx = y * mapState.width + x;
    const code = mapState.classes[idx] ?? 0;
    const elev = mapState.elevCm[idx] ?? 0;
    const left = x > 0 ? (mapState.elevCm[idx - 1] ?? elev) : elev;
    const down = y > 0 ? (mapState.elevCm[idx - mapState.width] ?? elev) : elev;
    const maxDelta = Math.max(Math.abs(elev - left), Math.abs(elev - down));
    const gradePct = (maxDelta / (mapState.cellCm || 1000)) * 100;
    return {
      x,
      y,
      terrain: terrainByCode(code),
      elevM: (elev / 100).toFixed(2),
      gradePct: gradePct.toFixed(1),
    };
  });

  onMount(() => {
    if (!localCapable()) return;
    let cancelled = false;
    void pixelModule()
      .then((m) => {
        if (!cancelled) renderer = new LocalPicture(m);
      })
      .catch(() => {
        // Ignore in headless test environments without WASM.
      });
    return () => {
      cancelled = true;
    };
  });

  function pushUndo(): void {
    undoStack = [...undoStack.slice(-(MAX_UNDO - 1)), cloneMapState(mapState)];
    redoStack = [];
  }

  function undo(): void {
    const prev = undoStack[undoStack.length - 1];
    if (!prev) return;
    redoStack = [...redoStack, cloneMapState(mapState)];
    undoStack = undoStack.slice(0, -1);
    mapState = prev;
  }

  function redo(): void {
    const next = redoStack[redoStack.length - 1];
    if (!next) return;
    undoStack = [...undoStack, cloneMapState(mapState)];
    redoStack = redoStack.slice(0, -1);
    mapState = next;
  }

  function applyPreset(): void {
    pushUndo();
    mapState = generatePresetMap(preset, presetSize, presetSize, seed);
    statusMsg = `Generated ${mapState.label} (${mapState.width}×${mapState.height}, seed ${seed}).`;
  }

  function setBiome(biome: BiomeId): void {
    mapState = { ...mapState, biome };
  }

  function rotateCamera(deltaDeg: number): void {
    const next = (mapState.camera.headingDeg + deltaDeg + 360) % 360;
    mapState = {
      ...mapState,
      camera: { ...mapState.camera, headingDeg: next },
    };
  }

  function cyclePreviewView(): void {
    previewView =
      previewView === "cockpit"
        ? "iso25d"
        : previewView === "iso25d"
          ? "overhead"
          : "cockpit";
  }

  function cellFromMouse(ev: MouseEvent): [number, number] | null {
    const c = gridCanvas;
    const px = cellPx;
    if (!c || px <= 0) return null;
    const rect = c.getBoundingClientRect();
    const scaleX = rect.width > 0 ? c.width / rect.width : 1;
    const scaleY = rect.height > 0 ? c.height / rect.height : 1;
    const mx = (ev.clientX - rect.left) * scaleX;
    const my = (ev.clientY - rect.top) * scaleY;
    const col = Math.floor(mx / px);
    const rowFromNorth = Math.floor(my / px);
    const y = mapState.height - 1 - rowFromNorth;
    if (col < 0 || col >= mapState.width || y < 0 || y >= mapState.height) return null;
    return [col, y];
  }

  function applySingleCellAction(x: number, y: number): void {
    if (tool === "eyedrop") {
      const idx = y * mapState.width + x;
      terrainCode = mapState.classes[idx] ?? 3;
      targetElevCm = mapState.elevCm[idx] ?? 0;
      statusMsg = `Sampled ${terrainByCode(terrainCode).label} at (${x}, ${y}), elev ${(targetElevCm / 100).toFixed(2)} m.`;
      return;
    }
    if (tool === "camera") {
      mapState = {
        ...mapState,
        camera: { ...mapState.camera, x, y },
      };
      return;
    }
    if (tool === "spot") {
      const letter = spotLetter.trim().slice(0, 1).toUpperCase() || "A";
      const name = spotName.trim() || `Spot ${letter}`;
      const existing = mapState.spots.filter(
        (s) => s.letter !== letter && (s.cell[0] !== x || s.cell[1] !== y),
      );
      mapState = {
        ...mapState,
        spots: [...existing, { letter, name, cell: [x, y] }],
      };
      statusMsg = `Placed spot ${letter} (${name}) at (${x}, ${y}).`;
      return;
    }
    if (tool === "fill") {
      const next = cloneMapState(mapState);
      applyFloodFill(next, x, y, terrainCode);
      mapState = next;
      return;
    }
    if (
      tool === "paint" ||
      tool === "raise" ||
      tool === "lower" ||
      tool === "flatten" ||
      tool === "smooth"
    ) {
      const next = cloneMapState(mapState);
      applyBrushDab(next, x, y, {
        tool,
        terrainCode,
        radius: brushRadius,
        shape: brushShape,
        densityPct,
        elevStepCm,
        targetElevCm,
        strokeSeed,
      });
      mapState = next;
    }
  }

  function onCanvasPointerDown(ev: MouseEvent): void {
    const cell = cellFromMouse(ev);
    if (!cell) return;
    hoverCell = cell;
    // Shift+Click anywhere moves/aims the 3D preview camera pin immediately.
    if (ev.shiftKey) {
      mapState = {
        ...mapState,
        camera: { ...mapState.camera, x: cell[0], y: cell[1] },
      };
      return;
    }
    if (ev.altKey) {
      const idx = cell[1] * mapState.width + cell[0];
      terrainCode = mapState.classes[idx] ?? 3;
      targetElevCm = mapState.elevCm[idx] ?? 0;
      return;
    }
    if (tool !== "eyedrop" && tool !== "camera") {
      pushUndo();
    }
    strokeSeed = (strokeSeed + 37) % 10000;
    isDragging = true;
    dragStart = cell;
    if (
      tool === "paint" ||
      tool === "raise" ||
      tool === "lower" ||
      tool === "flatten" ||
      tool === "smooth" ||
      tool === "fill" ||
      tool === "eyedrop" ||
      tool === "spot" ||
      tool === "camera"
    ) {
      applySingleCellAction(cell[0], cell[1]);
    }
  }

  function onCanvasPointerMove(ev: MouseEvent): void {
    const cell = cellFromMouse(ev);
    hoverCell = cell;
    if (!isDragging || !cell) return;
    if (
      tool === "paint" ||
      tool === "raise" ||
      tool === "lower" ||
      tool === "flatten" ||
      tool === "smooth" ||
      tool === "camera"
    ) {
      strokeSeed = (strokeSeed + 13) % 10000;
      applySingleCellAction(cell[0], cell[1]);
    }
  }

  function onCanvasPointerUp(ev: MouseEvent): void {
    if (!isDragging) return;
    const endCell = cellFromMouse(ev) ?? hoverCell;
    const start = dragStart;
    isDragging = false;
    dragStart = null;
    if (!start || !endCell) return;

    if (tool === "line") {
      const next = cloneMapState(mapState);
      applyLineStroke(next, start[0], start[1], endCell[0], endCell[1], {
        tool: "paint",
        terrainCode,
        radius: brushRadius,
        shape: brushShape,
        densityPct: 100,
        elevStepCm,
        targetElevCm,
      });
      mapState = next;
    } else if (tool === "ramp") {
      const next = cloneMapState(mapState);
      applyRampStroke(
        next,
        start[0],
        start[1],
        endCell[0],
        endCell[1],
        brushRadius,
        rampPavesRoad ? 7 : null,
      );
      mapState = next;
      statusMsg = `Carved graded ramp from (${start[0]}, ${start[1]}) to (${endCell[0]}, ${endCell[1]}).`;
    } else if (tool === "rect") {
      const next = cloneMapState(mapState);
      applyRectStroke(
        next,
        start[0],
        start[1],
        endCell[0],
        endCell[1],
        terrainCode,
        hollowRect,
      );
      mapState = next;
    } else if (tool === "muster_red" || tool === "muster_blue") {
      const x0 = Math.min(start[0], endCell[0]);
      const y0 = Math.min(start[1], endCell[1]);
      const x1 = Math.max(start[0], endCell[0]);
      const y1 = Math.max(start[1], endCell[1]);
      if (tool === "muster_red") {
        mapState = {
          ...mapState,
          redMuster: { ...mapState.redMuster, rect: [x0, y0, x1, y1] },
        };
      } else {
        mapState = {
          ...mapState,
          blueMuster: { ...mapState.blueMuster, rect: [x0, y0, x1, y1] },
        };
      }
    }
  }

  function adjustLightness(hex: string, delta: number): string {
    if (!hex.startsWith("#") || hex.length !== 7 || delta === 0) return hex;
    const r = Math.max(0, Math.min(255, parseInt(hex.slice(1, 3), 16) + delta));
    const g = Math.max(0, Math.min(255, parseInt(hex.slice(3, 5), 16) + delta));
    const b = Math.max(0, Math.min(255, parseInt(hex.slice(5, 7), 16) + delta));
    return `#${r.toString(16).padStart(2, "0")}${g.toString(16).padStart(2, "0")}${b.toString(16).padStart(2, "0")}`;
  }

  // Draw the 2D Editing Grid + Elevation Shading + Muster/Spot overlays + Unity Brush Gizmo.
  $effect(() => {
    const c = gridCanvas;
    const st = mapState;
    const px = cellPx;
    if (!c || px <= 0) return;
    c.width = st.width * px;
    c.height = st.height * px;
    const ctx = c.getContext("2d");
    if (!ctx) return;
    ctx.clearRect(0, 0, c.width, c.height);

    const palette = biomeTerrainColours(st.biome);
    for (let y = 0; y < st.height; y += 1) {
      const screenRow = st.height - 1 - y;
      const py = screenRow * px;
      for (let x = 0; x < st.width; x += 1) {
        const idx = y * st.width + x;
        const code = st.classes[idx] ?? 0;
        const glyph = terrainByCode(code).glyph;
        const base = palette[glyph] ?? "#22262c";
        if (base === "transparent") continue;
        let fill = base;
        if (showContours) {
          const elev = st.elevCm[idx] ?? 0;
          const west = x > 0 ? (st.elevCm[idx - 1] ?? elev) : elev;
          const north = y + 1 < st.height ? (st.elevCm[idx + st.width] ?? elev) : elev;
          const slopeShade = Math.round(((west - elev) + (elev - north)) * 0.35);
          const heightShade = Math.round(elev * 0.04);
          fill = adjustLightness(base, slopeShade + heightShade);
        }
        ctx.fillStyle = fill;
        ctx.fillRect(x * px, py, px, px);

        // Contour line on 100cm (1m) boundaries when showContours is active.
        if (showContours) {
          const elev = st.elevCm[idx] ?? 0;
          const band = Math.floor(elev / 100);
          const eastBand = x + 1 < st.width ? Math.floor((st.elevCm[idx + 1] ?? elev) / 100) : band;
          const northBand =
            y + 1 < st.height ? Math.floor((st.elevCm[idx + st.width] ?? elev) / 100) : band;
          if (band !== eastBand || band !== northBand) {
            ctx.fillStyle = "rgba(0, 0, 0, 0.22)";
            ctx.fillRect(x * px, py, px, 1);
          }
        }
      }
    }

    // Grid lines
    if (showGridLines && px >= 5) {
      ctx.strokeStyle = "rgba(255, 255, 255, 0.07)";
      ctx.lineWidth = 1;
      for (let x = 0; x <= st.width; x += 1) {
        if (x % 8 === 0) continue;
        ctx.beginPath();
        ctx.moveTo(x * px, 0);
        ctx.lineTo(x * px, c.height);
        ctx.stroke();
      }
      for (let y = 0; y <= st.height; y += 1) {
        if (y % 8 === 0) continue;
        ctx.beginPath();
        ctx.moveTo(0, y * px);
        ctx.lineTo(c.width, y * px);
        ctx.stroke();
      }
      // 8-cell sector lines
      ctx.strokeStyle = "rgba(255, 255, 255, 0.18)";
      for (let x = 0; x <= st.width; x += 8) {
        ctx.beginPath();
        ctx.moveTo(x * px, 0);
        ctx.lineTo(x * px, c.height);
        ctx.stroke();
      }
      for (let y = 0; y <= st.height; y += 8) {
        ctx.beginPath();
        ctx.moveTo(0, y * px);
        ctx.lineTo(c.width, y * px);
        ctx.stroke();
      }
    }

    // Muster boxes
    const drawMuster = (rect: [number, number, number, number], colour: string, tag: string) => {
      const [x0, y0, x1, y1] = rect;
      const left = Math.min(x0, x1) * px;
      const top = (st.height - 1 - Math.max(y0, y1)) * px;
      const w = (Math.abs(x1 - x0) + 1) * px;
      const h = (Math.abs(y1 - y0) + 1) * px;
      ctx.strokeStyle = colour;
      ctx.lineWidth = 2;
      ctx.strokeRect(left + 1, top + 1, Math.max(2, w - 2), Math.max(2, h - 2));
      ctx.fillStyle = colour;
      ctx.font = "bold 10px ui-monospace, monospace";
      ctx.fillText(tag, left + 4, top + 11);
    };
    drawMuster(st.redMuster.rect, "#e05a5a", "RED");
    drawMuster(st.blueMuster.rect, "#4f7fd1", "BLUE");

    // Named spots
    for (const spot of st.spots) {
      const cx = spot.cell[0] * px + px / 2;
      const cy = (st.height - 1 - spot.cell[1]) * px + px / 2;
      const side = Math.max(5, px * 0.9);
      ctx.strokeStyle = "#d8c8a0";
      ctx.lineWidth = 1.5;
      ctx.strokeRect(cx - side / 2, cy - side / 2, side, side);
      ctx.fillStyle = "#d8c8a0";
      ctx.font = "10px ui-monospace, monospace";
      ctx.fillText(`${spot.letter}:${spot.name}`, cx + side * 0.7, cy + 3);
    }

    // 3D Preview Camera Pin (`@`) + FOV wedge
    {
      const cx = st.camera.x * px + px / 2;
      const cy = (st.height - 1 - st.camera.y) * px + px / 2;
      const rad = (st.camera.headingDeg * Math.PI) / 180;
      const coneLen = Math.max(18, px * 3.5);
      const halfFov = 0.52;
      ctx.strokeStyle = "rgba(242, 201, 76, 0.75)";
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      ctx.moveTo(cx, cy);
      ctx.lineTo(cx + Math.sin(rad - halfFov) * coneLen, cy - Math.cos(rad - halfFov) * coneLen);
      ctx.moveTo(cx, cy);
      ctx.lineTo(cx + Math.sin(rad + halfFov) * coneLen, cy - Math.cos(rad + halfFov) * coneLen);
      ctx.stroke();

      ctx.fillStyle = "#f2c94c";
      ctx.beginPath();
      ctx.arc(cx, cy, Math.max(3.5, px * 0.55), 0, Math.PI * 2);
      ctx.fill();
    }

    // Live Brush / Drag Gizmo Overlay
    if (hoverCell) {
      const [hx, hy] = hoverCell;
      ctx.strokeStyle = "#56ccf2";
      ctx.lineWidth = 1.5;
      if (isDragging && dragStart && (tool === "line" || tool === "ramp")) {
        for (const [lx, ly] of bresenhamCells(dragStart[0], dragStart[1], hx, hy)) {
          const sy = st.height - 1 - ly;
          ctx.strokeRect(lx * px, sy * px, px, px);
        }
      } else if (
        isDragging &&
        dragStart &&
        (tool === "rect" || tool === "muster_red" || tool === "muster_blue")
      ) {
        const minX = Math.min(dragStart[0], hx);
        const maxX = Math.max(dragStart[0], hx);
        const minY = Math.min(dragStart[1], hy);
        const maxY = Math.max(dragStart[1], hy);
        const top = (st.height - 1 - maxY) * px;
        ctx.strokeRect(
          minX * px,
          top,
          (maxX - minX + 1) * px,
          (maxY - minY + 1) * px,
        );
      } else {
        const pts = brushFootprint(hx, hy, brushRadius, brushShape, st.width, st.height);
        for (const p of pts) {
          const sy = st.height - 1 - p.y;
          ctx.strokeRect(p.x * px + 0.5, sy * px + 0.5, Math.max(1, px - 1), Math.max(1, px - 1));
        }
      }
    }
  });

  // Render the live 3D / 2.5D / Overhead preview via `mush-pixel-wasm`.
  $effect(() => {
    const r = renderer;
    const c = previewCanvas;
    const st = mapState;
    const mode = previewView;
    if (!r || !c) return;
    const feeds = editorToPictureFeeds(st);
    r.forget();
    if (!r.draw(feeds.cockpit, feeds.map, 0, mode)) return;
    const ctx = c.getContext("2d");
    if (!ctx) return;
    ctx.putImageData(r.image(), 0, 0);
  });

  function openExportDrawer(): void {
    jsonBuffer = JSON.stringify(exportArenaDefJson(mapState), null, 2);
    showExportDrawer = !showExportDrawer;
  }

  function importFromBuffer(): void {
    try {
      const parsed = JSON.parse(jsonBuffer) as unknown;
      const loaded = importArenaDefJson(parsed, mapState.id);
      if (!loaded) {
        statusMsg = "Invalid ArenaDef JSON or tile base64 payload.";
        return;
      }
      pushUndo();
      mapState = loaded;
      statusMsg = `Imported ${loaded.label} (${loaded.width}×${loaded.height}, biome ${loaded.biome}).`;
    } catch {
      statusMsg = "Could not parse JSON.";
    }
  }

  function prefillAtlasWindow(): void {
    app.focusComposer(`@atlas/window open 0,0,${mapState.width * 10},${mapState.height * 10}=${mapState.cellCm}`);
  }
</script>

<div class="studio" aria-label="terrain and arena studio">
  <!-- Top Control Bar -->
  <div class="studio-bar">
    <label class="field">
      <span>Preset</span>
      <select bind:value={preset} aria-label="map preset">
        <option value="proving_ground">Proving Ground (Temperate)</option>
        <option value="mars_canyon">Ares Canyon (Mars)</option>
        <option value="lunar_crater">Tycho Crater (Lunar)</option>
        <option value="ice_glacier">Europa Shelf (Ice)</option>
        <option value="urban_compound">Forward Compound (Urban)</option>
        <option value="blank">Blank Grid</option>
      </select>
    </label>
    <label class="field">
      <span>Size</span>
      <select bind:value={presetSize} aria-label="grid size">
        <option value={32}>32×32</option>
        <option value={64}>64×64</option>
        <option value={96}>96×96</option>
      </select>
    </label>
    <label class="field">
      <span>Seed</span>
      <input type="number" class="num" bind:value={seed} aria-label="generator seed" />
    </label>
    <button type="button" class="btn primary" onclick={applyPreset}>Generate Base</button>

    <span class="sep"></span>

    <label class="field">
      <span>Biome</span>
      <select
        value={mapState.biome}
        aria-label="biome palette"
        onchange={(ev) => setBiome((ev.currentTarget as HTMLSelectElement).value as BiomeId)}
      >
        <option value="temperate">Temperate</option>
        <option value="mars">Mars</option>
        <option value="lunar">Lunar</option>
        <option value="ice">Ice</option>
      </select>
    </label>

    <label class="chk">
      <input type="checkbox" bind:checked={showGridLines} />
      Grid
    </label>
    <label class="chk">
      <input type="checkbox" bind:checked={showContours} />
      Relief Shading
    </label>

    <span class="grow"></span>

    <button type="button" class="btn" disabled={undoStack.length === 0} onclick={undo}>
      Undo ({undoStack.length})
    </button>
    <button type="button" class="btn" disabled={redoStack.length === 0} onclick={redo}>
      Redo ({redoStack.length})
    </button>
    <button type="button" class="btn" onclick={openExportDrawer}>
      {showExportDrawer ? "Hide JSON" : "Import / Export JSON"}
    </button>
    <button type="button" class="btn" onclick={prefillAtlasWindow}>
      @atlas/window
    </button>
  </div>

  {#if statusMsg}
    <p class="status-banner">{statusMsg}</p>
  {/if}

  <div class="studio-main">
    <!-- Left Dock: Unity-style Toolbrushes & Terrain Palette -->
    <aside class="dock-left" aria-label="brush tools">
      <section class="panel-box">
        <h4>Toolbrushes</h4>
        <div class="tool-grid">
          <button type="button" class:active={tool === "paint"} onclick={() => (tool = "paint")}>
            🖌 Paint
          </button>
          <button type="button" class:active={tool === "raise"} onclick={() => (tool = "raise")}>
            ▲ Raise Elev
          </button>
          <button type="button" class:active={tool === "lower"} onclick={() => (tool = "lower")}>
            ▼ Lower Elev
          </button>
          <button type="button" class:active={tool === "flatten"} onclick={() => (tool = "flatten")}>
            ▬ Flatten Mesa
          </button>
          <button type="button" class:active={tool === "smooth"} onclick={() => (tool = "smooth")}>
            ≈ Smooth
          </button>
          <button type="button" class:active={tool === "ramp"} onclick={() => (tool = "ramp")}>
            ⟋ Carve Ramp
          </button>
          <button type="button" class:active={tool === "line"} onclick={() => (tool = "line")}>
            ╱ Line / Road
          </button>
          <button type="button" class:active={tool === "rect"} onclick={() => (tool = "rect")}>
            ▭ Rectangle
          </button>
          <button type="button" class:active={tool === "fill"} onclick={() => (tool = "fill")}>
            ▣ Flood Fill
          </button>
          <button type="button" class:active={tool === "eyedrop"} onclick={() => (tool = "eyedrop")}>
            ⌖ Eyedropper
          </button>
          <button type="button" class:active={tool === "spot"} onclick={() => (tool = "spot")}>
            ⚑ Place Spot
          </button>
          <button
            type="button"
            class:active={tool === "muster_red"}
            onclick={() => (tool = "muster_red")}
          >
            R Red Muster
          </button>
          <button
            type="button"
            class:active={tool === "muster_blue"}
            onclick={() => (tool = "muster_blue")}
          >
            B Blue Muster
          </button>
          <button type="button" class:active={tool === "camera"} onclick={() => (tool = "camera")}>
            @ 3D Camera
          </button>
        </div>
      </section>

      <section class="panel-box">
        <h4>Brush Settings</h4>
        <div class="shape-row">
          <span>Shape:</span>
          <button
            type="button"
            class:active={brushShape === "circle"}
            onclick={() => (brushShape = "circle")}>● Circle</button
          >
          <button
            type="button"
            class:active={brushShape === "square"}
            onclick={() => (brushShape = "square")}>■ Square</button
          >
          <button
            type="button"
            class:active={brushShape === "diamond"}
            onclick={() => (brushShape = "diamond")}>◆ Diamond</button
          >
        </div>
        <label class="slider-row">
          <span>Brush Radius: <strong>{brushRadius}</strong> ({brushRadius * 2 - 1}×{brushRadius * 2 - 1})</span>
          <input type="range" min="1" max="8" bind:value={brushRadius} aria-label="brush radius" />
        </label>
        {#if tool === "paint"}
          <label class="slider-row">
            <span>Scatter Density: <strong>{densityPct}%</strong></span>
            <input type="range" min="10" max="100" step="5" bind:value={densityPct} />
          </label>
        {/if}
        {#if tool === "raise" || tool === "lower"}
          <label class="slider-row">
            <span>Sculpt Step: <strong>{(elevStepCm / 100).toFixed(2)} m</strong></span>
            <input type="range" min="10" max="100" step="5" bind:value={elevStepCm} />
          </label>
        {/if}
        {#if tool === "flatten"}
          <label class="slider-row">
            <span>Mesa Target: <strong>{(targetElevCm / 100).toFixed(2)} m</strong></span>
            <input type="range" min="-300" max="800" step="20" bind:value={targetElevCm} />
          </label>
        {/if}
        {#if tool === "rect"}
          <label class="chk">
            <input type="checkbox" bind:checked={hollowRect} />
            Hollow perimeter walls only
          </label>
        {/if}
        {#if tool === "ramp"}
          <label class="chk">
            <input type="checkbox" bind:checked={rampPavesRoad} />
            Pave ramp with Road (`=`)
          </label>
        {/if}
        {#if tool === "spot"}
          <div class="spot-inputs">
            <input
              type="text"
              maxlength="1"
              class="spot-letter"
              bind:value={spotLetter}
              aria-label="spot letter"
            />
            <input
              type="text"
              maxlength="32"
              class="spot-name"
              bind:value={spotName}
              placeholder="Spot name"
              aria-label="spot name"
            />
          </div>
        {/if}
      </section>

      <section class="panel-box">
        <h4>Terrain Palette</h4>
        <div class="swatch-grid">
          {#each EDITOR_TERRAINS.filter((t) => t.code !== 0) as t (t.code)}
            {@const col = biomeTerrainColours(mapState.biome)[t.glyph] ?? "#333"}
            <button
              type="button"
              class="swatch-btn"
              class:active={terrainCode === t.code}
              onclick={() => {
                terrainCode = t.code;
                if (
                  tool !== "paint" &&
                  tool !== "line" &&
                  tool !== "rect" &&
                  tool !== "fill"
                ) {
                  tool = "paint";
                }
              }}
            >
              <span class="swatch-chip" style="background: {col}">{t.glyph}</span>
              <span class="swatch-label">{t.label}</span>
            </button>
          {/each}
        </div>
      </section>
    </aside>

    <!-- Center Viewport: 2D Editing Grid -->
    <section class="grid-pane" aria-label="editing grid">
      <header class="pane-head">
        <strong>{mapState.label}</strong>
        <span class="muted">
          {mapState.width}×{mapState.height} cells ({(mapState.width * mapState.cellCm) / 100}m ×
          {(mapState.height * mapState.cellCm) / 100}m) · {mapState.biome}
        </span>
      </header>
      <div class="grid-scroll">
        <canvas
          bind:this={gridCanvas}
          aria-label="map editor canvas"
          style="width: {mapState.width * cellPx}px; height: {mapState.height * cellPx}px"
          onmousedown={onCanvasPointerDown}
          onmousemove={onCanvasPointerMove}
          onmouseup={onCanvasPointerUp}
          onmouseleave={() => {
            hoverCell = null;
            isDragging = false;
          }}
        ></canvas>
      </div>
      <footer class="cell-readout">
        {#if hoveredInfo}
          <span>Cell <strong>({hoveredInfo.x}, {hoveredInfo.y})</strong></span>
          <span>Terrain: <code>{hoveredInfo.terrain.glyph}</code> {hoveredInfo.terrain.label}</span>
          <span>Elev: <strong>{hoveredInfo.elevM} m</strong></span>
          <span>Grade: <strong>{hoveredInfo.gradePct}%</strong></span>
        {:else}
          <span class="muted">
            Hover or drag on the grid to paint/sculpt · Shift+Click moves the @ 3D camera · Alt+Click samples cell
          </span>
        {/if}
      </footer>
    </section>

    <!-- Right Dock: Live 3D / 2.5D / Overhead Preview -->
    <aside class="dock-right" aria-label="live 3d preview">
      <section class="panel-box">
        <header class="preview-head">
          <h4>Live 3D / 2.5D Preview</h4>
          <button type="button" class="btn small" onclick={cyclePreviewView}>
            View: {previewView === "cockpit"
              ? "1st-Person 3D"
              : previewView === "iso25d"
                ? "2.5D Tactical"
                : "Overhead"}
          </button>
        </header>
        <canvas
          bind:this={previewCanvas}
          width={FRAME_W}
          height={FRAME_H}
          class="preview-canvas"
          aria-label="live 3d preview canvas"
        ></canvas>
        <div class="cam-controls">
          <span class="muted">
            Cam @ ({mapState.camera.x}, {mapState.camera.y}) · {mapState.camera.headingDeg}°
          </span>
          <span class="grow"></span>
          <button type="button" class="btn small" onclick={() => rotateCamera(-22)}>↺ -22°</button>
          <button type="button" class="btn small" onclick={() => rotateCamera(22)}>↻ +22°</button>
          <button
            type="button"
            class="btn small"
            onclick={() => (mapState = { ...mapState, camera: { ...mapState.camera, headingDeg: 0 } })}
            >N</button
          >
          <button
            type="button"
            class="btn small"
            onclick={() => (mapState = { ...mapState, camera: { ...mapState.camera, headingDeg: 90 } })}
            >E</button
          >
          <button
            type="button"
            class="btn small"
            onclick={() =>
              (mapState = { ...mapState, camera: { ...mapState.camera, headingDeg: 180 } })}
            >S</button
          >
          <button
            type="button"
            class="btn small"
            onclick={() =>
              (mapState = { ...mapState, camera: { ...mapState.camera, headingDeg: 270 } })}
            >W</button
          >
        </div>
      </section>

      {#if showExportDrawer}
        <section class="panel-box">
          <h4>ArenaDef JSON &amp; Base64 Tile (`mush_atlas::Cells`)</h4>
          <div class="meta-inputs">
            <input
              type="text"
              bind:value={mapState.label}
              placeholder="Arena label"
              aria-label="arena label"
            />
            <input
              type="text"
              bind:value={mapState.id}
              placeholder="arena_id"
              aria-label="arena id"
            />
          </div>
          <textarea
            rows="8"
            class="json-box"
            bind:value={jsonBuffer}
            aria-label="arena json buffer"
          ></textarea>
          <div class="cam-controls">
            <button
              type="button"
              class="btn"
              onclick={() => {
                jsonBuffer = JSON.stringify(exportArenaDefJson(mapState), null, 2);
                statusMsg = "Refreshed ArenaDef JSON from current grid.";
              }}>Refresh JSON</button
            >
            <button
              type="button"
              class="btn"
              onclick={() => {
                jsonBuffer = encodeTileCellsBase64(mapState);
                statusMsg = "Encoded raw mush_atlas::Cells base64 tile string.";
              }}>Tile Base64 Only</button
            >
            <button type="button" class="btn primary" onclick={importFromBuffer}>
              Load JSON into Editor
            </button>
          </div>
        </section>
      {/if}
    </aside>
  </div>
</div>

<style>
  .studio {
    display: flex;
    flex-direction: column;
    gap: 0.45rem;
    font-size: 0.8rem;
  }

  .studio-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.45rem;
    padding: 0.4rem 0.6rem;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-panel);
  }

  .field {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    color: var(--fg-muted);
  }

  .field select,
  .field input,
  .spot-inputs input,
  .meta-inputs input {
    background: var(--bg-input);
    color: var(--fg);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 0.2rem 0.4rem;
    font: inherit;
  }

  .num {
    width: 4rem;
  }

  .chk {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    color: var(--fg-muted);
    cursor: pointer;
  }

  .sep {
    width: 1px;
    height: 1.2rem;
    background: var(--border);
  }

  .grow {
    flex: 1;
  }

  .status-banner {
    margin: 0;
    padding: 0.25rem 0.6rem;
    background: var(--bg-panel);
    border-left: 3px solid var(--accent);
    color: var(--fg);
    font-family: var(--font-mono);
    font-size: 0.76rem;
  }

  .studio-main {
    display: grid;
    grid-template-columns: 16.5rem minmax(20rem, 1fr) 23rem;
    gap: 0.5rem;
    align-items: start;
  }

  @media (max-width: 1180px) {
    .studio-main {
      grid-template-columns: 1fr;
    }
  }

  .dock-left,
  .dock-right {
    display: flex;
    flex-direction: column;
    gap: 0.45rem;
  }

  .panel-box {
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-panel);
    padding: 0.45rem 0.55rem;
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }

  .panel-box h4 {
    margin: 0;
    font-size: 0.72rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--fg-muted);
  }

  .tool-grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 0.25rem;
  }

  .tool-grid button,
  .shape-row button,
  .btn {
    background: var(--bg-input);
    color: var(--fg);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 0.24rem 0.45rem;
    font: inherit;
    font-size: 0.75rem;
    cursor: pointer;
    text-align: left;
  }

  .tool-grid button.active,
  .shape-row button.active,
  .swatch-btn.active {
    border-color: var(--accent);
    background: var(--accent-dim);
    font-weight: 600;
  }

  .btn.primary {
    background: var(--accent-dim);
    border-color: var(--accent);
  }

  .btn.small {
    padding: 0.15rem 0.4rem;
    font-size: 0.72rem;
  }

  .btn:disabled {
    opacity: 0.45;
    cursor: default;
  }

  .shape-row {
    display: flex;
    align-items: center;
    gap: 0.25rem;
    flex-wrap: wrap;
  }

  .slider-row {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    font-size: 0.75rem;
    color: var(--fg-muted);
  }

  .swatch-grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 0.22rem;
    max-height: 15rem;
    overflow-y: auto;
  }

  .swatch-btn {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    background: var(--bg-input);
    color: var(--fg);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 0.18rem 0.35rem;
    font: inherit;
    font-size: 0.73rem;
    cursor: pointer;
    text-align: left;
  }

  .swatch-chip {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.25rem;
    height: 1.25rem;
    border-radius: 3px;
    border: 1px solid rgba(255, 255, 255, 0.2);
    font-family: var(--font-mono);
    font-weight: 700;
    color: #fff;
    text-shadow: 0 1px 2px #000;
    flex-shrink: 0;
  }

  .swatch-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .grid-pane {
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-panel);
    padding: 0.45rem 0.6rem;
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    min-width: 0;
  }

  .pane-head,
  .preview-head,
  .cam-controls {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.35rem;
    flex-wrap: wrap;
  }

  .grid-scroll {
    overflow: auto;
    max-height: 62vh;
    display: flex;
    justify-content: center;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 0.25rem;
  }

  canvas {
    image-rendering: pixelated;
    display: block;
    cursor: crosshair;
  }

  .preview-canvas {
    width: 100%;
    height: auto;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: #0d121a;
    cursor: default;
  }

  .cell-readout {
    display: flex;
    flex-wrap: wrap;
    gap: 0.75rem;
    font-family: var(--font-mono);
    font-size: 0.75rem;
  }

  .spot-inputs,
  .meta-inputs {
    display: flex;
    gap: 0.3rem;
  }

  .spot-letter {
    width: 2.2rem;
    text-align: center;
    font-family: var(--font-mono);
  }

  .spot-name {
    flex: 1;
    min-width: 0;
  }

  .json-box {
    width: 100%;
    font-family: var(--font-mono);
    font-size: 0.72rem;
    background: var(--bg-input);
    color: var(--fg);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 0.35rem;
    resize: vertical;
  }

  .muted {
    color: var(--fg-muted);
  }
</style>
