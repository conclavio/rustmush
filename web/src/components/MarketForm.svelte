<script lang="ts">
  // The market screen's one form (17g cut C3c): the TUI's forms (17g cut C3b) field for field
  // — the purchase's four steps, the renewal's three, the resale or charter offer, the accept,
  // the listing and the withdrawal — with the same titles, rows, defaults and feet. Submit
  // builds ONE typed `MarketAction` from the fields (`MarketStore.build`) and hands it to
  // `app.submitMarket`; nothing here writes a command line. Esc closes the form; once the
  // request is out it closes the window and never cancels the request.
  import type { GameApp } from "$lib/game.svelte";
  import {
    ACCEPT_HINT,
    FORM_FOOT,
    NOTICE_FOOT,
    RENEW_NOTE,
    SENT_FOOT,
    WITHDRAW_FOOT,
  } from "$lib/market";
  import {
    BUY_STEPS,
    LISTING_FORMS,
    OFFER_KINDS,
    RENEW_STEPS,
    formTitle,
    withdrawText,
  } from "$lib/stores/market.svelte";

  let { app }: { app: GameApp } = $props();

  const m = $derived(app.market);
  const form = $derived(m.form);
  const pending = $derived(m.pending);
  /** The client's own refusal of the typed fields (the TUI's `Request refused: …`). */
  let refusal = $state("");
  // A new form (the offer's closing notice after a staged reply, say) starts with no refusal.
  $effect(() => {
    void form;
    refusal = "";
  });

  let formEl = $state<HTMLFormElement | null>(null);
  /**
   * Focus the first field (or the submit button) whenever a NEW form is shown and nothing is
   * out, so Enter sends it or closes it. Not only on mount: a staged offer swaps the form for
   * its "Offer sent" notice, and a `shown` reply re-fills the accept form, in the same
   * `<form>` element — and while the request was out every control was disabled, so the focus
   * had already left. A field edit mutates the form in place and never moves the focus.
   */
  $effect(() => {
    const node = formEl;
    if (node === null || form === null || pending) {
      return;
    }
    queueMicrotask(() => {
      node.querySelector<HTMLElement>("input:not(:disabled), button[type='submit']:not(:disabled)")?.focus();
    });
  });

  function onKeydown(ev: KeyboardEvent): void {
    if (ev.key === "Escape") {
      ev.preventDefault();
      app.market.closeForm();
    } else if (ev.key === "Enter" && form?.kind === "notice" && !ev.altKey && !ev.ctrlKey && !ev.metaKey) {
      // `NOTICE_FOOT` says Enter closes, wherever the focus is (the shell leaves the bare
      // keys to an open market form, so nothing else takes it).
      ev.preventDefault();
      app.market.closeForm();
    }
  }

  function submit(ev: SubmitEvent): void {
    ev.preventDefault();
    if (pending) {
      return;
    }
    const built = app.market.build();
    if (built === null) {
      return;
    }
    if (typeof built === "string") {
      refusal = built;
      return;
    }
    refusal = "";
    void app.submitMarket(built);
  }

  /** The chosen option as the TUI paints it: `[pay]`. */
  const mark = (on: boolean, label: string): string => (on ? `[${label}]` : label);
</script>

<svelte:window onkeydown={onKeydown} />

