<script lang="ts">
  // WHO rail: the latest listing with idle/conn ticking locally between
  // refreshes (1 s clock; the server refreshes on `who` events + the
  // refresh button's Req).
  import type { GameApp } from "$lib/game.svelte";
  import { fmtDuration } from "$lib/render";

  let { app }: { app: GameApp } = $props();

  let now = $state(Date.now());

  $effect(() => {
    const timer = setInterval(() => {
      now = Date.now();
    }, 1000);
    return () => clearInterval(timer);
  });
</script>

<div class="who">
  <div class="head">
    <strong>Who</strong>
    <span class="count">{app.who.count}</span>
    <button
      type="button"
      class="refresh"
      title="Refresh"
      aria-label="refresh who list"
      onclick={() => app.refreshWho()}>⟳</button
    >
  </div>
  <div class="list">
    {#if app.who.count === 0}
      <p class="muted">Nobody yet.</p>
    {:else}
      <table>
        <thead>
          <tr><th>Name</th><th>Idle</th><th>On</th></tr>
        </thead>
        <tbody>
          {#each app.who.entries as e (e.name)}
            <tr>
              <td class="name" title={e.doing ?? ""}>
                {e.name}{#if e.alias}<span class="muted"> ({e.alias})</span>{/if}
              </td>
              <td class="num">{fmtDuration(app.who.idleOf(e, now))}</td>
              <td class="num">{fmtDuration(app.who.connOf(e, now))}</td>
            </tr>
            {#if e.doing}
              <tr class="doing"><td colspan="3">{e.doing}</td></tr>
            {/if}
          {/each}
        </tbody>
      </table>
    {/if}
  </div>
  <!-- Mobile-only: Game.svelte's header hides the "characters"/"sign out"
       text buttons below 800px for space, so this drawer (reachable via the
       always-visible ☰ button) is the phone-sized path to both — otherwise
       a phone user could never sign out (the token in localStorage on a
       shared/borrowed phone would stay live indefinitely). -->
  <div class="foot">
    <button type="button" class="footbtn" onclick={() => app.toCharSelect()}>
      Characters
    </button>
    <button type="button" class="footbtn" onclick={() => void app.logout()}>Sign out</button>
  </div>
</div>

<style>
  .who {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }

  .head {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.5rem 0.75rem;
    border-bottom: 1px solid var(--border);
  }

  .count {
    background: var(--bg-input);
    border-radius: 999px;
    padding: 0 0.5rem;
    font-size: 0.75rem;
    color: var(--fg-muted);
  }

  .refresh {
    margin-left: auto;
    background: none;
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--fg-muted);
    cursor: pointer;
    font: inherit;
    padding: 0.05rem 0.4rem;
  }

  .refresh:hover {
    color: var(--fg);
  }

  .list {
    flex: 1;
    overflow-y: auto;
    padding: 0.4rem 0.75rem;
  }

  .muted {
    color: var(--fg-muted);
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.82rem;
  }

  th {
    text-align: left;
    color: var(--fg-muted);
    font-weight: 500;
    padding: 0.15rem 0.3rem 0.15rem 0;
    border-bottom: 1px solid var(--border);
  }

  td {
    padding: 0.18rem 0.3rem 0.18rem 0;
    vertical-align: baseline;
  }

  .name {
    overflow-wrap: anywhere;
  }

  .num {
    font-family: var(--font-mono);
    font-size: 0.78rem;
    color: var(--fg-muted);
    white-space: nowrap;
  }

  .doing td {
    color: var(--fg-faint);
    font-size: 0.75rem;
    font-style: italic;
    padding-top: 0;
  }

  .foot {
    display: none;
    flex: none;
    gap: 0.5rem;
    padding: 0.5rem 0.75rem;
    border-top: 1px solid var(--border);
  }

  .footbtn {
    flex: 1;
    background: none;
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--fg-muted);
    cursor: pointer;
    font: inherit;
    font-size: 0.82rem;
    padding: 0.35rem 0.5rem;
  }

  .footbtn:hover {
    color: var(--fg);
  }

  /* Matches Game.svelte's header breakpoint, where the text buttons hide. */
  @media (max-width: 800px) {
    .foot {
      display: flex;
    }
  }
</style>
