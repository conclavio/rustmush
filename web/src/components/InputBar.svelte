<script lang="ts">
  // Command input: Enter submits, ↑/↓ browse history (per-instance buffer,
  // 200 entries), masked mode renders as a password field with no history.
  import { HistoryBuffer } from "$lib/history";

  let {
    onsubmit,
    masked = false,
    placeholder = "",
    autofocus = false,
  }: {
    onsubmit: (line: string) => void;
    masked?: boolean;
    placeholder?: string;
    autofocus?: boolean;
  } = $props();

  const history = new HistoryBuffer();
  let value = $state("");
  let input: HTMLInputElement | undefined = $state();

  export function focus(): void {
    input?.focus();
  }

  function onKeydown(ev: KeyboardEvent): void {
    if (ev.key === "Enter") {
      ev.preventDefault();
      const line = value;
      value = "";
      if (!masked && line.trim().length > 0) {
        history.push(line);
      } else {
        history.reset();
      }
      onsubmit(line);
      return;
    }
    if (masked) {
      return;
    }
    if (ev.key === "ArrowUp") {
      const prev = history.up(value);
      if (prev !== null) {
        ev.preventDefault();
        value = prev;
      }
    } else if (ev.key === "ArrowDown") {
      const next = history.down();
      if (next !== null) {
        ev.preventDefault();
        value = next;
      }
    }
  }

  $effect(() => {
    if (autofocus) {
      input?.focus();
    }
  });
</script>

<div class="bar">
  <span class="chev" aria-hidden="true">&gt;</span>
  <input
    bind:this={input}
    bind:value
    type={masked ? "password" : "text"}
    autocomplete="off"
    autocapitalize="off"
    spellcheck="false"
    {placeholder}
    aria-label="command input"
    onkeydown={onKeydown}
  />
</div>

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.45rem 0.75rem;
    border-top: 1px solid var(--border);
    background: var(--bg-panel);
  }

  .chev {
    color: var(--accent);
    font-family: var(--font-mono);
    user-select: none;
  }

  input {
    flex: 1;
    background: var(--bg-input);
    color: var(--fg);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 0.4rem 0.6rem;
    font-family: var(--font-mono);
    font-size: 0.92rem;
    outline: none;
  }

  input:focus {
    border-color: var(--accent-dim);
  }
</style>