{#if form !== null}
  <div class="sheet" role="dialog" aria-label={formTitle(form)}>
    <form bind:this={formEl} onsubmit={submit}>
      <strong class="title">{formTitle(form)}</strong>
      {#if form.kind !== "notice"}
        <span class="state">{pending ? "Sent - waiting for the server" : "Not sent"}</span>
      {/if}

      {#if form.kind === "buy"}
        <div class="choice" role="group" aria-label="Step">
          <span class="label">Step</span>
          {#each BUY_STEPS as step (step)}
            <button type="button" disabled={pending} aria-pressed={form.step === step} onclick={() => {
              if (form?.kind === "buy") form.step = step;
            }}>{mark(form.step === step, step)}</button>
          {/each}
        </div>
        {#each form.confirm as line, i (i)}<p class="line">{line}</p>{/each}
      {:else if form.kind === "renew"}
        <div class="choice" role="group" aria-label="Step">
          <span class="label">Step</span>
          {#each RENEW_STEPS as step (step)}
            <button type="button" disabled={pending} aria-pressed={form.step === step} onclick={() => {
              if (form?.kind === "renew") form.step = step;
            }}>{mark(form.step === step, step)}</button>
          {/each}
        </div>
        <p class="line">{RENEW_NOTE}</p>
      {:else if form.kind === "offer"}
        <label>
          <span class="label">Player</span>
          <input type="text" autocomplete="off" maxlength="40" disabled={pending} bind:value={form.player} />
        </label>
        <label>
          <span class="label">Price</span>
          <input type="text" autocomplete="off" maxlength="40" disabled={pending} bind:value={form.price} />
        </label>
      {:else if form.kind === "accept"}
        <div class="choice" role="group" aria-label="Kind">
          <span class="label">Kind</span>
          {#each OFFER_KINDS as kind (kind)}
            <button type="button" disabled={pending} aria-pressed={form.target === kind} onclick={() => {
              if (form?.kind === "accept") form.target = kind;
            }}>{mark(form.target === kind, kind)}</button>
          {/each}
        </div>
        <label>
          <span class="label">Number</span>
          <input type="text" inputmode="numeric" autocomplete="off" maxlength="20" disabled={pending} bind:value={form.number} />
        </label>
        <label>
          <span class="label">Price</span>
          <input type="text" autocomplete="off" maxlength="40" disabled={pending} bind:value={form.price} />
        </label>
        <p class="line muted">{ACCEPT_HINT}</p>
        {#each form.shown as line, i (i)}<p class="line shown">{line}</p>{/each}
      {:else if form.kind === "publish"}
        <div class="choice" role="group" aria-label="Form">
          <span class="label">Form</span>
          {#each LISTING_FORMS as f (f.value)}
            <button type="button" disabled={pending} aria-pressed={form.form === f.value} onclick={() => {
              if (form?.kind === "publish") form.form = f.value;
            }}>{mark(form.form === f.value, f.label)}</button>
          {/each}
        </div>
        <label>
          <span class="label">Royalty</span>
          <input type="text" inputmode="numeric" autocomplete="off" maxlength="20" disabled={pending} bind:value={form.royalty} />
          <span class="muted">per mille (empty is 0)</span>
        </label>
        <div class="choice" role="group" aria-label="Transfer">
          <span class="label">Transfer</span>
          {#each [true, false] as yes (yes)}
            <button type="button" disabled={pending} aria-pressed={form.transfer === yes} onclick={() => {
              if (form?.kind === "publish") form.transfer = yes;
            }}>{mark(form.transfer === yes, yes ? "yes" : "no")}</button>
          {/each}
        </div>
      {:else if form.kind === "withdraw"}
        <p class="line">{withdrawText(form.filed, form.name)}</p>
      {:else if form.kind === "notice"}
        {#each form.lines as line, i (i)}<p class="line">{line}</p>{/each}
      {/if}

      {#if refusal !== ""}<p class="refusal" role="alert">{refusal}</p>{/if}

      <div class="buttons">
        {#if form.kind === "notice"}
          <button type="submit">Close</button>
        {:else}
          <button type="submit" disabled={pending}>{form.kind === "withdraw" ? "Withdraw" : "Send"}</button>
          <button type="button" onclick={() => app.market.closeForm()}>{pending ? "Close" : "Cancel"}</button>
        {/if}
      </div>
      <small class="foot">
        {#if pending}{SENT_FOOT}{:else if form.kind === "notice"}{NOTICE_FOOT}{:else if form.kind === "withdraw"}{WITHDRAW_FOOT}{:else}{FORM_FOOT}{/if}
      </small>
    </form>
  </div>
{/if}

<style>
  .sheet {
    position: absolute;
    top: 0.75rem;
    left: 50%;
    transform: translateX(-50%);
    width: min(44rem, calc(100% - 1.5rem));
    z-index: 5;
  }
  form {
    display: grid;
    gap: 0.45rem;
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 0.75rem;
    background: var(--bg-raised);
    box-shadow: 0 6px 24px rgb(0 0 0 / 0.35);
  }
  .title {
    font-size: 0.95rem;
  }
  .state,
  .muted,
  .foot {
    color: var(--fg-muted);
    font-size: 0.8rem;
  }
  .label {
    display: inline-block;
    min-width: 5.5rem;
    font-size: 0.85rem;
  }
  label {
    display: flex;
    gap: 0.4rem;
    align-items: center;
  }
  .choice {
    display: flex;
    gap: 0.35rem;
    align-items: center;
    flex-wrap: wrap;
  }
  input,
  button {
    font: inherit;
    color: var(--fg);
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 0.3rem 0.5rem;
    min-width: 0;
  }
  input {
    flex: 1 1 auto;
  }
  button {
    cursor: pointer;
  }
  button[aria-pressed="true"] {
    border-color: var(--accent);
    font-weight: 600;
  }
  button:disabled {
    cursor: default;
    opacity: 0.6;
  }
  .line {
    margin: 0;
    font-size: 0.85rem;
    overflow-wrap: anywhere;
  }
  .shown {
    font-family: var(--font-mono, monospace);
  }
  .refusal {
    margin: 0;
    color: var(--danger, #e06c75);
    font-size: 0.85rem;
  }
  .buttons {
    display: flex;
    gap: 0.5rem;
  }
</style>
