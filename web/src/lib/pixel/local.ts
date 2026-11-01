// The cockpit and overhead pictures drawn here, in the browser (doc 21 cut
// G.3b; doc 28 P2): the wasm module fed this connection's own `arena.cockpit`
// and `arena.map` keyframes and drawn into the picture's canvas, so the
// gateway draws nothing for a web viewer.
import type { PictureKind, SpaceCameraMode } from "../picture";
import wasmUrl from "./mush_pixel.wasm?url";
import {
  FEED_CHART,
  FEED_COCKPIT,
  FEED_MAP,
  FEED_REPLAY,
  FEED_REPLAY_CURSOR,
  FEED_SPACE_COCKPIT,
  FEED_SPACE_DOCKING,
  FEED_SPACE_HOLOTANK,
  KIND_COCKPIT,
  KIND_DOCKING,
  KIND_HOLOTANK,
  KIND_ISO25D,
  KIND_OVERHEAD,
  KIND_REPLAY,
  KIND_SPACE_COCKPIT,
  type ModelUrl,
  type PixelModule,
  loadPixelModule,
} from "./wasm";

/** Camera view mode for match replay and live `@arena/watch`. */
export type ReplayCameraMode = "overview" | "iso25d" | "first_person";

/** The frame drawn: 640x480 (doc 21 §5). */
export const FRAME_W = 640;
export const FRAME_H = 480;
/** High-definition widescreen frame drawn when HD mode is enabled (`1280x720`). */
export const FRAME_W_HD = 1280;
export const FRAME_H_HD = 720;

/** The clock: at most a frame every 50 ms (20 Hz, the cockpit's own). */
export const LOCAL_FRAME_MS = 50;

/** Whether this browser can draw the picture itself. */
export function localCapable(): boolean {
  return (
    typeof WebAssembly === "object" &&
    typeof ImageData === "function" &&
    typeof requestAnimationFrame === "function"
  );
}

let loading: Promise<PixelModule> | null = null;

// Doc 31 D-31-11: every model file `npm run wasm` copied beside the module,
// fingerprinted by Vite into `assets/` (served immutable) and fetched only
// when a unit drawn from it is in a feed.
const MODEL_URLS = import.meta.glob<string>("./models/*.mupk", {
  query: "?url",
  import: "default",
  eager: true,
});

/** Where model file `file` is served from. */
const modelUrl: ModelUrl = (file) => MODEL_URLS[`./models/${file}`];

/** The module, loaded once for the page. */
export function pixelModule(): Promise<PixelModule> {
  loading ??= loadPixelModule(wasmUrl, modelUrl);
  return loading;
}

/** **The picture drawn here**: the keyframes handed to the module when they
 * change (by identity: every keyframe is a fresh object), a frame drawn for
 * the camera (`cockpit` or `overhead`) and look, and whether it differs from
 * the last one shown. */
export class LocalPicture {
  readonly #m: PixelModule;
  #cockpit: unknown = undefined;
  #map: unknown = undefined;
  #holotank: unknown = undefined;
  #spaceCockpit: unknown = undefined;
  #docking: unknown = undefined;
  #chart: unknown = undefined;
  #kind: PictureKind | SpaceCameraMode | null = null;
  #shown: string | null = null;

  constructor(module: PixelModule) {
    this.#m = module;
  }

