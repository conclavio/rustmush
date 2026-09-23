// Screens (interface plan I4, D-I-7, D-I-17, D-I-18, D-I-19): the
// top-level concept of the browser client. The built-ins share their names
// and digits with the TUI (1 scene, 2 sheet, 3 mail, 4 chan, 5 rolls, 7
// actions, 0 cockpit; 8 build and 9 llm are the web's own; 6 intel is the
// TUI's), and a ruleset screen registers by a panel-feed suffix so it
// appears only when the server's manifest carries the feed. One binding
// table per screen, with the same `local` / `prefill` / `run` kinds and the
// same read-only rule for `run` as the TUI (`RUN_ALLOWLIST`, checked by
// `validateScreens`).
//
// 16b cut Q.5 added the tenth digit. `0` is LAST in the strip — the order of
// the digits on a keyboard, and the same order the TUI's tab bar reads — so
// it sorts after 9 rather than before 1, and `activeDef`'s fallback is still
// the scene screen and not the cockpit.

import { COCKPIT_FEED } from "../cockpit";
import { GLOBAL_KEYS, JUMP_KEYS, RESERVED_EXIT_KEYS } from "../keys";

/** What a bound key does (D-I-18). `toggle` flips the active screen's own
 * layout where it has two (interface cut I8, D-I8-4). */
export type KeyAction =
  | { kind: "local"; action: "refresh" | "clear" | "who" | "toggle" }
  | { kind: "prefill"; line: string }
  | { kind: "run"; cmd: string };

export interface KeyBinding {
  key: string;
  label: string;
  action: KeyAction;
}

/** What a screen's gate may read. */
export interface GateInput {
  panels: readonly string[];
  features: readonly string[];
  /** Whether this viewer holds a cockpit seat (16b cut B.3, `T-63`, 16c
   * Q21): `true` from a match's first seated frame until the cleared one, and
   * kept across a full rehydrate (`CockpitStore.seatedOnce`). The cockpit's
   * tab is gated on it, as the TUI's is on `cockpit_seat`. */
  cockpitSeated: boolean;
}

export interface ScreenDef {
  id: string;
  /** The hotkey digit, shared with the TUI where the screen exists there. */
  digit: number;
  label: string;
  keys: KeyBinding[];
  /** Whether the room's one-letter exits are keys here (the scene screen). */
  exitsAreKeys: boolean;
  /** Whether the server's manifest allows the screen. */
  gate: (input: GateInput) => boolean;
  /** A `.view` feed this screen renders generically (interface plan I9);
   * `undefined` for a built-in or bespoke screen. */
  viewFeed?: string;
}

/**
 * Verbs a bare key may send (D-I-18): every one reads state only.
 *
 * Four are bound today — `look`, `inventory`, `+roll/last`, `+contacts` —
 * and the rest are reserved for the screens a ruleset declares. Look-back
 * LB-16 took two off it: `@mail <n>` MARKS THE MESSAGE READ (`mush-mail`)
 * and `+orders` writes work orders, so neither belonged on a list whose
 * whole promise is that a bare key mutates nothing. `@mail` was unreachable
 * from the TUI anyway — `screen.rs` refuses any `@` Run outright — which is
 * exactly how a stale entry survives unnoticed on a hand-kept list.
 *
 * This is one of two hand-kept copies (`mush-net-ssh/src/tui/screen.rs` is
 * the other); both are pinned to the committed fixture `run-allowlist.json`
 * beside this file — by a vitest here and a Rust test there — until K8.5's
 * read-only classification can replace the hand-keeping.
 *
 * 16b cut Q.4 added the cockpit's seven, which are the seven of the twelve
 * cockpit verbs that only read: `VIEW`, `CONTACTS`, `STATUS`, `WEAPONSTATUS`,
 * `SCAN`, `RANGE` and `BEARING` all answer off the last frame the arena
 * published and stage no intent. The other five — `STARTUP`, `HEADING`,
 * `SPEED`, `LOCK`, `FIRE` — each stage one, so none of them is here and none
 * of them is a bare key: the cockpit screen only ever PREFILLS them.
 */
export const RUN_ALLOWLIST: readonly string[] = [
  "look",
  "l",
  "who",
  "inventory",
  "i",
  "+sheet",
  "+contacts",
  "+intel",
  "+actions",
  "+scene",
  "+recap",
  "+roll/last",
  "help",
  "view",
  "contacts",
  "status",
  "weaponstatus",
  "scan",
  "range",
  "bearing",
];

const hasPanel = (g: GateInput, p: string): boolean => g.panels.includes(p);
const hasFeature = (g: GateInput, f: string): boolean => g.features.includes(f);
const hasSuffix = (g: GateInput, suffix: string): boolean =>
  g.panels.some((p) => p.endsWith(suffix));

