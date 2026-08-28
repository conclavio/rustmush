<script lang="ts">
  // Right rail: tabbed Mail / Room / Sheet / Rolls panels. Mail/Sheet/Rolls
  // are gated on the server's hello features/panels — Sheet and Rolls key
  // off any panel feed ending `.sheet`/`.roll` (SessionStore.sheetFeed /
  // .rollFeed), never a hardcoded ruleset id, so this tab bar needs no
  // change when Vampire/Mage/Werewolf (or a future ruleset) come online.
  import type { GameApp } from "$lib/game.svelte";

  import MailPanel from "./MailPanel.svelte";
  import RollsPanel from "./RollsPanel.svelte";
  import RoomPanel from "./RoomPanel.svelte";
  import SheetPanel from "./SheetPanel.svelte";

  let { app, onCmd }: { app: GameApp; onCmd: (cmd: string) => void } = $props();

  const mailEnabled = $derived(app.session.hasFeature("mail") || app.session.hasPanel("mail"));
  const sheetEnabled = $derived(app.session.sheetFeed !== null);
  const rollsEnabled = $derived(app.session.rollFeed !== null);

  type Tab = "mail" | "room" | "sheet" | "rolls";
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
      (tab === "rolls" && !rollsEnabled)
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
  </div>

  <div class="body">
    {#if activeTab === "mail" && mailEnabled}
      <MailPanel {app} />
    {:else if activeTab === "sheet" && sheetEnabled}
      <SheetPanel {app} />
    {:else if activeTab === "rolls" && rollsEnabled}
      <RollsPanel {app} />
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
