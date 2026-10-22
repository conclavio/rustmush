// The quarry's store (doc 20 cut SM.9): the latest `site.map` and `body.ground` keyframes,
// decoded once at the JSON boundary, the moment the site frame landed (the leg rule places a
// moving rig from it), and the screen's client state — the view, the family asked for and the
// rig picked. Pulled, never pushed: the screen asks when it becomes active, on `r`, on each of
// its view keys, and when a pushed `tv.status` keyframe says a rig's projection changed.

import {
  nextFamily,
  nextMode,
  nextRig,
  parseGroundFrame,
  parseSiteFrame,
  type GroundFrame,
  type QuarryMode,
  type SiteFrame,
} from "../quarry";

export class QuarryStore {
  site = $state<SiteFrame | null>(null);
  ground = $state<GroundFrame | null>(null);
  /** The wall clock (ms) the held site frame landed at. */
  siteAt = $state(0);
  /** The last keyframe that did not decode, for the screen's one sentence. */
  refused = $state(false);
  mode = $state<QuarryMode>("drive");
  /** The family word asked for (`null`: the server's default). */
  family = $state<string | null>(null);
  /**
   * The rig picked with `t` or a click (`null`: the frame's addressed rig); cleared when a held
   * `site.map` frame no longer lists that slot (a reason frame lists none).
   */
  rig = $state<number | null>(null);

  applySite(raw: unknown, nowMs: number): void {
    const frame = parseSiteFrame(raw);
    if (frame) {
      this.site = frame;
      this.siteAt = nowMs;
      this.refused = false;
      // The pre-commit round (the judge's correctness/F7): a frame that no longer lists the
      // picked rig — returned, or the hull moved — clears the pick, so the next ask carries no
      // rig; a reason frame lists none, so it clears a pick too.
      if (this.rig !== null && !(frame.site?.rigs.some((r) => r.slot === this.rig) ?? false)) {
        this.rig = null;
      }
    } else {
      this.refused = true;
    }
  }

  applyGround(raw: unknown): void {
    const frame = parseGroundFrame(raw);
    if (frame) {
      this.ground = frame;
    } else {
      this.refused = true;
    }
  }

  /** The rig a key addresses: the picked one, else the frame's addressed one. */
  get selectedRig(): number | null {
    return this.rig ?? this.site?.site?.addressed ?? null;
  }

  /** `t`: the next of the frame's rigs. */
  stepRig(): void {
    const slots = this.site?.site?.rigs.map((r) => r.slot) ?? [];
    this.rig = nextRig(slots, this.selectedRig);
  }

  /** `z`: the next view. */
  stepMode(): void {
    this.mode = nextMode(this.mode);
  }

  /** `l`: the next family of the frame shown. */
  stepFamily(): void {
    const block = this.mode === "ground" ? this.ground?.ground : this.site?.site;
    const shown = block?.family_word ?? this.family;
    this.family = nextFamily(block?.families ?? [], shown ?? null);
  }

  clear(): void {
    this.site = null;
    this.ground = null;
    this.siteAt = 0;
    this.refused = false;
    this.mode = "drive";
    this.family = null;
    this.rig = null;
  }
}
