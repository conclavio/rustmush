<script lang="ts">
  // The tactical unit screen (interface cut I8, D-I8-4): ONE screen on the
  // one free web digit hosting BOTH layouts — `t` flips them — where the
  // TUI has room for two. The `damage` layout is the journal read as a
  // screen (the section map, the after-action table, the ring); the `unit`
  // layout is the piloting side (sections, the object's own gauges and
  // mobility, its components and capabilities, the contacts list).
  //
  // Composed from EXISTING widgets only, and from the frame's own words:
  // this file names no part, no pool and no ruleset (the probe in
  // vitalsboard.test.ts greps it for exactly that). Rule 1 again: every
  // glyph and colour comes from a `state`; a `condition` word is printed.
  //
  // Panes render in `unitPaneOrder`, which puts what has no producer yet
  // LAST — so the narrow-width rule below drops the placeholders first and
  // the real panes survive on a phone.
  import type { GameApp } from "$lib/game.svelte";
  import {
    afterAction,
    beforeAfterRows,
    capabilityRows,
    componentGroups,
    CONTACTS_FEED,
    journalTableRows,
    mobilityLine,
    NO_UNIT_LINE,
    truncatedFooter,
    UNIT_PANE_TITLES,
    UNIT_PLACEHOLDERS,
    unitCells,
    unitPaneOrder,
    type UnitGroupRow,
  } from "$lib/stores/unit.svelte";
  import type { TableColumn } from "$lib/widgets";

  import GenericView from "./GenericView.svelte";
  import UnitSectionMap from "./UnitSectionMap.svelte";
  import Gauge from "./widgets/Gauge.svelte";
  import KeyValue from "./widgets/KeyValue.svelte";
  import Table from "./widgets/Table.svelte";

  let { app }: { app: GameApp } = $props();

  const BEFORE_AFTER_COLUMNS: TableColumn[] = [
    { key: "addr", label: "addr" },
    { key: "before", label: "before", align: "right" },
    { key: "after", label: "after", align: "right" },
  ];

  const JOURNAL_COLUMNS: TableColumn[] = [
    { key: "rev", label: "rev", align: "right" },
    { key: "cause", label: "cause" },
    { key: "entry", label: "entry" },
  ];

  const view = $derived(app.unit.view);
  const contacts = $derived(app.genericViews.get(CONTACTS_FEED));
  const panes = $derived(unitPaneOrder(app.unit.layout, contacts !== null));

  /** The title of a pane with no producer yet, or `null` for a real one. */
  function placeholderTitle(pane: string): string | null {
    if (pane === "contacts" && contacts !== null) {
      return null;
    }
    return UNIT_PLACEHOLDERS.find((p) => p.id === pane)?.title ?? null;
  }

  /** A grouped row for `KeyValue`. Its `condition` prop is the glyph/colour
   * seam, so it is fed the row's STATE; the ruleset's own condition word
   * rides in `value`, printed and never interpreted (rule 1). */
  function kv(rows: UnitGroupRow[]): {
    key: string;
    value: string;
    condition: string;
    detail: string | null;
  }[] {
    return rows.map((r) => ({ key: r.key, value: r.value, condition: r.state, detail: r.detail }));
  }
</script>

