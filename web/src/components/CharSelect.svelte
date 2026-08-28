<script lang="ts">
  // Character select: the account's characters (from the login payload) as
  // a grid; picking one opens the WebSocket session as that character.
  import type { GameApp } from "$lib/game.svelte";

  let { app }: { app: GameApp } = $props();
</script>

<div class="screen">
  <div class="card">
    <h1>Choose a character</h1>
    <p class="sub">
      {app.account ?? "?"} — {app.characters.length}
      character{app.characters.length === 1 ? "" : "s"}
    </p>

    {#if app.characters.length === 0}
      <p class="muted">
        No characters on this account yet. Create one over SSH (<code>ssh new@…</code>)
        or register a fresh account.
      </p>
    {:else}
      <div class="grid">
        {#each app.characters as c (c.key)}
          <button type="button" class="char" onclick={() => app.enterGame(c.key)}>
            <span class="avatar" aria-hidden="true">{c.name.slice(0, 1).toUpperCase()}</span>
            <span class="name">{c.name}</span>
            {#if app.lastCharKey === c.key}<span class="last">last played</span>{/if}
          </button>
        {/each}
      </div>
    {/if}

    <button type="button" class="ghost" onclick={() => void app.logout()}>Sign out</button>
  </div>
</div>

<style>
  .screen {
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 1rem;
  }

  .card {
    width: min(30rem, 100%);
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 1.5rem 1.6rem;
    display: flex;
    flex-direction: column;
    gap: 0.9rem;
  }

  h1 {
    margin: 0;
    font-size: 1.2rem;
  }

  .sub {
    margin: -0.5rem 0 0;
    color: var(--fg-muted);
    font-size: 0.85rem;
  }

  .muted {
    color: var(--fg-muted);
    font-size: 0.88rem;
  }

  code {
    font-family: var(--font-mono);
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(8.5rem, 1fr));
    gap: 0.6rem;
  }

  .char {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.45rem;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 0.9rem 0.5rem;
    cursor: pointer;
    font: inherit;
    color: var(--fg);
  }

  .char:hover {
    border-color: var(--accent);
  }

  .avatar {
    width: 2.6rem;
    height: 2.6rem;
    border-radius: 50%;
    background: var(--accent-dim);
    color: var(--fg);
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 1.2rem;
    font-weight: 700;
  }

  .name {
    font-weight: 600;
    overflow-wrap: anywhere;
  }

  .last {
    font-size: 0.68rem;
    color: var(--accent);
  }

  .ghost {
    align-self: flex-start;
    background: none;
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--fg-muted);
    padding: 0.3rem 0.8rem;
    cursor: pointer;
    font: inherit;
    font-size: 0.82rem;
  }

  .ghost:hover {
    color: var(--fg);
  }
</style>
