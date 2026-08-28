<script lang="ts">
  // Right rail: tabbed Mail / Room / Sheet panels. Mail is gated on the
  // server's hello features/panels; Sheet is a placeholder until the WoD
  // ruleset lands (M6).
  import type { GameApp } from "$lib/game.svelte";

  import MailPanel from "./MailPanel.svelte";
  import RoomPanel from "./RoomPanel.svelte";

  let { app, onCmd }: { app: GameApp; onCmd: (cmd: string) => void } = $props();

  const mailEnabled = $derived(app.session.hasFeature("mail") || app.session.hasPanel("mail"));

  type Tab = "mail" | "room" | "sheet";
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

  const activeTab = $derived<Tab>(tab === "mail" && !mailEnabled ? "room" : tab);
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
  </div>

  <div class="body">
    {#if activeTab === "mail" && mailEnabled}
      <MailPanel {app} />
    {:else if activeTab === "room"}
      <RoomPanel {app} {onCmd} />
    {:else}
      <div class="sheet muted">
        <p><strong>Character sheets</strong></p>
        <p>Coming with the World of Darkness ruleset — dots, tracks and rolls will live here.</p>
      </div>
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

  .sheet {
    padding: 1rem;
    font-size: 0.88rem;
  }

  .muted {
    color: var(--fg-muted);
  }
</style>
