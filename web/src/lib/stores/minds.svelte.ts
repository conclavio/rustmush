import type { MindDraft, MindEdit, MindEditBase, MindRequest, MindResponse, MindDiagnostic, MindPictureReport, MindLifecycle } from "../proto";
import { decodeMindCard, parseCardJson } from "../mind-card";
import type { ProvingView } from "../mind-bulk";
import type { BenchView } from "../mind-bench";

export type MindRpc = (request: MindRequest, progress?: (job: number) => void) => Promise<MindResponse>;
const bytes = (value: string) => new TextEncoder().encode(value).length;

/** Unsaved editor state is RAM-only and belongs to one connection/character. */
export class MindsStore {
  open = $state(false);
  names = $state<string[]>([]);
  name = $state("");
  revision = $state<string | null>(null);
  draft = $state<MindDraft | null>(null);
  base = $state<MindEditBase | null>(null);
  rulesText = $state<Record<string, string[]>>({});
  rulesSource = $state<Record<string, (string | null)[]>>({});
  profile = $state("");
  dirty = $state(false);
  busy = $state(false);
  job = $state<number | null>(null);
  error = $state("");
  notice = $state("");
  removalRequested = $state<string[]>([]);
  diagnostics = $state<MindDiagnostic[]>([]);
  reports = $state<MindPictureReport[]>([]);
  trace = $state<unknown>(null);
  proving = $state<ProvingView | null>(null);
  bench = $state<BenchView | null>(null);
  #generation = 0;
  constructor(private rpc: MindRpc, public readProving: (run: number | null, page: number | null) => void = () => {}) {}

  get profiles(): string[] {
    const profiles = this.draft?.profiles;
    return profiles && typeof profiles === "object" && !Array.isArray(profiles)
      ? Object.keys(profiles).slice(0, 4) : [];
  }
  get card() {
    const profiles = this.draft?.profiles;
    return this.draft && profiles && typeof profiles === "object" && !Array.isArray(profiles)
      ? decodeMindCard({ schema: this.draft.schema, values: (profiles as Record<string, unknown>)[this.profile] }) : null;
  }
  get readonly(): boolean { return this.revision !== null || this.busy; }

