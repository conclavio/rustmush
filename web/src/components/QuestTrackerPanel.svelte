<script lang="ts">
  // Collapsible HUD Quest Tracker bar mounted on the Scene screen.
  // Shows the currently tracked active quest, objective checklist, 1-click Verify/Complete/Abandon,
  // or a subtle prompt when a quest NPC in the room has an available quest.
  import type { GameApp } from "$lib/game.svelte";

  let { app, onCmd }: { app: GameApp; onCmd: (cmd: string) => void } = $props();

  const q = $derived(app.quests);
  const tracked = $derived(q.trackedQuest);
  const firstOffer = $derived(q.available[0] ?? null);
  const doneCount = $derived(
    tracked ? tracked.checkpoints.filter((cp) => cp.checked).length : 0,
  );
</script>

{#if tracked}
  <section class="tracker" aria-label="Quest tracker">
    <div class="tracker-bar">
      <button
        type="button"
        class="toggle"
        onclick={() => (q.trackerCollapsed = !q.trackerCollapsed)}
        title={q.trackerCollapsed ? "Expand quest tracker" : "Collapse quest tracker"}
      >
        {q.trackerCollapsed ? "▸" : "▾"}
      </button>
      <span class="qmark" data-marker={tracked.canFinishNow || tracked.readyToFinish ? "turn_in" : "in_progress"}>?</span>
      <button type="button" class="qtitle" onclick={() => q.openQuestDialog(tracked.key)}>
        {tracked.title}
      </button>
      <span class="progress">
        ({doneCount}/{tracked.checkpoints.length})
      </span>
      {#if q.trackerCollapsed}
        <span class="inline-next">{tracked.nextStep}</span>
      {/if}
      <span class="grow"></span>
      {#if tracked.canFinishNow}
        <button type="button" class="tbtn complete" onclick={() => onCmd(tracked.finishCmd)}>
          Complete
        </button>
      {:else if tracked.canCheckNow}
        <button type="button" class="tbtn primary" onclick={() => onCmd(tracked.checkCmd)}>
          Verify Step
        </button>
      {/if}
      <button type="button" class="tbtn" onclick={() => q.openLog("active")}>
        Quest Log ({q.active.length})
      </button>
    </div>

    {#if !q.trackerCollapsed}
      <div class="tracker-body">
        <ul class="obj-list">
          {#each tracked.checkpoints as cp (cp.id)}
            <li class:done={cp.checked} class:ready={!cp.checked && cp.liveReady}>
              <span class="icon">{cp.checked ? "✓" : cp.liveReady ? "◆" : "○"}</span>
              <span class="label">{cp.title}</span>
              {#if cp.roomName && !cp.checked}
                <span class="loc">· {cp.roomName}</span>
              {/if}
            </li>
          {/each}
        </ul>
        <div class="next-row">
          <span class="next-text">
            <strong>Next:</strong> {tracked.nextStep}
            {#if tracked.readyToFinish}
              — speak with {tracked.turnInName}
            {/if}
          </span>
          <button type="button" class="link-btn" onclick={() => q.openQuestDialog(tracked.key)}>
            Details / Abandon
          </button>
        </div>
      </div>
    {/if}
  </section>
{:else if firstOffer}
  <section class="tracker offer-hint" aria-label="Available quest">
    <div class="tracker-bar">
      <span class="qmark" data-marker="available">!</span>
      <span class="inline-next">
        <strong>{firstOffer.giverName}</strong> offers: <em>{firstOffer.title}</em>
      </span>
      <span class="grow"></span>
      <button type="button" class="tbtn primary" onclick={() => q.openNpcDialog(firstOffer.giverId)}>
        Open Quest Dialog
      </button>
      {#if q.completed.length > 0}
        <button type="button" class="tbtn" onclick={() => q.openLog("completed")}>
          Completed ({q.completed.length})
        </button>
      {/if}
    </div>
  </section>
{/if}

<style>
  .tracker {
    border-bottom: 1px solid var(--border, #263447);
    background: linear-gradient(90deg, rgba(212, 166, 74, 0.09), rgba(15, 23, 36, 0.92));
    font-size: 0.8rem;
  }
  .tracker-bar {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.3rem 0.75rem;
    flex-wrap: wrap;
  }
  .toggle {
    background: none;
    border: none;
    color: var(--fg-muted, #94a3b8);
    cursor: pointer;
    padding: 0 0.2rem;
    font-size: 0.8rem;
  }
  .qmark {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.15rem;
    height: 1.15rem;
    border-radius: 999px;
    font-weight: 800;
    font-size: 0.78rem;
  }
  .qmark[data-marker="available"],
  .qmark[data-marker="turn_in"] {
    background: #fbbf24;
    color: #161103;
  }
  .qmark[data-marker="in_progress"] {
    background: #475569;
    color: #e2e8f0;
  }
  .qtitle {
    background: none;
    border: none;
    color: #f7d98a;
    font: inherit;
    font-weight: 700;
    cursor: pointer;
    padding: 0;
  }
  .qtitle:hover {
    text-decoration: underline;
  }
  .progress {
    color: var(--fg-muted, #94a3b8);
    font-size: 0.75rem;
  }
  .inline-next {
    color: var(--fg-muted, #cbd5e1);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .grow {
    flex: 1;
  }
  .tbtn {
    border: 1px solid var(--border, #334155);
    border-radius: 5px;
    background: var(--bg-input, #1e293b);
    color: var(--fg, #e2e8f0);
    font: inherit;
    font-size: 0.74rem;
    font-weight: 600;
    padding: 0.14rem 0.5rem;
    cursor: pointer;
  }
  .tbtn.primary {
    background: #d49422;
    border-color: #f5b935;
    color: #140f02;
  }
  .tbtn.complete {
    background: #10b981;
    border-color: #34d399;
    color: #031911;
  }
  .tracker-body {
    padding: 0.25rem 0.75rem 0.45rem 1.8rem;
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }
  .obj-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
  }
  .obj-list li {
    display: flex;
    align-items: baseline;
    gap: 0.35rem;
    color: var(--fg-muted, #cbd5e1);
  }
  .obj-list li.done {
    color: #6ee7b7;
    text-decoration: line-through;
    opacity: 0.8;
  }
  .obj-list li.ready {
    color: #fde68a;
    font-weight: 600;
  }
  .loc {
    color: #7dd3fc;
    font-size: 0.74rem;
  }
  .next-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    font-size: 0.75rem;
    color: var(--fg-muted, #94a3b8);
  }
  .link-btn {
    background: none;
    border: none;
    color: #7dd3fc;
    font: inherit;
    font-size: 0.74rem;
    cursor: pointer;
    padding: 0;
  }
  .link-btn:hover {
    text-decoration: underline;
  }
</style>
