<script lang="ts">
  // The notification surface (interface plan I11): transient toasts for the
  // newest unseen notices, and a persistent tray (the bell) for the history.
  // A notice's action is a Prefill (D-I-18): a tap opens the composer with the
  // line ready. Colour and order come from the server's severity — the client
  // never decides how loud a notice is.
  import type { GameApp } from "$lib/game.svelte";
  import { renderRich } from "$lib/render";
  import { TOAST_TTL_MS, type Notification } from "$lib/stores/notifications.svelte";

  import RichLine from "./RichLine.svelte";

  let { app }: { app: GameApp } = $props();

  let trayOpen = $state(false);

  const toasts = $derived(app.notifications.toasts);

  // Each visible toast auto-dismisses after the TTL (marks it seen, so it
  // leaves the toasts but stays in the tray).
  $effect(() => {
    const timers = toasts.map((t) =>
      setTimeout(() => app.notifications.markSeen(t.rowId), TOAST_TTL_MS),
    );
    return () => timers.forEach(clearTimeout);
  });

  function act(n: Notification): void {
    if (n.action) {
      app.focusComposer(n.action.prefill);
    }
    app.notifications.markSeen(n.rowId);
    trayOpen = false;
  }

  function toggleTray(): void {
    trayOpen = !trayOpen;
    if (trayOpen) {
      app.notifications.markAllSeen();
    }
  }
</script>

<!-- Toasts: top-right, transient -->
<div class="toasts" aria-live="polite">
  {#each toasts as n (n.rowId)}
    <div class="toast sev-{n.severity}">
      <span class="cat">{n.category}</span>
      <div class="body"><RichLine line={renderRich(n.text)} onCmd={() => {}} /></div>
      {#if n.action}
        <button type="button" class="act" onclick={() => act(n)}>{n.action.label}</button>
      {/if}
      <button type="button" class="x" aria-label="dismiss" onclick={() => app.notifications.markSeen(n.rowId)}>×</button>
    </div>
  {/each}
</div>

<!-- The bell + tray -->
<div class="bell-wrap">
  <button
    type="button"
    class="bell"
    title="notifications"
    aria-label="notifications"
    onclick={toggleTray}
  >
    🔔
    {#if app.notifications.unseen > 0}
      <span class="badge">{app.notifications.unseen > 99 ? "99+" : app.notifications.unseen}</span>
    {/if}
  </button>
  {#if trayOpen}
    <div class="tray" role="dialog" aria-label="notifications">
      <header>
        <strong>Notifications</strong>
        {#if app.notifications.list.length > 0}
          <button type="button" class="clear" onclick={() => app.notifications.clear()}>Clear</button>
        {/if}
      </header>
      {#if app.notifications.list.length === 0}
        <p class="empty">Nothing here.</p>
      {/if}
      <ul>
        {#each app.notifications.list as n (n.rowId)}
          <li class="sev-{n.severity}">
            <span class="cat">{n.category}</span>
            <div class="body"><RichLine line={renderRich(n.text)} onCmd={() => {}} /></div>
            <div class="row">
              {#if n.action}
                <button type="button" class="act" onclick={() => act(n)}>{n.action.label}</button>
              {/if}
              <button type="button" class="x" onclick={() => app.notifications.dismiss(n.rowId)}>Dismiss</button>
            </div>
          </li>
        {/each}
      </ul>
    </div>
  {/if}
</div>

<style>
  .toasts {
    position: fixed;
    top: 0.75rem;
    right: 0.75rem;
    z-index: 60;
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    width: min(22rem, calc(100vw - 1.5rem));
    pointer-events: none;
  }

  .toast {
    pointer-events: auto;
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.45rem 0.6rem;
    border-radius: 8px;
    border: 1px solid var(--border);
    border-left: 3px solid var(--accent);
    background: var(--bg-panel);
    box-shadow: 0 4px 16px rgb(0 0 0 / 30%);
    font-size: 0.85rem;
  }

  .sev-warn {
    border-left-color: var(--warn);
  }

  .sev-urgent {
    border-left-color: var(--error);
  }

  .cat {
    font-size: 0.68rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--fg-faint);
    flex: none;
  }

  .body {
    flex: 1;
    min-width: 0;
  }

  .act {
    flex: none;
    font: inherit;
    font-size: 0.78rem;
    padding: 0.15rem 0.55rem;
    border-radius: 5px;
    border: 1px solid var(--accent);
    background: var(--accent);
    color: var(--bg);
    cursor: pointer;
  }

  .x {
    flex: none;
    background: none;
    border: none;
    color: var(--fg-muted);
    cursor: pointer;
    font-size: 1rem;
    line-height: 1;
  }

  .bell-wrap {
    position: relative;
    display: inline-block;
  }

  .bell {
    background: none;
    border: none;
    cursor: pointer;
    font-size: 1.1rem;
    position: relative;
    padding: 0.2rem;
  }

  .badge {
    position: absolute;
    top: -0.1rem;
    right: -0.2rem;
    background: var(--error);
    color: var(--bg);
    border-radius: 999px;
    font-size: 0.6rem;
    font-weight: 700;
    padding: 0 0.3rem;
    min-width: 1rem;
    text-align: center;
  }

  .tray {
    position: absolute;
    top: 2rem;
    right: 0;
    width: min(24rem, calc(100vw - 1.5rem));
    max-height: 60vh;
    overflow-y: auto;
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: 10px;
    box-shadow: 0 8px 32px rgb(0 0 0 / 40%);
    z-index: 60;
  }

  .tray header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 0.5rem 0.7rem;
    border-bottom: 1px solid var(--border);
  }

  .clear {
    background: none;
    border: none;
    color: var(--fg-muted);
    cursor: pointer;
    font: inherit;
    font-size: 0.8rem;
  }

  .empty {
    color: var(--fg-faint);
    padding: 0.8rem;
    font-size: 0.85rem;
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  li {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    padding: 0.5rem 0.7rem;
    border-bottom: 1px solid var(--border);
    border-left: 3px solid transparent;
    font-size: 0.85rem;
  }

  li.sev-warn {
    border-left-color: var(--warn);
  }

  li.sev-urgent {
    border-left-color: var(--error);
  }

  .row {
    display: flex;
    gap: 0.5rem;
    align-items: center;
  }

  @media (max-width: 800px) {
    .tray {
      position: fixed;
      top: auto;
      bottom: 0;
      right: 0;
      left: 0;
      width: 100vw;
      max-height: 70vh;
      border-radius: 12px 12px 0 0;
    }
  }
</style>
