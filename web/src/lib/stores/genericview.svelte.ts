// Generic-view store (interface plan I9): the latest parsed `GenericView`
// for each `<name>.view` feed. A module that publishes a schema-bearing
// view keyframe on a `.view` feed gets it held here and rendered by
// `GenericView.svelte` (web) or `render_view_rows` (TUI) with no bespoke
// code — the `.view` suffix joins the `.sheet` / `.roll` / `.vitals`
// conventions the clients already key off.

import { parseGenericView, type GenericView } from "../genericview";

/** The `.view` feed suffix a generic view rides. */
export const VIEW_FEED_SUFFIX = ".view";

/** Whether `feed` is a generic-view feed. */
export function isViewFeed(feed: string): boolean {
  return feed.endsWith(VIEW_FEED_SUFFIX);
}

export class GenericViewStore {
  views = $state<Record<string, GenericView>>({});

  /** Apply a keyframe for `feed`; `false` when the payload was not a usable
   * generic view (a malformed schema is ignored, never thrown). */
  apply(feed: string, raw: unknown): boolean {
    const view = parseGenericView(raw);
    if (!view) {
      return false;
    }
    this.views = { ...this.views, [feed]: view };
    return true;
  }

  /** The held view for `feed`, or `null`. */
  get(feed: string): GenericView | null {
    return this.views[feed] ?? null;
  }

  /** The feeds held, sorted. */
  get feeds(): string[] {
    return Object.keys(this.views).sort();
  }

  clear(): void {
    this.views = {};
  }
}
