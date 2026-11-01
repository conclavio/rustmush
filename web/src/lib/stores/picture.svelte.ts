// The cockpit's picture (doc 21 cut G.3c): whether this connection has asked
// the gateway for it, at what look, and the last frame it sent — one PNG,
// kept as the Blob it arrived as and drawn by `PictureView.svelte`.

import type { PictureKind, SpaceCameraMode } from "../picture";

/** The picture's state for this connection. */
export class PictureStore {
  /** Whether the picture is asked for. */
  on = $state(false);
  /** The look sent with it, millidegrees. */
  lookMdeg = $state(0);
  /** Which camera is wanted (doc 21 cut G.12c): the first-person view, or the
   * overhead — which only the gateway draws, so while it is chosen the
   * picture is the server's whatever this browser could draw. Kept here, not
   * in the view, so the screen switched away and back keeps it (doc 21 §7.3). */
  kind = $state<PictureKind>("cockpit");
  /** Active space camera mode (`holotank`, `space_cockpit`, or `docking`, `SF.18`) when in space. */
  spaceMode = $state<SpaceCameraMode | null>(null);
  /** The camera last sent to the gateway, so a change of camera alone is
   * sent even when `kind` was set before the ask. */
  sentKind: PictureKind = "cockpit";
  /** The last frame, or `null` before one arrived or after the picture went
   * off. */
  frame = $state<Blob | null>(null);
  /** Frames received since the picture last went on. */
  received = $state(0);
  /** **The keyframes the picture is drawn from in the browser** (doc 21
   * cut G.3b): this connection's latest `arena.cockpit` (none once it says
   * the seat went) and `arena.map`, as they arrived. Raw state: a keyframe is
   * replaced whole, never edited, and the drawing tells them apart by
   * identity. */
  cockpitData = $state.raw<unknown>(null);
  mapData = $state.raw<unknown>(null);
  /** Space panel keyframes (`space.holotank`, `space.cockpit`, `space.docking`, `SF.18`). */
  spaceHolotankData = $state.raw<unknown>(null);
  spaceCockpitData = $state.raw<unknown>(null);
  spaceDockingData = $state.raw<unknown>(null);

  /** Whether any 3D spaceflight panel keyframe is currently held (`SF.18`). */
  get hasSpaceFeeds(): boolean {
    return (
      this.spaceHolotankData !== null ||
      this.spaceCockpitData !== null ||
      this.spaceDockingData !== null
    );
  }

  /** A cockpit keyframe: kept, or dropped when it says the seat went. */
  observeCockpit(data: unknown): void {
    const seated = (data as { seated?: unknown } | null)?.seated;
    this.cockpitData = seated === false ? null : data;
  }

  /** An `arena.map` keyframe: kept. */
  observeMap(data: unknown): void {
    this.mapData = data;
  }

  /** A `space.holotank` keyframe (`SF.18`). */
  observeSpaceHolotank(data: unknown): void {
    this.spaceHolotankData = data;
    if (this.spaceMode === null && data !== null) {
      this.spaceMode = "holotank";
    }
  }

  /** A `space.cockpit` keyframe (`SF.18`). */
  observeSpaceCockpit(data: unknown): void {
    this.spaceCockpitData = data;
    if (this.spaceMode === null && data !== null) {
      this.spaceMode = "space_cockpit";
    }
  }

  /** A `space.docking` keyframe (`SF.18`). */
  observeSpaceDocking(data: unknown): void {
    this.spaceDockingData = data;
    if (data !== null) {
      this.spaceMode = "docking";
    }
  }

  /** A binary message from the gateway: the next frame. Ignored while off (a
   * frame already in flight when the picture went off). */
  receive(data: Blob | ArrayBuffer): void {
    if (!this.on) return;
    this.frame = data instanceof ArrayBuffer ? new Blob([data], { type: "image/png" }) : data;
    this.received += 1;
  }

  /** The picture went off, or the connection went: nothing is shown. */
  clear(): void {
    this.frame = null;
    this.received = 0;
  }
}
