<script lang="ts">
  // The mind market (17g cut C3c; the C3 plan §2.5), on the letter hotkey `m`: the counter's
  // listings on the left with the actor's own quotes and waiting purchases under their header,
  // the actor's own holdings on the right with the section toggle, and the screen's status
  // sentence. The web twin of the TUI's market screen (17g cut C3b): the same columns, the same
  // forms, the same sentences — though the web has the room to paint every column, so the
  // holdings' PHASE and FLAGS show here at any width.
  //
  // **Nothing here sends.** A click selects a row; the legend's keys (`b k t a l x`) open a form
  // over the selection; only Submit inside the form sends, through `app.submitMarket` — one
  // typed `MindRequest::Market`, never a line, never `onCmd`. The rows paint the keyframe's own
  // fields: a price is its `price_text`, a listing is its 8-character code, and nothing else of
  // a token, an account or a digest exists in what the decoder keeps.
  import type { GameApp } from "$lib/game.svelte";
  import {
    EMPTY_SECTION,
    MORE_PAGES,
    NO_PAGE,
    NO_PAGE_YET,
    holdingFlags,
    holdingKind,
    holdingNumber,
    listingFlags,
    ownStrip,
    phaseCell,
    termCell,
  } from "$lib/market";
  import type { MindHoldingsSection } from "$lib/proto";
  import MarketForm from "./MarketForm.svelte";

  let { app }: { app: GameApp } = $props();

  const m = $derived(app.market);
  const view = $derived(m.market);
  const holdings = $derived(m.shownHoldings);
  const strip = $derived(view ? ownStrip(view) : []);
  /** The own strip's codes, in the strip's order (a line selects its code). */
  const stripCodes = $derived(view ? [...view.own.quotes.map((q) => q.code), ...view.own.waiting] : []);

  // A pick lets go of the button it clicked: the shell leaves every bare key to a focused
  // control (`isInteractiveControl`, `Game.svelte`), so a row button that kept the focus would
  // swallow the very `b k t a l x h ] [` the selection was made for.
  function pickListing(code: string, button: HTMLElement): void {
    button.blur();
    app.market.selected = code;
    app.market.focus = "listings";
  }

  function pickHolding(n: number, button: HTMLElement): void {
    button.blur();
    app.market.selectedHolding = n;
    app.market.focus = "holdings";
  }

  function pickSection(section: MindHoldingsSection, button: HTMLElement): void {
    button.blur();
    app.market.focus = "holdings";
    if (section !== app.market.section) {
      app.localAction("marketSection");
    }
  }
</script>

