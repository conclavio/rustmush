// The web helm against the arena as it meets it (doc 21 cut G.4b): the same
// cases, the same model arena and the same figures as the tests in
// `mush-net-ssh/src/tui/helm.rs`, so the two clients are pinned to turn alike.

import { describe, expect, it } from "vitest";
import {
  DEFAULT_TURN_RATE_MDEG_S,
  Helm,
  type HelmFrame,
  ORDER_GAP_MS,
  helmFrame,
} from "./helm";
import { FULL_MDEG, arcMdeg } from "./picture";

/** The helm's clock at the start of a case: any instant will do. */
const T0 = 1_000_000;

function frame(tMs: number | null, headingDeg: number, tempo: string, more: Partial<HelmFrame> = {}): HelmFrame {
  return { tMs, headingMdeg: headingDeg * 1_000, tempo, turnRateMdegS: null, canTurn: true, ...more };
}

const PULSE_MS: Record<string, number> = { quiet: 5_000, alert: 1_000, engaged: 500 };

/**
 * The arena as the helm meets it: the standing order is resolved at the pulse
 * with the whole pulse of turning, whenever in the pulse it came, on the short
 * arc; and a frame goes out when the heading moved. Its frames carry no turn
 * rate, as a replay's do not.
 */
class Arena {
  heading: number;
  order: number | null = null;
  tMs = 1_000;

  constructor(
    headingDeg: number,
    readonly rate: number,
    readonly tempo: string,
  ) {
    this.heading = headingDeg * 1_000;
  }

  get pulseMs(): number {
    return PULSE_MS[this.tempo] ?? 1_000;
  }

  frame(): HelmFrame {
    return { tMs: this.tMs, headingMdeg: this.heading, tempo: this.tempo, turnRateMdegS: null, canTurn: true };
  }

  /** One pulse: the frame it sends, if the heading moved. */
  pulse(): HelmFrame | null {
    const pulse = this.pulseMs;
    this.tMs += pulse;
    const before = this.heading;
    if (this.order !== null) {
      const d = arcMdeg(this.heading, this.order);
      const step = Math.min(Math.abs(d), Math.trunc((this.rate * pulse) / 1_000));
      this.heading = (((this.heading + Math.sign(d) * step) % FULL_MDEG) + FULL_MDEG) % FULL_MDEG;
    }
    return this.heading !== before ? this.frame() : null;
  }

  /** Take the helm's lines as the arena's command queue does. */
  take(lines: string[]): void {
    for (const line of lines) {
      const m = /^HEADING (\d+) QUIET$/.exec(line);
      expect(m, `not a quiet heading order: ${line}`).not.toBeNull();
      this.order = Number(m?.[1]) * 1_000;
    }
  }
}

/**
 * Paint every 50 ms for `total` ms against the arena, the turn key down at
 * `press` and up at `release`: the heading the picture showed at each paint,
 * the first at 0 ms.
 */
function drive(arena: Arena, h: Helm, press: number, release: number, total: number): number[] {
  const shown: number[] = [];
  for (let at = 0; at <= total; at += 50) {
    const now = T0 + at;
    if (at > 0 && at % arena.pulseMs === 0) {
      const f = arena.pulse();
      if (f !== null) h.observe(f, now);
    }
    if (at === press) h.start(1, now);
    if (at === release) h.stop(now);
    h.tick(now);
    arena.take(h.takeOrders());
    shown.push(h.predicted(now) ?? -1);
  }
  return shown;
}

/** The step of each paint after the first. */
function steps(shown: number[]): number[] {
  return shown.slice(1).map((v, i) => arcMdeg(shown[i] ?? -1, v));
}

