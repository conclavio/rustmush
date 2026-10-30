<script lang="ts">
  // The composer (interface plan I4): the shell's one text box, mounted by
  // `Game.svelte` under every screen but the channel screen (16b cut B.1),
  // so its draft, sticky mode and history survive a screen switch.
  // Focus is input mode (D-I-17): while it has focus the shell only takes
  // Alt+key and ^K; Enter sends and keeps focus, Enter on an empty box and
  // Esc blur it (Esc keeps the draft), Shift+Enter is a newline collapsed
  // to a space on send, `/` on an empty box opens the palette, ↑/↓ browse
  // history, Tab completes a command name. The Say / Pose / Emit / OOC
  // segmented control is the client-side sticky mode (D-I-8) with a
  // "renders as" preview; an explicit trigger or a channel alias verb
  // always wins. Every keystroke feeds the composing signal.
  import {
    COMPOSE_MODES,
    composeLine,
    modeAfterPrefill,
    previewLine,
    type ComposeMode,
  } from "$lib/composer";
  import { ComposingSignal } from "$lib/composing";
  import type { GameApp } from "$lib/game.svelte";
  import { HistoryBuffer } from "$lib/history";

  import RichLine from "./RichLine.svelte";

  let { app }: { app: GameApp } = $props();

  const history = new HistoryBuffer();
  const signal = new ComposingSignal();
  let value = $state("");
  let mode = $state<ComposeMode>("none");
  let box: HTMLTextAreaElement | HTMLInputElement | undefined = $state();

  const aliases = $derived(app.channels.list.map((c) => c.alias).filter((a) => a.length > 0));
  const preview = $derived(
    previewLine(mode, value, app.session.character?.name ?? "You", aliases),
  );
  const masked = $derived(app.session.masked);

  /** Focus the box, optionally replacing its text (a prefill or a draft). */
  export function focus(prefill?: string): void {
    if (prefill !== undefined) {
      value = prefill;
    }
    // A prefill is a command, whatever mode the box was left in (16b cut B.1).
    mode = modeAfterPrefill(mode, prefill);
    box?.focus();
    // Put the caret at the end of a prefill.
    queueMicrotask(() => {
      if (box) {
        const n = box.value.length;
        box.setSelectionRange(n, n);
      }
    });
  }

  /** Blur the box, keeping its text as the draft. */
  export function blur(): void {
    box?.blur();
  }

  export function cycleMode(): void {
    const i = COMPOSE_MODES.indexOf(mode);
    mode = COMPOSE_MODES[(i + 1) % COMPOSE_MODES.length] ?? "none";
  }

  export function text(): string {
    return value;
  }

  export function setText(next: string): void {
    value = next;
  }

  function hint(h: "start" | "stop" | null): void {
    if (h === "start") {
      app.sendComposing(true);
    } else if (h === "stop") {
      app.sendComposing(false);
    }
  }

  function send(): void {
    const line = composeLine(mode, value, aliases);
    value = "";
    hint(signal.send());
    if (line.length === 0) {
      return;
    }
    if (!masked) {
      history.push(line);
    } else {
      history.reset();
    }
    app.submit(line);
  }

  function onKeydown(ev: KeyboardEvent): void {
    if ((ev.ctrlKey || ev.metaKey) && ev.key === "Enter" && app.screens.paletteOpen) {
      ev.preventDefault();
      app.palettePick(true);
      return;
    }
    if (ev.altKey || ev.ctrlKey || ev.metaKey) {
      return; // the shell's (Alt+key, ^K) — handled at the window
    }
    if (ev.key === "Enter") {
      if (ev.shiftKey) {
        return; // a newline, collapsed on send
      }
      ev.preventDefault();
      if (app.screens.paletteOpen) {
        app.palettePick();
        return;
      }
      if (value.trim().length === 0) {
        value = "";
        blur();
        return;
      }
      send();
      if (app.screens.active === "cockpit" && app.picture.on) {
        blur();
      }
      return;
    }
    if (ev.key === "Escape") {
      ev.preventDefault();
      if (app.screens.paletteOpen) {
        app.screens.paletteOpen = false;
        return;
      }
      blur();
      return;
    }
    if (ev.key === "/" && value.length === 0) {
      // Let the slash land, then open the palette behind the caret.
      queueMicrotask(() => app.openPalette());
      return;
    }
    if (app.screens.paletteOpen) {
      if (ev.key === "ArrowUp") {
        ev.preventDefault();
        app.paletteMove(-1);
        return;
      }
      if (ev.key === "ArrowDown") {
        ev.preventDefault();
        app.paletteMove(1);
        return;
      }
      if (ev.key === "Tab") {
        ev.preventDefault();
        app.palettePick();
        return;
      }
      if (ev.key === "?") {
        // The highlighted row's help page (I4-C23): a read-only `help <cmd>`.
        // A literal `?` in the palette query could only ever match nothing.
        ev.preventDefault();
        app.paletteHelp();
        return;
      }
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
    } else if (ev.key === "Tab") {
      const first = value.split(/\s/)[0] ?? "";
      if (!value.includes(" ") && first.length > 0) {
        const matches = app.commands.complete(first);
        if (matches.length > 0) {
          ev.preventDefault();
          value = `${matches[0]} `;
        }
      }
    }
  }

  function onInput(): void {
    hint(signal.keystroke(Date.now(), value));
    if (app.screens.paletteOpen) {
      app.paletteQuery(value);
    }
  }

  function onFocus(): void {
    app.screens.inputFocused = true;
  }

  function onBlur(): void {
    app.screens.inputFocused = false;
    if (app.screens.paletteOpen) {
      app.screens.paletteOpen = false;
    }
  }

  // The idle stop and the composing pill timer.
  $effect(() => {
    const timer = setInterval(() => hint(signal.idle(Date.now())), 1000);
    return () => clearInterval(timer);
  });
