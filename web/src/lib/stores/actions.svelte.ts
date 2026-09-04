import type { ViewEnvelope } from "../proto";
import { classifyViewFrame, cursorAfter, type ViewCursor } from "../view";

export interface LegalActionPresentation {
  verb: string;
  label: string;
  help: string;
}

/** Viewer-safe subset consumed by the web action surface. */
export interface LegalAction {
  offer: string;
  presentation: LegalActionPresentation;
  /** The remaining projected fields stay opaque until a control implements them. */
  projected: Record<string, unknown>;
}

function parseActions(data: unknown): LegalAction[] | null {
  if (!Array.isArray(data) || data.length > 128) {
    return null;
  }
  const actions: LegalAction[] = [];
  for (const raw of data) {
    if (typeof raw !== "object" || raw === null) {
      return null;
    }
    const action = raw as Record<string, unknown>;
    const presentation = action.presentation;
    if (
      typeof action.offer !== "string" ||
      typeof presentation !== "object" ||
      presentation === null
    ) {
      return null;
    }
    const shown = presentation as Record<string, unknown>;
    if (
      typeof shown.verb !== "string" ||
      typeof shown.label !== "string" ||
      typeof shown.help !== "string"
    ) {
      return null;
    }
    actions.push({
      offer: action.offer,
      presentation: { verb: shown.verb, label: shown.label, help: shown.help },
      projected: action,
    });
  }
  return actions;
}

export type LegalActionViewResult = "applied" | "ignored" | "resync";

export class LegalActionsStore {
  actions = $state<LegalAction[]>([]);
  loaded = $state(false);
  resyncPending = $state(false);
  #cursor: ViewCursor | null = null;

  apply(frame: ViewEnvelope): LegalActionViewResult {
    const disposition = classifyViewFrame(frame, "legal.actions", 1, this.#cursor);
    if (disposition === "ignore_stale") {
      return "ignored";
    }
    // No legal.actions delta merge schema exists yet. Even a continuous delta
    // therefore fails closed until that schema is registered client-side.
    if (disposition !== "replace") {
      this.requireResync();
      return "resync";
    }
    const actions = parseActions(frame.data);
    if (actions === null) {
      this.requireResync();
      return "resync";
    }
    this.actions = actions;
    this.loaded = true;
    this.resyncPending = false;
    this.#cursor = cursorAfter(frame);
    return "applied";
  }

  requireResync(): boolean {
    this.actions = [];
    this.loaded = false;
    if (this.resyncPending) {
      return false;
    }
    this.resyncPending = true;
    return true;
  }

  clear(): void {
    this.actions = [];
    this.loaded = false;
    this.resyncPending = false;
    this.#cursor = null;
  }
}
