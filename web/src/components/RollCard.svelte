<script lang="ts">
  // The dice card (interface plan I5): the roller's own roll inline in the
  // scrollback from the `.roll` feed — every die with successes and 1s
  // highlighted, the result, the difficulty and the Willpower or automatic
  // successes. Everyone else in the room reads the one-line `roll` narration
  // from I3; the card is the roller's private detail.
  import { classifyResult, dieClass } from "$lib/roll";
  import type { RollEntry } from "$lib/stores/rolls.svelte";

  let { entry }: { entry: RollEntry } = $props();

  const result = $derived(classifyResult(entry.outcome.result));
</script>

<div class="card" data-kind={result.kind} aria-label="your roll">
  <div class="dice">
    {#each entry.outcome.dice as d, i (i)}
      <span class={`die ${dieClass(d)}`}>{d.face}{#if d.doubled}<sup>×2</sup>{/if}</span>
    {/each}
  </div>
  <span class="result">{result.label}</span>
  <span class="meta">
    diff {entry.outcome.difficulty}{#if entry.outcome.willpower_success} · +WP{/if}{#if entry.outcome.extra_successes > 0}
      · +{entry.outcome.extra_successes} auto{/if}
  </span>
</div>

<style>
  .card {
    display: inline-flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.5rem;
    padding: 0.3rem 0.6rem;
    border: 1px solid var(--border);
    border-left: 3px solid var(--c-npc);
    border-radius: 6px;
    background: var(--bg-panel);
    font-family: var(--font-mono);
    font-size: 0.82rem;
  }

  .card[data-kind="botch"] {
    border-left-color: var(--error);
  }

  .card[data-kind="success"] {
    border-left-color: var(--success);
  }

  .dice {
    display: inline-flex;
    gap: 0.25rem;
  }

  .die {
    display: inline-block;
    min-width: 1.4rem;
    text-align: center;
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 0 0.2rem;
    color: var(--fg-muted);
  }

  .die-success {
    color: var(--success);
    border-color: var(--success);
  }

  .die-one {
    color: var(--error);
    border-color: var(--error);
  }

  .die-doubled {
    font-weight: 700;
  }

  sup {
    font-size: 0.6em;
  }

  .result {
    font-weight: 600;
  }

  .card[data-kind="botch"] .result {
    color: var(--error);
  }

  .card[data-kind="success"] .result {
    color: var(--success);
  }

  .meta {
    color: var(--fg-faint);
  }
</style>
