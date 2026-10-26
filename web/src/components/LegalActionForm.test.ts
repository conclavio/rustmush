// @vitest-environment jsdom
import { fireEvent, render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import { describe, expect, it, vi } from "vitest";
import LegalActionForm from "./LegalActionForm.svelte";
import { actionControl } from "$lib/legal-action-control";
import { ActionRequestsStore } from "$lib/stores/action-requests.svelte";
import type { LegalAction } from "$lib/stores/actions.svelte";
describe("actual typed action form", () => {
  it("submits the exact large integer and displayed token, then locks an uncertain result", async () => {
    const action: LegalAction = { offer: "a".repeat(64), presentation: { verb: "shared", label: "File mind", help: "File the selected revision." },
      projected: { subject: { kind: "self_actor" }, targets: { kind: "none" }, parameters: [
        { id: "payer", required: true, kind: { kind: "integer", minimum: 0, maximum: 9223372036854775807 } },
      ] } };
    const request = vi.fn().mockResolvedValue({ state: "unavailable", reference: null });
    const requests = new ActionRequestsStore(request);
    render(LegalActionForm, { props: { action, control: actionControl(action)!, requests, submit: values => requests.submit(action, values) } });
    await fireEvent.input(screen.getByLabelText("payer"), { target: { value: "9223372036854775807" } });
    await fireEvent.submit(screen.getByRole("form", { name: "File mind" }));
    await tick();
    expect(request).toHaveBeenCalledWith({ op: "submit_self", offer: action.offer, parameters: { payer: { kind: "integer", value: "9223372036854775807" } } });
    expect((screen.getByRole("button", { name: "File mind" }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByRole("status").textContent).toContain("does not mean");
    await fireEvent.submit(screen.getByRole("form", { name: "File mind" }));
    expect(request).toHaveBeenCalledTimes(1);
  });
});
