// Session — connection status, server hello (feature/panel gating), the
// attached character, and the current input mode.

import type { AssetPackInfo, ObjRef, PromptSpec } from "../proto";
import type { ConnStatus } from "../ws";

export class SessionStore {
  status = $state<ConnStatus>("idle");
  character = $state<ObjRef | null>(null);
  account = $state<string | null>(null);
  connections = $state(0);
  serverName = $state("");
  features = $state<string[]>([]);
  panels = $state<string[]>([]);
  /** The asset pack the hello named (doc 21 cut G.11), or `null`: what this
   * client could fetch over the player's own SSH connection. Nothing on the
   * web reads it yet. */
  assetPack = $state<AssetPackInfo | null>(null);
  prompt = $state<PromptSpec | null>(null);

  /** `hello`-declared feature gate (e.g. hide Mail when "mail" is absent). */
  hasFeature(f: string): boolean {
    return this.features.includes(f);
  }

  hasPanel(p: string): boolean {
    return this.panels.includes(p);
  }

  /**
   * The manifest feed carrying `suffix`, SORTED-first (LB-21).
   *
   * A world with two rulesets loaded publishes two `.sheet` feeds, and the
   * manifest's own order is the server's registration order — which the
   * gateway's bridge and this client walk differently. Taking the first in
   * the array meant two clients on the same server could disagree about
   * which sheet is "the" sheet. The TUI already sorts (`panels.rs`), so
   * sorting here makes the tie-break identical in every client.
   */
  #feedWithSuffix(suffix: string): string | null {
    let best: string | null = null;
    for (const p of this.panels) {
      if (p.endsWith(suffix) && (best === null || p < best)) {
        best = p;
      }
    }
    return best;
  }

  /** The active ruleset's sheet panel feed (e.g. `"wod.sheet"`), or `null`
   * when no ruleset with a sheet is loaded — derived from the `hello`
   * panel manifest by suffix, never a hardcoded ruleset id, so a future
   * second ruleset (or a rename) needs no client change. Feature-gates the
   * Sheet tab. */
  get sheetFeed(): string | null {
    return this.#feedWithSuffix(".sheet");
  }

  /** The active ruleset's roll panel feed (e.g. `"wod.roll"`); same
   * suffix-derived convention as `sheetFeed`. Feature-gates the Rolls tab.
   * Note this is the feed the ROLL BUILDER targets — `#routePanel` renders
   * every `.roll` feed's frames, not just this one (LB-21). */
  get rollFeed(): string | null {
    return this.#feedWithSuffix(".roll");
  }

  /** The tactical unit panel feed (e.g. `"tactics.unit"`); same
   * suffix-derived convention as `sheetFeed`/`rollFeed`. Gates the unit
   * screen's registration and the hydration subscribe (interface cut I8). */
  get unitFeed(): string | null {
    return this.#feedWithSuffix(".unit");
  }

  /** Whether the worldgen draft-preview panel is available: `hello`
   * advertises the `"worldgen"` feature and/or the `"worldgen.draft"` panel
   * (fixed ids, unlike `sheetFeed`/`rollFeed` — the worldgen module isn't
   * swapped per ruleset, so there's nothing to derive by suffix). Gates the
   * Build tab; the client has no way to know the viewer's `BUILDER_LLM`
   * power, so a non-builder still sees the tab and gets the refusal as a
   * `Sys` line when they try `@gen` (docs/design/03 §5.4's gate rides the
   * normal command-permission path server-side). */
  get worldgenAvailable(): boolean {
    return this.hasFeature("worldgen") || this.hasPanel("worldgen.draft");
  }

  /** Whether the staff LLM cost dashboard tab should show at all. Same
   * contract as `worldgenAvailable`: the panel is advertised whenever
   * worldgen is loaded, regardless of the viewer's role — the client has
   * no way to know that, so a non-staff viewer still sees the tab and gets
   * the server's refusal rendered in it (`LlmDashboardStore.refused`). */
  get llmDashboardAvailable(): boolean {
    return this.hasPanel("llm.dashboard");
  }

  applyHello(d: { server: string; features: string[]; panels: string[]; asset_pack?: AssetPackInfo }): void {
    this.serverName = d.server;
    this.features = d.features;
    this.panels = d.panels;
    this.assetPack = d.asset_pack ?? null;
  }

  get masked(): boolean {
    return this.prompt?.mode === "masked" || this.prompt?.mode === "no_echo";
  }

  clear(): void {
    this.character = null;
    this.account = null;
    this.connections = 0;
    this.prompt = null;
  }
}
