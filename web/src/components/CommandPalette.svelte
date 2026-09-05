<script lang="ts">
  // The command palette (interface plan I4): `commands.list` merged with
  // the channel aliases and the exits, filtered on what follows the slash.
  // Enter inserts the highlighted entry into the composer, Ctrl+Enter runs
  // it, `?` on an entry asks `help <cmd>` in scrollback; Esc closes and
  // keeps the text. Rendered above the composer; the composer owns the
  // keys (it has focus), this is the list.
  import type { GameApp } from "$lib/game.svelte";

  let { app }: { app: GameApp } = $props();
</script>

{#if app.screens.paletteOpen}
  <div class="palette" role="listbox" aria-label="command palette">
    {#if app.paletteEntries.length === 0}
      <div class="empty">no matches</div>
    {/if}
    {#each app.paletteEntries as e, i (e.label + e.detail)}
      <button
        type="button"
        role="option"
        class="entry"
        class:selected={i === app.paletteSelected}
        aria-selected={i === app.paletteSelected}
        onmousedown={(ev) => ev.preventDefault()}
        onclick={() => {
          app.paletteSelected = i;
          app.palettePick();
        }}
      >
        <span class="label" class:local={e.local}>{e.label}</span>
        <span class="detail">{e.detail}</span>
      </button>
    {/each}
    <div class="hint">Enter inserts · Ctrl+Enter runs · Esc closes</div>
  </div>
{/if}

<style>
  .palette {
    position: absolute;
    left: 0.75rem;
    right: 0.75rem;
    bottom: 100%;
    max-height: 16rem;
    overflow-y: auto;
    background: var(--bg-panel);
    border: 1px solid var(--accent-dim);
    border-radius: 8px 8px 0 0;
    box-shadow: 0 -4px 16px rgb(0 0 0 / 35%);
    display: flex;
    flex-direction: column;
    z-index: 20;
  }

  .entry {
    display: flex;
    gap: 0.8rem;
    align-items: baseline;
    text-align: left;
    background: none;
    border: none;
    color: var(--fg);
    cursor: pointer;
    font: inherit;
    font-size: 0.85rem;
    padding: 0.3rem 0.7rem;
  }

  .entry.selected {
    background: var(--accent-dim);
  }

  .label {
    font-family: var(--font-mono);
    min-width: 10rem;
  }

  .label.local {
    color: var(--accent);
  }

  .detail {
    color: var(--fg-muted);
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .empty,
  .hint {
    padding: 0.3rem 0.7rem;
    color: var(--fg-faint);
    font-size: 0.75rem;
  }
</style>
