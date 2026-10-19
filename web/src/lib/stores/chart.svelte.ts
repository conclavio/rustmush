// The sky chart's store (doc 19 cut A.3b): the latest `sky.chart` keyframe,
// decoded once at the JSON boundary. Pulled, never pushed: the hello pass asks
// for it where the manifest carries the feed, the chart screen's `r` asks
// again, and nothing here moves on its own — the sky moves by the IC day and
// a stale chart is a chart of an hour ago.

import { parseChartFrame, type ChartFrame } from "../chart";

export class ChartStore {
  frame = $state<ChartFrame | null>(null);
  /** The last keyframe that did not decode, for the screen's one sentence. */
  refused = $state(false);

  applyKeyframe(raw: unknown): void {
    const frame = parseChartFrame(raw);
    if (frame) {
      this.frame = frame;
      this.refused = false;
    } else {
      this.refused = true;
    }
  }

  clear(): void {
    this.frame = null;
    this.refused = false;
  }
}
