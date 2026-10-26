import { afterEach, describe, expect, it, vi } from "vitest";
import { GameApp } from "./game.svelte";
import type { WebSocketLike } from "./ws";

class Socket implements WebSocketLike {
  readyState = 0;
  sent: string[] = [];
  onopen: (() => void) | null = null;
  onclose: ((event: { code: number; wasClean: boolean }) => void) | null = null;
  onerror: (() => void) | null = null;
  onmessage: ((event: { data: unknown }) => void) | null = null;
  send(value: string): void { this.sent.push(value); }
  close(): void { this.readyState = 3; this.onclose?.({ code: 1000, wasClean: true }); }
  open(): void { this.readyState = 1; this.onopen?.(); }
  deliver(t: string, d: unknown): void { this.onmessage?.({ data: JSON.stringify({ seq: 0, ts: 1, t, d }) }); }
  invokes(): { id: number; request: unknown }[] {
    return this.sent.flatMap(value => { const frame = JSON.parse(value); return frame.t === "invoke" ? [frame.d] : []; });
  }
}

afterEach(() => vi.useRealTimers());
describe("GameApp typed action dispatch", () => {
  it("uses the current exact offer, correlates the result and clears requests across character changes", async () => {
    vi.useFakeTimers();
    const socket = new Socket();
    const app = new GameApp({ url: "ws://test/ws", webSocketFactory: () => socket });
    app.client.connect("token", "7"); socket.open();
    socket.deliver("hello", { proto: 1, server: "test", features: [], panels: ["legal.actions"] });
    const offer = "a".repeat(64);
    const shown = { offer, subject: { kind: "self_actor" }, targets: { kind: "none" }, parameters: [],
      presentation: { verb: "shared", label: "Action", help: "" } };
    socket.deliver("view", { view: "legal.actions", schema_version: 1, generation: 1, sequence: 1, data: [shown] });
    const action = app.actions.actions[0]!;
    expect(action.offer).toBe(offer);
    const submitted = app.submitLegalAction(action, {});
    const frame = socket.invokes()[0]!;
    expect(frame.request).toEqual({ service: "legal_action", request: { op: "submit_self", offer, parameters: {} } });
    const reference = `k5:market:purchase:0000000000000000:${"c".repeat(32)}`;
    socket.deliver("invoke_result", { id: frame.id, result: { service: "legal_action", result: { state: "owned", reference } } });
    await submitted;
    await app.submitLegalAction(action, {});
    expect(socket.invokes()).toHaveLength(1);

    const next = { ...shown, offer: "b".repeat(64) };
    socket.deliver("view", { view: "legal.actions", schema_version: 1, generation: 1, sequence: 2, data: [next] });
    await app.submitLegalAction(action, {});
    expect(app.actionRequests.notice).toContain("changed");
    expect(socket.invokes()).toHaveLength(1);
    const status = app.actionRequests.status(action.offer);
    const lookup = socket.invokes()[1]!;
    expect(lookup.request).toEqual({ service: "legal_action", request: { op: "status", reference } });
    socket.deliver("invoke_result", { id: lookup.id, result: { service: "legal_action", result: { state: "pending", reference } } });
    await status;
    expect(app.actionRequests.entries[action.offer]!.reference).toBe(reference);
    const pending = app.submitLegalAction(app.actions.actions[0]!, {});
    const second = socket.invokes()[2]!;
    socket.deliver("session_info", { character: { id: "8", name: "Other" }, account: "test", connections: [] });
    await pending;
    expect(app.actionRequests.entries).toEqual({});
    socket.deliver("invoke_result", { id: second.id, result: { service: "legal_action", result: { state: "owned",
      reference: `k5:market:purchase:0000000000000000:${"c".repeat(32)}` } } });
    expect(app.actionRequests.entries).toEqual({});
    vi.advanceTimersByTime(31_000);
    expect(socket.invokes()).toHaveLength(3);
    app.client.close();
  });
});
