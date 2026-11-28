<script lang="ts">
  // Collapsible HUD Quest Tracker bar mounted on the Scene screen.
  // Shows the currently tracked active quest, 7-step Guided Tour progress pill,
  // turn-by-turn 1-click station route step, objective checklist, 1-click Verify/Complete/Abandon,
  // or a quick-accept prompt when a quest NPC in the room has an available quest.
  import type { GameApp } from "$lib/game.svelte";
  import {
    GUIDED_TOUR_KEYS,
    evaluateSailingPreflightReadiness,
    extractActiveDestinationPort,
    extractActiveSurfaceMiningStatus,
    extractActiveTradeNeed,
    extractSurfaceMiningPrepActions,
    formatQuestHoldReadinessSummary,
    inferStationPortFromRoomEntry,
    needsOriginPortProcurement,
    needsSurfaceExpeditionBoarding,
    resolveNextGuidedOfferDestination,
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
  const trackedStep = $derived(
    tracked ? (GUIDED_TOUR_KEYS.indexOf(tracked.key) >= 0 ? GUIDED_TOUR_KEYS.indexOf(tracked.key) + 1 : null) : null,
  );
  const suggested = $derived(q.suggestedAction);
  const tour = $derived(q.guidedTourProgress);
  const firstOffer = $derived(
    q.available.find(
      (item) => item.category === "guided" || GUIDED_TOUR_KEYS.includes(item.key),
    ) ??
      q.available[0] ??
      null,
  );
  const firstOfferStep = $derived(
    firstOffer
      ? GUIDED_TOUR_KEYS.indexOf(firstOffer.key) >= 0
        ? GUIDED_TOUR_KEYS.indexOf(firstOffer.key) + 1
        : null
      : null,
  );
  const extraOfferCount = $derived(Math.max(0, q.available.length - 1));
  const otherReadyHere = $derived(
    tracked
      ? q.active.filter((item) => item.key !== tracked.key && item.canFinishNow)
      : [],
  );
  const nextOffer = $derived(q.nextGuidedOffer);
  const currentPort = $derived(
    app.tacsim?.ambient?.port ??
      app.tacsim?.trade?.hullPort ??
      inferStationPortFromRoomEntry(
        (app.tacsim?.view3dOpen ? app.tacsim?.physicalLocationKey : null) ??
          app.tacsim?.serverStationRoomEntry,
      ),
  );
  const nextOfferDest = $derived(
    !tracked && !firstOffer && nextOffer && (currentPort || app.tacsim?.isAboardShip)
      ? resolveNextGuidedOfferDestination(nextOffer, currentPort)
      : null,
  );
  const doneCount = $derived(
    tracked ? tracked.checkpoints.filter((cp) => cp.checked).length : 0,
  );
  const trackedIndex = $derived(
    tracked ? Math.max(0, q.active.findIndex((item) => item.key === tracked.key)) : 0,
  );
  const liveStowage = $derived(
    app.tacsim?.ambient?.ownStowage ??
      app.tacsim?.ambient?.stowage ??
      (app.tacsim?.trade?.hullId ? app.tacsim.trade.stowage : null),
  );
  const holdReadiness = $derived(
    tracked ? formatQuestHoldReadinessSummary(tracked, liveStowage) : null,
  );
  const trackedDest = $derived(
    tracked ? extractActiveDestinationPort([tracked], tracked.key) : null,
  );
  const trackedPreflight = $derived(
    tracked && trackedDest
      ? evaluateSailingPreflightReadiness(
          trackedDest,
          extractActiveTradeNeed([tracked], tracked.key),
          liveStowage,
        )
      : null,
  );
  const trackedMining = $derived(
    tracked ? extractActiveSurfaceMiningStatus([tracked], tracked.key) : null,
  );
  const trackedMiningPrep = $derived(
    tracked && !tracked.canFinishNow && !tracked.readyToFinish
      ? extractSurfaceMiningPrepActions(trackedMining)
      : [],
  );
  const routeCrumb = $derived.by(() => {
    const tacsim = app.tacsim;
    if (!tacsim) return null;
    if (!tracked) {
      if (firstOffer || !nextOffer) return null;
      if (tacsim.isAboardShip) {
        if (tacsim.seatedAtBridgeHelm) return null;
        const settledPad = Boolean(tacsim.ambient?.settledPad);
        const dockedPortId =
          tacsim.ambient?.docked && !settledPad
            ? normalizePortVenueTarget(tacsim.ambient?.port ?? tacsim.trade?.hullPort)
            : null;
        const targetPort = inferStationPortFromRoomEntry(nextOffer.giverRoomEntry);
        const targetComp =
          dockedPortId && targetPort && dockedPortId === targetPort ? "airlock" : "bridge";
        const currentComp =
          (tacsim.view3dOpen ? tacsim.physicalLocationKey : null) ?? tacsim.serverShipCompartment;
        return computeShipQuestBreadcrumb(tacsim.shipTemplate, currentComp, targetComp);
      }
      const currentEntry =
        (tacsim.view3dOpen ? tacsim.physicalLocationKey : null) ?? tacsim.serverStationRoomEntry;
      return computeNextQuestExitBreadcrumb(
        tacsim.stationPack,
        currentEntry,
        nextOffer.giverRoomEntry,
      );
    }
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
      {#if trackedStep !== null}
        <span class="step-pill">Step {trackedStep}/7</span>
      {/if}
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
      {#if otherReadyHere.length > 0}
        <button
          type="button"
          class="ready-here-pill"
          onclick={() => {
            const target = otherReadyHere[0]!;
            if (otherReadyHere.length === 1 && target.finishCmd) {
              onCmd(target.finishCmd);
            } else if (otherReadyHere.length === 1) {
              q.track(target.key);
              q.openQuestDialog(target.key);
            } else {
              q.openLog("active");
            }
          }}
          title={otherReadyHere.length === 1
            ? `Complete ${otherReadyHere[0]!.title} in this room`
            : `${otherReadyHere.length} other quests ready to turn in here`}
        >
          [?] {otherReadyHere.length === 1
            ? `Turn In: ${otherReadyHere[0]!.title}`
            : `${otherReadyHere.length} ready to turn in`}
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
        {#if holdReadiness}
          <span
            class="hold-pill"
            class:ready={holdReadiness.ready}
            aria-label="Tracked quest hold readiness"
          >
            {holdReadiness.tag} {holdReadiness.text}
          </span>
        {/if}
        {#if trackedPreflight && trackedDest}
          <span
            class="hold-pill"
            class:ready={trackedPreflight.ready}
            aria-label="Tracked quest sailing preflight"
          >
            {trackedPreflight.ready ? "[PRE-FLIGHT OK]" : "[CHECK BEFORE UNDOCK]"} Fuel {trackedPreflight.propellantTonnes}t · Provisions {trackedPreflight.provisionsDays}d → {trackedDest.portLabel}
          </span>
        {/if}
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
          {:else if routeCrumb.nextExitCommand === "+tv/bay in" || routeCrumb.nextExitCommand === "+tv/bay"}
            Enter Vehicle Bay
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
        {#if holdReadiness}
          <div
            class="hold-strip"
            class:ready={holdReadiness.ready}
            aria-label="Tracked quest hold readiness"
          >
            <span class="hold-tag">{holdReadiness.tag}</span>
            <span>{holdReadiness.text}</span>
          </div>
        {/if}
        {#if trackedPreflight && trackedDest}
          <div
            class="hold-strip"
            class:ready={trackedPreflight.ready}
            aria-label="Tracked quest sailing preflight"
          >
            <span class="hold-tag">
              {trackedPreflight.ready ? "[PRE-FLIGHT OK]" : "[CHECK BEFORE UNDOCK]"}
            </span>
            <span>
              {trackedPreflight.propellantOk ? "✓" : "✗"} Propellant {trackedPreflight.propellantTonnes}t · {trackedPreflight.provisionsOk ? "✓" : "✗"} Provisions {trackedPreflight.provisionsDays}d → {trackedDest.portLabel}
            </span>
          </div>
        {/if}
        {#if trackedMiningPrep.length > 0}
          <div class="prep-strip" aria-label="Tracked quest prospecting prep">
            <span class="hold-tag">[PREP]</span>
            {#each trackedMiningPrep as act (act.command)}
              <button
                type="button"
                class="prep-chip"
                title={`Prefill ${act.command}`}
                onclick={() => app.focusComposer?.(act.command)}
              >
                {act.label} ({act.command})
              </button>
            {/each}
          </div>
        {/if}
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
      {#if firstOfferStep !== null}
        <span class="step-pill">Step {firstOfferStep}/7</span>
      {/if}
      <span class="inline-next">
        <strong>{firstOffer.giverName}</strong> offers: <em>{firstOffer.title}</em>{#if extraOfferCount > 0}{" "}<span class="extra-count">(+{extraOfferCount} more)</span>{/if}
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
      {#if extraOfferCount > 0}
        <button type="button" class="tbtn" onclick={() => q.openLog("available")}>
          All Offers ({q.available.length})
        </button>
      {/if}
      {#if q.completed.length > 0}
        <button type="button" class="tbtn" onclick={() => q.openLog("completed")}>
          Completed ({q.completed.length})
        </button>
      {/if}
    </div>
  </section>
{:else if nextOffer}
  <section class="tracker offer-hint" aria-label="Next guided quest">
    <div class="tracker-bar">
      <span class="qmark" data-marker="available">!</span>
      <span class="inline-next">
        <strong>Next Guided ({nextOffer.step}/7):</strong> {nextOffer.shortLabel} — speak with <strong>{nextOffer.giverLabel}</strong> ({nextOffer.giverRoomName})
      </span>
      {#if tour}
        <button
          type="button"
          class="tour-pill"
          onclick={() => q.openLog("completed")}
          title="Guided Captain's Tour progress"
        >
          Tour {tour.completed}/{tour.total}
        </button>
      {/if}
      <span class="grow"></span>
      {#if routeCrumb}
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
          {:else if routeCrumb.nextExitCommand === "+tv/bay in" || routeCrumb.nextExitCommand === "+tv/bay"}
            Enter Vehicle Bay
          {:else}
            Walk → {routeCrumb.nextRoomName} ({routeCrumb.hops}h)
          {/if}
        </button>
      {/if}
      {#if nextOfferDest}
        <button
          type="button"
          class="tbtn primary"
          title={`Prefill +ship/plan ${nextOfferDest.portId}`}
          onclick={() => {
            const cmd = `+ship/plan ${nextOfferDest.portId}`;
            if (app.focusComposer) {
              app.focusComposer(cmd);
            } else {
              onCmd(cmd);
            }
          }}
        >
          Sail → {nextOfferDest.portLabel}
        </button>
      {/if}
      <button type="button" class="tbtn" onclick={() => q.openLog("completed")}>
        Quest Log ({q.completed.length})
      </button>
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
      <button type="button" class="tbtn primary" onclick={() => onCmd("+market/jobs")}>
        Jobs Board (+market/jobs)
      </button>
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
  .step-pill {
    display: inline-flex;
    align-items: center;
    padding: 0.05rem 0.38rem;
    border-radius: 999px;
    border: 1px solid rgba(251, 191, 36, 0.5);
    background: rgba(251, 191, 36, 0.14);
    color: #fde68a;
    font-family: var(--mono, monospace);
    font-size: 0.67rem;
    font-weight: 700;
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
  .ready-here-pill {
    display: inline-flex;
    align-items: center;
    padding: 0.08rem 0.48rem;
    border-radius: 999px;
    border: 1px solid #34d399;
    background: rgba(16, 185, 129, 0.22);
    color: #a7f3d0;
    font: inherit;
    font-size: 0.7rem;
    font-weight: 700;
    cursor: pointer;
  }
  .ready-here-pill:hover {
    filter: brightness(1.14);
  }
  .extra-count {
    color: #fde68a;
    font-weight: 700;
    font-size: 0.74rem;
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
  .hold-strip {
    display: inline-flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.35rem;
    padding: 0.2rem 0.5rem;
    border-radius: 5px;
    border: 1px solid rgba(251, 191, 36, 0.45);
    background: rgba(62, 46, 12, 0.36);
    color: #fde68a;
    font-size: 0.72rem;
    align-self: flex-start;
  }
  .hold-strip.ready,
  .hold-pill.ready {
    border-color: rgba(52, 211, 153, 0.6);
    background: rgba(16, 74, 52, 0.38);
    color: #a7f3d0;
  }
  .hold-tag {
    font-family: var(--mono, monospace);
    font-weight: 700;
    color: #fbbf24;
  }
  .hold-strip.ready .hold-tag {
    color: #6ee7b7;
  }
  .hold-pill {
    display: inline-flex;
    align-items: center;
    padding: 0.06rem 0.42rem;
    border-radius: 999px;
    border: 1px solid rgba(251, 191, 36, 0.5);
    background: rgba(62, 46, 12, 0.4);
    color: #fde68a;
    font-family: var(--mono, monospace);
    font-size: 0.68rem;
    font-weight: 600;
  }
  .prep-strip {
    display: inline-flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.3rem;
    padding: 0.18rem 0.45rem;
    border-radius: 5px;
    border: 1px solid rgba(56, 189, 248, 0.4);
    background: rgba(14, 42, 62, 0.4);
    align-self: flex-start;
  }
  .prep-chip {
    background: rgba(18, 60, 88, 0.72);
    border: 1px solid rgba(56, 189, 248, 0.55);
    color: #bae6fd;
    border-radius: 4px;
    padding: 0.1rem 0.42rem;
    font-size: 0.7rem;
    font-family: var(--mono, monospace);
    cursor: pointer;
  }
  .prep-chip:hover {
    background: rgba(24, 82, 118, 0.85);
    color: #f0f9ff;
  }
</style>
