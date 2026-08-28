<script lang="ts">
  // One rendered line: spans with tag classes, explicit styles, and
  // click-to-run CommandLinks (buttons, for keyboard accessibility).
  import type { RLine } from "$lib/render";

  let { line, onCmd }: { line: RLine; onCmd: (cmd: string) => void } = $props();
</script>

<div class="rline">
  {#each line as s, i (i)}
    {#if s.cmd !== null}
      <button
        type="button"
        class="lspan islink {s.cls}"
        style={s.css}
        title={s.cmd}
        onclick={() => onCmd(s.cmd!)}>{s.text}</button
      >
    {:else}
      <span class="lspan {s.cls}" style={s.css}>{s.text}</span>
    {/if}
  {/each}
</div>

<style>
  .rline {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  /* `:where()` zeroes this rule's specificity so it never outranks the
     single-class `tag-*`/`ansi-*` color classes from app.css (a plain
     `.lspan { color: inherit }` — even un-scoped — has the same specificity
     as those classes; Svelte's scoping hash bumps it to two classes, which
     silently wins and makes every semantic/explicit color invisible). Only
     `color` needs this; `font` isn't contested. */
  :where(.lspan) {
    color: inherit;
  }

  .lspan {
    font: inherit;
  }

  button.islink {
    background: none;
    border: none;
    padding: 0;
    margin: 0;
    cursor: pointer;
    text-decoration: underline dotted;
    text-underline-offset: 2px;
  }

  button.islink:hover {
    text-decoration-style: solid;
    filter: brightness(1.2);
  }
</style>
