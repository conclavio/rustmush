// The staff unit designer's server half, asked over the `unit_design` invoke
// (doc 31 cut UD.2): a design's rules checked by the server's own loader and
// catalogue build, answered with the refusals or the finished pack.
import type { UnitDesignRequest, UnitDesignResponse } from "../proto";
import type { WsClient } from "../ws";

/** How long a check may take before the studio says so. */
export const CHECK_TIMEOUT_MS = 30_000;

/** One connection's `unit_design` requests and their answers. */
export class UnitDesignRpc {
  #pending = new Map<number, { resolve: (r: UnitDesignResponse) => void; timer: ReturnType<typeof setTimeout> }>();
  readonly #off: () => void;

  constructor(private client: WsClient) {
    this.#off = client.on("invoke_result", (envelope) => {
      const pending = this.#pending.get(envelope.d.id);
      if (!pending || envelope.d.result.service !== "unit_design") return;
      this.#pending.delete(envelope.d.id);
      clearTimeout(pending.timer);
      pending.resolve(envelope.d.result.result);
    });
  }

  /** Ask; a request that never left, or no answer in time, is an `Error`
   * answer rather than a rejection, so the studio has one path to show. */
  request(request: UnitDesignRequest): Promise<UnitDesignResponse> {
    return new Promise((resolve) => {
      const id = this.client.invoke({ service: "unit_design", request });
      if (id === null) {
        resolve({ status: "error", code: "offline", message: "Not connected, or the design is too large." });
        return;
      }
      const timer = setTimeout(() => {
        this.#pending.delete(id);
        resolve({ status: "error", code: "timeout", message: "The server did not answer the check." });
      }, CHECK_TIMEOUT_MS);
      this.#pending.set(id, { resolve, timer });
    });
  }

  /** Stop listening; every pending request is answered as gone. */
  close(): void {
    this.#off();
    for (const [, pending] of this.#pending) {
      clearTimeout(pending.timer);
      pending.resolve({ status: "error", code: "closed", message: "The designer was closed." });
    }
    this.#pending.clear();
  }
}
