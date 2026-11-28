<script lang="ts">
  // WoW-style NPC Quest Dialog & Full Quest Log Modal (Active / Available Here / Completed).
  // Every button dispatches a server-signed `+quest/...` command via `onCmd`.
  import type { GameApp } from "$lib/game.svelte";
  import {
    GUIDED_TOUR_KEYS,
    evaluateCheckpointLiveBadge,
    evaluateSailingPreflightReadiness,
    extractActiveDestinationPort,
    extractActiveSurfaceMiningStatus,
    extractActiveTradeNeed,
    extractQuestSuggestedAction,
    extractSurfaceMiningPrepActions,
    formatQuestHoldReadinessSummary,
    resolveNextGuidedOfferDestination,
    type ActiveQuestView,
    type AvailableQuestView,
  } from "$lib/stores/quests.svelte";

  function guidedStepNum(key: string): number | null {
    const idx = (GUIDED_TOUR_KEYS as readonly string[]).indexOf(key);
    return idx >= 0 ? idx + 1 : null;
  }

  let { app, onCmd }: { app: GameApp; onCmd: (cmd: string) => void } = $props();

  let confirmAbandonKey = $state<string | null>(null);
  let confirmDeclineKey = $state<string | null>(null);

  const q = $derived(app.quests);
  const tour = $derived(q.guidedTourProgress);
  const roadmap = $derived(tour ? q.guidedTourRoadmap : []);
  const nextOffer = $derived(q.nextGuidedOffer);
  const nextOfferDest = $derived(
    resolveNextGuidedOfferDestination(
      nextOffer,
      app.chart?.frame?.chart?.vantage?.port ||
        app.chart?.frame?.chart?.vantage?.body ||
        app.tacsim?.ambient?.port ||
        app.tacsim?.trade?.hullPort ||
        app.tacsim?.trade?.venueId ||
        null,
    ),
  );
  const liveStowage = $derived(
    app.tacsim?.ambient?.ownStowage ??
      app.tacsim?.ambient?.stowage ??
      (app.tacsim?.trade?.hullId ? app.tacsim.trade.stowage : null),
  );

  function prefillAndClose(cmd: string): void {
    confirmAbandonKey = null;
    confirmDeclineKey = null;
    q.closeModals();
    app.focusComposer?.(cmd);
  }

  const npcMarker = $derived(
    q.dialogNpcId ? q.roomMarkers.find((m) => m.npcId === q.dialogNpcId) ?? null : null,
  );

  const npcActiveQuests = $derived(
    q.dialogNpcId
      ? q.active
          .filter((item) => item.turnInId === q.dialogNpcId || item.giverId === q.dialogNpcId)
          .sort((a, b) => {
            const ra = a.canFinishNow || a.readyToFinish || a.canCheckNow ? 0 : 1;
            const rb = b.canFinishNow || b.readyToFinish || b.canCheckNow ? 0 : 1;
            return ra - rb;
          })
      : [],
  );

  function isGuidedQuest(item: { key: string; category?: string }): boolean {
    return item.category === "guided" || guidedStepNum(item.key) !== null;
  }

  const npcAvailableQuests = $derived(
    q.dialogNpcId
      ? q.available
          .filter((item) => item.giverId === q.dialogNpcId)
          .sort((a, b) => (isGuidedQuest(a) ? 0 : 1) - (isGuidedQuest(b) ? 0 : 1))
      : [],
  );

  const orderedActiveLog = $derived(
    [...q.active].sort((a, b) => {
      const rank = (item: ActiveQuestView) =>
        item.canFinishNow
          ? 0
          : item.canCheckNow || item.readyToFinish
            ? 1
            : isGuidedQuest(item)
              ? 2
              : 3;
      return rank(a) - rank(b);
    }),
  );

  const orderedAvailableLog = $derived(
    [...q.available].sort(
      (a, b) => (isGuidedQuest(a) ? 0 : 1) - (isGuidedQuest(b) ? 0 : 1),
    ),
  );

  const selectedActive = $derived<ActiveQuestView | null>(
    q.dialogQuestKey
      ? q.active.find((item) => item.key === q.dialogQuestKey) ?? null
      : npcActiveQuests.find((item) => item.canFinishNow || item.canCheckNow) ??
          (npcAvailableQuests.length === 0 ? npcActiveQuests[0] ?? null : null),
  );

  const selectedOffer = $derived<AvailableQuestView | null>(
    selectedActive
      ? null
      : q.dialogQuestKey
        ? q.available.find((item) => item.key === q.dialogQuestKey) ?? null
        : npcAvailableQuests[0] ?? null,
  );

  function selectDialogQuest(key: string): void {
    confirmAbandonKey = null;
    confirmDeclineKey = null;
    q.dialogQuestKey = key;
  }

  function switchLogTab(tab: "active" | "available" | "completed"): void {
    confirmAbandonKey = null;
    confirmDeclineKey = null;
    q.logTab = tab;
  }

  function runAndReset(cmd: string, closeDialog = false): void {
    confirmAbandonKey = null;
    confirmDeclineKey = null;
    onCmd(cmd);
    if (closeDialog) {
      q.closeModals();
    }
  }

  function handleBackdropKey(ev: KeyboardEvent): void {
    if (ev.key === "Escape") {
      ev.stopPropagation();
      confirmAbandonKey = null;
      confirmDeclineKey = null;
      q.closeModals();
    }
  }
</script>