  clear(): void {
    this.#generation++;
    this.open = false; this.names = []; this.name = ""; this.revision = null;
    this.draft = null; this.base = null; this.rulesText = {}; this.rulesSource = {}; this.profile = "";
    this.dirty = false; this.busy = false; this.job = null; this.error = "";
    this.notice = ""; this.removalRequested = []; this.diagnostics = []; this.reports = []; this.trace = null; this.proving = null; this.bench = null;
  }
  async #request(request: MindRequest, refused?: () => void): Promise<MindResponse> {
    const generation = this.#generation;
    const response = await this.rpc(request, job => { if (generation === this.#generation) this.job = job; });
    if (generation !== this.#generation) throw new Error("The character or connection changed.");
    if (response.status === "error") { refused?.(); throw new Error(response.message); }
    return response;
  }
  async #run(operation: () => Promise<void>): Promise<void> {
    if (this.busy) return;
    const generation = this.#generation;
    this.busy = true; this.error = ""; this.notice = "";
    try { await operation(); }
    catch (error) { if (generation === this.#generation) this.error = error instanceof Error ? error.message : "Mind request failed."; }
    finally { if (generation === this.#generation) { this.busy = false; this.job = null; } }
  }
  async show(): Promise<void> {
    this.open = true;
    await this.#run(async () => {
      const response = await this.#request({ op: "list" });
      if (response.status === "library") this.#library(response.names);
    });
  }
  #library(names: string[]): void {
    this.names = names.slice(0, 20);
    const absent = this.removalRequested.filter(name => !names.includes(name));
    this.removalRequested = this.removalRequested.filter(name => names.includes(name));
    // A requested removal is not success. Only this fresh library response can
    // establish absence; keep any editor text until the player changes it.
    if (absent.length) this.notice = `${absent.join(", ")} no longer appears in your library. Retained revisions and existing seals remain available.`;
    else if (this.removalRequested.length) this.notice = "Removal requested; still present in the latest library. Refresh to check again.";
  }
  #requested(name: string): void {
    if (!this.removalRequested.includes(name)) this.removalRequested = [...this.removalRequested, name].slice(-20);
    this.notice = `Removal requested for ${name}. Refresh your library to confirm; the request may still be pending.`;
  }
  removeAttributed(name: string): Promise<void> {
    return this.#run(async () => {
      if (!this.names.includes(name)) throw new Error("Select an entry from your library first.");
      if (this.dirty) throw new Error("Check or save your unsent changes before managing a library entry.");
      if (this.removalRequested.includes(name)) throw new Error("Removal already requested. Refresh your library before trying again.");
      // A transport failure may hide admission. Retain this marker until
      // explicit refusal or a fresh library proves absence; never auto-replay.
      this.#requested(name);
      const response = await this.#request({ op: "remove", name }, () => {
        this.removalRequested = this.removalRequested.filter(item => item !== name);
        this.notice = "";
      });
      if (response.status !== "removal_requested" || response.name !== name) throw new Error("The removal request was not acknowledged.");
      this.#requested(name);
    });
  }
  async #read(name: string, revision: string | null): Promise<void> {
    const response = await this.#request({ op: "read", name, revision });
    if (response.status !== "read") throw new Error("The server did not return the requested mind.");
    this.name = response.name; this.revision = response.revision;
    this.draft = structuredClone(response.draft); this.base = response.base;
    this.rulesText = response.rules_text;
    this.rulesSource = response.rules_source;
    if (!this.profiles.includes(this.profile)) this.profile = this.profiles[0] ?? "";
    this.dirty = false; this.diagnostics = response.diagnostics; this.reports = [];
  }
  load(name: string, revision: string | null = null, discard = false): Promise<void> {
    if (this.dirty && !discard) {
      this.error = "Check or save your unsent changes first, or explicitly reload to discard them.";
      return Promise.resolve();
    }
    return this.#run(() => this.#read(name, revision));
  }
  create(name: string, from: string | null): Promise<void> {
    if (this.dirty) { this.error = "Check or save your unsent changes before opening another working copy."; return Promise.resolve(); }
    return this.#run(async () => {
      const response = await this.#request({ op: "create", name, from });
      if (response.status !== "edited") throw new Error("The server did not create the mind.");
      await this.#read(response.name, null);
      const library = await this.#request({ op: "list" });
      if (library.status === "library") this.names = library.names.slice(0, 20);
    });
  }
  setSource(source: string): void {
    if (!this.draft || this.readonly || source === this.draft.source) return;
    if (bytes(source) > 32 * 1024 || source.includes("\0")) { this.error = "Source must fit 32 KiB and contain no NUL characters."; return; }
    this.draft = { ...this.draft, source }; this.dirty = true;
    this.diagnostics = []; this.reports = [];
  }
  setEnvelope(text: string): void {
    if (!this.draft || this.readonly) return;
    try {
      if (bytes(text) > 64 * 1024) throw new Error("Card definition must fit 64 KiB.");
      const envelope = parseCardJson(text);
      if (!envelope || typeof envelope !== "object" || Array.isArray(envelope)) throw new Error("Expected a card definition object.");
      const e = envelope as Record<string, unknown>;
      if (Object.keys(e).some(key => !["schema", "presets", "profiles", "rules", "declared_class", "mind_abi"].includes(key))) throw new Error("Unknown card definition field.");
      if (typeof e.declared_class !== "string" || typeof e.mind_abi !== "string" || !e.schema || !e.profiles || !e.rules || !e.presets) throw new Error("Complete schema, presets, profiles, rules, class and ABI are required.");
      this.draft = { source: this.draft.source, ...e } as MindDraft;
      this.dirty = true; this.error = ""; this.rulesText = {}; this.rulesSource = {}; this.diagnostics = []; this.reports = [];
      if (!this.profiles.includes(this.profile)) this.profile = this.profiles[0] ?? "";
    } catch (error) { this.error = error instanceof Error ? error.message : "Invalid card definition."; }
  }
  async #sync(): Promise<void> {
    if (!this.draft || !this.base || this.revision !== null) throw new Error("Open a working copy first.");
    if (!this.dirty) return;
    // Keep local text on every refusal. Never retry with a guessed newer base.
    // JsonValue is recursive; snapshot's mapped type must not expand that graph.
    const draft = structuredClone($state.snapshot(this.draft as unknown)) as MindDraft;
    const response = await this.#request({ op: "replace", name: this.name, base: this.base, draft });
    if (response.status !== "edited") throw new Error("The server did not accept the working copy.");
    this.base = response.base; this.dirty = false;
  }
  edit(edit: MindEdit): Promise<void> {
    return this.#run(async () => {
      await this.#sync();
      const response = await this.#request({ op: "edit", name: this.name, base: this.base!, edit });
      if (response.status !== "edited") throw new Error("The card edit did not complete.");
      await this.#read(this.name, null);
    });
  }
  check(): Promise<void> {
    return this.#run(async () => {
      await this.#sync();
      const response = await this.#request({ op: "check", name: this.name, base: this.base! });
      if (response.status === "checked") {
        this.diagnostics = response.diagnostics; this.reports = response.reports;
        this.notice = "Check completed against the server's six isolated pictures.";
      } else if (response.status === "cancelled") this.notice = "Check cancelled.";
      else throw new Error("Unexpected check response.");
    });
  }
  save(notes: string, version: string | null): Promise<void> {
    return this.#run(async () => {
      if (bytes(notes) > 2 * 1024) throw new Error("Revision notes must fit 2 KiB.");
      await this.#sync();
      const response = await this.#request({ op: "save", name: this.name, base: this.base!, notes, version });
      if (response.status === "saved") {
        await this.#read(this.name, null);
        this.notice = `Saved revision ${response.revision} (${response.digest.slice(0, 12)}). Existing seals keep their earlier revision.`;
      } else if (response.status === "checked") { this.diagnostics = response.diagnostics; this.reports = response.reports; }
      else if (response.status === "cancelled") this.notice = "Save cancelled.";
      else throw new Error("The revision was not saved.");
    });
  }
  async cancel(): Promise<void> {
    if (this.job === null) return;
    try { await this.#request({ op: "cancel", job: this.job }); }
    catch (error) { this.error = error instanceof Error ? error.message : "Cancellation failed."; }
  }
  lifecycle(edit: MindLifecycle): Promise<void> {
    return this.#run(async () => {
      if (!this.base || this.revision !== null) throw new Error("Open the working copy first.");
      if (this.dirty) throw new Error("Check or save your unsent changes before managing this library entry.");
      if (edit.kind === "remove" && this.removalRequested.includes(this.name)) throw new Error("Removal already requested. Refresh your library to check it.");
      if (edit.kind === "remove") this.#requested(this.name);
      const response = await this.#request({ op: "lifecycle", name: this.name, base: this.base, edit }, () => {
        if (edit.kind === "remove") { this.removalRequested = this.removalRequested.filter(item => item !== this.name); this.notice = ""; }
      });
      if (response.status === "removal_requested") {
        if (edit.kind !== "remove" || response.name !== this.name) throw new Error("Mismatched removal request acknowledgement.");
        this.#requested(response.name);
        return;
      } else if (response.status === "removed") {
        this.removalRequested = this.removalRequested.filter(name => name !== response.name);
        this.name = ""; this.revision = null; this.base = null; this.draft = null;
        this.rulesText = {}; this.rulesSource = {}; this.profile = ""; this.diagnostics = []; this.reports = [];
        this.notice = `Removed ${response.name} from your library. Retained revisions and existing seals remain available.`;
      } else if (response.status === "edited") {
        await this.#read(response.name, null);
        this.notice = "Library entry updated.";
      } else throw new Error("The library change did not complete.");
      const library = await this.#request({ op: "list" });
      if (library.status === "library") this.names = library.names.slice(0, 20);
    });
  }
  prune(): Promise<void> {
    return this.#run(async () => {
      const response = await this.#request({ op: "prune", limit: 128 });
      if (response.status !== "pruned") throw new Error("Revision cleanup did not complete.");
      this.notice = `Removed ${response.revisions} unreferenced revision records. ${response.source_candidates_retained} source candidates remain retained for other references.`;
    });
  }
}
