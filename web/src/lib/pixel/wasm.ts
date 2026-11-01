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
export const PIXEL_ABI = 3;

/** Where model file `file` (`models/index.json`'s `file`, doc 31 D-31-2) is
 * fetched from, or `undefined` for a file this page does not serve. */
export type ModelUrl = (file: string) => string | undefined;

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
/** `mp_feed`'s number for `space.holotank` (doc 35 cut SF.7). */
export const FEED_SPACE_HOLOTANK = 6;
/** `mp_feed`'s number for `space.cockpit` (doc 35 cut SF.7). */
export const FEED_SPACE_COCKPIT = 7;
/** `mp_feed`'s number for `space.docking` (doc 35 cut SF.7). */
export const FEED_SPACE_DOCKING = 8;

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
/** `mp_kind`'s number for the cockpit's 2.5D tactical camera (`Iso25DPicture`). */
export const KIND_ISO25D = 6;
/** `mp_kind`'s number for the 3D Space Holo-Tank (`HolotankPicture`, doc 35 cut SF.7). */
export const KIND_HOLOTANK = 7;
/** `mp_kind`'s number for the First-Person Space Bridge Canopy (`SpaceCockpitPicture`, doc 35 cut SF.7). */
export const KIND_SPACE_COCKPIT = 8;
/** `mp_kind`'s number for the 6-DOF Bow Docking Camera (`DockingPicture`, doc 35 cut SF.7). */
export const KIND_DOCKING = 9;

/** What the picture is drawn for: the look's offset from the hull, and the
 * hull's own offset from the frame's heading and an ordered heading when the
 * helm predicts one (millidegrees; `mush_pixel::picture::Look`). */
export interface PixelLook {
  offsetMdeg: number;
  hullOffsetMdeg?: number;
  orderedMdeg?: number | null;
  dxCm?: number;
  dyCm?: number;
  zoomPermille?: number;
}

interface PixelExports {
  memory: WebAssembly.Memory;
  mp_abi(): number;
  mp_kind(kind: number): number;
  mp_input(len: number): number;
  mp_feed(feed: number, len: number): number;
  mp_offset_cm?(dxCm: number, dyCm: number): void;
  mp_zoom_permille?(zoomPermille: number): void;
  mp_render(w: number, h: number, look: number, hull: number, ordered: number, hasOrdered: number): number;
  mp_pixels(): number;
  mp_hash_hi(): number;
  mp_hash_lo(): number;
  mp_reset(): void;
  mp_units?(len: number): number;
  mp_wanted?(): number;
  mp_output?(): number;
  mp_check_look?(len: number): number;
  mp_wanted_looks?(): number;
  mp_looks?(len: number): number;
  mp_preview?(len: number): number;
}

/** The look check's answer (`mush_pixel::design::LookCheck`). */
export interface LookCheck {
  refusals: string[];
  notes: string[];
}

