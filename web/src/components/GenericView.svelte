<script lang="ts">
  // The generic semantic-view renderer (interface plan I9): one component
  // that walks a `GenericView` schema into a list, a detail pane or a
  // read-only form, so a module ships a view and gets rendered here with no
  // bespoke code. The web renders richer than the TUI from the same schema
  // (a real table, an inline detail), but from the identical convention
  // `genericview.ts` and the TUI's `render_view_rows` both walk.
  //
  // D-I-18: a row's action is always a Prefill. Clicking a row opens the
  // composer with its line ready; the user confirms with Enter. This
  // component never sends a mutating command on its own.
  import { cellText, rowActionLine, type GenericView } from "$lib/genericview";

  let {
    view,
    onPrefill,
  }: { view: GenericView; onPrefill: (line: string) => void } = $props();

  const schema = $derived(view.schema);

  function fieldText(key: string): string {
    return view.record ? cellText(view.record, key) : "";
  }
</script>

{#if schema.shape === "list"}
  <div class="scroll">
    <table class="gv" aria-label={schema.id}>
      <thead>
        <tr>
          {#each schema.columns as c (c.key)}
            <th class:right={c.align === "right"}>{c.label}</th>
          {/each}
        </tr>
      </thead>
      <tbody>
        {#if view.rows.length === 0}
          <tr><td class="empty" colspan={schema.columns.length}>Nothing here.</td></tr>
        {/if}
        {#each view.rows as row, i (i)}
          {@const line = rowActionLine(row, schema)}
          <tr
            class:actionable={line !== null}
            onclick={() => line !== null && onPrefill(line)}
            title={line ?? undefined}
          >
            {#each schema.columns as c (c.key)}
              <td class:right={c.align === "right"}>{cellText(row, c.key)}</td>
            {/each}
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
{:else}
  <dl class="detail" aria-label={schema.id}>
    {#each schema.fields as f (f.key)}
      <div class="row">
        <dt>{f.label}</dt>
        <dd class:num={f.kind === "number"}>{fieldText(f.key)}</dd>
      </div>
    {/each}
    {#if schema.fields.length === 0}
      <p class="empty">This view has no fields.</p>
    {/if}
  </dl>
{/if}

<style>
  .scroll {
    overflow-x: auto;
  }

  .gv {
    border-collapse: collapse;
    width: 100%;
    font-family: var(--font-mono);
    font-size: 0.85rem;
  }

  .gv th,
  .gv td {
    text-align: left;
    padding: 0.2rem 0.6rem 0.2rem 0;
    border-bottom: 1px solid var(--border);
    white-space: nowrap;
  }

  .gv th {
    color: var(--fg-muted);
    font-weight: 600;
  }

  .right {
    text-align: right;
  }

  tr.actionable {
    cursor: pointer;
  }

  tr.actionable:hover td {
    background: var(--bg-panel);
    color: var(--fg);
  }

  .empty {
    color: var(--fg-faint);
    padding: 0.4rem 0;
  }

  .detail {
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    font-family: var(--font-mono);
    font-size: 0.88rem;
  }

  .detail .row {
    display: flex;
    gap: 0.6rem;
    align-items: baseline;
  }

  .detail dt {
    min-width: 8rem;
    color: var(--fg-muted);
  }

  .detail dd {
    margin: 0;
  }

  .detail dd.num {
    font-variant-numeric: tabular-nums;
  }
</style>
