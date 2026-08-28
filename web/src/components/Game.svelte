<script lang="ts">
  // The game screen (design 02-connectivity §7.3): collapsible WHO rail
  // left, sacred center column (channel tabs + scrollback + input), tabbed
  // panel right. On mobile the center stands alone and the rails become
  // overlay drawers. Rail visibility persists in localStorage.
  import type { GameApp } from "$lib/game.svelte";
  import { plainText } from "$lib/render";
  import { storageGet, storageSet } from "$lib/storage";
  import { systemPrefersLight, toggledTheme } from "$lib/theme";

  import ChannelTabs from "./ChannelTabs.svelte";
  import ChannelView from "./ChannelView.svelte";
  import InputBar from "./InputBar.svelte";
  import RightPanel from "./RightPanel.svelte";
  import Scrollback from "./Scrollback.svelte";
  import WhoRail from "./WhoRail.svelte";

  let { app }: { app: GameApp } = $props();

  let whoOpen = $state(storageGet("rustmush.rail.who") !== "0");
  let rightOpen = $state(storageGet("rustmush.rail.right") !== "0");

  function toggleWho(): void {
    whoOpen = !whoOpen;
    storageSet("rustmush.rail.who", whoOpen ? "1" : "0");
  }

  function toggleRight(): void {
    rightOpen = !rightOpen;
    storageSet("rustmush.rail.right", rightOpen ? "1" : "0");
  }

  /** Click-to-run: submit like typed input (with echo). */
  function runCmd(cmd: string): void {
    app.submit(cmd);
  }

  const statusLabel = $derived(
    (
      {
        idle: "idle",
        connecting: "connecting…",
        connected: "connected",
        reconnecting: "reconnecting…",
        closed: "offline",
      } as const
    )[app.session.status],
  );

  const promptText = $derived(
    app.session.prompt?.text ? plainText(app.session.prompt.text) : "",
  );

  // The main InputBar now stays mounted across channel-tab switches (see
  // the markup below), so its own mount-time `autofocus` only fires once,
  // ever. Refocus by hand whenever we come back to the main pane.
  let mainInput: InputBar | undefined = $state();
  $effect(() => {
    if (app.channels.active === null) {
      mainInput?.focus();
    }
  });
</script>

