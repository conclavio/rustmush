// The tank's helm as the web picture drives it (doc 21 cut G.4b): the turn
// keys steer the tank, and the picture turns as the tank will. This is
// `mush-net-ssh/src/tui/helm.rs`'s `Helm` in the browser, rule for rule and
// figure for figure, so the two clients turn alike and vitest pins this one
// against the same model arena.
//
// A turn is a `HEADING <deg> QUIET` line, the verb and the authority a pilot
// types. The arena resolves orders at its pulses (500, 1000 or 5000 ms by
// tempo), and an order that arrives late in a pulse is still given the whole
// pulse of turning: a frame's heading moves in steps of up to a pulse's turn.
// The picture must not step with it. So the helm keeps one continuous shown
// heading (`predicted`), carried on from where it stands at every event and
// never restarted from a frame:
//
// - held, it swings from where it stood at the press: a slow first `TAP_MS`
//   so a tap is one degree, then the unit's turn rate. The press orders that
//   one degree, and every `ORDER_GAP_MS` after it the helm orders the heading
//   the picture will show one gap on, so at each pulse the arena lands on the
//   picture or a few degrees ahead of it, and the frames of a turn in hand
//   are not drawn;
// - released, the helm orders the heading the picture shows and the picture
//   holds it;
// - steered (a click on the heading tape or the map), the picture closes on
//   the heading clicked at the turn rate;
// - otherwise (a typed order, a machine at the helm) it follows the frames at
//   the turn rate, at most a pulse behind, in place of stepping.
//
// The leash: the picture never runs further ahead of the last frame than a
// pulse and a half of turning, a hull the frame says cannot turn is not drawn
// turning, and a turn the frames do not bear out is dropped a second past its
// pulse, the picture easing back to the frames.
//
// Everything is whole millidegrees and whole milliseconds, divided as the
// TUI's integers are, so the same presses give the same lines on both.

import { arcMdeg, wrap360Mdeg } from "./picture";

/** The rate assumed until a frame carries one or a turn is measured: the
 * stock tank's, 30°/s. */
export const DEFAULT_TURN_RATE_MDEG_S = 30_000;
/** The turn rate is kept inside this band (millidegrees a second). */
const RATE_BAND: readonly [number, number] = [1_000, 360_000];
/** The time between two orders while a turn is held, milliseconds. */
export const ORDER_GAP_MS = 250;
/** How often the view runs the helm's clock (`tick`) while a turn or an
 * order stands, milliseconds: the picture's own frame time. */
export const HELM_TICK_MS = 50;
/** A held turn's order runs ahead of the picture by what the picture will
 * swing in one `ORDER_GAP_MS`, kept inside this band (millidegrees). */
const LEAD_BAND: readonly [number, number] = [1_000, 25_000];
/** The furthest the picture runs ahead of the last frame (millidegrees): a
 * pulse and a half of turning inside this band. */
const LEASH_BAND: readonly [number, number] = [20_000, 150_000];
/** A frame within this of the order has reached it (millidegrees). */
const REACHED = 1_000;
/** The least a press turns the tank, and what a tap turns it: one degree. */
const TAP = 1_000;
/** A press this short is a tap: the swing's slow start, milliseconds. */
export const TAP_MS = 200;
/** No prediction runs longer than this without an event, milliseconds. */
const RUN_CAP_MS = 600_000;

/** The pulse length a tempo word stands for, at the shipped
 * `[tacsim.arena.tempo]` defaults. */
const TEMPO_MS: Readonly<Record<string, number>> = { quiet: 5_000, alert: 1_000, engaged: 500 };

/** What the helm reads of a cockpit frame. */
export interface HelmFrame {
  /** The frame's arena time, or `null` on a feed without one. */
  tMs: number | null;
  headingMdeg: number;
  /** The arena's tempo word; anything else leaves the pulse as it was. */
  tempo: string;
  /** The unit's turn rate (`turn_rate_mdeg_s`), or `null` on a frame without
   * one (a replay, a fixture). */
  turnRateMdegS: number | null;
  /** Whether the hull can turn at all (powered, mobile, not a wreck): the
   * arena refuses a `HEADING` otherwise. */
  canTurn: boolean;
}

/** The decoded frame's fields the helm reads. */
interface Decoded {
  t_ms: number | null;
  tempo: string;
  power: string;
  wreck: boolean;
  immobile: boolean;
}

/**
 * The helm's reading of the cockpit the view holds: the decoded `frame`, the
 * keyframe as it arrived (`raw`, for the millidegree heading and the turn
 * rate the decoder does not carry), and the header's whole-degree heading for
 * a keyframe without one. With no frame the heading alone is the reading.
 */
