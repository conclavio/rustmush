<script lang="ts">
  // Staff Content Studio shell (doc 34 cut CS.0a, D-34-1 .. D-34-9):
  // Sub-tabs for NPCs & Cast, Quests, Components & Goods, and Corridor & Relays;
  // live catalog hydration, server-authoritative Check & Sign, Publish to Running
  // World, and Download for Repo (with historical `-vN` preservation and new-pack
  // Rust registration snippet).
  import { onMount } from "svelte";
  import type { GameApp } from "$lib/game.svelte";
  import type { JsonValue } from "$lib/proto";
  import { ContentStudioRpc } from "$lib/content/rpc";
  import NpcStudio from "./NpcStudio.svelte";
  import QuestStudio from "./QuestStudio.svelte";
  import {
    type CatalogData,
    DocumentHistory,
    emptyCatalog,
    historicalSnapshotPath,
    newPackRegistrationSnippet,
    type PackSummary,
    parseCatalog,
    type StudioSubTab,
  } from "$lib/content/store";

  let { app }: { app: GameApp } = $props();

  let subTab = $state<StudioSubTab>("npcs");
  let catalog = $state<CatalogData>(emptyCatalog());
  let selectedPackId = $state<string>("tacsim.crew.ceres");
  let repoPath = $state<string>("crew/ceres.json");
  let latchedVersion = $state<number | null>(null);
  let retired = $state<boolean>(false);
  let documentJson = $state<JsonValue | null>(null);
  let rawEditorText = $state<string>("");

  let loading = $state<boolean>(false);
  let checking = $state<boolean>(false);
  let publishing = $state<boolean>(false);
  let statusMessage = $state<string>("");
  let errorMessage = $state<string>("");
  let refusals = $state<string[]>([]);
  let computedDigest = $state<string | null>(null);
  let cascadePacks = $state<JsonValue[]>([]);
  let liveIncrementExport = $state<boolean>(false);
  let relocateNpcs = $state<string[]>([]);
  let showRawJson = $state<boolean>(false);

  const history = new DocumentHistory<JsonValue>();
  let canUndo = $state<boolean>(false);
  let canRedo = $state<boolean>(false);

  let rpc: ContentStudioRpc | null = null;

  const filteredPacks = $derived<PackSummary[]>(
    catalog.packs.filter((p) => {
      if (subTab === "npcs") return p.family === "npc";
      if (subTab === "quests") return p.family === "quest";
      if (subTab === "components") return p.family === "component";
      if (subTab === "corridor") return p.family === "corridor";
      return true;
    }),
  );

  onMount(() => {
    rpc = new ContentStudioRpc(app.client);
    void refreshCatalogAndLoad("tacsim.crew.ceres");
    return () => {
      rpc?.close();
      rpc = null;
    };
  });

  async function refreshCatalogAndLoad(initialPackId?: string): Promise<void> {
    if (!rpc) return;
    loading = true;
    errorMessage = "";
    const res = await rpc.request({ op: "catalog" });
    loading = false;
    if (res.status === "error") {
      errorMessage = res.message;
      return;
    }
    if (res.status === "catalog") {
      catalog = parseCatalog(res.catalog);
      const target = initialPackId ?? selectedPackId;
      if (target) {
        await loadPack(target);
      }
    }
  }

  async function loadPack(id: string): Promise<void> {
    if (!rpc) return;
    loading = true;
    errorMessage = "";
    statusMessage = "";
    refusals = [];
    const res = await rpc.request({ op: "read", id });
    loading = false;
    if (res.status === "error") {
      errorMessage = res.message;
      return;
    }
    if (res.status === "document") {
      selectedPackId = res.id;
      repoPath = res.repo_path;
      latchedVersion = res.latched_version;
      retired = res.retired;
      documentJson = res.content;
      rawEditorText = JSON.stringify(res.content, null, 2);
      history.clear();
      syncHistoryFlags();
    }
  }

  function syncHistoryFlags(): void {
    canUndo = history.canUndo;
    canRedo = history.canRedo;
  }

  function selectSubTab(next: StudioSubTab): void {
    subTab = next;
    const first = catalog.packs.find((p) => {
      if (next === "npcs") return p.family === "npc";
      if (next === "quests") return p.family === "quest";
      if (next === "components") return p.family === "component";
      if (next === "corridor") return p.family === "corridor";
      return false;
    });
    if (first && first.id !== selectedPackId) {
      void loadPack(first.id);
    }
  }

  function applyStructuredEdit(nextDoc: JsonValue): void {
    if (documentJson !== null) {
      history.record(documentJson);
    }
    documentJson = nextDoc;
    rawEditorText = JSON.stringify(nextDoc, null, 2);
    errorMessage = "";
    syncHistoryFlags();
  }

  function applyRawJsonEdit(nextText: string): void {
    rawEditorText = nextText;
    try {
      const parsed = JSON.parse(nextText) as JsonValue;
      if (documentJson !== null) {
        history.record(documentJson);
      }
      documentJson = parsed;
      errorMessage = "";
      syncHistoryFlags();
    } catch (err) {
      errorMessage = err instanceof Error ? `JSON syntax: ${err.message}` : "Invalid JSON";
    }
  }

  function undoEdit(): void {
    if (!documentJson) return;
    const prev = history.undo(documentJson);
    if (prev !== null) {
      documentJson = prev;
      rawEditorText = JSON.stringify(prev, null, 2);
      syncHistoryFlags();
    }
  }

  function redoEdit(): void {
    if (!documentJson) return;
    const next = history.redo(documentJson);
    if (next !== null) {
      documentJson = next;
      rawEditorText = JSON.stringify(next, null, 2);
      syncHistoryFlags();
    }
  }

  async function checkAndSign(): Promise<void> {
    if (!rpc || !documentJson) return;
    checking = true;
    errorMessage = "";
    statusMessage = "";
    refusals = [];
    const res = await rpc.request({
      op: "check",
      id: selectedPackId,
      content: documentJson,
      live_check: true,
    });
    checking = false;
    if (res.status === "error") {
      errorMessage = res.message;
      return;
    }
    if (res.status === "checked") {
      refusals = res.refusals;
      computedDigest = res.digest;
      cascadePacks = res.cascade;
      if (res.pack !== null) {
        documentJson = res.pack;
        rawEditorText = JSON.stringify(res.pack, null, 2);
        statusMessage = res.digest
          ? `Signed ${res.repo_path} (digest ${res.digest.slice(0, 12)}…)`
          : `Validated ${res.repo_path}`;
      }
    }
  }

  async function publishToWorld(): Promise<void> {
    if (!rpc || !documentJson) return;
    publishing = true;
    errorMessage = "";
    statusMessage = "";
    refusals = [];
    const res = await rpc.request({
      op: "publish",
      id: selectedPackId,
      content: documentJson,
      relocate_npcs: relocateNpcs,
    });
    publishing = false;
    if (res.status === "error") {
      errorMessage = res.message;
      return;
    }
    if (res.status === "published") {
      refusals = res.refusals;
      statusMessage = res.summary;
      if (res.pack !== null) {
        documentJson = res.pack;
        rawEditorText = JSON.stringify(res.pack, null, 2);
      }
      await refreshCatalogAndLoad(selectedPackId);
    }
  }

  function downloadFile(filename: string, text: string): void {
    if (typeof document === "undefined") return;
    const blob = new Blob([text.endsWith("\n") ? text : `${text}\n`], {
      type: "application/json",
    });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = filename.split("/").pop() ?? filename;
    a.click();
    URL.revokeObjectURL(url);
  }

  async function downloadForRepo(): Promise<void> {
    await checkAndSign();
    if (refusals.length > 0 || errorMessage || !documentJson) return;
    downloadFile(repoPath, JSON.stringify(documentJson, null, 2));
  }
