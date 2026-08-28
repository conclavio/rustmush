<script lang="ts">
  // Quick-compose modal: a single-shot `@mail/quick <to>=<subject>/<body>`.
  // Multi-line bodies belong to the terminal compose flow (`@mail/send`,
  // the input-context wizard) — this modal keeps the body single-shot.
  let {
    onsend,
    oncancel,
  }: {
    onsend: (to: string, subject: string, body: string) => void;
    oncancel: () => void;
  } = $props();

  let to = $state("");
  let subject = $state("");
  let body = $state("");

  const valid = $derived(
    to.trim().length > 0 && subject.trim().length > 0 && body.trim().length > 0,
  );

  function submit(ev: SubmitEvent): void {
    ev.preventDefault();
    if (valid) {
      onsend(to, subject, body);
    }
  }

  function onKeydown(ev: KeyboardEvent): void {
    if (ev.key === "Escape") {
      oncancel();
    }
  }
</script>

<svelte:window onkeydown={onKeydown} />

<div
  class="backdrop"
  role="presentation"
  onclick={(ev) => {
    if (ev.target === ev.currentTarget) {
      oncancel();
    }
  }}
>
  <form class="modal" aria-label="compose mail" onsubmit={submit}>
    <h2>Quick mail</h2>
    <label>
      To
      <input bind:value={to} placeholder="player, player2" />
    </label>
    <label>
      Subject
      <input bind:value={subject} placeholder="subject" />
    </label>
    <label>
      Message
      <textarea bind:value={body} rows="5" placeholder="one-shot message body"></textarea>
    </label>
    <p class="hint">
      Sends as <code>@mail/quick</code>. For a longer letter use
      <code>@mail/send</code> in the terminal (multi-line compose).
    </p>
    <div class="row">
      <button type="button" class="ghost" onclick={oncancel}>Cancel</button>
      <button type="submit" class="primary" disabled={!valid}>Send</button>
    </div>
  </form>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgb(0 0 0 / 55%);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 40;
  }

  .modal {
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 1.1rem 1.25rem;
    width: min(26rem, calc(100vw - 2rem));
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
  }

  h2 {
    margin: 0;
    font-size: 1rem;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    font-size: 0.82rem;
    color: var(--fg-muted);
  }

  input,
  textarea {
    background: var(--bg-input);
    color: var(--fg);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 0.4rem 0.55rem;
    font: inherit;
    font-size: 0.9rem;
    resize: vertical;
  }

  input:focus,
  textarea:focus {
    outline: none;
    border-color: var(--accent-dim);
  }

  .hint {
    margin: 0;
    font-size: 0.75rem;
    color: var(--fg-faint);
  }

  code {
    font-family: var(--font-mono);
  }

  .row {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
  }

  button {
    border-radius: 6px;
    padding: 0.35rem 0.9rem;
    font: inherit;
    font-size: 0.88rem;
    cursor: pointer;
    border: 1px solid var(--border);
  }

  .ghost {
    background: none;
    color: var(--fg-muted);
  }

  .primary {
    background: var(--accent-dim);
    color: var(--fg);
    border-color: transparent;
  }

  .primary:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
