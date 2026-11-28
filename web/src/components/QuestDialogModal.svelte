<script lang="ts">
  // WoW-style NPC Quest Dialog & Full Quest Log Modal (Active / Available Here / Completed).
  // Every button dispatches a server-signed `+quest/...` command via `onCmd`.
  import type { GameApp } from "$lib/game.svelte";
  import {
    GUIDED_TOUR_KEYS,
    evaluateQuestStowageReadiness,
    evaluateRefitHoldReadiness,
    extractActiveSurfaceMiningStatus,
    extractActiveTradeNeed,
    extractQuestSuggestedAction,
    extractSurfaceMiningPrepActions,
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
      ? q.active.filter((item) => item.turnInId === q.dialogNpcId || item.giverId === q.dialogNpcId)
      : [],
  );

  const npcAvailableQuests = $derived(
    q.dialogNpcId ? q.available.filter((item) => item.giverId === q.dialogNpcId) : [],
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
          {#if npcMarker}
            <span class="qbadge" data-marker={npcMarker.marker}>{npcMarker.symbol}</span>
          {:else if selectedOffer}
            <span class="qbadge" data-marker="available">!</span>
          {:else if selectedActive?.canFinishNow || selectedActive?.readyToFinish}
            <span class="qbadge" data-marker="turn_in">?</span>
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
              onclick={() => (q.dialogQuestKey = item.key)}
            >
              <span class="mini-sym" data-marker={item.readyToFinish || item.canCheckNow ? "turn_in" : "in_progress"}>?</span>
              {item.title}
            </button>
          {/each}
          {#each npcAvailableQuests as item (item.key)}
            <button
              type="button"
              class="qpick"
              class:selected={selectedOffer?.key === item.key}
              onclick={() => (q.dialogQuestKey = item.key)}
            >
              <span class="mini-sym" data-marker="available">!</span>
              {item.title}
            </button>
          {/each}
        </div>
      {/if}

      {#if selectedOffer}
        <div class="qbody">
          {#if selectedOffer.category === "guided"}
            <span class="cat-pill guided">Optional Guided Tour</span>
          {:else}
            <span class="cat-pill">Optional Quest</span>
          {/if}
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
        {@const activeMining = extractActiveSurfaceMiningStatus([selectedActive], selectedActive.key)}
        {@const miningPrep = extractSurfaceMiningPrepActions(activeMining)}
        {@const activeSuggested = extractQuestSuggestedAction(selectedActive)}
        {@const activeRefitCheck =
          activeSuggested?.command.startsWith("+coop/refit")
            ? evaluateRefitHoldReadiness(liveStowage)
            : null}
        {@const activeHoldProgress =
          !activeRefitCheck
            ? evaluateQuestStowageReadiness(
                extractActiveTradeNeed([selectedActive], selectedActive.key),
                liveStowage,
              )
            : null}
        <div class="qbody">
          <div class="row-tags">
            {#if selectedActive.category === "guided"}
              <span class="cat-pill guided">Optional Guided Tour</span>
            {:else}
              <span class="cat-pill">Optional Quest</span>
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
              <li class="step" class:done={cp.checked} class:ready={!cp.checked && cp.liveReady}>
                <div class="step-head">
                  <span class="step-icon">{cp.checked ? "✓" : cp.liveReady ? "◆" : "○"}</span>
                  <strong>{cp.title}</strong>
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

          {#if activeRefitCheck}
            <div class="prep-strip" class:ready={activeRefitCheck.ready} aria-label="refit hold readiness">
              <span class="prep-tag">{activeRefitCheck.ready ? "[READY]" : "[REFIT CHECK]"}</span>
              <span class:ok={activeRefitCheck.cargoEmpty}>
                {activeRefitCheck.cargoEmpty ? "✓ Cargo empty" : `✗ Cargo ${(activeRefitCheck.cargoKg / 1000).toFixed(1)}t`}
              </span>
              <span class:ok={activeRefitCheck.provisionsEmpty}>
                {activeRefitCheck.provisionsEmpty ? "✓ Stores empty" : `✗ Stores ${activeRefitCheck.provisionsCrewDays}d`}
              </span>
              <span class="muted">Tank {activeRefitCheck.propellantTonnes}t OK</span>
            </div>
          {:else if activeHoldProgress}
            <div class="prep-strip" class:ready={activeHoldProgress.ready} aria-label="quest hold progress">
              <span class="prep-tag">{activeHoldProgress.ready ? "[HOLD READY]" : "[HOLD]"}</span>
              <span class:ok={activeHoldProgress.ready}>
                {activeHoldProgress.ready ? "✓" : "○"} {activeHoldProgress.label}: {activeHoldProgress.currentQty}/{activeHoldProgress.targetQty} {activeHoldProgress.unit}
              </span>
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
            {@const suggested = extractQuestSuggestedAction(selectedActive)}
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
        <button type="button" class="ltab" class:active={q.logTab === "active"} onclick={() => (q.logTab = "active")}>
          Active ({q.active.length})
        </button>
        <button type="button" class="ltab" class:active={q.logTab === "available"} onclick={() => (q.logTab = "available")}>
          Available Here ({q.available.length})
        </button>
        <button type="button" class="ltab" class:active={q.logTab === "completed"} onclick={() => (q.logTab = "completed")}>
          Completed ({q.completed.length})
        </button>
      </div>

      <div class="qbody">
        {#if q.logTab === "active"}
          {#if q.active.length === 0}
            {#if nextOffer}
              <p class="empty">
                No active quests. <strong>Next Guided Step ({nextOffer.step}/7): {nextOffer.shortLabel}</strong> — speak with <strong>{nextOffer.giverLabel}</strong> in <em>{nextOffer.giverRoomName}</em> ({nextOffer.locationLabel}).
              </p>
            {:else}
              <p class="empty">No active quests. Look for NPCs with a golden <strong>!</strong> badge (like Maren Halvorsen in Port Watch Arrivals).</p>
            {/if}
          {:else}
            {#each q.active as item (item.key)}
              {@const itemSuggested = extractQuestSuggestedAction(item)}
              {@const itemRefitCheck =
                itemSuggested?.command.startsWith("+coop/refit")
                  ? evaluateRefitHoldReadiness(liveStowage)
                  : null}
              {@const itemHoldProgress =
                !itemRefitCheck
                  ? evaluateQuestStowageReadiness(
                      extractActiveTradeNeed([item], item.key),
                      liveStowage,
                    )
                  : null}
              <article class="qcard" class:tracked={q.trackedQuest?.key === item.key}>
                <div class="qcard-top">
                  <strong>{item.title}</strong>
                  <span class="cat-pill" class:guided={item.category === "guided"}>
                    {item.category === "guided" ? "Guided" : "Optional"}{item.allowRestart ? " · Restartable" : ""}
                  </span>
                </div>
                <p class="next-line">Next: {item.nextStep}{#if item.nextRoomName} · <em>{item.nextRoomName}</em>{/if}</p>
                <div class="qcard-steps">
                  {#each item.checkpoints as cp (cp.id)}
                    <span class="mini-step" class:done={cp.checked} class:ready={!cp.checked && cp.liveReady}>
                      {cp.checked ? "✓" : cp.liveReady ? "◆" : "○"} {cp.title}
                    </span>
                  {/each}
                </div>
                {#if itemRefitCheck}
                  <div class="prep-strip" class:ready={itemRefitCheck.ready} aria-label="refit hold readiness">
                    <span class="prep-tag">{itemRefitCheck.ready ? "[READY]" : "[REFIT CHECK]"}</span>
                    <span class:ok={itemRefitCheck.cargoEmpty}>
                      {itemRefitCheck.cargoEmpty ? "✓ Cargo empty" : `✗ Cargo ${(itemRefitCheck.cargoKg / 1000).toFixed(1)}t`}
                    </span>
                    <span class:ok={itemRefitCheck.provisionsEmpty}>
                      {itemRefitCheck.provisionsEmpty ? "✓ Stores empty" : `✗ Stores ${itemRefitCheck.provisionsCrewDays}d`}
                    </span>
                    <span class="muted">Tank {itemRefitCheck.propellantTonnes}t OK</span>
                  </div>
                {:else if itemHoldProgress}
                  <div class="prep-strip" class:ready={itemHoldProgress.ready} aria-label="quest hold progress">
                    <span class="prep-tag">{itemHoldProgress.ready ? "[HOLD READY]" : "[HOLD]"}</span>
                    <span class:ok={itemHoldProgress.ready}>
                      {itemHoldProgress.ready ? "✓" : "○"} {itemHoldProgress.label}: {itemHoldProgress.currentQty}/{itemHoldProgress.targetQty} {itemHoldProgress.unit}
                    </span>
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
            {#each q.available as offer (offer.key)}
              <article class="qcard">
                <div class="qcard-top">
                  <strong><span class="mini-sym" data-marker="available">!</span> {offer.title}</strong>
                  <span class="qcard-meta">
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
                </div>
              </article>
            {/each}
          {/if}
        {:else}
          {#if tour?.allDone}
            <div class="tour-complete-banner" role="status" aria-label="Guided corridor tour complete">
              ★ Guided Corridor Tour Complete (7/7) — Corridor Captain ★
            </div>
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
</style>
