<script lang="ts">
  import type { GameApp } from "$lib/game.svelte";

  let {
    app,
    onCmd,
  }: {
    app: GameApp;
    onCmd: (command: string) => void;
  } = $props();

  function invoke(verb: string): void {
    // The line adapter re-enumerates the current offer before submitting its
    // opaque ID. This deliberately cannot turn the displayed token into a
    // client-authored exact action or stale authorization cache.
    onCmd(`+act ${verb}`);
  }
</script>

<div class="actions">
  <div class="head">
    <strong>Available actions</strong>
    <button type="button" class="ghost" onclick={() => app.refreshLegalActions()}>Refresh</button>
  </div>

  {#if !app.actions.loaded}
    <p class="muted">Loading current actions…</p>
  {:else if app.actions.actions.length === 0}
    <p class="muted">No actions are currently available.</p>
  {:else}
    <div class="list">
      {#each app.actions.actions as action (action.offer)}
        <button
          type="button"
          class="action"
          title={action.presentation.help}
          onclick={() => invoke(action.presentation.verb)}
        >
          <strong>{action.presentation.label}</strong>
          {#if action.presentation.help}<span>{action.presentation.help}</span>{/if}
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .actions {
    height: 100%;
    overflow: auto;
    padding: 0.7rem;
  }

  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    margin-bottom: 0.7rem;
  }

  .ghost {
    border: 1px solid var(--border);
    border-radius: 6px;
    background: transparent;
    color: var(--fg-muted);
    cursor: pointer;
    padding: 0.25rem 0.55rem;
  }

  .list {
    display: grid;
    gap: 0.5rem;
  }

  .action {
    display: grid;
    gap: 0.2rem;
    width: 100%;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg-raised);
    color: var(--fg);
    cursor: pointer;
    padding: 0.65rem;
    text-align: left;
  }

  .action:hover,
  .action:focus-visible {
    border-color: var(--accent);
  }

  .action span,
  .muted {
    color: var(--fg-muted);
    font-size: 0.82rem;
  }
</style>
