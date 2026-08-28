// Sheet panel state — the `<ruleset>.sheet` keyframe (schema + data +
// approval), kept schema-generic: this store never inspects field ids or
// template names, only the shapes `lib/sheet.ts` defines.
//
// Hydration: `GameApp` issues `Req { q: sheet }` on `hello` (gated on a
// `*.sheet` panel being in the server's panel manifest — see
// `SessionStore.sheetFeed`) and again on manual refresh
// (`GameApp.refreshSheet`) and a light poll while the Sheet tab is mounted
// (`SheetPanel.svelte`) — there is no server-pushed delta yet for sheet
// mutations (`+harm`, `+xp` approval, splat pool spends, …); see
// web/README.md for the exact gap. `applyDelta` exists and is wired for the
// day a delta frame ships (a partial `data`/`approved` patch), so this store
// needs no further change when that lands.

import {
  parseApproval,
  parseSheetPayload,
  type SheetApproval,
  type SheetSchema,
} from "../sheet";

export class SheetStore {
  schema = $state<SheetSchema | null>(null);
  data = $state<Record<string, unknown>>({});
  approval = $state<SheetApproval | null>(null);
  target = $state<string | null>(null);
  /** A keyframe has been successfully applied at least once. */
  loaded = $state(false);
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
   * reply). Returns whether the payload was usable. */
  applyKeyframe(raw: unknown): boolean {
    const parsed = parseSheetPayload(raw);
    if (!parsed) {
      return false;
    }
    this.schema = parsed.schema;
    this.data = parsed.data;
    this.approval = parsed.approved;
    this.target = parsed.target;
    this.loaded = true;
    this.unavailable = false;
    return true;
  }

  /** Apply an incremental delta: shallow-merges `data`/`approved` onto the
   * current state (no server delta frame exists yet — see the module docs —
   * so this is exercised by tests and future-proofing, not live traffic). */
  applyDelta(raw: unknown): void {
    if (typeof raw !== "object" || raw === null) {
      return;
    }
    const o = raw as Record<string, unknown>;
    if (typeof o.data === "object" && o.data !== null && !Array.isArray(o.data)) {
      this.data = { ...this.data, ...(o.data as Record<string, unknown>) };
      this.loaded = true;
    }
    if (o.approved !== undefined) {
      this.approval = parseApproval(o.approved);
    }
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
    this.unavailable = false;
  }
}
