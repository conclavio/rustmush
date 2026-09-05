// oWoD20 roll readout — pure types + formatting helpers for the `wod.roll`
// panel feed (crates/ruleset-wod/src/roll.rs's `emit_roll` +
// crates/mush-dice/src/owod20.rs's `Owod20Outcome`).
//
// Hand-written types for the same reason as sheet.ts: `Owod20Outcome` lives
// in mush-dice (not mush-proto) and the panel payload wraps it inside an
// opaque `serde_json::Value` (`OutputEvent::Panel.data`) — adding ts-rs there
// is a cross-crate touch outside this agent's web/-only scope. Shapes below
// mirror serde's *default* representation exactly (no `#[serde(...)]`
// attributes on `RollResult`/`Owod20Outcome`/`Owod20Die` — see owod20.rs):
//   RollResult -> "Failure" | { Botch: number } | { Success: number }
//   (externally tagged; a unit variant serializes as a bare string, a
//   tuple variant as `{ "<Variant>": <payload> }`)

export interface RollDie {
  face: number;
  is_success: boolean;
  is_one: boolean;
  doubled: boolean;
}

export type RollResultJson = "Failure" | { Botch: number } | { Success: number };

export interface Owod20Outcome {
  difficulty: number;
  dice: RollDie[];
  raw_successes: number;
  ones: number;
  willpower_success: boolean;
  extra_successes: number;
  net: number;
  result: RollResultJson;
}

/** The `wod.roll` panel payload (`roll.rs`'s `emit_roll` -> `roll_feed.keyframe`). */
export interface RollPanelData {
  /** The roller's dbref, `"#123"` form (`ObjId::to_string()`). */
  who: string;
  /** The classic one-line summary already rendered server-side. */
  summary: string;
  outcome: Owod20Outcome;
}

// ---------------------------------------------------------------------------
// Defensive parsing
// ---------------------------------------------------------------------------

function isObj(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function parseDie(raw: unknown): RollDie | null {
  if (!isObj(raw) || typeof raw.face !== "number") {
    return null;
  }
  return {
    face: raw.face,
    is_success: raw.is_success === true,
    is_one: raw.is_one === true,
    doubled: raw.doubled === true,
  };
}

function parseResult(raw: unknown): RollResultJson {
  if (raw === "Failure") {
    return "Failure";
  }
  if (isObj(raw)) {
    if (typeof raw.Botch === "number") {
      return { Botch: raw.Botch };
    }
    if (typeof raw.Success === "number") {
      return { Success: raw.Success };
    }
  }
  return "Failure";
}

function parseOutcome(raw: unknown): Owod20Outcome | null {
  if (!isObj(raw) || typeof raw.difficulty !== "number") {
    return null;
  }
  const dice = Array.isArray(raw.dice) ? raw.dice.flatMap((d) => parseDie(d) ?? []) : [];
  return {
    difficulty: raw.difficulty,
    dice,
    raw_successes: typeof raw.raw_successes === "number" ? raw.raw_successes : 0,
    ones: typeof raw.ones === "number" ? raw.ones : 0,
    willpower_success: raw.willpower_success === true,
    extra_successes: typeof raw.extra_successes === "number" ? raw.extra_successes : 0,
    net: typeof raw.net === "number" ? raw.net : 0,
    result: parseResult(raw.result),
  };
}

/** Parse a `wod.roll` panel payload; `null` if it doesn't even carry a
 * usable outcome (the readout has nothing to show in that case). */
export function parseRollPanel(raw: unknown): RollPanelData | null {
  if (!isObj(raw) || typeof raw.who !== "string") {
    return null;
  }
  const outcome = parseOutcome(raw.outcome);
  if (!outcome) {
    return null;
  }
  return {
    who: raw.who,
    summary: typeof raw.summary === "string" ? raw.summary : fallbackSummary(outcome),
    outcome,
  };
}

function fallbackSummary(o: Owod20Outcome): string {
  return classifyResult(o.result).label;
}

// ---------------------------------------------------------------------------
// Readout formatting
// ---------------------------------------------------------------------------

export type RollResultKind = "botch" | "failure" | "success";

export interface RollResultInfo {
  kind: RollResultKind;
  label: string;
}

/** `RollResult` -> a display kind (for CSS) + a human label. */
export function classifyResult(r: RollResultJson): RollResultInfo {
  if (r === "Failure") {
    return { kind: "failure", label: "Failure" };
  }
  if ("Botch" in r) {
    return { kind: "botch", label: r.Botch === 1 ? "Botch" : `Botch (${r.Botch})` };
  }
  return { kind: "success", label: r.Success === 1 ? "1 success" : `${r.Success} successes` };
}

/** One die's CSS class: highlights successes (green) and 1s (red); doubled
 * (specialty) successes get an extra marker class. */
export function dieClass(d: RollDie): string {
  if (d.is_success) {
    return d.doubled ? "die-success die-doubled" : "die-success";
  }
  if (d.is_one) {
    return "die-one";
  }
  return "die-plain";
}

/** Whether a `.roll` frame is the viewer's own (the dice card is theirs;
 * everyone else reads the room's one-line narration — interface plan I5). */
export function isOwnRoll(who: string, selfId: string | null): boolean {
  return selfId !== null && who === selfId;
}

/** `who` (a dbref) -> "You" when it's the viewer's own character, else the
 * raw dbref — the client has no name-resolution for arbitrary dbrefs beyond
 * its own session, and the roll panel is pushed only to the roller anyway. */
export function youOrId(who: string, selfId: string | null): string {
  return selfId !== null && who === selfId ? "You" : who;
}