describe("the helm", () => {
  it("orders nothing without a frame: there is no heading to turn from", () => {
    const h = new Helm();
    h.start(1, T0);
    h.hold(T0);
    expect(h.takeOrders()).toEqual([]);
    expect(h.predicted(T0)).toBeNull();
    expect(h.busy).toBe(false);
  });

  it("swings from the press, and a release orders the heading shown", () => {
    const h = new Helm();
    h.observe(frame(10_000, 90, "alert"), T0);
    expect(h.predicted(T0)).toBe(90_000);
    h.start(1, T0 + 200);
    // The press orders the tap's degree, not a pulse and a half of turn.
    expect(h.takeOrders()).toEqual(["HEADING 91 QUIET"]);
    expect(h.predicted(T0 + 200), "moves from the press").toBe(90_000);
    expect(h.predicted(T0 + 400), "the slow start").toBe(91_000);
    expect(h.predicted(T0 + 700), "then 30°/s").toBe(100_000);
    // The first order of the hold: where the picture will be a gap on.
    h.tick(T0 + 450);
    expect(h.takeOrders()).toEqual(["HEADING 100 QUIET"]);
    h.stop(T0 + 800);
    expect(h.takeOrders()).toEqual(["HEADING 103 QUIET"]);
    expect(h.turning).toBe(0);
    // A frame short of the order does not pull the picture back to it…
    h.observe(frame(11_000, 100, "alert"), T0 + 1_000);
    expect(h.predicted(T0 + 1_000)).toBe(103_000);
    expect(h.predicted(T0 + 1_900)).toBe(103_000);
    expect(h.ordered).toBe(103_000);
    expect(h.busy).toBe(true);
    // …and the frame at the ordered heading closes the order.
    h.observe(frame(12_000, 103, "alert"), T0 + 2_000);
    expect(h.ordered).toBeNull();
    expect(h.busy).toBe(false);
    expect(h.predicted(T0 + 5_000)).toBe(103_000);
  });

  it("turns the tank one degree a tap, however short the press", () => {
    let h = new Helm();
    h.observe(frame(1_000, 87, "alert"), T0);
    h.start(1, T0 + 1_000);
    h.stop(T0 + 1_030);
    expect(h.takeOrders(), "one line a tap").toEqual(["HEADING 88 QUIET"]);
    expect(h.predicted(T0 + 1_200)).toBe(88_000);
    // A slower tap is the same degree, and two taps are two degrees.
    h.start(1, T0 + 2_000);
    h.stop(T0 + 2_180);
    expect(h.takeOrders()).toEqual(["HEADING 89 QUIET"]);
    expect(h.predicted(T0 + 2_400)).toBe(89_000);
    // Left, across north.
    h = new Helm();
    h.observe(frame(1_000, 0, "quiet"), T0);
    h.start(-1, T0);
    h.stop(T0 + 90);
    expect(h.takeOrders()).toEqual(["HEADING 359 QUIET"]);
    expect(h.predicted(T0 + 400)).toBe(359_000);
  });

  it("orders, while held, where the picture will be a gap on", () => {
    const h = new Helm();
    h.observe(frame(1_000, 90, "alert"), T0);
    h.start(1, T0);
    expect(h.takeOrders()).toEqual(["HEADING 91 QUIET"]);
    let sent = 0;
    for (let at = 50; at <= 1_000; at += 50) {
      const now = T0 + at;
      h.tick(now);
      const orders = h.takeOrders();
      if (at % ORDER_GAP_MS === 0) {
        const ahead = h.predicted(now + ORDER_GAP_MS) ?? -1;
        const deg = Math.trunc((ahead + 500) / 1_000);
        expect(orders, `at ${at} ms`).toEqual([`HEADING ${deg} QUIET`]);
        expect(arcMdeg(h.predicted(now) ?? -1, deg * 1_000)).toBeLessThanOrEqual(8_000);
        sent += 1;
      } else {
        expect(orders, `the gap holds at ${at} ms`).toEqual([]);
      }
    }
    expect(sent).toBe(4);
  });

  // The owner's report (2026-11-09): the picture "jerks and moves in wide
  // swaths". Against an arena that resolves orders a pulse at a time, at every
  // tempo and wherever in the pulse the key goes down, a three-second hold is
  // one swing: never back, never faster than the turn rate, still from the
  // release on, and the tank comes to the heading the picture stopped at.
  it.each([
    ["engaged", 150],
    ["alert", 700],
    ["alert", 950],
    ["quiet", 700],
    ["quiet", 4_800],
  ] as const)("is one smooth swing at %s, the key down at %i ms", (tempo, press) => {
    const release = press + 3_000;
    const arena = new Arena(90, 30_000, tempo);
    const h = new Helm();
    h.observe(arena.frame(), T0);
    const shown = drive(arena, h, press, release, release + 12_000);
    // One degree in the slow start, then 2.8 s at 30°/s.
    const target = 175_000;
    steps(shown).forEach((step, i) => {
      const at = (i + 1) * 50;
      expect(step, `${step} mdeg in the paint at ${at} ms`).toBeGreaterThanOrEqual(0);
      expect(step, `${step} mdeg in the paint at ${at} ms`).toBeLessThanOrEqual(1_500);
      if (at <= press) expect(shown[i + 1], "before the press").toBe(90_000);
      if (at >= release) expect(shown[i + 1], `still at ${at} ms`).toBe(target);
    });
    expect(arena.heading, "the tank came to it").toBe(target);
    expect(h.ordered).toBeNull();
    expect(h.rate, "no measure").toBe(DEFAULT_TURN_RATE_MDEG_S);
  });

  it("swings at the frame's turn rate, and measures nothing over it", () => {
    const h = new Helm();
    h.observe(frame(1_000, 0, "alert", { turnRateMdegS: 60_000 }), T0);
    expect(h.rate).toBe(60_000);
    h.start(1, T0);
    // One degree in the slow start, then half a second at 60°/s.
    expect(h.predicted(T0 + 700)).toBe(31_000);
    for (let at = 50; at <= 950; at += 50) h.tick(T0 + at);
    // A whole pulse of turning that left the tank ten degrees on and short of
    // its order: a measure, were the rate not the frame's.
    h.observe(frame(2_000, 10, "alert", { turnRateMdegS: 60_000 }), T0 + 1_000);
    expect(h.turning).toBe(1);
    expect(h.rate).toBe(60_000);
  });

  it("measures a slower hull when the frame carries no rate", () => {
    const arena = new Arena(90, 20_000, "alert");
    const h = new Helm();
    h.observe(arena.frame(), T0);
    const shown = drive(arena, h, 100, 2_500, 8_000);
    expect(h.rate).toBe(20_000);
    for (const step of steps(shown)) {
      expect(step).toBeGreaterThanOrEqual(0);
      expect(step).toBeLessThanOrEqual(1_500);
    }
    expect(arena.heading).toBe(152_000);
    expect(shown.at(-1)).toBe(arena.heading);
  });

  it("drops a turn the frames do not bear out, and eases back to them", () => {
    // Frames come, and the tank is no nearer its order a whole pulse on.
    let h = new Helm();
    h.observe(frame(1_000, 90, "alert"), T0);
    h.start(1, T0 + 100);
    for (let at = 150; at <= 2_000; at += 50) {
      const now = T0 + at;
      if (at === 1_000) {
        h.observe(frame(2_000, 90, "alert"), now);
        expect(h.turning, "the press came inside that pulse").toBe(1);
      }
      if (at === 2_000) h.observe(frame(3_000, 90, "alert"), now);
      h.tick(now);
    }
    expect(h.turning).toBe(0);
    expect(h.ordered).toBeNull();
    h.takeOrders();
    // The picture had run to the leash, a pulse and a half of turning, and
    // goes back to the frame inside a pulse: no snap.
    expect(h.predicted(T0 + 2_000)).toBe(135_000);
    expect(h.predicted(T0 + 2_500)).toBe(112_500);
    expect(h.predicted(T0 + 3_000)).toBe(90_000);
    h.tick(T0 + 3_000);
    expect(h.takeOrders(), "a dropped turn orders nothing").toEqual([]);

    // No frame at all: dropped a second past the pulse.
    h = new Helm();
    h.observe(frame(1_000, 90, "alert"), T0);
    h.start(1, T0 + 100);
    for (let at = 150; at <= 2_050; at += 50) h.tick(T0 + at);
    expect(h.turning).toBe(1);
    h.tick(T0 + 2_150);
    expect(h.turning).toBe(0);
    expect(h.ordered).toBeNull();
    expect(h.busy).toBe(false);
    expect(h.predicted(T0 + 2_150)).toBe(135_000);
    expect(h.predicted(T0 + 3_150)).toBe(90_000);
  });

  it("does not draw a hull turning that the frame says cannot turn", () => {
    const h = new Helm();
    h.observe(frame(1_000, 90, "alert", { canTurn: false }), T0);
    h.start(1, T0);
    // The order still goes: the arena's refusal is what tells the pilot why.
    expect(h.takeOrders()).toEqual(["HEADING 91 QUIET"]);
    expect(h.predicted(T0 + 1_000)).toBe(90_000);
    h.stop(T0 + 1_000);
    expect(h.predicted(T0 + 1_500)).toBe(90_000);
  });

  it("follows a turn that is not its own, and never steps to it", () => {
    const h = new Helm();
    h.observe(frame(1_000, 90, "alert"), T0);
    h.observe(frame(2_000, 120, "alert"), T0 + 1_000);
    expect(h.predicted(T0 + 1_000), "no step").toBe(90_000);
    expect(h.predicted(T0 + 1_500)).toBe(105_000);
    expect(h.predicted(T0 + 2_000)).toBe(120_000);
    expect(h.predicted(T0 + 9_000)).toBe(120_000);
    h.tick(T0 + 2_000);
    expect(h.takeOrders()).toEqual([]);
    expect(h.busy).toBe(false);
    // A hull faster than the helm knows is followed inside a pulse.
    h.observe(frame(3_000, 210, "alert"), T0 + 2_000);
    expect(h.predicted(T0 + 2_500)).toBe(165_000);
    expect(h.predicted(T0 + 3_000)).toBe(210_000);
    // The arena's clock going back is another picture: shown as it is.
    h.observe(frame(500, 10, "alert"), T0 + 3_100);
    expect(h.predicted(T0 + 3_100)).toBe(10_000);
  });

  it("reads a frame once, by its arena time", () => {
    const h = new Helm();
    h.observe(frame(1_000, 90, "alert"), T0);
    h.observe(frame(2_000, 90, "alert"), T0 + 1_000);
    expect(h.live).toBe(true);
    expect(h.frameAt).toBe(T0 + 1_000);
    // The same keyframe held across a redraw is not a new frame.
    h.observe(frame(2_000, 90, "alert"), T0 + 1_400);
    expect(h.frameAt).toBe(T0 + 1_000);
  });

  // The web's own: a feed without an arena clock (a fixture, a header alone)
  // is told apart by what it says, carried like any other, and never judged.
  it("follows a feed without an arena clock by its heading", () => {
    const h = new Helm();
    h.observe(frame(null, 45, ""), T0);
    h.observe(frame(null, 45, ""), T0 + 500);
    expect(h.frameAt).toBe(T0);
    h.observe(frame(null, 60, ""), T0 + 1_000);
    expect(h.live).toBe(false);
    expect(h.predicted(T0 + 1_000), "no step").toBe(45_000);
    expect(h.predicted(T0 + 1_500)).toBe(60_000);
  });

  it("steers to a heading the pilot picked, from where the picture stands", () => {
    const h = new Helm();
    h.observe(frame(1_000, 90, "alert"), T0);
    h.steer(120_400, T0 + 100);
    expect(h.takeOrders()).toEqual(["HEADING 120 QUIET"]);
    expect(h.ordered).toBe(120_000);
    expect(h.predicted(T0 + 100), "no step").toBe(90_000);
    expect(h.predicted(T0 + 600)).toBe(105_000);
    expect(h.predicted(T0 + 2_000)).toBe(120_000);
    // The same heading again is still sent: a click is the pilot's word.
    h.steer(120_000, T0 + 300);
    expect(h.takeOrders()).toEqual(["HEADING 120 QUIET"]);
    // A click during a held turn ends the turn; the key's release adds nothing.
    h.start(-1, T0 + 400);
    h.steer(60_000, T0 + 500);
    expect(h.turning).toBe(0);
    h.stop(T0 + 600);
    expect(h.takeOrders()).toEqual(["HEADING 98 QUIET", "HEADING 60 QUIET"]);
  });

  it("holds the heading shown on all stop, ordering a turn in hand away", () => {
    const h = new Helm();
    h.observe(frame(1_000, 45, "alert"), T0);
    h.hold(T0 + 100);
    expect(h.takeOrders()).toEqual(["HEADING 45 QUIET"]);
    h.steer(100_000, T0 + 200);
    h.hold(T0 + 1_200);
    // A second at 30°/s toward 100 from 45.
    expect(h.takeOrders()).toEqual(["HEADING 100 QUIET", "HEADING 75 QUIET"]);
    expect(h.predicted(T0 + 5_000)).toBe(75_000);
    // A released order no frame speaks to is dropped a second past its pulse.
    h.tick(T0 + 3_300);
    expect(h.busy).toBe(false);
  });
});

