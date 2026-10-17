/** Closed, bounded card projection shared by the sealed cockpit and workshop. */
export type CardValue = number | boolean | string | string[];
export type Widget =
  | { kind: "slider"; min: number; max: number; step: number; scale: number; unit: string; help: string }
  | { kind: "choice"; choices: string[]; help: string }
  | { kind: "toggle" | "bearing" | "facet_set" | "contact_class_set"; help: string };
export interface CardField { id: string; label: string; widget: Widget }
export interface CardSection { id: string; label: string; station: string; fields: CardField[] }
export interface MindCard { sections: CardSection[]; values: Record<string, CardValue> }

/** Reject fractions/unsafe integers before JSON.parse can round the user's text. */
export function parseCardJson(text: string): unknown {
  let quoted = false;
  for (let at = 0; at < text.length; at++) {
    const ch = text[at]!;
    if (quoted) {
      if (ch === "\\") at++;
      else if (ch === '"') quoted = false;
    } else if (ch === '"') quoted = true;
    else if (ch === "-" || /\d/.test(ch)) {
      const token = text.slice(at).match(/^-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?/)?.[0];
      if (token) {
        if (/[.eE]/.test(token) || BigInt(token) > 9007199254740991n || BigInt(token) < -9007199254740991n)
          throw new Error("Card numbers must be exact integers between -9007199254740991 and 9007199254740991.");
        at += token.length - 1;
      }
    }
  }
  return JSON.parse(text);
}

function object(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
function text(value: unknown, max: number): string | null {
  return typeof value === "string" && value.length <= max && !/[\u0000-\u001f\u007f]/.test(value)
    ? value : null;
}
function integer(value: unknown): value is number { return Number.isSafeInteger(value); }
function words(value: unknown, max: number): string[] | null {
  if (!Array.isArray(value) || value.length > max) return null;
  const entries = value.map(word => text(word, 64));
  return entries.every((word): word is string => word !== null) && new Set(entries).size === entries.length
    ? entries : null;
}
function widget(raw: unknown): Widget | null {
  if (!object(raw)) return null;
  const help = text(raw.help, 1024);
  if (help === null) return null;
  switch (raw.kind) {
    case "slider": {
      const unit = text(raw.unit, 32);
      if (!integer(raw.min) || !integer(raw.max) || !integer(raw.step) || raw.min > raw.max || raw.step <= 0
        || ![1, 10, 100, 1000].includes(Number(raw.scale)) || unit === null) return null;
      return { kind: "slider", min: raw.min, max: raw.max, step: raw.step, scale: Number(raw.scale), unit, help };
    }
    case "choice": {
      const choices = words(raw.choices, 32);
      return choices?.length ? { kind: "choice", choices, help } : null;
    }
    case "toggle": case "bearing": case "facet_set": case "contact_class_set":
      return { kind: raw.kind, help };
    default: return null;
  }
}

export function validCardValue(widget: Widget, value: unknown): value is CardValue {
  switch (widget.kind) {
    case "slider": return integer(value) && value >= widget.min && value <= widget.max
      && (BigInt(value) - BigInt(widget.min)) % BigInt(widget.step) === 0n;
    case "bearing": return integer(value) && value >= 0 && value <= 359;
    case "choice": return typeof value === "string" && widget.choices.includes(value);
    case "toggle": return typeof value === "boolean";
    case "facet_set": case "contact_class_set": {
      const allowed = widget.kind === "facet_set" ? ["front", "left", "rear", "right", "top"]
        : ["friendly", "hostile", "unknown"];
      const set = words(value, allowed.length);
      return set !== null && set.every((word, at) => allowed.includes(word) && (at === 0 || set[at - 1]! < word));
    }
  }
}

/** Refuse a malformed whole card; never invent defaults or partially rewrite a draft. */
export function decodeMindCard(raw: unknown): MindCard | null {
  if (!object(raw) || !object(raw.schema) || !object(raw.values) || !Array.isArray(raw.schema.sections)
    || raw.schema.sections.length > 16) return null;
  const sections: CardSection[] = [];
  const ids = new Set<string>();
  const values: Record<string, CardValue> = Object.create(null);
  for (const section of raw.schema.sections) {
    if (!object(section)) return null;
    const id = text(section.id, 64), label = text(section.label, 128);
    if (!id || label === null || !["helm", "sensors", "fire"].includes(String(section.station))
      || !Array.isArray(section.fields) || section.fields.length > 64) return null;
    const fields: CardField[] = [];
    for (const field of section.fields) {
      if (!object(field)) return null;
      const id = text(field.id, 64), label = text(field.label, 128), control = widget(field.widget);
      if (!id || !/^[a-z][a-z0-9_]*$/.test(id) || ids.has(id) || ids.size >= 64 || label === null || !control) return null;
      const value = raw.values[id];
      if (!validCardValue(control, value)) return null;
      ids.add(id);
      fields.push({ id, label, widget: control });
      values[id] = Array.isArray(value) ? [...value] : value;
    }
    sections.push({ id, label, station: String(section.station), fields });
  }
  return { sections, values };
}

/** Integer division display avoids slider rounding while retaining its exact stored ticks. */
export function cardValueText(control: Widget, value: CardValue): string {
  if (control.kind === "slider" && typeof value === "number") {
    const negative = value < 0 ? "-" : "";
    const ticks = BigInt(value < 0 ? -value : value), scale = BigInt(control.scale);
    const decimals = Math.log10(control.scale);
    const rest = String(ticks % scale).padStart(decimals, "0").replace(/0+$/, "");
    return `${negative}${ticks / scale}${rest ? `.${rest}` : ""}${control.unit ? ` ${control.unit}` : ""}`;
  }
  if (control.kind === "bearing") return `${value}°`;
  if (typeof value === "boolean") return value ? "on" : "off";
  return Array.isArray(value) ? value.join(", ") || "none" : String(value);
}