export const BUILTIN_SCREENS: readonly ScreenDef[] = [
  {
    id: "scene",
    digit: 1,
    label: "scene",
    keys: [
      { key: "l", label: "look", action: { kind: "run", cmd: "look" } },
      { key: "i", label: "inventory", action: { kind: "run", cmd: "inventory" } },
      { key: "p", label: "page", action: { kind: "prefill", line: "page " } },
      { key: "r", label: "roll", action: { kind: "prefill", line: "+roll " } },
    ],
    exitsAreKeys: true,
    gate: () => true,
  },
  {
    id: "sheet",
    digit: 2,
    label: "sheet",
    keys: [{ key: "r", label: "refresh", action: { kind: "local", action: "refresh" } }],
    exitsAreKeys: false,
    gate: (g) => hasSuffix(g, ".sheet"),
  },
  {
    id: "mail",
    digit: 3,
    label: "mail",
    keys: [
      { key: "r", label: "refresh", action: { kind: "local", action: "refresh" } },
      { key: "s", label: "send", action: { kind: "prefill", line: "@mail/send " } },
    ],
    exitsAreKeys: false,
    gate: (g) => hasFeature(g, "mail") || hasPanel(g, "mail"),
  },
  {
    id: "channels",
    digit: 4,
    label: "chan",
    keys: [{ key: "r", label: "refresh", action: { kind: "local", action: "refresh" } }],
    exitsAreKeys: false,
    gate: (g) => hasPanel(g, "comsys") || hasFeature(g, "channels"),
  },
  {
    id: "rolls",
    digit: 5,
    label: "rolls",
    keys: [
      { key: "r", label: "roll", action: { kind: "prefill", line: "+roll " } },
      { key: "h", label: "history", action: { kind: "run", cmd: "+roll/last" } },
      { key: "v", label: "refresh vitals", action: { kind: "local", action: "refresh" } },
    ],
    exitsAreKeys: false,
    gate: (g) => hasSuffix(g, ".roll"),
  },
  {
    id: "actions",
    digit: 7,
    label: "actions",
    keys: [{ key: "r", label: "refresh", action: { kind: "local", action: "refresh" } }],
    exitsAreKeys: false,
    gate: (g) => hasPanel(g, "legal.actions"),
  },
  {
    id: "build",
    digit: 8,
    label: "build",
    keys: [],
    exitsAreKeys: false,
    gate: (g) => hasFeature(g, "worldgen") || hasPanel(g, "worldgen.draft"),
  },
  {
    id: "llm",
    digit: 9,
    label: "llm",
    keys: [{ key: "r", label: "refresh", action: { kind: "local", action: "refresh" } }],
    exitsAreKeys: false,
    gate: (g) => hasPanel(g, "llm.dashboard"),
  },
  // The cockpit (16b cut Q.5; doc 16 §5), LAST in the table and last in the
  // strip, on the tenth digit the TUI gave it. **16b cut B.3** (`T-63`, 16c
  // Q21): gated on the feed AND a seat, as the TUI's tab is — it appears on a
  // match's first seated frame and goes on the cleared one, so a bystander on
  // an arena server has no tab at all rather than one that says a match
  // exists (doc 16 §1), and a pilot who leaves the tank loses it (`activeDef`
  // then falls back to the scene).
  //
  // None of the five ORDERING verbs is here: `STARTUP`, `HEADING`, `SPEED`,
  // `LOCK` and `FIRE` each stage an intent, so each is a prefill and none of
  // them is on `RUN_ALLOWLIST` — a bare key on this screen never fires a gun.
  {
    id: "cockpit",
    digit: 0,
    label: "cockpit",
    keys: [
      { key: "k", label: "lock", action: { kind: "prefill", line: "lock " } },
      { key: "f", label: "fire", action: { kind: "prefill", line: "fire " } },
      { key: "h", label: "heading", action: { kind: "prefill", line: "heading " } },
      { key: "m", label: "speed", action: { kind: "prefill", line: "speed " } },
      // `p`, pull (16b cut B.2, `B-59`): the frame asked for again, and the
      // map too when it has gone dark (`GameApp.localAction`'s cockpit arm).
      // Not `g`, which is the pane-jump key (`JUMP_KEYS`); right after `m`,
      // where the TUI puts it too.
      { key: "p", label: "pull", action: { kind: "local", action: "refresh" } },
      { key: "s", label: "scan a contact", action: { kind: "prefill", line: "scan " } },
      { key: "v", label: "view", action: { kind: "run", cmd: "view" } },
      { key: "c", label: "contacts", action: { kind: "run", cmd: "contacts" } },
      { key: "t", label: "status", action: { kind: "run", cmd: "status" } },
      { key: "w", label: "weapons", action: { kind: "run", cmd: "weaponstatus" } },
      { key: "r", label: "range", action: { kind: "run", cmd: "range" } },
      { key: "b", label: "bearing", action: { kind: "run", cmd: "bearing" } },
      { key: "n", label: "scan the lock", action: { kind: "run", cmd: "scan" } },
    ],
    exitsAreKeys: false,
    gate: (g) => hasPanel(g, COCKPIT_FEED) && g.cockpitSeated,
  },
];

