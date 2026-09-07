<script lang="ts">
  // A key/value list (interface plan I6) with the condition glyph in front
  // of each row: `✓ Label  value`. Values and ids are opaque.
  import { conditionGlyph } from "$lib/widgets";

  export interface KeyValueRow {
    key: string;
    value: string;
    /** A condition id through the glyph map (a vitals state by default). */
    condition?: string;
    detail?: string | null;
  }

  let { rows, glyphs }: { rows: KeyValueRow[]; glyphs?: Readonly<Record<string, string>> } = $props();
</script>

<dl class="kv">
  <!-- Keyed by INDEX, like Table.svelte: the keys are producer-chosen ids,
       and a repeat is legal upstream (one id granted from two sources), so
       keying on them would throw the whole block out on a duplicate. Rows
       are replaced wholesale on every frame, so nothing is lost. -->
  {#each rows as r, i (i)}
    <div class="row state-{r.condition ?? 'ok'}" title={r.detail ?? r.key}>
      <span class="glyph" aria-hidden="true">{conditionGlyph(r.condition ?? "ok", glyphs)}</span>
      <dt>{r.key}</dt>
      <dd>{r.value}{#if r.detail}<span class="detail"> {r.detail}</span>{/if}</dd>
    </div>
  {/each}
</dl>

<style>
  .kv {
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    font-family: var(--font-mono);
    font-size: 0.9rem;
  }

  .row {
    display: flex;
    gap: 0.5rem;
    align-items: baseline;
  }

  .glyph {
    width: 1rem;
    text-align: center;
  }

  dt {
    min-width: 6rem;
    color: var(--fg-muted);
  }

  dd {
    margin: 0;
  }

  .detail {
    color: var(--fg-faint);
    font-size: 0.8rem;
  }

  .state-warn {
    color: var(--warn);
  }

  .state-crit,
  .state-down {
    color: var(--error);
  }
</style>
