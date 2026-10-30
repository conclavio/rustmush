<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import type { GameApp } from '../lib/game.svelte';
  import { pixelModule } from '../lib/pixel/local';
  import {
    FEED_COCKPIT,
    FEED_MAP,
    KIND_COCKPIT,
    KIND_ISO25D,
    KIND_OVERHEAD,
    type PixelModule
  } from '../lib/pixel/wasm';
  import {
    BIOME_OPTIONS,
    TERRAIN_PALETTE,
    DEFAULT_SLIDER_MIN_ELEV_CM,
    DEFAULT_SLIDER_MAX_ELEV_CM,
    DEFAULT_HULL_SIZE_CM,
    createEditorMap,
    cloneEditorMap,
    EditorUndoStack,
    paintTerrainBrush,
    paintTerrainLine,
    floodFillTerrain,
    applySculptBrush,
    exportGroundDraftJson,
    importGroundDraftJson,
    editorToPictureFeeds,
    computePassabilityOverlay,
    computeViewshed,
    checkEditorMap,
    routeEditorMap,
    relaxEditorSlopes,
    despeckleEditorMap,
    markStudioEdited,
    type BiomeName,
    type BrushShape,
    type DriveName,
    type EditorCameraPin,
    type EditorMapState,
    type EditorTool,
    type GroundCheckReport,
    type GroundDraftJson,
    type SculptMode,
    type StudioOverlayMode,
    type TerrainClassCode
  } from '../lib/map-editor';

  interface Props {
    app?: GameApp;
    game?: GameApp;
  }

  let { app, game }: Props = $props();
  const controller = $derived(app ?? game);

  let gridSize = $state<64 | 128 | 256>(64);
  let editorState = $state<EditorMapState>(createEditorMap(64, 64, 'temperate', 3));
  let undoStack = new EditorUndoStack(40);
  let canUndo = $state(false);
  let canRedo = $state(false);

  // Active Tool & Brush State
  let activeTool = $state<EditorTool>('paint');
  let selectedClass = $state<TerrainClassCode>(2);
  let sculptMode = $state<SculptMode>('raise');
  let brushShape = $state<BrushShape>('circle');
  let brushRadius = $state(3);
  let brushStrengthCm = $state(120);
  let brushTargetElevCm = $state(0);
  let brushFalloff = $state(0.5);
  let relaxToPassable = $state(false);

  // Overlays & Viewshed
  let showContours = $state(true);
  let showGridLines = $state(true);
  let overlayMode = $state<StudioOverlayMode>('none');
  let overlayDrive = $state<DriveName>('tracked');
  let viewshedSizeCm = $state(DEFAULT_HULL_SIZE_CM);
  let highlightedCells = $state<number[]>([]);
  let lastRepairNotice = $state<string | null>(null);

  // Camera Pin for Live 3D / 2.5D / Overhead Preview
  let previewPin = $state<EditorCameraPin>({ x: 16, y: 32, headingDeg: 90 });
  let previewSubMode = $state<'3d' | '25d' | 'overhead' | 'split'>('split');

  // Canvas Interaction State
  let gridCanvas = $state<HTMLCanvasElement | null>(null);
  let previewFpCanvas = $state<HTMLCanvasElement | null>(null);
  let preview25dCanvas = $state<HTMLCanvasElement | null>(null);
  let previewOverheadCanvas = $state<HTMLCanvasElement | null>(null);

  let isDragging = $state(false);
  let dragStartCell = $state<[number, number] | null>(null);
  let hoverCell = $state<[number, number] | null>(null);
  let selectedWaypointIdx = $state<number | null>(null);
  let draggingWaypointIdx = $state<number | null>(null);
  let statusNotice = $state<string | null>(null);

  // WASM Engine for Live Preview
  let engine = $state<PixelModule | null>(null);
  let renderTick = $state(0);
  let matchRev = 1;
  let lastLoadedDraftFrame: Record<string, unknown> | null = null;

  let checkReport = $derived.by<GroundCheckReport>(() => {
    void renderTick;
    return checkEditorMap(editorState);
  });

  function syncUndoFlags() {
    canUndo = undoStack.canUndo;
    canRedo = undoStack.canRedo;
  }

  function pushSnapshot() {
    undoStack.push(editorState);
    syncUndoFlags();
  }

  function handleUndo() {
    const prev = undoStack.undo(editorState);
    if (prev) {
      editorState = prev;
      highlightedCells = [];
      syncUndoFlags();
      refreshPreview();
    }
  }

  function handleRedo() {
    const next = undoStack.redo(editorState);
    if (next) {
      editorState = next;
      highlightedCells = [];
      syncUndoFlags();
      refreshPreview();
    }
  }

  function handleNewMap(size: 64 | 128 | 256, biome: BiomeName) {
    pushSnapshot();
    gridSize = size;
    const defaultClass: TerrainClassCode =
      biome === 'mars' ? 8 : biome === 'lunar' ? 17 : biome === 'ice' ? 16 : 3;
    editorState = createEditorMap(size, size, biome, defaultClass);
    highlightedCells = [];
    previewPin = {
      x: Math.floor(size * 0.2),
      y: Math.floor(size * 0.5),
      headingDeg: 90
    };
    refreshPreview();
  }

  function handleBiomeChange(biome: BiomeName) {
    pushSnapshot();
    editorState.biome = biome;
    markStudioEdited(editorState);
    refreshPreview();
  }

  function handleLoadServerDraft() {
    statusNotice = `Requesting draft "${editorState.id || 'proving_ground'}" from server...`;
    controller?.requestGroundDraft?.(editorState.id || 'proving_ground');
  }

  $effect(() => {
    const frame = controller?.groundDraftFrame ?? null;
    if (!frame || frame === lastLoadedDraftFrame) return;
    lastLoadedDraftFrame = frame;
    untrack(() => {
      if (typeof frame.error === 'string') {
        statusNotice = `Server draft error: ${frame.error}`;
        return;
      }
      try {
        const imported = importGroundDraftJson(frame as unknown as GroundDraftJson);
        pushSnapshot();
        editorState = imported;
        if (imported.width === 64 || imported.width === 128 || imported.width === 256) {
          gridSize = imported.width;
        }
        highlightedCells = [];
        const firstSpot = imported.keySpots[0];
        previewPin = firstSpot
          ? { x: firstSpot.x, y: firstSpot.y, headingDeg: 90 }
          : { x: Math.floor(imported.width / 2), y: Math.floor(imported.height / 2), headingDeg: 90 };
        statusNotice = `Loaded server draft "${imported.id}" (${imported.width}x${imported.height})`;
        refreshPreview();
      } catch (err) {
        statusNotice = `Failed to load server draft: ${(err as Error).message}`;
      }
    });
  });

  function handleRelaxSlopes() {
    pushSnapshot();
    const changed = relaxEditorSlopes(editorState, 45);
    highlightedCells = changed;
    lastRepairNotice = `Relaxed ${changed.length} steep cell${changed.length === 1 ? '' : 's'}`;
    refreshPreview();
  }

  function handleDespeckle() {
    pushSnapshot();
    const changed = despeckleEditorMap(editorState, 3);
    highlightedCells = changed;
    lastRepairNotice = `Despeckled ${changed.length} cell${changed.length === 1 ? '' : 's'}`;
    refreshPreview();
  }

  function handleAutoRoute() {
    pushSnapshot();
    const graph = routeEditorMap(editorState);
    lastRepairNotice = `Routed ${graph.nodes.length} nodes, ${graph.edges.length} edges`;
    refreshPreview();
  }

  function cellFromPointerEvent(e: MouseEvent): [number, number] | null {
    if (!gridCanvas) return null;
    const rect = gridCanvas.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) return null;
    const u = (e.clientX - rect.left) / rect.width;
    const v = 1 - (e.clientY - rect.top) / rect.height; // South-up: y=0 at bottom
    const x = Math.max(0, Math.min(editorState.width - 1, Math.floor(u * editorState.width)));
    const y = Math.max(0, Math.min(editorState.height - 1, Math.floor(v * editorState.height)));
    return [x, y];
  }

  function applyToolAt(x: number, y: number, isStart: boolean, shiftKey = false) {
    if (activeTool === 'paint') {
      if (isStart) pushSnapshot();
      if (paintTerrainBrush(editorState, x, y, brushRadius, brushShape, selectedClass)) {
        highlightedCells = [];
        refreshPreview();
      }
    } else if (activeTool === 'sculpt') {
      if (isStart) pushSnapshot();
      if (
        applySculptBrush(editorState, x, y, {
          mode: sculptMode,
          shape: brushShape,
          radius: brushRadius,
          strengthCm: brushStrengthCm,
          targetElevCm: brushTargetElevCm,
          falloff: brushFalloff,
          relaxToPassable
        })
      ) {
        highlightedCells = [];
        refreshPreview();
      }
    } else if (activeTool === 'fill' && isStart) {
      pushSnapshot();
      if (floodFillTerrain(editorState, x, y, selectedClass) > 0) {
        highlightedCells = [];
        refreshPreview();
      }
    } else if (activeTool === 'camera_pin') {
      if (isStart) {
        previewPin = { ...previewPin, x, y };
      } else {
        const dx = x - previewPin.x;
        const dy = y - previewPin.y;
        if (dx !== 0 || dy !== 0) {
          const deg = Math.round(((Math.atan2(dx, dy) * 180) / Math.PI + 360) % 360);
          previewPin = { ...previewPin, headingDeg: deg };
        }
      }
      refreshPreview();
    } else if (activeTool === 'spot' && isStart) {
      pushSnapshot();
      const existingIdx = editorState.keySpots.findIndex(
        (s) => Math.abs(s.x - x) <= 1 && Math.abs(s.y - y) <= 1
      );
      if (existingIdx >= 0) {
        editorState.keySpots.splice(existingIdx, 1);
        markStudioEdited(editorState);
      } else if (editorState.keySpots.length < 12) {
        const letter = String.fromCharCode(65 + editorState.keySpots.length);
        editorState.keySpots.push({ name: `Point ${letter}`, x, y });
        markStudioEdited(editorState);
      }
      refreshPreview();
    } else if (activeTool === 'waypoint') {
      if (isStart) {
        const hitIdx = editorState.waypoints.nodes.findIndex(
          (n) => Math.abs(n.x - x) <= 2 && Math.abs(n.y - y) <= 2
        );
        if (hitIdx >= 0) {
          if (shiftKey && selectedWaypointIdx !== null && selectedWaypointIdx !== hitIdx) {
            const nodeA = editorState.waypoints.nodes[selectedWaypointIdx];
            const nodeB = editorState.waypoints.nodes[hitIdx];
            if (nodeA && nodeB) {
              pushSnapshot();
              const a = nodeA.id;
              const b = nodeB.id;
              const existingEdgeIdx = editorState.waypoints.edges.findIndex(
                ([e0, e1]) => (e0 === a && e1 === b) || (e0 === b && e1 === a)
              );
              if (existingEdgeIdx >= 0) {
                editorState.waypoints.edges.splice(existingEdgeIdx, 1);
              } else if (editorState.waypoints.edges.length < 1024) {
                editorState.waypoints.edges.push([a, b]);
              }
              markStudioEdited(editorState);
              selectedWaypointIdx = hitIdx;
              refreshPreview();
            }
          } else {
            pushSnapshot();
            selectedWaypointIdx = hitIdx;
            draggingWaypointIdx = hitIdx;
          }
        } else if (editorState.waypoints.nodes.length < 256) {
          pushSnapshot();
          let seq = editorState.waypoints.nodes.length;
          let id = `wp_${String(seq).padStart(3, '0')}`;
          while (editorState.waypoints.nodes.some((n) => n.id === id)) {
            seq++;
            id = `wp_${String(seq).padStart(3, '0')}`;
          }
          editorState.waypoints.nodes.push({ id, x, y });
          const newIdx = editorState.waypoints.nodes.length - 1;
          if (shiftKey && selectedWaypointIdx !== null) {
            const prevNode = editorState.waypoints.nodes[selectedWaypointIdx];
            if (prevNode) {
              editorState.waypoints.edges.push([prevNode.id, id]);
            }
          }
          markStudioEdited(editorState);
          selectedWaypointIdx = newIdx;
          draggingWaypointIdx = newIdx;
          refreshPreview();
        }
      } else if (draggingWaypointIdx !== null) {
        const node = editorState.waypoints.nodes[draggingWaypointIdx];
        if (node && (node.x !== x || node.y !== y)) {
          node.x = x;
          node.y = y;
          markStudioEdited(editorState);
          refreshPreview();
        }
      }
    }
  }

  function onCanvasPointerDown(e: MouseEvent) {
    if (e.button !== 0) return;
    const cell = cellFromPointerEvent(e);
    if (!cell) return;
    isDragging = true;
    dragStartCell = cell;
    hoverCell = cell;
    if (activeTool === 'line' || activeTool === 'zone_red' || activeTool === 'zone_blue') {
      pushSnapshot();
    } else {
      applyToolAt(cell[0], cell[1], true, e.shiftKey);
    }
  }

  function onCanvasPointerMove(e: MouseEvent) {
    const cell = cellFromPointerEvent(e);
    hoverCell = cell;
    if (!isDragging || !cell) return;
    if (activeTool === 'paint' || activeTool === 'sculpt' || activeTool === 'camera_pin' || activeTool === 'waypoint') {
      applyToolAt(cell[0], cell[1], false, e.shiftKey);
    }
  }

  function onCanvasPointerUp(e: MouseEvent) {
    if (!isDragging) return;
    const cell = cellFromPointerEvent(e) ?? hoverCell;
    if (cell && dragStartCell) {
      if (activeTool === 'line') {
        paintTerrainLine(
          editorState,
          dragStartCell[0],
          dragStartCell[1],
          cell[0],
          cell[1],
          brushRadius,
          brushShape,
          selectedClass
        );
        refreshPreview();
      } else if (activeTool === 'zone_red' || activeTool === 'zone_blue') {
        const team = activeTool === 'zone_red' ? 'red' : 'blue';
        const x0 = Math.min(dragStartCell[0], cell[0]);
        const x1 = Math.max(dragStartCell[0], cell[0]);
        const y0 = Math.min(dragStartCell[1], cell[1]);
        const y1 = Math.max(dragStartCell[1], cell[1]);
        const idx = editorState.entryZones.findIndex((z) => z.team === team);
        const zone = { team: team as 'red' | 'blue', x0, y0, x1, y1 };
        if (idx >= 0) {
          editorState.entryZones[idx] = zone;
        } else {
          editorState.entryZones.push(zone);
        }
        markStudioEdited(editorState);
        refreshPreview();
      }
    }
    isDragging = false;
    dragStartCell = null;
    draggingWaypointIdx = null;
  }

  function refreshPreview() {
    renderTick += 1;
    if (!engine) return;
    const feeds = editorToPictureFeeds(editorState, previewPin);
    (feeds.arenaMap as Record<string, unknown>).match = -matchRev;
    matchRev += 1;
    engine.feed(FEED_MAP, JSON.stringify(feeds.arenaMap));
    engine.feed(FEED_COCKPIT, JSON.stringify(feeds.arenaCockpit));
  }

  function draw2DGridCanvas() {
    if (!gridCanvas) return;
    const ctx = gridCanvas.getContext('2d');
    if (!ctx) return;

    const w = editorState.width;
    const h = editorState.height;
    const cw = gridCanvas.width;
    const ch = gridCanvas.height;
    const cellW = cw / w;
    const cellH = ch / h;

    ctx.fillStyle = '#0d1016';
    ctx.fillRect(0, 0, cw, ch);

    const swatchByCode = new Map<number, string>();
    for (const s of TERRAIN_PALETTE) {
      swatchByCode.set(s.code, s.color);
    }

    const passability =
      overlayMode === 'passability'
        ? computePassabilityOverlay(editorState, overlayDrive)
        : null;
    const viewshed =
      overlayMode === 'viewshed'
        ? computeViewshed(editorState, previewPin.x, previewPin.y, viewshedSizeCm)
        : null;

    // 1. Draw cells with elevation hillshading + active overlay
    for (let y = 0; y < h; y++) {
      const py = ch - (y + 1) * cellH;
      for (let x = 0; x < w; x++) {
        const px = x * cellW;
        const idx = y * w + x;
        const code = editorState.classes[idx] ?? 2;
        const baseHex = swatchByCode.get(code) ?? '#4a6b3b';
        const elev = editorState.elevCm[idx] ?? 0;
        const eastElev = x + 1 < w ? (editorState.elevCm[y * w + (x + 1)] ?? elev) : elev;
        const northElev = y + 1 < h ? (editorState.elevCm[(y + 1) * w + x] ?? elev) : elev;
        const slopeShade = ((eastElev - elev) + (northElev - elev)) / 900;

        ctx.fillStyle = baseHex;
        ctx.fillRect(px, py, Math.ceil(cellW), Math.ceil(cellH));

        if (elev !== 0 || slopeShade !== 0) {
          const lum = Math.max(-0.35, Math.min(0.35, elev / 8000 + slopeShade * 0.25));
          if (lum > 0) {
            ctx.fillStyle = `rgba(255, 248, 220, ${lum.toFixed(2)})`;
            ctx.fillRect(px, py, Math.ceil(cellW), Math.ceil(cellH));
          } else if (lum < 0) {
            ctx.fillStyle = `rgba(8, 12, 20, ${(-lum).toFixed(2)})`;
            ctx.fillRect(px, py, Math.ceil(cellW), Math.ceil(cellH));
          }
        }

        if (passability) {
          const stateCode = passability[idx];
          if (stateCode === 0) {
            ctx.fillStyle = 'rgba(180, 28, 28, 0.62)';
          } else if (stateCode === 2) {
            ctx.fillStyle = 'rgba(245, 158, 11, 0.72)';
          } else {
            ctx.fillStyle = 'rgba(34, 197, 94, 0.22)';
          }
          ctx.fillRect(px, py, Math.ceil(cellW), Math.ceil(cellH));
        } else if (viewshed) {
          const vs = viewshed[idx];
          if (vs === 0) {
            ctx.fillStyle = 'rgba(10, 12, 18, 0.68)';
          } else if (vs === 2) {
            ctx.fillStyle = 'rgba(245, 158, 11, 0.58)';
          } else if (vs === 3) {
            ctx.fillStyle = 'rgba(56, 189, 248, 0.45)';
          } else {
            ctx.fillStyle = 'rgba(74, 222, 128, 0.28)';
          }
          ctx.fillRect(px, py, Math.ceil(cellW), Math.ceil(cellH));
        }
      }
    }

    // 2. Contour lines every 500 cm (5 m)
    if (showContours) {
      ctx.strokeStyle = 'rgba(255, 230, 160, 0.32)';
      ctx.lineWidth = 1;
      ctx.beginPath();
      for (let y = 0; y < h - 1; y++) {
        const py = ch - (y + 1) * cellH;
        for (let x = 0; x < w - 1; x++) {
          const px = x * cellW;
          const b0 = Math.floor((editorState.elevCm[y * w + x] ?? 0) / 500);
          const bE = Math.floor((editorState.elevCm[y * w + (x + 1)] ?? 0) / 500);
          const bN = Math.floor((editorState.elevCm[(y + 1) * w + x] ?? 0) / 500);
          if (b0 !== bE) {
            ctx.moveTo(px + cellW, py);
            ctx.lineTo(px + cellW, py + cellH);
          }
          if (b0 !== bN) {
            ctx.moveTo(px, py);
            ctx.lineTo(px + cellW, py);
          }
        }
      }
      ctx.stroke();
    }

    // 3. Highlighted cells from Relax Slopes / Despeckle
    if (highlightedCells.length > 0) {
      ctx.fillStyle = 'rgba(250, 204, 21, 0.48)';
      for (const idx of highlightedCells) {
        const x = idx % w;
        const y = (idx - x) / w;
        const px = x * cellW;
        const py = ch - (y + 1) * cellH;
        ctx.fillRect(px, py, Math.max(2, Math.ceil(cellW)), Math.max(2, Math.ceil(cellH)));
      }
    }

    // 4. Grid lines
    if (showGridLines && cellW >= 4) {
      ctx.strokeStyle = 'rgba(255, 255, 255, 0.06)';
      ctx.lineWidth = 1;
      ctx.beginPath();
      for (let x = 0; x <= w; x += 8) {
        ctx.moveTo(x * cellW, 0);
        ctx.lineTo(x * cellW, ch);
      }
      for (let y = 0; y <= h; y += 8) {
        ctx.moveTo(0, y * cellH);
        ctx.lineTo(cw, y * cellH);
      }
      ctx.stroke();
    }

    // 5. Entry Zones (Red & Blue)
    for (const z of editorState.entryZones) {
      const x0 = Math.min(z.x0, z.x1);
      const x1 = Math.max(z.x0, z.x1);
      const y0 = Math.min(z.y0, z.y1);
      const y1 = Math.max(z.y0, z.y1);
      const rx = x0 * cellW;
      const ry = ch - (y1 + 1) * cellH;
      const rw = (x1 - x0 + 1) * cellW;
      const rh = (y1 - y0 + 1) * cellH;

      ctx.fillStyle = z.team === 'red' ? 'rgba(220, 60, 60, 0.20)' : 'rgba(60, 140, 240, 0.20)';
      ctx.strokeStyle = z.team === 'red' ? '#ef4444' : '#3b82f6';
      ctx.lineWidth = 2;
      ctx.fillRect(rx, ry, rw, rh);
      ctx.strokeRect(rx, ry, rw, rh);
    }

    // 6. Waypoint Graph (Edges & Nodes)
    if (editorState.waypoints.nodes.length > 0) {
      const nodeById = new Map<string, { x: number; y: number }>();
      for (const n of editorState.waypoints.nodes) {
        nodeById.set(n.id, n);
      }
      ctx.strokeStyle = 'rgba(56, 189, 248, 0.78)';
      ctx.lineWidth = 1.5;
      ctx.setLineDash([4, 3]);
      ctx.beginPath();
      for (const [aId, bId] of editorState.waypoints.edges) {
        const a = nodeById.get(aId);
        const b = nodeById.get(bId);
        if (!a || !b) continue;
        ctx.moveTo((a.x + 0.5) * cellW, ch - (a.y + 0.5) * cellH);
        ctx.lineTo((b.x + 0.5) * cellW, ch - (b.y + 0.5) * cellH);
      }
      ctx.stroke();
      ctx.setLineDash([]);

      editorState.waypoints.nodes.forEach((n, idx) => {
        const nx = (n.x + 0.5) * cellW;
        const ny = ch - (n.y + 0.5) * cellH;
        ctx.fillStyle = idx === selectedWaypointIdx ? '#facc15' : '#38bdf8';
        ctx.strokeStyle = '#0f172a';
        ctx.lineWidth = 1.5;
        ctx.beginPath();
        ctx.arc(nx, ny, 4.5, 0, Math.PI * 2);
        ctx.fill();
        ctx.stroke();
      });
    }

    // 7. Key Spots
    editorState.keySpots.forEach((spot, idx) => {
      const sx = (spot.x + 0.5) * cellW;
      const sy = ch - (spot.y + 0.5) * cellH;
      const letter = String.fromCharCode(65 + idx);
      ctx.fillStyle = '#facc15';
      ctx.strokeStyle = '#000';
      ctx.lineWidth = 2;
      ctx.beginPath();
      ctx.arc(sx, sy, 8, 0, Math.PI * 2);
      ctx.fill();
      ctx.stroke();
      ctx.fillStyle = '#000';
      ctx.font = 'bold 10px monospace';
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      ctx.fillText(letter, sx, sy);
    });

    // 8. Camera Pin + FOV Wedge
    const camX = (previewPin.x + 0.5) * cellW;
    const camY = ch - (previewPin.y + 0.5) * cellH;
    const rad = ((previewPin.headingDeg - 90) * Math.PI) / 180;
    const fovHalf = (30 * Math.PI) / 180;
    const rayLen = 42;

    ctx.fillStyle = 'rgba(56, 189, 248, 0.22)';
    ctx.strokeStyle = '#38bdf8';
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    ctx.moveTo(camX, camY);
    ctx.lineTo(camX + Math.cos(rad - fovHalf) * rayLen, camY + Math.sin(rad - fovHalf) * rayLen);
    ctx.lineTo(camX + Math.cos(rad + fovHalf) * rayLen, camY + Math.sin(rad + fovHalf) * rayLen);
    ctx.closePath();
    ctx.fill();
    ctx.stroke();

    ctx.fillStyle = '#38bdf8';
    ctx.beginPath();
    ctx.arc(camX, camY, 5, 0, Math.PI * 2);
    ctx.fill();

    // 9. Hover Brush Outline
    if (hoverCell) {
      const [hx, hy] = hoverCell;
      const r = activeTool === 'paint' || activeTool === 'sculpt' || activeTool === 'line' ? brushRadius : 0;
      ctx.strokeStyle = 'rgba(255, 255, 255, 0.85)';
      ctx.lineWidth = 1.5;
      if (brushShape === 'circle' && r > 0) {
        ctx.beginPath();
        ctx.arc((hx + 0.5) * cellW, ch - (hy + 0.5) * cellH, (r + 0.5) * cellW, 0, Math.PI * 2);
        ctx.stroke();
      } else {
        const bx = (hx - r) * cellW;
        const by = ch - (hy + r + 1) * cellH;
        const bw = (r * 2 + 1) * cellW;
        const bh = (r * 2 + 1) * cellH;
        ctx.strokeRect(bx, by, bw, bh);
      }
    }
  }

  function renderWasmToCanvas(canvas: HTMLCanvasElement | null, kind: 'cockpit' | 'iso25d' | 'overhead') {
    if (!engine || !canvas) return;
    const w = canvas.width;
    const h = canvas.height;
    if (w <= 0 || h <= 0) return;
    const wasmKind =
      kind === 'overhead' ? KIND_OVERHEAD : kind === 'iso25d' ? KIND_ISO25D : KIND_COCKPIT;
    const drawn = engine.render(w, h, { offsetMdeg: 0, zoomPermille: 1000 }, wasmKind);
    if (!drawn) return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;
    ctx.putImageData(engine.image(), 0, 0);
  }

  $effect(() => {
    void renderTick;
    void hoverCell;
    void showContours;
    void showGridLines;
    void brushRadius;
    void brushShape;
    void activeTool;
    void overlayMode;
    void overlayDrive;
    void viewshedSizeCm;
    void highlightedCells;
    draw2DGridCanvas();
  });

  $effect(() => {
    void renderTick;
    void previewSubMode;
    if (!engine) return;
    if (previewSubMode === '3d' || previewSubMode === 'split') {
      renderWasmToCanvas(previewFpCanvas, 'cockpit');
    }
    if (previewSubMode === '25d' || previewSubMode === 'split') {
      renderWasmToCanvas(preview25dCanvas, 'iso25d');
    }
    if (previewSubMode === 'overhead' || previewSubMode === 'split') {
      renderWasmToCanvas(previewOverheadCanvas, 'overhead');
    }
  });

  function handleExportDraftJson() {
    const payload = exportGroundDraftJson(editorState);
    const blob = new Blob([JSON.stringify(payload, null, 2)], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `${editorState.id || 'custom_arena'}.draft.json`;
    a.click();
    URL.revokeObjectURL(url);
    statusNotice = `Exported ${editorState.id || 'custom_arena'}.draft.json (ground.draft.v1)`;
  }

  function handleImportDraftJson(e: Event) {
    const input = e.target as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) return;
    const reader = new FileReader();
    reader.onload = () => {
      try {
        const parsed = JSON.parse(String(reader.result)) as GroundDraftJson;
        pushSnapshot();
        editorState = importGroundDraftJson(parsed);
        if (
          editorState.width === 64 ||
          editorState.width === 128 ||
          editorState.width === 256
        ) {
          gridSize = editorState.width;
        }
        highlightedCells = [];
        statusNotice = `Imported ${file.name} (${editorState.width}x${editorState.height})`;
        refreshPreview();
      } catch (err) {
        statusNotice = `Import failed: ${(err as Error).message}`;
      }
    };
    reader.readAsText(file);
  }

  onMount(() => {
    let cancelled = false;
    pixelModule()
      .then((loaded) => {
        if (cancelled) return;
        engine = loaded;
        refreshPreview();
      })
      .catch(() => {
        // Fallback: 2D canvas editor still works if WASM fails to load
      });
    return () => {
      cancelled = true;
    };
  });
