// @vitest-environment jsdom
import { fireEvent, render, screen, within } from "@testing-library/svelte";
import { tick } from "svelte";
import { describe, expect, it, vi } from "vitest";
import ActionsPanel from "./ActionsPanel.svelte";
import { ActionRequestsStore } from "$lib/stores/action-requests.svelte";
import { LegalActionsStore, type LegalAction } from "$lib/stores/actions.svelte";
import type { GameApp } from "$lib/game.svelte";

describe("persistent transaction controls", () => {
  it("retains returned-reference status after offers disappear and disables unsupported targets", async () => {
    const reference = `k5:market:purchase:0000000000000000:${"c".repeat(32)}`;
    const request = vi.fn().mockResolvedValueOnce({ state: "owned", reference }).mockResolvedValue({ state: "pending", reference });
    const requests = new ActionRequestsStore(request);
    const action: LegalAction = { offer: "a".repeat(64), presentation: { verb: "purchase", label: "Buy", help: "" },
      projected: { subject: { kind: "self_actor" }, targets: { kind: "none" }, parameters: [] } };
    await requests.submit(action, {});
    const actions = new LegalActionsStore();
    actions.loaded = true;
    actions.actions = [{ ...action, offer: "b".repeat(64), presentation: { ...action.presentation, label: "Unsupported" },
      projected: { ...action.projected, targets: { kind: "choose" } } }];
    const onCmd = vi.fn();
    const app = { actions, actionRequests: requests, refreshLegalActions: vi.fn(), submitLegalAction: vi.fn() } as unknown as GameApp;
    render(ActionsPanel, { props: { app, onCmd } });
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
