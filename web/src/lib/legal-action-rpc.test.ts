import { afterEach, describe, expect, it, vi } from "vitest";
import { actionResponse, LegalActionRpcClient } from "./legal-action-rpc";
import type { WsClient } from "./ws";
import type { LegalActionResponse } from "./proto";
const status = `k5:market:purchase:0000000000000000:${"a".repeat(32)}`;
const request = { op: "status" as const, reference: status };
function rig() {
  let id = 0;
  let receive: (value: unknown) => void = () => {};
  const transport = { on: (_: string, callback: typeof receive) => { receive = callback; }, invoke: vi.fn(() => ++id) };
  const rpc = new LegalActionRpcClient(transport as unknown as WsClient);
  const result = (id: number, result: LegalActionResponse, service = "legal_action") => receive({ d: { id, result: { service, result } } });
  return { rpc, transport, result, malformed: (value: unknown) => receive(value) };
}
afterEach(() => vi.useRealTimers());
describe("typed action correlation", () => {
  it("correlates one result and never interprets queue custody as commitment", async () => {
    vi.useFakeTimers();
    const { rpc, transport, result, malformed } = rig();
    let done = false;
    const waiting = rpc.request(request).then(value => { done = true; return value; });
    malformed({ d: null }); malformed({ d: { id: 1, result: null } });
    result(2, { state: "committed", reference: status, commit: "b".repeat(32) });
    result(1, { state: "queued" }, "mind");
    await Promise.resolve(); expect(done).toBe(false);
    result(1, { state: "queued" });
    expect(await waiting).toEqual({ state: "queued" });
    vi.advanceTimersByTime(60_000);
    expect(transport.invoke).toHaveBeenCalledTimes(1);
  });
  it("bounds pending work, treats timeout as uncertain and ignores prior-connection replies", async () => {
    vi.useFakeTimers();
    const { rpc, transport, result } = rig();
    const old = Array.from({ length: 8 }, () => rpc.request(request).catch(error => error.message));
    await expect(rpc.request(request)).rejects.toThrow("Too many");
    vi.advanceTimersByTime(30_000);
    expect((await Promise.all(old)).every(value => value.includes("No completion"))).toBe(true);
    expect(transport.invoke).toHaveBeenCalledTimes(8);
    const lost = rpc.request(request).catch(error => error.message);
    rpc.clear(); expect(await lost).toContain("does not prove");
    const fresh = rpc.request(request);
    result(9, { state: "committed", reference: status, commit: "b".repeat(32) });
    result(10, { state: "pending", reference: status });
    expect(await fresh).toEqual({ state: "pending", reference: status });
  });
  it("refuses malformed output rather than claiming a committed or absent action", () => {
    expect(actionResponse({ state: "committed", reference: status, commit: "short" })).toBeNull();
    expect(actionResponse({ state: "owned", reference: "unknown" })).toBeNull();
    expect(actionResponse({ state: "queued", actor: 1 })).toBeNull();
    expect(actionResponse({ state: "unavailable" })).toBeNull();
    expect(actionResponse({ state: "unavailable", reference: status })).toEqual({ state: "unavailable", reference: status });
  });
});
