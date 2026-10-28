/** DT.1b: only server-projected own-roster knowledge and exact command prefills. */
export interface DetachmentSlot {
  slot: number;
  callsign: string;
  pilot: string | null;
  controller: "player" | "npc" | "automated";
  order_prefill: string;
}

export interface Detachment {
  id: string;
  revision: string;
  term: string;
  leader_slot: number | null;
  slots: DetachmentSlot[];
  all_order_prefill: string;
  my_order: { sequence: string; from: string; objective: string; response: "wilco" | "negative" | null } | null;
  wilco_command: string | null;
  negative_command: string | null;
}

const record = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);
const counter = (value: unknown): value is string => typeof value === "string"
  && /^(0|[1-9][0-9]{0,19})$/.test(value) && BigInt(value) <= 18446744073709551615n;
const label = (value: unknown, max: number): value is string => typeof value === "string"
  && value.length > 0 && Array.from(value).length <= max && !/[\u0000-\u001f\u007f-\u009f\u2028\u2029]/u.test(value);

/** Invalid projected commands disable this whole surface; never trim or repair them. */
export function decodeDetachment(value: unknown): Detachment | null {
  if (!record(value) || !counter(value.id) || !counter(value.revision) || !counter(value.term)
    || !Array.isArray(value.slots) || value.slots.length === 0 || value.slots.length > 6) return null;
  const prefix = `ORDER DT:${value.id}:${value.revision}:${value.term} `;
  if (value.all_order_prefill !== `${prefix}ALL `) return null;
  const slots: DetachmentSlot[] = [];
  const seen = new Set<number>();
  for (const raw of value.slots) {
    if (!record(raw) || typeof raw.slot !== "number" || !Number.isInteger(raw.slot)
      || raw.slot < 1 || raw.slot > 6 || seen.has(raw.slot)
      || !label(raw.callsign, 32) || (raw.pilot !== null && !label(raw.pilot, 96))
      || typeof raw.controller !== "string" || !["player", "npc", "automated"].includes(raw.controller)
      || raw.order_prefill !== `${prefix}${raw.slot} `) return null;
    seen.add(raw.slot);
    slots.push({ slot: raw.slot, callsign: raw.callsign, pilot: raw.pilot,
      controller: raw.controller as DetachmentSlot["controller"], order_prefill: raw.order_prefill });
  }
  if (value.leader_slot !== null && (typeof value.leader_slot !== "number" || !seen.has(value.leader_slot))) return null;
  let my_order: Detachment["my_order"] = null;
  if (value.my_order !== null) {
    const order = value.my_order;
    if (!record(order) || !counter(order.sequence) || !label(order.from, 96)
      || !label(order.objective, 256) || ![null, "wilco", "negative"].includes(order.response as null | string)) return null;
    my_order = { sequence: order.sequence, from: order.from, objective: order.objective,
      response: order.response as "wilco" | "negative" | null };
  }
  const reply = (verb: string) => my_order === null ? null : `${verb} DT:${value.id}:${value.revision} ${my_order.sequence}`;
  if (value.wilco_command !== reply("WILCO") || value.negative_command !== reply("NEGATIVE")) return null;
  return { id: value.id, revision: value.revision, term: value.term, leader_slot: value.leader_slot,
    slots: slots.sort((a, b) => a.slot - b.slot), all_order_prefill: value.all_order_prefill,
    my_order, wilco_command: value.wilco_command as string | null, negative_command: value.negative_command as string | null };
}

/** Undefined means an older server; null means a present, invalid projection. */
export function detachmentKeyPrefill(detachment: Detachment | null | undefined, key: number): string | null {
  if (!Number.isInteger(key) || key < 1 || key > 6) return null;
  if (detachment === undefined) return `ORDER ${key === 1 ? "ALL" : key} `;
  return detachment?.slots.find(slot => slot.slot === key)?.order_prefill ?? null;
}

export function detachmentLines(detachment: Detachment | null | undefined): string[] {
  if (!detachment) return [];
  const lines = [`Leader: ${detachment.leader_slot === null ? "none" : `slot ${detachment.leader_slot}`}`];
  for (const slot of detachment.slots) {
    lines.push(`${slot.slot} ${slot.callsign} — ${slot.pilot ?? slot.controller}${slot.slot === detachment.leader_slot ? " (leader)" : ""}`);
  }
  if (detachment.my_order) {
    lines.push(`Order from ${detachment.my_order.from}: ${detachment.my_order.objective}`);
    lines.push(`Response: ${detachment.my_order.response?.toUpperCase() ?? "not acknowledged"}`);
  }
  return lines;
}
