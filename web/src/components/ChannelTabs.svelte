<script lang="ts">
  // The channel list beside the channel view (interface plan I7): "All"
  // (the main stream), then the in-character channels and the
  // out-of-character ones apart, each with its color chip, unread badge and
  // gag marker. A phone folds it into a horizontal strip.
  import type { GameApp } from "$lib/game.svelte";
  import type { ChannelMeta } from "$lib/stores/channels.svelte";

  let { app }: { app: GameApp } = $props();

  const groups: { label: string; ic: boolean }[] = [
    { label: "IC", ic: true },
    { label: "OOC", ic: false },
  ];

  function isActive(chan: ChannelMeta): boolean {
    return app.channels.active === chan.name.toLowerCase();
  }
</script>

<nav class="list" aria-label="channels">
  <button
    type="button"
    class="tab"
    class:active={app.channels.active === null}
    aria-current={app.channels.active === null ? "true" : undefined}
    onclick={() => app.channels.activate(null)}
  >
    All
    {#if app.channels.active !== null && app.channels.totalUnread > 0}
      <span class="badge">{app.channels.totalUnread}</span>
    {/if}
  </button>
  {#each groups as group (group.label)}
    {@const chans = app.channels.channelsWhere(group.ic)}
    <div class="group" class:ic={group.ic} class:ooc={!group.ic}>
      <div class="heading">{group.label}</div>
      {#if chans.length === 0}
        <div class="none">none</div>
      {/if}
      {#each chans as chan (chan.name)}
        {@const unread = app.channels.unreadOf(chan.name)}
        <button
          type="button"
          class="tab"
          class:active={isActive(chan)}
          class:gagged={chan.gagged}
          aria-current={isActive(chan) ? "true" : undefined}
          title={chan.gagged ? `${chan.name} (gagged)` : chan.name}
          onclick={() => app.openChannelTab(chan.name)}
        >
          <span class="chip" style={chan.colorHex ? `background:${chan.colorHex}` : ""}></span>
          {chan.name}
          {#if unread > 0 && !isActive(chan)}
            <span class="badge">{unread > 99 ? "99+" : unread}</span>
          {/if}
        </button>
      {/each}
    </div>
  {/each}
</nav>

<style>
  .list {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    padding: 0.5rem 0.4rem;
    border-right: 1px solid var(--border);
    background: var(--bg-panel);
    overflow-y: auto;
    scrollbar-width: thin;
  }

  .group {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    margin-top: 0.5rem;
    padding-left: 0.35rem;
    border-left: 2px solid var(--border);
  }

  .group.ic {
    border-left-color: var(--success);
  }

  .group.ooc {
    border-left-color: var(--c-npc);
  }

  .heading {
    font-size: 0.7rem;
    font-weight: 700;
    letter-spacing: 0.08em;
    color: var(--fg-faint);
    padding: 0 0.4rem;
  }

  .none {
    font-size: 0.78rem;
    color: var(--fg-faint);
    padding: 0.1rem 0.5rem;
  }

  .tab {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.3rem 0.5rem;
    border: 1px solid transparent;
    border-radius: 6px;
    background: none;
    color: var(--fg-muted);
    cursor: pointer;
    font: inherit;
    font-size: 0.85rem;
    text-align: left;
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
    margin-left: auto;
    background: var(--accent);
    color: var(--bg);
    border-radius: 999px;
    font-size: 0.68rem;
    font-weight: 700;
    padding: 0 0.35rem;
    min-width: 1.1rem;
    text-align: center;
  }

  @media (max-width: 800px) {
    .list {
      flex-direction: row;
      flex-wrap: wrap;
      align-items: center;
      border-right: none;
      border-bottom: 1px solid var(--border);
      padding: 0.35rem 0.5rem;
    }

    .group {
      flex-direction: row;
      align-items: center;
      margin-top: 0;
      border-left: none;
      padding-left: 0;
    }

    .heading {
      padding: 0 0.25rem;
    }
  }
</style>
