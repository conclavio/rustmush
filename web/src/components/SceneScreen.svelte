<script lang="ts">
  // The scene screen (interface plan I4): the room band, the scrollback,
  // and the vitals strip (or the textual prompt while no frame exists). The
  // composer and its palette are the shell's, under every screen but the
  // channel screen (16b cut B.1; `Game.svelte`), so this column ends at the
  // vitals strip or the prompt line.
  import type { GameApp } from "$lib/game.svelte";
  import { plainText } from "$lib/render";

  import RoomBand from "./RoomBand.svelte";
  import MapCanvas from "./MapCanvas.svelte";
  import Scrollback from "./Scrollback.svelte";
  import VitalsStrip from "./VitalsStrip.svelte";

  let { app, onCmd }: { app: GameApp; onCmd: (cmd: string) => void } = $props();

  const promptText = $derived(
    app.session.prompt?.text ? plainText(app.session.prompt.text) : "",
  );
</script>

<div class="scene">
  <RoomBand {app} {onCmd} />
  <Scrollback sb={app.scrollback} {onCmd} onLoadOlder={() => app.loadOlderNarrative()} />
  {#if app.map.loaded}
    <MapCanvas {app} />
  {/if}
  {#if app.vitals.loaded}
    <VitalsStrip {app} />
  {:else if promptText}
    <div class="prompt">{promptText}</div>
  {/if}
</div>

<style>
  .scene {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  .prompt {
    padding: 0.15rem 0.75rem;
    font-family: var(--font-mono);
    font-size: 0.82rem;
    color: var(--accent);
    border-top: 1px solid var(--border);
    background: var(--bg-panel);
  }
</style>
