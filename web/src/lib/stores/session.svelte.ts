// Session — connection status, server hello (feature/panel gating), the
// attached character, and the current input mode.

import type { ObjRef, PromptSpec } from "../proto";
import type { ConnStatus } from "../ws";

export class SessionStore {
  status = $state<ConnStatus>("idle");
  character = $state<ObjRef | null>(null);
  account = $state<string | null>(null);
  connections = $state(0);
  serverName = $state("");
  features = $state<string[]>([]);
  panels = $state<string[]>([]);
  prompt = $state<PromptSpec | null>(null);

  /** `hello`-declared feature gate (e.g. hide Mail when "mail" is absent). */
  hasFeature(f: string): boolean {
    return this.features.includes(f);
  }

  hasPanel(p: string): boolean {
    return this.panels.includes(p);
  }

  /** The active ruleset's sheet panel feed (e.g. `"wod.sheet"`), or `null`
   * when no ruleset with a sheet is loaded — derived from the `hello`
   * panel manifest by suffix, never a hardcoded ruleset id, so a future
   * second ruleset (or a rename) needs no client change. Feature-gates the
   * Sheet tab. */
  get sheetFeed(): string | null {
    return this.panels.find((p) => p.endsWith(".sheet")) ?? null;
  }

  /** The active ruleset's roll panel feed (e.g. `"wod.roll"`); same
   * suffix-derived convention as `sheetFeed`. Feature-gates the Rolls tab. */
  get rollFeed(): string | null {
    return this.panels.find((p) => p.endsWith(".roll")) ?? null;
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

  applyHello(d: { server: string; features: string[]; panels: string[] }): void {
    this.serverName = d.server;
    this.features = d.features;
    this.panels = d.panels;
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
