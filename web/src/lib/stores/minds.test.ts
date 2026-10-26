import { describe, expect, it, vi } from "vitest";
import { MindsStore, type MindRpc } from "./minds.svelte";
import type { MindResponse } from "../proto";

function read(): MindResponse {
  return { status: "read", name: "mine", revision: null, base: { edit: 3, revision: 1 },
    draft: { source: "fn think(view, card) {}", schema: { sections: [] }, profiles: { Line: {} }, rules: { Line: [] }, presets: {}, declared_class: "II", mind_abi: "test" },
    rules_text: { Line: [] }, rules_source: { Line: [] }, diagnostics: [] };
}
describe("owned mind working copies", () => {
  it("keeps edited source and exact base on a stale refusal, without retrying", async () => {
    const rpc = vi.fn<MindRpc>().mockResolvedValueOnce(read()).mockResolvedValueOnce({ status: "error", code: "stale", message: "The draft changed. Refresh." });
    const store = new MindsStore(rpc);
    await store.load("mine");
    store.setSource("fn think(view, card) { this.x = 2; }");
    await store.check();
    expect(rpc).toHaveBeenCalledTimes(2);
    expect(rpc.mock.calls[1]![0]).toMatchObject({ op: "replace", base: { edit: 3, revision: 1 } });
    expect(store.draft!.source).toContain("this.x = 2");
    expect(store.dirty).toBe(true);
    expect(store.base).toEqual({ edit: 3, revision: 1 });
    expect(store.error).toContain("Refresh");
  });
  it("checks the newly returned token after syncing and never sends source as a line", async () => {
    const rpc = vi.fn<MindRpc>().mockResolvedValueOnce(read())
      .mockResolvedValueOnce({ status: "edited", name: "mine", base: { edit: 4, revision: 1 } })
      .mockResolvedValueOnce({ status: "checked", job: 7, diagnostics: [], reports: [] });
    const store = new MindsStore(rpc);
    await store.load("mine");
    store.setSource("fn think(view, card) { /* @destroy; $(echo x) */ }");
    await store.check();
    expect(rpc.mock.calls[2]![0]).toEqual({ op: "check", name: "mine", base: { edit: 4, revision: 1 } });
    expect(store.dirty).toBe(false);
    expect(store.error).toBe("");
  });
  it("fences late source replies across character changes", async () => {
    let finish!: (value: MindResponse) => void;
    const rpc: MindRpc = () => new Promise(resolve => { finish = resolve; });
    const store = new MindsStore(rpc);
    const pending = store.load("mine");
    store.clear();
    finish(read());
    await pending;
    expect(store.draft).toBeNull();
    expect(store.name).toBe("");
    expect(store.error).toBe("");
    expect(store.busy).toBe(false);
  });
  it("requires explicit reload before discarding unsent changes", async () => {
    const rpc = vi.fn<MindRpc>().mockResolvedValue(read());
    const store = new MindsStore(rpc);
    await store.load("mine");
    store.setSource("new source");
    await store.load("another");
    expect(rpc).toHaveBeenCalledTimes(1);
    expect(store.draft!.source).toBe("new source");
    await store.load("mine", null, true);
    expect(rpc).toHaveBeenCalledTimes(2);
    expect(store.dirty).toBe(false);
  });
  it("uses UTF-8 source bounds and rejects imprecise metadata before sending", async () => {
    const rpc = vi.fn<MindRpc>().mockResolvedValue(read());
    const store = new MindsStore(rpc);
    await store.load("mine");
    const before = store.draft!.source;
    store.setSource("😀".repeat(8193));
    expect(store.draft!.source).toBe(before);
    store.setEnvelope('{"profiles":{"x":9007199254740993}}');
    expect(store.error).toContain("exact integers");
    expect(rpc).toHaveBeenCalledTimes(1);
  });
  it("keeps the working copy and exact base when library removal is stale", async () => {
    const rpc = vi.fn<MindRpc>().mockResolvedValueOnce(read())
      .mockResolvedValueOnce({ status: "error", code: "stale", message: "Reload the changed draft." });
    const store = new MindsStore(rpc);
    await store.load("mine");
    await store.lifecycle({ kind: "remove" });
    expect(rpc.mock.calls[1]![0]).toEqual({ op: "lifecycle", name: "mine", base: { edit: 3, revision: 1 }, edit: { kind: "remove" } });
    expect(store.name).toBe("mine");
    expect(store.draft).not.toBeNull();
    expect(store.base).toEqual({ edit: 3, revision: 1 });
    expect(store.error).toContain("Reload");
    expect(rpc).toHaveBeenCalledTimes(2);
  });
  it("refuses to discard unsent source and clears only the removed library entry after success", async () => {
    const rpc = vi.fn<MindRpc>().mockResolvedValueOnce(read());
    const store = new MindsStore(rpc);
    await store.load("mine");
    store.setSource("unsent source");
    await store.lifecycle({ kind: "remove" });
    expect(rpc).toHaveBeenCalledTimes(1);
    expect(store.draft!.source).toBe("unsent source");
    rpc.mockResolvedValueOnce(read());
    await store.load("mine", null, true);
    rpc.mockResolvedValueOnce({ status: "removed", name: "mine" }).mockResolvedValueOnce({ status: "library", names: ["other"] });
    await store.lifecycle({ kind: "remove" });
    expect(store.draft).toBeNull();
    expect(store.base).toBeNull();
    expect(store.names).toEqual(["other"]);
    expect(store.notice).toContain("existing seals remain");
  });
  it("requests an attributed entry by name without reading source or clearing it optimistically", async () => {
    const rpc = vi.fn<MindRpc>().mockResolvedValueOnce({ status: "library", names: ["mine"] })
      .mockResolvedValueOnce({ status: "removal_requested", name: "mine" });
    const store = new MindsStore(rpc);
    await store.show(); await store.removeAttributed("mine");
    expect(rpc.mock.calls[1]![0]).toEqual({ op: "remove", name: "mine" });
    expect(store.names).toEqual(["mine"]); expect(store.draft).toBeNull();
    expect(store.removalRequested).toEqual(["mine"]); expect(store.notice).toContain("requested");
    await store.removeAttributed("mine"); expect(rpc).toHaveBeenCalledTimes(2);
    rpc.mockResolvedValueOnce({ status: "library", names: ["mine"] });
    await store.show(); expect(store.removalRequested).toEqual(["mine"]);
    rpc.mockResolvedValueOnce({ status: "library", names: [] });
    await store.show(); expect(store.removalRequested).toEqual([]); expect(store.notice).toContain("no longer appears");
  });
  it("retains an uncertain removal without replay and accepts only explicit refusal or library absence", async () => {
    const rpc = vi.fn<MindRpc>().mockResolvedValueOnce({ status: "library", names: ["mine"] })
      .mockRejectedValueOnce(new Error("connection unavailable"));
    const store = new MindsStore(rpc); await store.show(); await store.removeAttributed("mine");
    expect(store.removalRequested).toEqual(["mine"]); expect(store.names).toEqual(["mine"]);
    await store.removeAttributed("mine"); expect(rpc).toHaveBeenCalledTimes(2);
    store.clear(); expect(store.removalRequested).toEqual([]);
    rpc.mockResolvedValueOnce({ status: "library", names: ["mine"] })
      .mockResolvedValueOnce({ status: "error", code: "stale", message: "Unavailable" });
    await store.show(); await store.removeAttributed("mine");
    expect(store.removalRequested).toEqual([]); expect(store.names).toEqual(["mine"]);
  });
  it("keeps the loaded draft and base when lifecycle removal is only requested", async () => {
    const rpc = vi.fn<MindRpc>().mockResolvedValueOnce(read())
      .mockResolvedValueOnce({ status: "removal_requested", name: "mine" });
    const store = new MindsStore(rpc); await store.load("mine");
    const before = store.draft; await store.lifecycle({ kind: "remove" });
    expect(store.draft).toEqual(before); expect(store.base).toEqual({ edit: 3, revision: 1 });
    expect(store.removalRequested).toEqual(["mine"]); expect(rpc).toHaveBeenCalledTimes(2);
  });

});
