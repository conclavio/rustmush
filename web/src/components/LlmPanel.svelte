<script lang="ts">
  // The staff-only LLM cost dashboard: today's cost/tokens by feature and by
  // provider (bars), budget-vs-cap meters, top spenders, and a recent-calls
  // table. Pull-only (no server-pushed delta — see `LlmDashboardStore`'s
  // doc): this effect fetches on mount and lightly polls while the tab
  // stays open, exactly like `SheetPanel`'s poll; the head button is a
  // manual refresh. A non-staff viewer sees a plain "staff only" message
  // instead — the client only knows the panel is *advertised*
  // (`SessionStore.llmDashboardAvailable`), never the viewer's role, so the
  // server's refusal is what actually gates this (see
  // `LlmDashboardStore.markRefused`'s doc and the worldgen module's
  // `LlmDashboardQuery`).
  import type { GameApp } from "$lib/game.svelte";
  import {
    budgetMeterPct,
    budgetOverCap,
    featureLabel,
    formatCost,
    formatTokens,
    isFeatureBudget,
    latestDay,
    providerTotalsForDay,
    relativeBarPct,
    scopeLabel,
    type BudgetStatus,
  } from "$lib/llm";

  let { app }: { app: GameApp } = $props();

  const llm = $derived(app.llm);

  const POLL_MS = 20_000;
  $effect(() => {
    if (!app.session.llmDashboardAvailable) {
      return;
    }
    app.refreshLlm();
    const id = setInterval(() => app.refreshLlm(), POLL_MS);
    return () => clearInterval(id);
  });

  const globalBudget = $derived(
    (llm.data?.budgets ?? []).find((b) => b.scope === "global") ?? null,
  );
  const featureRows = $derived(
    (llm.data?.budgets ?? [])
      .filter(isFeatureBudget)
      .slice()
      .sort((a, b) => b.costUsd - a.costUsd),
  );
  const otherBudgetRows = $derived(
    (llm.data?.budgets ?? []).filter((b) => b.scope !== "global" && !isFeatureBudget(b)),
  );
  const featureMaxCost = $derived(featureRows.reduce((m, r) => Math.max(m, r.costUsd), 0));

  const providerRows = $derived(
    providerTotalsForDay(llm.data?.dailySeries ?? [], latestDay(llm.data?.dailySeries ?? [])),
  );
  const providerMaxCost = $derived(providerRows.reduce((m, r) => Math.max(m, r.costUsd), 0));
</script>

