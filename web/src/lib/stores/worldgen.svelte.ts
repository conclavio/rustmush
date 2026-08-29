// Worldgen draft-preview panel state — the `worldgen.draft` keyframe
// (mush-worldgen's `emit_draft_panel`): the LLM-generated room content
// pending the builder's `@gen/accept|retry|discard` review.
//
// Pure push, one draft at a time for the viewer (drafts are keyed
// per-room-per-builder server-side, but the panel only ever shows "the
// draft for the room the last @gen touched" — a fresh keyframe simply
// replaces the last one). There is no delta frame and, unlike the sheet
// panel, no query handler to re-fetch the current draft on demand — so:
//
// - `@gen/accept` / `@gen/discard` always clear the *server-side* draft
//   (see commands.rs's `gen_accept`/`gen_discard`, both call `clear_draft`
//   unconditionally), but the server never pushes a frame saying so. The
//   panel therefore clears optimistically the moment the Line is sent.
// - `@gen/retry` keeps the old draft in place server-side until the new
//   one is ready (an off-loop LLM call — can take several seconds), so the
//   panel keeps showing it too, marked `pending`, until the fresh keyframe
//   lands and clears the flag.

import { parseWorldgenDraft, type WorldgenDraft } from "../worldgen";

export class WorldgenStore {
  draft = $state<WorldgenDraft | null>(null);
  /** A retry was sent and no fresh keyframe has landed yet. */
  pending = $state(false);

  /** Apply a `worldgen.draft` panel frame (keyframe today; a delta is
   * treated identically — see the module docs, there's no incremental
   * variant on the wire, so this is future-proofing plus defensive
   * handling of anything unrecognized). Returns whether it was usable. */
  applyKeyframe(raw: unknown): boolean {
    const parsed = parseWorldgenDraft(raw);
    if (!parsed) {
      return false;
    }
    this.draft = parsed;
    this.pending = false;
    return true;
  }

  /** `@gen/retry` was sent: keep the draft visible, flag it stale. */
  markPending(): void {
    if (this.draft !== null) {
      this.pending = true;
    }
  }

  clear(): void {
    this.draft = null;
    this.pending = false;
  }
}
