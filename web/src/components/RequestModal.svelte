<script lang="ts">
  // The server-initiated request modal (interface plan I10, D-I-21): renders
  // the oldest pending Request and reports the viewer's choice by id. The
  // server owns the effect; this only sends a RespondChoice. On desktop it is
  // a centred dialog, on a phone a bottom sheet (the CSS). A line-only or
  // disconnected client never sees it and the server applies its default.
  import type { GameApp } from "$lib/game.svelte";
  import { renderRich } from "$lib/render";
  import type { RespondChoice } from "$lib/proto";

  import RichLine from "./RichLine.svelte";

  let { app }: { app: GameApp } = $props();

  const req = $derived(app.requests.current);

  let text = $state("");

  // A fresh request clears any typed answer.
  $effect(() => {
    void req?.id;
    text = "";
  });

  // The request expires on its own deadline; the server applies its default,
  // so the client just drops it (I10). Scoped to while the modal is mounted.
  $effect(() => {
    const r = req;
    if (!r || r.deadlineMs === null) {
      return;
    }
    const timer = setTimeout(() => app.requests.expireDue(), Math.max(0, r.deadlineMs - Date.now()));
    return () => clearTimeout(timer);
  });

  function answer(choice: RespondChoice): void {
    if (req) {
      app.respond(req.id, choice);
    }
  }
</script>

{#if req}
  <div class="backdrop" role="presentation">
    <div
      class="sheet"
      role="dialog"
      aria-modal="true"
      aria-label="the game is asking you something"
    >
      <div class="prompt">
        <RichLine line={renderRich(req.prompt)} onCmd={() => {}} />
      </div>

      {#if req.kind === "confirm"}
        <div class="actions">
          <button type="button" class="primary" onclick={() => answer({ t: "confirm", d: true })}>
            Yes
          </button>
          <button type="button" onclick={() => answer({ t: "confirm", d: false })}>No</button>
        </div>
      {:else if req.kind === "choice"}
        <div class="actions wrap">
          {#each req.options as o (o.id)}
            <button type="button" onclick={() => answer({ t: "choose", d: o.id })}>{o.label}</button>
          {/each}
        </div>
      {:else}
        <form
          class="textform"
          onsubmit={(e) => {
            e.preventDefault();
            if (text.trim().length > 0) {
              answer({ t: "text", d: text.trim() });
            }
          }}
        >
          <input
            type="text"
            bind:value={text}
            placeholder={req.kind === "target" ? "a name or #dbref" : "your answer"}
          />
          <button type="submit" class="primary" disabled={text.trim().length === 0}>Send</button>
        </form>
      {/if}

      <button type="button" class="cancel" onclick={() => answer({ t: "cancel" })}>Cancel</button>
    </div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgb(0 0 0 / 45%);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 50;
  }

  .sheet {
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 1rem 1.1rem;
    width: min(28rem, calc(100vw - 2rem));
    box-shadow: 0 12px 40px rgb(0 0 0 / 45%);
    display: flex;
    flex-direction: column;
    gap: 0.8rem;
  }

  .prompt {
    font-size: 1rem;
    line-height: 1.4;
  }

  .actions {
    display: flex;
    gap: 0.5rem;
  }

  .actions.wrap {
    flex-wrap: wrap;
  }

  .textform {
    display: flex;
    gap: 0.5rem;
  }

  input {
    flex: 1;
    font: inherit;
    padding: 0.35rem 0.5rem;
    background: var(--bg-input);
    color: var(--fg);
    border: 1px solid var(--border);
    border-radius: 6px;
  }

  input:focus {
    border-color: var(--accent);
  }

  button {
    font: inherit;
    padding: 0.35rem 0.9rem;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--bg-input);
    color: var(--fg);
    cursor: pointer;
  }

  button:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .primary {
    border-color: var(--accent);
    background: var(--accent);
    color: var(--bg);
    font-weight: 600;
  }

  .cancel {
    align-self: flex-end;
    background: none;
    border: none;
    color: var(--fg-muted);
    padding: 0.2rem 0.3rem;
  }

  .cancel:hover {
    color: var(--fg);
  }

  @media (max-width: 800px) {
    .backdrop {
      align-items: flex-end;
    }

    .sheet {
      width: 100vw;
      border-radius: 12px 12px 0 0;
      padding-bottom: 1.4rem;
    }
  }
</style>
