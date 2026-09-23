<script lang="ts">
  // The game shell (interface plan I4, D-I-7, D-I-17): a header with the
  // tab strip (the same digits as the TUI), the active screen, the WHO
  // rail as a global toggle, and the legend — the screen's own keys as
  // chips, which are the whole story on a phone where focus is the only
  // mode. Keys are decided by `$lib/keys` from one window listener: bare
  // keys act while nothing has focus; inside the composer only Alt+key and
  // Ctrl/⌘+K are the shell's. The composer and its palette are the shell's
  // too (16b cut B.1, Q5): they sit under every screen but the channel
  // screen, so a prefill never has to leave the screen it was typed on.
  import type { GameApp } from "$lib/game.svelte";
  import { isInteractiveControl, isTextField, navIntent } from "$lib/keys";
  import { storageGet, storageSet } from "$lib/storage";
  import { systemPrefersLight, toggledTheme } from "$lib/theme";

  import ActionsPanel from "./ActionsPanel.svelte";
  import ChannelTabs from "./ChannelTabs.svelte";
  import ChannelView from "./ChannelView.svelte";
  import CockpitScreen from "./CockpitScreen.svelte";
  import CommandPalette from "./CommandPalette.svelte";
  import Composer from "./Composer.svelte";
  import GenericViewScreen from "./GenericViewScreen.svelte";
  import NotificationTray from "./NotificationTray.svelte";
  import RequestModal from "./RequestModal.svelte";
  import LlmPanel from "./LlmPanel.svelte";
  import MailPanel from "./MailPanel.svelte";
  import RollsPanel from "./RollsPanel.svelte";
  import SceneScreen from "./SceneScreen.svelte";
  import SheetPanel from "./SheetPanel.svelte";
  import UnitScreen from "./UnitScreen.svelte";
  import WhoRail from "./WhoRail.svelte";
  import WorldgenPanel from "./WorldgenPanel.svelte";

  let { app }: { app: GameApp } = $props();

  // The one command box registers itself with the app so the shell's keys
  // can focus and prefill it (16b cut B.1, moved here from the scene
  // screen). On the channel screen it is not mounted, `bind:this` yields
  // `undefined` and the app holds `null`: the channel's own box keeps the
  // focus it took, and a prefill or `Enter` from that screen alone switches
  // to the scene screen, where this box is (`GameApp.focusComposer`).
  let composer: Composer | undefined = $state();

  $effect(() => {
    app.composer = composer ?? null;
    return () => {
      app.composer = null;
    };
  });

  let whoOpen = $state(storageGet("rustmush.rail.who") !== "0");

  function toggleWho(): void {
    whoOpen = !whoOpen;
    storageSet("rustmush.rail.who", whoOpen ? "1" : "0");
  }

  /** Click-to-run: submit like typed input (with echo). */
  function runCmd(cmd: string): void {
    app.submit(cmd);
  }

  const gate = $derived(app.gate);
  const screens = $derived(app.screens.available(gate));
  const active = $derived(app.screens.activeDef(gate));

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

  function badge(id: string): number {
    switch (id) {
      case "mail":
        return app.mail.unread;
      case "channels":
        return app.channels.totalUnread;
      case "actions":
        return app.actions.actions.length;
      case "rolls":
        return app.rolls.entries.length;
      case "build":
        return app.worldgen.draft !== null ? 1 : 0;
      case "unit":
        // Count only what the panel can actually open: until the server has
        // answered, the screen is a loading line and no rows (the ring does
        // render under the no-subject line, so that case still counts).
        return app.unit.answered ? app.unit.journal.length : 0;
      case "cockpit":
        // What the pilot's own platform holds — a count the payload carries
        // and nothing else knows (16b cut Q.5).
        return app.cockpit.frame?.contacts.length ?? 0;
      default:
        return 0;
    }
  }

  function onKeydown(ev: KeyboardEvent): void {
    if (ev.defaultPrevented || ev.isComposing || app.requests.current || app.screens.helpOpen || app.disconnectReason !== null) return;
    // Only explicit shell modifiers may leave a focused control. Bare
    // Enter, Space, Tab, arrows and typing belong to that control.
    if (!ev.altKey && !ev.ctrlKey && !ev.metaKey && isInteractiveControl(ev.target)) return;
    const intent = navIntent(
      {
        key: ev.key,
        alt: ev.altKey,
        ctrl: ev.ctrlKey,
        meta: ev.metaKey,
        shift: ev.shiftKey,
        inField: isTextField(ev.target),
      },
      app.screens.exitKeysFor(gate),
      app.screens.bindingKeys(gate),
    );
    if (intent && app.act(intent)) {
      ev.preventDefault();
    }
  }

  const channelName = $derived(app.channels.active ?? app.channels.list[0]?.name ?? null);
