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
