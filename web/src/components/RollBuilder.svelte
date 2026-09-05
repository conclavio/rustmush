<script lang="ts">
  // The roll builder (interface plan I6): trait pickers from the sheet's
  // categories, a difficulty in 3..9, the specialty switch, and
  // the `+roll` line it will send — shown before it goes, editable in the
  // composer with one click. Hidden until a sheet with categorised traits
  // is loaded.
  import type { GameApp } from "$lib/game.svelte";
  import {
    DIFFICULTY_DEFAULT,
    DIFFICULTY_MAX,
    DIFFICULTY_MIN,
    buildRollLine,
    traitGroups,
  } from "$lib/rollbuilder";

  let { app }: { app: GameApp } = $props();

  const groups = $derived(traitGroups(app.sheet.schema));
  const difficulties = Array.from(
    { length: DIFFICULTY_MAX - DIFFICULTY_MIN + 1 },
    (_, i) => DIFFICULTY_MIN + i,
  );

  let first = $state("");
  let second = $state("");
  let difficulty = $state(DIFFICULTY_DEFAULT);
  let spec = $state(false);

  const line = $derived(
    buildRollLine({ first, second: second.length > 0 ? second : null, difficulty, spec }),
  );

  function roll(): void {
    if (line !== null) {
      app.submit(line);
    }
  }

  function edit(): void {
    if (line !== null) {
      app.focusComposer(line);
    }
  }
</script>

{#if groups.length > 0}
  <form class="builder" onsubmit={(e) => { e.preventDefault(); roll(); }} aria-label="roll builder">
    <label>
      <span>Trait</span>
      <select bind:value={first}>
        <option value="">—</option>
        {#each groups as g (g.category)}
          <optgroup label={g.label}>
            {#each g.traits as t (t.id)}
              <option value={t.id}>{t.label}</option>
            {/each}
          </optgroup>
        {/each}
      </select>
    </label>
    <label>
      <span>+ Trait</span>
      <select bind:value={second}>
        <option value="">—</option>
        {#each groups as g (g.category)}
          <optgroup label={g.label}>
            {#each g.traits as t (t.id)}
              <option value={t.id}>{t.label}</option>
            {/each}
          </optgroup>
        {/each}
      </select>
    </label>
    <label>
      <span>vs</span>
      <select bind:value={difficulty}>
        {#each difficulties as d (d)}
          <option value={d}>{d}</option>
        {/each}
      </select>
    </label>
    <label class="check"><input type="checkbox" bind:checked={spec} /> spec</label>
    <code class="line">{line ?? "pick a trait"}</code>
    <button type="submit" disabled={line === null}>Roll</button>
    <button type="button" disabled={line === null} onclick={edit}>Edit</button>
  </form>
{/if}

<style>
  .builder {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.5rem 0.75rem;
    padding: 0.5rem 0.65rem;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-panel);
    font-size: 0.85rem;
  }

  label {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    color: var(--fg-muted);
  }

  select {
    font: inherit;
    background: var(--bg-input);
    color: var(--fg);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 0.15rem 0.3rem;
  }

  .line {
    font-family: var(--font-mono);
    font-size: 0.82rem;
    color: var(--fg);
    padding: 0.15rem 0.4rem;
    border-radius: 4px;
    background: var(--bg-input);
  }

  button {
    font: inherit;
    padding: 0.2rem 0.7rem;
    border-radius: 4px;
    border: 1px solid var(--border);
    background: var(--bg-input);
    color: var(--fg);
    cursor: pointer;
  }

  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
