import type { LegalActionRequest, LegalActionResponse } from "../proto";
import { actionControl, submitSelf } from "../legal-action-control";
import type { LegalAction } from "./actions.svelte";

export interface ActionAttempt {
  label: string;
  busy: boolean;
  blocked: boolean;
  message: string;
  reference: string | null;
  response: LegalActionResponse | null;
}
type Request = (request: LegalActionRequest) => Promise<LegalActionResponse>;

export function actionMessage(response: LegalActionResponse): string {
  switch (response.state) {
    case "committed": return "Action committed.";
    case "owned": return "Transaction accepted. Check its status for completion.";
    case "pending": return "Transaction is still pending.";
    case "queued": return "Submission queued. Wait for a transaction result before submitting again.";
    case "unavailable": return "Completion is unavailable. This does not mean the action failed.";
    case "unknown": return "No accessible transaction was found. This does not prove the action failed.";
    case "expired": return "The transaction reference expired. Check your current state before taking another action.";
    case "busy": return "The server is busy. Try again later.";
    case "rejected": return "The offer was refused. Refresh available actions.";
    case "invalid": return "The inputs were refused. Check them and refresh available actions.";
  }
}

/** Bounded connection-local attempts. Unresolved submissions are never evicted or replayed. */
export class ActionRequestsStore {
  entries = $state<Record<string, ActionAttempt>>({});
  notice = $state("");
  #epoch = 0;
  constructor(private request: Request, private committed: () => void = () => {}) {}

  async submit(action: LegalAction, values: Record<string, string>): Promise<void> {
    if (this.entries[action.offer]?.blocked || this.entries[action.offer]?.busy) return;
    const control = actionControl(action);
    if (!control) { this.notice = "This action needs another control surface."; return; }
    let request: LegalActionRequest;
    try { request = submitSelf(control, values); }
    catch (error) { this.notice = error instanceof Error ? error.message : "Invalid action inputs."; return; }
    if (!this.entries[action.offer] && Object.keys(this.entries).length >= 64) {
      const retired = Object.keys(this.entries).find(key => {
        const entry = this.entries[key]!;
        return !entry.busy && (!entry.blocked || entry.response?.state === "committed");
      });
      if (retired === undefined) { this.notice = "Too many unresolved actions. Check their transaction status first."; return; }
      delete this.entries[retired];
    }
    this.notice = "";
    this.entries[action.offer] = { label: action.presentation.label, busy: true, blocked: true, message: "Waiting for the server…", reference: null, response: null };
    await this.resolve(action.offer, request, this.#epoch);
  }

  async status(offer: string): Promise<void> {
    const entry = this.entries[offer];
    if (!entry || entry.busy || !entry.reference) return;
    entry.busy = true;
    await this.resolve(offer, { op: "status", reference: entry.reference }, this.#epoch);
  }

  private async resolve(offer: string, request: LegalActionRequest, epoch: number): Promise<void> {
    try {
      const response = await this.request(request);
      const entry = this.entries[offer];
      if (epoch !== this.#epoch || !entry) return;
      entry.response = response;
      entry.message = actionMessage(response);
      if ("reference" in response && response.reference !== null) entry.reference = response.reference;
      // A refused status *lookup* is not a refused original submission.
      if (request.op === "submit_self") entry.blocked = !["busy", "rejected", "invalid"].includes(response.state);
      if (response.state === "committed") { entry.blocked = true; this.committed(); }
    } catch (error) {
      const entry = this.entries[offer];
      if (epoch !== this.#epoch || !entry) return;
      entry.message = error instanceof Error ? error.message : "Completion is unavailable. Do not submit again yet.";
      // Timeout, disconnect and missing replies never unlock a submitted offer.
      entry.blocked = true;
    } finally {
      if (epoch === this.#epoch && this.entries[offer]) this.entries[offer]!.busy = false;
    }
  }

  clear(): void { this.#epoch += 1; this.entries = {}; this.notice = ""; }
}
