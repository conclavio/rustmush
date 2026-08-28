<script lang="ts">
  // Room panel: the latest `RoomView` event — name/desc/contents/exits with
  // click-to-run exit links.
  import type { GameApp } from "$lib/game.svelte";
  import { renderRich } from "$lib/render";

  import RichLine from "./RichLine.svelte";

  let { app, onCmd }: { app: GameApp; onCmd: (cmd: string) => void } = $props();
</script>

<div class="room">
  {#if !app.roomView}
    <p class="muted">No room yet — try <code>look</code>.</p>
  {:else}
    <h2>{app.roomView.name} <span class="dbref">{app.roomView.id}</span></h2>
    <div class="desc">
      <RichLine line={renderRich(app.roomView.desc)} {onCmd} />
    </div>
    {#if app.roomView.contents.length > 0}
      <h3>Contents</h3>
      <ul>
        {#each app.roomView.contents as obj (obj.id)}
          <li>
            <button type="button" class="linky" onclick={() => onCmd(`look ${obj.name}`)}>
              {obj.name}
            </button>
          </li>
        {/each}
      </ul>
    {/if}
    {#if app.roomView.exits.length > 0}
      <h3>Exits</h3>
      <div class="exits">
        {#each app.roomView.exits as exit (exit.name)}
          <button
            type="button"
            class="exit"
            title={exit.aliases.length > 0 ? `also: ${exit.aliases.join(", ")}` : exit.name}
            onclick={() => onCmd(exit.name)}
          >
            {exit.name}
          </button>
        {/each}
      </div>
    {/if}
    {#if app.roomView.zone}
      <p class="muted zone">Zone: {app.roomView.zone.name}</p>
    {/if}
  {/if}
</div>

<style>
  .room {
    padding: 0.6rem 0.85rem;
    overflow-y: auto;
    height: 100%;
    font-size: 0.88rem;
  }

  .muted {
    color: var(--fg-muted);
  }

  code {
    font-family: var(--font-mono);
    background: var(--bg-input);
    border-radius: 4px;
    padding: 0.05em 0.3em;
  }

  h2 {
    margin: 0 0 0.4rem;
    font-size: 0.98rem;
    color: var(--c-room);
  }

  .dbref {
    color: var(--fg-faint);
    font-size: 0.75rem;
    font-family: var(--font-mono);
  }

  h3 {
    margin: 0.8rem 0 0.25rem;
    font-size: 0.78rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--fg-muted);
  }

  .desc {
    font-family: var(--font-mono);
    font-size: 0.85rem;
  }

  ul {
    margin: 0;
    padding-left: 1.1rem;
  }

  .linky {
    background: none;
    border: none;
    padding: 0;
    color: var(--c-player);
    cursor: pointer;
    font: inherit;
    text-decoration: underline dotted;
    text-underline-offset: 2px;
  }

  .exits {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
  }

  .exit {
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--c-exit);
    padding: 0.25rem 0.65rem;
    cursor: pointer;
    font: inherit;
    font-size: 0.85rem;
  }

  .exit:hover {
    border-color: var(--c-exit);
  }

  .zone {
    margin-top: 0.9rem;
    font-size: 0.75rem;
  }
</style>
