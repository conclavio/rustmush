// Presence pills (interface plan I4): NPCs "thinking" (the `npc.thinking`
// feed, cleared by its end frame or `THINKING_TTL_S`) and people
// "composing" (`Composing` events, cleared by `active: false` or
// `COMPOSING_TTL_S`). A lost end frame costs nothing: pills expire on the
// component's clock.

import { COMPOSING_TTL_S, THINKING_TTL_S } from "../composing";

export interface Pill {
  name: string;
  /** Unix ms when the pill was (last) raised. */
  at: number;
}

function isObj(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

export class PresenceStore {
  thinking = $state<Pill[]>([]);
  composing = $state<Pill[]>([]);

  /** Apply an `npc.thinking` frame (`{ npc, name, active }`). */
  applyThinking(raw: unknown, atMs = Date.now()): void {
    if (!isObj(raw) || typeof raw.name !== "string" || raw.name.length === 0) {
      return;
    }
    const active = raw.active !== false;
    this.thinking = this.thinking.filter((p) => p.name !== raw.name);
    if (active) {
      this.thinking = [...this.thinking, { name: raw.name, at: atMs }];
    }
  }

  /** Apply a `Composing` event. */
  applyComposing(name: string, active: boolean, atMs = Date.now()): void {
    if (name.length === 0) {
      return;
    }
    this.composing = this.composing.filter((p) => p.name !== name);
    if (active) {
      this.composing = [...this.composing, { name, at: atMs }];
    }
  }

  /** Drop expired pills as of `nowMs`. */
  prune(nowMs: number): void {
    const thinking = this.thinking.filter((p) => nowMs - p.at < THINKING_TTL_S * 1000);
    if (thinking.length !== this.thinking.length) {
      this.thinking = thinking;
    }
    const composing = this.composing.filter((p) => nowMs - p.at < COMPOSING_TTL_S * 1000);
    if (composing.length !== this.composing.length) {
      this.composing = composing;
    }
  }

  clear(): void {
    this.thinking = [];
    this.composing = [];
  }
}
