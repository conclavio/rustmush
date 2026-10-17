<script lang="ts">
  import { onDestroy } from "svelte";
  import type { GameApp } from "$lib/game.svelte";
  import MindCard from "./MindCard.svelte";
  let { app }: { app: GameApp } = $props();
  let dialog = $state<HTMLDialogElement>();
  let returnFocus: HTMLElement | null = null;
  const mind = $derived(app.cockpit.frame?.mind);
  $effect(() => {
    const d = dialog;
    if (!d) return;
    if (app.mindReadoutOpen && !d.open) {
      returnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      d.showModal();
    } else if (!app.mindReadoutOpen && d.open) { d.close(); if (returnFocus?.isConnected) returnFocus.focus(); returnFocus = null; }
  });
  onDestroy(() => { dialog?.close(); if (returnFocus?.isConnected) returnFocus.focus(); });
</script>

<dialog bind:this={dialog} aria-label="Sealed mind card and trace" oncancel={event => { event.preventDefault(); app.mindReadoutOpen = false; }}>
  <header><h2>{mind?.name ?? "Sealed mind"}</h2><button type="button" onclick={() => app.mindReadoutOpen = false}>Close</button></header>
  {#if mind}
    <p>Class {mind.class} · {mind.profiles[mind.active] ?? "no active profile"}. This is the sealed card; edits belong in a workshop working copy.</p>
    <div class="profiles">{#each mind.profiles as profile, i (i)}
      <button type="button" disabled={i === mind.active} onclick={() => { app.mindReadoutOpen = false; app.focusComposer(`MIND PROFILE ${profile}`); }}>{profile}{i === mind.active ? " (active)" : ""}</button>
    {/each}</div>
    {#if mind.card}<MindCard card={mind.card} stations={app.cockpit.frame?.weapons.length ? ["helm", "sensors", "fire"] : ["helm", "sensors"]} />{/if}
    <h3>Sealed rules</h3><ol>{#each mind.sealedRules as rule, i (i)}<li><span title="Current truth is unavailable; sampled truth appears in the trace">?</span> {rule}</li>{/each}</ol>
  {/if}
  <h3>Mind trace</h3>
  <button type="button" onclick={() => app.refreshMindTrace()}>Read trace</button>
  <button type="button" onclick={() => { app.mindReadoutOpen = false; app.focusComposer("MIND TRACE"); }}>Prepare follow command</button>
  {#if app.mindTrace?.available}
    <p>Trace for unit #{app.mindTrace.unit}</p>
    <pre aria-label="Mind trace">{app.mindTrace.lines.join("\n") || "No completed thinks yet."}</pre>
  {:else}<p>No mind trace is available.</p>{/if}
</dialog>

<style>
  dialog { width: min(850px, 94vw); max-height: 90vh; background: var(--bg, #181a1d); color: var(--fg, #ddd); border: 1px solid var(--border, #666); border-radius: .5rem; }
  dialog::backdrop { background: #000a; }
  header, .profiles { display: flex; gap: .6rem; justify-content: space-between; flex-wrap: wrap; }
  h2 { margin: 0; }
  button { overflow-wrap: anywhere; max-width: 100%; }
  pre, li { white-space: pre-wrap; overflow-wrap: anywhere; }
</style>
