import type { ViewEnvelope } from "./proto";
import type { JsonValue } from "./proto/serde_json/JsonValue";

export interface ViewCursor {
  view: string;
  schemaVersion: number;
  generation: number;
  sequence: number;
}

export type ViewFrameDisposition = "replace" | "apply_delta" | "ignore_stale" | "resync";

const VIEW_ID = /^[a-z0-9._-]{1,96}$/;
const MAX_SAFE_WIRE_INTEGER = Number.MAX_SAFE_INTEGER;
const LOWER_HEX_128 = /^[0-9a-f]{32}$/;
const CONTROL_CHARACTER = /[\u0000-\u001f\u007f-\u009f]/;
const MAX_ACTION_STATUS_BYTES = 256;
const MAX_ACTION_SUMMARY_BYTES = 4 * 1024;

export interface ProjectedActionResultView {
  kind: string;
  summary: string;
  data: JsonValue;
}

export interface ProjectedActionReceiptView {
  status: string;
  commit: string;
  result: ProjectedActionResultView | null;
}

function canonicalId(value: unknown): value is string {
  return (
    typeof value === "string" &&
    /^[a-z0-9_.]{1,96}$/.test(value) &&
    !value.startsWith(".") &&
    !value.endsWith(".") &&
    !value.includes("..")
  );
}

function actionStatusId(value: unknown): value is string {
  if (typeof value !== "string" || new TextEncoder().encode(value).length > MAX_ACTION_STATUS_BYTES) {
    return false;
  }
  const parts = value.split(":");
  return (
    parts.length === 5 &&
    parts[0] === "k5" &&
    canonicalId(parts[1]) &&
    canonicalId(parts[2]) &&
    /^[0-9a-f]{16}$/.test(parts[3] ?? "") &&
    LOWER_HEX_128.test(parts[4] ?? "")
  );
}

/** Strictly decode the v1 deterministic `resolution.outcome` keyframe. */
export function parseResolutionOutcome(
  frame: ViewEnvelope,
): ProjectedActionReceiptView | null {
  if (
    frame.view !== "resolution.outcome" ||
    frame.schema_version !== 1 ||
    frame.sequence !== 1 ||
    (frame.base_sequence ?? null) !== null ||
    !wireInteger(frame.generation, false) ||
    typeof frame.commit !== "string" ||
    !LOWER_HEX_128.test(frame.commit)
  ) {
    return null;
  }
  const expectedGeneration = Math.max(1, Number.parseInt(frame.commit.slice(0, 13), 16));
  if (frame.generation !== expectedGeneration) return null;
  const value = frame.data;
  if (typeof value !== "object" || value === null || Array.isArray(value)) return null;
  const receipt = value as Record<string, JsonValue | undefined>;
  if (
    receipt.schema_version !== 1 ||
    !actionStatusId(receipt.status) ||
    receipt.commit !== frame.commit
  ) {
    return null;
  }
  const rawResult = receipt.result;
  if (rawResult === undefined || rawResult === null) {
    return { status: receipt.status, commit: frame.commit, result: null };
  }
  if (typeof rawResult !== "object" || Array.isArray(rawResult)) return null;
  const result = rawResult as Record<string, JsonValue | undefined>;
  if (
    !canonicalId(result.kind) ||
    typeof result.summary !== "string" ||
    result.summary.length === 0 ||
    new TextEncoder().encode(result.summary).length > MAX_ACTION_SUMMARY_BYTES ||
    CONTROL_CHARACTER.test(result.summary) ||
    result.data === undefined
  ) {
    return null;
  }
  return {
    status: receipt.status,
    commit: frame.commit,
    result: {
      kind: result.kind,
      summary: result.summary,
      data: result.data,
    },
  };
}

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