</script>

<div class="content-studio">
  <div class="studio-header">
    <div class="pill-bar" role="tablist" aria-label="Content Studio families">
      <button
        type="button"
        role="tab"
        aria-selected={subTab === "npcs"}
        class="pill"
        class:active={subTab === "npcs"}
        onclick={() => selectSubTab("npcs")}
      >
        NPCs &amp; Cast ({catalog.npcs.length})
      </button>
      <button
        type="button"
        role="tab"
        aria-selected={subTab === "quests"}
        class="pill"
        class:active={subTab === "quests"}
        onclick={() => selectSubTab("quests")}
      >
        Quests
      </button>
      <button
        type="button"
        role="tab"
        aria-selected={subTab === "components"}
        class="pill"
        class:active={subTab === "components"}
        onclick={() => selectSubTab("components")}
      >
        Components &amp; Goods
      </button>
      <button
        type="button"
        role="tab"
        aria-selected={subTab === "corridor"}
        class="pill"
        class:active={subTab === "corridor"}
        onclick={() => selectSubTab("corridor")}
      >
        Corridor &amp; Relays
      </button>
    </div>

    <div class="pack-bar">
      <label class="pack-picker">
        <span class="muted">Document:</span>
        <select
          value={selectedPackId}
          onchange={(e) => void loadPack((e.currentTarget as HTMLSelectElement).value)}
        >
          {#each filteredPacks as pack (pack.id)}
            <option value={pack.id}>
              {pack.repo_path} ({pack.id}@{pack.version})
            </option>
          {/each}
        </select>
      </label>

      {#if latchedVersion !== null}
        <span class="badge latched" title="Latched on the running world">
          Live v{latchedVersion}
        </span>
      {/if}
      {#if retired}
        <span class="badge retired" title="Retired on the running world">RETIRED</span>
      {/if}

      <div class="actions">
        <button type="button" onclick={undoEdit} disabled={!canUndo} title="Undo">Undo</button>
        <button type="button" onclick={redoEdit} disabled={!canRedo} title="Redo">Redo</button>
        <button type="button" onclick={() => void checkAndSign()} disabled={checking || loading}>
          {checking ? "Checking…" : "Check & Sign"}
        </button>
        <button
          type="button"
          class="primary"
          onclick={() => void publishToWorld()}
          disabled={publishing || loading}
        >
          {publishing ? "Publishing…" : "Publish to Running World"}
        </button>
        <button type="button" onclick={() => void downloadForRepo()} disabled={checking || loading}>
          Download for Repo ({repoPath})
        </button>
      </div>
    </div>
  </div>

  {#if errorMessage}
    <div class="banner error" role="alert">{errorMessage}</div>
  {/if}
  {#if refusals.length > 0}
    <div class="banner error" role="alert">
      <strong>Validation refusals ({refusals.length}):</strong>
      <ul>
        {#each refusals as ref, i (i)}
          <li>{ref}</li>
        {/each}
      </ul>
    </div>
  {/if}
  {#if statusMessage}
    <div class="banner ok">{statusMessage}</div>
  {/if}

  {#if cascadePacks.length > 0}
    <div class="banner cascade">
      <strong>Cascade notice:</strong> Dependent packs require re-signing for this version change:
      {#each cascadePacks as c, i (i)}
        {@const dep = c as Record<string, unknown>}
        <button
          type="button"
          class="small-btn"
          onclick={() => downloadFile(String(dep.repo_path), JSON.stringify(dep.pack, null, 2))}
        >
          Download {dep.repo_path} (v{dep.version})
        </button>
      {/each}
    </div>
  {/if}

  <div class="studio-body">
    <div class="summary-pane">
      <h4>Catalog &amp; Export</h4>
      <p class="muted">
        Path: <code>crates/tacsim/content/{repoPath}</code>
      </p>
      {#if computedDigest}
        <p class="muted">
          Canonical SHA-256: <code>{computedDigest.slice(0, 16)}…</code>
        </p>
      {/if}
      <label class="checkbox-row">
        <input type="checkbox" bind:checked={liveIncrementExport} />
        <span>Live-world version bump (keep historical <code>{historicalSnapshotPath(repoPath, latchedVersion ?? 1)}</code>)</span>
      </label>
      <details class="snippet-box">
        <summary>New pack file Rust registration snippet (D-34-5)</summary>
        <pre>{newPackRegistrationSnippet(selectedPackId, repoPath)}</pre>
      </details>
    </div>

    <div class="editor-pane">
      {#if subTab === "npcs"}
        <NpcStudio
          {documentJson}
          {catalog}
          {rpc}
          bind:relocateNpcs
          onChange={applyStructuredEdit}
          onStatus={(msg) => (statusMessage = msg)}
          onError={(msg) => (errorMessage = msg)}
        />
      {:else if subTab === "quests"}
        <QuestStudio
          packId={selectedPackId}
          {latchedVersion}
          {documentJson}
          {catalog}
          {rpc}
          onChange={applyStructuredEdit}
          onStatus={(msg) => (statusMessage = msg)}
          onError={(msg) => (errorMessage = msg)}
        />
      {/if}
      <div class="pane-head">
        <span>Canonical Pack / Table JSON (<code>{selectedPackId}</code>)</span>
        <button
          type="button"
          class="small-btn"
          onclick={() => (showRawJson = !showRawJson)}
        >
          {showRawJson || (subTab !== "npcs" && subTab !== "quests")
            ? "Hide Raw JSON"
            : "Show Raw JSON"}
        </button>
      </div>
      {#if showRawJson || (subTab !== "npcs" && subTab !== "quests")}
        <textarea
          class="json-textarea"
          spellcheck="false"
          value={rawEditorText}
          oninput={(e) => applyRawJsonEdit((e.currentTarget as HTMLTextAreaElement).value)}
        ></textarea>
      {/if}
    </div>
  </div>
</div>

<style>
  .content-studio {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    color: var(--fg, #d8dee9);
    font-size: 12px;
  }

  .studio-header {
    display: flex;
    flex-direction: column;
    gap: 0.45rem;
    padding: 0.5rem 0.65rem;
    border-bottom: 1px solid var(--border, #2a2f3a);
    background: var(--bg-elev, #141821);
  }

  .pill-bar {
    display: flex;
    gap: 0.35rem;
    flex-wrap: wrap;
  }

  .pill {
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid var(--border, #2a2f3a);
    color: var(--muted, #8892b0);
    border-radius: 999px;
    padding: 0.22rem 0.7rem;
    font: inherit;
    font-weight: 600;
    cursor: pointer;
  }

  .pill.active {
    color: var(--accent, #88c0d0);
    border-color: var(--accent, #88c0d0);
    background: rgba(136, 192, 208, 0.12);
  }

  .pack-bar {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex-wrap: wrap;
  }

  .pack-picker {
    display: flex;
    align-items: center;
    gap: 0.35rem;
  }

  .pack-picker select {
    background: var(--bg, #0f1218);
    color: inherit;
    border: 1px solid var(--border, #2a2f3a);
    border-radius: 3px;
    padding: 0.2rem 0.4rem;
    font: inherit;
  }

  .actions {
    margin-left: auto;
    display: flex;
    gap: 0.35rem;
    flex-wrap: wrap;
  }

  button {
    background: var(--btn-bg, #1e2430);
    color: inherit;
    border: 1px solid var(--border, #2e3440);
    border-radius: 3px;
    padding: 0.22rem 0.55rem;
    font: inherit;
    cursor: pointer;
  }

  button.primary {
    border-color: var(--accent, #88c0d0);
    color: var(--accent, #88c0d0);
  }

  button:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .badge {
    font-size: 10px;
    padding: 0.1rem 0.4rem;
    border-radius: 3px;
    font-weight: 600;
  }

  .badge.latched {
    background: rgba(163, 190, 140, 0.18);
    color: #a3be8c;
  }

  .badge.retired {
    background: rgba(191, 97, 106, 0.2);
    color: #bf616a;
  }

  .banner {
    padding: 0.4rem 0.65rem;
    border-bottom: 1px solid var(--border, #2a2f3a);
  }

  .banner.error {
    background: rgba(191, 97, 106, 0.16);
    color: #ebcb8b;
  }

  .banner.ok {
    background: rgba(163, 190, 140, 0.14);
    color: #a3be8c;
  }

  .banner.cascade {
    background: rgba(235, 203, 139, 0.14);
    color: #ebcb8b;
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex-wrap: wrap;
  }

  .studio-body {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: 280px 1fr;
  }

  .summary-pane {
    padding: 0.65rem;
    border-right: 1px solid var(--border, #2a2f3a);
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .summary-pane h4 {
    margin: 0;
  }

  .checkbox-row {
    display: flex;
    align-items: flex-start;
    gap: 0.4rem;
  }

  .snippet-box pre {
    white-space: pre-wrap;
    word-break: break-all;
    background: rgba(0, 0, 0, 0.25);
    padding: 0.4rem;
    border-radius: 3px;
    font-size: 11px;
  }

  .editor-pane {
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  .pane-head {
    padding: 0.35rem 0.65rem;
    border-bottom: 1px solid var(--border, #2a2f3a);
    color: var(--muted, #8892b0);
  }

  .json-textarea {
    flex: 1;
    width: 100%;
    min-height: 0;
    resize: none;
    border: none;
    background: var(--bg, #0f1218);
    color: inherit;
    font-family: monospace;
    font-size: 12px;
    padding: 0.65rem;
    box-sizing: border-box;
  }

  .muted {
    color: var(--muted, #8892b0);
  }
</style>
