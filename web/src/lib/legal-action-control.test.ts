import { describe, expect, it } from "vitest";
import { actionControl, submitSelf } from "./legal-action-control";
import type { LegalAction } from "./stores/actions.svelte";

function action(parameters: unknown[], offer = "a".repeat(64)): LegalAction {
  return { offer, presentation: { verb: "shared-verb", label: "File", help: "" },
    projected: { subject: { kind: "self_actor" }, targets: { kind: "none" }, parameters } };
}
const integer = { id: "payer", required: true, kind: { kind: "integer", minimum: 0, maximum: 9223372036854775807 } };

describe("typed projected action controls", () => {
  it("allows required empty text and distinguishes omitted optional text from explicit clearing", () => {
    const control = actionControl(action([
      { id: "required", required: true, kind: { kind: "text", maximum_bytes: 8 } },
      { id: "optional", required: false, kind: { kind: "text", maximum_bytes: 8 } },
    ]))!;
    expect(submitSelf(control, {})).toMatchObject({ parameters: { required: { kind: "text", value: "" } } });
    const untouched = submitSelf(control, {});
    expect(untouched.op === "submit_self" && Object.hasOwn(untouched.parameters, "optional")).toBe(false);
    expect(submitSelf(control, { required: "", optional: "" })).toMatchObject({ parameters: {
      required: { kind: "text", value: "" }, optional: { kind: "text", value: "" },
    } });
  });
  it("preserves exact integer input and chooses the displayed offer even with duplicate verbs", () => {
    const first = actionControl(action([integer]))!;
    const second = actionControl(action([integer], "b".repeat(64)))!;
    const values = { payer: "9223372036854775807" };
    expect(submitSelf(first, values)).toEqual({ op: "submit_self", offer: "a".repeat(64), parameters: { payer: { kind: "integer", value: values.payer } } });
    expect(submitSelf(second, values)).toMatchObject({ offer: "b".repeat(64) });
    for (const payer of ["9223372036854775808", "01", "1.0", "1e3", "-0", "+1", " 1"]) {
      expect(() => submitSelf(first, { payer })).toThrow();
    }
    expect(() => submitSelf(first, { payer: "-1" })).toThrow();
  });
  it("rejects unsupported subjects, malformed schemas and ambiguous fields", () => {
    const foreign = action([]);
    foreign.projected.subject = { kind: "known_object", id: 1, revision: 1 };
    expect(actionControl(foreign)).toBeNull();
    expect(actionControl(action([integer, integer]))).toBeNull();
    expect(actionControl(action([{ ...integer, kind: { kind: "integer", minimum: 1, maximum: Infinity } }]))).toBeNull();
    expect(actionControl(action([], "not-an-offer"))).toBeNull();
    expect(actionControl(action([{ ...integer, id: "bad..name" }]))).toBeNull();
  });
  it("uses exact bounded strings, optional omission and false without command interpolation", () => {
    const control = actionControl(action([
      { id: "name", required: true, kind: { kind: "text", maximum_bytes: 8 } },
      { id: "choice", required: true, kind: { kind: "choice", choices: ["Open", "Sealed"] } },
      { id: "renew", required: true, kind: { kind: "boolean" } },
      { id: "optional", required: false, kind: { kind: "text", maximum_bytes: 8 } },
    ]))!;
    const inputs = { name: "a; +act", choice: "Sealed", renew: "false" };
    expect(submitSelf(control, inputs)).toMatchObject({ parameters: {
      name: { kind: "text", value: "a; +act" }, choice: { kind: "text", value: "Sealed" }, renew: { kind: "boolean", value: false },
    } });
    expect(submitSelf(control, inputs).op).toBe("submit_self");
    expect(() => submitSelf(control, { ...inputs, name: "a\n" })).toThrow();
    expect(() => submitSelf(control, { ...inputs, name: "é".repeat(5) })).toThrow();
    expect(() => submitSelf(control, { ...inputs, choice: "Other" })).toThrow();
    expect(() => submitSelf(control, { ...inputs, renew: "yes" })).toThrow();
  });
});