</script>

<div class="studio-shell" data-testid="map-editor-studio">
  <!-- Top Toolbar -->
  <header class="studio-header">
    <div class="studio-brand">
      <span class="studio-badge">GROUND.DRAFT.V1</span>
      <input
        class="studio-title-input"
        type="text"
        bind:value={editorState.label}
        aria-label="Arena label"
      />
      <input
        class="studio-id-input"
        type="text"
        bind:value={editorState.id}
        aria-label="Arena ID"
        title="Arena ID"
      />
    </div>

    <div class="studio-controls">
      <label class="ctrl-item">
        <span>Biome</span>
        <select
          value={editorState.biome}
          onchange={(e) => handleBiomeChange((e.currentTarget as HTMLSelectElement).value as BiomeName)}
        >
          {#each BIOME_OPTIONS as b (b)}
            <option value={b}>{b.toUpperCase()}</option>
          {/each}
        </select>
      </label>

      <label class="ctrl-item">
        <span>Grid</span>
        <select
          value={gridSize}
          onchange={(e) =>
            handleNewMap(
              Number((e.currentTarget as HTMLSelectElement).value) as 64 | 128 | 256,
              editorState.biome
            )}
        >
          <option value={64}>64×64 (640m)</option>
          <option value={128}>128×128 (1.28km)</option>
          <option value={256}>256×256 (2.56km)</option>
        </select>
      </label>

      <button type="button" class="btn" disabled={!canUndo} onclick={handleUndo} title="Undo">
        Undo
      </button>
      <button type="button" class="btn" disabled={!canRedo} onclick={handleRedo} title="Redo">
        Redo
      </button>

      <button type="button" class="btn" onclick={handleLoadServerDraft} title="Load draft from server">
        Load Server Draft
      </button>

      <button type="button" class="btn btn-primary" onclick={handleExportDraftJson}>
        Export Draft
      </button>
      <label class="btn file-btn">
        Import Draft
        <input type="file" accept=".json" onchange={handleImportDraftJson} hidden />
      </label>
    </div>
  </header>

  <div class="studio-body">
    <!-- Left Dock: Tools, Brushes, Overlays & Terrain Swatches -->
    <aside class="studio-dock-left">
      <section class="dock-section">
        <h3>Tools</h3>
        <div class="tool-grid">
          <button
            type="button"
            class="tool-btn"
            class:active={activeTool === 'paint'}
            onclick={() => (activeTool = 'paint')}
          >
            Paint
          </button>
          <button
            type="button"
            class="tool-btn"
            class:active={activeTool === 'line'}
            onclick={() => (activeTool = 'line')}
          >
            Line
          </button>
          <button
            type="button"
            class="tool-btn"
            class:active={activeTool === 'fill'}
            onclick={() => (activeTool = 'fill')}
          >
            Fill
          </button>
          <button
            type="button"
            class="tool-btn"
            class:active={activeTool === 'sculpt'}
            onclick={() => (activeTool = 'sculpt')}
          >
            Sculpt
          </button>
          <button
            type="button"
            class="tool-btn"
            class:active={activeTool === 'spot'}
            onclick={() => (activeTool = 'spot')}
          >
            Key Spot
          </button>
          <button
            type="button"
            class="tool-btn"
            class:active={activeTool === 'waypoint'}
            onclick={() => (activeTool = 'waypoint')}
          >
            Waypoints
          </button>
          <button
            type="button"
            class="tool-btn"
            class:active={activeTool === 'camera_pin'}
            onclick={() => (activeTool = 'camera_pin')}
          >
            3D Pin
          </button>
          <button
            type="button"
            class="tool-btn"
            class:active={activeTool === 'zone_red'}
            onclick={() => (activeTool = 'zone_red')}
          >
            Red Zone
          </button>
          <button
            type="button"
            class="tool-btn"
            class:active={activeTool === 'zone_blue'}
            onclick={() => (activeTool = 'zone_blue')}
          >
            Blue Zone
          </button>
        </div>
      </section>

      <section class="dock-section">
        <h3>Brush Settings</h3>
        <div class="brush-row">
          <span>Shape</span>
          <div class="pill-group">
            <button
              type="button"
              class="pill"
              class:active={brushShape === 'circle'}
              onclick={() => (brushShape = 'circle')}
            >
              Circle
            </button>
            <button
              type="button"
              class="pill"
              class:active={brushShape === 'square'}
              onclick={() => (brushShape = 'square')}
            >
              Square
            </button>
          </div>
        </div>
        <label class="slider-row">
          <span>Radius: {brushRadius} cells</span>
          <input type="range" min="0" max="16" step="1" bind:value={brushRadius} />
        </label>

        {#if activeTool === 'sculpt'}
          <div class="brush-row">
            <span>Mode</span>
            <select bind:value={sculptMode}>
              <option value="raise">Raise (+)</option>
              <option value="lower">Lower (-)</option>
              <option value="smooth">Smooth</option>
              <option value="flatten">Flatten to Target</option>
            </select>
          </div>
          <label class="slider-row">
            <span>Step: {(brushStrengthCm / 100).toFixed(1)} m</span>
            <input type="range" min="25" max="1000" step="25" bind:value={brushStrengthCm} />
          </label>
          {#if sculptMode === 'flatten'}
            <label class="slider-row">
              <span>Target: {(brushTargetElevCm / 100).toFixed(1)} m</span>
              <input
                type="range"
                min={DEFAULT_SLIDER_MIN_ELEV_CM}
                max={DEFAULT_SLIDER_MAX_ELEV_CM}
                step="100"
                bind:value={brushTargetElevCm}
              />
            </label>
          {/if}
          <label class="check-row">
            <input type="checkbox" bind:checked={relaxToPassable} />
            <span>Relax stroke to passable (≤45%)</span>
          </label>
        {/if}

        <label class="check-row">
          <input type="checkbox" bind:checked={showContours} />
          <span>Show 5m Contour Lines</span>
        </label>
        <label class="check-row">
          <input type="checkbox" bind:checked={showGridLines} />
          <span>Show Sector Grid</span>
        </label>
      </section>

      <section class="dock-section">
        <h3>Canvas Overlays & Repair</h3>
        <div class="brush-row">
          <span>Overlay</span>
          <select bind:value={overlayMode}>
            <option value="none">None</option>
            <option value="passability">Passability (§5)</option>
            <option value="viewshed">Viewshed (Camera Pin)</option>
          </select>
        </div>
        {#if overlayMode === 'passability'}
          <div class="brush-row">
            <span>Drive</span>
            <select bind:value={overlayDrive}>
              <option value="tracked">Tracked (≤45%)</option>
              <option value="wheeled">Wheeled (≤25%)</option>
              <option value="legged">Legged (≤70%)</option>
            </select>
          </div>
        {:else if overlayMode === 'viewshed'}
          <label class="slider-row">
            <span>Target Hull: {(viewshedSizeCm / 100).toFixed(1)} m</span>
            <input type="range" min="50" max="600" step="25" bind:value={viewshedSizeCm} />
          </label>
        {/if}

        <div class="repair-btn-row">
          <button
            type="button"
            class="btn"
            onclick={handleRelaxSlopes}
            title="Pull steep orthogonal edges down to ≤45% (roads ≤20%)"
          >
            Relax Slopes
          </button>
          <button
            type="button"
            class="btn"
            onclick={handleDespeckle}
            title="Replace non-built class islands under 3 cells with their majority neighbour"
          >
            Despeckle
          </button>
          <button
            type="button"
            class="btn"
            onclick={handleAutoRoute}
            title="Synthesize patrol waypoints over the tracked cost grid"
          >
            Auto-Route
          </button>
        </div>
        {#if lastRepairNotice}
          <div class="repair-notice">{lastRepairNotice}</div>
        {/if}
      </section>

      <section class="dock-section swatch-section">
        <h3>Terrain Palette</h3>
        <div class="swatch-list">
          {#each TERRAIN_PALETTE as swatch (swatch.code)}
            <button
              type="button"
              class="swatch-item"
              class:active={selectedClass === swatch.code}
              onclick={() => {
                selectedClass = swatch.code;
                if (activeTool !== 'paint' && activeTool !== 'line' && activeTool !== 'fill') {
                  activeTool = 'paint';
                }
              }}
            >
              <span class="swatch-color" style:background={swatch.color}>{swatch.glyph}</span>
              <span class="swatch-label">{swatch.label}</span>
            </button>
          {/each}
        </div>
      </section>
    </aside>

    <!-- Center: Interactive 2D Grid & Elevation Canvas -->
    <main class="studio-canvas-stage">
      <div class="canvas-frame">
        <canvas
          bind:this={gridCanvas}
          width={512}
          height={512}
          class="editor-grid-canvas"
          aria-label="map editor canvas"
          onmousedown={onCanvasPointerDown}
          onmousemove={onCanvasPointerMove}
          onmouseup={onCanvasPointerUp}
          onmouseleave={onCanvasPointerUp}
        ></canvas>
      </div>
      <footer class="canvas-hud">
        {#if hoverCell}
          {@const idx = hoverCell[1] * editorState.width + hoverCell[0]}
          {@const code = editorState.classes[idx] ?? 2}
          {@const elev = editorState.elevCm[idx] ?? 0}
          {@const swatch = TERRAIN_PALETTE.find((s) => s.code === code)}
          <span>Cell ({hoverCell[0]}, {hoverCell[1]})</span>
          <span>Class: {swatch?.label ?? 'Ground'}</span>
          <span>Elev: {(elev / 100).toFixed(2)} m</span>
        {:else}
          <span>Drag on canvas to paint or sculpt; Shift-click waypoints to connect edges.</span>
        {/if}
        {#if statusNotice}
          <span class="status-notice">{statusNotice}</span>
        {/if}
      </footer>
    </main>

    <!-- Right Dock: Live 3D / 2.5D / Overhead WASM Preview & §5 Check Report -->
    <aside class="studio-dock-right">
      <div class="preview-header">
        <h3>Live Engine Preview</h3>
        <div class="pill-group">
          <button
            type="button"
            class="pill"
            class:active={previewSubMode === 'split'}
            onclick={() => (previewSubMode = 'split')}
          >
            All
          </button>
          <button
            type="button"
            class="pill"
            class:active={previewSubMode === '3d'}
            onclick={() => (previewSubMode = '3d')}
          >
            1st-Person
          </button>
          <button
            type="button"
            class="pill"
            class:active={previewSubMode === '25d'}
            onclick={() => (previewSubMode = '25d')}
          >
            2.5D
          </button>
          <button
            type="button"
            class="pill"
            class:active={previewSubMode === 'overhead'}
            onclick={() => (previewSubMode = 'overhead')}
          >
            Overhead
          </button>
        </div>
      </div>

      <div class="camera-pin-bar">
        <span>Pin ({previewPin.x}, {previewPin.y})</span>
        <label>
          <span>Hdg {previewPin.headingDeg}°</span>
          <input
            type="range"
            min="0"
            max="359"
            step="5"
            value={previewPin.headingDeg}
            oninput={(e) => {
              previewPin = {
                ...previewPin,
                headingDeg: Number((e.currentTarget as HTMLInputElement).value)
              };
              refreshPreview();
            }}
          />
        </label>
      </div>

      <div class="preview-stack" class:single={previewSubMode !== 'split'}>
        {#if previewSubMode === '3d' || previewSubMode === 'split'}
          <div class="preview-card">
            <span class="preview-tag">1ST-PERSON COCKPIT (WASM)</span>
            <canvas
              bind:this={previewFpCanvas}
              width={320}
              height={180}
              aria-label="live 3d preview canvas"
            ></canvas>
          </div>
        {/if}
        {#if previewSubMode === '25d' || previewSubMode === 'split'}
          <div class="preview-card">
            <span class="preview-tag">2.5D ISOMETRIC TACTICAL (WASM)</span>
            <canvas bind:this={preview25dCanvas} width={320} height={180}></canvas>
          </div>
        {/if}
        {#if previewSubMode === 'overhead' || previewSubMode === 'split'}
          <div class="preview-card">
            <span class="preview-tag">OVERHEAD SATELLITE (WASM)</span>
            <canvas bind:this={previewOverheadCanvas} width={320} height={180}></canvas>
          </div>
        {/if}
      </div>

      <section class="dock-section check-report-panel" data-testid="ground-check-report">
        <div class="report-title-row">
          <h3>§5 Check Report</h3>
          <span class="verdict-badge" class:pass={checkReport.passed} class:fail={!checkReport.passed}>
            {checkReport.passed ? 'PASS' : 'FAIL'}
          </span>
        </div>
        <div class="report-grid">
          <span>Wire Size</span>
          <span class:bad={checkReport.map_bytes > checkReport.map_max_bytes}>
            {(checkReport.map_bytes / 1024).toFixed(1)} / {(checkReport.map_max_bytes / 1024).toFixed(0)} KiB
          </span>
          <span>Tracked / Legged</span>
          <span class:bad={checkReport.tracked_passable_pct < 90 || checkReport.legged_passable_pct < 95}>
            {checkReport.tracked_passable_pct}% / {checkReport.legged_passable_pct}%
          </span>
          <span>Wheeled Reach</span>
          <span>
            {checkReport.wheeled_passable_pct}% (Spots: {checkReport.key_spots_wheeled_reachable ? 'OK' : 'FAIL'})
          </span>
          <span>Zones Mutual</span>
          <span class:bad={!checkReport.zones_reachable_tracked || !checkReport.zones_reachable_legged}>
            T:{checkReport.zones_reachable_tracked ? 'Y' : 'N'} W:{checkReport.zones_reachable_wheeled ? 'Y' : 'N'} L:{checkReport.zones_reachable_legged ? 'Y' : 'N'}
          </span>
          <span>Max / Road Grade</span>
          <span class:bad={checkReport.max_road_grade_pct > 20}>
            {checkReport.max_grade_pct}% / {checkReport.max_road_grade_pct}% (≤20%)
          </span>
          <span>Muster Step</span>
          <span>{checkReport.max_entry_zone_step_cm} cm</span>
          <span>Waypoints</span>
          <span class:bad={!checkReport.waypoints_reachable}>
            {editorState.waypoints.nodes.length} nodes / {editorState.waypoints.edges.length} edges
          </span>
        </div>
        {#if checkReport.failures.length > 0}
          <ul class="failure-list">
            {#each checkReport.failures as fail (fail)}
              <li>{fail}</li>
            {/each}
          </ul>
        {/if}
      </section>

      <section class="dock-section spots-panel">
        <h3>Key Spots ({editorState.keySpots.length})</h3>
        <div class="spots-list">
          {#each editorState.keySpots as spot, i (i)}
            <div class="spot-row">
              <span class="spot-badge">{String.fromCharCode(65 + i)}</span>
              <input
                type="text"
                bind:value={spot.name}
                oninput={() => {
                  markStudioEdited(editorState);
                  refreshPreview();
                }}
                aria-label="Spot name"
              />
              <span class="spot-coords">({spot.x},{spot.y})</span>
              <button
                type="button"
                class="spot-Jump"
                onclick={() => {
                  previewPin = { ...previewPin, x: spot.x, y: spot.y };
                  refreshPreview();
                }}
                title="Jump camera pin here"
              >
                Cam
              </button>
            </div>
          {/each}
        </div>
      </section>
    </aside>
  </div>
</div>

<style>
  .studio-shell {
    display: flex;
    flex-direction: column;
    height: 100%;
    width: 100%;
    background: var(--bg, #0b0e14);
    color: var(--fg, #e2e8f0);
    font-family: var(--font-mono, monospace);
    font-size: 12px;
    overflow: hidden;
  }

  .studio-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 8px 14px;
    background: rgba(15, 20, 28, 0.95);
    border-bottom: 1px solid rgba(148, 163, 184, 0.18);
    flex-wrap: wrap;
  }

  .studio-brand {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .studio-badge {
    padding: 2px 6px;
    background: #1d4ed8;
    color: #fff;
    font-weight: 700;
    font-size: 10px;
    border-radius: 3px;
    letter-spacing: 0.06em;
  }

  .studio-title-input,
  .studio-id-input {
    background: rgba(30, 41, 59, 0.7);
    border: 1px solid rgba(148, 163, 184, 0.3);
    color: inherit;
    padding: 4px 8px;
    border-radius: 4px;
    font-family: inherit;
    font-weight: 600;
  }

  .studio-id-input {
    width: 120px;
    font-weight: 400;
    color: #94a3b8;
  }

  .studio-controls {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }

  .ctrl-item {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .ctrl-item select {
    background: #1e293b;
    color: inherit;
    border: 1px solid rgba(148, 163, 184, 0.3);
    border-radius: 4px;
    padding: 3px 6px;
    font-family: inherit;
    font-size: 11px;
  }

  .btn {
    background: #1e293b;
    color: #e2e8f0;
    border: 1px solid rgba(148, 163, 184, 0.3);
    border-radius: 4px;
    padding: 4px 9px;
    font-family: inherit;
    font-size: 11px;
    cursor: pointer;
  }

  .btn:hover:not(:disabled) {
    background: #334155;
  }

  .btn:disabled {
    opacity: 0.4;
    cursor: default;
  }

  .btn-primary {
    background: #0284c7;
    border-color: #38bdf8;
    color: #fff;
  }

  .studio-body {
    flex: 1;
    display: grid;
    grid-template-columns: 240px 1fr 340px;
    min-height: 0;
    overflow: hidden;
  }

  .studio-dock-left,
  .studio-dock-right {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 10px;
    background: rgba(13, 17, 24, 0.92);
    border-right: 1px solid rgba(148, 163, 184, 0.15);
    overflow-y: auto;
  }

  .studio-dock-right {
    border-right: none;
    border-left: 1px solid rgba(148, 163, 184, 0.15);
  }

  .dock-section {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding-bottom: 8px;
    border-bottom: 1px solid rgba(148, 163, 184, 0.12);
  }

  .dock-section h3,
  .preview-header h3 {
    margin: 0;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: #94a3b8;
  }

  .tool-grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 4px;
  }

  .tool-btn {
    background: #161f2e;
    color: #cbd5e1;
    border: 1px solid rgba(148, 163, 184, 0.22);
    border-radius: 4px;
    padding: 5px 4px;
    font-family: inherit;
    font-size: 10px;
    cursor: pointer;
  }

  .tool-btn.active {
    background: #0369a1;
    border-color: #38bdf8;
    color: #fff;
    font-weight: 600;
  }

  .brush-row,
  .slider-row,
  .check-row {
    display: flex;
    flex-direction: column;
    gap: 3px;
    font-size: 11px;
  }

  .check-row {
    flex-direction: row;
    align-items: center;
    gap: 6px;
  }

  .repair-btn-row {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
    margin-top: 2px;
  }

  .repair-notice {
    font-size: 10px;
    color: #facc15;
  }

  .pill-group {
    display: flex;
    gap: 4px;
  }

  .pill {
    flex: 1;
    background: #161f2e;
    color: #cbd5e1;
    border: 1px solid rgba(148, 163, 184, 0.22);
    border-radius: 3px;
    padding: 3px 6px;
    font-family: inherit;
    font-size: 10px;
    cursor: pointer;
  }

  .pill.active {
    background: #0284c7;
    color: #fff;
    border-color: #38bdf8;
  }

  .swatch-list {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 4px;
  }

  .swatch-item {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 6px;
    background: #131b28;
    border: 1px solid rgba(148, 163, 184, 0.18);
    border-radius: 4px;
    color: #e2e8f0;
    font-family: inherit;
    font-size: 10px;
    cursor: pointer;
    text-align: left;
  }

  .swatch-item.active {
    border-color: #38bdf8;
    background: #1e293b;
  }

  .swatch-color {
    width: 16px;
    height: 16px;
    border-radius: 3px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
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

  .studio-canvas-stage {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 12px;
    background: #07090e;
    min-height: 0;
  }

  .canvas-frame {
    display: flex;
    align-items: center;
    justify-content: center;
    max-width: 100%;
    max-height: calc(100% - 28px);
    aspect-ratio: 1 / 1;
    border: 1px solid rgba(56, 189, 248, 0.35);
    border-radius: 4px;
    overflow: hidden;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.65);
  }

  .editor-grid-canvas {
    width: 100%;
    height: 100%;
    cursor: crosshair;
    image-rendering: pixelated;
  }

  .canvas-hud {
    display: flex;
    gap: 16px;
    margin-top: 8px;
    color: #94a3b8;
    font-size: 11px;
  }

  .status-notice {
    color: #38bdf8;
  }

  .preview-header {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .camera-pin-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 4px 8px;
    background: #161f2e;
    border-radius: 4px;
    font-size: 11px;
  }

  .camera-pin-bar label {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .preview-stack {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .preview-card {
    position: relative;
    background: #05070b;
    border: 1px solid rgba(148, 163, 184, 0.25);
    border-radius: 4px;
    overflow: hidden;
  }

  .preview-card canvas {
    display: block;
    width: 100%;
    height: auto;
    image-rendering: pixelated;
  }

  .preview-tag {
    position: absolute;
    top: 4px;
    left: 6px;
    padding: 1px 5px;
    background: rgba(0, 0, 0, 0.68);
    color: #38bdf8;
    font-size: 9px;
    border-radius: 2px;
    pointer-events: none;
  }

  .report-title-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .verdict-badge {
    padding: 1px 6px;
    border-radius: 3px;
    font-weight: 700;
    font-size: 10px;
  }

  .verdict-badge.pass {
    background: rgba(22, 163, 74, 0.25);
    color: #4ade80;
    border: 1px solid #22c55e;
  }

  .verdict-badge.fail {
    background: rgba(220, 38, 38, 0.25);
    color: #f87171;
    border: 1px solid #ef4444;
  }

  .report-grid {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 3px 8px;
    font-size: 10px;
    color: #cbd5e1;
  }

  .report-grid .bad {
    color: #f87171;
    font-weight: 600;
  }

  .failure-list {
    margin: 4px 0 0;
    padding-left: 16px;
    color: #f87171;
    font-size: 10px;
  }

  .spots-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .spot-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .spot-badge {
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: #facc15;
    color: #000;
    font-weight: 700;
    font-size: 10px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }

  .spot-row input {
    flex: 1;
    background: #161f2e;
    border: 1px solid rgba(148, 163, 184, 0.25);
    color: inherit;
    padding: 2px 6px;
    border-radius: 3px;
    font-family: inherit;
    font-size: 11px;
  }

  .spot-coords {
    color: #64748b;
    font-size: 10px;
  }

  .spot-Jump {
    background: #1e293b;
    color: #38bdf8;
    border: 1px solid rgba(56, 189, 248, 0.35);
    border-radius: 3px;
    padding: 2px 5px;
    font-size: 10px;
    cursor: pointer;
  }
</style>
