import { describe, expect, it, vi } from "vitest";
import { ActionRequestsStore } from "./action-requests.svelte";
import type { LegalActionResponse } from "../proto";
import type { LegalAction } from "./actions.svelte";
const reference = `k5:market:purchase:0000000000000000:${"a".repeat(32)}`;
function action(ordinal = 1): LegalAction {
  return { offer: ordinal.toString(16).padStart(64, "0"), presentation: { verb: "same", label: "Purchase", help: "" },
    projected: { subject: { kind: "self_actor" }, targets: { kind: "none" }, parameters: [] } };
}
describe("uncertain action custody in the controls", () => {
  it("blocks repeat submissions after every uncertain outcome and only checks existing status", async () => {
    for (const response of [{ state: "queued" }, { state: "unavailable", reference }, { state: "owned", reference }] as LegalActionResponse[]) {
      const request = vi.fn().mockResolvedValue(response);
      const store = new ActionRequestsStore(request);
      await store.submit(action(), {}); await store.submit(action(), {});
      expect(request).toHaveBeenCalledTimes(1);
      if ("reference" in response) {
        request.mockResolvedValue({ state: "busy" });
        await store.status(action().offer);
        expect(request).toHaveBeenLastCalledWith({ op: "status", reference });
        await store.submit(action(), {});
        expect(request).toHaveBeenCalledTimes(2); // status Busy cannot unlock original submission
      }
    }
  });
  it("retains unknown custody on timeout, caps64 entries and discards late replies after context reset", async () => {
    const request = vi.fn().mockRejectedValue(new Error("No completion"));
    const store = new ActionRequestsStore(request);
    for (let i = 1; i <= 65; i++) await store.submit(action(i), {});
    expect(request).toHaveBeenCalledTimes(64);
    expect(Object.keys(store.entries)).toHaveLength(64);
    expect(store.notice).toContain("unresolved");
    let finish!: (value: LegalActionResponse) => void;
    const delayed = new ActionRequestsStore(() => new Promise(resolve => { finish = resolve; }));
    const waiting = delayed.submit(action(), {});
    delayed.clear(); finish({ state: "owned", reference }); await waiting;
    expect(delayed.entries).toEqual({});
  });
  it("allows explicit retry only after known pre-ownership refusal and refreshes on commit", async () => {
    const request = vi.fn().mockResolvedValueOnce({ state: "busy" }).mockResolvedValueOnce({ state: "committed", reference, commit: "b".repeat(32) });
    const committed = vi.fn();
    const store = new ActionRequestsStore(request, committed);
    await store.submit(action(), {}); await store.submit(action(), {}); await store.submit(action(), {});
    expect(request).toHaveBeenCalledTimes(2); expect(committed).toHaveBeenCalledTimes(1);
  });
});
