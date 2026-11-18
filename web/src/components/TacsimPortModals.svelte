<script lang="ts">
  // Diegetic Svelte 5 Modals for Corridor Port Trade, Sailing Plans, Co-op Chartering,
  // NPC Conversations, Prospector Rigs, and the OOC 6v6 Simulator Console.
  // Preserves `Game.test.ts`'s single `"command input"` invariant by naming every
  // modal field distinctly.

  import type { GameApp } from "$lib/game.svelte";
  import {
    buildNpcTopics,
    computeMaxFitQuantity,
    CORRIDOR_DESTINATIONS,
    type PortListingRow,
  } from "$lib/tacsim/trade";

  let { app, onCmd }: { app: GameApp; onCmd: (cmd: string) => void } = $props();

  let npcSayDraft = $state("");
  let hullNameDraft = $state("");

  $effect(() => {
    if (app.tacsim.activeModal === "coop") {
      hullNameDraft = app.tacsim.coopSuggestedHull;
    } else if (app.tacsim.activeModal === "npc") {
      npcSayDraft = "";
    }
  });

  const modal = $derived(app.tacsim.activeModal);
  const trade = $derived(app.tacsim.trade);
  const st = $derived(trade.stowage);

  const massPct = $derived(Math.min(100, Math.round((st.cargoKg / Math.max(1, st.cargoMaxKg)) * 100)));
  const volPct = $derived(Math.min(100, Math.round((st.cargoL / Math.max(1, st.cargoMaxL)) * 100)));
  const tankPct = $derived(Math.min(100, Math.round((st.propellantKg / Math.max(1, st.propellantMaxKg)) * 100)));
  const storesPct = $derived(
    Math.min(100, Math.round((st.provisionsCrewDays / Math.max(1, st.provisionsMaxCrewDays)) * 100)),
  );

  function resolveLotQty(row: PortListingRow): number {
    const preset = app.tacsim.selectedQtyPreset;
    if (preset === "max") {
      return computeMaxFitQuantity(row, app.tacsim.goodsById, st, trade.creditsMinor);
    }
    return Math.max(1, Math.min(row.quantity || preset, preset));
  }

  function submitNpcSay(): void {
    const npc = app.tacsim.focusedNpc;
    const trimmed = npcSayDraft.trim();
    if (!npc || !trimmed) return;
    onCmd(`say ${npc.name}, ${trimmed}`);
    npcSayDraft = "";
  }

  function submitCoopEnroll(): void {
    const chosen = hullNameDraft.trim() || app.tacsim.coopSuggestedHull;
    onCmd(`+coop/enroll ${chosen}`);
  }

  const simSlots = $derived(
    trade.simulator?.slots && trade.simulator.slots.length > 0
      ? trade.simulator.slots
      : app.tacsim.ambient.simSlots,
  );
</script>