describe("the helm's reading of the cockpit", () => {
  const decoded = { t_ms: 4_000, tempo: "engaged", power: "online", wreck: false, immobile: false };

  it("takes the millidegree heading and the turn rate from the keyframe", () => {
    expect(helmFrame(decoded, { heading_mdeg: 90_400, turn_rate_mdeg_s: 45_000 }, 90)).toEqual({
      tMs: 4_000,
      headingMdeg: 90_400,
      tempo: "engaged",
      turnRateMdegS: 45_000,
      canTurn: true,
    });
  });

  it("falls back to the header's whole degrees and no rate", () => {
    expect(helmFrame(decoded, null, 359)).toMatchObject({ headingMdeg: 359_000, turnRateMdegS: null });
    expect(helmFrame(null, null, 45)).toEqual({
      tMs: null,
      headingMdeg: 45_000,
      tempo: "",
      turnRateMdegS: null,
      canTurn: true,
    });
  });

  it.each([
    [{ power: "off" }, false],
    [{ power: "starting" }, false],
    [{ immobile: true }, false],
    [{ wreck: true }, false],
    [{ power: "?" }, true],
  ])("reads whether the hull can turn: %o", (more, canTurn) => {
    expect(helmFrame({ ...decoded, ...more }, null, 0).canTurn).toBe(canTurn);
  });
});
