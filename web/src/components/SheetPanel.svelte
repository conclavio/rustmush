<script lang="ts">
  // The schema-driven character sheet: walks `app.sheet.schema.sections`
  // and renders each one purely from its `layout`/`FieldType` — no field id
  // or template name is ever special-cased here, so Vampire/Mage/Werewolf
  // (and any future ruleset) need zero changes on this side. See
  // $lib/sheet.ts for the rendering rules (pip-vs-bar threshold, track mark
  // glyphs, map/list dispatch) and web/README.md for the payload contract.
  import type { GameApp } from "$lib/game.svelte";
  import { GUIDED_TOUR_KEYS } from "$lib/stores/quests.svelte";
  import {
    approvalKind,
    asNumber,
    asStringList,
    asText,
    barPct,
    capitalize,
    deepestLevel,
    isPipRange,
    layoutKind,
    listItems,
    mapEntries,
    pipRow,
    trackCells,
    trackMarkSymbol,
    visibleFields,
    type FieldType,
  } from "$lib/sheet";

  let { app }: { app: GameApp } = $props();

  const sheet = $derived(app.sheet);
  const trackedQuest = $derived(
    app.quests?.active.find((q) => q.key === app.quests?.trackedKey) ?? app.quests?.active[0] ?? null,
  );
  const completedQuestsCount = $derived(app.quests?.completed.length ?? 0);
  const availableQuestsCount = $derived(app.quests?.available.length ?? 0);
  const trackedDoneCount = $derived(
    trackedQuest ? trackedQuest.checkpoints.filter((c) => c.checked || c.liveReady).length : 0,
  );
  const guidedTour = $derived(app.quests?.guidedTourProgress ?? null);
  const guidedRoadmap = $derived(guidedTour ? (app.quests?.guidedTourRoadmap ?? []) : []);
  const nextGuidedOffer = $derived(app.quests?.nextGuidedOffer ?? null);
  const trackedStepNo = $derived.by(() => {
    if (!trackedQuest) return 0;
    const idx = (GUIDED_TOUR_KEYS as readonly string[]).indexOf(trackedQuest.key);
    return idx >= 0 ? idx + 1 : 0;
  });
  const otherReadyHere = $derived(
    (app.quests?.active ?? []).filter((item) => item.key !== trackedQuest?.key && item.canFinishNow),
  );
  const topOffer = $derived.by(() => {
    const avail = app.quests?.available ?? [];
    if (avail.length === 0) return null;
    return (
      avail.find(
        (item) =>
          item.category.toLowerCase() === "guided" ||
          (GUIDED_TOUR_KEYS as readonly string[]).includes(item.key),
      ) ?? avail[0]
    );
  });

  // No poll: since interface cut I1 every command-side sheet write
  // republishes the keyframe to the subject (see web/README.md), so the
  // panel is live by push; the Refresh button remains a manual nudge.
</script>