{#if q.dialogOpen}
  <div
    class="quest-backdrop"
    role="presentation"
    onclick={() => q.closeModals()}
    onkeydown={handleBackdropKey}
  >
    <div
      class="quest-modal dialog"
      role="dialog"
      aria-label="Quest dialog"
      tabindex="-1"
      onclick={(ev) => ev.stopPropagation()}
      onkeydown={handleBackdropKey}
    >
      <header class="qhdr">
        <div class="qhdr-title">
          {#if selectedOffer}
            <span class="qbadge" data-marker="available">!</span>
          {:else if selectedActive?.canFinishNow || selectedActive?.readyToFinish || selectedActive?.canCheckNow}
            <span class="qbadge" data-marker="turn_in">?</span>
          {:else if selectedActive}
            <span class="qbadge" data-marker="in_progress">?</span>
          {:else if npcMarker}
            <span class="qbadge" data-marker={npcMarker.marker}>{npcMarker.symbol}</span>
          {:else}
            <span class="qbadge" data-marker="in_progress">?</span>
          {/if}
          <div>
            <h2>{selectedActive?.title ?? selectedOffer?.title ?? npcMarker?.npcName ?? "Quest"}</h2>
            <p class="sub">
              {#if selectedActive}
                Giver: {selectedActive.giverName}
                {#if selectedActive.turnInId !== selectedActive.giverId}
                  · Turn in to: {selectedActive.turnInName}
                {/if}
              {:else if selectedOffer}
                Offered by {selectedOffer.giverName}
                {#if selectedOffer.turnInId !== selectedOffer.giverId}
                  · Turn in to: {selectedOffer.turnInName}
                {/if}
              {:else if npcMarker}
                {npcMarker.npcName}
              {/if}
            </p>
          </div>
        </div>
        <button type="button" class="qclose" onclick={() => q.closeModals()} aria-label="Close quest dialog">✕</button>
      </header>

      {#if npcActiveQuests.length + npcAvailableQuests.length > 1}
        <div class="qpicker" role="tablist" aria-label="Quests with this NPC">
          {#each npcActiveQuests as item (item.key)}
            <button
              type="button"
              class="qpick"
              class:selected={selectedActive?.key === item.key}
              onclick={() => selectDialogQuest(item.key)}
            >
              <span class="mini-sym" data-marker={item.canFinishNow || item.readyToFinish || item.canCheckNow ? "turn_in" : "in_progress"}>?</span>
              {item.title}
            </button>
          {/each}
          {#each npcAvailableQuests as item (item.key)}
            <button
              type="button"
              class="qpick"
              class:selected={selectedOffer?.key === item.key}
              onclick={() => selectDialogQuest(item.key)}
            >
              <span class="mini-sym" data-marker="available">!</span>
              {item.title}
            </button>
          {/each}
        </div>
      {/if}

      {#if selectedOffer}
        {@const offerStepNo = guidedStepNum(selectedOffer.key)}
        <div class="qbody">
          <div class="row-tags">
            {#if selectedOffer.category === "guided"}
              <span class="cat-pill guided">Optional Guided Tour</span>
            {:else}
              <span class="cat-pill">Optional Quest</span>
            {/if}
            {#if offerStepNo !== null}
              <span class="step-pill">Step {offerStepNo}/7</span>
            {/if}
          </div>
          <p class="prose">{selectedOffer.introduction}</p>

          <h3>Requirements &amp; Steps</h3>
          <ul class="steps">
            {#each selectedOffer.checkpoints as cp (cp.id)}
              <li class="step">
                <div class="step-head">
                  <span class="step-icon">○</span>
                  <strong>{cp.title}</strong>
                  {#if cp.roomName}<span class="room-tag">{cp.roomName}</span>{/if}
                </div>
                <p class="step-inst">{cp.instructions}</p>
              </li>
            {/each}
          </ul>

          <div class="reward-box">
            <span class="reward-label">REWARD</span>
            <span>{selectedOffer.rewardSummary}</span>
          </div>
          {#if selectedOffer.allowRestart}
            <p class="footnote">Optional guided quest: you can abandon and restart it at any time without penalty.</p>
          {/if}
        </div>

        <footer class="qactions">
          <button
            type="button"
            class="btn primary"
            onclick={() => runAndReset(selectedOffer.acceptCmd, true)}
          >
            Accept Quest
          </button>
          <button type="button" class="btn" onclick={() => q.closeModals()}>Not Now</button>
          {#if !selectedOffer.allowRestart}
            {#if confirmDeclineKey === selectedOffer.key}
              <button
                type="button"
                class="btn danger"
                onclick={() => runAndReset(selectedOffer.declineCmd, true)}
              >
                Confirm Permanent Decline
              </button>
            {:else}
              <button
                type="button"
                class="btn ghost-danger"
                onclick={() => (confirmDeclineKey = selectedOffer.key)}
              >
                Decline Permanently
              </button>
            {/if}
          {/if}
        </footer>
      {:else if selectedActive}
        {@const activeStepNo = guidedStepNum(selectedActive.key)}
        {@const activeMining = extractActiveSurfaceMiningStatus([selectedActive], selectedActive.key)}
        {@const miningPrep = extractSurfaceMiningPrepActions(activeMining)}
        {@const activeHoldSummary = formatQuestHoldReadinessSummary(selectedActive, liveStowage)}
        {@const activeDest = extractActiveDestinationPort([selectedActive], selectedActive.key)}
        {@const activePreflight = evaluateSailingPreflightReadiness(
          activeDest,
          extractActiveTradeNeed([selectedActive], selectedActive.key),
          liveStowage,
        )}
        <div class="qbody">
          <div class="row-tags">
            {#if selectedActive.category === "guided"}
              <span class="cat-pill guided">Optional Guided Tour</span>
            {:else}
              <span class="cat-pill">Optional Quest</span>
            {/if}
            {#if activeStepNo !== null}
              <span class="step-pill">Step {activeStepNo}/7</span>
            {/if}
            {#if selectedActive.canFinishNow}
              <span class="cat-pill ready">Ready to Complete</span>
            {:else if selectedActive.canCheckNow}
              <span class="cat-pill ready">Objective Ready to Verify</span>
            {/if}
          </div>
          <p class="prose">{selectedActive.introduction}</p>

          <h3>Objectives</h3>
          <ul class="steps">
            {#each selectedActive.checkpoints as cp (cp.id)}
              {@const liveBadge = evaluateCheckpointLiveBadge(cp, liveStowage)}
              <li
                class="step"
                class:done={cp.checked}
                class:ready={!cp.checked && (cp.liveReady || liveBadge?.kind === "ready")}
              >
                <div class="step-head">
                  <span class="step-icon">{cp.checked ? "✓" : cp.liveReady || liveBadge?.kind === "ready" ? "◆" : "○"}</span>
                  <strong>{cp.title}</strong>
                  {#if liveBadge}
                    <span class="cp-live-badge" data-kind={liveBadge.kind}>{liveBadge.label}</span>
                  {/if}
                  {#if cp.roomName}<span class="room-tag">{cp.roomName}</span>{/if}
                </div>
                {#if cp.detailText}
                  <p class="step-done-note">{cp.detailText}</p>
                {:else}
                  <p class="step-inst">{cp.instructions}</p>
                {/if}
              </li>
            {/each}
          </ul>

          {#if activeHoldSummary?.kind === "refit" && activeHoldSummary.refit}
            <div class="prep-strip" class:ready={activeHoldSummary.ready} aria-label="refit hold readiness">
              <span class="prep-tag">{activeHoldSummary.tag}</span>
              <span class:ok={activeHoldSummary.refit.cargoEmpty}>
                {activeHoldSummary.refit.cargoEmpty
                  ? "✓ Cargo empty"
                  : `✗ Cargo ${(activeHoldSummary.refit.cargoKg / 1000).toFixed(1)}t`}
              </span>
              <span class:ok={activeHoldSummary.refit.provisionsEmpty}>
                {activeHoldSummary.refit.provisionsEmpty
                  ? "✓ Stores empty"
                  : `✗ Stores ${activeHoldSummary.refit.provisionsCrewDays}d`}
              </span>
              <span class="muted">Tank {activeHoldSummary.refit.propellantTonnes}t OK</span>
            </div>
          {:else if activeHoldSummary?.kind === "trade" && activeHoldSummary.primary}
            <div class="prep-strip" class:ready={activeHoldSummary.ready} aria-label="quest hold progress">
              <span class="prep-tag">{activeHoldSummary.tag}</span>
              <span class:ok={activeHoldSummary.primary.ready}>
                {activeHoldSummary.primary.ready ? "✓" : "○"} {activeHoldSummary.primary.label}: {activeHoldSummary.primary.currentQty}/{activeHoldSummary.primary.targetQty} {activeHoldSummary.primary.unit}
              </span>
              {#each activeHoldSummary.secondary ?? [] as extra (extra.commoditySlug)}
                <span class="muted" class:ok={extra.ready}>
                  | {extra.label} {extra.currentQty}/{extra.targetQty}{extra.unit}{extra.ready ? " OK" : ""}
                </span>
              {/each}
            </div>
          {/if}
          {#if activePreflight && activeDest}
            <div class="prep-strip" class:ready={activePreflight.ready} aria-label="quest sailing preflight">
              <span class="prep-tag">
                {activePreflight.ready ? "[PRE-FLIGHT OK]" : "[CHECK BEFORE UNDOCK]"}
              </span>
              <span class:ok={activePreflight.propellantOk}>
                {activePreflight.propellantOk
                  ? `✓ Propellant ${activePreflight.propellantTonnes}t`
                  : "✗ Propellant 0t"}
              </span>
              <span class:ok={activePreflight.provisionsOk}>
                {activePreflight.provisionsOk
                  ? `✓ Provisions ${activePreflight.provisionsDays}d`
                  : "✗ Provisions 0d"}
              </span>
              <span class="muted">→ {activeDest.portLabel}</span>
            </div>
          {/if}
          {#if miningPrep.length > 0}
            <div class="prep-strip" aria-label="surface prospecting quick commands">
              <span class="prep-tag">[PROSPECTING]</span>
              {#each miningPrep as act (act.command)}
                <button
                  type="button"
                  class="btn sm"
                  title={`Prefill ${act.command}`}
                  onclick={() => prefillAndClose(act.command)}
                >
                  {act.label} ({act.command})
                </button>
              {/each}
            </div>
          {/if}
          <div class="reward-box">
            <span class="reward-label">REWARD</span>
            <span>{selectedActive.rewardSummary}</span>
          </div>
          {#if selectedActive.allowRestart}
            <p class="footnote">Optional · Restartable: you can abandon and pick this quest up again at any time without penalty.</p>
          {/if}
        </div>

        <footer class="qactions">
          {#if selectedActive.canFinishNow}
            <button
              type="button"
              class="btn complete"
              onclick={() => runAndReset(selectedActive.finishCmd, true)}
            >
              Complete Quest
            </button>
          {:else if selectedActive.canCheckNow}
            <button
              type="button"
              class="btn primary"
              onclick={() => runAndReset(selectedActive.checkCmd, false)}
            >
              Verify Objective
            </button>
          {:else}
            {@const suggested = extractQuestSuggestedAction(selectedActive, liveStowage)}
            {#if suggested}
              <button
                type="button"
                class="btn primary"
                title={`Prefill ${suggested.command}`}
                onclick={() => prefillAndClose(suggested.command)}
              >
                {suggested.label}
              </button>
            {/if}
          {/if}
          <button
            type="button"
            class="btn"
            onclick={() => {
              q.track(selectedActive.key);
              q.closeModals();
            }}
          >
            {q.trackedQuest?.key === selectedActive.key ? "Tracked" : "Track Quest"}
          </button>
          {#if confirmAbandonKey === selectedActive.key}
            <button
              type="button"
              class="btn danger"
              onclick={() => runAndReset(selectedActive.abandonCmd, true)}
            >
              Confirm Abandon
            </button>
          {:else}
            <button
              type="button"
              class="btn ghost-danger"
              onclick={() => (confirmAbandonKey = selectedActive.key)}
            >
              Abandon Quest
            </button>
          {/if}
        </footer>
      {:else}
        <div class="qbody">
          <p class="prose">
            {#if npcMarker}
              {npcMarker.npcName} has no eligible offers open right now. Check <code>+quest</code> or visit later.
            {:else}
              No quest selected.
            {/if}
          </p>
        </div>
        <footer class="qactions">
          <button type="button" class="btn" onclick={() => q.openLog()}>Open Quest Log</button>
          <button type="button" class="btn" onclick={() => q.closeModals()}>Close</button>
        </footer>
      {/if}
    </div>
  </div>
{:else if q.logOpen}
  <div
    class="quest-backdrop"
    role="presentation"
    onclick={() => q.closeModals()}
    onkeydown={handleBackdropKey}
  >
    <div
      class="quest-modal log"
      role="dialog"
      aria-label="Quest log"
      tabindex="-1"
      onclick={(ev) => ev.stopPropagation()}
      onkeydown={handleBackdropKey}
    >
      <header class="qhdr">
        <div class="qhdr-title">
          <span class="qbadge" data-marker="available">!</span>
          <div>
            <h2>Quest Log</h2>
            <p class="sub">
              {q.active.length} active · {q.available.length} available here · {q.completed.length} completed
            </p>
          </div>
        </div>
        <button type="button" class="qclose" onclick={() => q.closeModals()} aria-label="Close quest log">✕</button>
      </header>

      {#if tour}
        <div class="tour-strip" class:all-done={tour.allDone} aria-label="Guided tour progress">
          <div class="tour-meta">
            <span class="tour-label">
              {tour.allDone ? "Corridor Captain — Guided Tour Complete" : "Guided Tour"}
            </span>
            <span class="tour-count">
              {tour.allDone
                ? `${tour.completed}/${tour.total} mastered`
                : `${tour.completed}/${tour.total} complete${tour.currentStep ? ` · Step ${tour.currentStep}` : ""}`}
            </span>
          </div>
          <div class="tour-pips" role="group" aria-label="Guided corridor tour steps">
            {#each roadmap as step (step.key)}
              {@const clickable =
                step.status === "active" || step.status === "available" || step.status === "completed"}
              {@const badge =
                step.status === "completed"
                  ? "✓"
                  : step.status === "active"
                    ? step.readyToTurnIn
                      ? "?"
                      : "▸"
                    : step.status === "available"
                      ? "!"
                      : "○"}
              {@const tip = `Step ${step.step}/7 · ${step.shortLabel} · ${step.giverLabel} (${step.locationLabel})`}
              {#if clickable}
                <button
                  type="button"
                  class="tour-step"
                  class:done={step.status === "completed"}
                  class:current={step.status === "active"}
                  class:ready={step.readyToTurnIn}
                  class:avail={step.status === "available"}
                  title={step.status === "completed"
                    ? `${tip} — click to view completed history`
                    : `${tip} — click to open quest dialog`}
                  onclick={() => {
                    if (step.status === "completed") {
                      q.logTab = "completed";
                    } else {
                      q.openQuestDialog(step.key);
                    }
                  }}
                >
                  <span class="tour-step-badge">{badge}</span>
                  <span class="tour-step-name">{step.step}. {step.shortLabel}</span>
                </button>
              {:else}
                <span
                  class="tour-step"
                  class:done={step.status === "completed"}
                  class:upcoming={step.status === "upcoming"}
                  title={tip}
                >
                  <span class="tour-step-badge">{badge}</span>
                  <span class="tour-step-name">{step.step}. {step.shortLabel}</span>
                </span>
              {/if}
            {/each}
          </div>
        </div>
      {/if}

      <div class="log-tabs" role="tablist">
        <button type="button" class="ltab" class:active={q.logTab === "active"} onclick={() => switchLogTab("active")}>
          Active ({q.active.length})
        </button>
        <button type="button" class="ltab" class:active={q.logTab === "available"} onclick={() => switchLogTab("available")}>
          Available Here ({q.available.length})
        </button>
        <button type="button" class="ltab" class:active={q.logTab === "completed"} onclick={() => switchLogTab("completed")}>
          Completed ({q.completed.length})
        </button>
      </div>

      <div class="qbody">
        {#if q.logTab === "active"}
          {#if q.active.length === 0}
            {#if nextOffer}
              <article class="qcard">
                <div class="qcard-top">
                  <strong>
                    <span class="mini-sym" data-marker="available">!</span>{" "}
                    Next Guided Step ({nextOffer.step}/7): {nextOffer.shortLabel}
                  </strong>
                  <span class="qcard-meta">
                    <span class="step-pill">Step {nextOffer.step}/7</span>
                    <span class="room-tag">{nextOffer.giverRoomName}</span>
                  </span>
                </div>
                <p class="next-line">
                  No active quests. Speak with <strong>{nextOffer.giverLabel}</strong> in <em>{nextOffer.giverRoomName}</em> ({nextOffer.locationLabel}).
                </p>
                {#if nextOfferDest || q.available.length > 0}
                  <div class="qcard-actions">
                    {#if nextOfferDest}
                      <button
                        type="button"
                        class="btn sm primary"
                        onclick={() => prefillAndClose(`+ship/plan ${nextOfferDest.portId}`)}
                      >
                        Sail → {nextOfferDest.portLabel} (+ship/plan {nextOfferDest.portId})
                      </button>
                    {/if}
                    {#if q.available.length > 0}
                      <button
                        type="button"
                        class="btn sm"
                        onclick={() => switchLogTab("available")}
                      >
                        View {q.available.length} Offer{q.available.length === 1 ? "" : "s"} Here
                      </button>
                    {/if}
                  </div>
                {/if}
              </article>
            {:else if tour?.allDone}
              <article class="qcard next-guided-card" aria-label="guided tour mastery">
                <div class="qcard-top">
                  <strong>
                    <span class="mini-sym" data-marker="turn_in">★</span>{" "}
                    Corridor Captain — Guided Tour Complete ({tour.completed}/{tour.total})
                  </strong>
                  <span class="qcard-meta">
                    <span class="cat-pill guided">Mastered</span>
                  </span>
                </div>
                <p class="next-line">
                  All 7 guided onboarding steps complete. Take corridor haulage, survey commissions, or optional NPC contracts next.
                </p>
                <div class="qcard-actions">
                  <button
                    type="button"
                    class="btn sm primary"
                    onclick={() => switchLogTab("completed")}
                  >
                    Completed Log ({q.completed.length})
                  </button>
                  <button
                    type="button"
                    class="btn sm"
                    onclick={() => {
                      onCmd("+market/jobs");
                      q.closeModals();
                    }}
                  >
                    Jobs Board (+market/jobs)
                  </button>
                  {#if q.available.length > 0}
                    <button
                      type="button"
                      class="btn sm"
                      onclick={() => switchLogTab("available")}
                    >
                      View {q.available.length} Offer{q.available.length === 1 ? "" : "s"} Here
                    </button>
                  {/if}
                </div>
              </article>
            {:else}
              <p class="empty">No active quests. Look for NPCs with a golden <strong>!</strong> badge (like Maren Halvorsen in Port Watch Arrivals).</p>
              {#if q.available.length > 0}
                <div class="qcard-actions">
                  <button
                    type="button"
                    class="btn sm primary"
                    onclick={() => switchLogTab("available")}
                  >
                    View {q.available.length} Offer{q.available.length === 1 ? "" : "s"} Here
                  </button>
                </div>
              {/if}
            {/if}
          {:else}
            {#each orderedActiveLog as item (item.key)}
              {@const activeLogStepNo = guidedStepNum(item.key)}
              {@const itemSuggested = extractQuestSuggestedAction(item, liveStowage)}
              {@const itemHoldSummary = formatQuestHoldReadinessSummary(item, liveStowage)}
              {@const itemDest = extractActiveDestinationPort([item], item.key)}
              {@const itemPreflight = evaluateSailingPreflightReadiness(
                itemDest,
                extractActiveTradeNeed([item], item.key),
                liveStowage,
              )}
              <article class="qcard" class:tracked={q.trackedQuest?.key === item.key}>
                <div class="qcard-top">
                  <strong>
                    {#if item.canFinishNow || item.readyToFinish}
                      <span class="mini-sym" data-marker="turn_in">?</span>{" "}
                    {/if}
                    {item.title}
                  </strong>
                  <span class="qcard-meta">
                    {#if activeLogStepNo !== null}
                      <span class="step-pill">Step {activeLogStepNo}/7</span>
                    {/if}
                    <span class="cat-pill" class:guided={item.category === "guided"}>
                      {item.category === "guided" ? "Guided" : "Optional"}{item.allowRestart ? " · Restartable" : ""}
                    </span>
                  </span>
                </div>
                <p class="next-line">Next: {item.nextStep}{#if item.nextRoomName} · <em>{item.nextRoomName}</em>{/if}</p>
                <div class="qcard-steps">
                  {#each item.checkpoints as cp (cp.id)}
                    {@const liveBadge = evaluateCheckpointLiveBadge(cp, liveStowage)}
                    <span
                      class="mini-step"
                      class:done={cp.checked}
                      class:ready={!cp.checked && (cp.liveReady || liveBadge?.kind === "ready")}
                    >
                      {cp.checked ? "✓" : cp.liveReady || liveBadge?.kind === "ready" ? "◆" : "○"} {cp.title}{#if liveBadge}{" "}<span class="cp-live-badge" data-kind={liveBadge.kind}>{liveBadge.label}</span>{/if}
                    </span>
                  {/each}
                </div>
                {#if itemHoldSummary?.kind === "refit" && itemHoldSummary.refit}
                  <div class="prep-strip" class:ready={itemHoldSummary.ready} aria-label="refit hold readiness">
                    <span class="prep-tag">{itemHoldSummary.tag}</span>
                    <span class:ok={itemHoldSummary.refit.cargoEmpty}>
                      {itemHoldSummary.refit.cargoEmpty
                        ? "✓ Cargo empty"
                        : `✗ Cargo ${(itemHoldSummary.refit.cargoKg / 1000).toFixed(1)}t`}
                    </span>
                    <span class:ok={itemHoldSummary.refit.provisionsEmpty}>
                      {itemHoldSummary.refit.provisionsEmpty
                        ? "✓ Stores empty"
                        : `✗ Stores ${itemHoldSummary.refit.provisionsCrewDays}d`}
                    </span>
                    <span class="muted">Tank {itemHoldSummary.refit.propellantTonnes}t OK</span>
                  </div>
                {:else if itemHoldSummary?.kind === "trade" && itemHoldSummary.primary}
                  <div class="prep-strip" class:ready={itemHoldSummary.ready} aria-label="quest hold progress">
                    <span class="prep-tag">{itemHoldSummary.tag}</span>
                    <span class:ok={itemHoldSummary.primary.ready}>
                      {itemHoldSummary.primary.ready ? "✓" : "○"} {itemHoldSummary.primary.label}: {itemHoldSummary.primary.currentQty}/{itemHoldSummary.primary.targetQty} {itemHoldSummary.primary.unit}
                    </span>
                    {#each itemHoldSummary.secondary ?? [] as extra (extra.commoditySlug)}
                      <span class="muted" class:ok={extra.ready}>
                        | {extra.label} {extra.currentQty}/{extra.targetQty}{extra.unit}{extra.ready ? " OK" : ""}
                      </span>
                    {/each}
                  </div>
                {/if}
                {#if itemPreflight && itemDest}
                  <div class="prep-strip" class:ready={itemPreflight.ready} aria-label="quest sailing preflight">
                    <span class="prep-tag">
                      {itemPreflight.ready ? "[PRE-FLIGHT OK]" : "[CHECK BEFORE UNDOCK]"}
                    </span>
                    <span class:ok={itemPreflight.propellantOk}>
                      {itemPreflight.propellantOk
                        ? `✓ Propellant ${itemPreflight.propellantTonnes}t`
                        : "✗ Propellant 0t"}
                    </span>
                    <span class:ok={itemPreflight.provisionsOk}>
                      {itemPreflight.provisionsOk
                        ? `✓ Provisions ${itemPreflight.provisionsDays}d`
                        : "✗ Provisions 0d"}
                    </span>
                    <span class="muted">→ {itemDest.portLabel}</span>
                  </div>
                {/if}
                <div class="qcard-actions">
                  {#if item.canFinishNow}
                    <button type="button" class="btn sm complete" onclick={() => runAndReset(item.finishCmd, false)}>Complete</button>
                  {:else if item.canCheckNow}
                    <button type="button" class="btn sm primary" onclick={() => runAndReset(item.checkCmd, false)}>Verify Step</button>
                  {:else if itemSuggested}
                    <button
                      type="button"
                      class="btn sm primary"
                      title={`Prefill ${itemSuggested.command}`}
                      onclick={() => prefillAndClose(itemSuggested.command)}
                    >
                      {itemSuggested.label}
                    </button>
                  {/if}
                  <button type="button" class="btn sm" onclick={() => q.openQuestDialog(item.key)}>Details</button>
                  <button type="button" class="btn sm" onclick={() => q.track(item.key)}>
                    {q.trackedQuest?.key === item.key ? "Tracked" : "Track"}
                  </button>
                  {#if confirmAbandonKey === item.key}
                    <button type="button" class="btn sm danger" onclick={() => runAndReset(item.abandonCmd, false)}>Confirm Abandon</button>
                  {:else}
                    <button type="button" class="btn sm ghost-danger" onclick={() => (confirmAbandonKey = item.key)}>Abandon</button>
                  {/if}
                </div>
              </article>
            {/each}
          {/if}
        {:else if q.logTab === "available"}
          {#if q.available.length === 0}
            <p class="empty">No quests are available in {q.roomName || "this room"} right now.</p>
          {:else}
            {#each orderedAvailableLog as offer (offer.key)}
              {@const availStepNo = guidedStepNum(offer.key)}
              <article class="qcard">
                <div class="qcard-top">
                  <strong><span class="mini-sym" data-marker="available">!</span> {offer.title}</strong>
                  <span class="qcard-meta">
                    {#if availStepNo !== null}
                      <span class="step-pill">Step {availStepNo}/7</span>
                    {/if}
                    {#if offer.allowRestart}
                      <span class="cat-pill guided">Optional · Restartable</span>
                    {/if}
                    <span class="room-tag">{offer.giverName}</span>
                  </span>
                </div>
                <p class="next-line">{offer.introduction}</p>
                <p class="reward-inline">Reward: {offer.rewardSummary}</p>
                <div class="qcard-actions">
                  <button type="button" class="btn sm primary" onclick={() => runAndReset(offer.acceptCmd, false)}>Accept</button>
                  <button type="button" class="btn sm" onclick={() => q.openQuestDialog(offer.key)}>Read Full Offer</button>
                  {#if !offer.allowRestart && offer.declineCmd}
                    {#if confirmDeclineKey === offer.key}
                      <button
                        type="button"
                        class="btn sm danger"
                        onclick={() => runAndReset(offer.declineCmd, false)}
                      >
                        Confirm Permanent Decline
                      </button>
                    {:else}
                      <button
                        type="button"
                        class="btn sm ghost-danger"
                        onclick={() => (confirmDeclineKey = offer.key)}
                      >
                        Decline Permanently
                      </button>
                    {/if}
                  {/if}
                </div>
              </article>
            {/each}
          {/if}
        {:else}
          {#if tour?.allDone}
            <div class="tour-complete-banner" role="status" aria-label="Guided corridor tour complete">
              <span>★ Guided Corridor Tour Complete (7/7) — Corridor Captain ★</span>
              <button
                type="button"
                class="btn sm primary"
                onclick={() => prefillAndClose("+market/jobs")}
              >
                Jobs Board (+market/jobs)
              </button>
            </div>
          {:else if nextOffer && q.active.length === 0}
            <article class="qcard" aria-label="Next guided step from completed log">
              <div class="qcard-top">
                <strong>
                  <span class="mini-sym" data-marker="available">!</span>{" "}
                  Next Guided Step ({nextOffer.step}/7): {nextOffer.shortLabel}
                </strong>
                <span class="qcard-meta">
                  <span class="step-pill">Step {nextOffer.step}/7</span>
                  <span class="room-tag">{nextOffer.giverRoomName}</span>
                </span>
              </div>
              <p class="next-line">
                Speak with <strong>{nextOffer.giverLabel}</strong> in <em>{nextOffer.giverRoomName}</em> ({nextOffer.locationLabel}).
              </p>
              {#if nextOfferDest || q.available.length > 0}
                <div class="qcard-actions">
                  {#if nextOfferDest}
                    <button
                      type="button"
                      class="btn sm primary"
                      onclick={() => prefillAndClose(`+ship/plan ${nextOfferDest.portId}`)}
                    >
                      Sail → {nextOfferDest.portLabel} (+ship/plan {nextOfferDest.portId})
                    </button>
                  {/if}
                  {#if q.available.length > 0}
                    <button
                      type="button"
                      class="btn sm primary"
                      onclick={() => switchLogTab("available")}
                    >
                      View {q.available.length} Offer{q.available.length === 1 ? "" : "s"} Here
                    </button>
                  {/if}
                </div>
              {/if}
            </article>
          {/if}
          {#if q.completed.length === 0}
            <p class="empty">No completed quests recorded yet.</p>
          {:else}
            {#each q.completed as done (done.key)}
              {@const stepNo = guidedStepNum(done.key)}
              <article class="qcard done">
                <div class="qcard-top">
                  <strong>✓ {done.title}</strong>
                  <span class="qcard-meta">
                    {#if stepNo !== null}
                      <span class="step-pill">Step {stepNo}/7</span>
                    {/if}
                    <span class="room-tag">{done.turnInName}</span>
                  </span>
                </div>
                <p class="next-line">{done.completion}</p>
                {#if done.rewardSummary}
                  <p class="reward-inline">Reward: {done.rewardSummary}</p>
                {/if}
              </article>
            {/each}
          {/if}
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .quest-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(5, 10, 18, 0.74);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 1rem;
    z-index: 60;
  }
  .quest-modal {
    width: min(640px, 100%);
    max-height: 85vh;
    background: var(--bg-panel, #111822);
    border: 1px solid #d4a64a;
    border-radius: 10px;
    box-shadow: 0 18px 48px rgba(0, 0, 0, 0.55);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    color: var(--fg, #e6edf5);
  }
  .qhdr {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 0.75rem;
    padding: 0.85rem 1rem;
    border-bottom: 1px solid var(--border, #263447);
    background: linear-gradient(90deg, rgba(212, 166, 74, 0.14), transparent);
  }
  .qhdr-title {
    display: flex;
    align-items: center;
    gap: 0.65rem;
  }
  .qhdr h2 {
    margin: 0;
    font-size: 1.05rem;
    color: #f7d98a;
  }
  .sub {
    margin: 0.15rem 0 0;
    font-size: 0.78rem;
    color: var(--fg-muted, #94a3b8);
  }
  .qbadge {
    width: 2rem;
    height: 2rem;
    border-radius: 999px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-weight: 800;
    font-size: 1.15rem;
    flex-shrink: 0;
  }
  .qbadge[data-marker="available"] {
    background: #f5b935;
    color: #161103;
  }
  .qbadge[data-marker="turn_in"] {
    background: #fbbf24;
    color: #161103;
    box-shadow: 0 0 12px rgba(251, 191, 36, 0.55);
  }
  .qbadge[data-marker="in_progress"] {
    background: #475569;
    color: #e2e8f0;
  }
  .qclose {
    background: none;
    border: 1px solid var(--border, #2b3b4e);
    border-radius: 6px;
    color: var(--fg-muted, #94a3b8);
    cursor: pointer;
    padding: 0.2rem 0.5rem;
  }
  .qpicker,
  .log-tabs {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
    padding: 0.55rem 1rem;
    border-bottom: 1px solid var(--border, #263447);
    background: rgba(0, 0, 0, 0.16);
  }
  .qpick,
  .ltab {
    background: transparent;
    border: 1px solid var(--border, #2c3e55);
    border-radius: 6px;
    color: var(--fg-muted, #cbd5e1);
    font: inherit;
    font-size: 0.8rem;
    padding: 0.25rem 0.6rem;
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
  }
  .qpick.selected,
  .ltab.active {
    border-color: #f5b935;
    color: #fde68a;
    background: rgba(245, 185, 53, 0.12);
  }
  .mini-sym {
    font-weight: 800;
  }
  .mini-sym[data-marker="available"],
  .mini-sym[data-marker="turn_in"] {
    color: #fbbf24;
  }
  .mini-sym[data-marker="in_progress"] {
    color: #94a3b8;
  }
  .qbody {
    padding: 0.9rem 1rem;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 0.7rem;
  }
  .row-tags {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
  }
  .cat-pill {
    align-self: flex-start;
    font-size: 0.72rem;
    font-weight: 600;
    padding: 0.1rem 0.45rem;
    border-radius: 999px;
    border: 1px solid var(--border, #334155);
    color: var(--fg-muted, #94a3b8);
  }
  .cat-pill.guided {
    border-color: #d4a64a;
    color: #fde68a;
    background: rgba(212, 166, 74, 0.12);
  }
  .cat-pill.ready {
    border-color: #34d399;
    color: #6ee7b7;
    background: rgba(52, 211, 153, 0.14);
  }
  .prose {
    margin: 0;
    line-height: 1.45;
    font-size: 0.9rem;
  }
  h3 {
    margin: 0.2rem 0 0;
    font-size: 0.82rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: #f7d98a;
  }
  .steps {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .step {
    padding: 0.55rem 0.7rem;
    border-radius: 7px;
    border: 1px solid var(--border, #263447);
    background: rgba(0, 0, 0, 0.2);
  }
  .step.done {
    border-color: rgba(52, 211, 153, 0.45);
    opacity: 0.85;
  }
  .step.ready {
    border-color: #fbbf24;
    background: rgba(251, 191, 36, 0.08);
  }
  .step-head {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    flex-wrap: wrap;
    font-size: 0.86rem;
  }
  .step-icon {
    font-weight: 800;
    color: #fbbf24;
  }
  .step.done .step-icon {
    color: #34d399;
  }
  .room-tag {
    margin-left: auto;
    font-size: 0.74rem;
    color: #7dd3fc;
  }
  .step-inst,
  .step-done-note {
    margin: 0.3rem 0 0;
    font-size: 0.8rem;
    color: var(--fg-muted, #94a3b8);
    line-height: 1.38;
  }
  .step-done-note {
    color: #6ee7b7;
  }
  .reward-box {
    display: flex;
    align-items: baseline;
    gap: 0.55rem;
    padding: 0.55rem 0.75rem;
    border-radius: 7px;
    border: 1px solid rgba(212, 166, 74, 0.45);
    background: rgba(212, 166, 74, 0.08);
    font-size: 0.84rem;
  }
  .reward-label {
    font-weight: 800;
    font-size: 0.72rem;
    letter-spacing: 0.06em;
    color: #fbbf24;
  }
  .footnote {
    margin: 0;
    font-size: 0.76rem;
    color: var(--fg-muted, #94a3b8);
  }
  .qactions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    padding: 0.75rem 1rem;
    border-top: 1px solid var(--border, #263447);
    background: rgba(0, 0, 0, 0.22);
  }
  .btn {
    border: 1px solid var(--border, #334155);
    border-radius: 6px;
    background: var(--bg-input, #1e293b);
    color: var(--fg, #e2e8f0);
    font: inherit;
    font-size: 0.83rem;
    font-weight: 600;
    padding: 0.4rem 0.85rem;
    cursor: pointer;
  }
  .btn.sm {
    font-size: 0.76rem;
    padding: 0.22rem 0.55rem;
  }
  .btn.primary {
    background: #d49422;
    border-color: #f5b935;
    color: #140f02;
  }
  .btn.complete {
    background: #10b981;
    border-color: #34d399;
    color: #031911;
  }
  .btn.danger {
    background: #b91c1c;
    border-color: #ef4444;
    color: #fff;
  }
  .btn.ghost-danger {
    border-color: rgba(239, 68, 68, 0.45);
    color: #fca5a5;
    margin-left: auto;
  }
  .qcard {
    border: 1px solid var(--border, #263447);
    border-radius: 8px;
    padding: 0.65rem 0.75rem;
    background: rgba(0, 0, 0, 0.18);
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }
  .qcard.tracked {
    border-color: #d4a64a;
  }
  .qcard.done {
    border-color: rgba(52, 211, 153, 0.35);
  }
  .qcard-top {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
  }
  .next-line {
    margin: 0;
    font-size: 0.82rem;
    color: var(--fg-muted, #cbd5e1);
  }
  .reward-inline {
    margin: 0;
    font-size: 0.76rem;
    color: #f7d98a;
  }
  .qcard-steps {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    font-size: 0.78rem;
  }
  .mini-step.done {
    color: #6ee7b7;
  }
  .mini-step.ready {
    color: #fde68a;
  }
  .qcard-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
    margin-top: 0.2rem;
  }
  .empty {
    margin: 0.5rem 0;
    color: var(--fg-muted, #94a3b8);
    font-size: 0.86rem;
  }
  .tour-strip {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    padding: 0.55rem 1rem;
    border-bottom: 1px solid var(--border, #263447);
    background: rgba(212, 166, 74, 0.08);
  }
  .tour-strip.all-done {
    background: linear-gradient(90deg, rgba(16, 185, 129, 0.16), rgba(212, 166, 74, 0.14));
  }
  .tour-strip.all-done .tour-label {
    color: #6ee7b7;
  }
  .tour-meta {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-size: 0.76rem;
  }
  .tour-label {
    font-weight: 700;
    color: #f7d98a;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .tour-count {
    color: var(--fg-muted, #cbd5e1);
    font-family: var(--mono, monospace);
  }
  .tour-pips {
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem;
  }
  .tour-step {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.18rem 0.45rem;
    border-radius: 5px;
    border: 1px solid rgba(148, 163, 184, 0.25);
    background: rgba(15, 23, 42, 0.55);
    color: var(--fg-muted, #94a3b8);
    font: inherit;
    font-size: 0.72rem;
    line-height: 1.2;
  }
  button.tour-step {
    cursor: pointer;
  }
  button.tour-step:hover {
    filter: brightness(1.15);
  }
  .tour-step.done {
    border-color: rgba(52, 211, 153, 0.55);
    background: rgba(16, 185, 129, 0.16);
    color: #6ee7b7;
  }
  .tour-step.current {
    border-color: rgba(251, 191, 36, 0.75);
    background: rgba(212, 166, 74, 0.22);
    color: #fde68a;
    font-weight: 700;
  }
  .tour-step.current.ready {
    border-color: #34d399;
    background: rgba(16, 185, 129, 0.24);
    color: #a7f3d0;
  }
  .tour-step.avail {
    border-color: rgba(250, 204, 21, 0.7);
    background: rgba(250, 204, 21, 0.14);
    color: #fef08a;
    font-weight: 700;
  }
  .tour-step.upcoming {
    opacity: 0.68;
  }
  .tour-step-badge {
    font-family: var(--mono, monospace);
    font-weight: 800;
  }
  .prep-strip {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.45rem;
    padding: 0.45rem 0.65rem;
    border-radius: 6px;
    border: 1px solid rgba(212, 166, 74, 0.55);
    background: rgba(62, 46, 12, 0.42);
    font-size: 0.76rem;
    color: #fde68a;
  }
  .prep-strip.ready {
    border-color: rgba(52, 211, 153, 0.65);
    background: rgba(16, 74, 52, 0.42);
    color: #a7f3d0;
  }
  .prep-tag {
    font-family: var(--mono, monospace);
    font-weight: 700;
    color: #fbbf24;
  }
  .prep-strip.ready .prep-tag,
  .prep-strip .ok {
    color: #6ee7b7;
  }
  .qcard-meta {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    flex-wrap: wrap;
  }
  .step-pill {
    font-size: 0.7rem;
    font-weight: 700;
    font-family: var(--mono, monospace);
    padding: 0.1rem 0.42rem;
    border-radius: 999px;
    border: 1px solid rgba(52, 211, 153, 0.55);
    background: rgba(16, 185, 129, 0.18);
    color: #6ee7b7;
  }
  .tour-complete-banner {
    padding: 0.55rem 0.75rem;
    border-radius: 7px;
    border: 1px solid rgba(52, 211, 153, 0.7);
    background: rgba(16, 74, 52, 0.45);
    color: #a7f3d0;
    font-weight: 700;
    font-size: 0.82rem;
    text-align: center;
  }
  .cp-live-badge {
    font-family: var(--mono, monospace);
    font-size: 0.68rem;
    font-weight: 700;
    padding: 0.05rem 0.36rem;
    border-radius: 4px;
    border: 1px solid rgba(251, 191, 36, 0.5);
    background: rgba(78, 56, 12, 0.45);
    color: #fde68a;
  }
  .cp-live-badge[data-kind="ready"] {
    border-color: rgba(52, 211, 153, 0.65);
    background: rgba(16, 74, 52, 0.5);
    color: #6ee7b7;
  }
  .cp-live-badge[data-kind="blocked"] {
    border-color: rgba(248, 113, 113, 0.55);
    background: rgba(92, 24, 24, 0.45);
    color: #fca5a5;
  }
</style>
