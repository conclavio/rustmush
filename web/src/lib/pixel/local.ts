// The cockpit picture drawn here, in the browser (doc 21 cut G.3b): the
// wasm module fed this connection's own `arena.cockpit` and `arena.map`
// keyframes and drawn into the picture's canvas, so the gateway draws
// nothing for a web viewer. The server's picture (G.3c) is the fallback: a
// browser without WebAssembly or `ImageData`, a module that will not load
// (an older gateway's CSP refuses to compile it), one that traps, or a
// viewer who chose it.
import wasmUrl from "./mush_pixel.wasm?url";
import { FEED_COCKPIT, FEED_MAP, type PixelModule, loadPixelModule } from "./wasm";

/** Where this browser remembers whether the picture is drawn here or by the
 * server (`"server"` to choose the server; anything else draws here). */
export const PICTURE_SOURCE_KEY = "rustmush.picture.source";

/** The frame drawn: the gateway's own size (`mush_net_web::picture`). */
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

/** The module, loaded once for the page (a failure stays failed: the picture
 * falls back to the server until the page is reloaded). */
export function pixelModule(): Promise<PixelModule> {
  loading ??= loadPixelModule(wasmUrl);
  return loading;
}

/** **The picture drawn here**: the keyframes handed to the module when they
 * change (by identity: every keyframe is a fresh object), a frame drawn for
 * the look, and whether it differs from the last one shown. */
export class LocalPicture {
  readonly #m: PixelModule;
  #cockpit: unknown = undefined;
  #map: unknown = undefined;
  #shown: string | null = null;

  constructor(module: PixelModule) {
    this.#m = module;
  }

  /** Draw for `lookMdeg` over the held keyframes: whether a new frame is
   * ready for {@link image} (`false` with no cockpit, or unchanged). */
  draw(cockpit: unknown, map: unknown, lookMdeg: number): boolean {
    if (cockpit !== this.#cockpit) {
      this.#m.feed(FEED_COCKPIT, cockpit == null ? null : JSON.stringify(cockpit));
      this.#cockpit = cockpit;
    }
    if (map !== this.#map) {
      this.#m.feed(FEED_MAP, map == null ? null : JSON.stringify(map));
      this.#map = map;
    }
    if (!this.#m.render(FRAME_W, FRAME_H, { offsetMdeg: lookMdeg })) {
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
