<script lang="ts">
  // The scene screen (interface plan I4): the room band, the scrollback,
  // the vitals strip (or the textual prompt while no frame exists), and
  // the composer with the palette above it. The composer registers itself
  // with the app so the shell's keys can focus and prefill it.
  import type { GameApp } from "$lib/game.svelte";
  import { plainText } from "$lib/render";

  import CommandPalette from "./CommandPalette.svelte";
  import Composer from "./Composer.svelte";
  import RoomBand from "./RoomBand.svelte";
  import Scrollback from "./Scrollback.svelte";
  import VitalsStrip from "./VitalsStrip.svelte";

  let { app, onCmd }: { app: GameApp; onCmd: (cmd: string) => void } = $props();

  let composer: Composer | undefined = $state();

  $effect(() => {
    app.composer = composer ?? null;
    return () => {
      app.composer = null;
    };
  });

  const promptText = $derived(
    app.session.prompt?.text ? plainText(app.session.prompt.text) : "",
  );
</script>

<div class="scene">
  <RoomBand {app} {onCmd} />
  <Scrollback sb={app.scrollback} {onCmd} onLoadOlder={() => app.loadOlderNarrative()} />
  {#if app.vitals.loaded}
    <VitalsStrip {app} />
  {:else if promptText}
    <div class="prompt">{promptText}</div>
  {/if}
  <div class="composer-wrap">
    <CommandPalette {app} />
    <Composer bind:this={composer} {app} />
  </div>
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

  .composer-wrap {
    position: relative;
  }
</style>
