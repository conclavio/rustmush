// The cockpit's picture (doc 21 cuts G.3c, G.8): whether it is up, the camera
// and the look, the keyframes `PictureView.svelte` draws it from, and what the
// pilot has set that a screen switched away and back must not lose (the zoom
// of each camera, the ordered speed, a lock asked for and not yet shown).

import { Throttle } from "../helm";
import { ZOOM_DEFAULT_BY_KIND, type PictureKind, type SpaceCameraMode } from "../picture";

/** The picture's state for this connection. */
export class PictureStore {
  /** Whether the picture is up and drawing. */
  on = $state(false);
  /** The look, millidegrees off the tank's heading: the viewer's alone, and
   * kept while the picture is hidden, as the TUI keeps its own. */
  lookMdeg = $state(0);
  /** Which camera is up (doc 21 cut G.12c): first person, overhead or 2.5D.
   * Kept here, not in the view, so the screen switched away and back keeps
   * it (doc 21 section 7.3). */
  kind = $state<PictureKind>("cockpit");
  /** Active space camera mode (`holotank`, `space_cockpit`, or `docking`, `SF.18`) when in space. */
  spaceMode = $state<SpaceCameraMode | null>(null);
  /** The zoom of each ground camera, permille: the TUI's `zoom_by_camera`.
   * Each camera keeps its own, and the 2.5D one opens zoomed out. */
  zoomByKind = $state<Record<PictureKind, number>>({ ...ZOOM_DEFAULT_BY_KIND });
  /** The space cameras' zoom, permille. */
  spaceZoomPermille = $state(1_000);
  /** The ordered speed and its coalescing (`../helm`'s `Throttle`): here so
   * an order still outstanding survives the view's teardown. */
  readonly throttle = new Throttle();
  /** The lock last asked for, until a frame shows it or it goes stale: `t`
   * walks on from it, not from the frame's lock a pulse behind. */
  lockAsked: { designator: string; at: number } | null = null;
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
    if (this.cockpitData !== null) {
      this.spaceHolotankData = null;
      this.spaceCockpitData = null;
      this.spaceDockingData = null;
      this.spaceMode = null;
    }
  }

  /** An `arena.map` keyframe: kept. */
  observeMap(data: unknown): void {
    this.mapData = data;
  }

  private reconcileSpaceMode(): void {
    if (!this.hasSpaceFeeds) {
      this.spaceMode = null;
      return;
    }
    if (this.spaceMode === "docking" && this.spaceDockingData === null) {
      this.spaceMode =
        this.spaceCockpitData !== null
          ? "space_cockpit"
          : this.spaceHolotankData !== null
            ? "holotank"
            : null;
    } else if (this.spaceMode === "space_cockpit" && this.spaceCockpitData === null) {
      this.spaceMode =
        this.spaceHolotankData !== null
          ? "holotank"
          : this.spaceDockingData !== null
            ? "docking"
            : null;
    } else if (this.spaceMode === "holotank" && this.spaceHolotankData === null) {
      this.spaceMode =
        this.spaceCockpitData !== null
          ? "space_cockpit"
          : this.spaceDockingData !== null
            ? "docking"
            : null;
    }
  }

  /** A `space.holotank` keyframe (`SF.18`). */
  observeSpaceHolotank(data: unknown): void {
    const active = (data as { active?: unknown; seated?: unknown } | null);
    const cleared = data === null || active?.active === false || active?.seated === false;
    this.spaceHolotankData = cleared ? null : data;
    if (this.spaceMode === null && this.spaceHolotankData !== null) {
      this.spaceMode = "holotank";
    } else {
      this.reconcileSpaceMode();
    }
  }

  /** A `space.cockpit` keyframe (`SF.18`). */
  observeSpaceCockpit(data: unknown): void {
    const active = (data as { active?: unknown; seated?: unknown } | null);
    const cleared = data === null || active?.active === false || active?.seated === false;
    this.spaceCockpitData = cleared ? null : data;
    if (this.spaceMode === null && this.spaceCockpitData !== null) {
      this.spaceMode = "space_cockpit";
    } else {
      this.reconcileSpaceMode();
    }
  }

  /** A `space.docking` keyframe (`SF.18`). */
  observeSpaceDocking(data: unknown): void {
    const active = (data as { active?: unknown; seated?: unknown } | null);
    const cleared = data === null || active?.active === false || active?.seated === false;
    this.spaceDockingData = cleared ? null : data;
    if (this.spaceDockingData !== null) {
      this.spaceMode = "docking";
    } else {
      this.reconcileSpaceMode();
    }
  }

  /** The connection went: the space feeds it carried are gone with it. */
  clear(): void {
    this.spaceHolotankData = null;
    this.spaceCockpitData = null;
    this.spaceDockingData = null;
    this.spaceMode = null;
  }
}