</script>

<div class="composer" class:focused={app.screens.inputFocused}>
  <div class="modes" role="radiogroup" aria-label="sticky mode">
    {#each COMPOSE_MODES as m (m)}
      <button
        type="button"
        role="radio"
        class="mode"
        class:active={mode === m}
        aria-checked={mode === m}
        onclick={() => {
          mode = m;
          box?.focus();
        }}
      >
        {m === "none" ? "cmd" : m}
      </button>
    {/each}
    {#if preview}
      <span class="preview" aria-label="renders as">
        <RichLine line={preview} onCmd={() => {}} />
      </span>
    {/if}
  </div>
  <div class="row">
    <span class="chev" aria-hidden="true">{app.screens.inputFocused ? ">" : "‹nav›"}</span>
    {#if masked}
      <input
        bind:this={box}
        bind:value
        type="password"
        autocomplete="off"
        aria-label="command input (hidden)"
        placeholder="(input hidden)"
        onkeydown={onKeydown}
        oninput={onInput}
        onfocus={onFocus}
        onblur={onBlur}
      />
    {:else}
      <textarea
        bind:this={box}
        bind:value
        rows="1"
        autocomplete="off"
        autocapitalize="off"
        spellcheck="false"
        aria-label="command input"
        placeholder={app.screens.inputFocused
          ? "say hello, or /help"
          : "Enter to type · ? for keys"}
        onkeydown={onKeydown}
        oninput={onInput}
        onfocus={onFocus}
        onblur={onBlur}
      ></textarea>
    {/if}
  </div>
</div>

<style>
  .composer {
    border-top: 1px solid var(--border);
    background: var(--bg-panel);
    padding: 0.35rem 0.75rem 0.45rem;
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }

  .modes {
    display: flex;
    align-items: center;
    gap: 0.25rem;
    font-size: 0.78rem;
    min-height: 1.4rem;
  }

  .mode {
    background: none;
    border: 1px solid var(--border);
    border-radius: 999px;
    color: var(--fg-muted);
    cursor: pointer;
    font: inherit;
    font-size: 0.75rem;
    padding: 0.05rem 0.55rem;
  }

  .mode.active {
    background: var(--accent-dim);
    color: var(--fg);
    border-color: var(--accent-dim);
  }

  .preview {
    margin-left: 0.6rem;
    color: var(--fg-muted);
    font-family: var(--font-mono);
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    min-width: 0;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .chev {
    color: var(--fg-faint);
    font-family: var(--font-mono);
    font-size: 0.8rem;
    user-select: none;
    min-width: 2.6rem;
  }

  .focused .chev {
    color: var(--accent);
  }

  textarea,
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
    resize: none;
    line-height: 1.3;
  }

  textarea:focus,
  input:focus {
    border-color: var(--accent);
  }

  /* A phone: the composer is a bottom sheet with the mode chips on top,
   * and focus is the only mode (interface plan I5). */
  @media (max-width: 800px) {
    .composer {
      position: sticky;
      bottom: 0;
      box-shadow: 0 -4px 16px rgb(0 0 0 / 35%);
    }

    .preview {
      display: none;
    }
  }
</style>
