// The staff content studio's server half, asked over the `content_studio` invoke
// (doc 34 cut CS.0a, D-34-7): catalog indexing, pack/table check and canonical
// SHA-256 signing, live dev-world apply, and `BUILDER_LLM`-gated co-authoring.
import type { ContentStudioRequest, ContentStudioResponse } from "../proto";
import type { WsClient } from "../ws";

/** How long a studio invoke may take before reporting a timeout. */
export const STUDIO_TIMEOUT_MS = 30_000;

/** One connection's `content_studio` requests and their answers. */
export class ContentStudioRpc {
  #pending = new Map<
    number,
    { resolve: (r: ContentStudioResponse) => void; timer: ReturnType<typeof setTimeout> }
  >();
  readonly #off: () => void;

  constructor(private client: WsClient) {
    this.#off = client.on("invoke_result", (envelope) => {
      const pending = this.#pending.get(envelope.d.id);
      if (!pending || envelope.d.result.service !== "content_studio") return;
      this.#pending.delete(envelope.d.id);
      clearTimeout(pending.timer);
      pending.resolve(envelope.d.result.result);
    });
  }

  /** Send a `content_studio` request; offline or timeout resolves to an `error` response. */
  request(request: ContentStudioRequest): Promise<ContentStudioResponse> {
    return new Promise((resolve) => {
      const id = this.client.invoke({ service: "content_studio", request });
      if (id === null) {
        resolve({
          status: "error",
          code: "offline",
          message: "Not connected, or the document exceeds the wire limit.",
        });
        return;
      }
      const timer = setTimeout(() => {
        this.#pending.delete(id);
        resolve({
          status: "error",
          code: "timeout",
          message: "The server did not answer the Content Studio request.",
        });
      }, STUDIO_TIMEOUT_MS);
      this.#pending.set(id, { resolve, timer });
    });
  }

  /** Stop listening; every pending request is answered as closed. */
  close(): void {
    this.#off();
    for (const [, pending] of this.#pending) {
      clearTimeout(pending.timer);
      pending.resolve({
        status: "error",
        code: "closed",
        message: "The Content Studio was closed.",
      });
    }
    this.#pending.clear();
  }
}