export function helmFrame(frame: Decoded | null, raw: unknown, headingDeg: number): HelmFrame {
  const r = (raw ?? {}) as { heading_mdeg?: unknown; turn_rate_mdeg_s?: unknown };
  const headingMdeg = wrap360Mdeg(
    typeof r.heading_mdeg === "number" ? r.heading_mdeg : headingDeg * 1_000,
  );
  if (frame === null) {
    return { tMs: null, headingMdeg, tempo: "", turnRateMdegS: null, canTurn: true };
  }
  return {
    tMs: frame.t_ms,
    headingMdeg,
    tempo: frame.tempo,
    turnRateMdegS: typeof r.turn_rate_mdeg_s === "number" ? r.turn_rate_mdeg_s : null,
    canTurn: !frame.wreck && !frame.immobile && frame.power !== "off" && frame.power !== "starting",
  };
}

/** `a / b` toward zero, as the TUI's integers divide. */
function div(a: number, b: number): number {
  return Math.trunc(a / b);
}

function clamp(v: number, band: readonly [number, number]): number {
  return Math.min(Math.max(v, band[0]), band[1]);
}

/** Whole milliseconds from `from` to `to`, zero when `to` is the earlier. */
function msBetween(from: number, to: number): number {
  return Math.min(Math.max(Math.trunc(to - from), 0), RUN_CAP_MS);
}

/** The last frame the helm saw. */
interface Seen {
  tMs: number | null;
  heading: number;
  canTurn: boolean;
  /** When it arrived here. */
  at: number;
}

/** A heading at an instant. */
type At = readonly [mdeg: number, at: number];

/** The tank's helm as the picture drives it. Every `now` is the caller's
 * clock in milliseconds. */
export class Helm {
  /** `-1` turning left, `0` not, `1` turning right. */
  #turning: -1 | 0 | 1 = 0;
  /** The heading last ordered, and when it was first ordered. */
  #ordered: At | null = null;
  /** The order before the standing one, within the turn in hand: one the
   * arena has certainly had by the time a frame arrives. */
  #prior: number | null = null;
  #seen: Seen | null = null;
  /** The picture's heading at an instant. Every prediction runs on from
   * here, and every event (a frame, a press, a release, a dropped turn)
   * moves it to where the picture stood, so the picture never jumps. */
  #shown: At | null = null;
  /** The picture's heading when the turn in hand began, and when. */
  #turnFrom: At | null = null;
  /** True once consecutive arena pulses have been observed. */
  #live = false;
  #rate = DEFAULT_TURN_RATE_MDEG_S;
  /** True once a frame has carried the unit's turn rate: nothing is measured
   * after. */
  #rateRead = false;
  #tempoMs = 1_000;
  /** `HEADING` lines waiting to be sent. */
  #orders: string[] = [];

  /** Which way the pilot is turning the tank. */
  get turning(): -1 | 0 | 1 {
    return this.#turning;
  }

  /** The heading last ordered and not yet reached, millidegrees. While a
   * turn is held it is the running lead, a few degrees ahead of the picture,
   * and not a heading the pilot chose. */
  get ordered(): number | null {
    return this.#ordered?.[0] ?? null;
  }

  /** The turn rate in use, millidegrees a second. */
  get rate(): number {
    return this.#rate;
  }

  /** Whether consecutive arena pulses are arriving. */
  get live(): boolean {
    return this.#live;
  }

  /** When the last frame arrived, or `0` before any. */
  get frameAt(): number {
    return this.#seen?.at ?? 0;
  }

  /** Whether a turn or an order stands: the helm's clock (`tick`) is wanted. */
  get busy(): boolean {
    return this.#turning !== 0 || this.#ordered !== null;
  }

