<script lang="ts">
  // A screen backed by a `.view` feed (interface plan I9): renders the feed's
  // held generic view through the shared `GenericView`, and a row's Prefill
  // opens the composer (D-I-18). No feature ships bespoke screen code — the
  // schema drives the whole thing.
  import type { GameApp } from "$lib/game.svelte";

  import GenericView from "./GenericView.svelte";

  let { app, feed }: { app: GameApp; feed: string } = $props();

  const view = $derived(app.genericViews.get(feed));
</script>

<div class="viewscreen">
  {#if view}
    <GenericView {view} onPrefill={(line) => app.focusComposer(line)} />
  {:else}
    <p class="muted">Loading…</p>
  {/if}
</div>

<style>
  .viewscreen {
    height: 100%;
    overflow-y: auto;
    padding: 0.6rem 0.75rem;
  }

  .muted {
    color: var(--fg-muted);
    font-size: 0.85rem;
  }
</style>
