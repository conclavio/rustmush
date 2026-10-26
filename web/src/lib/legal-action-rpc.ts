import type { LegalActionRequest, LegalActionResponse } from "./proto";
import type { WsClient } from "./ws";

interface Pending {
  resolve: (value: LegalActionResponse) => void;
  reject: (error: Error) => void;
  timer: ReturnType<typeof setTimeout>;
}
const reference = (value: unknown): value is string => typeof value === "string"
  && value.length <= 256 && /^k5:[a-z0-9_]+(?:\.[a-z0-9_]+)*:[a-z0-9_]+(?:\.[a-z0-9_]+)*:[a-f0-9]{16}:[a-f0-9]{32}$/.test(value);

/** WsClient is a typed transport, not a JSON schema validator. */
export function actionResponse(value: unknown): LegalActionResponse | null {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return null;
  const raw = value as Record<string, unknown>;
  const allowed = new Set(["state"]);
  switch (raw.state) {
    case "queued": case "unknown": case "expired": case "busy": case "rejected": case "invalid": break;
    case "owned": case "pending":
      if (!reference(raw.reference)) return null;
      allowed.add("reference"); break;
    case "committed":
      if (!reference(raw.reference) || typeof raw.commit !== "string" || !/^[a-f0-9]{32}$/.test(raw.commit)) return null;
      allowed.add("reference"); allowed.add("commit"); break;
    case "unavailable":
      if (raw.reference !== null && !reference(raw.reference)) return null;
      allowed.add("reference"); break;
    default: return null;
  }
  if (Object.keys(raw).some(key => !allowed.has(key))) return null;
  return raw as unknown as LegalActionResponse;
}

export class LegalActionRpcClient {
  #pending = new Map<number, Pending>();
  constructor(private client: WsClient) {
    client.on("invoke_result", envelope => {
      if (typeof envelope.d !== "object" || envelope.d === null
        || !Number.isSafeInteger(envelope.d.id) || envelope.d.id < 0
        || typeof envelope.d.result !== "object" || envelope.d.result === null) return;
      const pending = this.#pending.get(envelope.d.id);
      if (!pending || envelope.d.result.service !== "legal_action") return;
      const response = actionResponse(envelope.d.result.result);
      if (!response) return;
      clearTimeout(pending.timer);
      this.#pending.delete(envelope.d.id);
      pending.resolve(response);
    });
  }
  request(request: LegalActionRequest): Promise<LegalActionResponse> {
    if (this.#pending.size >= 8) return Promise.reject(new Error("Too many pending actions. Wait for a result."));
    return new Promise((resolve, reject) => {
      const id = this.client.invoke({ service: "legal_action", request });
      if (id === null) { reject(new Error("Not connected. No completion is available.")); return; }
      const timer = setTimeout(() => {
        this.#pending.delete(id);
        reject(new Error("No completion received. Check transaction status before submitting again."));
      }, 30_000);
      this.#pending.set(id, { resolve, reject, timer });
    });
  }
  clear(): void {
    for (const pending of this.#pending.values()) {
      clearTimeout(pending.timer);
      pending.reject(new Error("The connection or character changed. This does not prove the action failed."));
    }
    this.#pending.clear();
  }
}
