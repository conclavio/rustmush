import type { ViewEnvelope } from "./proto";

export interface ViewCursor {
  view: string;
  schemaVersion: number;
  generation: number;
  sequence: number;
}

export type ViewFrameDisposition = "replace" | "apply_delta" | "ignore_stale" | "resync";

const VIEW_ID = /^[a-z0-9._-]{1,96}$/;
const MAX_SAFE_WIRE_INTEGER = Number.MAX_SAFE_INTEGER;

function wireInteger(value: number, allowZero: boolean): boolean {
  return (
    Number.isSafeInteger(value) &&
    value >= (allowZero ? 0 : 1) &&
    value <= MAX_SAFE_WIRE_INTEGER
  );
}

/**
 * Classify one untrusted semantic-view frame without changing client state.
 * Only `replace` and `apply_delta` authorize a cursor advance.
 */
export function classifyViewFrame(
  frame: ViewEnvelope,
  expectedView: string,
  supportedSchemaVersion: number,
  current: ViewCursor | null,
): ViewFrameDisposition {
  const base = frame.base_sequence ?? null;
  if (
    !VIEW_ID.test(frame.view) ||
    frame.view !== expectedView ||
    !wireInteger(frame.schema_version, false) ||
    frame.schema_version !== supportedSchemaVersion ||
    !wireInteger(frame.generation, false) ||
    !wireInteger(frame.sequence, true) ||
    (base !== null && (!wireInteger(base, true) || base >= frame.sequence))
  ) {
    return "resync";
  }

  if (base === null) {
    if (
      current !== null &&
      current.view === frame.view &&
      current.schemaVersion === frame.schema_version &&
      current.generation === frame.generation &&
      current.sequence >= frame.sequence
    ) {
      return "ignore_stale";
    }
    return "replace";
  }

  if (
    current === null ||
    current.view !== frame.view ||
    current.schemaVersion !== frame.schema_version ||
    current.generation !== frame.generation
  ) {
    return "resync";
  }
  if (frame.sequence <= current.sequence) {
    return "ignore_stale";
  }
  return base === current.sequence ? "apply_delta" : "resync";
}

export function cursorAfter(frame: ViewEnvelope): ViewCursor {
  return {
    view: frame.view,
    schemaVersion: frame.schema_version,
    generation: frame.generation,
    sequence: frame.sequence,
  };
}