</script>

<svelte:window onkeydown={onKeydown} />

<div class="game">
  <header>
    <button type="button" class="railbtn" title="Toggle who rail" onclick={toggleWho}>☰</button>
    <h1>rustmush</h1>
    <nav class="tabs" aria-label="screens">
      {#each screens as s (s.id)}
        {@const n = badge(s.id)}
        <button
          type="button"
          class="tab"
          class:active={active.id === s.id}
          aria-current={active.id === s.id ? "page" : undefined}
          title="{s.label} (key {s.digit})"
          onclick={() => app.screens.switchTo(s.id, gate)}
        >
          <kbd>{s.digit}</kbd>
          {s.label}
          {#if n > 0}<span class="badge">{n > 99 ? "99+" : n}</span>{/if}
        </button>
      {/each}
    </nav>
    <span class="grow"></span>
    {#if app.session.character}
      <span class="charname">{app.session.character.name}</span>
    {/if}
    {#if app.session.connections > 1}
      <span class="muted">×{app.session.connections}</span>
    {/if}
    <span class="status" data-status={app.session.status} title={app.session.serverName}>
      <span class="dot" aria-hidden="true"></span>
      {statusLabel}
    </span>
    <NotificationTray {app} />
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
  </header>

  <div class="cols">
    <aside class="who" class:open={whoOpen}>
      <WhoRail {app} />
    </aside>

    <section class="center">
      <div class="screen">
        {#if active.id === "scene"}
          <SceneScreen {app} onCmd={runCmd} />
        {:else if active.id === "channels"}
          <div class="chan">
            <ChannelTabs {app} />
            {#if channelName !== null}
              <ChannelView {app} channel={channelName} onCmd={runCmd} />
            {:else}
              <p class="muted pad">No channels joined. Every channel line also appears on the scene screen.</p>
            {/if}
          </div>
        {:else if active.id === "mail"}
          <MailPanel {app} />
        {:else if active.id === "sheet"}
          <SheetPanel {app} />
        {:else if active.id === "rolls"}
          <RollsPanel {app} />
        {:else if active.id === "actions"}
          <ActionsPanel {app} onCmd={runCmd} />
        {:else if active.id === "build"}
          <WorldgenPanel {app} />
        {:else if active.id === "llm"}
          <LlmPanel {app} />
        {:else if active.id === "unit"}
          <UnitScreen {app} />
        {:else if active.id === "cockpit"}
          <CockpitScreen {app} />
        {:else if active.viewFeed}
          <GenericViewScreen {app} feed={active.viewFeed} />
        {:else}
          <p class="muted pad">{active.label}: this screen has no view yet.</p>
        {/if}
      </div>

      <!-- The shell's command box (16b cut B.1, Q5): under every screen, so
           the draft, the sticky mode and the history survive a screen switch
           and a prefill leaves the pilot where they are. NOT on the channel
           screen: `ChannelView` mounts its own box, addressed to the channel;
           a second `command input` there would give two boxes one accessible
           name and make Enter send a channel line as a game command. -->
      {#if active.id !== "channels"}
        <div class="composer-wrap">
          <CommandPalette {app} />
          <Composer bind:this={composer} {app} />
        </div>
      {/if}

      <footer class="legend" aria-label="keys">
        {#if app.screens.inputFocused}
          <span class="muted">Enter sends · Esc leaves the box, keeping the draft · Alt+key is the nav key · ^K palette</span>
        {:else}
          {#each active.keys as b (b.key)}
            <button type="button" class="chip" onclick={() => app.runBinding(b.key)}>
              <kbd>{b.key}</kbd>
              {b.label}
            </button>
          {/each}
          {#if active.exitsAreKeys}
            {#each [...app.screens.exits] as [k, name] (k)}
              <button type="button" class="chip exitchip" onclick={() => runCmd(name)}>
                <kbd>{k}</kbd>
                {name}
              </button>
            {/each}
          {/if}
          <button type="button" class="chip" onclick={() => app.focusComposer()}>
            <kbd>Enter</kbd> type
          </button>
          <button type="button" class="chip" onclick={() => (app.screens.helpOpen = !app.screens.helpOpen)}>
            <kbd>?</kbd> help
          </button>
        {/if}
      </footer>
    </section>
  </div>

  <RequestModal {app} />

  {#if app.screens.helpOpen}
    <div class="backdrop" role="presentation" onclick={() => (app.screens.helpOpen = false)}>
      <div
        class="modal help"
        role="dialog"
        aria-label="keys"
        tabindex="-1"
        onclick={(ev) => ev.stopPropagation()}
        onkeydown={(ev) => {
          if (ev.key === "Escape") {
            app.screens.helpOpen = false;
          }
        }}
      >
        <h2>{active.label} screen</h2>
        <dl>
          <dt>0–9</dt><dd>switch screens (Alt+digit from the box); 0 is the cockpit</dd>
          <dt>Enter</dt><dd>open the box; sends and stays; empty Enter closes it</dd>
          <dt>" : ; \</dt><dd>open the box with that speech trigger typed</dd>
          <dt>/</dt><dd>the command palette (^K from anywhere)</dd>
          <dt>Esc</dt><dd>leave the box, keeping the draft</dd>
          <dt>Alt+key</dt><dd>the key's nav action from inside the box</dd>
          {#each active.keys as b (b.key)}
            <dt>{b.key}</dt><dd>{b.label}</dd>
          {/each}
          {#if active.exitsAreKeys && app.screens.exits.size > 0}
            <dt>exits</dt>
            <dd>{[...app.screens.exits].map(([k, n]) => `${k} ${n}`).join(", ")}</dd>
          {/if}
        </dl>
        <div class="row">
          <button type="button" class="ghost" onclick={() => (app.screens.helpOpen = false)}>Close</button>
        </div>
      </div>
    </div>
  {/if}

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
    padding: 0.35rem 0.7rem;
    background: var(--bg-panel);
    border-bottom: 1px solid var(--border);
    flex: none;
    flex-wrap: wrap;
  }

  h1 {
    margin: 0;
    font-size: 1rem;
    letter-spacing: 0.04em;
  }

  .tabs {
    display: flex;
    gap: 0.2rem;
    flex-wrap: wrap;
  }

  .tab {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.2rem 0.6rem;
    border: 1px solid transparent;
    border-radius: 6px;
    background: none;
    color: var(--fg-muted);
    cursor: pointer;
    font: inherit;
    font-size: 0.85rem;
  }

  .tab.active {
    background: var(--bg);
    border-color: var(--border);
    color: var(--fg);
  }

  kbd {
    font-family: var(--font-mono);
    font-size: 0.7rem;
    background: var(--bg-input);
    border-radius: 3px;
    padding: 0 0.3rem;
    color: var(--fg-muted);
  }

  .tab.active kbd {
    color: var(--accent);
  }

  .badge {
    background: var(--accent);
    color: var(--bg);
    border-radius: 999px;
    font-size: 0.68rem;
    font-weight: 700;
    padding: 0 0.35rem;
    min-width: 1.1rem;
    text-align: center;
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

  .pad {
    padding: 0.8rem;
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

  .who {
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

  .center {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  .screen {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  /* The channel screen (I7): the IC/OOC list beside the channel view; a
   * phone stacks them. */
  .chan {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: 12rem minmax(0, 1fr);
  }

  @media (max-width: 800px) {
    .chan {
      grid-template-columns: minmax(0, 1fr);
      grid-template-rows: auto minmax(0, 1fr);
    }
  }

  /* The palette opens above the box, positioned against this wrap. */
  .composer-wrap {
    position: relative;
    flex: none;
  }

  .legend {
    flex: none;
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem;
    align-items: center;
    padding: 0.3rem 0.7rem;
    border-top: 1px solid var(--border);
    background: var(--bg-panel);
    font-size: 0.78rem;
    min-height: 1.9rem;
  }

  .chip {
    background: none;
    border: 1px solid var(--border);
    border-radius: 999px;
    color: var(--fg-muted);
    cursor: pointer;
    font: inherit;
    font-size: 0.75rem;
    padding: 0.05rem 0.55rem;
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
  }

  .chip:hover {
    color: var(--fg);
  }

  .exitchip {
    color: var(--c-exit);
  }

  /* Mobile: the rail becomes an overlay drawer; the tab strip wraps. */
  @media (max-width: 800px) {
    .who.open {
      position: fixed;
      top: 2.6rem;
      bottom: 0;
      left: 0;
      z-index: 30;
      width: min(19rem, 85vw);
      box-shadow: 0 0 24px rgb(0 0 0 / 45%);
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

  .modal.help {
    width: min(32rem, calc(100vw - 2rem));
    max-height: 80vh;
    overflow-y: auto;
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

  dl {
    margin: 0;
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 0.25rem 0.8rem;
    font-size: 0.85rem;
  }

  dt {
    font-family: var(--font-mono);
    color: var(--accent);
  }

  dd {
    margin: 0;
    color: var(--fg-muted);
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