<div class="market">
  <section class="pane listings" class:focused={m.focus === "listings"} aria-label="mind market">
    <h2>mind market</h2>
    {#if view === null}
      <p class="muted">{m.marketBad ? NO_PAGE : NO_PAGE_YET}</p>
    {:else}
      {#if view.notice !== null}<p class="notice">{view.notice}</p>{/if}
      {#if strip.length > 0}
        <ul class="own" aria-label="your quotes and purchases">
          {#each strip as line, i (i)}
            <li>
              <button
                type="button"
                class="row"
                class:sel={m.selected === stripCodes[i]}
                aria-pressed={m.selected === stripCodes[i]}
                onclick={(ev) => pickListing(stripCodes[i] ?? "", ev.currentTarget)}>{line}</button
              >
            </li>
          {/each}
        </ul>
      {/if}
      {#if view.rows.length > 0}
        <table>
          <thead>
            <tr>
              <th>CODE</th><th>NAME</th><th>FORM</th><th>CLASS</th><th class="num">PRICE</th><th>TERM+GRACE</th><th>FLAGS</th>
            </tr>
          </thead>
          <tbody>
            <!-- Keyed by place, not code: a code is a token's first 8 characters and two
                 listings may share one (the server's `ambiguous` answer). -->
            {#each view.rows as row, i (i)}
              <tr class:sel={m.selected === row.code}>
                <td>
                  <button
                    type="button"
                    class="code"
                    aria-pressed={m.selected === row.code}
                    onclick={(ev) => pickListing(row.code, ev.currentTarget)}>{row.code}</button
                  >
                </td>
                <td>{row.name}</td>
                <td>{row.form}</td>
                <td>{row.class}</td>
                <td class="num">{row.priceText}</td>
                <td>{termCell(row)}</td>
                <td>{listingFlags(row)}</td>
              </tr>
            {/each}
          </tbody>
        </table>
        {#if view.after !== null && m.focus === "listings"}<p class="muted">{MORE_PAGES}</p>{/if}
      {/if}
    {/if}
  </section>

  <section class="pane holdings" class:focused={m.focus === "holdings"} aria-label="holdings">
    <h2>holdings</h2>
    <div class="sections" role="group" aria-label="section">
      {#each ["owner", "paid"] as const as s (s)}
        <button type="button" class:on={m.section === s} aria-pressed={m.section === s} onclick={(ev) => pickSection(s, ev.currentTarget)}
          >{s}</button
        >
      {/each}
    </div>
    {#if holdings === null}
      <p class="muted">{m.holdingsBad ? NO_PAGE : NO_PAGE_YET}</p>
    {:else}
      {#if holdings.notice !== null}<p class="notice">{holdings.notice}</p>{/if}
      {#if holdings.rows.length > 0}
        <table>
          <thead>
            <tr><th>NUMBER</th><th>KIND</th><th>NAME</th><th>FORM</th><th>PHASE</th><th>FLAGS</th></tr>
          </thead>
          <tbody>
            <!-- By place too: the decoder keeps what the server sent, and a repeated number
                 must never break the pane. -->
            {#each holdings.rows as row, i (i)}
              <tr class:sel={m.selectedHolding === holdingNumber(row)}>
                <td>
                  <button
                    type="button"
                    class="code"
                    aria-pressed={m.selectedHolding === holdingNumber(row)}
                    onclick={(ev) => pickHolding(holdingNumber(row), ev.currentTarget)}>{holdingNumber(row)}</button
                  >
                </td>
                <td>{holdingKind(row)}</td>
                <td>{row.name}</td>
                <td>{row.kind === "paid" ? row.form : ""}</td>
                <td>{phaseCell(row.phase)}</td>
                <td>{holdingFlags(row)}</td>
              </tr>
            {/each}
          </tbody>
        </table>
        {#if holdings.after !== null && m.focus === "holdings"}<p class="muted">{MORE_PAGES}</p>{/if}
      {:else if holdings.notice === null}
        <p class="muted">{EMPTY_SECTION}</p>
      {/if}
    {/if}
  </section>

  {#if m.form !== null}
    <MarketForm {app} />
  {/if}
</div>
{#if m.notice !== ""}
  <p class="status" role="status">{m.notice}</p>
{/if}

<style>
  .market {
    display: flex;
    gap: 0.75rem;
    padding: 0.5rem 0.75rem;
    flex: 1;
    min-height: 0;
    box-sizing: border-box;
    overflow: auto;
    position: relative;
    flex-wrap: wrap;
    align-content: flex-start;
  }
  .pane {
    border: 1px solid transparent;
    border-radius: 6px;
    padding: 0.25rem 0.4rem;
    min-width: 0;
  }
  .pane.focused {
    border-color: var(--border);
  }
  .listings {
    flex: 3 1 38rem;
  }
  .holdings {
    flex: 2 1 26rem;
  }
  h2 {
    font-size: 1rem;
    margin: 0 0 0.4rem;
  }
  table {
    border-collapse: collapse;
    width: 100%;
    font-family: var(--font-mono, monospace);
    font-size: 0.8rem;
  }
  th {
    text-align: left;
    color: var(--fg-muted);
    font-weight: 600;
    padding: 0.1rem 0.4rem;
  }
  td {
    padding: 0.1rem 0.4rem;
    white-space: nowrap;
  }
  .num {
    text-align: right;
  }
  tr.sel,
  button.row.sel {
    background: var(--bg-raised);
    outline: 1px solid var(--accent);
  }
  button.code,
  button.row {
    background: none;
    border: none;
    color: inherit;
    font: inherit;
    cursor: pointer;
    padding: 0;
    text-align: left;
  }
  .own {
    list-style: none;
    margin: 0 0 0.4rem;
    padding: 0;
    font-family: var(--font-mono, monospace);
    font-size: 0.8rem;
  }
  .sections {
    display: flex;
    gap: 0.35rem;
    margin-bottom: 0.4rem;
  }
  .sections button {
    font: inherit;
    font-size: 0.8rem;
    color: var(--fg);
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 0.1rem 0.5rem;
    cursor: pointer;
  }
  .sections button.on {
    border-color: var(--accent);
    font-weight: 600;
  }
  .notice {
    margin: 0 0 0.4rem;
  }
  .muted {
    color: var(--fg-muted);
    margin: 0.25rem 0;
  }
  .status {
    margin: 0;
    padding: 0.25rem 0.75rem;
    border-top: 1px solid var(--border);
    font-size: 0.85rem;
  }
</style>
