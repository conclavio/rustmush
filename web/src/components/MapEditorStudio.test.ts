// @vitest-environment jsdom
import { fireEvent, render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { GameApp } from "$lib/game.svelte";
import MapEditorStudio from "./MapEditorStudio.svelte";

describe("MapEditorStudio", () => {
  beforeEach(() => {
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockImplementation(
      () =>
        ({
          clearRect: vi.fn(),
          fillRect: vi.fn(),
          strokeRect: vi.fn(),
          beginPath: vi.fn(),
          moveTo: vi.fn(),
          lineTo: vi.fn(),
          stroke: vi.fn(),
          arc: vi.fn(),
          fill: vi.fn(),
          fillText: vi.fn(),
          putImageData: vi.fn(),
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

  function mockApp() {
    const focusComposer = vi.fn();
    return {
      app: { focusComposer } as unknown as GameApp,
      focusComposer,
    };
  }

  it("renders toolbrushes, terrain palette, editing grid canvas, and live 3D preview controls", async () => {
    const { app } = mockApp();
    render(MapEditorStudio, { app });
    await tick();

    expect(screen.getByLabelText("map editor canvas")).toBeTruthy();
    expect(screen.getByLabelText("live 3d preview canvas")).toBeTruthy();
    expect(screen.getByText("Terrain Palette")).toBeTruthy();
    expect(screen.getByText("Toolbrushes")).toBeTruthy();

    // Cycle preview view mode
    const viewBtn = screen.getByRole("button", { name: /View: 1st-Person 3D/i });
    await fireEvent.click(viewBtn);
    expect(screen.getByRole("button", { name: /View: 2.5D Tactical/i })).toBeTruthy();
    await fireEvent.click(viewBtn);
    expect(screen.getByRole("button", { name: /View: Overhead/i })).toBeTruthy();
  });

  it("opens the Import / Export ArenaDef JSON drawer and pre-fills @atlas/window open", async () => {
    const { app, focusComposer } = mockApp();
    render(MapEditorStudio, { app });
    await tick();

    const exportBtn = screen.getByRole("button", { name: /Import \/ Export JSON/i });
    await fireEvent.click(exportBtn);
    await tick();

    const textarea = screen.getByLabelText("arena json buffer") as HTMLTextAreaElement;
    expect(textarea.value).toContain('"label": "The Proving Ground"');
    expect(textarea.value).toContain('"tile":');

    const atlasBtn = screen.getByRole("button", { name: /^@atlas\/window$/i });
    await fireEvent.click(atlasBtn);
    expect(focusComposer).toHaveBeenCalledWith(expect.stringContaining("@atlas/window open "));
  });
});