{#snippet meterRow(row: BudgetStatus)}
  {@const pct = budgetMeterPct(row)}
  {@const over = budgetOverCap(row)}
  <div class="meterrow">
    <div class="meterhead">
      <span class="meterlabel">{scopeLabel(row.scope)}</span>
      <span class="metertext muted small">
        {formatTokens(row.inputTokens + row.outputTokens)}{#if row.tokensCap !== null}/{formatTokens(
            row.tokensCap,
          )}{/if} tok · {formatCost(row.costUsd)}{#if row.costCap !== null}/{formatCost(
            row.costCap,
          )}{/if}
      </span>
    </div>
    {#if pct !== null}
      <div class="meter">
        <span class="meterfill" class:over style={`width:${pct}%`}></span>
      </div>
    {:else}
      <p class="nocap muted small">no cap configured</p>
    {/if}
  </div>
{/snippet}

{#snippet barRow(label: string, cost: number, maxCost: number, tokens: number)}
  {@const pct = relativeBarPct(cost, maxCost)}
  <div class="barrow">
    <span class="barlabel" title={label}>{label}</span>
    <div class="bar"><span class="barfill" style={`width:${pct}%`}></span></div>
    <span class="bartext muted small">{formatCost(cost)} · {formatTokens(tokens)} tok</span>
  </div>
{/snippet}

<div class="llm">
  <div class="head">
    <strong>LLM dashboard</strong>
    {#if llm.data}
      <span class="killbadge" data-engaged={llm.data.killSwitchEngaged}>
        {llm.data.killSwitchEngaged ? "DISABLED" : "live"}
      </span>
    {/if}
    <span class="grow"></span>
    <button
      type="button"
      class="refresh"
      title="Refresh"
      aria-label="refresh LLM dashboard"
      disabled={llm.refreshing}
      onclick={() => app.refreshLlm()}
    >
      {llm.refreshing ? "…" : "↻"}
    </button>
  </div>

  <div class="body">
    {#if llm.refused}
      <div class="empty">
        <p><strong>Staff only</strong></p>
        <p class="muted">The LLM cost dashboard is restricted to Royalty-or-better staff.</p>
      </div>
    {:else if llm.unavailable}
      <div class="empty">
        <p class="muted">
          Not available on this server yet.
          <button type="button" class="linklike" onclick={() => app.refreshLlm()}>Try again</button>
        </p>
      </div>
    {:else if llm.data === null}
      <p class="muted loading">{llm.refreshing ? "Loading…" : "No data yet."}</p>
    {:else}
      {@const d = llm.data}

      <section class="block">
        <h3>Budgets today (used / cap)</h3>
        {#if globalBudget}
          {@render meterRow(globalBudget)}
        {/if}
        {#each featureRows as row (row.scope)}
          {@render meterRow(row)}
        {/each}
        {#each otherBudgetRows as row (row.scope)}
          {@render meterRow(row)}
        {/each}
        {#if globalBudget === null && featureRows.length === 0 && otherBudgetRows.length === 0}
          <p class="muted small">No budgets configured.</p>
        {/if}
      </section>

      {#if featureRows.length > 0}
        <section class="block">
          <h3>Today by feature</h3>
          {#each featureRows as row (row.scope)}
            {@render barRow(
              featureLabel(row.scope),
              row.costUsd,
              featureMaxCost,
              row.inputTokens + row.outputTokens,
            )}
          {/each}
        </section>
      {/if}

      {#if providerRows.length > 0}
        <section class="block">
          <h3>Today by provider</h3>
          {#each providerRows as row (row.provider)}
            {@render barRow(
              row.provider,
              row.costUsd,
              providerMaxCost,
              row.inputTokens + row.outputTokens,
            )}
          {/each}
        </section>
      {/if}

      {#if d.topActors.length > 0 || d.topNpcs.length > 0}
        <section class="block">
          <h3>Top spenders today</h3>
          {#each d.topActors as row (row.obj.id)}
            <div class="spendrow">
              <span class="spendname">{row.obj.name}</span>
              <span class="spendcost">{formatCost(row.costUsd)}</span>
            </div>
          {/each}
          {#each d.topNpcs as row (row.obj.id)}
            <div class="spendrow">
              <span class="spendname">{row.obj.name} <span class="muted small">NPC</span></span>
              <span class="spendcost">{formatCost(row.costUsd)}</span>
            </div>
          {/each}
        </section>
      {/if}

      <section class="block">
        <h3>Recent calls</h3>
        {#if d.recentCalls.length === 0}
          <p class="muted small">No calls logged yet.</p>
        {:else}
          <div class="tablewrap">
            <table>
              <thead>
                <tr>
                  <th>Feature</th>
                  <th>Provider</th>
                  <th>Actor/NPC</th>
                  <th class="num">In</th>
                  <th class="num">Out</th>
                  <th class="num">Cost</th>
                  <th class="num">ms</th>
                  <th>Outcome</th>
                </tr>
              </thead>
              <tbody>
                {#each d.recentCalls as c, i (i)}
                  <tr>
                    <td>{c.feature}</td>
                    <td class="muted">{c.provider}</td>
                    <td>{c.actor?.name ?? c.npc?.name ?? "—"}</td>
                    <td class="num">{formatTokens(c.inputTokens)}</td>
                    <td class="num">{formatTokens(c.outputTokens)}</td>
                    <td class="num">{c.costUsd !== null ? formatCost(c.costUsd) : "—"}</td>
                    <td class="num">{c.latencyMs}</td>
                    <td><span class="outcome" data-outcome={c.outcome}>{c.outcome}</span></td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
        {/if}
      </section>
    {/if}
  </div>
</div>

<style>
  .llm {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }

  .head {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.5rem 0.75rem;
    border-bottom: 1px solid var(--border);
  }

  .grow {
    flex: 1;
  }

  .killbadge {
    display: inline-flex;
    align-items: center;
    border-radius: 999px;
    padding: 0.05rem 0.55rem;
    font-size: 0.68rem;
    font-weight: 700;
    letter-spacing: 0.04em;
    color: var(--success);
    border: 1px solid var(--success);
  }

  .killbadge[data-engaged="true"] {
    color: var(--error);
    border-color: var(--error);
  }

  .refresh {
    background: none;
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--fg-muted);
    cursor: pointer;
    font: inherit;
    padding: 0.05rem 0.5rem;
  }

  .refresh:hover:not(:disabled) {
    color: var(--fg);
  }

  .refresh:disabled {
    opacity: 0.6;
    cursor: default;
  }

  .body {
    flex: 1;
    overflow-y: auto;
    padding: 0.6rem 0.75rem 1rem;
    font-size: 0.85rem;
    display: flex;
    flex-direction: column;
    gap: 0.7rem;
  }

  .empty,
  .loading {
    padding: 0.5rem 0.1rem;
  }

  .empty p {
    margin: 0 0 0.4rem;
  }

  .muted {
    color: var(--fg-muted);
  }

  .small {
    font-size: 0.78rem;
  }

  .linklike {
    background: none;
    border: none;
    color: var(--accent);
    cursor: pointer;
    font: inherit;
    padding: 0;
    text-decoration: underline;
  }

  .block h3 {
    margin: 0 0 0.4rem;
    font-size: 0.78rem;
    letter-spacing: 0.03em;
    text-transform: uppercase;
    color: var(--fg-muted);
  }

  /* Budget meters */
  .meterrow {
    margin-bottom: 0.5rem;
  }

  .meterrow:last-child {
    margin-bottom: 0;
  }

  .meterhead {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 0.5rem;
    margin-bottom: 0.15rem;
  }

  .meterlabel {
    color: var(--fg);
    font-weight: 600;
    overflow-wrap: anywhere;
  }

  .metertext {
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }

  .meter {
    height: 0.45rem;
    border-radius: 999px;
    background: var(--bg-input);
    overflow: hidden;
  }

  .meterfill {
    display: block;
    height: 100%;
    background: var(--accent);
  }

  .meterfill.over {
    background: var(--error);
  }

  .nocap {
    margin: 0;
  }

  /* By-feature / by-provider bars */
  .barrow {
    display: grid;
    grid-template-columns: 6.5rem 1fr;
    align-items: center;
    gap: 0.4rem 0.5rem;
    margin-bottom: 0.35rem;
  }

  .barrow:last-child {
    margin-bottom: 0;
  }

  .barlabel {
    color: var(--fg);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 0.8rem;
  }

  .bar {
    grid-column: 2;
    height: 0.5rem;
    border-radius: 999px;
    background: var(--bg-input);
    overflow: hidden;
  }

  .barfill {
    display: block;
    height: 100%;
    background: var(--accent-dim);
  }

  .bartext {
    grid-column: 2;
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }

  /* Top spenders */
  .spendrow {
    display: flex;
    justify-content: space-between;
    gap: 0.5rem;
    padding: 0.12rem 0;
  }

  .spendname {
    color: var(--fg);
    overflow-wrap: anywhere;
  }

  .spendcost {
    color: var(--fg-muted);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  /* Recent calls table */
  .tablewrap {
    overflow-x: auto;
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.78rem;
  }

  th {
    text-align: left;
    color: var(--fg-muted);
    font-weight: 500;
    padding: 0.15rem 0.4rem 0.15rem 0;
    border-bottom: 1px solid var(--border);
    white-space: nowrap;
  }

  td {
    padding: 0.2rem 0.4rem 0.2rem 0;
    vertical-align: baseline;
    white-space: nowrap;
  }

  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }

  .outcome {
    display: inline-block;
    border-radius: 4px;
    padding: 0 0.35rem;
    font-size: 0.72rem;
    font-weight: 600;
    color: var(--fg-muted);
    border: 1px solid var(--border);
  }

  .outcome[data-outcome="ok"] {
    color: var(--success);
    border-color: var(--success);
  }

  .outcome[data-outcome="refused"],
  .outcome[data-outcome="error"] {
    color: var(--error);
    border-color: var(--error);
  }

  .outcome[data-outcome="budget"],
  .outcome[data-outcome="disabled"] {
    color: var(--warn);
    border-color: var(--warn);
  }
</style>