{#if modal !== null}
  <div
    class="tac-backdrop"
    role="presentation"
    onclick={() => app.tacsim.closeModal()}
  >
    <div
      class="tac-modal"
      class:wide={modal === "trade" || modal === "sailing" || modal === "sim"}
      role="dialog"
      aria-label="port terminal dialog"
      tabindex="-1"
      onclick={(ev) => ev.stopPropagation()}
      onkeydown={(ev) => {
        if (ev.key === "Escape") {
          ev.stopPropagation();
          app.tacsim.closeModal();
        }
      }}
    >
      {#if modal === "trade" || modal === "sailing"}
        <header class="mhead">
          <div>
            <h2>{trade.venueLabel.toUpperCase()}</h2>
            <span class="sub">Liquid Balance: <strong class="cr">{trade.creditsText}</strong></span>
          </div>
          <div class="tab-switch">
            <button
              type="button"
              class="tbtn"
              class:active={modal === "trade"}
              onclick={() => (app.tacsim.activeModal = "trade")}
            >
              Commodity Exchange
            </button>
            <button
              type="button"
              class="tbtn"
              class:active={modal === "sailing"}
              onclick={() => {
                app.tacsim.activeModal = "sailing";
                onCmd("+ship/orders");
              }}
            >
              Sailing Plan & Route
            </button>
            <button type="button" class="ghost" onclick={() => app.tacsim.requestPortTrade()}>
              Refresh
            </button>
            <button type="button" class="ghost" onclick={() => app.tacsim.closeModal()}>✕</button>
          </div>
        </header>

        <div class="gauges">
          <div class="gauge">
            <div class="gtop"><span>Hold Mass</span><span>{(st.cargoKg / 1000).toFixed(1)} / {(st.cargoMaxKg / 1000).toFixed(0)} t</span></div>
            <div class="gbar"><div class="gfill mass" style="width: {massPct}%"></div></div>
          </div>
          <div class="gauge">
            <div class="gtop"><span>Packed Vol</span><span>{(st.cargoL / 1000).toFixed(1)} / {(st.cargoMaxL / 1000).toFixed(0)} m³</span></div>
            <div class="gbar"><div class="gfill vol" style="width: {volPct}%"></div></div>
          </div>
          <div class="gauge">
            <div class="gtop"><span>Reaction Tank</span><span>{(st.propellantKg / 1000).toFixed(1)} / {(st.propellantMaxKg / 1000).toFixed(0)} t</span></div>
            <div class="gbar"><div class="gfill tank" style="width: {tankPct}%"></div></div>
          </div>
          <div class="gauge">
            <div class="gtop"><span>Crew Stores</span><span>{st.provisionsCrewDays} / {st.provisionsMaxCrewDays} d</span></div>
            <div class="gbar"><div class="gfill stores" style="width: {storesPct}%"></div></div>
          </div>
        </div>

        {#if modal === "trade"}
          <div class="qty-row">
            <span class="muted">Lot Quantity:</span>
            {#each [1, 5, 10, 25, "max"] as const as q (q)}
              <button
                type="button"
                class="qchip"
                class:on={app.tacsim.selectedQtyPreset === q}
                onclick={() => (app.tacsim.selectedQtyPreset = q)}
              >
                {q === "max" ? "MAX FIT" : `${q}`}
              </button>
            {/each}
          </div>

          <div class="tables">
            <section class="tcol">
              <h3>Port Asks (Buy Into Hold)</h3>
              {#if trade.asks.length === 0}
                <p class="muted">No active vendor asks at this port venue.</p>
              {:else}
                <div class="rows">
                  {#each trade.asks as row (row.listingId)}
                    {@const qty = resolveLotQty(row)}
                    <div class="lrow">
                      <div class="lmeta">
                        <strong>{row.itemName}</strong>
                        <span class="muted">#{row.listingId} · {row.vendorName} · {row.quantity} {row.unit} avail</span>
                      </div>
                      <div class="lact">
                        <span class="price">{row.unitPriceText}</span>
                        <button
                          type="button"
                          class="buybtn"
                          onclick={() => app.tacsim.buyListing(row, qty)}
                        >
                          Buy {qty} {row.unit}
                        </button>
                      </div>
                    </div>
                  {/each}
                </div>
              {/if}
            </section>

            <section class="tcol">
              <h3>Standing Bids (Sell From Hold)</h3>
              {#if trade.bids.length === 0}
                <p class="muted">No standing buyer bids at this port venue.</p>
              {:else}
                <div class="rows">
                  {#each trade.bids as row (row.listingId)}
                    {@const qty = resolveLotQty(row)}
                    <div class="lrow">
                      <div class="lmeta">
                        <strong>{row.itemName}</strong>
                        <span class="muted">#{row.listingId} · {row.vendorName} · wants {row.quantity} {row.unit}</span>
                      </div>
                      <div class="lact">
                        <span class="price bid">{row.unitPriceText}</span>
                        <button
                          type="button"
                          class="sellbtn"
                          onclick={() => app.tacsim.sellListing(row, qty, false)}
                        >
                          Sell {qty}
                        </button>
                        <button
                          type="button"
                          class="ghost sm"
                          onclick={() => app.tacsim.sellListing(row, qty, true)}
                        >
                          All
                        </button>
                      </div>
                    </div>
                  {/each}
                </div>
              {/if}
            </section>
          </div>

          {#if trade.hasMore && trade.nextAfterId !== null}
            <div class="foot-row">
              <button
                type="button"
                class="ghost"
                onclick={() => app.tacsim.requestPortTrade(trade.nextAfterId ?? undefined)}
              >
                Load More Listings
              </button>
            </div>
          {/if}
        {:else}
          <div class="sailing-body">
            <h3>Select Corridor Destination Port</h3>
            <div class="dest-grid">
              {#each CORRIDOR_DESTINATIONS as dest (dest.id)}
                <button
                  type="button"
                  class="dest-card"
                  class:on={app.tacsim.selectedDestination === dest.id}
                  onclick={() => (app.tacsim.selectedDestination = dest.id)}
                >
                  <strong>{dest.label}</strong>
                  <span>{dest.body}</span>
                </button>
              {/each}
            </div>
            <div class="act-row">
              <button
                type="button"
                class="buybtn"
                onclick={() => onCmd(`+ship/plan ${app.tacsim.selectedDestination}`)}
              >
                File Sailing Plan (+ship/plan {app.tacsim.selectedDestination})
              </button>
              <button type="button" class="ghost" onclick={() => onCmd("+ship/orders")}>
                Inspect Orders (+ship/orders)
              </button>
              <button
                type="button"
                class="sellbtn"
                onclick={() => onCmd(`+ship/undock ${app.tacsim.selectedDestination}`)}
              >
                Clear & Undock (+ship/undock {app.tacsim.selectedDestination})
              </button>
            </div>
          </div>
        {/if}

      {:else if modal === "coop"}
        <header class="mhead">
          <h2>CO-OP FIRST CAPTAIN PROGRAMME DESK</h2>
          <button type="button" class="ghost" onclick={() => app.tacsim.closeModal()}>✕</button>
        </header>
        <p class="prose">
          The Co-op stakes a Stickney-built Dray bulk hauler and your initial hold; it takes its lay
          from settled cargo sales until buyout. Choose a unique hull registry name:
        </p>
        <div class="form-row">
          <input
            type="text"
            aria-label="charter hull name"
            bind:value={hullNameDraft}
            placeholder="Hull registry name..."
          />
          <button type="button" class="buybtn" onclick={submitCoopEnroll}>
            Enroll Charter Hull
          </button>
        </div>
        <div class="act-row">
          <button type="button" class="ghost" onclick={() => onCmd("+coop")}>Read Terms (+coop)</button>
          <button type="button" class="ghost" onclick={() => onCmd("+coop/statement")}>Statement (+coop/statement)</button>
          <button type="button" class="ghost" onclick={() => onCmd("+coop/refit prospector")}>Refit Dray-P (+coop/refit prospector)</button>
        </div>

      {:else if modal === "npc" && app.tacsim.focusedNpc}
        {@const npc = app.tacsim.focusedNpc}
        <header class="mhead">
          <div>
            <h2>{npc.name.toUpperCase()}</h2>
            {#if npc.bio}<span class="sub">{npc.bio}</span>{/if}
          </div>
          <button type="button" class="ghost" onclick={() => app.tacsim.closeModal()}>✕</button>
        </header>
        <div class="topic-list">
          {#each buildNpcTopics(npc.name) as t (t.label)}
            <button type="button" class="topic-btn" onclick={() => onCmd(t.command)}>
              {t.label}
            </button>
          {/each}
        </div>
        <form
          class="form-row"
          onsubmit={(ev) => {
            ev.preventDefault();
            submitNpcSay();
          }}
        >
          <input
            type="text"
            aria-label="say to NPC"
            bind:value={npcSayDraft}
            placeholder="Speak directly to {npc.name}..."
          />
          <button type="submit" class="buybtn">Say</button>
        </form>

      {:else if modal === "sim"}
        <header class="mhead">
          <div>
            <h2>OOC TACTICAL SIMULATOR CONSOLE (6v6)</h2>
            <span class="sub">
              State: <strong>{trade.simulator?.matchState ?? app.tacsim.ambient.simMatchState ?? "staging"}</strong>
              · AI Fill: <strong>{trade.simulator?.aiFill ?? app.tacsim.ambient.simAiFill}</strong>
            </span>
          </div>
          <button type="button" class="ghost" onclick={() => app.tacsim.closeModal()}>✕</button>
        </header>
        {#if simSlots.length > 0}
          <div class="sim-roster">
            {#each simSlots as s (`${s.team}-${s.index}`)}
              <span class="slot-chip" class:red={s.team === "red"} class:blue={s.team !== "red"}>
                {s.team === "red" ? "R" : "B"}{s.index} · {s.pilotName} ({s.unit.replace("tacsim.unit.", "")}){s.seated ? " [POD]" : ""}
              </span>
            {/each}
          </div>
        {/if}
        <div class="act-row">
          <button type="button" class="sellbtn" onclick={() => onCmd("+sim/join red")}>Join Red Team</button>
          <button type="button" class="buybtn" onclick={() => onCmd("+sim/join blue")}>Join Blue Team</button>
          <button type="button" class="ghost" onclick={() => onCmd("+sim/ready")}>Ready (+sim/ready)</button>
          <button type="button" class="ghost" onclick={() => onCmd("board")}>Board Pod (board)</button>
        </div>
        <div class="act-row">
          <span class="muted">Unit Chassis:</span>
          {#each ["line_tank", "scout", "missile_carrier", "heavy_mech"] as u (u)}
            <button type="button" class="qchip" onclick={() => onCmd(`+sim/unit ${u}`)}>{u}</button>
          {/each}
          <span class="muted">AI Fill:</span>
          {#each [0, 2, 4, 6] as ai (ai)}
            <button type="button" class="qchip" onclick={() => onCmd(`+sim/ai ${ai}`)}>{ai}</button>
          {/each}
        </div>

      {:else if modal === "rig"}
        <header class="mhead">
          <h2>DRAY-P VEHICLE BAY — PROSPECTOR RIG CRADLE</h2>
          <button type="button" class="ghost" onclick={() => app.tacsim.closeModal()}>✕</button>
        </header>
        <p class="prose">
          Deploy a tracked Prospector Rig down the stern ramp onto a planetary surface site
          (Occator Rim, Rheasilvia, Ahuna Flank), or switch to the Quarry screen (hotkey <code>q</code>):
        </p>
        <div class="act-row">
          <button type="button" class="buybtn" onclick={() => onCmd("+tv/1 launch go")}>
            Launch Rig 1 (+tv/1 launch go)
          </button>
          <button type="button" class="ghost" onclick={() => onCmd("+tv/lease go")}>
            Lease Rig (+tv/lease go)
          </button>
          <button type="button" class="ghost" onclick={() => onCmd("+tv/bay out")}>
            Return to Hold (+tv/bay out)
          </button>
        </div>

      {:else if modal === "detail" && app.tacsim.detailModal}
        <header class="mhead">
          <h2>{app.tacsim.detailModal.title}</h2>
          <button type="button" class="ghost" onclick={() => app.tacsim.closeModal()}>✕</button>
        </header>
        <p class="prose">{app.tacsim.detailModal.body}</p>
      {/if}

      {#if app.tacsim.modalLog.length > 0}
        <div class="modal-log" aria-label="terminal output">
          {#each app.tacsim.modalLog as ln, idx (idx)}
            <div>{ln}</div>
          {/each}
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .tac-backdrop {
    position: fixed;
    inset: 0;
    background: rgb(0 0 0 / 62%);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 48;
    padding: 1rem;
  }

  .tac-modal {
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 1rem 1.2rem;
    width: min(34rem, calc(100vw - 2rem));
    max-height: 86vh;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    box-shadow: 0 12px 36px rgb(0 0 0 / 55%);
  }

  .tac-modal.wide {
    width: min(56rem, calc(100vw - 2rem));
  }

  .mhead {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 0.75rem;
    flex-wrap: wrap;
  }

  .mhead h2 {
    margin: 0;
    font-size: 0.98rem;
    letter-spacing: 0.03em;
  }

  .sub {
    font-size: 0.8rem;
    color: var(--fg-muted);
  }

  .cr {
    color: #66eeaa;
    font-family: var(--font-mono);
  }

  .tab-switch {
    display: flex;
    gap: 0.35rem;
    flex-wrap: wrap;
  }

  .tbtn {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--fg-muted);
    font: inherit;
    font-size: 0.8rem;
    padding: 0.25rem 0.65rem;
    cursor: pointer;
  }

  .tbtn.active {
    background: #154566;
    border-color: #3da4e0;
    color: #fff;
  }

  .gauges {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(10.5rem, 1fr));
    gap: 0.5rem;
    padding: 0.55rem;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 8px;
  }

  .gauge {
    display: flex;
    flex-direction: column;
    gap: 0.22rem;
  }

  .gtop {
    display: flex;
    justify-content: space-between;
    font-size: 0.73rem;
    font-family: var(--font-mono);
    color: var(--fg-muted);
  }

  .gbar {
    height: 7px;
    background: var(--bg-input);
    border-radius: 999px;
    overflow: hidden;
  }

  .gfill {
    height: 100%;
  }

  .gfill.mass { background: #3da8f5; }
  .gfill.vol { background: #9b72f2; }
  .gfill.tank { background: #3dd68c; }
  .gfill.stores { background: #f5b642; }

  .qty-row, .act-row, .foot-row {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.4rem;
  }

  .qchip {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 999px;
    color: var(--fg-muted);
    font-family: var(--font-mono);
    font-size: 0.74rem;
    padding: 0.14rem 0.58rem;
    cursor: pointer;
  }

  .qchip.on {
    background: #174c70;
    border-color: #44b0ef;
    color: #fff;
  }

  .tables {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(22rem, 1fr));
    gap: 0.75rem;
  }

  .tcol h3, .sailing-body h3 {
    margin: 0 0 0.4rem 0;
    font-size: 0.84rem;
    color: var(--fg-muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .rows {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    max-height: 16rem;
    overflow-y: auto;
  }

  .lrow {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    padding: 0.4rem 0.55rem;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
  }

  .lmeta {
    display: flex;
    flex-direction: column;
    font-size: 0.82rem;
  }

  .lact {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    flex: none;
  }

  .price {
    font-family: var(--font-mono);
    font-size: 0.8rem;
    color: #77d8ff;
  }

  .price.bid {
    color: #ffd077;
  }

  .buybtn, .sellbtn, .ghost {
    border-radius: 6px;
    font: inherit;
    font-size: 0.78rem;
    padding: 0.28rem 0.65rem;
    cursor: pointer;
  }

  .buybtn {
    background: #165942;
    border: 1px solid #35b884;
    color: #eafff5;
  }

  .sellbtn {
    background: #5c3818;
    border: 1px solid #d4883a;
    color: #fff5e8;
  }

  .ghost {
    background: none;
    border: 1px solid var(--border);
    color: var(--fg-muted);
  }

  .ghost.sm {
    padding: 0.22rem 0.45rem;
  }

  .muted {
    color: var(--fg-muted);
    font-size: 0.78rem;
  }

  .prose {
    margin: 0;
    font-size: 0.88rem;
    line-height: 1.45;
    color: var(--fg);
  }

  .form-row {
    display: flex;
    gap: 0.45rem;
  }

  .form-row input {
    flex: 1;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--fg);
    font: inherit;
    font-size: 0.86rem;
    padding: 0.38rem 0.6rem;
  }

  .topic-list {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
  }

  .topic-btn {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 6px;
    color: #9ad8ff;
    font: inherit;
    font-size: 0.8rem;
    padding: 0.3rem 0.65rem;
    cursor: pointer;
  }

  .dest-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(10rem, 1fr));
    gap: 0.45rem;
    margin-bottom: 0.65rem;
  }

  .dest-card {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 0.15rem;
    padding: 0.5rem 0.65rem;
    border-radius: 7px;
    border: 1px solid var(--border);
    background: var(--bg);
    color: var(--fg);
    font: inherit;
    font-size: 0.8rem;
    cursor: pointer;
    text-align: left;
  }

  .dest-card span {
    font-size: 0.72rem;
    color: var(--fg-muted);
  }

  .dest-card.on {
    background: #144262;
    border-color: #44b2f0;
  }

  .sim-roster {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
  }

  .slot-chip {
    font-family: var(--font-mono);
    font-size: 0.75rem;
    padding: 0.15rem 0.5rem;
    border-radius: 6px;
    border: 1px solid var(--border);
  }

  .slot-chip.red { color: #ff8877; border-color: #883328; }
  .slot-chip.blue { color: #77bbff; border-color: #285588; }

  .modal-log {
    background: #070d16;
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 0.45rem 0.6rem;
    font-family: var(--font-mono);
    font-size: 0.75rem;
    color: #b8d8f0;
    max-height: 7.5rem;
    overflow-y: auto;
  }
</style>