  /** Draw a space camera (`holotank`, `space_cockpit`, or `docking`) (`SF.18`). */
  drawSpace(
    spaceMode: SpaceCameraMode,
    holotank: unknown,
    spaceCockpit: unknown,
    docking: unknown,
    chart: unknown,
    lookMdeg = 0,
    zoomPermille = 1000,
    width = FRAME_W,
    height = FRAME_H,
  ): boolean {
    if (spaceMode !== this.#kind) {
      const wasmKind =
        spaceMode === "holotank"
          ? KIND_HOLOTANK
          : spaceMode === "docking"
            ? KIND_DOCKING
            : KIND_SPACE_COCKPIT;
      this.#m.setKind(wasmKind);
      this.#kind = spaceMode;
    }
    if (holotank !== this.#holotank) {
      this.#m.feed(FEED_SPACE_HOLOTANK, holotank == null ? null : JSON.stringify(holotank));
      this.#holotank = holotank;
    }
    if (spaceCockpit !== this.#spaceCockpit) {
      this.#m.feed(
        FEED_SPACE_COCKPIT,
        spaceCockpit == null ? null : JSON.stringify(spaceCockpit),
      );
      this.#spaceCockpit = spaceCockpit;
    }
    if (docking !== this.#docking) {
      this.#m.feed(FEED_SPACE_DOCKING, docking == null ? null : JSON.stringify(docking));
      this.#docking = docking;
    }
    if (chart !== this.#chart) {
      this.#m.feed(FEED_CHART, chart == null ? null : JSON.stringify(chart));
      this.#chart = chart;
    }
    if (
      !this.#m.render(width, height, {
        offsetMdeg: lookMdeg,
        zoomPermille,
      })
    ) {
      this.#shown = null;
      return false;
    }
    const hash = this.#m.hash();
    if (hash === this.#shown) return false;
    this.#shown = hash;
    return true;
  }

  /** Draw `kind` for `lookMdeg` over the held keyframes: whether a new frame is
   * ready for {@link image} (`false` with no cockpit, or unchanged). */
  draw(
    cockpit: unknown,
    map: unknown,
    lookMdeg: number,
    kind: PictureKind = "cockpit",
    hullOffsetMdeg = 0,
    orderedMdeg: number | null = null,
    dxCm = 0,
    dyCm = 0,
    zoomPermille = 1000,
    width = FRAME_W,
    height = FRAME_H,
  ): boolean {
    if (kind !== this.#kind) {
      const wasmKind =
        kind === "overhead" ? KIND_OVERHEAD : kind === "iso25d" ? KIND_ISO25D : KIND_COCKPIT;
      this.#m.setKind(wasmKind);
      this.#kind = kind;
    }
    if (cockpit !== this.#cockpit) {
      this.#m.feed(FEED_COCKPIT, cockpit == null ? null : JSON.stringify(cockpit));
      this.#cockpit = cockpit;
    }
    if (map !== this.#map) {
      this.#m.feed(FEED_MAP, map == null ? null : JSON.stringify(map));
      this.#map = map;
    }
    if (
      !this.#m.render(width, height, {
        offsetMdeg: hullOffsetMdeg + lookMdeg,
        hullOffsetMdeg,
        orderedMdeg,
        dxCm,
        dyCm,
        zoomPermille,
      })
    ) {
      this.#shown = null;
      return false;
    }
    const hash = this.#m.hash();
    if (hash === this.#shown) return false;
    this.#shown = hash;
    return true;
  }

  /** The frame {@link draw} made, over the module's memory: put it on the
   * canvas at once. */
  image(): ImageData {
    return this.#m.image();
  }

  /** Forget what was shown: the next {@link draw} reports its frame as new
   * whether or not the picture changed — for a canvas with nothing on it
   * yet (the picture hidden and shown again makes a fresh element; doc 21a
   * `X-04`: an unchanged scene was never painted onto it until something
   * moved). */
  forget(): void {
    this.#shown = null;
  }
}

/** Browser WASM renderer for `ReplayPicture` (`KIND_REPLAY`) across all three
 * views (`overview`, `iso25d` follow-cam, and `first_person` ride-along). */
export class LocalReplayPicture {
  readonly #m: PixelModule;
  #replayJson: string | null = null;
  #mapJson: string | null = null;
  #shown: string | null = null;

  constructor(module: PixelModule) {
    this.#m = module;
  }

  draw(
    replayDoc: unknown,
    map: unknown,
    matchId: number,
    tMs: number,
    focus: number,
    view: ReplayCameraMode,
    zoomPermille = 1000,
    lookMdeg = 0,
    width = FRAME_W,
    height = FRAME_H,
  ): boolean {
    this.#m.setKind(KIND_REPLAY);
    const replayPayload =
      replayDoc == null
        ? null
        : JSON.stringify({
            schema: { id: "arena.replay", version: 1, shape: "replay" },
            ...(replayDoc as Record<string, unknown>),
          });
    if (replayPayload !== this.#replayJson) {
      this.#m.feed(FEED_REPLAY, replayPayload);
      this.#replayJson = replayPayload;
    }
    const mapPayload = map == null ? null : JSON.stringify(map);
    if (mapPayload !== this.#mapJson) {
      this.#m.feed(FEED_MAP, mapPayload);
      this.#mapJson = mapPayload;
    }
    this.#m.feed(
      FEED_REPLAY_CURSOR,
      JSON.stringify({ match: matchId, t_ms: tMs, focus, view }),
    );
    if (!this.#m.render(width, height, { offsetMdeg: lookMdeg, zoomPermille })) {
      this.#shown = null;
      return false;
    }
    const hash = this.#m.hash();
    if (hash === this.#shown) return false;
    this.#shown = hash;
    return true;
  }

  image(): ImageData {
    return this.#m.image();
  }

  forget(): void {
    this.#shown = null;
  }
}
