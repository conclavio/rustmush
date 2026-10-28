import { describe, expect, it } from "vitest";
import { decodeDetachment, detachmentKeyPrefill, detachmentLines } from "./detachment";
import { decodeCockpitFrame } from "./cockpit";

function detachmentFixture() {
  return {
    id: "18446744073709551615", revision: "2", term: "3", leader_slot: 2,
    slots: [
      { slot: 1, callsign: "RA", pilot: null, controller: "automated", order_prefill: "ORDER DT:18446744073709551615:2:3 1 " },
      { slot: 2, callsign: "RB", pilot: "Aster", controller: "player", order_prefill: "ORDER DT:18446744073709551615:2:3 2 " },
    ],
    all_order_prefill: "ORDER DT:18446744073709551615:2:3 ALL ",
    my_order: { sequence: "9", from: "Aster", objective: "hold", response: null },
    wilco_command: "WILCO DT:18446744073709551615:2 9",
    negative_command: "NEGATIVE DT:18446744073709551615:2 9",
  };
}

describe("detachment cockpit projection", () => {
  it("retains exact counter strings, stable slots and separate acknowledgement", () => {
    const d = decodeDetachment(detachmentFixture());
    expect(d).not.toBeNull();
    expect(d?.id).toBe("18446744073709551615");
    expect(detachmentLines(d)).toEqual([
      "Leader: slot 2", "1 RA — automated", "2 RB — Aster (leader)",
      "Order from Aster: hold", "Response: not acknowledged",
    ]);
    expect(detachmentKeyPrefill(d, 1)).toBe("ORDER DT:18446744073709551615:2:3 1 ");
    expect(detachmentKeyPrefill(d, 2)).toBe("ORDER DT:18446744073709551615:2:3 2 ");
    expect(detachmentKeyPrefill(d, 3)).toBeNull();
  });

  it("preserves old keys only for a missing projection", () => {
    expect(detachmentKeyPrefill(undefined, 1)).toBe("ORDER ALL ");
    expect(detachmentKeyPrefill(undefined, 2)).toBe("ORDER 2 ");
    expect(detachmentKeyPrefill(null, 1)).toBeNull();
    expect(decodeCockpitFrame({ seated: true })?.detachment).toBeUndefined();
    expect(decodeCockpitFrame({ seated: true, detachment: {} })?.detachment).toBeNull();
  });

  it("accepts the server's character-bounded multibyte labels", () => {
    const d = detachmentFixture();
    const name = "🛠".repeat(96);
    const projected = decodeDetachment({ ...d,
      slots: d.slots.map(slot => ({ ...slot, pilot: name })),
      my_order: { ...d.my_order, from: name },
    });
    expect(projected?.my_order?.from).toBe(name);
    expect(projected?.slots[0]?.pilot).toBe(name);
  });

  it("rejects malformed counters, spoofed prefills and invalid recipient replies", () => {
    for (const patch of [
      { id: "01" }, { term: "18446744073709551616" }, { revision: 2 }, { leader_slot: 6 },
      { all_order_prefill: "ORDER ALL \nsay injected" }, { wilco_command: "WILCO DT:18446744073709551615:2 8" },
      { my_order: { sequence: "9", from: "Aster", objective: "hold", response: "completed" } },
    ]) {
      expect(decodeDetachment({ ...detachmentFixture(), ...patch })).toBeNull();
    }
    const d = detachmentFixture();
    expect(decodeDetachment({ ...d, slots: [d.slots[0], d.slots[0]] })).toBeNull();
    expect(decodeDetachment({ ...d, slots: [{ ...d.slots[0], order_prefill: "ORDER DT:18446744073709551615:2:2 1 " }] })).toBeNull();
    expect(decodeDetachment({ ...d, slots: [{ ...d.slots[0], controller: "unknown" }] })).toBeNull();
    expect(decodeDetachment({ ...d, slots: [{ ...d.slots[0], controller: { toString: "player" } }] })).toBeNull();
  });
});
