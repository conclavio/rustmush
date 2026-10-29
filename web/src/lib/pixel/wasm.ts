// The pixel engine drawn in the browser (doc 21 cut G.3b; doc 28 P2): the
// `mush-pixel-wasm` module's plain exports (no wasm-bindgen), wrapped. The
// ABI is the crate's own doc; this file is its one caller.
//
// The module owns its buffers: a feed's JSON is written into the one
// `mp_input` hands out, and a frame is read as RGBA from `mp_pixels`. Memory
// can grow on any call that allocates, which detaches every view over the
// old `memory.buffer`, so a view is taken fresh after each call and never
// kept.

/** The ABI version this file speaks (`mush_pixel_wasm::ABI_VERSION`). */
export const PIXEL_ABI = 2;

/** `mp_feed`'s number for `arena.cockpit`. */
export const FEED_COCKPIT = 1;
/** `mp_feed`'s number for `arena.map`. */
export const FEED_MAP = 2;
/** `mp_feed`'s number for `sky.chart`. */
export const FEED_CHART = 3;
/** `mp_feed`'s number for `arena.replay`. */
export const FEED_REPLAY = 4;
/** `mp_feed`'s number for `replay.cursor`. */
export const FEED_REPLAY_CURSOR = 5;

/** `mp_kind`'s number for the first-person cockpit (`CockpitPicture`). */
export const KIND_COCKPIT = 1;
/** `mp_kind`'s number for the cockpit's overhead camera (`OverheadPicture`). */
export const KIND_OVERHEAD = 2;
/** `mp_kind`'s number for the orbital sky chart (`ChartPicture`). */
export const KIND_CHART = 3;
/** `mp_kind`'s number for the armour doll page (`DollPicture`). */
export const KIND_DOLL = 4;
/** `mp_kind`'s number for the match replay (`ReplayPicture`). */
export const KIND_REPLAY = 5;

/** What the picture is drawn for: the look's offset from the hull, and the
 * hull's own offset from the frame's heading and an ordered heading when the
 * helm predicts one (millidegrees; `mush_pixel::picture::Look`). */
export interface PixelLook {
  offsetMdeg: number;
  hullOffsetMdeg?: number;
  orderedMdeg?: number | null;
}

interface PixelExports {
  memory: WebAssembly.Memory;
  mp_abi(): number;
  mp_kind(kind: number): number;
  mp_input(len: number): number;
  mp_feed(feed: number, len: number): number;
  mp_render(w: number, h: number, look: number, hull: number, ordered: number, hasOrdered: number): number;
  mp_pixels(): number;
  mp_hash_hi(): number;
  mp_hash_lo(): number;
  mp_reset(): void;
}

const EXPORTS: (keyof PixelExports)[] = [
  "memory",
  "mp_abi",
  "mp_kind",
  "mp_input",
  "mp_feed",
  "mp_render",
  "mp_pixels",
  "mp_hash_hi",
  "mp_hash_lo",
  "mp_reset",
];

/** **The picture module, instantiated.** Throws on a module that is not
 * this ABI. */
export class PixelModule {
  readonly #x: PixelExports;
  #width = 0;
  #height = 0;

  constructor(instance: WebAssembly.Instance) {
    const x = instance.exports as unknown as PixelExports;
    for (const name of EXPORTS) {
      if (!(name in x)) throw new Error(`picture module: no export ${name}`);
    }
    if (x.mp_abi() !== PIXEL_ABI) throw new Error(`picture module: ABI ${x.mp_abi()}, not ${PIXEL_ABI}`);
    this.#x = x;
  }

  /** Choose which picture {@link render} draws (`KIND_*`); whether the module
   * knew it. */
  setKind(kind: number): boolean {
    return this.#x.mp_kind(kind) === 1;
  }

  /** Keep a keyframe of `feed` (raw JSON text, or `null` to forget it);
   * whether the module kept it. */
  feed(feed: number, json: string | null): boolean {
    const bytes = json === null ? new Uint8Array(0) : new TextEncoder().encode(json);
    const ptr = this.#x.mp_input(bytes.length);
    new Uint8Array(this.#x.memory.buffer, ptr, bytes.length).set(bytes);
    return this.#x.mp_feed(feed, bytes.length) === 1;
  }

  /** Draw a `width` x `height` frame of the chosen picture (or `kind` when
   * given) for `look`; whether one was drawn. */
  render(width: number, height: number, look: PixelLook, kind?: number): boolean {
    if (kind !== undefined && !this.setKind(kind)) return false;
    const ordered = look.orderedMdeg ?? null;
    const drawn =
      this.#x.mp_render(
        width,
        height,
        Math.round(look.offsetMdeg) | 0,
        Math.round(look.hullOffsetMdeg ?? 0) | 0,
        ordered === null ? 0 : Math.round(ordered) | 0,
        ordered === null ? 0 : 1,
      ) === 1;
    if (drawn) {
      this.#width = width;
      this.#height = height;
    }
    return drawn;
  }

  /** The last frame as an `ImageData` over the module's memory (no copy):
   * draw it at once, before the next call into the module. */
  image(): ImageData {
    const len = this.#width * this.#height * 4;
    const px = new Uint8ClampedArray(this.#x.memory.buffer, this.#x.mp_pixels(), len);
    return new ImageData(px, this.#width, this.#height);
  }

  /** The last frame's RGBA, copied (tests). */
  pixels(): Uint8Array {
    const len = this.#width * this.#height * 4;
    return new Uint8Array(this.#x.memory.buffer, this.#x.mp_pixels(), len).slice();
  }

  /** The last frame's hash, 16 hex digits (`Frame::hash`). */
  hash(): string {
    const hex = (n: number) => (n >>> 0).toString(16).padStart(8, "0");
    return hex(this.#x.mp_hash_hi()) + hex(this.#x.mp_hash_lo());
  }

  /** A fresh picture (its animation from the start), the feeds kept. */
  reset(): void {
    this.#x.mp_reset();
  }
}

/** **Load the module from `url`**: streamed and compiled as it arrives where
 * the server says `application/wasm`, else fetched whole and compiled. */
export async function loadPixelModule(url: string): Promise<PixelModule> {
  let instance: WebAssembly.Instance;
  try {
    instance = (await WebAssembly.instantiateStreaming(fetch(url), {})).instance;
  } catch {
    const bytes = await (await fetch(url)).arrayBuffer();
    instance = (await WebAssembly.instantiate(bytes, {})).instance;
  }
  return new PixelModule(instance);
}