<div class="unitscreen">
  {#if app.unit.unavailable !== null}
    <p class="muted pad">{app.unit.unavailable}</p>
  {:else if view === null}
    <p class="muted pad">{NO_UNIT_LINE}</p>
  {:else}
    {@const footer = truncatedFooter(view)}
    <header class="head">
      <h3>
        {view.object.name}
        <span class="rev">rev {view.revision}</span>
      </h3>
      <p class="sub">
        <span>{view.object.id}</span>
        <span>{view.state}</span>
        {#if view.record !== null}<span class="held">{view.record}</span>{/if}
      </p>
    </header>

    <div class="panes">
      {#each panes as pane (pane)}
        {@const missing = placeholderTitle(pane)}
        <section class="pane" class:placeholder={missing !== null} aria-label={pane}>
          {#if missing !== null}
            <p class="muted">{missing}</p>
          {:else}
            <h4 class="pt">{UNIT_PANE_TITLES[pane] ?? pane}</h4>
            {#if pane === "sectionmap"}
              <UnitSectionMap sections={view.sections} cells={view.cells} />
            {:else if pane === "sections"}
              <UnitSectionMap sections={view.sections} cells={view.cells} compact />
            {:else if pane === "plant"}
              {#each unitCells(view.cells) as c (c.addr)}
                <Gauge
                  label={c.label}
                  current={c.current}
                  min={c.min}
                  max={c.max}
                  state={c.state}
                />
              {/each}
              <p class="line">{mobilityLine(view.mobility)}</p>
            {:else if pane === "components"}
              {#each componentGroups(view) as g (g.id)}
                <p class="grp">{g.label}</p>
                <KeyValue rows={kv(g.rows)} />
              {/each}
              {#if componentGroups(view).length === 0}
                <p class="muted">Nothing here.</p>
              {/if}
            {:else if pane === "capabilities"}
              <KeyValue rows={kv(capabilityRows(view))} />
              {#if view.capabilities.length === 0}
                <p class="muted">Nothing here.</p>
              {/if}
            {:else if pane === "afteraction"}
              <Table
                columns={BEFORE_AFTER_COLUMNS}
                rows={beforeAfterRows(afterAction(view, app.unit.journal))}
              />
            {:else if pane === "journal"}
              <Table
                columns={JOURNAL_COLUMNS}
                rows={journalTableRows(app.unit.journal, view.object.id)}
              />
            {:else if pane === "contacts" && contacts !== null}
              <GenericView view={contacts} onPrefill={(line) => app.focusComposer(line)} />
            {/if}
          {/if}
        </section>
      {/each}
    </div>

    {#if footer !== null}
      <p class="footer">{footer}</p>
    {/if}
  {/if}
</div>

<style>
  .unitscreen {
    height: 100%;
    overflow-y: auto;
    padding: 0.6rem 0.75rem;
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
  }

  .head h3 {
    margin: 0;
    font-size: 0.95rem;
    display: flex;
    justify-content: space-between;
    align-items: baseline;
  }

  .rev {
    font-size: 0.7rem;
    color: var(--fg-faint);
    font-family: var(--font-mono);
  }

  .sub {
    margin: 0.1rem 0 0;
    display: flex;
    gap: 0.6rem;
    font-family: var(--font-mono);
    font-size: 0.75rem;
    color: var(--fg-muted);
  }

  .held {
    color: var(--warn);
  }

  .panes {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(18rem, 1fr));
    gap: 0.6rem;
    align-items: start;
  }

  .pane {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    padding: 0.5rem 0.65rem;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-panel);
    min-width: 0;
    overflow-x: auto;
  }

  .pt {
    margin: 0 0 0.1rem;
    font-size: 0.72rem;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--fg-muted);
    font-weight: 600;
  }

  .grp {
    margin: 0.25rem 0 0.1rem;
    font-family: var(--font-mono);
    font-size: 0.75rem;
    color: var(--fg-faint);
  }

  .line {
    margin: 0.2rem 0 0;
    font-family: var(--font-mono);
    font-size: 0.85rem;
  }

  .footer {
    margin: 0;
    color: var(--fg-faint);
    font-size: 0.75rem;
    font-family: var(--font-mono);
  }

  .muted {
    color: var(--fg-muted);
    font-size: 0.85rem;
    margin: 0;
  }

  .pad {
    padding: 0.8rem;
  }

  /* A phone drops what has no producer yet before it drops anything real —
   * the degrade order the pane list already encodes. */
  @media (max-width: 800px) {
    .pane.placeholder {
      display: none;
    }
  }
</style>