  /**
   * Read the held cockpit frame. A new one (by its arena time) is the truth
   * about the tank, and the picture carries on from where it stood: a turn in
   * hand keeps swinging, a released one keeps its heading, and anything else
   * is followed at the turn rate. The frame also gives the turn rate (or lets
   * it be measured), and a held turn it does not bear out is dropped.
   */
  observe(f: HelmFrame, now: number): void {
    const prev = this.#seen;
    const heading = wrap360Mdeg(f.headingMdeg);
    if (prev !== null) {
      // A feed without an arena clock is told apart by what it says.
      const same =
        f.tMs !== null
          ? f.tMs === prev.tMs
          : prev.tMs === null && heading === prev.heading && f.canTurn === prev.canTurn;
      if (same) return;
    }
    // Where the picture stands, by the old frame, rate and tempo: the new
    // ones change where it goes from here, never where it is.
    const stands = this.predicted(now);
    const tempo = TEMPO_MS[f.tempo];
    if (tempo !== undefined) this.#tempoMs = tempo;
    if (f.turnRateMdegS !== null && f.turnRateMdegS > 0) {
      this.#rate = clamp(Math.round(f.turnRateMdegS), RATE_BAND);
      this.#rateRead = true;
    }
    let carried: number | null = null;
    if (prev !== null) {
      if (f.tMs !== null && prev.tMs !== null) {
        const dt = f.tMs - prev.tMs;
        this.#live = dt > 0 && dt <= 10_000;
        if (dt > 0) {
          carried = stands;
          this.#judge(prev, heading, dt, now);
        }
      } else {
        this.#live = false;
        // Neither frame has a clock: carried, with nothing to judge by.
        if (f.tMs === null && prev.tMs === null) carried = stands;
      }
    }
    // The first frame, and the first after the arena's clock went back
    // (another match, a replay), is shown as it is.
    this.#shown = [carried ?? heading, now];
    this.#seen = { tMs: f.tMs, heading, canTurn: f.canTurn, at: now };
    if (
      this.#turning === 0 &&
      this.#ordered !== null &&
      Math.abs(arcMdeg(heading, this.#ordered[0])) <= REACHED
    ) {
      this.#ordered = null;
    }
  }

  /** What the frame after `prev` says of the helm's own order: `dt` arena
   * milliseconds on, the tank is at `heading`. */
  #judge(prev: Seen, heading: number, dt: number, now: number): void {
    if (this.#turning === 0) {
      // A released order the tank is no longer closing on, a second past its
      // pulse, was not taken.
      if (this.#ordered !== null) {
        const [o, at] = this.#ordered;
        if (
          now - at > this.#grace() &&
          Math.abs(arcMdeg(heading, o)) >= Math.abs(arcMdeg(prev.heading, o))
        ) {
          this.#ordered = null;
        }
      }
      return;
    }
    const dir = this.#turning;
    // Only a pulse the turn was in hand for from end to end, and that left
    // the tank short of an order the arena certainly had, says anything: the
    // arena spent the whole of it turning, or would not turn.
    const whole = this.#turnFrom !== null && this.#turnFrom[1] <= prev.at;
    const short = this.#prior !== null && dir * arcMdeg(heading, this.#prior) > REACHED;
    if (!(whole && short)) return;
    const turned = dir * arcMdeg(prev.heading, heading);
    if (turned <= 0) {
      // Ordered ahead for a whole pulse and no nearer: refused, or a hull
      // that cannot turn that way.
      this.#turning = 0;
      this.#turnFrom = null;
      this.#ordered = null;
      this.#prior = null;
    } else if (!this.#rateRead && dt <= this.#tempoMs + div(this.#tempoMs, 2)) {
      this.#rate = clamp(div(turned * 1_000, dt), RATE_BAND);
    }
  }

  /**
   * The heading the picture shows at `now`: continuous in time, and carried
   * across every frame, press and release from where it stood. A held turn
   * swings on at the turn rate, no further ahead of the last frame than the
   * leash; a released or steered one closes on the heading ordered and holds
   * it; with no order of its own the picture follows the last frame's
   * heading, at the turn rate or in a pulse, whichever is sooner. `null`
   * before any frame.
   */
  predicted(now: number): number | null {
    const seen = this.#seen;
    if (seen === null) return null;
    const [v0, t0] = this.#shown ?? [seen.heading, seen.at];
    const ms = msBetween(t0, now);
    let v: number;
    if (this.#turning !== 0 && this.#turnFrom !== null) {
      const dir = this.#turning;
      const from = this.#turnFrom[1];
      const swung = seen.canTurn
        ? this.#swing(msBetween(from, now)) - this.#swing(msBetween(from, t0))
        : 0;
      const room = Math.max(this.#leash() - dir * arcMdeg(seen.heading, v0), 0);
      v = v0 + dir * Math.min(swung, room);
    } else if (this.#ordered !== null) {
      const run = seen.canTurn ? div(this.#rate * ms, 1_000) : 0;
      const d = arcMdeg(v0, this.#ordered[0]);
      v = v0 + Math.sign(d) * Math.min(Math.abs(d), run);
    } else {
      const d = arcMdeg(v0, seen.heading);
      const inAPulse = div(Math.abs(d) * ms, Math.max(this.#tempoMs, 1));
      v = v0 + Math.sign(d) * Math.min(Math.abs(d), Math.max(div(this.#rate * ms, 1_000), inAPulse));
    }
    return wrap360Mdeg(v);
  }

  /** How long an order may go unanswered: its pulse and a second. */
  #grace(): number {
    return this.#tempoMs + 1_000;
  }

  /** How far a turn held for `ms` has swung the picture: `TAP` over the
   * first `TAP_MS` (or the turn rate, if that is slower), then the turn rate.
   * The slow start is what makes a tap one degree and no more. */
  #swing(ms: number): number {
    const slow = Math.min(Math.max(ms, 0), TAP_MS);
    const fast = Math.max(ms - slow, 0);
    return (
      Math.min(div(TAP * slow, TAP_MS), div(this.#rate * slow, 1_000)) +
      div(this.#rate * fast, 1_000)
    );
  }

  /** How far ahead of the picture a turn held for `ms` orders: what the
   * picture swings in the next `ORDER_GAP_MS`, when the next order goes. A
   * few degrees (7.5° on the stock tank), so a pulse that resolves between
   * two orders leaves the tank that little ahead of the picture, and the
   * release brings it back no further. */
  #lead(ms: number): number {
    return clamp(this.#swing(ms + ORDER_GAP_MS) - this.#swing(ms), LEAD_BAND);
  }

  /** The furthest the picture runs ahead of the last frame: a pulse and a
   * half of turning, inside `LEASH_BAND`. */
  #leash(): number {
    return clamp(div(this.#rate * (this.#tempoMs + div(this.#tempoMs, 2)), 1_000), LEASH_BAND);
  }

  /** Order `heading`, to the whole degree. A degree already standing is not
   * ordered again unless `always`. */
  #order(heading: number, now: number, always = false): void {
    const deg = div(wrap360Mdeg(heading) + 500, 1_000) % 360;
    if (!always && this.ordered === deg * 1_000) return;
    this.#orders.push(`HEADING ${deg} QUIET`);
    this.#ordered = [deg * 1_000, now];
  }

  /** Start turning the tank `dir`, from the heading the picture shows, and
   * order the tap's one degree: let go at once and that order stands. Nothing
   * before a frame has been seen: there is no heading to turn from. */
  start(dir: -1 | 1, now: number): void {
    const p = this.predicted(now);
    if (p === null) return;
    this.#shown = [p, now];
    this.#turnFrom = [p, now];
    this.#turning = dir;
    this.#order(p + dir * TAP, now);
    this.#prior = null;
  }

  /** Stop turning: order the heading the picture shows, which the picture
   * then holds. A press let go inside its first degree is a tap, and turns
   * the tank that one degree. */
  stop(now: number): void {
    if (this.#turning === 0) return;
    const dir = this.#turning;
    const stands = this.predicted(now);
    this.#turning = 0;
    this.#prior = null;
    const turnFrom = this.#turnFrom;
    this.#turnFrom = null;
    if (stands === null) return;
    this.#shown = [stands, now];
    const from = turnFrom?.[0] ?? stands;
    this.#order(dir * arcMdeg(from, stands) < TAP ? from + dir * TAP : stands, now);
  }

  /** Steer to `headingMdeg`, a heading the pilot picked (a click on the tape
   * or the map): any held turn ends, the line always goes, and the picture
   * closes on it from where it stands. */
  steer(headingMdeg: number, now: number): void {
    const stands = this.predicted(now);
    this.#turning = 0;
    this.#turnFrom = null;
    this.#prior = null;
    if (stands !== null) this.#shown = [stands, now];
    this.#order(headingMdeg, now, true);
  }

  /** Hold the heading the picture shows (all stop): a held turn ends, and a
   * heading ordered and not yet reached is ordered away. */
  hold(now: number): void {
    const stands = this.predicted(now);
    if (stands !== null) this.steer(stands, now);
  }

  /** While a turn is held, order on ahead of the picture every
   * `ORDER_GAP_MS`. A turn or a released order no frame has spoken to for a
   * second past its pulse was not taken: it is dropped, and the picture goes
   * back to the frames. */
  tick(now: number): void {
    const seen = this.#seen;
    if (seen === null) return;
    const grace = this.#grace();
    const silent = now - seen.at > grace;
    if (this.#turning === 0) {
      if (silent && this.#ordered !== null && now - this.#ordered[1] > grace) {
        const p = this.predicted(now);
        if (p !== null) this.#shown = [p, now];
        this.#ordered = null;
      }
      return;
    }
    const p = this.predicted(now);
    const turnFrom = this.#turnFrom;
    if (p === null || turnFrom === null) return;
    if (silent && now - turnFrom[1] > grace) {
      this.#shown = [p, now];
      this.#turning = 0;
      this.#turnFrom = null;
      this.#ordered = null;
      this.#prior = null;
      return;
    }
    if (this.#ordered === null || now - this.#ordered[1] >= ORDER_GAP_MS) {
      const lead = this.#lead(msBetween(turnFrom[1], now));
      const was = this.ordered;
      this.#order(p + this.#turning * lead, now);
      if (this.ordered !== was) this.#prior = was;
    }
  }

  /** The `HEADING` lines to send, oldest first. */
  takeOrders(): string[] {
    const out = this.#orders;
    this.#orders = [];
    return out;
  }
}
