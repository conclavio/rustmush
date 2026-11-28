<script lang="ts">
  // Collapsible HUD Quest Tracker bar mounted on the Scene screen.
  // Shows the currently tracked active quest, 7-step Guided Tour progress pill,
  // turn-by-turn 1-click station route step, objective checklist, 1-click Verify/Complete/Abandon,
  // or a quick-accept prompt when a quest NPC in the room has an available quest.
  import type { GameApp } from "$lib/game.svelte";
  import {
    needsOriginPortProcurement,
    needsSurfaceExpeditionBoarding,
    resolveQuestShipTargetCompartment,
  } from "$lib/stores/quests.svelte";
  import {
    computeNextQuestExitBreadcrumb,
    computeShipQuestBreadcrumb,
    normalizePortVenueTarget,
  } from "$lib/tacsim/spatial";

  let { app, onCmd }: { app: GameApp; onCmd: (cmd: string) => void } = $props();

  const q = $derived(app.quests);
  const tracked = $derived(q.trackedQuest);
  const suggested = $derived(q.suggestedAction);
  const tour = $derived(q.guidedTourProgress);
  const firstOffer = $derived(q.available[0] ?? null);
  const doneCount = $derived(
    tracked ? tracked.checkpoints.filter((cp) => cp.checked).length : 0,
  );
  const trackedIndex = $derived(
    tracked ? Math.max(0, q.active.findIndex((item) => item.key === tracked.key)) : 0,
  );
  const routeCrumb = $derived.by(() => {
    const tacsim = app.tacsim;
    if (!tracked || !tacsim) return null;
    if (tacsim.isAboardShip) {
      if (tacsim.seatedAtBridgeHelm) return null;
      const settledPad = Boolean(tacsim.ambient?.settledPad);
      const dockedPortId =
        tacsim.ambient?.docked && !settledPad
          ? normalizePortVenueTarget(tacsim.ambient?.port ?? tacsim.trade?.hullPort)
          : null;
      const targetComp = resolveQuestShipTargetCompartment(
        q.active,
        q.trackedKey,
        dockedPortId,
        settledPad,
      );
      const currentComp =
        (tacsim.view3dOpen ? tacsim.physicalLocationKey : null) ?? tacsim.serverShipCompartment;
      return computeShipQuestBreadcrumb(tacsim.shipTemplate, currentComp, targetComp);
    }
    const targetEntry =
      tracked.nextRoomEntry ||
      tracked.checkpoints.find((c) => !c.checked && !c.liveReady && c.roomEntry)?.roomEntry ||
      null;
    const currentEntry =
      (tacsim.view3dOpen ? tacsim.physicalLocationKey : null) ?? tacsim.serverStationRoomEntry;
    const liveStowage =
      tacsim.ambient?.ownStowage ??
      tacsim.ambient?.stowage ??
      (tacsim.trade?.hullId ? tacsim.trade.stowage : null);
    const needsOriginExchange = needsOriginPortProcurement(q.active, q.trackedKey, liveStowage);
    const needsShipExpedition = needsSurfaceExpeditionBoarding(q.active, q.trackedKey);
    return computeNextQuestExitBreadcrumb(
      tacsim.stationPack,
      currentEntry,
      targetEntry,
      needsOriginExchange,
      needsShipExpedition,
    );
  });
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
      {#if q.active.length > 1}
        <button
          type="button"
          class="cycle-pill"
          onclick={() => q.cycleTracked()}
          title="Cycle tracked active quest"
        >
          {trackedIndex + 1}/{q.active.length} ▸
        </button>
      {/if}
      {#if tour}
        <button
          type="button"
          class="tour-pill"
          class:mastered={tour.allDone}
          onclick={() => q.openLog("active")}
          title="Guided Captain's Tour progress"
        >
          {tour.allDone ? `Corridor Captain ${tour.completed}/${tour.total}` : `Tour ${tour.completed}/${tour.total}`}
        </button>
      {/if}
      {#if firstOffer}
        <button
          type="button"
          class="offer-pill"
          onclick={() =>
            q.available.length === 1 && firstOffer.giverId
              ? q.openNpcDialog(firstOffer.giverId)
              : q.openLog("available")}
          title="Available quest offers in this room"
        >
          [!] {q.available.length === 1 ? "1 offer here" : `${q.available.length} offers here`}
        </button>
      {/if}
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
      {:else if routeCrumb}
        <button
          type="button"
          class="tbtn route"
          title={`Quest route: ${routeCrumb.hops} ${routeCrumb.hops === 1 ? "hop" : "hops"} to ${routeCrumb.targetRoomName} (${routeCrumb.nextExitCommand})`}
          onclick={() => onCmd(routeCrumb.nextExitCommand)}
        >
          {#if routeCrumb.nextExitCommand === "+ship/board"}
            Board Ship
          {:else if routeCrumb.nextExitCommand === "+ship/leave"}
            Cycle Ashore
          {:else}
            Walk → {routeCrumb.nextRoomName} ({routeCrumb.hops}h)
          {/if}
        </button>
      {:else if suggested}
        <button
          type="button"
          class="tbtn primary"
          title={`Prefill ${suggested.command}`}
          onclick={() => app.focusComposer?.(suggested.command)}
        >
          {suggested.label}
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
      {#if tour}
        <span class="tour-pill">
          Tour {tour.completed}/{tour.total}
        </span>
      {/if}
      <span class="grow"></span>
      {#if firstOffer.acceptCmd}
        <button type="button" class="tbtn primary" onclick={() => onCmd(firstOffer.acceptCmd)}>
          Accept Quest
        </button>
      {/if}
      <button type="button" class="tbtn" onclick={() => q.openNpcDialog(firstOffer.giverId)}>
        Open Quest Dialog
      </button>
      {#if q.completed.length > 0}
        <button type="button" class="tbtn" onclick={() => q.openLog("completed")}>
          Completed ({q.completed.length})
        </button>
      {/if}
    </div>
  </section>
{:else if tour?.allDone}
  <section class="tracker mastered-hint" aria-label="Guided tour complete">
    <div class="tracker-bar">
      <span class="qmark mastered-mark" aria-hidden="true">★</span>
      <span class="inline-next mastered-text">
        <strong>Corridor Captain</strong> — Guided Tour Complete ({tour.completed}/{tour.total})
      </span>
      <span class="grow"></span>
      <button type="button" class="tbtn" onclick={() => q.openLog("completed")}>
        Completed ({q.completed.length})
      </button>
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
  .tbtn.route {
    background: rgba(251, 191, 36, 0.18);
    border-color: #fbbf24;
    color: #fef3c7;
  }
  .tbtn.route:hover {
    filter: brightness(1.14);
  }
  .tour-pill {
    display: inline-flex;
    align-items: center;
    padding: 0.08rem 0.45rem;
    border-radius: 999px;
    border: 1px solid rgba(251, 191, 36, 0.55);
    background: rgba(251, 191, 36, 0.12);
    color: #fde68a;
    font: inherit;
    font-size: 0.7rem;
    font-weight: 700;
    cursor: pointer;
  }
  .tour-pill.mastered {
    border-color: #34d399;
    background: rgba(16, 185, 129, 0.18);
    color: #a7f3d0;
  }
  .cycle-pill {
    display: inline-flex;
    align-items: center;
    padding: 0.06rem 0.42rem;
    border-radius: 999px;
    border: 1px solid rgba(148, 163, 184, 0.45);
    background: rgba(30, 41, 59, 0.72);
    color: #cbd5e1;
    font: inherit;
    font-size: 0.69rem;
    font-weight: 700;
    cursor: pointer;
  }
  .cycle-pill:hover {
    border-color: #fbbf24;
    color: #fef3c7;
  }
  .offer-pill {
    display: inline-flex;
    align-items: center;
    padding: 0.08rem 0.48rem;
    border-radius: 999px;
    border: 1px solid #fbbf24;
    background: rgba(251, 191, 36, 0.2);
    color: #fef3c7;
    font: inherit;
    font-size: 0.7rem;
    font-weight: 700;
    cursor: pointer;
  }
  .offer-pill:hover {
    filter: brightness(1.14);
  }
  .tracker.mastered-hint {
    background: linear-gradient(90deg, rgba(16, 185, 129, 0.14), rgba(15, 23, 36, 0.92));
  }
  .qmark.mastered-mark {
    background: #10b981;
    color: #031911;
  }
  .mastered-text {
    color: #a7f3d0;
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
