<script lang="ts">
  import type { GameApp } from "$lib/game.svelte";
  import { actionControl } from "$lib/legal-action-control";
  import LegalActionForm from "./LegalActionForm.svelte";
  import ActionTransactions from "./ActionTransactions.svelte";

  let {
    app,
  }: {
    app: GameApp;
    onCmd: (command: string) => void;
  } = $props();

</script>

<div class="actions">
  <div class="head">
    <strong>Available actions</strong>
    <button type="button" class="ghost" onclick={() => app.refreshLegalActions()}>Refresh</button>
  </div>

  {#if app.actionRequests.notice}<p role="alert">{app.actionRequests.notice}</p>{/if}
  <ActionTransactions requests={app.actionRequests} />
  {#if !app.actions.loaded}
    <p class="muted">Loading current actions…</p>
  {:else if app.actions.actions.length === 0}
    <p class="muted">No actions are currently available.</p>
  {:else}
    <div class="list">
      {#each app.actions.actions as action (action.offer)}
        {@const control = actionControl(action)}
        {#if control}
          <LegalActionForm {action} {control} requests={app.actionRequests}
            submit={values => app.submitLegalAction(action, values)} />
        {:else}
        <button
          type="button"
          class="action"
          title={action.presentation.help}
          disabled
        >
          <strong>{action.presentation.label}</strong>
          {#if action.presentation.help}<span>{action.presentation.help}</span>{/if}
          <span>This action is unavailable in these controls.</span>
        </button>
        {/if}
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

  .action:disabled { cursor: default; opacity: 0.6; }

  .action span,
  .muted {
    color: var(--fg-muted);
    font-size: 0.82rem;
  }
</style>
