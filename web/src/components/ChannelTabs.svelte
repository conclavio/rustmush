<script lang="ts">
  // Channel tab bar above the scrollback: "All" (main stream) plus one tab
  // per joined channel — color chip, unread badge, gag marker.
  import type { GameApp } from "$lib/game.svelte";

  let { app }: { app: GameApp } = $props();
</script>

<div class="tabs" role="tablist" aria-label="channels">
  <button
    type="button"
    role="tab"
    class="tab"
    class:active={app.channels.active === null}
    aria-selected={app.channels.active === null}
    onclick={() => app.channels.activate(null)}
  >
    All
    {#if app.channels.active !== null && app.channels.totalUnread > 0}
      <span class="badge">{app.channels.totalUnread}</span>
    {/if}
  </button>
  {#each app.channels.list as chan (chan.name)}
    {@const key = chan.name.toLowerCase()}
    {@const unread = app.channels.unreadOf(chan.name)}
    <button
      type="button"
      role="tab"
      class="tab"
      class:active={app.channels.active === key}
      class:gagged={chan.gagged}
      aria-selected={app.channels.active === key}
      title={chan.gagged ? `${chan.name} (gagged)` : chan.name}
      onclick={() => app.openChannelTab(chan.name)}
    >
      <span class="chip" style={chan.colorHex ? `background:${chan.colorHex}` : ""}></span>
      {chan.name}
      {#if unread > 0 && app.channels.active !== key}
        <span class="badge">{unread > 99 ? "99+" : unread}</span>
      {/if}
    </button>
  {/each}
</div>

<style>
  .tabs {
    display: flex;
    gap: 0.25rem;
    padding: 0.35rem 0.5rem 0;
    border-bottom: 1px solid var(--border);
    background: var(--bg-panel);
    overflow-x: auto;
    scrollbar-width: thin;
  }

  .tab {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.3rem 0.7rem;
    border: 1px solid transparent;
    border-bottom: none;
    border-radius: 8px 8px 0 0;
    background: none;
    color: var(--fg-muted);
    cursor: pointer;
    font: inherit;
    font-size: 0.85rem;
    white-space: nowrap;
  }

  .tab:hover {
    color: var(--fg);
  }

  .tab.active {
    background: var(--bg);
    border-color: var(--border);
    color: var(--fg);
  }

  .tab.gagged {
    opacity: 0.55;
    text-decoration: line-through;
  }

  .chip {
    width: 0.55rem;
    height: 0.55rem;
    border-radius: 50%;
    background: var(--fg-faint);
    flex: none;
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
</style>
