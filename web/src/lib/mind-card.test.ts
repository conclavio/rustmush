import { describe, expect, it } from "vitest";
import { cardValueText, decodeMindCard, parseCardJson, validCardValue, type Widget } from "./mind-card";

const slider: Widget = { kind: "slider", min: -100, max: 1000, step: 5, scale: 10, unit: "%", help: "" };
const sample = () => ({ schema: { sections: [{ id: "fire", label: "Fire", station: "fire", fields: [
  { id: "min_odds", label: "Minimum odds", widget: slider },
  { id: "facets", label: "Aspects", widget: { kind: "facet_set", help: "Earned aspects" } },
] }] }, values: { min_odds: 625, facets: ["front", "rear"] } });

describe("general mind cards", () => {
  it("shows scaled integer ticks exactly and admits only the same slider grid", () => {
    expect(cardValueText(slider, 625)).toBe("62.5 %");
    expect(cardValueText(slider, -5)).toBe("-0.5 %");
    expect(cardValueText({ ...slider, scale: 1000 }, 5)).toBe("0.005 %");
    expect(validCardValue(slider, 625)).toBe(true);
    expect(validCardValue(slider, 626)).toBe(false);
    expect(validCardValue(slider, 625.1)).toBe(false);
  });
  it("keeps exact safe endpoints when a slider crosses the entire signed client range", () => {
    const wide: Widget = { ...slider, min: -9007199254740991, max: 9007199254740991, step: 7 };
    expect(validCardValue(wide, 9007199254740991)).toBe((18014398509481982n % 7n) === 0n);
  });
  it("refuses malformed whole cards instead of rewriting some fields", () => {
    expect(decodeMindCard(sample())?.values.min_odds).toBe(625);
    const missing = sample(); delete (missing.values as Record<string, unknown>).facets;
    expect(decodeMindCard(missing)).toBeNull();
    const duplicate = sample(); duplicate.schema.sections[0]!.fields.push(duplicate.schema.sections[0]!.fields[0]!);
    expect(decodeMindCard(duplicate)).toBeNull();
    const unsorted = sample(); unsorted.values.facets.reverse();
    expect(decodeMindCard(unsorted)).toBeNull();
  });
  it("refuses lossy JSON tokens before parsing while preserving quoted source-like labels", () => {
    expect(parseCardJson('{"n":9007199254740991,"label":"1.0000000000000001 1e10"}')).toEqual({ n: 9007199254740991, label: "1.0000000000000001 1e10" });
    for (const text of ['{"n":1.0000000000000001}', '{"n":1e3}', '{"n":9007199254740992}', '{"n":-9007199254740992}']) {
      expect(() => parseCardJson(text)).toThrow("exact integers");
    }
    expect(() => parseCardJson('{"n":}')).toThrow();
  });
});
