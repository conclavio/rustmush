// @vitest-environment jsdom
import { fireEvent, render, screen, within } from "@testing-library/svelte";
import { tick } from "svelte";
import { describe, expect, it, vi } from "vitest";
import ActionsPanel from "./ActionsPanel.svelte";
import { ActionRequestsStore } from "$lib/stores/action-requests.svelte";
import { LegalActionsStore, type LegalAction } from "$lib/stores/actions.svelte";
import type { GameApp } from "$lib/game.svelte";

describe("persistent transaction controls", () => {
  it("retains returned-reference status after offers disappear, hands an aimed offer to +act and disables what it cannot read", async () => {
    const reference = `k5:market:purchase:0000000000000000:${"c".repeat(32)}`;
    const request = vi.fn().mockResolvedValueOnce({ state: "owned", reference }).mockResolvedValue({ state: "pending", reference });
    const requests = new ActionRequestsStore(request);
    const action: LegalAction = { offer: "a".repeat(64), presentation: { verb: "purchase", label: "Buy", help: "" },
      projected: { subject: { kind: "self_actor" }, targets: { kind: "none" }, parameters: [] } };
    await requests.submit(action, {});
    const actions = new LegalActionsStore();
    actions.loaded = true;
    actions.actions = [{ ...action, offer: "b".repeat(64), presentation: { ...action.presentation, label: "Aimed" },
      projected: { ...action.projected, targets: { kind: "choose" } } },
      { ...action, offer: "d".repeat(64), presentation: { ...action.presentation, label: "Unsupported" },
      projected: { ...action.projected, parameters: "nonsense" } }];
    const onCmd = vi.fn();
    const focusComposer = vi.fn();
    const app = { actions, actionRequests: requests, refreshLegalActions: vi.fn(), submitLegalAction: vi.fn(), focusComposer } as unknown as GameApp;
    render(ActionsPanel, { props: { app, onCmd } });
    // An aimed offer is a live button that prefills the verb, as the TUI's `A` does.
    const aimed = screen.getByRole("button", { name: /Aimed/ }) as HTMLButtonElement;
    expect(aimed.disabled).toBe(false);
    expect(within(aimed).getByText("This action needs a target; use +act.")).toBeTruthy();
    await fireEvent.click(aimed);
    expect(focusComposer).toHaveBeenCalledExactlyOnceWith("+act purchase");
    // One this client cannot read at all stays a dead button.
    expect((screen.getByRole("button", { name: /Unsupported/ }) as HTMLButtonElement).disabled).toBe(true);
    expect(onCmd).not.toHaveBeenCalled();
    actions.actions = []; await tick();
    expect(screen.getByText("No actions are currently available.")).toBeTruthy();
    const transactions = screen.getByRole("region", { name: "Recent transactions" });
    expect(within(transactions).getByText(/even if the available offers change/)).toBeTruthy();
    await fireEvent.click(within(transactions).getByRole("button", { name: "Check transaction status" }));
    await tick();
    expect(request).toHaveBeenLastCalledWith({ op: "status", reference });
    expect(request).toHaveBeenCalledTimes(2);
    expect(within(transactions).getByText("Transaction is still pending.")).toBeTruthy();
  });
});
