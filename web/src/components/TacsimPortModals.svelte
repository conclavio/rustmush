<script lang="ts">
  // Diegetic Svelte 5 Modals for Corridor Port Trade, Sailing Plans, Co-op Chartering,
  // NPC Conversations, Prospector Rigs, and the OOC 6v6 Simulator Console.
  // Preserves `Game.test.ts`'s single `"command input"` invariant by naming every
  // modal field distinctly.

  import type { GameApp } from "$lib/game.svelte";
  import { matchesQuestTradeRow } from "$lib/stores/quests.svelte";
  import {
    formatEscortPostCommand,
    formatHaulPostCommand,
    formatJobCommand,
    formatMarketDepthCommand,
    formatMarketHistoryCommand,
    formatSurveyAcceptCommand,
    HISTORY_GOODS,
    JOB_BOARD_FILTERS,
    parseMarketHistoryLines,
    SIM_UNIT_CHASSIS,
    SURVEY_OFFERS,
    WORKS_RECIPES,
    YARD_RECIPES,
  } from "$lib/tacsim/economy";
  import {
    buildNpcTopics,
    computeMaxFitQuantity,
    CORRIDOR_DESTINATIONS,
    type PortListingRow,
  } from "$lib/tacsim/trade";

  let { app, onCmd }: { app: GameApp; onCmd: (cmd: string) => void } = $props();

  type TradeSubView = "book" | "history" | "jobs" | "industry";

  let modalEl: HTMLDivElement | undefined = $state();
  let npcSayDraft = $state("");
  let hullNameDraft = $state("");
  let tradeSubView = $state<TradeSubView>("book");
  let historyGood = $state<string>("volatiles");
  let activeJobFilter = $state<string>("all");
  let jobIdDraft = $state<string>("");
  let surveyIdDraft = $state<string>(SURVEY_OFFERS[0]!.id);
  let haulQtyDraft = $state<number>(10);
  let haulFeeDraft = $state<number>(250);
  let workOrderIdDraft = $state<string>("1");
  let debtIdDraft = $state<string>("1");

  $effect(() => {
    if (app.tacsim.activeModal !== null) {
      modalEl?.focus();
    }
    if (app.tacsim.activeModal === "coop") {
      hullNameDraft = app.tacsim.coopSuggestedHull;
    } else if (app.tacsim.activeModal === "npc") {
      npcSayDraft = "";
    } else if (app.tacsim.activeModal === "trade") {
      tradeSubView = "book";
    }
  });

  function onWindowKeyDown(ev: KeyboardEvent): void {
    if (app.tacsim.activeModal !== null && ev.key === "Escape") {
      ev.preventDefault();
      ev.stopPropagation();
      app.tacsim.closeModal();
    }
  }

  const parsedHistory = $derived(parseMarketHistoryLines(app.tacsim.marketHistoryLines));
  const escortHullRef = $derived(app.tacsim.activeHullRef);
  const escortPostLine = $derived(
    formatEscortPostCommand(
      escortHullRef,
      app.tacsim.selectedDestination,
      haulFeeDraft,
      true,
    ),
  );

  const modal = $derived(app.tacsim.activeModal);
  const trade = $derived(app.tacsim.trade);
  const st = $derived(trade.stowage);
  const questTradeNeed = $derived(app.quests.activeTradeNeed);
  const questDestPort = $derived(app.quests.activeDestinationPort);
  const bayHullTarget = $derived(
    app.tacsim.activeHullRef ||
      app.tacsim.ambient.ownHullName ||
      app.tacsim.ambient.hullName ||
      "ship",
  );

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

<svelte:window onkeydown={onWindowKeyDown} />

