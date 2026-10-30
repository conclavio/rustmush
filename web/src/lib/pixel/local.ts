// The cockpit and overhead pictures drawn here, in the browser (doc 21 cut
// G.3b; doc 28 P2): the wasm module fed this connection's own `arena.cockpit`
// and `arena.map` keyframes and drawn into the picture's canvas, so the
// gateway draws nothing for a web viewer.
import type { PictureKind } from "../picture";
import wasmUrl from "./mush_pixel.wasm?url";
import {
  FEED_COCKPIT,
  FEED_MAP,
  KIND_COCKPIT,
  KIND_ISO25D,
  KIND_OVERHEAD,
  type PixelModule,
  loadPixelModule,
} from "./wasm";

/** The frame drawn: 640x480 (doc 21 §5). */
export const FRAME_W = 640;
export const FRAME_H = 480;

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

/** The module, loaded once for the page. */
export function pixelModule(): Promise<PixelModule> {
  loading ??= loadPixelModule(wasmUrl);
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
  #kind: PictureKind | null = null;
  #shown: string | null = null;

  constructor(module: PixelModule) {
    this.#m = module;
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
      !this.#m.render(FRAME_W, FRAME_H, {
        offsetMdeg: hullOffsetMdeg + lookMdeg,
        hullOffsetMdeg,
        orderedMdeg,
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
