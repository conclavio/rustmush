<script lang="ts">
  // The main scrollback: renders the store's visible window, autoscrolls
  // while pinned to the bottom, shows the "N new lines" pill while the
  // reader has scrolled up, and offers "show older" at the top.
  import { tick } from "svelte";

  import { fmtClock } from "$lib/render";
  import type { ScrollbackStore } from "$lib/stores/scrollback.svelte";

  import RichLine from "./RichLine.svelte";

  let { sb, onCmd }: { sb: ScrollbackStore; onCmd: (cmd: string) => void } = $props();

  let viewport: HTMLDivElement | undefined = $state();

  /** Pinned = within this many px of the bottom. */
  const PIN_SLOP = 48;

  function onScroll(): void {
    if (!viewport) {
      return;
    }
    const dist = viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight;
    sb.setPinned(dist < PIN_SLOP);
  }

  function jumpToBottom(): void {
    if (viewport) {
      viewport.scrollTop = viewport.scrollHeight;
    }
    sb.setPinned(true);
  }

  // Autoscroll on new content while pinned.
  $effect(() => {
    void sb.entries.length; // track
    if (sb.pinned) {
      void tick().then(() => {
        if (viewport) {
          viewport.scrollTop = viewport.scrollHeight;
        }
      });
    }
  });
</script>

<div class="wrap">
  <div class="viewport" bind:this={viewport} onscroll={onScroll}>
    {#if sb.hiddenCount > 0}
      <button type="button" class="older" onclick={() => sb.showOlder()}>
        Show older ({sb.hiddenCount} more)
      </button>
    {/if}
    {#each sb.visible as block (block.id)}
      <div class="block {block.cls}">
        <span class="ts">{fmtClock(block.ts)}</span>
        <div class="lines">
          {#each block.lines as line, i (i)}
            <RichLine {line} {onCmd} />
          {/each}
        </div>
      </div>
    {/each}
  </div>
  {#if !sb.pinned && sb.unseen > 0}
    <button type="button" class="pill" onclick={jumpToBottom}>
      {sb.unseen} new line{sb.unseen === 1 ? "" : "s"} ↓
    </button>
  {/if}
</div>

<style>
  .wrap {
    position: relative;
    flex: 1;
    min-height: 0;
    display: flex;
  }

  .viewport {
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
    padding: 0.5rem 0.75rem;
    font-family: var(--font-mono);
    font-size: 0.92rem;
  }

  .older {
    display: block;
    width: 100%;
    margin: 0 0 0.5rem;
    padding: 0.3rem;
    background: var(--bg-input);
    color: var(--fg-muted);
    border: 1px solid var(--border);
    border-radius: 6px;
    cursor: pointer;
    font: inherit;
    font-size: 0.8rem;
  }

  .older:hover {
    color: var(--fg);
  }

  .block {
    display: flex;
    gap: 0.6rem;
    padding: 0.07rem 0;
  }

  .ts {
    flex: none;
    width: 2.9rem;
    color: var(--fg-faint);
    font-size: 0.75rem;
    line-height: 1.7;
    user-select: none;
  }

  .lines {
    flex: 1;
    min-width: 0;
  }

  .pill {
    position: absolute;
    bottom: 0.75rem;
    left: 50%;
    transform: translateX(-50%);
    padding: 0.25rem 0.8rem;
    border-radius: 999px;
    border: 1px solid var(--accent);
    background: var(--bg-panel);
    color: var(--accent);
    cursor: pointer;
    font: inherit;
    font-size: 0.8rem;
    box-shadow: 0 2px 8px rgb(0 0 0 / 35%);
  }
</style>
