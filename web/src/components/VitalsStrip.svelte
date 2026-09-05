<script lang="ts">
  // The status strip (interface plan I4): every `.vitals` frame as one
  // group of gauges — a track as boxes with its level, a pool as a bar
  // with `current/max`, a readout as text — `Dominant` first. Replaces the
  // textual prompt when a frame exists.
  import type { GameApp } from "$lib/game.svelte";
  import { boxGlyph, orderedVitals, vitalPct } from "$lib/stores/vitals.svelte";

  let { app }: { app: GameApp } = $props();
</script>

<div class="strip" aria-label="vitals">
  {#each app.vitals.list as group (group.feed)}
    <span class="group" title={group.frame.subject.name}>
      {#each orderedVitals(group.frame) as v (v.id)}
        <span class="vital state-{v.state} role-{v.role}" title={v.detail ?? v.label}>
          <span class="label">{v.label}</span>
          {#if v.value.kind === "bounded"}
            <span class="bar" aria-hidden="true">
              <span class="fill" style="width: {vitalPct(v) ?? 0}%"></span>
            </span>
            <span class="num">{v.value.current}/{v.value.max}</span>
          {:else if v.value.kind === "track"}
            <span class="boxes" aria-hidden="true">
              {#each v.value.boxes as b, i (i)}
                <span class="box" class:filled={b !== null}>{boxGlyph(b)}</span>
              {/each}
            </span>
            {#if v.value.level}<span class="num">{v.value.level}</span>{/if}
          {:else}
            <span class="num">{v.value.text}</span>
          {/if}
        </span>
      {/each}
    </span>
  {/each}
</div>

<style>
  .strip {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem 1rem;
    padding: 0.25rem 0.75rem;
    border-top: 1px solid var(--border);
    background: var(--bg-panel);
    font-family: var(--font-mono);
    font-size: 0.78rem;
  }

  .group {
    display: inline-flex;
    flex-wrap: wrap;
    gap: 0.35rem 0.8rem;
  }

  .vital {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    color: var(--fg-muted);
  }

  .role-dominant .label {
    font-weight: 700;
  }

  .state-warn {
    color: var(--warn);
  }

  .state-crit,
  .state-down {
    color: var(--error);
  }

  .state-down .label {
    font-weight: 700;
  }

  .bar {
    display: inline-block;
    width: 4.5rem;
    height: 0.5rem;
    border: 1px solid var(--border);
    border-radius: 3px;
    overflow: hidden;
    background: var(--bg-input);
  }

  .fill {
    display: block;
    height: 100%;
    background: currentcolor;
    opacity: 0.7;
  }

  .boxes {
    display: inline-flex;
    gap: 1px;
  }

  .box {
    display: inline-block;
    width: 0.8rem;
    height: 0.9rem;
    line-height: 0.9rem;
    text-align: center;
    border: 1px solid var(--border);
    border-radius: 2px;
    font-size: 0.7rem;
  }

  .box.filled {
    border-color: currentcolor;
  }

  .num {
    color: var(--fg);
  }
</style>
