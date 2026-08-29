<script lang="ts">
  // Right rail: tabbed Mail / Room / Sheet / Rolls / Build / LLM panels.
  // Mail/Sheet/Rolls/Build are gated on the server's hello features/panels —
  // Sheet and Rolls key off any panel feed ending `.sheet`/`.roll`
  // (SessionStore.sheetFeed / .rollFeed), never a hardcoded ruleset id, so
  // this tab bar needs no change when Vampire/Mage/Werewolf (or a future
  // ruleset) come online. Build (the worldgen draft preview) keys off the
  // fixed `"worldgen"` feature/`"worldgen.draft"` panel — see
  // SessionStore.worldgenAvailable. LLM (the staff cost dashboard) keys off
  // the fixed `"llm.dashboard"` panel the same way — see
  // SessionStore.llmDashboardAvailable; a non-staff viewer still sees the
  // tab and gets the server's refusal rendered inside it, same contract as
  // Build's BUILDER_LLM gate.
  import type { GameApp } from "$lib/game.svelte";

  import LlmPanel from "./LlmPanel.svelte";
  import MailPanel from "./MailPanel.svelte";
  import RollsPanel from "./RollsPanel.svelte";
  import RoomPanel from "./RoomPanel.svelte";
  import SheetPanel from "./SheetPanel.svelte";
  import WorldgenPanel from "./WorldgenPanel.svelte";

  let { app, onCmd }: { app: GameApp; onCmd: (cmd: string) => void } = $props();

  const mailEnabled = $derived(app.session.hasFeature("mail") || app.session.hasPanel("mail"));
  const sheetEnabled = $derived(app.session.sheetFeed !== null);
  const rollsEnabled = $derived(app.session.rollFeed !== null);
  const worldgenEnabled = $derived(app.session.worldgenAvailable);
  const llmEnabled = $derived(app.session.llmDashboardAvailable);

  type Tab = "mail" | "room" | "sheet" | "rolls" | "worldgen" | "llm";
  let tab = $state<Tab>("room");

  // If mail turns out to be enabled once hello lands, prefer it as the
  // default tab exactly once.
  let defaulted = $state(false);
  $effect(() => {
    if (mailEnabled && !defaulted) {
      defaulted = true;
      tab = "mail";
    }
  });

  const activeTab = $derived<Tab>(
    (tab === "mail" && !mailEnabled) ||
      (tab === "sheet" && !sheetEnabled) ||
      (tab === "rolls" && !rollsEnabled) ||
      (tab === "worldgen" && !worldgenEnabled) ||
      (tab === "llm" && !llmEnabled)
      ? "room"
      : tab,
  );
</script>

<div class="panel">
  <div class="tabs" role="tablist" aria-label="panels">
    {#if mailEnabled}
      <button
        type="button"
        role="tab"
        class="tab"
        class:active={activeTab === "mail"}
        aria-selected={activeTab === "mail"}
        onclick={() => (tab = "mail")}
      >
        Mail
        {#if app.mail.unread > 0}<span class="badge">{app.mail.unread}</span>{/if}
      </button>
    {/if}
    <button
      type="button"
      role="tab"
      class="tab"
      class:active={activeTab === "room"}
      aria-selected={activeTab === "room"}
      onclick={() => (tab = "room")}
    >
      Room
    </button>
    {#if sheetEnabled}
      <button
        type="button"
        role="tab"
        class="tab"
        class:active={activeTab === "sheet"}
        aria-selected={activeTab === "sheet"}
        onclick={() => (tab = "sheet")}
      >
        Sheet
      </button>
    {/if}
    {#if rollsEnabled}
      <button
        type="button"
        role="tab"
        class="tab"
        class:active={activeTab === "rolls"}
        aria-selected={activeTab === "rolls"}
        onclick={() => (tab = "rolls")}
      >
        Rolls
        {#if app.rolls.entries.length > 0}<span class="badge">{app.rolls.entries.length}</span>{/if}
      </button>
    {/if}
    {#if worldgenEnabled}
      <button
        type="button"
        role="tab"
        class="tab"
        class:active={activeTab === "worldgen"}
        aria-selected={activeTab === "worldgen"}
        onclick={() => (tab = "worldgen")}
      >
        Build
        {#if app.worldgen.draft !== null}<span class="badge">1</span>{/if}
      </button>
    {/if}
    {#if llmEnabled}
      <button
        type="button"
        role="tab"
        class="tab"
        class:active={activeTab === "llm"}
        aria-selected={activeTab === "llm"}
        onclick={() => (tab = "llm")}
      >
        LLM
      </button>
    {/if}
  </div>

  <div class="body">
    {#if activeTab === "mail" && mailEnabled}
      <MailPanel {app} />
    {:else if activeTab === "sheet" && sheetEnabled}
      <SheetPanel {app} />
    {:else if activeTab === "rolls" && rollsEnabled}
      <RollsPanel {app} />
    {:else if activeTab === "worldgen" && worldgenEnabled}
      <WorldgenPanel {app} />
    {:else if activeTab === "llm" && llmEnabled}
      <LlmPanel {app} />
    {:else}
      <RoomPanel {app} {onCmd} />
    {/if}
  </div>
</div>

<style>
  .panel {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }

  .tabs {
    display: flex;
    gap: 0.25rem;
    padding: 0.35rem 0.5rem 0;
    border-bottom: 1px solid var(--border);
  }

  .tab {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.3rem 0.7rem;
    border: 1px solid transparent;
    border-bottom: none;
    border-radius: 8px 8px 0 0;
    background: none;
    color: var(--fg-muted);
    cursor: pointer;
    font: inherit;
    font-size: 0.85rem;
  }

  .tab.active {
    background: var(--bg);
    border-color: var(--border);
    color: var(--fg);
  }

  .badge {
    background: var(--accent);
    color: var(--bg);
    border-radius: 999px;
    font-size: 0.68rem;
    font-weight: 700;
    padding: 0 0.35rem;
    min-width: 1.1rem;
    text-align: center;
  }

  .body {
    flex: 1;
    min-height: 0;
  }
</style>
