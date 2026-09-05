<script lang="ts">
  // A table (interface plan I6) whose columns the caller declares; rows are
  // plain records and a missing cell is empty.
  import { tableCells, type TableColumn } from "$lib/widgets";

  let {
    columns,
    rows,
    caption = null,
  }: { columns: TableColumn[]; rows: Record<string, unknown>[]; caption?: string | null } = $props();
</script>

<table class="tbl">
  {#if caption}<caption>{caption}</caption>{/if}
  <thead>
    <tr>
      {#each columns as c (c.key)}
        <th class:right={c.align === "right"}>{c.label}</th>
      {/each}
    </tr>
  </thead>
  <tbody>
    {#each rows as row, i (i)}
      <tr>
        {#each tableCells(columns, row) as cell, j (j)}
          <td class:right={columns[j]?.align === "right"}>{cell}</td>
        {/each}
      </tr>
    {/each}
  </tbody>
</table>

<style>
  .tbl {
    border-collapse: collapse;
    width: 100%;
    font-family: var(--font-mono);
    font-size: 0.85rem;
  }

  caption {
    text-align: left;
    color: var(--fg-muted);
    font-size: 0.75rem;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    padding-bottom: 0.25rem;
  }

  th,
  td {
    text-align: left;
    padding: 0.15rem 0.5rem 0.15rem 0;
    border-bottom: 1px solid var(--border);
    white-space: nowrap;
  }

  th {
    color: var(--fg-muted);
    font-weight: 600;
  }

  .right {
    text-align: right;
  }
</style>