{#if modal !== null}
  <div
    class="tac-backdrop"
    role="presentation"
    onclick={() => app.tacsim.closeModal()}
  >
    <div
      bind:this={modalEl}
      class="tac-modal"
      class:wide={modal === "trade" || modal === "sailing" || modal === "sim"}
      role="dialog"
      aria-label="port terminal dialog"
      tabindex="-1"
      onclick={(ev) => ev.stopPropagation()}
      onkeydown={(ev) => {
        if (ev.key === "Escape") {
          ev.preventDefault();
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
              class:active={modal === "trade" && tradeSubView === "book"}
              onclick={() => {
                app.tacsim.activeModal = "trade";
                tradeSubView = "book";
              }}
            >
              Commodity Exchange
            </button>
            <button
              type="button"
              class="tbtn"
              class:active={modal === "trade" && tradeSubView === "history"}
              onclick={() => {
                app.tacsim.activeModal = "trade";
                tradeSubView = "history";
                onCmd(formatMarketHistoryCommand(historyGood));
              }}
            >
              24-Week History
            </button>
            <button
              type="button"
              class="tbtn"
              class:active={modal === "trade" && tradeSubView === "jobs"}
              onclick={() => {
                app.tacsim.activeModal = "trade";
                tradeSubView = "jobs";
                onCmd("+market/jobs");
              }}
            >
              Jobs & Crew
            </button>
            <button
              type="button"
              class="tbtn"
              class:active={modal === "trade" && tradeSubView === "industry"}
              onclick={() => {
                app.tacsim.activeModal = "trade";
                tradeSubView = "industry";
                onCmd("+works");
              }}
            >
              Works & Yard
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

        {#if modal === "trade" && tradeSubView === "book"}
          {#if questTradeNeed}
            <div class="quest-banner" role="status" aria-label="active quest trade guidance">
              <div class="qb-text">
                <span class="qtag">[QUEST]</span>
                <strong>
                  {questTradeNeed.wantsBuy && questTradeNeed.wantsSell
                    ? "BUY / SELL"
                    : questTradeNeed.wantsSell
                      ? "SELL"
                      : "BUY"}
                  {questTradeNeed.targetQuantity} × {questTradeNeed.commoditySlug.replaceAll("_", " ").toUpperCase()}
                </strong>
                <span class="muted">— {questTradeNeed.questTitle}</span>
              </div>
              <button
                type="button"
                class="qbtn-gold"
                onclick={() => (app.tacsim.selectedQtyPreset = questTradeNeed.targetQuantity)}
              >
                Set Quest Qty ({questTradeNeed.targetQuantity})
              </button>
            </div>
          {/if}

          <div class="qty-row">
            <span class="muted">Lot Quantity:</span>
            {#each [1, 5, 10, 25, "max"] as const as q (q)}
              <button
                type="button"
                class="qchip"
                class:on={app.tacsim.selectedQtyPreset === q}
                onclick={() => (app.tacsim.selectedQtyPreset = q)}
              >
                {q === "max" ? "Max Fit" : `${q}`}
              </button>
            {/each}
            <div class="book-acct-btns">
              <button type="button" class="ghost sm" onclick={() => onCmd("+market/mine")}>My Listings</button>
              <button type="button" class="ghost sm" onclick={() => onCmd("+market/orders")}>Open Quotes</button>
              <button type="button" class="ghost sm" onclick={() => onCmd("+locker")}>Check Locker</button>
              <button type="button" class="ghost sm" onclick={() => onCmd("+locker/open")}>Open Locker</button>
              <button type="button" class="ghost sm" onclick={() => onCmd("+debts")}>Arrears & Debts</button>
            </div>
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
                    {@const isQuestMatch = matchesQuestTradeRow(questTradeNeed, row, false)}
                    <div class="lrow" class:quest-match={isQuestMatch}>
                      <div class="lmeta">
                        <strong>
                          {#if isQuestMatch}<span class="qtag">[QUEST]</span> {/if}{row.itemName}
                        </strong>
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
                    {@const isQuestMatch = matchesQuestTradeRow(questTradeNeed, row, true)}
                    <div class="lrow" class:quest-match={isQuestMatch}>
                      <div class="lmeta">
                        <strong>
                          {#if isQuestMatch}<span class="qtag">[QUEST]</span> {/if}{row.itemName}
                        </strong>
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

        {:else if modal === "trade" && tradeSubView === "history"}
          <div class="econ-pane">
            <div class="qty-row">
              <span class="muted">Commodity Series:</span>
              {#each HISTORY_GOODS as g (g.code)}
                <button
                  type="button"
                  class="qchip"
                  class:on={historyGood === g.code}
                  onclick={() => {
                    historyGood = g.code;
                    onCmd(formatMarketHistoryCommand(g.code));
                  }}
                >
                  {g.label}
                </button>
              {/each}
            </div>

            <div class="act-row">
              <button
                type="button"
                class="buybtn"
                onclick={() => onCmd(formatMarketHistoryCommand(historyGood))}
              >
                Load 24-Week Trend ({formatMarketHistoryCommand(historyGood)})
              </button>
              <button
                type="button"
                class="ghost"
                onclick={() => onCmd(formatMarketDepthCommand(historyGood))}
              >
                Public Depth ({formatMarketDepthCommand(historyGood)})
              </button>
              <button type="button" class="ghost" onclick={() => onCmd("+debts")}>
                Check Debts (+debts)
              </button>
              <input
                type="text"
                class="mini-input"
                aria-label="debt obligation id"
                bind:value={debtIdDraft}
                placeholder="Debt #"
              />
              <button
                type="button"
                class="sellbtn"
                onclick={() => onCmd(`+debts/pay ${debtIdDraft.trim() || "1"}`)}
              >
                Pay Debt
              </button>
            </div>

            {#if parsedHistory.rowsNewestFirst.length > 0}
              <div class="spark-grid">
                <div class="spark-card">
                  <span class="muted">House Ask Trend ({parsedHistory.goodLabel ?? historyGood})</span>
                  <div class="spark-line ask">{parsedHistory.askSparkline}</div>
                  <span class="muted">Latest: <strong>{parsedHistory.latestAsk}</strong> · Band: {parsedHistory.askRange}</span>
                </div>
                <div class="spark-card">
                  <span class="muted">House Bid Trend ({parsedHistory.goodLabel ?? historyGood})</span>
                  <div class="spark-line bid">{parsedHistory.bidSparkline}</div>
                  <span class="muted">Latest: <strong>{parsedHistory.latestBid}</strong> · Band: {parsedHistory.bidRange}</span>
                </div>
              </div>
              <div class="rows">
                {#each parsedHistory.rowsNewestFirst as hr, i (i)}
                  <div class="lrow">
                    <span class="muted">{hr.when}</span>
                    <div class="lact">
                      <span class="price">Ask: {hr.askText}</span>
                      <span class="price bid">Bid: {hr.bidText}</span>
                    </div>
                  </div>
                {/each}
              </div>
            {:else}
              <p class="muted">
                Select a commodity above to query the venue's 24 IC-week house ask and bid series.
              </p>
            {/if}
          </div>

        {:else if modal === "trade" && tradeSubView === "jobs"}
          <div class="econ-pane">
            <div class="qty-row">
              <span class="muted">Board Filter:</span>
              {#each JOB_BOARD_FILTERS as jf (jf.id)}
                <button
                  type="button"
                  class="qchip"
                  class:on={activeJobFilter === jf.id}
                  onclick={() => {
                    activeJobFilter = jf.id;
                    onCmd(jf.cmd);
                  }}
                >
                  {jf.label}
                </button>
              {/each}
              <button type="button" class="ghost sm" onclick={() => onCmd("+sortie/crew")}>
                Crew Terms (+sortie/crew)
              </button>
              <button type="button" class="ghost sm" onclick={() => onCmd("+sortie/contract")}>
                My Crew (+sortie/contract)
              </button>
            </div>

            <div class="tables">
              <section class="tcol">
                <h3>Accept or Deliver Open Job</h3>
                <div class="act-row">
                  <input
                    type="text"
                    class="mini-input"
                    aria-label="job contract id"
                    bind:value={jobIdDraft}
                    placeholder="Job #"
                  />
                  {#each [
                    { label: "Accept Haul", cls: "buybtn", verb: "+haul/accept", go: true },
                    { label: "Deliver Haul", cls: "sellbtn", verb: "+haul/deliver", go: true },
                  ] as job (job.label)}
                    {@const line = formatJobCommand(job.verb, jobIdDraft, job.go)}
                    <button
                      type="button"
                      class={job.cls}
                      disabled={line === null}
                      title={line ?? "Type a job id from the board first."}
                      onclick={() => {
                        if (line !== null) onCmd(line);
                      }}
                    >
                      {job.label}
                    </button>
                  {/each}
                </div>
                <div class="act-row">
                  <select
                    class="mini-select"
                    aria-label="survey commission id"
                    bind:value={surveyIdDraft}
                  >
                    {#each SURVEY_OFFERS as s (s.id)}
                      <option value={s.id}>{s.label}</option>
                    {/each}
                  </select>
                  <button
                    type="button"
                    class="ghost sm"
                    title={formatSurveyAcceptCommand(surveyIdDraft, false)}
                    onclick={() => onCmd(formatSurveyAcceptCommand(surveyIdDraft, false))}
                  >
                    Preview Survey
                  </button>
                  <button
                    type="button"
                    class="buybtn"
                    title={formatSurveyAcceptCommand(surveyIdDraft, true)}
                    onclick={() => onCmd(formatSurveyAcceptCommand(surveyIdDraft, true))}
                  >
                    Accept Survey
                  </button>
                </div>
                <div class="act-row">
                  <button type="button" class="ghost sm" onclick={() => onCmd("+haul")}>My Hauls (+haul)</button>
                  <button type="button" class="ghost sm" onclick={() => onCmd("+escort")}>My Escorts (+escort)</button>
                  <button type="button" class="ghost sm" onclick={() => onCmd("+survey")}>Surveys (+survey)</button>
                  <button type="button" class="ghost sm" onclick={() => onCmd("+salvage/jobs")}>Retrieval (+salvage/jobs)</button>
                  <button type="button" class="ghost sm" onclick={() => onCmd("+passage")}>Passage (+passage)</button>
                </div>
              </section>

              <section class="tcol">
                <h3>Post Shipper Haul or Convoy Escort</h3>
                <div class="act-row">
                  <input
                    type="number"
                    class="mini-input"
                    aria-label="haul quantity tonnes"
                    bind:value={haulQtyDraft}
                    min="1"
                  />
                  <input
                    type="number"
                    class="mini-input"
                    aria-label="job fee credits"
                    bind:value={haulFeeDraft}
                    min="1"
                  />
                  <button
                    type="button"
                    class="buybtn"
                    onclick={() =>
                      onCmd(
                        formatHaulPostCommand(
                          haulQtyDraft,
                          historyGood,
                          app.tacsim.selectedDestination,
                          haulFeeDraft,
                          true,
                        ),
                      )}
                  >
                    Post Haul ({historyGood} → {app.tacsim.selectedDestination})
                  </button>
                  <button
                    type="button"
                    class="sellbtn"
                    disabled={escortPostLine === null}
                    title={escortPostLine === null
                      ? "No hull at your helm to escort (requires #<dbref>)."
                      : escortPostLine}
                    onclick={() => {
                      if (escortPostLine !== null) onCmd(escortPostLine);
                    }}
                  >
                    Post Escort ({escortHullRef ?? "no hull"} → {app.tacsim.selectedDestination})
                  </button>
                </div>
              </section>
            </div>
          </div>

        {:else if modal === "trade" && tradeSubView === "industry"}
          <div class="econ-pane">
            <div class="act-row">
              <button type="button" class="ghost sm" onclick={() => onCmd("+works")}>Works Status (+works)</button>
              <button type="button" class="ghost sm" onclick={() => onCmd("+yard")}>Shipyard Status (+yard)</button>
              <button
                type="button"
                class="ghost sm"
                onclick={() => onCmd(`+bay ${bayHullTarget}`)}
              >
                Bay & Magazines (+bay)
              </button>
              <button
                type="button"
                class="ghost sm"
                onclick={() => onCmd(`+bay/order ${bayHullTarget}=rearm`)}
              >
                Rearm Magazines (+bay/order =rearm)
              </button>
              <button
                type="button"
                class="ghost sm"
                onclick={() => onCmd(`+bay/order ${bayHullTarget}=all`)}
              >
                Full Repair (+bay/order =all)
              </button>
              <button type="button" class="ghost sm" onclick={() => onCmd("+core")}>Core Shop (+core)</button>
              <button type="button" class="ghost sm" onclick={() => onCmd("@facility/list")}>Facilities (@facility/list)</button>
            </div>

            <div class="act-row">
              <span class="muted">Active Order #:</span>
              <input
                type="text"
                class="mini-input"
                aria-label="work or yard order id"
                bind:value={workOrderIdDraft}
                placeholder="Order #"
              />
              <button
                type="button"
                class="buybtn"
                onclick={() => onCmd(`+works/buy ${workOrderIdDraft.trim() || "1"}`)}
              >
                Buy Works Bill (+works/buy)
              </button>
              <button
                type="button"
                class="sellbtn"
                onclick={() => onCmd(`+works/collect ${workOrderIdDraft.trim() || "1"}`)}
              >
                Collect Works (+works/collect)
              </button>
              <button
                type="button"
                class="buybtn"
                onclick={() => onCmd(`+yard/buy ${workOrderIdDraft.trim() || "1"}`)}
              >
                Buy Yard Bill (+yard/buy)
              </button>
              <button
                type="button"
                class="sellbtn"
                onclick={() => onCmd(`+yard/collect ${workOrderIdDraft.trim() || "1"}`)}
              >
                Commission Hull (+yard/collect)
              </button>
            </div>

            <div class="tables">
              <section class="tcol">
                <h3>Port Works — Milling, Munitions & Core Modules</h3>
                <div class="rows">
                  {#each WORKS_RECIPES as r (r.id)}
                    <div class="lrow">
                      <div class="lmeta">
                        <strong>{r.label}</strong>
                        <span class="muted">{r.inputsSummary} → {r.outputSummary}</span>
                      </div>
                      <div class="lact">
                        <button type="button" class="ghost sm" onclick={() => onCmd(`+works/quote ${r.id}`)}>
                          Quote
                        </button>
                        <button type="button" class="buybtn" onclick={() => onCmd(`+works/order ${r.id}`)}>
                          Book
                        </button>
                      </div>
                    </div>
                  {/each}
                </div>
              </section>

              <section class="tcol">
                <h3>Shipyard — Light & Heavy Hull Commissioning</h3>
                <div class="rows">
                  {#each YARD_RECIPES as y (y.id)}
                    <div class="lrow">
                      <div class="lmeta">
                        <strong>{y.label}</strong>
                        <span class="muted">{y.inputsSummary} · {y.outputSummary}</span>
                      </div>
                      <div class="lact">
                        <button type="button" class="ghost sm" onclick={() => onCmd(`+yard/quote ${y.id}`)}>
                          Quote
                        </button>
                        <button type="button" class="sellbtn" onclick={() => onCmd(`+yard/order ${y.id}`)}>
                          Order Hull
                        </button>
                      </div>
                    </div>
                  {/each}
                </div>
              </section>
            </div>
          </div>
        {:else}
          <div class="sailing-body">
            {#if questDestPort}
              <div class="quest-banner" role="status" aria-label="active quest destination">
                <div class="qb-text">
                  <span class="qtag">[QUEST]</span>
                  <strong>DESTINATION: {questDestPort.portLabel.toUpperCase()}</strong>
                  <span class="muted">— {questDestPort.questTitle}</span>
                </div>
                <button
                  type="button"
                  class="qbtn-gold"
                  onclick={() => (app.tacsim.selectedDestination = questDestPort.portId)}
                >
                  Select Quest Port ({questDestPort.portId})
                </button>
              </div>
            {/if}
            <h3>Select Corridor Destination Port</h3>
            <div class="dest-grid">
              {#each CORRIDOR_DESTINATIONS as dest (dest.id)}
                {@const isQuestDest = questDestPort?.portId === dest.id}
                <button
                  type="button"
                  class="dest-card"
                  class:on={app.tacsim.selectedDestination === dest.id}
                  class:quest-match={isQuestDest}
                  onclick={() => (app.tacsim.selectedDestination = dest.id)}
                >
                  <strong>{#if isQuestDest}<span class="qtag">[QUEST]</span> {/if}{dest.label}</strong>
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
        {@const npcQuestMarker = app.quests.roomMarkers.find((m) => m.npcId === npc.id) ?? null}
        {@const npcHasQuest =
          npcQuestMarker !== null ||
          app.quests.available.some((q) => q.giverId === npc.id) ||
          app.quests.active.some((q) => q.giverId === npc.id || q.turnInId === npc.id)}
        <header class="mhead">
          <div>
            <h2>{npc.name.toUpperCase()}</h2>
            {#if npc.bio}<span class="sub">{npc.bio}</span>{/if}
          </div>
          <button type="button" class="ghost" onclick={() => app.tacsim.closeModal()}>✕</button>
        </header>
        {#if npcHasQuest}
          <div class="act-row">
            <button
              type="button"
              class={npcQuestMarker?.marker === "in_progress" ? "ghost" : "sellbtn"}
              onclick={() => {
                app.tacsim.closeModal();
                app.quests.openNpcDialog(npc.id);
              }}
            >
              {#if npcQuestMarker?.marker === "available"}
                [!] Open Quest Dialog{npcQuestMarker.questTitle ? `: ${npcQuestMarker.questTitle}` : ""}
              {:else if npcQuestMarker?.marker === "turn_in"}
                [?] Turn In Quest{npcQuestMarker.questTitle ? `: ${npcQuestMarker.questTitle}` : ""}
              {:else}
                [?] Open Quest Dialog
              {/if}
            </button>
          </div>
        {/if}
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
          <button type="button" class="sellbtn" onclick={() => onCmd("SIM TEAM red")}>Join Red Team</button>
          <button type="button" class="buybtn" onclick={() => onCmd("SIM TEAM blue")}>Join Blue Team</button>
          <button type="button" class="ghost" onclick={() => onCmd("SIM START")}>Start Match (SIM START)</button>
          <button type="button" class="ghost" onclick={() => onCmd("board")}>Board Pod (board)</button>
        </div>
        <div class="act-row">
          <span class="muted">Unit Chassis:</span>
          {#each SIM_UNIT_CHASSIS as u (u.id)}
            <button type="button" class="qchip" onclick={() => onCmd(`SIM UNIT ${u.id}`)}>{u.label}</button>
          {/each}
          <span class="muted">Match Mode:</span>
          {#each ["skirmish", "hold"] as mode (mode)}
            <button type="button" class="qchip" onclick={() => onCmd(`SIM MODE ${mode}`)}>{mode}</button>
          {/each}
          <button type="button" class="qchip" onclick={() => onCmd("SIM RANGE 1")}>Range 1</button>
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

  .book-acct-btns {
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem;
    margin-left: auto;
  }

  .econ-pane {
    display: flex;
    flex-direction: column;
    gap: 0.65rem;
  }

  .mini-input {
    width: 5.5rem;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--fg);
    font: inherit;
    font-family: var(--font-mono);
    font-size: 0.78rem;
    padding: 0.25rem 0.45rem;
  }

  .mini-select {
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--fg);
    font: inherit;
    font-family: var(--font-mono);
    font-size: 0.76rem;
    padding: 0.25rem 0.45rem;
  }

  .sellbtn:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }

  .spark-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(18rem, 1fr));
    gap: 0.6rem;
  }

  .spark-card {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    padding: 0.55rem 0.7rem;
    border-radius: 8px;
    border: 1px solid var(--border);
    background: var(--bg);
  }

  .spark-line {
    font-family: var(--font-mono);
    font-size: 1.05rem;
    letter-spacing: 0.08em;
  }

  .spark-line.ask {
    color: #66d8ff;
  }

  .spark-line.bid {
    color: #ffd077;
  }

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

  .quest-banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.6rem;
    flex-wrap: wrap;
    padding: 0.45rem 0.7rem;
    border-radius: 7px;
    border: 1px solid #d4a634;
    background: rgba(62, 46, 12, 0.9);
    color: #fff2c4;
    font-size: 0.8rem;
  }

  .qb-text {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    flex-wrap: wrap;
  }

  .qtag {
    color: #ffd84d;
    font-weight: 700;
    font-family: var(--font-mono);
  }

  .qbtn-gold {
    padding: 0.25rem 0.65rem;
    border-radius: 6px;
    border: 1px solid #ffd84d;
    background: #7a5410;
    color: #ffffff;
    font: inherit;
    font-size: 0.76rem;
    font-weight: 700;
    cursor: pointer;
  }

  .lrow.quest-match {
    border-color: #d4a634;
    background: rgba(62, 48, 14, 0.55);
  }

  .dest-card.quest-match {
    border-color: #d4a634;
    box-shadow: inset 0 0 0 1px rgba(255, 216, 77, 0.35);
  }
</style>
