import type { MindRequest, MindResponse } from "./proto";
import type { WsClient } from "./ws";

interface Pending {
  resolve: (response: MindResponse) => void;
  reject: (reason: Error) => void;
  progress?: (job: number) => void;
  timer: ReturnType<typeof setTimeout>;
}

/**
 * A request that never left this client: the pending cap was full, or there was no socket to
 * send it on. Unlike a timeout or a `clear()`, no result of it can exist anywhere, so a caller
 * may say "not sent" instead of "not seen" (17g cut C3c: the market's `LOST` sentence is only
 * for a request that did leave). The message is the one these two refusals always carried.
 */
export class MindRpcUnsent extends Error {
  constructor(message: string, readonly reason: "full" | "offline") {
    super(message);
  }
}

/** Connection-local source replies never enter scrollback, storage or resume state. */
export class MindRpcClient {
  #pending = new Map<number, Pending>();
  constructor(private client: WsClient) {
    client.on("invoke_result", envelope => {
      const pending = this.#pending.get(envelope.d.id);
      if (!pending || envelope.d.result.service !== "mind") return;
      const response = envelope.d.result.result;
      if (response.status === "queued" || response.status === "pending") {
        pending.progress?.(response.job);
        return;
      }
      this.#pending.delete(envelope.d.id);
      clearTimeout(pending.timer);
      pending.resolve(response);
    });
  }

  request(request: MindRequest, progress?: (job: number) => void): Promise<MindResponse> {
    if (this.#pending.size >= 8) return Promise.reject(new MindRpcUnsent("Too many pending mind requests.", "full"));
    return new Promise((resolve, reject) => {
      // WebSocket replies are delivered in a later task, after this registration.
      const id = this.client.invoke({ service: "mind", request });
      if (id === null) { reject(new MindRpcUnsent("Reconnect before opening the mind workshop.", "offline")); return; }
      const timer = setTimeout(() => {
        this.#pending.delete(id);
        reject(new Error("The mind request timed out. Refresh before retrying a save."));
      }, 5 * 60 * 1000);
      this.#pending.set(id, { resolve, reject, progress, timer });
    });
  }

  clear(): void {
    for (const pending of this.#pending.values()) {
      clearTimeout(pending.timer);
      pending.reject(new Error("The character or connection changed. Reopen the mind workshop."));
    }
    this.#pending.clear();
  }
}
