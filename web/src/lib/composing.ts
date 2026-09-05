// The composing signal (interface plan I4, D-I-11): the client sends
// `composing { active: true }` on the first keystroke of a draft and
// `false` on send, on a cleared box, or after `COMPOSING_IDLE_MS` without a
// keystroke. The bridge rate-limits repeated starts, so a burst of typing
// after an idle stop costs one hint. Pure — time is an argument.

/** Milliseconds of silence before a draft counts as abandoned. */
export const COMPOSING_IDLE_MS = 5000;
/** Seconds a "typing" pill lives without an `active: false`. */
export const COMPOSING_TTL_S = 8;
/** Seconds an NPC "thinking" pill lives without an end frame. */
export const THINKING_TTL_S = 20;

/** What the client should send now, if anything. */
export type ComposingHint = "start" | "stop" | null;

export class ComposingSignal {
  #active = false;
  #lastKey = 0;

  /** Whether a draft is in progress as far as the room knows. */
  get active(): boolean {
    return this.#active;
  }

  /** A keystroke left `text` in the box at `now`. */
  keystroke(now: number, text: string): ComposingHint {
    if (text.length === 0) {
      return this.#stop();
    }
    this.#lastKey = now;
    if (this.#active) {
      return null;
    }
    this.#active = true;
    return "start";
  }

  /** The draft was sent or discarded. */
  send(): ComposingHint {
    return this.#stop();
  }

  /** A periodic check at `now`: stops a draft idle past the threshold. */
  idle(now: number): ComposingHint {
    if (this.#active && now - this.#lastKey >= COMPOSING_IDLE_MS) {
      return this.#stop();
    }
    return null;
  }

  #stop(): ComposingHint {
    if (!this.#active) {
      return null;
    }
    this.#active = false;
    return "stop";
  }
}
