// @vitest-environment jsdom
import { fireEvent, render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { GameApp } from "$lib/game.svelte";
import { createEditorMap, exportGroundDraftJson } from "$lib/map-editor";
import MapEditorStudio from "./MapEditorStudio.svelte";

vi.mock("../lib/pixel/local", () => ({
  pixelModule: () => Promise.reject(new Error("no wasm in jsdom")),
}));

describe("MapEditorStudio (WG.4 ground.draft.v1)", () => {
  beforeEach(() => {
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockImplementation(
      () =>
        ({
          clearRect: () => {},
          fillRect: () => {},
          strokeRect: () => {},
          beginPath: () => {},
          closePath: () => {},
          moveTo: () => {},
          lineTo: () => {},
          stroke: () => {},
          arc: () => {},
          fill: () => {},
          fillText: () => {},
          putImageData: () => {},
          setLineDash: () => {},
          save: () => {},
          restore: () => {},
          translate: () => {},
          scale: () => {},
          createImageData: (w: number, h: number) => ({
            data: new Uint8ClampedArray(w * h * 4),
            width: w,
            height: h,
          }),
          fillStyle: "",
          strokeStyle: "",
          lineWidth: 1,
          font: "",
        }) as unknown as CanvasRenderingContext2D,
    );
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  function mockApp(groundDraftFrame: Record<string, unknown> | null = null) {
    const requestGroundDraft = vi.fn();
    const stageGroundDraft = vi.fn(() => true);
    return {
      app: {
        requestGroundDraft,
        stageGroundDraft,
        groundDraftFrame,
      } as unknown as GameApp,
      requestGroundDraft,
      stageGroundDraft,
    };
  }

  it("renders ground.draft.v1 badge, toolbrushes, terrain palette, editing grid canvas, live preview, and §5 check report", async () => {
    const { app } = mockApp();
    render(MapEditorStudio, { app });
    await tick();

    expect(screen.getByText("GROUND.DRAFT.V1")).toBeTruthy();
    expect(screen.getByLabelText("map editor canvas")).toBeTruthy();
    expect(screen.getByLabelText("live 3d preview canvas")).toBeTruthy();
    expect(screen.getByText("Terrain Palette")).toBeTruthy();
    expect(screen.getByText("Tools")).toBeTruthy();
    expect(screen.getByTestId("ground-check-report")).toBeTruthy();

    // Verify no @atlas/window button exists (D-14-28)
    expect(screen.queryByRole("button", { name: /@atlas\/window/i })).toBeNull();

    // Switch preview sub-mode buttons
    const fpBtn = screen.getByRole("button", { name: /^1st-Person$/i });
    await fireEvent.click(fpBtn);
    expect(screen.getByLabelText("live 3d preview canvas")).toBeTruthy();
  });

  it("runs Relax Slopes, Despeckle, Auto-Route, and requests/stages server draft", async () => {
    const draft = exportGroundDraftJson(createEditorMap(64, 64, "mars", 8));
    const { app, requestGroundDraft, stageGroundDraft } = mockApp(
      draft as unknown as Record<string, unknown>,
    );
    render(MapEditorStudio, { app });
    await tick();

    const loadBtn = screen.getByRole("button", { name: /Load Server/i });
    await fireEvent.click(loadBtn);
    expect(requestGroundDraft).toHaveBeenCalled();

    const stageBtn = screen.getByRole("button", { name: /Stage to Server/i });
    await fireEvent.click(stageBtn);
    expect(stageGroundDraft).toHaveBeenCalled();

    const relaxBtn = screen.getByRole("button", { name: /Relax Slopes/i });
    await fireEvent.click(relaxBtn);
    await tick();
    expect(screen.getByText(/Relaxed \d+ steep cell/i)).toBeTruthy();

    const despeckleBtn = screen.getByRole("button", { name: /Despeckle/i });
    await fireEvent.click(despeckleBtn);
    await tick();
    expect(screen.getByText(/Despeckled \d+ cell/i)).toBeTruthy();

    const routeBtn = screen.getByRole("button", { name: /Auto-Route/i });
    await fireEvent.click(routeBtn);
    await tick();
    expect(screen.getByText(/Routed \d+ nodes/i)).toBeTruthy();
  });

  it("supports Procedural Generator presets, 512x512 grid + zoom controls, Hold Objective editor, and Tank Roster editor", async () => {
    const { app, requestGroundDraft } = mockApp();
    render(MapEditorStudio, { app });
    await tick();

    // 1. Server Arena selector passes arena name to requestGroundDraft
    const arenaSelect = screen.getByLabelText("Server arena draft") as HTMLSelectElement;
    await fireEvent.change(arenaSelect, { target: { value: "mars_canyon" } });
    const loadBtn = screen.getByRole("button", { name: /Load Server/i });
    await fireEvent.click(loadBtn);
    expect(requestGroundDraft).toHaveBeenCalledWith("mars_canyon");

    // 2. Procedural Generator panel generates preset maps
    expect(screen.getByTestId("procedural-generator-panel")).toBeTruthy();
    const presetSelect = screen.getByLabelText("Generator preset") as HTMLSelectElement;
    await fireEvent.change(presetSelect, { target: { value: "mars_canyon" } });
    const genBtn = screen.getByRole("button", { name: /Generate Preset/i });
    await fireEvent.click(genBtn);
    await tick();
    expect(screen.getByText(/Generated preset "mars_canyon"/i)).toBeTruthy();

    // 3. Zoom controls & 512x512 grid option
    const zoomInBtn = screen.getByLabelText("Zoom in");
    await fireEvent.click(zoomInBtn);
    await tick();
    const zoomResetBtn = screen.getByLabelText("Reset zoom");
    expect(zoomResetBtn.textContent).toContain("2x");

    const gridSelect = screen.getByLabelText("Grid size") as HTMLSelectElement;
    await fireEvent.change(gridSelect, { target: { value: "512" } });
    await tick();
    expect(screen.getByText(/2D Canvas \(512×512, 16Q\)/i)).toBeTruthy();

    // 4. Hold Objective & Tank Roster panels
    expect(screen.getByTestId("hold-objective-panel")).toBeTruthy();
    expect(screen.getByTestId("tank-roster-panel")).toBeTruthy();

    const musterBtn = screen.getByRole("button", { name: /Auto-Muster Tanks/i });
    await fireEvent.click(musterBtn);
    await tick();
    expect(screen.getByText(/Mustered 12 tanks/i)).toBeTruthy();
  });
});
