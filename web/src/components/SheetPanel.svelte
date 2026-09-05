<script lang="ts">
  // The schema-driven character sheet: walks `app.sheet.schema.sections`
  // and renders each one purely from its `layout`/`FieldType` — no field id
  // or template name is ever special-cased here, so Vampire/Mage/Werewolf
  // (and any future ruleset) need zero changes on this side. See
  // $lib/sheet.ts for the rendering rules (pip-vs-bar threshold, track mark
  // glyphs, map/list dispatch) and web/README.md for the payload contract.
  import type { GameApp } from "$lib/game.svelte";
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
        The sheet query isn't wired up on this server yet.
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
</style>