<div class="game">
  <header>
    <button type="button" class="railbtn" title="Toggle who rail" onclick={toggleWho}>☰</button>
    <h1>rustmush</h1>
    {#if app.session.character}
      <span class="charname">{app.session.character.name}</span>
    {/if}
    {#if app.session.connections > 1}
      <span class="muted">×{app.session.connections}</span>
    {/if}
    <span class="grow"></span>
    <span class="status" data-status={app.session.status} title={app.session.serverName}>
      <span class="dot" aria-hidden="true"></span>
      {statusLabel}
    </span>
    <button
      type="button"
      class="railbtn"
      title="Toggle theme"
      aria-label="toggle theme"
      onclick={() => app.setTheme(toggledTheme(app.theme, systemPrefersLight()))}>◐</button
    >
    <button type="button" class="railbtn text" onclick={() => app.toCharSelect()}>
      characters
    </button>
    <button type="button" class="railbtn text" onclick={() => void app.logout()}>sign out</button>
    <button type="button" class="railbtn" title="Toggle panel" onclick={toggleRight}>⊞</button>
  </header>

  <div class="cols">
    <aside class="who" class:open={whoOpen}>
      <WhoRail {app} />
    </aside>

    <section class="center">
      <ChannelTabs {app} />
      <!-- Both panes stay mounted and toggle via CSS (not {#if}/{:else}):
           unmounting Scrollback+InputBar on every tab switch used to reset
           the input's draft/history (InputBar owns a per-instance
           HistoryBuffer) and the scrollback's DOM scroll position. -->
      <div class="pane" class:pane-hidden={app.channels.active === null}>
        <ChannelView {app} channel={app.channels.active ?? ""} onCmd={runCmd} />
      </div>
      <div class="pane" class:pane-hidden={app.channels.active !== null}>
        <Scrollback sb={app.scrollback} onCmd={runCmd} />
        {#if promptText}
          <div class="prompt">{promptText}</div>
        {/if}
        <InputBar
          bind:this={mainInput}
          autofocus
          masked={app.session.masked}
          placeholder={app.session.masked ? "(input hidden)" : "say hello, or /help"}
          onsubmit={(line) => app.submit(line)}
        />
      </div>
    </section>

    <aside class="right" class:open={rightOpen}>
      <RightPanel {app} onCmd={runCmd} />
    </aside>
  </div>

  {#if app.disconnectReason !== null}
    <div class="backdrop" role="presentation">
      <div class="modal" role="dialog" aria-label="disconnected">
        <h2>Disconnected</h2>
        <p>{app.disconnectReason}</p>
        <div class="row">
          <button type="button" class="ghost" onclick={() => app.toCharSelect()}>
            Character select
          </button>
          <button type="button" class="primary" onclick={() => app.reconnect()}>Reconnect</button>
        </div>
      </div>
    </div>
  {/if}

  <div class="toasts" aria-live="polite">
    {#each app.toasts as t (t.id)}
      <div class="toast">{t.text}</div>
    {/each}
  </div>
</div>

<style>
  .game {
    height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  header {
    display: flex;
    align-items: center;
    gap: 0.55rem;
    padding: 0.4rem 0.7rem;
    background: var(--bg-panel);
    border-bottom: 1px solid var(--border);
    flex: none;
  }

  h1 {
    margin: 0;
    font-size: 1rem;
    letter-spacing: 0.04em;
  }

  .charname {
    color: var(--c-player);
    font-weight: 600;
    font-size: 0.9rem;
  }

  .muted {
    color: var(--fg-muted);
    font-size: 0.8rem;
  }

  .grow {
    flex: 1;
  }

  .status {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    font-size: 0.78rem;
    color: var(--fg-muted);
  }

  .dot {
    width: 0.55rem;
    height: 0.55rem;
    border-radius: 50%;
    background: var(--fg-faint);
  }

  .status[data-status="connected"] .dot {
    background: var(--success);
  }

  .status[data-status="connecting"] .dot,
  .status[data-status="reconnecting"] .dot {
    background: var(--warn);
    animation: pulse 1.2s infinite;
  }

  .status[data-status="closed"] .dot {
    background: var(--error);
  }

  @keyframes pulse {
    50% {
      opacity: 0.35;
    }
  }

  .railbtn {
    background: none;
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--fg-muted);
    cursor: pointer;
    font: inherit;
    font-size: 0.85rem;
    padding: 0.15rem 0.5rem;
  }

  .railbtn:hover {
    color: var(--fg);
  }

  .cols {
    flex: 1;
    display: flex;
    min-height: 0;
  }

  .who,
  .right {
    flex: none;
    background: var(--bg-panel);
    min-height: 0;
    display: none;
    overflow: hidden;
  }

  .who.open {
    display: block;
    width: 14rem;
    border-right: 1px solid var(--border);
  }

  .right.open {
    display: block;
    width: 19rem;
    border-left: 1px solid var(--border);
  }

  .center {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  /* The channel-tab pane and the main scrollback+input pane both stay
     mounted; only visibility toggles (see the markup comment above). */
  .pane {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  .pane-hidden {
    display: none;
  }

  .prompt {
    padding: 0.15rem 0.75rem;
    font-family: var(--font-mono);
    font-size: 0.82rem;
    color: var(--accent);
    border-top: 1px solid var(--border);
    background: var(--bg-panel);
  }

  /* Mobile: center only; rails become overlay drawers. */
  @media (max-width: 800px) {
    .who.open,
    .right.open {
      position: fixed;
      top: 2.6rem;
      bottom: 0;
      z-index: 30;
      width: min(19rem, 85vw);
      box-shadow: 0 0 24px rgb(0 0 0 / 45%);
    }

    .who.open {
      left: 0;
    }

    .right.open {
      right: 0;
    }

    .railbtn.text {
      display: none;
    }
  }

  .backdrop {
    position: fixed;
    inset: 0;
    background: rgb(0 0 0 / 55%);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 50;
  }

  .modal {
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 1.2rem 1.4rem;
    width: min(22rem, calc(100vw - 2rem));
    display: flex;
    flex-direction: column;
    gap: 0.7rem;
  }

  .modal h2 {
    margin: 0;
    font-size: 1rem;
  }

  .modal p {
    margin: 0;
    color: var(--fg-muted);
    font-size: 0.9rem;
  }

  .row {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
  }

  .primary {
    background: var(--accent-dim);
    color: var(--fg);
    border: none;
    border-radius: 6px;
    padding: 0.4rem 0.9rem;
    cursor: pointer;
    font: inherit;
  }

  .ghost {
    background: none;
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--fg-muted);
    padding: 0.4rem 0.9rem;
    cursor: pointer;
    font: inherit;
  }

  .toasts {
    position: fixed;
    bottom: 1rem;
    right: 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    z-index: 45;
  }

  .toast {
    background: var(--bg-panel);
    border: 1px solid var(--accent-dim);
    border-left: 3px solid var(--accent);
    border-radius: 8px;
    padding: 0.5rem 0.8rem;
    font-size: 0.85rem;
    max-width: 20rem;
    box-shadow: 0 4px 16px rgb(0 0 0 / 35%);
  }
</style>
