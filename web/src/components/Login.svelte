<script lang="ts">
  // Login / register card. Calls /api/auth/* via $lib/auth; the token lands
  // in localStorage (GameApp) and the flow moves to character select.
  import { AuthError } from "$lib/auth";
  import type { GameApp } from "$lib/game.svelte";

  let { app }: { app: GameApp } = $props();

  let mode = $state<"login" | "register">("login");
  // svelte-ignore state_referenced_locally -- deliberate: seed the field
  // with the remembered account once; the user edits it freely afterwards.
  let account = $state(app.account ?? "");
  let password = $state("");
  let charName = $state("");
  let error = $state<string | null>(null);
  let busy = $state(false);

  async function submit(ev: SubmitEvent): Promise<void> {
    ev.preventDefault();
    if (busy) {
      return;
    }
    error = null;
    busy = true;
    try {
      if (mode === "login") {
        await app.login(account.trim(), password);
      } else {
        await app.register(account.trim(), password, charName.trim());
      }
      password = "";
    } catch (e) {
      error = e instanceof AuthError ? e.message : "Something went wrong — try again.";
    } finally {
      busy = false;
    }
  }
</script>

<div class="screen">
  <form class="card" onsubmit={submit}>
    <h1>rustmush</h1>
    <p class="sub">a modern MUSH — play in your browser or over SSH</p>

    {#if app.loginNotice}
      <p class="notice">{app.loginNotice}</p>
    {/if}

    <div class="modes" role="tablist" aria-label="sign in or register">
      <button
        type="button"
        class:active={mode === "login"}
        onclick={() => {
          mode = "login";
          error = null;
        }}>Sign in</button
      >
      <button
        type="button"
        class:active={mode === "register"}
        onclick={() => {
          mode = "register";
          error = null;
        }}>New account</button
      >
    </div>

    <label>
      Account
      <input
        bind:value={account}
        autocomplete="username"
        autocapitalize="off"
        spellcheck="false"
        required
      />
    </label>
    <label>
      Password
      <input
        type="password"
        bind:value={password}
        autocomplete={mode === "login" ? "current-password" : "new-password"}
        required
      />
    </label>
    {#if mode === "register"}
      <label>
        Character name
        <input bind:value={charName} autocapitalize="off" spellcheck="false" required />
      </label>
      <p class="hint">Your first character is created with the account.</p>
    {/if}

    {#if error}
      <p class="error" role="alert">{error}</p>
    {/if}

    <button type="submit" class="primary" disabled={busy}>
      {busy ? "…" : mode === "login" ? "Sign in" : "Create account"}
    </button>

    <p class="hint alt">
      Prefer a terminal? <code>ssh play@&lt;host&gt;</code> works too.
    </p>
  </form>
</div>

<style>
  .screen {
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 1rem;
  }

  .card {
    width: min(22rem, 100%);
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 1.5rem 1.6rem;
    display: flex;
    flex-direction: column;
    gap: 0.7rem;
  }

  h1 {
    margin: 0;
    font-size: 1.4rem;
    letter-spacing: 0.05em;
  }

  .sub {
    margin: -0.4rem 0 0.3rem;
    color: var(--fg-muted);
    font-size: 0.85rem;
  }

  .notice {
    margin: 0;
    padding: 0.4rem 0.6rem;
    background: var(--bg-input);
    border: 1px solid var(--accent-dim);
    border-radius: 6px;
    color: var(--accent);
    font-size: 0.82rem;
  }

  .modes {
    display: flex;
    gap: 0.4rem;
  }

  .modes button {
    flex: 1;
    background: none;
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--fg-muted);
    padding: 0.35rem;
    cursor: pointer;
    font: inherit;
    font-size: 0.85rem;
  }

  .modes button.active {
    border-color: var(--accent);
    color: var(--accent);
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    font-size: 0.82rem;
    color: var(--fg-muted);
  }

  input {
    background: var(--bg-input);
    color: var(--fg);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 0.45rem 0.6rem;
    font: inherit;
  }

  input:focus {
    outline: none;
    border-color: var(--accent-dim);
  }

  .error {
    margin: 0;
    color: var(--error);
    font-size: 0.85rem;
  }

  .primary {
    background: var(--accent-dim);
    color: var(--fg);
    border: none;
    border-radius: 6px;
    padding: 0.5rem;
    cursor: pointer;
    font: inherit;
    font-weight: 600;
  }

  .primary:hover {
    filter: brightness(1.15);
  }

  .primary:disabled {
    opacity: 0.6;
    cursor: default;
  }

  .hint {
    margin: 0;
    color: var(--fg-faint);
    font-size: 0.75rem;
  }

  .alt {
    text-align: center;
  }

  code {
    font-family: var(--font-mono);
  }
</style>
