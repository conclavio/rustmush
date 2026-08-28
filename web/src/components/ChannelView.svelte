<script lang="ts">
  // One channel's tab: its own message list (backlog + live) and an input
  // whose lines are wrapped in the member's alias verb (`pub <text>`).
  import { tick } from "svelte";

  import type { GameApp } from "$lib/game.svelte";
  import { channelLine, fmtClock } from "$lib/render";

  import InputBar from "./InputBar.svelte";
  import RichLine from "./RichLine.svelte";

  let {
    app,
    channel,
    onCmd,
  }: { app: GameApp; channel: string; onCmd: (cmd: string) => void } = $props();

  const meta = $derived(app.channels.byName(channel));
  const messages = $derived(app.channels.messagesOf(channel));

  let viewport: HTMLDivElement | undefined = $state();
  let inputBar: InputBar | undefined = $state();

  $effect(() => {
    void messages.length; // track: keep the channel view pinned to bottom
    void tick().then(() => {
      if (viewport) {
        viewport.scrollTop = viewport.scrollHeight;
      }
    });
  });

  // Game.svelte keeps this component mounted across tab switches (so its
  // own input/history/scroll survive) instead of remounting it per tab, so
  // `InputBar`'s own mount-time `autofocus` only fires once, ever. Refocus
  // by hand whenever the visible channel changes (a no-op while this pane
  // is hidden — a `display:none` input can't take focus).
  $effect(() => {
    void channel;
    inputBar?.focus();
  });
</script>

<div class="chanview">
  <div class="head">
    <strong style={meta?.colorHex ? `color:${meta.colorHex}` : ""}>{meta?.name ?? channel}</strong>
    {#if meta?.title}<span class="muted">title: {meta.title}</span>{/if}
    {#if meta?.gagged}<span class="warn">gagged — you won't hear new messages</span>{/if}
    {#if meta && !meta.logged}<span class="muted">(no history: LOGGED is off)</span>{/if}
    {#if meta?.logged}
      <button type="button" class="linky" onclick={() => app.loadOlderChannel(channel)}>
        load older
      </button>
    {/if}
  </div>
  <div class="viewport" bind:this={viewport}>
    {#if messages.length === 0}
      <p class="muted empty">No messages yet.</p>
    {/if}
    {#each messages as msg (msg.id)}
      <div class="msg">
        <span class="ts">{fmtClock(msg.ts)}</span>
        <div class="body">
          <RichLine
            line={channelLine(
              {
                channel: meta?.name ?? channel,
                actor: msg.actor,
                title: msg.title,
                kind: msg.kind,
                text: msg.text,
              },
              meta?.colorHex,
            )}
            {onCmd}
          />
        </div>
      </div>
    {/each}
  </div>
  <InputBar
    bind:this={inputBar}
    autofocus
    placeholder={`Message ${meta?.name ?? channel} (sent as: ${meta?.alias ?? channel} …)`}
    onsubmit={(line) => app.channelSend(channel, line)}
  />
</div>

<style>
  .chanview {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  .head {
    display: flex;
    align-items: baseline;
    gap: 0.7rem;
    padding: 0.4rem 0.75rem;
    border-bottom: 1px solid var(--border);
    font-size: 0.85rem;
    flex-wrap: wrap;
  }

  .muted {
    color: var(--fg-muted);
  }

  .warn {
    color: var(--error);
  }

  .linky {
    margin-left: auto;
    background: none;
    border: none;
    color: var(--accent);
    cursor: pointer;
    font: inherit;
    font-size: 0.8rem;
    text-decoration: underline dotted;
  }

  .viewport {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 0.5rem 0.75rem;
    font-family: var(--font-mono);
    font-size: 0.92rem;
  }

  .empty {
    margin-top: 1rem;
    text-align: center;
  }

  .msg {
    display: flex;
    gap: 0.6rem;
    padding: 0.07rem 0;
  }

  .ts {
    flex: none;
    width: 2.9rem;
    color: var(--fg-faint);
    font-size: 0.75rem;
    line-height: 1.7;
    user-select: none;
  }

  .body {
    flex: 1;
    min-width: 0;
  }
</style>
