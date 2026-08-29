// Worldgen draft-preview panel — the `worldgen.draft` panel feed
// (`crates/mush-worldgen/src/commands.rs`'s `emit_draft_panel`, payload
// shape from `DraftPayload`/`StoredDraft` in `crates/mush-worldgen/src/data.rs`).
// Hand-written types walked defensively at runtime, same convention as
// `sheet.ts`/`roll.ts` — this is opaque JSON to `mush-proto` (a module
// panel payload, never a ts-rs type), and unlike the sheet/roll feeds the
// worldgen module is fixed (not swapped per ruleset), so the feed id
// `"worldgen.draft"` is a literal, not a suffix-derived one.
//
// Contract (one keyframe per finished `@gen`/`@gen/retry` call, to the
// requesting builder only — there is no delta variant and no query handler
// to re-fetch on demand; see `WorldgenStore`'s doc comment for what that
// means for Accept/Retry/Discard):
//
// ```jsonc
// {
//   "room": "#42",                 // ObjId::to_string()
//   "room_name": "The Pawn Shop",
//   "builder": "#7",
//   "model": "claude-opus-5",
//   "current": { "desc": "…the room's desc attribute right now…" },
//   "proposed": {
//     "desc": "…generated desc…",
//     "short_desc": "a cramped pawn shop",
//     "details": [ { "keyword": "counter", "desc": "…" } ],
//     "exits": [ { "direction": "north", "name": "Out", "desc": "…", "transit_msg": "…" } ]
//   }
// }
// ```
//
// `desc` is the model's only required field server-side; everything else
// may be empty/absent, so every field here defaults rather than failing
// parse (`DraftPayload`'s `#[serde(default)]`, mirrored here).

export interface WorldgenDetail {
  keyword: string;
  desc: string;
}

export interface WorldgenExit {
  direction: string;
  name: string;
  desc: string;
  transitMsg: string;
}

export interface WorldgenDraftPayload {
  desc: string;
  shortDesc: string;
  details: WorldgenDetail[];
  exits: WorldgenExit[];
}

export interface WorldgenDraft {
  room: string;
  roomName: string;
  builder: string;
  model: string;
  /** The room's current `desc` attribute, for the side-by-side compare
   * (the only field the server sends a "current" counterpart for). */
  currentDesc: string;
  proposed: WorldgenDraftPayload;
}

function str(v: unknown): string {
  return typeof v === "string" ? v : "";
}

function obj(v: unknown): Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v)
    ? (v as Record<string, unknown>)
    : {};
}

function parseDetails(v: unknown): WorldgenDetail[] {
  if (!Array.isArray(v)) {
    return [];
  }
  return v
    .map(obj)
    .map((d) => ({ keyword: str(d.keyword), desc: str(d.desc) }))
    .filter((d) => d.keyword.trim().length > 0 || d.desc.trim().length > 0);
}

function parseExits(v: unknown): WorldgenExit[] {
  if (!Array.isArray(v)) {
    return [];
  }
  return v
    .map(obj)
    .map((e) => ({
      direction: str(e.direction),
      name: str(e.name),
      desc: str(e.desc),
      transitMsg: str(e.transit_msg),
    }))
    .filter((e) => e.direction.trim().length > 0 || e.name.trim().length > 0 || e.desc.trim().length > 0);
}

function parsePayload(v: unknown): WorldgenDraftPayload {
  const o = obj(v);
  return {
    desc: str(o.desc),
    shortDesc: str(o.short_desc),
    details: parseDetails(o.details),
    exits: parseExits(o.exits),
  };
}

/** Parse one `worldgen.draft` panel frame. `null` when the payload isn't a
 * usable draft at all (not an object, or carries no room reference) — an
 * empty `desc` inside an otherwise-real draft is not itself a parse
 * failure (the model can legitimately return sparse fields; the server's
 * own `validate()` is what rejects an empty `desc` before this ever ships). */
export function parseWorldgenDraft(raw: unknown): WorldgenDraft | null {
  if (typeof raw !== "object" || raw === null) {
    return null;
  }
  const o = raw as Record<string, unknown>;
  const room = str(o.room);
  if (room.trim().length === 0) {
    return null;
  }
  return {
    room,
    roomName: str(o.room_name),
    builder: str(o.builder),
    model: str(o.model),
    currentDesc: str(obj(o.current).desc),
    proposed: parsePayload(o.proposed),
  };
}

// -- outbound Line builders --------------------------------------------
//
// No new server protocol (design note for this panel): every action is the
// existing `@gen` command surface (`crates/mush-worldgen/src/commands.rs`),
// sent as a plain Line, exactly like the mail compose modal's
// `@mail/quick`. Pure functions so the exact wire text is unit-testable
// without going through `GameApp`/`WsClient`.

/** `@gen/accept` — write every field of the current draft through the
 * normal attribute-set path. (The panel offers no per-field picker; that
 * stays a terminal-only `@gen/accept <fields>` flow.) */
export function genAcceptLine(): string {
  return "@gen/accept";
}

/** `@gen/discard` — throw the current draft away. */
export function genDiscardLine(): string {
  return "@gen/discard";
}

/** `@gen/retry [guidance]` — regenerate, with the builder's extra guidance
 * appended (newlines collapsed to spaces, like the mail quick-send fields;
 * the input is a single-line prompt in the panel). Empty/whitespace-only
 * guidance omits the argument entirely. */
export function genRetryLine(guidance: string): string {
  const g = guidance.replace(/[\r\n]+/g, " ").trim();
  return g.length > 0 ? `@gen/retry ${g}` : "@gen/retry";
}
