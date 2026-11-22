<script lang="ts">
  // The playtest guide (`docs/playthrough.md`, bundled at build time), shown
  // over the login screen. Escape or the close button returns to the login.
  import guideSource from "../../../docs/playthrough.md?raw";
  import { renderMarkdown } from "$lib/markdown";

  let { onclose }: { onclose: () => void } = $props();

  const html = renderMarkdown(guideSource);
  let closeButton = $state<HTMLButtonElement | null>(null);

  $effect(() => {
    closeButton?.focus();
  });

  function onkeydown(ev: KeyboardEvent): void {
    if (ev.key === "Escape") onclose();
  }
</script>

<svelte:window {onkeydown} />

<div class="overlay" role="dialog" aria-modal="true" aria-label="Playtest guide">
  <header>
    <span class="title">Playtest guide</span>
    <button type="button" bind:this={closeButton} onclick={onclose}>Back to sign in</button>
  </header>
  <article class="guide">
    <!-- Safe: renderMarkdown escapes raw HTML and refuses unsafe link targets. -->
    {@html html}
  </article>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 20;
    display: flex;
    flex-direction: column;
    background: var(--bg);
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    padding: 0.6rem 1rem;
    border-bottom: 1px solid var(--border);
    background: var(--bg-panel);
  }

  .title {
    font-weight: 600;
    letter-spacing: 0.03em;
  }

  header button {
    background: none;
    border: 1px solid var(--accent-dim);
    border-radius: 6px;
    color: var(--accent);
    padding: 0.35rem 0.7rem;
    font: inherit;
    font-size: 0.85rem;
    cursor: pointer;
  }

  .guide {
    flex: 1;
    overflow-y: auto;
    padding: 1.25rem 1rem 3rem;
    width: min(52rem, 100%);
    margin: 0 auto;
    box-sizing: border-box;
    line-height: 1.55;
    font-size: 0.92rem;
  }

  .guide :global(h1) {
    font-size: 1.45rem;
    margin: 0.2rem 0 0.8rem;
  }

  .guide :global(h2) {
    font-size: 1.15rem;
    margin: 1.8rem 0 0.6rem;
    padding-bottom: 0.25rem;
    border-bottom: 1px solid var(--border);
  }

  .guide :global(h3) {
    font-size: 1rem;
    margin: 1.3rem 0 0.4rem;
  }

  .guide :global(p),
  .guide :global(ul),
  .guide :global(ol) {
    margin: 0.5rem 0;
  }

  .guide :global(li) {
    margin: 0.3rem 0;
  }

  .guide :global(a) {
    color: var(--accent);
  }

  .guide :global(code) {
    font-family: var(--font-mono);
    font-size: 0.86em;
  }

  .guide :global(:not(pre) > code) {
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 0 0.25em;
  }

  .guide :global(pre) {
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 0.6rem 0.75rem;
    overflow-x: auto;
    margin: 0.5rem 0;
  }

  .guide :global(blockquote) {
    margin: 0.8rem 0;
    padding: 0.4rem 0.8rem;
    border-left: 3px solid var(--accent-dim);
    background: var(--bg-panel);
    color: var(--fg-muted);
  }

  .guide :global(.table-wrap) {
    overflow-x: auto;
    margin: 0.6rem 0;
  }

  .guide :global(table) {
    border-collapse: collapse;
    font-size: 0.86rem;
  }

  .guide :global(th),
  .guide :global(td) {
    border: 1px solid var(--border);
    padding: 0.3rem 0.55rem;
    text-align: left;
    vertical-align: top;
  }

  .guide :global(th) {
    background: var(--bg-panel);
  }
</style>