/** What the designer's preview draws (`mp_preview`). */
export interface PreviewSpec {
  model: string;
  view: "fp" | "iso";
  size: number;
  facing: string;
  pose: number;
  team: "red" | "blue";
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
  #modelUrl: ModelUrl | null = null;
  readonly #requested = new Set<string>();
  #askLooks: ((types: string[]) => void) | null = null;
  readonly #looksAsked = new Set<string>();

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
    const kept = this.#x.mp_feed(feed, bytes.length) === 1;
    if (kept) {
      this.#fetchWanted();
      this.#askWantedLooks();
    }
    return kept;
  }

  /** **Ask for published designs' looks through `ask`** (doc 31 cut UD.3b,
   * D-31-16): after every feed kept, each unit type the held feeds name
   * that has no look yet is asked for once, at most eight a call (the
   * `unit.looks` query's bound); the answer comes back through
   * {@link installLooks}. Until it does the unit draws as `unknown`. */
  askLooksWith(ask: (types: string[]) => void): void {
    this.#askLooks = ask;
    this.#askWantedLooks();
  }

  /** The unit types the held feeds name that have no look (`mp_wanted_looks`). */
  wantedLooks(): string[] {
    if (typeof this.#x.mp_wanted_looks !== "function" || typeof this.#x.mp_output !== "function") return [];
    const len = this.#x.mp_wanted_looks();
    const text = new TextDecoder().decode(new Uint8Array(this.#x.memory.buffer, this.#x.mp_output(), len));
    try {
      return JSON.parse(text) as string[];
    } catch {
      return [];
    }
  }

  /** Install a `unit.looks` keyframe's looks: how many were new. */
  installLooks(frame: unknown): number {
    if (typeof this.#x.mp_looks !== "function") return 0;
    return this.#x.mp_looks(this.#write(JSON.stringify(frame)));
  }

  #askWantedLooks(): void {
    const ask = this.#askLooks;
    if (ask === null) return;
    const fresh = this.wantedLooks().filter((t) => !this.#looksAsked.has(t));
    for (let i = 0; i < fresh.length; i += 8) {
      const batch = fresh.slice(i, i + 8);
      for (const t of batch) this.#looksAsked.add(t);
      ask(batch);
    }
  }

  /** **Fetch model files from `resolve` as the feeds need them** (doc 31
   * D-31-11): after every feed kept, each model a unit in the held feeds is
   * drawn from and the module lacks is fetched once and installed; until it
   * arrives the unit draws its look's text sheet. A fetch that fails is
   * tried again on a later feed. */
  fetchModelsFrom(resolve: ModelUrl): void {
    this.#modelUrl = resolve;
    this.#fetchWanted();
  }

  /** The model files the held feeds need and the module lacks
   * (`mp_wanted`). */
  wantedModels(): string[] {
    if (typeof this.#x.mp_wanted !== "function" || typeof this.#x.mp_output !== "function") return [];
    const len = this.#x.mp_wanted();
    if (len === 0) return [];
    const text = new TextDecoder().decode(new Uint8Array(this.#x.memory.buffer, this.#x.mp_output(), len));
    return text.split("\n").filter((f) => f.length > 0);
  }

  #write(text: string): number {
    const bytes = new TextEncoder().encode(text);
    const ptr = this.#x.mp_input(bytes.length);
    new Uint8Array(this.#x.memory.buffer, ptr, bytes.length).set(bytes);
    return bytes.length;
  }

  /** **The staff unit designer's look check** (doc 31 cut UD.2): `look`
   * against the unit `rules` (its `def`) and the model it names. A module
   * without the export answers one refusal saying so. */
  checkLook(look: unknown, rules: unknown): LookCheck {
    if (typeof this.#x.mp_check_look !== "function" || typeof this.#x.mp_output !== "function") {
      return { refusals: ["This picture module has no look check; rebuild it."], notes: [] };
    }
    const len = this.#x.mp_check_look(this.#write(JSON.stringify({ look, rules })));
    const text = new TextDecoder().decode(new Uint8Array(this.#x.memory.buffer, this.#x.mp_output(), len));
    return JSON.parse(text) as LookCheck;
  }

  /** **The designer's model preview**: one frame of a model's chain, a
   * `side` x `side` RGBA copy, or `null` for a model the catalogue lacks. */
  preview(spec: PreviewSpec): { side: number; rgba: Uint8ClampedArray<ArrayBuffer> } | null {
    if (typeof this.#x.mp_preview !== "function") return null;
    const side = this.#x.mp_preview(this.#write(JSON.stringify(spec)));
    if (side === 0) return null;
    const rgba = new Uint8ClampedArray(this.#x.memory.buffer, this.#x.mp_pixels(), side * side * 4).slice();
    return { side, rgba };
  }

  #fetchWanted(): void {
    const resolve = this.#modelUrl;
    if (resolve === null) return;
    for (const file of this.wantedModels()) {
      if (this.#requested.has(file)) continue;
      const url = resolve(file);
      if (url === undefined) continue;
      this.#requested.add(file);
      void fetch(url)
        .then((r) => (r.ok ? r.arrayBuffer() : Promise.reject(new Error(`${r.status}`))))
        .then((bytes) => {
          this.loadUnits(new Uint8Array(bytes));
        })
        .catch(() => {
          this.#requested.delete(file);
        });
    }
  }

  /** Draw a `width` x `height` frame of the chosen picture (or `kind` when
   * given) for `look`; whether one was drawn. */
  render(width: number, height: number, look: PixelLook, kind?: number): boolean {
    if (kind !== undefined && !this.setKind(kind)) return false;
    if (typeof this.#x.mp_offset_cm === "function") {
      this.#x.mp_offset_cm(Math.round(look.dxCm ?? 0) | 0, Math.round(look.dyCm ?? 0) | 0);
    }
    if (typeof this.#x.mp_zoom_permille === "function") {
      this.#x.mp_zoom_permille(Math.round(look.zoomPermille ?? 1000) >>> 0);
    }
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

  /** **Install one model's file** (doc 31 D-31-2: a baked model's size
   * chains, which the module does not embed); whether the module added it.
   * Until a model is added the units on it draw their look's text sheet. */
  loadUnits(pack: Uint8Array): boolean {
    if (typeof this.#x.mp_units !== "function") return false;
    const ptr = this.#x.mp_input(pack.length);
    new Uint8Array(this.#x.memory.buffer, ptr, pack.length).set(pack);
    return this.#x.mp_units(pack.length) === 1;
  }
}

/** **Load the module from `url`**: streamed and compiled as it arrives where
 * the server says `application/wasm`, else fetched whole and compiled; then,
 * when `modelUrl` is given, the model files are fetched from it as the feeds
 * need them ({@link PixelModule.fetchModelsFrom}; a model that does not
 * arrive leaves its units on their look's text sheet, never a failed load). */
export async function loadPixelModule(url: string, modelUrl?: ModelUrl): Promise<PixelModule> {
  let instance: WebAssembly.Instance;
  try {
    instance = (await WebAssembly.instantiateStreaming(fetch(url), {})).instance;
  } catch {
    const bytes = await (await fetch(url)).arrayBuffer();
    instance = (await WebAssembly.instantiate(bytes, {})).instance;
  }
  const module = new PixelModule(instance);
  if (modelUrl !== undefined) module.fetchModelsFrom(modelUrl);
  return module;
}
