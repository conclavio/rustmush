// Input history — classic readline up/down over the last N submitted lines,
// preserving the in-progress draft while browsing.

export const HISTORY_CAP = 200;

export class HistoryBuffer {
  #lines: string[] = [];
  #cursor: number | null = null;
  #draft = "";
  readonly #cap: number;

  constructor(cap = HISTORY_CAP) {
    this.#cap = cap;
  }

  get lines(): readonly string[] {
    return this.#lines;
  }

  /** Record a submitted line (consecutive duplicates collapse). */
  push(line: string): void {
    if (line.length > 0 && this.#lines[this.#lines.length - 1] !== line) {
      this.#lines.push(line);
      if (this.#lines.length > this.#cap) {
        this.#lines.splice(0, this.#lines.length - this.#cap);
      }
    }
    this.#cursor = null;
    this.#draft = "";
  }

  /** Arrow-up: older entry. `current` is the input's live value (saved as
   * the draft on the first step). Returns the value to show, or null when
   * there is no history. */
  up(current: string): string | null {
    if (this.#lines.length === 0) {
      return null;
    }
    if (this.#cursor === null) {
      this.#draft = current;
      this.#cursor = this.#lines.length - 1;
    } else if (this.#cursor > 0) {
      this.#cursor -= 1;
    }
    return this.#lines[this.#cursor] ?? null;
  }

  /** Arrow-down: newer entry, or back to the draft past the newest. Returns
   * the value to show, or null when not browsing. */
  down(): string | null {
    if (this.#cursor === null) {
      return null;
    }
    this.#cursor += 1;
    if (this.#cursor >= this.#lines.length) {
      this.#cursor = null;
      return this.#draft;
    }
    return this.#lines[this.#cursor] ?? null;
  }

  /** Stop browsing (e.g. the user edits the line). */
  reset(): void {
    this.#cursor = null;
    this.#draft = "";
  }
}
