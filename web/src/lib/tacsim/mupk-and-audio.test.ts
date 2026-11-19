import { readdirSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it, vi } from "vitest";
import * as THREE from "three";

import {
  computeProceduralWaveformSamples,
  type ProceduralSfxKind,
  sanitizeSampleRate,
  TacsimWebAudio,
} from "./audio";
import {
  AMBER_RIG_RAMP,
  BLUE_TEAM_RAMP,
  buildArticulatedFallbackUnitGroup,
  buildThreeMeshFromHullData,
  carveMupkVisualHull,
  decodeMupkModel,
  type DecodedMupkFrame2D,
  type DecodedMupkModel,
  disposeObject3DResources,
  isValidMupkPrefix,
  RED_TEAM_RAMP,
  resolveMupkAssetUrl,
  shadedMupkRgb,
  srgbChannelToLinear,
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

  it("preserves right-handed L/R chirality (W3D-M1), linearizes sRGB vertex colors (W3D-M2), and disposes fallback GPU objects", () => {
    const N = 8;
    const makeFrame = (
      fill: (du: number, dv: number) => number,
    ): DecodedMupkFrame2D => {
      const pixels = new Uint8Array(N * N);
      for (let py = 0; py < N; py++) {
        const dv = N - 1 - py;
        for (let px = 0; px < N; px++) {
          const du = px - N / 2;
          pixels[py * N + px] = fill(du, dv);
        }
      }
      return {
        sizePx: N,
        x: 0,
        y: 0,
        w: N,
        h: N,
        anchorX: N / 2,
        anchorY: N - 1,
        heightPx: N,
        pixels,
      };
    };

    // Centered torso (du in -1..1, dv in 0..3) + asymmetric UNIT-LEFT pod at front viewer-right (du = +2, dv = 2)
    // and matching rear viewer-left (du = -2, dv = 2).
    const LEFT_PX = 0x73; // RED_TEAM_RAMP.light (distinct bright red)
    const RIGHT_PX = 0x78; // steelLight (distinct neutral gray-blue)
    const FRONT_PX = 0x05;
    const model: DecodedMupkModel = {
      modelId: "test.asymmetric",
      assetPack: "test",
      prefab: "test",
      sizeCm: [200, 200],
      spanCm: 200,
      chosenSizePx: N,
      front: makeFrame((du, dv) =>
        dv >= 0 && dv <= 3 && ((du >= -1 && du <= 1) || (du === 2 && dv === 2)) ? FRONT_PX : 0,
      ),
      rear: makeFrame((du, dv) =>
        dv >= 0 && dv <= 3 && ((du >= -1 && du <= 1) || (du === -2 && dv === 2)) ? FRONT_PX : 0,
      ),
      left: makeFrame((du, dv) => (dv >= 0 && dv <= 3 && du >= -1 && du <= 1 ? LEFT_PX : 0)),
      right: makeFrame((du, dv) => (dv >= 0 && dv <= 3 && du >= -1 && du <= 1 ? RIGHT_PX : 0)),
    };

    const hull = carveMupkVisualHull(model, RED_TEAM_RAMP, 2.0);
    expect(hull.solidVoxelCount).toBeGreaterThan(0);

    // Asymmetric unit-left pod must protrude strictly farther on +X (unit LEFT) than -X (unit RIGHT)
    // relative to the torso center; verify the +X extreme has a voxel at dv=2 (middle height) whereas -X extreme does not.
    let minX = Infinity;
    let maxX = -Infinity;
    for (let i = 0; i < hull.positions.length; i += 3) {
      const x = hull.positions[i]!;
      if (x < minX) minX = x;
      if (x > maxX) maxX = x;
    }
    const leftPodFaceTris = [];
    const rightWallFaceTris = [];
    for (let v = 0; v < hull.positions.length; v += 3) {
      const nx = hull.normals[v]!;
      const x = hull.positions[v]!;
      if (nx === 1 && Math.abs(x - maxX) < 1e-4) leftPodFaceTris.push(v);
      if (nx === -1 && Math.abs(x - minX) < 1e-4) rightWallFaceTris.push(v);
    }
    // Pod is only 1 voxel tall (1×3 voxels = 3 quads = 18 verts on +X face), whereas flat torso right side is 4×3 = 12 quads = 72 verts!
    expect(leftPodFaceTris.length).toBeLessThan(rightWallFaceTris.length);

    // W3D-M2: +X face carries linearized pLeft color; -X face carries linearized pRight color
    const expectedLeftRgb = shadedMupkRgb(LEFT_PX, RED_TEAM_RAMP);
    const expectedRightRgb = shadedMupkRgb(RIGHT_PX, RED_TEAM_RAMP);
    const vLeft = leftPodFaceTris[0]!;
    const vRight = rightWallFaceTris[0]!;
    expect(hull.colors[vLeft]).toBeCloseTo(srgbChannelToLinear(expectedLeftRgb[0]), 4);
    expect(hull.colors[vLeft + 1]).toBeCloseTo(srgbChannelToLinear(expectedLeftRgb[1]), 4);
    expect(hull.colors[vRight]).toBeCloseTo(srgbChannelToLinear(expectedRightRgb[0]), 4);
    expect(srgbChannelToLinear(128)).toBeCloseTo(0.21586, 4);
    expect(srgbChannelToLinear(128)).toBeLessThan(128 / 255);

    // 48d mupk-mesh.ts:640 — disposeObject3DResources disposes every fallback geometry & material
    const fallback = buildArticulatedFallbackUnitGroup("sim_mech", RED_TEAM_RAMP);
    const geos = new Set<THREE.BufferGeometry>();
    const mats = new Set<THREE.Material>();
    fallback.traverse((obj) => {
      const m = obj as THREE.Mesh;
      if (m.geometry) geos.add(m.geometry);
      if (m.material && !Array.isArray(m.material)) mats.add(m.material);
    });
    expect(geos.size).toBe(9);
    expect(mats.size).toBe(3);
    const geoSpies = [...geos].map((g) => vi.spyOn(g, "dispose"));
    const matSpies = [...mats].map((m) => vi.spyOn(m, "dispose"));
    disposeObject3DResources(fallback);
    for (const s of geoSpies) expect(s).toHaveBeenCalledTimes(1);
    for (const s of matSpies) expect(s).toHaveBeenCalledTimes(1);
    expect(fallback.userData["disposed"]).toBe(true);
  });

  it("enforces .mupk prefix allowlist and bake.rs bounds caps (W3D-L1)", () => {
    expect(isValidMupkPrefix("vf.mech.brawler_med")).toBe(true);
    expect(isValidMupkPrefix("mc.vh.bug_rover")).toBe(true);
    expect(isValidMupkPrefix("../etc/passwd")).toBe(false);
    expect(isValidMupkPrefix("sub/dir.model")).toBe(false);
    expect(isValidMupkPrefix("bad name")).toBe(false);
    expect(resolveMupkAssetUrl("../secret")).toBeNull();

    // Minimal v5 header declaring sizePx = 0 (rejected by bake.rs:524)
    const badSizeZero = new Uint8Array([
      0x4d, 0x55, 0x50, 0x4b, // MUPK
      5, 0, // version 5 (u16 LE)
      1, // entryCount = 1
      1, 0x61, // modelId = "a"
      0, // assetPack = ""
      0, // assetVersion = ""
      0, // prefab = ""
      0, // clipCount = 0
      0, // bakeCommit = ""
      0, // reduceCommit = ""
      100, // widthCm
      100, // heightCm
      0, // muzzleNameCount = 0
      1, // chainCount = 1
      0, // viewTag = fp
      100, // spanCm
      1, // sizeCount = 1
      0, // sizePx = 0 (invalid per bake.rs:524)
    ]);
    expect(() => decodeMupkModel(badSizeZero)).toThrow(/size_px must be > 0/);

    // Entry count exceeding MUPK_MAX_LIST (64) is rejected
    const badEntryCount = new Uint8Array([0x4d, 0x55, 0x50, 0x4b, 5, 0, 65]);
    expect(() => decodeMupkModel(badEntryCount)).toThrow(/entry count/);
  });

  it("handles 96/192 kHz DACs, autoplay rejections, NaN guards, master gain mute, smooth envelopes, and laser routing (W3D-M3/M4)", () => {
    expect(sanitizeSampleRate(96_000)).toBe(96_000);
    expect(sanitizeSampleRate(192_000)).toBe(192_000);
    expect(sanitizeSampleRate(Number.NaN)).toBe(22_050);

    // Smoothness check on ui_chime & dock_clamp (no abrupt click jump across transitions)
    for (const kind of ["ui_chime", "dock_clamp", "radar_ping"] as const) {
      const pcm = computeProceduralWaveformSamples(kind, 48_000);
      let maxDelta = 0;
      for (let i = 1; i < pcm.length; i++) {
        const d = Math.abs(pcm[i]! - pcm[i - 1]!);
        if (d > maxDelta) maxDelta = d;
      }
      expect(maxDelta).toBeLessThan(0.35);
    }

    // Mock AudioContext at 96 kHz + verify createBuffer sample rate, NaN guard, master gain, and routing
    const createdBuffers: Array<{ length: number; sampleRate: number }> = [];
    const gainNodes: Array<{ gain: { value: number } }> = [];
    let startedCount = 0;
    let mockState: "running" | "suspended" = "running";
    let mockSampleRate = 96_000;
    let resumeRejects = false;

    class MockAudioContext {
      get sampleRate() {
        return mockSampleRate;
      }
      get state() {
        return mockState;
      }
      destination = {};
      resume() {
        return resumeRejects ? Promise.reject(new Error("autoplay blocked")) : Promise.resolve();
      }
      createBuffer(_ch: number, length: number, sampleRate: number) {
        createdBuffers.push({ length, sampleRate });
        const data = new Float32Array(length);
        return { getChannelData: () => data };
      }
      createBufferSource() {
        return {
          buffer: null as unknown,
          connect: () => {},
          start: () => {
            startedCount++;
          },
        };
      }
      createGain() {
        const g = { gain: { value: 1 }, connect: () => {} };
        gainNodes.push(g);
        return g;
      }
    }

    const g = globalThis as unknown as { AudioContext?: unknown };
    const origCtx = g.AudioContext;
    g.AudioContext = MockAudioContext;
    try {
      const audio = new TacsimWebAudio();
      audio.setMuted(false);

      // 96 kHz sample rate matches synthesized buffer rate (no 2x chipmunk pitch shift!)
      expect(audio.play("ui_chime", Number.NaN)).toBe(true);
      expect(createdBuffers[0]?.sampleRate).toBe(96_000);
      expect(createdBuffers[0]?.length).toBe(Math.floor(96_000 * 0.085));
      // Master gain (gainNodes[0]) is 1, clip gain (gainNodes[1]) fell back from NaN to 0.28
      expect(gainNodes[0]?.gain.value).toBe(1);
      expect(gainNodes[1]?.gain.value).toBeCloseTo(0.28, 5);

      // Muting zeroes master gain immediately
      audio.setMuted(true);
      expect(gainNodes[0]?.gain.value).toBe(0);
      audio.setMuted(false);
      expect(gainNodes[0]?.gain.value).toBe(1);

      // Suspended context with rejecting resume(): returns false, does NOT call start(), no unhandled rejection
      const beforeSuspended = startedCount;
      mockState = "suspended";
      resumeRejects = true;
      expect(audio.play("dock_clamp", 0.4)).toBe(false);
      expect(startedCount).toBe(beforeSuspended);

      // playForCommand routing (W3D-M4)
      mockState = "running";
      const playSpy = vi.spyOn(audio, "play");
      audio.playForCommand("FIRE laser_dorsal");
      audio.playForCommand("+tac/beam");
      audio.playForCommand("FIRE missile_pod");
      audio.playForCommand("+tac/torp");
      audio.playForCommand("FIRE rail_spinal");
      audio.playForCommand("+tac/scan");
      expect(playSpy.mock.calls.map((c) => c[0])).toEqual([
        "laser_pulse",
        "laser_pulse",
        "torpedo_launch",
        "torpedo_launch",
        "railgun_crack",
        "radar_ping",
      ]);

      // 192 kHz context works without clamping down to 48 kHz
      mockSampleRate = 192_000;
      const hiResAudio = new TacsimWebAudio();
      hiResAudio.setMuted(false);
      expect(hiResAudio.play("radar_ping", 0.3)).toBe(true);
      expect(createdBuffers[createdBuffers.length - 1]?.sampleRate).toBe(192_000);
    } finally {
      g.AudioContext = origCtx;
    }
  });
});
