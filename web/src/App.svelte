<script lang="ts">
  // Screen router: login → character select → game. All state lives in the
  // one GameApp controller ($lib/game.svelte.ts).
  import CharSelect from "./components/CharSelect.svelte";
  import Game from "./components/Game.svelte";
  import Login from "./components/Login.svelte";
  import { GameApp } from "$lib/game.svelte";

  const app = new GameApp();
</script>

{#if app.screen === "login"}
  <Login {app} />
{:else if app.screen === "chars"}
  <CharSelect {app} />
{:else}
  <Game {app} />
{/if}