/** Where a screen sits in the strip: `0` reads LAST, as it does on a keyboard
 * and in the TUI's tab bar. */
function stripOrder(def: ScreenDef): number {
  return def.digit === 0 ? 10 : def.digit;
}

/** A ruleset screen: present only when a manifest feed ends with `suffix`. */
export interface FeedScreen {
  suffix: string;
  def: ScreenDef;
}

/**
 * The tactical unit screen (interface cut I8, D-I8-4): ONE screen on the
 * one free web digit, hosting both layouts — `t` flips damage/unit, where
 * the TUI has room for two screens (8 and 9). Registered by `GameApp`
 * exactly once, on a hello whose manifest carries a `.unit` feed.
 *
 * `@unit/damage` is staff-gated and mutating, so it is a Prefill and never
 * a Run — `validateScreens` would refuse the whole screen otherwise
 * (D-I-18). `+contacts` is already on `RUN_ALLOWLIST`.
 *
 * The cost of taking digit 6 here is recorded: on a server that publishes
 * both a `.unit` feed and a `.view` feed, `registerViewScreen` will be
 * refused with "no free hotkey digit". It is deterministic rather than
 * racy — this screen registers synchronously in the hello pass, before any
 * `.view` keyframe can arrive — and the follow-up is a digitless-screen
 * policy, not a re-shuffle of the built-in digits.
 */
export const UNIT_SCREEN: FeedScreen = {
  suffix: ".unit",
  def: {
    id: "unit",
    digit: 6,
    label: "unit",
    keys: [
      { key: "r", label: "refresh", action: { kind: "local", action: "refresh" } },
      { key: "t", label: "damage / unit", action: { kind: "local", action: "toggle" } },
      { key: "c", label: "contacts", action: { kind: "run", cmd: "+contacts" } },
      { key: "d", label: "damage cmd", action: { kind: "prefill", line: "@unit/damage " } },
    ],
    exitsAreKeys: false,
    // A FeedScreen's own gate is never consulted (`available` filters by
    // suffix alone); the manifest feed IS the gate.
    gate: () => true,
  },
};

/**
 * The binding-table probe (D-I-18, D-I-19): every `run` verb is on the
 * read-only list, no screen rebinds a global or jump key, the screen that
 * gives exits keys never binds a reserved exit letter, digits and ids are
 * unique. Returns the first violation, or `null`.
 */
export function validateScreens(defs: readonly ScreenDef[]): string | null {
  const ids = new Set<string>();
  const digits = new Set<number>();
  for (const s of defs) {
    if (ids.has(s.id)) {
      return `screen ${s.id} declared twice`;
    }
    ids.add(s.id);
    // 16b cut Q.5 opened the range to the tenth digit the cockpit takes; a
    // ruleset screen still gets one of 1-9, because `freeDigit` hands out
    // nothing else.
    if (!Number.isInteger(s.digit) || s.digit < 0 || s.digit > 9) {
      return `screen ${s.id}: digit ${s.digit} is not 0-9`;
    }
    if (digits.has(s.digit)) {
      return `screen ${s.id}: digit ${s.digit} used twice`;
    }
    digits.add(s.digit);
    const keys = new Set<string>();
    for (const b of s.keys) {
      if (GLOBAL_KEYS.has(b.key) || JUMP_KEYS.has(b.key)) {
        return `screen ${s.id}: ${b.key} rebinds a global key`;
      }
      if (s.exitsAreKeys && RESERVED_EXIT_KEYS.includes(b.key)) {
        return `screen ${s.id}: ${b.key} is reserved for exits`;
      }
      if (keys.has(b.key)) {
        return `screen ${s.id}: ${b.key} bound twice`;
      }
      keys.add(b.key);
      if (b.action.kind === "run") {
        const verb = b.action.cmd.split(/\s+/)[0] ?? "";
        if (!RUN_ALLOWLIST.some((a) => a.toLowerCase() === verb.toLowerCase())) {
          return `screen ${s.id}: ${b.key} runs ${b.action.cmd}, not a read-only verb`;
        }
      }
    }
  }
  return null;
}

export class ScreensStore {
  active = $state("scene");
  feedScreens = $state<FeedScreen[]>([]);
  /** The room's one-letter exit keys (the scene screen). */
  exits = $state<Map<string, string>>(new Map());
  /** Input mode: the composer (or another field) has focus (D-I-17). */
  inputFocused = $state(false);
  paletteOpen = $state(false);
  helpOpen = $state(false);

