<script lang="ts">
  // The `wod.roll` panel feed: a compact, structured readout of the dice
  // engine's output (highlighted successes/1s, botch/failure/N-successes) —
  // pure push, nothing to fetch. See $lib/roll.ts for the formatting rules;
  // this component only maps `app.rolls.entries` through them.
  import type { GameApp } from "$lib/game.svelte";
  import { classifyResult, dieClass, youOrId } from "$lib/roll";
  import { fmtClock } from "$lib/render";

  import RollBuilder from "./RollBuilder.svelte";
  import VitalsBoard from "./VitalsBoard.svelte";

  let { app }: { app: GameApp } = $props();
</script>

<!-- Interface plan I6: the rolls screen is the dice-and-damage screen — the
     roll builder on top, the vitals board (tracks, pools, readouts, recent
     rolls) beside the roll list. No digit was free for a separate combat
     screen (the plan's I8 wants 8 and 9 on the TUI; the web has only 6). -->
<div class="rolls">
  <RollBuilder {app} />
  <div class="cols">
    <VitalsBoard {app} />
    <div class="list">
  {#if app.rolls.entries.length === 0}
    <p class="muted empty">
      No rolls yet — try <code>+roll &lt;pool&gt;</code> in the terminal.
    </p>
  {:else}
    {#each app.rolls.entries as r (r.rowId)}
      {@const result = classifyResult(r.outcome.result)}
      <div class="roll" data-kind={result.kind}>
        <div class="rollhead">
          <span class="who">{youOrId(r.who, app.session.character?.id ?? null)}</span>
          <span class="result">{result.label}</span>
          <span class="when muted">{fmtClock(r.ts)}</span>
        </div>
        <div class="dice">
          {#each r.outcome.dice as d, i (i)}
            <span class={`die ${dieClass(d)}`}>
              {d.face}{#if d.doubled}<sup>×2</sup>{/if}
            </span>
          {/each}
        </div>
        <div class="meta muted">
          diff {r.outcome.difficulty}{#if r.outcome.willpower_success} · +WP{/if}{#if r.outcome.extra_successes > 0}
            · +{r.outcome.extra_successes} auto{/if}
        </div>
      </div>
    {/each}
  {/if}
    </div>
  </div>
</div>

<style>
  .rolls {
    height: 100%;
    overflow-y: auto;
    padding: 0.6rem 0.75rem 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .cols {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    gap: 0.75rem;
    align-items: start;
  }

  .list {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  @media (max-width: 800px) {
    .cols {
      grid-template-columns: minmax(0, 1fr);
    }
  }

  .empty {
    font-size: 0.85rem;
    padding: 0.5rem 0.1rem;
  }

  .muted {
    color: var(--fg-muted);
  }

  code {
    font-family: var(--font-mono);
    font-size: 0.82em;
  }

  .roll {
    border: 1px solid var(--border);
    border-left: 3px solid var(--fg-faint);
    border-radius: 6px;
    padding: 0.4rem 0.55rem;
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }

  .roll[data-kind="success"] {
    border-left-color: var(--success);
  }

  .roll[data-kind="failure"] {
    border-left-color: var(--fg-faint);
  }

  .roll[data-kind="botch"] {
    border-left-color: var(--error);
  }

  .rollhead {
    display: flex;
    align-items: baseline;
    gap: 0.4rem;
    font-size: 0.82rem;
  }

  .who {
    font-weight: 600;
    color: var(--fg);
  }

  .result {
    flex: 1;
    color: var(--fg-muted);
  }

  .roll[data-kind="success"] .result {
    color: var(--success);
  }

  .roll[data-kind="botch"] .result {
    color: var(--error);
    font-weight: 700;
  }

  .when {
    font-size: 0.72rem;
  }

  .dice {
    display: flex;
    flex-wrap: wrap;
    gap: 0.25rem;
  }

  .die {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 1.35rem;
    height: 1.35rem;
    padding: 0 0.15rem;
    border-radius: 4px;
    border: 1px solid var(--border);
    font-size: 0.75rem;
    font-variant-numeric: tabular-nums;
    color: var(--fg-muted);
  }

  .die-success {
    border-color: var(--success);
    color: var(--success);
    font-weight: 700;
  }

  .die-one {
    border-color: var(--error);
    color: var(--error);
  }

  .die-doubled {
    background: color-mix(in srgb, var(--success) 18%, transparent);
  }

  .meta {
    font-size: 0.72rem;
  }
</style>
