// Rolls panel state — a small ring of recent `wod.roll` panel pushes (the
// dice-engine payoff on the web side: pool/difficulty/dice/result). Pure
// push: `emit_roll` sends one keyframe per roll to the roller only (no
// query/hydration exists or is needed — there is nothing to fetch on load).

import { parseRollPanel, type Owod20Outcome } from "../roll";

/** Ring cap — recent rolls only; this is a live readout, not a log (that's
 * `+roll/last`, a terminal command). */
const MAX_ROLLS = 30;

export interface RollEntry {
  /** Store-assigned id — the panel's `#each` key (mirrors ScrollbackStore's
   * `id` / MailStore's `rowId`: a malformed/duplicate wire value must never
   * become a Svelte key). */
  rowId: number;
  who: string;
  summary: string;
  /** The oWoD20 dice detail, or `null` for another engine's roll (I6-C11) —
   * the summary is what every ruleset carries, so it is what the row shows;
   * the dice card is gated on this. */
  outcome: Owod20Outcome | null;
  ts: number;
}

export class RollsStore {
  entries = $state<RollEntry[]>([]);

  #nextRowId = 1;

  /** Apply one `.roll` panel frame. Returns the new entry, or `null` if the
   * payload was not a usable roll (no roller, or neither a summary nor an
   * outcome). */
  push(raw: unknown, ts = Date.now()): RollEntry | null {
    const parsed = parseRollPanel(raw);
    if (!parsed) {
      return null;
    }
    const entry: RollEntry = {
      rowId: this.#nextRowId++,
      who: parsed.who,
      summary: parsed.summary,
      outcome: parsed.outcome,
      ts,
    };
    this.entries = [entry, ...this.entries].slice(0, MAX_ROLLS);
    return entry;
  }

  clear(): void {
    this.entries = [];
  }
}
