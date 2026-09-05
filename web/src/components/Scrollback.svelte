<script lang="ts">
  // The main scrollback: renders the store's visible window as speaker
  // groups (interface plan I5: consecutive speech and action by one actor
  // under a single name header, everything else standing alone), autoscrolls
  // while pinned to the bottom, shows the "N new lines" pill while the
  // reader has scrolled up, and offers "show older" at the top.
  import { tick } from "svelte";

  import { fmtClock } from "$lib/render";
  import type { ScrollbackStore } from "$lib/stores/scrollback.svelte";

  import RichLine from "./RichLine.svelte";
  import RollCard from "./RollCard.svelte";

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
    {#each sb.visibleGroups as group (group.id)}
      {#if group.speaker}
        <div class="group">
          <div class="ghead">
            <span class="ts">{fmtClock(group.blocks[0]?.ts ?? 0)}</span>
            <span class="speaker tag-player">{group.speaker.name}</span>
          </div>
          {#each group.blocks as block (block.id)}
            <div class="block grouped {block.cls}">
              <span class="ts">{fmtClock(block.ts)}</span>
              <div class="lines">
                {#each block.body ?? block.lines as line, i (i)}
                  <RichLine {line} {onCmd} />
                {/each}
              </div>
            </div>
          {/each}
        </div>
      {:else}
        {#each group.blocks as block (block.id)}
          <div class="block {block.cls}">
            <span class="ts">{fmtClock(block.ts)}</span>
            {#if block.roll}
              <div class="lines">
                <RollCard entry={block.roll} />
              </div>
            {:else}
              <div class="lines">
                {#each block.lines as line, i (i)}
                  <RichLine {line} {onCmd} />
                {/each}
              </div>
            {/if}
          </div>
        {/each}
      {/if}
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

  .group {
    margin: 0.35rem 0;
  }

  .ghead {
    display: flex;
    gap: 0.6rem;
    align-items: baseline;
  }

  .ghead .ts {
    visibility: hidden;
  }

  .speaker {
    font-family: var(--font-ui);
    font-weight: 600;
    font-size: 0.85rem;
  }

  .grouped .ts {
    opacity: 0;
    transition: opacity 120ms;
  }

  .grouped:hover .ts {
    opacity: 1;
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

  /* A phone (I5): the timestamp column goes, prose fills the width. */
  @media (max-width: 800px) {
    .block .ts,
    .ghead .ts {
      display: none;
    }

    .viewport {
      padding: 0.4rem 0.5rem;
    }
  }
</style>