{#snippet pips(value: number, max: number)}
  <span class="pips">
    {#each pipRow(value, max) as filled, i (i)}
      <span class="pip" class:filled aria-hidden="true">{filled ? "●" : "○"}</span>
    {/each}
    <span class="pipnum">{Math.max(0, Math.min(max, Math.trunc(value)))}</span>
  </span>
{/snippet}

{#snippet numBar(value: number, min: number, max: number)}
  <span class="numbar">
    <span class="numval">{value}<span class="nummax">/{max}</span></span>
    <span class="bar"><span class="barfill" style={`width:${barPct(value, min, max)}%`}></span></span>
  </span>
{/snippet}

{#snippet typedValue(ty: FieldType, value: unknown)}
  {#if ty.kind === "dots"}
    {@render pips(asNumber(value), ty.max)}
  {:else if ty.kind === "number"}
    {#if isPipRange(ty.min, ty.max)}
      {@render pips(asNumber(value), ty.max)}
    {:else}
      {@render numBar(asNumber(value), ty.min, ty.max)}
    {/if}
  {:else if ty.kind === "text"}
    <span class="text">{asText(value) || "—"}</span>
  {:else if ty.kind === "enum"}
    <span class="text">{capitalize(asText(value))}</span>
  {:else if ty.kind === "list"}
    {#if asStringList(value).length > 0}
      <span class="taglist">
        {#each asStringList(value) as item, i (i)}
          <span class="chip">{item}</span>
        {/each}
      </span>
    {:else}
      <span class="text muted">—</span>
    {/if}
  {:else if ty.kind === "track"}
    {@render trackRow(trackCells(value), ty.levels)}
  {:else}
    <span class="text muted">—</span>
  {/if}
{/snippet}

{#snippet trackRow(marks: (string | null)[], levels: { id: string; label: string; penalty: number }[])}
  {@const cur = deepestLevel(marks, levels)}
  <span class="track">
    <span class="boxes">
      {#each marks as mark, i (i)}
        <span class="box" class:filled={mark !== null}>{trackMarkSymbol(mark)}</span>
      {/each}
    </span>
    {#if cur}
      <span class="tracklevel muted">{cur.label}{#if cur.penalty < 0}&nbsp;({cur.penalty}){/if}</span>
    {/if}
  </span>
{/snippet}

<div class="sheet">
  {#if sheet.unavailable}
    <div class="empty">
      <p><strong>Character sheets</strong></p>
      <p class="muted">
        The sheet isn't available right now.
        <button type="button" class="linklike" onclick={() => app.refreshSheet()}>Try again</button>
      </p>
    </div>
  {:else if !sheet.loaded || sheet.schema === null}
    <p class="muted loading">Loading…</p>
  {:else}
    {@const schema = sheet.schema}
    <div class="head">
      {#if sheet.approval}
        <span class="approval" data-kind={approvalKind(sheet.approval)}>{sheet.approval.state}</span>
      {/if}
      <span class="grow"></span>
      <button type="button" class="refresh" title="Refresh sheet" onclick={() => app.refreshSheet()}>
        ↻
      </button>
    </div>

    <div class="sections">
      {#if trackedQuest || completedQuestsCount > 0 || availableQuestsCount > 0}
        <section class="block quest-summary" class:mastered={guidedTour?.allDone} aria-label="guided quest progress">
          <div class="qsum-head">
            <h3>
              Guided Quests
              {#if guidedTour}
                <span class="qsum-tour-badge" class:mastered={guidedTour.allDone}>
                  {guidedTour.allDone
                    ? `Corridor Captain (${guidedTour.completed}/${guidedTour.total})`
                    : `Tour ${guidedTour.completed}/${guidedTour.total}`}
                </span>
              {/if}
            </h3>
            <button type="button" class="qlog-btn" onclick={() => app.quests?.openLog()}>
              Open Quest Log
            </button>
          </div>
          {#if guidedRoadmap.length > 0}
            <div class="qsum-roadmap" role="group" aria-label="guided corridor tour steps">
              {#each guidedRoadmap as step (step.key)}
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
                {@const tip = `Step ${step.step}/7 · ${step.shortLabel} · ${step.giverLabel} (${step.giverRoomName})`}
                {#if step.status === "active" || step.status === "available"}
                  <button
                    type="button"
                    class="qsum-step"
                    class:current={step.status === "active"}
                    class:avail={step.status === "available"}
                    title={`${tip} — click to open dialog`}
                    onclick={() => app.quests?.openQuestDialog(step.key)}
                  >
                    {badge} {step.step}. {step.shortLabel}
                  </button>
                {:else if step.status === "completed"}
                  <button
                    type="button"
                    class="qsum-step done"
                    title={`${tip} — click to view in Completed log`}
                    onclick={() => app.quests?.openLog("completed")}
                  >
                    {badge} {step.step}. {step.shortLabel}
                  </button>
                {:else}
                  <span
                    class="qsum-step upcoming"
                    title={tip}
                  >
                    {badge} {step.step}. {step.shortLabel}
                  </span>
                {/if}
              {/each}
            </div>
          {/if}
          {#if trackedQuest}
            <div class="qsum-line">
              <strong class="qsum-title">{trackedQuest.title}</strong>
              {#if trackedStepNo > 0}
                <span class="qsum-step-badge">Step {trackedStepNo}/7</span>
              {/if}
              {#if trackedQuest.checkpoints.length > 0}
                <span class="qsum-prog">({trackedDoneCount}/{trackedQuest.checkpoints.length})</span>
              {/if}
            </div>
            {#if trackedQuest.nextStep}
              <p class="qsum-next">{trackedQuest.nextStep}</p>
            {/if}
            {#if trackedQuest.canFinishNow || otherReadyHere.length > 0 || availableQuestsCount > 0}
              <div class="qsum-actions">
                {#if trackedQuest.canFinishNow}
                  <button
                    type="button"
                    class="qsum-action-btn ready"
                    onclick={() => app.quests?.openQuestDialog(trackedQuest.key)}
                  >
                    [?] Ready to turn in here
                  </button>
                {/if}
                {#if otherReadyHere.length === 1}
                  <button
                    type="button"
                    class="qsum-action-btn ready"
                    onclick={() => app.quests?.openQuestDialog(otherReadyHere[0]!.key)}
                  >
                    [?] Ready to turn in: {otherReadyHere[0]!.title}
                  </button>
                {:else if otherReadyHere.length > 1}
                  <button
                    type="button"
                    class="qsum-action-btn ready"
                    onclick={() => app.quests?.openLog("active")}
                  >
                    [?] {otherReadyHere.length} ready to turn in here
                  </button>
                {/if}
                {#if availableQuestsCount > 0}
                  <button
                    type="button"
                    class="qsum-action-btn offer"
                    onclick={() =>
                      availableQuestsCount === 1 && topOffer
                        ? app.quests?.openQuestDialog(topOffer.key)
                        : app.quests?.openLog("available")}
                  >
                    [!] {availableQuestsCount} quest offer{availableQuestsCount === 1 ? "" : "s"} here
                  </button>
                {/if}
              </div>
            {/if}
          {:else if topOffer}
            <div class="qsum-actions">
              <button
                type="button"
                class="qsum-action-btn offer"
                onclick={() => app.quests?.openQuestDialog(topOffer.key)}
              >
                [!] {topOffer.giverName ? `${topOffer.giverName} offers: ` : ""}{topOffer.title}{availableQuestsCount > 1 ? ` (+${availableQuestsCount - 1} more)` : ""}
              </button>
              {#if availableQuestsCount > 1}
                <button
                  type="button"
                  class="qlog-btn"
                  onclick={() => app.quests?.openLog("available")}
                >
                  All Offers ({availableQuestsCount})
                </button>
              {/if}
            </div>
          {:else if nextGuidedOffer}
            <p class="qsum-next">
              <strong>Next Guided ({nextGuidedOffer.step}/7):</strong> {nextGuidedOffer.shortLabel} — speak with <strong>{nextGuidedOffer.giverLabel}</strong> ({nextGuidedOffer.giverRoomName})
            </p>
          {/if}
          {#if completedQuestsCount > 0}
            <p class="qsum-done">Completed: {completedQuestsCount}</p>
          {/if}
        </section>
      {/if}
      {#each schema.sections as section (section.id)}
        {@const kind = layoutKind(section.layout)}
        {@const fields = visibleFields(sheet.data, section)}
        {#if kind === "tracks"}
          {#each fields as field (field.id)}
            {#if field.ty.kind === "track"}
              <section class="block">
                <h3>{field.label}</h3>
                {@render trackRow(trackCells(sheet.data[field.id]), field.ty.levels)}
              </section>
            {/if}
          {/each}
        {:else if kind === "map"}
          {#if fields.length > 0}
            <section class="block">
              <h3>{section.label}</h3>
              {#each fields as field (field.id)}
                {#if field.ty.kind === "map"}
                  {@const entries = mapEntries(sheet.data, field)}
                  <div class="mapfield">
                    {#if fields.length > 1}<h4>{field.label}</h4>{/if}
                    {#if entries.length === 0}
                      <p class="muted small">None.</p>
                    {:else}
                      {#each entries as entry (entry.key)}
                        <div class="row maprow">
                          <span class="flabel">{entry.key}</span>
                          {@render typedValue(field.ty.value, entry.value)}
                        </div>
                      {/each}
                    {/if}
                  </div>
                {/if}
              {/each}
            </section>
          {/if}
        {:else if fields.length > 0}
          <section class="block">
            <h3>{section.label}</h3>
            <div class="fields" class:gridlayout={kind === "grid"}>
              {#each fields as field (field.id)}
                <div class="row">
                  <span class="flabel">
                    {field.label}
                    {#if field.staff_only}<span class="staffbadge" title="Staff-only field">★</span>{/if}
                  </span>
                  {@render typedValue(field.ty, sheet.data[field.id])}
                </div>
                {#if field.ty.kind === "list" && listItems(sheet.data, field).length > 0 && field.ty.of.kind !== "text"}
                  <!-- non-text list elements (none in the current schema, but
                       kept generic): fall back to the raw JSON rather than
                       silently dropping data. -->
                  <div class="row sub muted small">{JSON.stringify(listItems(sheet.data, field))}</div>
                {/if}
              {/each}
            </div>
          </section>
        {/if}
      {/each}
    </div>
  {/if}
</div>

<style>
  .sheet {
    height: 100%;
    overflow-y: auto;
    padding: 0.6rem 0.75rem 1rem;
    font-size: 0.85rem;
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
  }

  .empty {
    padding: 0.5rem 0.1rem;
  }

  .loading {
    padding: 0.5rem 0.1rem;
  }

  .muted {
    color: var(--fg-muted);
  }

  .small {
    font-size: 0.78rem;
  }

  .linklike {
    background: none;
    border: none;
    color: var(--accent);
    cursor: pointer;
    font: inherit;
    padding: 0;
    text-decoration: underline;
  }

  .head {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .grow {
    flex: 1;
  }

  .approval {
    display: inline-flex;
    align-items: center;
    border-radius: 999px;
    padding: 0.1rem 0.6rem;
    font-size: 0.72rem;
    font-weight: 600;
    text-transform: capitalize;
    border: 1px solid var(--border);
    color: var(--fg-muted);
  }

  .approval[data-kind="approved"] {
    color: var(--success);
    border-color: var(--success);
  }

  .approval[data-kind="pending"] {
    color: var(--warn);
    border-color: var(--warn);
  }

  .refresh {
    background: none;
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--fg-muted);
    cursor: pointer;
    font-size: 0.9rem;
    line-height: 1;
    padding: 0.15rem 0.45rem;
  }

  .refresh:hover {
    color: var(--fg);
  }

  .sections {
    display: flex;
    flex-direction: column;
    gap: 0.7rem;
  }

  .block h3 {
    margin: 0 0 0.35rem;
    font-size: 0.78rem;
    letter-spacing: 0.03em;
    text-transform: uppercase;
    color: var(--fg-muted);
  }

  .mapfield h4 {
    margin: 0.3rem 0 0.2rem;
    font-size: 0.78rem;
    color: var(--fg-muted);
    font-weight: 600;
  }

  .fields {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }

  .fields.gridlayout {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    column-gap: 0.75rem;
    row-gap: 0.3rem;
  }

  .row {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 0.5rem;
  }

  .row.sub {
    justify-content: flex-start;
  }

  .maprow {
    padding: 0.1rem 0;
    border-bottom: 1px solid var(--border);
  }

  .flabel {
    color: var(--fg);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .staffbadge {
    color: var(--warn);
    margin-left: 0.25rem;
    font-size: 0.7rem;
  }

  .text {
    color: var(--fg);
    text-align: right;
    overflow-wrap: anywhere;
  }

  .pips {
    display: inline-flex;
    align-items: center;
    gap: 0.05rem;
    white-space: nowrap;
  }

  .pip {
    color: var(--fg-faint);
    font-size: 0.85rem;
    line-height: 1;
  }

  .pip.filled {
    color: var(--accent);
  }

  .pipnum {
    color: var(--fg-muted);
    font-size: 0.7rem;
    margin-left: 0.3rem;
  }

  .numbar {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
  }

  .numval {
    color: var(--fg);
    font-variant-numeric: tabular-nums;
  }

  .nummax {
    color: var(--fg-faint);
  }

  .bar {
    display: inline-block;
    width: 4.5rem;
    height: 0.4rem;
    border-radius: 999px;
    background: var(--bg-input);
    overflow: hidden;
  }

  .barfill {
    display: block;
    height: 100%;
    background: var(--accent);
  }

  .taglist {
    display: flex;
    flex-wrap: wrap;
    gap: 0.25rem;
    justify-content: flex-end;
  }

  .chip {
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 0 0.45rem;
    font-size: 0.75rem;
    color: var(--fg-muted);
  }

  .track {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex-wrap: wrap;
  }

  .boxes {
    display: inline-flex;
    gap: 0.15rem;
  }

  .box {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.1rem;
    height: 1.1rem;
    border: 1px solid var(--border);
    border-radius: 3px;
    font-size: 0.68rem;
    font-weight: 700;
    color: var(--fg-muted);
  }

  .box.filled {
    border-color: var(--error);
    color: var(--error);
  }

  .tracklevel {
    font-size: 0.75rem;
  }

  .quest-summary {
    border-color: rgba(212, 166, 74, 0.65);
    background: rgba(212, 166, 74, 0.08);
  }

  .qsum-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    margin-bottom: 0.25rem;
  }

  .qsum-head h3 {
    margin: 0;
    color: #ffd84d;
  }

  .qlog-btn {
    border: 1px solid #d4a64a;
    border-radius: 4px;
    background: rgba(212, 166, 74, 0.2);
    color: #fef3c7;
    font: inherit;
    font-size: 0.72rem;
    font-weight: 600;
    padding: 0.1rem 0.45rem;
    cursor: pointer;
  }

  .qlog-btn:hover {
    background: rgba(212, 166, 74, 0.34);
  }

  .qsum-line {
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    gap: 0.4rem;
    font-size: 0.8rem;
  }

  .qsum-title {
    color: #fde68a;
  }

  .qsum-step-badge {
    padding: 0.04rem 0.36rem;
    border-radius: 999px;
    border: 1px solid rgba(251, 191, 36, 0.55);
    background: rgba(212, 166, 74, 0.16);
    color: #fde68a;
    font-family: var(--font-mono);
    font-size: 0.66rem;
    font-weight: 700;
  }

  .qsum-prog {
    color: #93c5fd;
    font-family: var(--font-mono);
    font-size: 0.72rem;
  }

  .qsum-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem;
    margin-top: 0.3rem;
  }

  .qsum-action-btn {
    border: 1px solid #d4a64a;
    border-radius: 4px;
    background: rgba(212, 166, 74, 0.18);
    color: #fef3c7;
    font: inherit;
    font-size: 0.72rem;
    font-weight: 600;
    padding: 0.14rem 0.45rem;
    cursor: pointer;
    text-align: left;
  }

  .qsum-action-btn.ready {
    border-color: rgba(52, 211, 153, 0.75);
    background: rgba(16, 185, 129, 0.22);
    color: #a7f3d0;
  }

  .qsum-action-btn.ready:hover {
    background: rgba(16, 185, 129, 0.34);
  }

  .qsum-action-btn.offer:hover {
    background: rgba(212, 166, 74, 0.34);
  }

  .qsum-next,
  .qsum-done {
    margin: 0.18rem 0 0;
    font-size: 0.75rem;
    color: var(--fg-muted);
  }

  .qsum-done {
    color: #86efac;
  }

  .quest-summary.mastered {
    border-color: rgba(52, 211, 153, 0.7);
    background: linear-gradient(90deg, rgba(16, 185, 129, 0.14), rgba(212, 166, 74, 0.1));
  }

  .qsum-tour-badge {
    margin-left: 0.45rem;
    padding: 0.06rem 0.4rem;
    border-radius: 999px;
    border: 1px solid rgba(251, 191, 36, 0.65);
    background: rgba(212, 166, 74, 0.2);
    color: #fde68a;
    font-family: var(--font-mono);
    font-size: 0.68rem;
    font-weight: 700;
    vertical-align: middle;
  }

  .qsum-tour-badge.mastered {
    border-color: rgba(52, 211, 153, 0.75);
    background: rgba(16, 185, 129, 0.22);
    color: #a7f3d0;
  }

  .qsum-roadmap {
    display: flex;
    flex-wrap: wrap;
    gap: 0.25rem;
    margin: 0.25rem 0 0.35rem;
  }

  .qsum-step {
    display: inline-flex;
    align-items: center;
    padding: 0.1rem 0.38rem;
    border-radius: 4px;
    border: 1px solid rgba(148, 163, 184, 0.28);
    background: rgba(15, 23, 42, 0.55);
    color: var(--fg-muted);
    font: inherit;
    font-size: 0.69rem;
    line-height: 1.2;
  }

  button.qsum-step {
    cursor: pointer;
  }

  .qsum-step.done {
    border-color: rgba(52, 211, 153, 0.55);
    background: rgba(16, 185, 129, 0.14);
    color: #6ee7b7;
  }

  .qsum-step.current,
  .qsum-step.avail {
    border-color: rgba(251, 191, 36, 0.75);
    background: rgba(212, 166, 74, 0.22);
    color: #fde68a;
    font-weight: 700;
  }

  .qsum-step.upcoming {
    opacity: 0.68;
  }
</style>
