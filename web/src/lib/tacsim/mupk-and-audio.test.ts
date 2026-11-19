import { readdirSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";
import type * as THREE from "three";

import {
  computeProceduralWaveformSamples,
  type ProceduralSfxKind,
  TacsimWebAudio,
} from "./audio";
import {
  AMBER_RIG_RAMP,
  BLUE_TEAM_RAMP,
  buildArticulatedFallbackUnitGroup,
  disposeObjectTree,
  buildThreeMeshFromHullData,
  carveMupkVisualHull,
  decodeMupkModel,
  RED_TEAM_RAMP,
} from "./mupk-mesh";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO_MODELS_DIR = resolve(HERE, "../../../../crates/mush-pixel/models");

function readShippedMupk(prefix: string): Uint8Array {
  const files = readdirSync(REPO_MODELS_DIR).filter(
    (f) => f.startsWith(`${prefix}.`) && f.endsWith(".mupk"),
  );
  expect(files.length).toBeGreaterThan(0);
  return new Uint8Array(readFileSync(resolve(REPO_MODELS_DIR, files[0]!)));
}

describe("Track 8: .mupk v5 3D visual-hull carver & procedural WebAudio", () => {
  it("decodes real shipped .mupk unit packs and carves culled 3D meshes", () => {
    for (const [prefix, ramp, targetH] of [
      ["vf.mech.brawler_med", RED_TEAM_RAMP, 2.25],
      ["mc.vh.bug_rover", AMBER_RIG_RAMP, 1.72],
      ["mc.st.tank_lvl2", BLUE_TEAM_RAMP, 1.95],
    ] as const) {
      const bytes = readShippedMupk(prefix);
      const decoded = decodeMupkModel(bytes, 24);
      expect(decoded).not.toBeNull();
      expect(decoded!.modelId).toBe(prefix);
      expect(decoded!.chosenSizePx).toBe(24);
      expect(decoded!.front.pixels.length).toBe(decoded!.front.w * decoded!.front.h);
      expect(decoded!.right.pixels.length).toBe(decoded!.right.w * decoded!.right.h);

      const hull = carveMupkVisualHull(decoded!, ramp, targetH);
      expect(hull.solidVoxelCount).toBeGreaterThan(20);
      expect(hull.quadCount).toBeGreaterThan(20);
      // 6-neighbor face culling must eliminate strictly internal shared faces:
      expect(hull.quadCount).toBeLessThan(hull.solidVoxelCount * 6);
      expect(hull.positions.length).toBe(hull.quadCount * 6 * 3);
      expect(hull.normals.length).toBe(hull.positions.length);
      expect(hull.colors.length).toBe(hull.positions.length);
      expect(hull.boundsMeters.heightY).toBeCloseTo(targetH, 2);

      const mesh = buildThreeMeshFromHullData(hull);
      expect(mesh.geometry.getAttribute("position")?.count).toBe(hull.quadCount * 6);
    }

    const fallbackMech = buildArticulatedFallbackUnitGroup("sim_mech", RED_TEAM_RAMP);
    const fallbackRover = buildArticulatedFallbackUnitGroup("prospector_rover", AMBER_RIG_RAMP);
    expect(fallbackMech.children.length).toBeGreaterThanOrEqual(8);
    expect(fallbackRover.children.length).toBeGreaterThanOrEqual(4);
  });

  it("disposes every geometry and material of a swapped-out fallback chassis (48e L8, clients:G17)", () => {
    const fallback = buildArticulatedFallbackUnitGroup("sim_mech", RED_TEAM_RAMP);
    const released = new Set<unknown>();
    const owned = new Set<unknown>();
    fallback.traverse((node) => {
      const mesh = node as THREE.Mesh;
      if (!mesh.isMesh) return;
      for (const res of [mesh.geometry, ...(Array.isArray(mesh.material) ? mesh.material : [mesh.material])]) {
        owned.add(res);
        res.addEventListener("dispose", () => released.add(res));
      }
    });
    expect(owned.size).toBeGreaterThan(2);
    disposeObjectTree(fallback);
    expect(released).toEqual(owned);
  });

  it("rejects invalid .mupk headers and unsupported format versions", () => {
    expect(decodeMupkModel(new Uint8Array([1, 2, 3]))).toBeNull();
    expect(() =>
      decodeMupkModel(new Uint8Array([0x42, 0x41, 0x44, 0x21, 5, 0, 1])),
    ).toThrow(/missing MUPK magic/);
    expect(() =>
      decodeMupkModel(new Uint8Array([0x4d, 0x55, 0x50, 0x4b, 3, 0, 1])),
    ).toThrow(/unsupported mupk version 3/);
  });

  it("synthesizes bounded non-silent PCM waveforms for all 8 diegetic SFX kinds and persists mute", () => {
    const kinds: ProceduralSfxKind[] = [
      "ui_chime",
      "market_fill",
      "airlock_cycle",
      "dock_clamp",
      "radar_ping",
      "laser_pulse",
      "railgun_crack",
      "torpedo_launch",
    ];

    for (const kind of kinds) {
      const pcm = computeProceduralWaveformSamples(kind, 22_050);
      expect(pcm.length).toBeGreaterThan(1_000);
      let peak = 0;
      for (let i = 0; i < pcm.length; i++) {
        const s = pcm[i]!;
        expect(Number.isFinite(s)).toBe(true);
        expect(s).toBeGreaterThanOrEqual(-1.0);
        expect(s).toBeLessThanOrEqual(1.0);
        if (Math.abs(s) > peak) peak = Math.abs(s);
      }
      expect(peak).toBeGreaterThan(0.15);
    }

    const audio = new TacsimWebAudio();
    const initial = audio.muted;
    expect(audio.toggleMuted()).toBe(!initial);
    expect(audio.toggleMuted()).toBe(initial);
  });
});