  /** The lowest hotkey digit 1-9 not used by a built-in or a registered
   * feed screen; `null` when all nine are taken (interface plan I9). */
  freeDigit(): number | null {
    const used = new Set<number>([
      ...BUILTIN_SCREENS.map((s) => s.digit),
      ...this.feedScreens.map((f) => f.def.digit),
    ]);
    for (let d = 1; d <= 9; d++) {
      if (!used.has(d)) {
        return d;
      }
    }
    return null;
  }

  /** Register a generic-view screen for a `.view` feed (interface plan I9),
   * idempotently, taking the next free digit. Returns `null` on success, a
   * reason when refused (already present, no free digit, or the probe). No
   * bespoke code: the schema in the feed drives the render. */
  registerViewScreen(feed: string): string | null {
    if (this.feedScreens.some((f) => f.def.viewFeed === feed)) {
      return null;
    }
    const digit = this.freeDigit();
    if (digit === null) {
      return "no free hotkey digit for a view screen";
    }
    const label = feed.replace(/\.view$/, "").slice(0, 10);
    const def: ScreenDef = {
      id: `view:${feed}`,
      digit,
      label,
      keys: [{ key: "r", label: "refresh", action: { kind: "local", action: "refresh" } }],
      exitsAreKeys: false,
      gate: (g) => g.panels.includes(feed),
      viewFeed: feed,
    };
    return this.registerFeedScreen({ suffix: feed, def });
  }

  /** Register a ruleset screen (I6, I8); refused when it fails the probe. */
  registerFeedScreen(fs: FeedScreen): string | null {
    const all = [...BUILTIN_SCREENS, ...this.feedScreens.map((f) => f.def), fs.def];
    const err = validateScreens(all);
    if (err !== null) {
      return err;
    }
    this.feedScreens = [...this.feedScreens, fs];
    return null;
  }

  /** The screens the manifest allows, in digit order. */
  available(gate: GateInput): ScreenDef[] {
    const feedDefs = this.feedScreens
      .filter((f) => gate.panels.some((p) => p.endsWith(f.suffix)))
      .map((f) => f.def);
    return [...BUILTIN_SCREENS.filter((s) => s.gate(gate)), ...feedDefs].sort(
      (a, b) => stripOrder(a) - stripOrder(b),
    );
  }

  /** The active screen's definition (the scene screen when the active one
   * is no longer available). */
  activeDef(gate: GateInput): ScreenDef {
    const avail = this.available(gate);
    return avail.find((s) => s.id === this.active) ?? avail[0] ?? BUILTIN_SCREENS[0]!;
  }

  /** Switch by id; `false` when the screen is not available. */
  switchTo(id: string, gate: GateInput): boolean {
    if (!this.available(gate).some((s) => s.id === id)) {
      return false;
    }
    this.active = id;
    this.helpOpen = false;
    return true;
  }

  /** Switch by hotkey digit; `false` when no available screen has it. */
  switchDigit(digit: number, gate: GateInput): boolean {
    const def = this.available(gate).find((s) => s.digit === digit);
    return def ? this.switchTo(def.id, gate) : false;
  }

  /** The active screen's binding keys, for the key model. */
  bindingKeys(gate: GateInput): Set<string> {
    return new Set(this.activeDef(gate).keys.map((b) => b.key));
  }

  /** The active screen's binding for `key`. */
  binding(key: string, gate: GateInput): KeyBinding | null {
    return this.activeDef(gate).keys.find((b) => b.key === key) ?? null;
  }

  /** The exit keys the key model should honour on the active screen. */
  exitKeysFor(gate: GateInput): ReadonlyMap<string, string> {
    return this.activeDef(gate).exitsAreKeys ? this.exits : new Map();
  }

  clear(): void {
    this.active = "scene";
    this.exits = new Map();
    this.paletteOpen = false;
    this.helpOpen = false;
  }

  /**
   * Drop every registered feed screen (LB-2) — a SEPARATE seam from
   * `clear()`, and only the full game-state reset calls it.
   *
   * `clear()` deliberately keeps them: a second hello on the same character
   * re-hydrates the same manifest, and `registerFeedScreen` has no
   * idempotency of its own, so a screen dropped there would be refused with
   * "screen unit declared twice" on every re-hello.
   *
   * A character switch is the opposite case. The screens belong to the OLD
   * server's manifest: a `foo.view` screen that took digit 6 on server A
   * survives into a tactics server, where `registerFeedScreen(UNIT_SCREEN)`
   * is then refused with "digit 6 used twice" and the unit screen simply
   * never appears.
   */
  clearFeedScreens(): void {
    this.feedScreens = [];
  }
}
