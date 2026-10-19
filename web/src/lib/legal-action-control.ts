import type { LegalActionParameters, LegalActionRequest } from "./proto";
import type { LegalAction } from "./stores/actions.svelte";

export type ActionField = { id: string; required: boolean } & (
  | { kind: "boolean" }
  | { kind: "integer"; minimum: number | null; maximum: number | null }
  | { kind: "text"; maximumBytes: number }
  | { kind: "choice"; choices: string[] }
);
export interface ActionControl { offer: string; fields: ActionField[] }
const canonical = (value: unknown): value is string => typeof value === "string"
  && value.length > 0 && value.length <= 96 && /^[a-z0-9_]+(?:\.[a-z0-9_]+)*$/.test(value);
const object = (value: unknown): value is Record<string, unknown> => typeof value === "object"
  && value !== null && !Array.isArray(value);
const bytes = (value: string): number => new TextEncoder().encode(value).length;
const text = (value: unknown, max: number): value is string => typeof value === "string"
  && bytes(value) <= max && !/[\u0000-\u001f\u007f-\u009f]/u.test(value);

/** Only the exact currently implemented self/no-target projection is interactive. */
export function actionControl(action: LegalAction): ActionControl | null {
  const raw = action.projected;
  if (!/^[a-f0-9]{64}$/.test(action.offer) || !object(raw.subject)
    || raw.subject.kind !== "self_actor" || !object(raw.targets) || raw.targets.kind !== "none"
    || !Array.isArray(raw.parameters) || raw.parameters.length > 32) return null;
  const fields: ActionField[] = [];
  const seen = new Set<string>();
  for (const value of raw.parameters) {
    if (!object(value) || !canonical(value.id) || seen.has(value.id)
      || typeof value.required !== "boolean" || !object(value.kind)) return null;
    seen.add(value.id);
    const base = { id: value.id, required: value.required };
    const kind = value.kind;
    switch (kind.kind) {
      case "boolean": fields.push({ ...base, kind: "boolean" }); break;
      case "integer":
        if (typeof kind.minimum !== "number" || !Number.isInteger(kind.minimum)
          || typeof kind.maximum !== "number" || !Number.isInteger(kind.maximum)
          || kind.minimum > kind.maximum) return null;
        // The projection predates exact integer text. Never turn a rounded
        // JSON boundary back into an allegedly exact limit; core rechecks it.
        fields.push({ ...base, kind: "integer",
          minimum: Number.isSafeInteger(kind.minimum) ? kind.minimum : null,
          maximum: Number.isSafeInteger(kind.maximum) ? kind.maximum : null });
        break;
      case "text":
        if (typeof kind.maximum_bytes !== "number" || !Number.isInteger(kind.maximum_bytes)
          || kind.maximum_bytes < 1 || kind.maximum_bytes > 4096) return null;
        fields.push({ ...base, kind: "text", maximumBytes: kind.maximum_bytes }); break;
      case "choice":
        if (!Array.isArray(kind.choices) || kind.choices.length < 1 || kind.choices.length > 256
          || !kind.choices.every(choice => text(choice, 4096) && choice.length > 0)
          || new Set(kind.choices).size !== kind.choices.length) return null;
        fields.push({ ...base, kind: "choice", choices: kind.choices as string[] }); break;
      default: return null;
    }
  }
  return { offer: action.offer, fields };
}

/** Exact scalar text is never evaluated as a command or converted via Number. */
export function submitSelf(control: ActionControl, values: Record<string, string>): LegalActionRequest {
  const parameters: LegalActionParameters = Object.create(null);
  for (const field of control.fields) {
    const supplied = Object.hasOwn(values, field.id);
    const value = typeof values[field.id] === "string" ? values[field.id]! : "";
    if (field.kind === "text") {
      // Required means key presence. Optional text distinguishes an untouched
      // input from a user explicitly clearing it to the valid empty string.
      if (!field.required && !supplied) continue;
    } else {
      if (!value && !field.required) continue;
      if (!value && field.required) throw new Error(`Enter ${field.id.replaceAll("_", " ")}.`);
    }
    switch (field.kind) {
      case "integer": {
        if (!/^(?:0|-?[1-9][0-9]{0,18})$/.test(value)) throw new Error("Enter a whole number without spaces or leading zeros.");
        const exact = BigInt(value);
        if (exact < -9223372036854775808n || exact > 9223372036854775807n
          || (field.minimum !== null && exact < BigInt(field.minimum))
          || (field.maximum !== null && exact > BigInt(field.maximum))) throw new Error("That number is outside the allowed range.");
        parameters[field.id] = { kind: "integer", value }; break;
      }
      case "boolean":
        if (value !== "true" && value !== "false") throw new Error("Choose Yes or No.");
        parameters[field.id] = { kind: "boolean", value: value === "true" }; break;
      case "choice":
        if (!field.choices.includes(value)) throw new Error("Choose an available option.");
        parameters[field.id] = { kind: "text", value }; break;
      case "text":
        if (!text(value, field.maximumBytes)) throw new Error("Text is too long or contains a control character.");
        parameters[field.id] = { kind: "text", value }; break;
    }
  }
  if (bytes(JSON.stringify(parameters)) > 65536) throw new Error("These inputs are too large.");
  return { op: "submit_self", offer: control.offer, parameters };
}
