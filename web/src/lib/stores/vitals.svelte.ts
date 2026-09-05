// Vitals (interface plan I1 D-I-2, I4): one `<ruleset>.vitals` frame per
// feed — a subject, a monotonic revision, and up to 16 gauges (a health
// track, a condition monitor, a pool, a readout). Whole frames replace; a
// lower revision is ignored (a late frame after a resume must not roll a
// gauge back). Read defensively at the JSON boundary; the wire types in
// ./proto are the contract.

import type { Vital, VitalRole, VitalValue, VitalsFrame } from "../proto";

/** One feed's latest frame. */
export interface VitalsGroup {
  feed: string;
  frame: VitalsFrame;
  /** Local receipt time (unix ms). */
  at: number;
}

const ROLES: readonly VitalRole[] = ["dominant", "track", "pool", "readout"];
const STATES = ["ok", "warn", "crit", "down"] as const;

function isObj(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function parseValue(raw: unknown): VitalValue | null {
  if (!isObj(raw) || typeof raw.kind !== "string") {
    return null;
  }
  switch (raw.kind) {
    case "bounded":
      return typeof raw.current === "number" &&
        typeof raw.min === "number" &&
        typeof raw.max === "number"
        ? { kind: "bounded", current: raw.current, min: raw.min, max: raw.max }
        : null;
    case "track":
      return Array.isArray(raw.boxes)
        ? {
            kind: "track",
            boxes: raw.boxes.map((b) => (typeof b === "string" ? b : null)),
            level: typeof raw.level === "string" ? raw.level : null,
          }
        : null;
    case "text":
      return typeof raw.text === "string" ? { kind: "text", text: raw.text } : null;
    default:
      return null;
  }
}

function parseVital(raw: unknown): Vital | null {
  if (!isObj(raw) || typeof raw.id !== "string" || typeof raw.label !== "string") {
    return null;
  }
  const role = ROLES.find((r) => r === raw.role);
  const state = STATES.find((s) => s === raw.state);
  const value = parseValue(raw.value);
  if (!role || !state || !value) {
    return null;
  }
  return {
    id: raw.id,
    label: raw.label,
    role,
    value,
    state,
    detail: typeof raw.detail === "string" ? raw.detail : null,
  };
}

/** Parse a `.vitals` keyframe; `null` when it is not one. */
export function parseVitalsFrame(raw: unknown): VitalsFrame | null {
  if (!isObj(raw) || !isObj(raw.subject) || typeof raw.revision !== "number") {
    return null;
  }
  const subject = raw.subject;
  if (typeof subject.id !== "string" || typeof subject.name !== "string") {
    return null;
  }
  if (!Array.isArray(raw.vitals)) {
    return null;
  }
  const vitals = raw.vitals.flatMap((v) => parseVital(v) ?? []).slice(0, 16);
  return {
    subject: { id: subject.id, name: subject.name },
    revision: raw.revision,
    vitals,
  };
}

/** A frame's vitals in display order: dominant, tracks, pools, readouts. */
export function orderedVitals(frame: VitalsFrame): Vital[] {
  const rank = (v: Vital): number => ROLES.indexOf(v.role);
  return [...frame.vitals].sort((a, b) => rank(a) - rank(b));
}

/** Fill percentage of a bounded gauge (0..100), or `null` for others. */
export function vitalPct(v: Vital): number | null {
  if (v.value.kind !== "bounded") {
    return null;
  }
  const { current, min, max } = v.value;
  const span = max - min;
  if (span <= 0) {
    return 0;
  }
  const clamped = Math.max(min, Math.min(max, current));
  return Math.round(((clamped - min) / span) * 100);
}

/** A track box's classic glyph: bashing `/`, lethal `X`, aggravated `*`,
 * anything else its first letter; an empty box is `""`. */
export function boxGlyph(mark: string | null): string {
  switch (mark) {
    case null:
      return "";
    case "bashing":
      return "/";
    case "lethal":
      return "X";
    case "aggravated":
      return "*";
    default:
      return mark.charAt(0).toUpperCase();
  }
}

export class VitalsStore {
  groups = $state<Record<string, VitalsGroup>>({});

  /** Apply a `.vitals` frame for `feed`; `false` when malformed or stale. */
  apply(feed: string, raw: unknown, atMs = Date.now()): boolean {
    const frame = parseVitalsFrame(raw);
    if (!frame) {
      return false;
    }
    const current = this.groups[feed];
    if (current && current.frame.revision > frame.revision) {
      return false;
    }
    this.groups = { ...this.groups, [feed]: { feed, frame, at: atMs } };
    return true;
  }

  /** Every group, by feed name. */
  get list(): VitalsGroup[] {
    return Object.values(this.groups).sort((a, b) => a.feed.localeCompare(b.feed));
  }

  get loaded(): boolean {
    return Object.keys(this.groups).length > 0;
  }

  clear(): void {
    this.groups = {};
  }
}
