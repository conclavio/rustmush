// Sheet panel state — the `<ruleset>.sheet` keyframe (schema + data +
// approval), kept schema-generic: this store never inspects field ids or
// template names, only the shapes `lib/sheet.ts` defines.
//
// Hydration: `GameApp` issues `Req { q: sheet }` on `hello` (gated on a
// `*.sheet` panel being in the server's panel manifest — see
// `SessionStore.sheetFeed`) and again on manual refresh
// (`GameApp.refreshSheet`) and a light poll while the Sheet tab is mounted
// (`SheetPanel.svelte`). Since interface plan I13 a sheet mutation republishes
// a live DELTA (the whole staff-filtered `data`, `schema` omitted) carrying a
// `revision` and a `base_revision`: this store is a pure revision watermark —
// a delta applies only when its `base_revision` matches the held revision,
// otherwise the store reports "resync" and `GameApp` re-requests a fresh
// keyframe (rule 3 self-heal). The watermark resets on `hello`.

import {
  parseApproval,
  parseSheetPayload,
  type SheetApproval,
  type SheetSchema,
} from "../sheet";

/** The outcome of applying a sheet delta (interface plan I13). */
export type SheetDeltaOutcome = "applied" | "stale" | "resync";

export class SheetStore {
  schema = $state<SheetSchema | null>(null);
  data = $state<Record<string, unknown>>({});
  approval = $state<SheetApproval | null>(null);
  target = $state<string | null>(null);
  /** A keyframe has been successfully applied at least once. */
  loaded = $state(false);
  /** The server's per-subject sheet revision the store currently holds
   * (interface plan I13); reset to 0 on `hello`/keyframe. */
  revision = $state(0);
  /** A keyframe re-request is in flight after a base-revision gap — the
   * storm guard so one gap does not fire a burst of `refreshSheet`s. */
  resyncPending = $state(false);
  /** The server answered the sheet query with a polite "not available"
   * (today: `mushd`'s `Query::Sheet` handler predates the ruleset landing —
   * see web/README.md) rather than a panel keyframe. */
  unavailable = $state(false);

  /** The owning ruleset id (`schema.ruleset`), once loaded — drives the
   * `wod`-tagged-Sys refresh heuristic in `GameApp` without hardcoding it. */
  get rulesetId(): string | null {
    return this.schema?.ruleset ?? null;
  }

  /** Apply a full keyframe (`kind: "keyframe"` panel frame, or the `Req`
   * reply). Seeds the revision watermark and clears any pending resync.
   * Returns whether the payload was usable. */
  applyKeyframe(raw: unknown): boolean {
    const parsed = parseSheetPayload(raw);
    if (!parsed) {
      return false;
    }
    this.schema = parsed.schema;
    this.data = parsed.data;
    this.approval = parsed.approved;
    this.target = parsed.target;
    const rev = (raw as { revision?: unknown }).revision;
    this.revision = typeof rev === "number" ? rev : 0;
    this.loaded = true;
    this.resyncPending = false;
    this.unavailable = false;
    return true;
  }

  /** Apply an incremental delta (interface plan I13). Approach B carries the
   * WHOLE staff-filtered `data`, so a match REPLACES `data` (a dropped field
   * is truly gone — no stale key). Returns:
   * - `"applied"` — `base_revision === revision`, state advanced;
   * - `"stale"` — an old or duplicate delta (or none held yet), ignored;
   * - `"resync"` — a gap or malformed frame; the caller re-requests a
   *   keyframe and the store is left untouched. */
  applyDelta(raw: unknown): SheetDeltaOutcome {
    if (!this.loaded) {
      return "stale";
    }
    if (typeof raw !== "object" || raw === null) {
      return "resync";
    }
    const o = raw as Record<string, unknown>;
    const rev = o.revision;
    const base = o.base_revision;
    if (typeof rev !== "number" || typeof base !== "number") {
      return "resync";
    }
    if (rev <= this.revision) {
      return "stale";
    }
    if (base !== this.revision) {
      return "resync";
    }
    if (typeof o.data === "object" && o.data !== null && !Array.isArray(o.data)) {
      this.data = o.data as Record<string, unknown>;
    }
    if (o.approved !== undefined) {
      this.approval = parseApproval(o.approved);
    }
    this.revision = rev;
    return "applied";
  }

  /** The sheet query answered with a fallback message instead of a panel
   * keyframe — the panel should render a graceful "not available" state. */
  markUnavailable(): void {
    this.loaded = false;
    this.unavailable = true;
  }

  clear(): void {
    this.schema = null;
    this.data = {};
    this.approval = null;
    this.target = null;
    this.loaded = false;
    this.revision = 0;
    this.resyncPending = false;
    this.unavailable = false;
  }
}
