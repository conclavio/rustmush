<script lang="ts">
  // Mail panel: folder dropdown + message list from the `mail.list`
  // keyframe. Opening a message submits `@mail <n>` (the body renders in
  // scrollback, classic-style) and refreshes the list; compose is the
  // one-shot `@mail/quick` modal — the full multi-line compose remains a
  // terminal flow.
  import type { GameApp } from "$lib/game.svelte";
  import { fmtClock } from "$lib/render";

  import ComposeModal from "./ComposeModal.svelte";

  let { app }: { app: GameApp } = $props();

  let composing = $state(false);
</script>

<div class="mail">
  <div class="head">
    <select
      aria-label="mail folder"
      value={app.mail.folder}
      onchange={(ev) => app.mailSwitchFolder(ev.currentTarget.value)}
    >
      {#each app.mail.folders as f (f.name)}
        <option value={f.name}>
          {f.name}
          {f.unread > 0 ? `(${f.unread} unread)` : `(${f.count})`}
        </option>
      {/each}
    </select>
    <button type="button" class="compose" onclick={() => (composing = true)}>Compose</button>
  </div>

  <div class="list">
    {#if !app.mail.loaded}
      <p class="muted">Loading…</p>
    {:else if app.mail.entries.length === 0}
      <p class="muted">No mail in {app.mail.folder}.</p>
    {:else}
      {#each app.mail.entries as e (e.rowId)}
        <button
          type="button"
          class="entry"
          class:unread={e.status !== "read"}
          onclick={() => app.mailOpen(e.index)}
          title={`@mail ${e.index}`}
        >
          <span class="from">
            {#if e.status === "urgent"}<span class="urgent" title="urgent">!</span>{/if}
            {e.from.name}
          </span>
          <span class="subject">{e.subject}{#if e.delivery}<br /><small>{e.delivery}</small>{/if}</span>
          <span class="when">{fmtClock(e.sentAt)}</span>
        </button>
      {/each}
    {/if}
  </div>

  <div class="foot">
    {#if app.mail.quota}
      <span class="muted">quota {app.mail.quota.used}/{app.mail.quota.max}</span>
    {:else if app.mail.loaded}
      <span class="muted">no quota (staff)</span>
    {/if}
    <span class="muted hint">All mail travels in character. Full compose: `@mail/send`.</span>
  </div>
</div>

{#if composing}
  <ComposeModal
    onsend={(to, subject, body) => {
      app.mailQuickSend(to, subject, body);
      composing = false;
    }}
    oncancel={() => (composing = false)}
  />
{/if}

<style>
  .mail {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }

  .head {
    display: flex;
    gap: 0.5rem;
    padding: 0.5rem 0.75rem;
    border-bottom: 1px solid var(--border);
  }

  select {
    flex: 1;
    background: var(--bg-input);
    color: var(--fg);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 0.25rem 0.4rem;
    font: inherit;
    font-size: 0.85rem;
  }

  .compose {
    background: var(--accent-dim);
    color: var(--fg);
    border: none;
    border-radius: 6px;
    padding: 0.25rem 0.7rem;
    cursor: pointer;
    font: inherit;
    font-size: 0.85rem;
  }

  .compose:hover {
    filter: brightness(1.15);
  }

  .list {
    flex: 1;
    overflow-y: auto;
    padding: 0.3rem 0.4rem;
  }

  .muted {
    color: var(--fg-muted);
    font-size: 0.85rem;
    padding: 0.3rem;
  }

  .entry {
    display: grid;
    grid-template-columns: 1fr auto;
    grid-template-areas: "from when" "subject subject";
    width: 100%;
    gap: 0 0.5rem;
    text-align: left;
    background: none;
    border: none;
    border-bottom: 1px solid var(--border);
    padding: 0.35rem 0.35rem;
    color: var(--fg-muted);
    cursor: pointer;
    font: inherit;
    font-size: 0.85rem;
  }

  .entry:hover {
    background: var(--bg-input);
  }

  .entry.unread {
    color: var(--fg);
  }

  .entry.unread .from,
  .entry.unread .subject {
    font-weight: 700;
  }

  .from {
    grid-area: from;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .urgent {
    color: var(--error);
    font-weight: 800;
    margin-right: 0.2rem;
  }

  .subject {
    grid-area: subject;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .when {
    grid-area: when;
    color: var(--fg-faint);
    font-size: 0.75rem;
  }

  .foot {
    display: flex;
    justify-content: space-between;
    gap: 0.5rem;
    padding: 0.35rem 0.75rem;
    border-top: 1px solid var(--border);
    flex-wrap: wrap;
  }

  .hint {
    font-size: 0.72rem;
  }
</style>
