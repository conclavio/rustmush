import { afterEach, describe, expect, it, vi } from "vitest";
import { MindRpcClient, MindRpcUnsent } from "./mind-rpc";
import type { WsClient } from "./ws";
import type { MindResponse } from "./proto";

function rig() {
  let id = 0;
  let receive: (value: unknown) => void = () => {};
  const transport = { on: (_: string, callback: typeof receive) => { receive = callback; }, invoke: vi.fn(() => ++id) };
  const rpc = new MindRpcClient(transport as unknown as WsClient);
  const result = (id: number, result: MindResponse) => receive({ d: { id, result: { service: "mind", result } } });
  return { rpc, transport, result };
}
afterEach(() => vi.useRealTimers());
describe("connection-local mind requests", () => {
  it("correlates progress and completion without resolving a queued job", async () => {
    vi.useFakeTimers();
    const { rpc, result } = rig();
    const progress = vi.fn();
    let resolved = false;
    const waiting = rpc.request({ op: "list" }, progress).then(value => { resolved = true; return value; });
    result(1, { status: "queued", job: 9 });
    await Promise.resolve();
    expect(resolved).toBe(false);
    expect(progress).toHaveBeenCalledWith(9);
    result(2, { status: "library", names: ["foreign"] });
    expect(resolved).toBe(false);
    result(1, { status: "library", names: ["owned"] });
    expect(await waiting).toEqual({ status: "library", names: ["owned"] });
    expect(vi.getTimerCount()).toBe(0);
  });
  it("drops all callbacks on disconnect and ignores a late private reply", async () => {
    vi.useFakeTimers();
    const { rpc, result } = rig();
    const old = rpc.request({ op: "list" }).catch(error => error.message);
    rpc.clear();
    expect(await old).toContain("connection changed");
    const fresh = rpc.request({ op: "list" });
    result(1, { status: "library", names: ["old private name"] });
    result(2, { status: "library", names: ["new"] });
    expect(await fresh).toEqual({ status: "library", names: ["new"] });
  });
  it("bounds pending work and expires unanswered requests without replaying mutations", async () => {
    vi.useFakeTimers();
    const { rpc, transport } = rig();
    const requests = Array.from({ length: 8 }, () => rpc.request({ op: "list" }).catch(error => error.message));
    await expect(rpc.request({ op: "list" })).rejects.toThrow("Too many");
    expect(transport.invoke).toHaveBeenCalledTimes(8);
    vi.advanceTimersByTime(300_000);
    expect((await Promise.all(requests)).every(message => message.includes("timed out"))).toBe(true);
    expect(transport.invoke).toHaveBeenCalledTimes(8);
  });
  it("tells a request that never left from one whose answer was not seen (17g cut C3c)", async () => {
    vi.useFakeTimers();
    const { rpc, transport } = rig();
    transport.invoke.mockReturnValueOnce(null as unknown as number);
    const offline = await rpc.request({ op: "list" }).catch(error => error);
    expect(offline).toBeInstanceOf(MindRpcUnsent);
    expect(offline.reason).toBe("offline");
    const sent = Array.from({ length: 8 }, () => rpc.request({ op: "list" }).catch(error => error));
    const full = await rpc.request({ op: "list" }).catch(error => error);
    expect(full).toBeInstanceOf(MindRpcUnsent);
    expect(full.reason).toBe("full");
    // A request that did leave is never `MindRpcUnsent`, whether it timed out or was cleared.
    rpc.clear();
    expect((await Promise.all(sent)).some(error => error instanceof MindRpcUnsent)).toBe(false);
  });
});
