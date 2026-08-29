// LLM cost dashboard panel state — the `llm.dashboard` keyframe (staff-only;
// see `crates/mush-worldgen/src/commands.rs`'s `LlmDashboardQuery`).
//
// Hydration: pull-only, exactly like the sheet panel — `GameApp.refreshLlm`
// sends `{t: "panel_subscribe", d: {feed: "llm.dashboard"}}` (the panel's
// mount effect, its Refresh button, and a light poll while it's the active
// tab; see `LlmPanel.svelte`). There is no server-pushed delta: the
// dashboard is a point-in-time aggregate snapshot, not a stream of events.
//
// Staff gating: the server refuses a non-staff query with a `Sys` line
// (never a Panel keyframe) — `markRefused` renders that distinctly from
// "not available at all" so a mortal who somehow opens the tab (they can:
// the client only knows the panel is *advertised*, never the viewer's role
// — see `SessionStore`'s `llmDashboardFeed` doc) sees "staff only", not a
// blank dashboard that looks broken.

import { parseDashboardPayload, type DashboardPayload } from "../llm";

export class LlmDashboardStore {
  data = $state<DashboardPayload | null>(null);
  /** A keyframe has been successfully applied at least once. */
  loaded = $state(false);
  /** The last query is still in flight (drives a spinner on the Refresh
   * button; distinct from `loaded` so a refresh doesn't blank the view
   * while it's running). */
  refreshing = $state(false);
  /** The server refused the query outright (not staff) — see the module
   * doc. Distinct from `unavailable` below. */
  refused = $state(false);
  /** The panel feed isn't advertised, or the query answered with some
   * other non-Panel fallback. */
  unavailable = $state(false);

  markRequested(): void {
    this.refreshing = true;
  }

  /** Apply a full keyframe. Returns whether the payload was usable. */
  applyKeyframe(raw: unknown): boolean {
    const parsed = parseDashboardPayload(raw);
    this.refreshing = false;
    if (!parsed) {
      return false;
    }
    this.data = parsed;
    this.loaded = true;
    this.refused = false;
    this.unavailable = false;
    return true;
  }

  /** The query answered `Sys` (staff-only refusal, or the feed isn't wired
   * up on this server at all). `text` distinguishes the two so the panel
   * can show the server's actual message. */
  markRefused(text: string): void {
    this.refreshing = false;
    // "Staff only" / "Royalty" phrasing is the refusal; anything else
    // (e.g. no ruleset/module loaded) reads as generic unavailability.
    if (/staff|royalty|wizard|permission/i.test(text)) {
      this.refused = true;
    } else {
      this.unavailable = true;
    }
  }

  clear(): void {
    this.data = null;
    this.loaded = false;
    this.refreshing = false;
    this.refused = false;
    this.unavailable = false;
  }
}
