<script lang="ts">
  // The room band (interface plan I4): name and id, zone, weather icon and
  // state, IC clock and season, the scene pill, the exits as buttons with
  // their nav keys (D-I-19), and the presence pills (NPCs thinking, people
  // typing) — all from `RoomView` plus `ambient`, read defensively.
  import type { GameApp } from "$lib/game.svelte";
  import { clockText, roomBand } from "$lib/roomband";

  let { app, onCmd }: { app: GameApp; onCmd: (cmd: string) => void } = $props();

  const band = $derived(roomBand(app.roomView, app.screens.exits));

  const WEATHER_ICONS: Record<string, string> = {
    clear: "☀",
    sunny: "☀",
    cloudy: "☁",
    overcast: "☁",
    rain: "🌧",
    drizzle: "🌦",
    storm: "⛈",
    thunder: "⛈",
    snow: "❄",
    fog: "🌫",
    mist: "🌫",
    wind: "🌬",
  };

  function weatherIcon(icon: string): string {
    return WEATHER_ICONS[icon.toLowerCase()] ?? "";
  }

  let now = $state(Date.now());
  $effect(() => {
    const timer = setInterval(() => {
      now = Date.now();
      app.presence.prune(now);
    }, 1000);
    return () => clearInterval(timer);
  });
</script>

<div class="band">
  {#if !band}
    <span class="muted">No room yet — <code>look</code> when you are in.</span>
  {:else}
    <span class="name">{band.name}</span>
    <span class="dbref">{band.id}</span>
    {#if band.zone}<span class="sep">·</span><span class="zone">{band.zone}</span>{/if}
    {#if band.weather}
      <span class="sep">·</span>
      <span class="weather" title={band.weather.line ?? band.weather.state}>
        {#if weatherIcon(band.weather.icon)}<span aria-hidden="true">{weatherIcon(band.weather.icon)}</span>{/if}
        {band.weather.state}{#if band.weather.temp !== null}<span class="muted"> {band.weather.temp}°</span>{/if}
      </span>
    {/if}
    {#if band.clock}
      <span class="sep">·</span>
      <span class="clock" title="in-character time">
        {clockText(band.clock)}{#if band.clock.season}<span class="muted"> {band.clock.season}</span>{/if}
      </span>
    {/if}
    {#if band.scene}
      <span class="pill scene" title={band.scene.status}>
        {band.scene.title}{#if band.scene.round !== null}<span class="muted"> r{band.scene.round}{#if band.scene.turn !== null}·t{band.scene.turn}{/if}</span>{/if}
      </span>
    {/if}
    <span class="grow"></span>
    {#each app.presence.thinking as t (t.name)}
      <span class="pill thinking" title="the NPC is thinking">{t.name}…</span>
    {/each}
    {#each app.presence.composing as c (c.name)}
      <span class="pill typing">{c.name} is typing</span>
    {/each}
    {#if band.exits.length > 0}
      <span class="exits" aria-label="exits">
        {#each band.exits as exit (exit.name)}
          <button type="button" class="exit" onclick={() => onCmd(exit.name)} title={exit.key ? `${exit.name} (key ${exit.key})` : exit.name}>
            {#if exit.key}<kbd>{exit.key}</kbd>{/if}{exit.name}
          </button>
        {/each}
      </span>
    {/if}
  {/if}
</div>

<style>
  .band {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.35rem;
    padding: 0.35rem 0.75rem;
    border-bottom: 1px solid var(--border);
    background: var(--bg-panel);
    font-size: 0.85rem;
    min-height: 2rem;
  }

  .name {
    color: var(--c-room);
    font-weight: 700;
  }

  .dbref {
    color: var(--fg-faint);
    font-size: 0.75rem;
  }

  .sep {
    color: var(--fg-faint);
  }

  .zone,
  .clock,
  .weather {
    color: var(--fg-muted);
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

  .grow {
    flex: 1;
  }

  .pill {
    border-radius: 999px;
    padding: 0 0.55rem;
    font-size: 0.75rem;
    border: 1px solid var(--border);
    color: var(--fg-muted);
  }

  .pill.scene {
    border-color: var(--accent-dim);
    color: var(--accent);
  }

  .pill.thinking {
    border-color: var(--c-npc);
    color: var(--c-npc);
  }

  .pill.typing {
    border-color: var(--c-player);
    color: var(--c-player);
  }

  .exits {
    display: inline-flex;
    gap: 0.3rem;
    flex-wrap: wrap;
  }

  .exit {
    background: none;
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--c-exit);
    cursor: pointer;
    font: inherit;
    font-size: 0.8rem;
    padding: 0.1rem 0.5rem;
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
  }

  .exit:hover {
    border-color: var(--c-exit);
  }

  kbd {
    font-family: var(--font-mono);
    font-size: 0.7rem;
    background: var(--bg-input);
    border-radius: 3px;
    padding: 0 0.25rem;
    color: var(--fg-muted);
  }
</style>
