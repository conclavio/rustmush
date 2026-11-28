<script lang="ts">
  // The sky chart screen (doc 19 cut A.3b; doc 19 §5): the `sky.chart` panel
  // drawn as a picture — the Sun at the centre, every body's orbit as a ring
  // at the server's own logarithmic radial scale, the bodies as discs with
  // their letters, the listed routes as lines from the port the chart is read
  // from, that port ringed, and (doc 21 cut G.5c) rings of 3, 6, 12 and 24
  // light-minutes about it, as the terminal's picture draws them — with the sky table, the passages and the window
  // beside it, and the server's flat character map under a toggle, so the
  // picture a web player sees and the map a TUI or line player sees are two
  // renders of ONE frame.
  //
  // Every string printed here comes out of `$lib/chart`'s pure helpers, which
  // vitest pins; nothing here decides a figure.
  import type { GameApp } from "$lib/game.svelte";
  import {
    bodyLines,
    bodyPoints,
    chartHeading,
    LAG_RING_MINUTES,
    lagRing,
    orbitRadius,
    routeLines,
    routePolyline,
    scaleCaption,
    windowLines,
    type ChartPoint,
    type ChartRoute,
  } from "$lib/chart";
  import { expandRows } from "$lib/stores/map.svelte";
  import {
    GUIDED_TOUR_KEYS,
    evaluateSailingPreflightReadiness,
    formatQuestHoldReadinessSummary,
    inferStationPortFromRoomEntry,
    resolveNextGuidedOfferDestination,
  } from "$lib/stores/quests.svelte";

  let { app }: { app: GameApp } = $props();

  const PORT_LABELS: Record<string, string> = {
    ceres: "Ceres Freeport",
    stickney: "Stickney Yards",
    luna: "Aitken Basin",
    deimos: "Voltaire Base",
    titan: "Ligeia Dock",
  };

  const frame = $derived(app.chart.frame);
  const chart = $derived(frame?.chart ?? null);
  const questMining = $derived(app.quests?.surfaceMiningStatus ?? null);
  const questStepNo = $derived.by((): number | null => {
    const key = app.quests?.trackedQuest?.key;
    if (!key) return null;
    const idx = GUIDED_TOUR_KEYS.indexOf(key);
    return idx >= 0 ? idx + 1 : null;
  });
  const questDest = $derived.by(() => {
    if (app.quests?.activeDestinationPort) return app.quests.activeDestinationPort;
    if (questMining?.mineDone) {
      const inferred = inferStationPortFromRoomEntry(app.quests?.trackedQuest?.nextRoomEntry) ?? "ceres";
      return {
        questTitle: questMining.questTitle,
        portId: inferred,
        portLabel: PORT_LABELS[inferred] ?? "Ceres Freeport",
      };
    }
    return resolveNextGuidedOfferDestination(
      app.quests?.nextGuidedOffer ?? null,
      chart?.vantage.port || chart?.vantage.body || null,
    );
  });
  const liveStowage = $derived(
    app.tacsim?.ambient?.ownStowage ??
      app.tacsim?.ambient?.stowage ??
      (app.tacsim?.trade?.hullId ? app.tacsim.trade.stowage : null),
  );
  const trackedHold = $derived(
    app.quests?.trackedQuest ? formatQuestHoldReadinessSummary(app.quests.trackedQuest, liveStowage) : null,
  );
  const needsRigRecovery = $derived(trackedHold?.tag === "[RECOVER RIG FIRST]");
  const oreHoldReady = $derived(trackedHold?.tag === "[ORE HOLD READY]");
  const arrivedAtDest = $derived.by(() => {
    if (!questDest) return false;
    if (app.tacsim?.ambient?.settledPad || needsRigRecovery) return false;
    const tracked = app.quests?.trackedQuest;
    if (questMining && !(tracked?.canFinishNow || tracked?.readyToFinish)) {
      return false;
    }
    const rawPort = (chart?.vantage.port || "").trim().toLowerCase();
    if (!rawPort) return false;
    const pid = questDest.portId.toLowerCase();
    const plbl = questDest.portLabel.toLowerCase();
    return rawPort === pid || rawPort === plbl || rawPort.includes(pid) || rawPort.includes(plbl);
  });
  const chartPreflight = $derived(
    !arrivedAtDest && !needsRigRecovery && questDest
      ? evaluateSailingPreflightReadiness(questDest, app.quests?.activeTradeNeed ?? null, liveStowage)
      : null,
  );

  /** The picture's size in its own units; it scales to the pane. */
  const SIZE = 640;
  let flat = $state(false);

  const points = $derived<ChartPoint[]>(chart ? bodyPoints(chart, SIZE) : []);
  const pointOf = $derived(new Map(points.map((p) => [p.id, p])));
  const heading = $derived(chart ? chartHeading(chart) : "");
  const bodies = $derived(chart ? bodyLines(chart) : []);
  const routes = $derived(chart ? routeLines(chart) : []);
  const windows = $derived(chart ? windowLines(chart) : []);
  /** The flat map's rows, north up (row 0 of the grid is the south edge). */
  const flatRows = $derived(frame ? expandRows({ ...frame.grid, units: [] }).map((r) => r.join("")).reverse() : []);

  const RING = "#3a3f4a";
  const ROUTE = "#c2a86b";
  const QUEST_GOLD = "#ffd84d";
  const HERE = "#f2c94c";
  const BODY = "#9fc4d8";
  const MOON = "#6b7a88";
  const LAG = "#46788f";
  /** Doc 21 cut G.5c: the light-lag rings about the viewer's body. */
  const rings = $derived(
    chart ? LAG_RING_MINUTES.map((minutes) => ({ minutes, points: lagRing(chart, minutes, SIZE) })) : [],
  );

  function isQuestRoute(r: ChartRoute | undefined): boolean {
    if (!r || !questDest) return false;
    const pid = questDest.portId.toLowerCase();
    return r.to.toLowerCase() === pid || r.venue.toLowerCase() === pid;
  }

  const orderedRouteItems = $derived.by(() => {
    if (!chart) return [];
    const items = routes.map((line, i) => ({
      line,
      route: chart.routes[i],
      isQuest: isQuestRoute(chart.routes[i]),
    }));
    if (!questDest) return items;
    return [
      ...items.filter((item) => item.isQuest),
      ...items.filter((item) => !item.isQuest),
    ];
  });

  const questBodyIds = $derived.by(() => {
    const out = new Set<string>();
    if (!chart) return out;
    if (questDest) {
      const pid = questDest.portId.toLowerCase();
      out.add(pid);
      for (const r of chart.routes) {
        if (r.to.toLowerCase() === pid || r.venue.toLowerCase() === pid) {
          out.add(r.body.toLowerCase());
        }
      }
    } else if (questMining && !questMining.mineDone) {
      const miningBody =
        inferStationPortFromRoomEntry(app.quests?.trackedQuest?.nextRoomEntry) ?? "ceres";
      out.add(miningBody.toLowerCase());
    } else {
      return out;
    }
    for (const b of chart.bodies) {
      if (out.has(b.id.toLowerCase()) && b.parent) {
        out.add(b.parent.toLowerCase());
      }
    }
    return out;
  });

  function prefillRoutePlan(portId: string | undefined): void {
    if (!portId) return;
    if (app.tacsim) {
      app.tacsim.selectedDestination = portId;
    }
    app.focusComposer?.(`+ship/plan ${portId}`);
  }

  function prefillQuestPlan(): void {
    prefillRoutePlan(questDest?.portId);
  }

  function openQuestGroundChart(): void {
    if (!questMining) return;
    if (app.quarry) {
      app.quarry.mode = "ground";
      app.quarry.family = questMining.recommendedFamily;
    }
    if (app.switchScreen?.("quarry")) {
      return;
    }
    app.focusComposer?.("+chart/ground");
  }
</script>

<div class="chart">
  {#if !chart}
    <p class="muted pad">
      {app.chart.refused ? "The sky chart the server sent did not read." : "Waiting for the sky chart (r asks again)."}
    </p>
  {:else}
    <h2>{heading}</h2>
    {#if questDest || (questMining && !questMining.mineDone)}
      <div class="quest-dest-banner" class:arrived={arrivedAtDest} role="status" aria-label="active quest destination">
        <span class="qbadge" class:arrived={arrivedAtDest || oreHoldReady}>
          {#if needsRigRecovery}[RECOVER RIG FIRST]{:else if arrivedAtDest}[QUEST ARRIVED]{:else if oreHoldReady}[ORE HOLD READY]{:else}[QUEST]{/if}{#if questStepNo !== null}{" "}Step {questStepNo}/7{/if}
        </span>
        {#if needsRigRecovery}
          <strong>Recover Rig First</strong>
          <span class="muted">— stow mined ore (+tv/1 recover go) before returning to {questDest?.portLabel ?? "Ceres Freeport"}</span>
        {:else if questDest && arrivedAtDest}
          <strong>At {questDest.portLabel}</strong>
          <span class="muted">— {questDest.questTitle}</span>
        {:else if questDest}
          <strong>{#if oreHoldReady}Return to{:else}Destination{/if}: {questDest.portLabel}</strong>
          <span class="muted">— {questDest.questTitle}</span>
        {:else if questMining}
          <strong>Surface Mining: {questMining.targetOreSlug}</strong>
          <span class="muted">— {questMining.questTitle}</span>
        {/if}
        {#if chartPreflight}
          <span class="qpreflight" class:ok={chartPreflight.ready} aria-label="chart sailing preflight">
            {#if chartPreflight.ready}[PRE-FLIGHT OK]{:else}[CHECK BEFORE UNDOCK]{/if}
            Fuel {chartPreflight.propellantTonnes}t · Stores {chartPreflight.provisionsDays}d{#if chartPreflight.cargoRequired} · {#if chartPreflight.cargoOk}✓{:else}✗{/if} {chartPreflight.cargoLabel} {chartPreflight.cargoQty}/{chartPreflight.cargoTarget}{/if}
          </span>
        {/if}
        <span class="qactions">
          {#if needsRigRecovery}
            <button
              type="button"
              class="qplan-btn"
              onclick={() => app.focusComposer?.("+tv/1 recover go")}
            >
              Recover Rig (+tv/1 recover go)
            </button>
          {:else if questDest && arrivedAtDest}
            {#if app.quests?.trackedQuest?.canFinishNow && app.quests.trackedQuest.finishCmd}
              <button
                type="button"
                class="qarrived-btn"
                onclick={() => app.focusComposer?.(app.quests.trackedQuest!.finishCmd)}
              >
                Turn In Now (?)
              </button>
            {:else if app.tacsim?.isAboardShip}
              <button
                type="button"
                class="qarrived-btn"
                onclick={() => app.focusComposer?.("+ship/leave")}
              >
                Step Ashore (+ship/leave)
              </button>
            {:else if app.quests?.trackedQuest}
              <button
                type="button"
                class="qarrived-btn"
                onclick={() => app.quests.openQuestDialog(app.quests.trackedQuest!.key)}
              >
                Open Quest Dialog
              </button>
            {/if}
          {:else}
            {#if questMining && !questMining.mineDone}
              <button type="button" class="qground-btn" onclick={openQuestGroundChart}>
                Ground Chart ({questMining.recommendedFamily.toUpperCase()})
              </button>
            {/if}
            {#if questDest}
              <button type="button" class="qplan-btn" onclick={prefillQuestPlan}>
                Plan +ship/plan {questDest.portId}
              </button>
            {/if}
          {/if}
        </span>
      </div>
    {/if}
    <div class="row">
      <div class="picture">
        {#if flat}
          <pre class="flat" aria-label="the flat sky map">{flatRows.join("\n")}</pre>
        {:else}
          <svg viewBox="0 0 {SIZE} {SIZE}" role="img" aria-label="the sky chart">
            <rect x="0" y="0" width={SIZE} height={SIZE} fill="#0e1116" />
            {#each chart.bodies.filter((b) => b.parent === null) as b (b.id)}
              <circle cx={SIZE / 2} cy={SIZE / 2} r={orbitRadius(b, chart, SIZE)} fill="none" stroke={RING} stroke-width="1" />
            {/each}
            {#each rings as ring (ring.minutes)}
              {#if ring.points.length > 1}
                <polyline
                  points={ring.points.map((p) => `${p.x.toFixed(1)},${p.y.toFixed(1)}`).join(" ")}
                  fill="none"
                  stroke={LAG}
                  stroke-width="1"
                  stroke-dasharray="2 4"
                />
                {@const top = ring.points[Math.floor(ring.points.length / 4)]}
                {#if top}
                  <text x={top.x} y={top.y - 3} fill={LAG} font-size="10" text-anchor="middle">{ring.minutes} min</text>
                {/if}
              {/if}
            {/each}
            {#each chart.routes as r (r.to)}
              {@const line = routePolyline(r, points)}
              {@const qRoute = isQuestRoute(r)}
              {#if line.length > 1}
                <polyline
                  points={line.map((p) => `${p.x.toFixed(1)},${p.y.toFixed(1)}`).join(" ")}
                  fill="none"
                  stroke={qRoute ? QUEST_GOLD : ROUTE}
                  stroke-width={qRoute ? "2.5" : "1.5"}
                  stroke-dasharray={qRoute ? undefined : "4 3"}
                  data-quest-route={qRoute ? r.to : undefined}
                />
              {/if}
            {/each}
            <circle cx={SIZE / 2} cy={SIZE / 2} r="5" fill="#f5d36b" />
            {#each chart.bodies as b (b.id)}
              {@const p = pointOf.get(b.id)}
              {@const qBody = questBodyIds.has(b.id.toLowerCase())}
              {#if p && b.parent === null}
                <circle cx={p.x} cy={p.y} r={b.here ? 6 : 4} fill={b.here ? HERE : BODY} />
                {#if b.here}
                  <circle cx={p.x} cy={p.y} r="11" fill="none" stroke={HERE} stroke-width="1.5" />
                {/if}
                {#if qBody}
                  <circle cx={p.x} cy={p.y} r="13.5" fill="none" stroke={QUEST_GOLD} stroke-width="2" data-quest-body={b.id} />
                  <circle cx={p.x} cy={p.y} r="16.5" fill="none" stroke="#fef08a" stroke-width="1" opacity="0.7" />
                {/if}
                <text x={p.x + 8} y={p.y - 6} fill={qBody ? QUEST_GOLD : b.here ? HERE : BODY} font-size="13">
                  {qBody ? `[!] ${b.label}` : b.label}
                </text>
              {:else if p}
                <text x={p.x + 8} y={p.y + 12} fill={qBody ? QUEST_GOLD : MOON} font-size="10">
                  {qBody ? `[!] ${b.label}` : b.label}
                </text>
              {/if}
            {/each}
          </svg>
        {/if}
        <div class="toggles">
          <button type="button" class:on={!flat} onclick={() => (flat = false)}>picture</button>
          <button type="button" class:on={flat} onclick={() => (flat = true)}>flat map</button>
          <span class="muted">{scaleCaption(chart)}</span>
        </div>
      </div>
      <div class="tables">
        <h3>the sky</h3>
        <pre>{bodies.join("\n")}</pre>
        {#if orderedRouteItems.length > 0}
          <h3>passages now</h3>
          <ul class="passage-list">
            {#each orderedRouteItems as item, i (item.route?.venue ?? item.route?.to ?? i)}
              {@const routePort = item.route?.venue || item.route?.to}
              <li class:quest-row={item.isQuest}>
                <button
                  type="button"
                  class="passage-btn"
                  title={routePort ? `Prefill +ship/plan ${routePort}` : undefined}
                  onclick={() => prefillRoutePlan(routePort)}
                >
                  {#if item.isQuest}<span class="qbadge">[QUEST]</span>{" "}{/if}{item.line}
                </button>
              </li>
            {/each}
          </ul>
        {/if}
        {#if windows.length > 0}
          <h3>the window</h3>
          <ul>
            {#each windows as line, i (i)}
              <li>{line}</li>
            {/each}
          </ul>
        {/if}
      </div>
    </div>
  {/if}
</div>

<style>
  .chart {
    padding: 0.5rem 0.75rem;
    overflow: auto;
    height: 100%;
    box-sizing: border-box;
  }
  .quest-dest-banner {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.5rem;
    padding: 0.35rem 0.65rem;
    margin: 0 0 0.6rem;
    border-radius: 6px;
    border: 1px solid #d4a64a;
    background: rgba(212, 166, 74, 0.14);
    color: #fef3c7;
    font-size: 0.84rem;
  }
  .qbadge {
    color: #ffd84d;
    font-weight: 700;
    font-family: var(--font-mono, monospace);
    font-size: 0.78rem;
  }
  .qactions {
    margin-left: auto;
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
  }
  .qplan-btn {
    border: 1px solid #d4a64a;
    border-radius: 4px;
    background: rgba(212, 166, 74, 0.24);
    color: #fef3c7;
    font: inherit;
    font-size: 0.78rem;
    font-weight: 600;
    padding: 0.15rem 0.55rem;
    cursor: pointer;
  }
  .qplan-btn:hover {
    background: rgba(212, 166, 74, 0.38);
  }
  .qground-btn {
    border: 1px solid #34d399;
    border-radius: 4px;
    background: rgba(16, 185, 129, 0.22);
    color: #d1fae5;
    font: inherit;
    font-size: 0.78rem;
    font-weight: 600;
    padding: 0.15rem 0.55rem;
    cursor: pointer;
  }
  .qground-btn:hover {
    background: rgba(16, 185, 129, 0.36);
  }
  .tables ul.passage-list {
    list-style: none;
    padding-left: 0;
  }
  .passage-btn {
    background: none;
    border: 1px solid transparent;
    border-radius: 4px;
    color: inherit;
    font: inherit;
    text-align: left;
    padding: 0.12rem 0.35rem;
    cursor: pointer;
    width: 100%;
  }
  .passage-btn:hover {
    border-color: rgba(212, 166, 74, 0.55);
    background: rgba(212, 166, 74, 0.1);
  }
  .tables li.quest-row {
    color: #fde68a;
    font-weight: 600;
  }
  .tables li.quest-row .passage-btn {
    border-color: rgba(255, 216, 77, 0.45);
    background: rgba(212, 166, 74, 0.12);
  }
  h2 {
    font-size: 1rem;
    margin: 0 0 0.5rem;
  }
  h3 {
    font-size: 0.85rem;
    margin: 0.75rem 0 0.25rem;
    color: var(--muted, #9aa);
  }
  .row {
    display: flex;
    gap: 1rem;
    flex-wrap: wrap;
    align-items: flex-start;
  }
  .picture {
    flex: 0 1 480px;
    min-width: 280px;
  }
  svg {
    width: 100%;
    height: auto;
    display: block;
    border-radius: 4px;
  }
  .flat {
    background: #0e1116;
    color: #cfd8e3;
    font-size: 0.7rem;
    line-height: 1.05;
    padding: 0.5rem;
    overflow: auto;
    border-radius: 4px;
    margin: 0;
  }
  .toggles {
    display: flex;
    gap: 0.5rem;
    align-items: center;
    margin-top: 0.35rem;
    font-size: 0.8rem;
  }
  .toggles button {
    background: transparent;
    border: 1px solid #555;
    color: inherit;
    border-radius: 3px;
    padding: 0.1rem 0.5rem;
    cursor: pointer;
  }
  .toggles button.on {
    border-color: #c2a86b;
    color: #c2a86b;
  }
  .tables {
    flex: 1 1 320px;
    min-width: 280px;
  }
  .tables pre {
    font-size: 0.78rem;
    margin: 0;
    white-space: pre;
    overflow: auto;
  }
  .tables ul {
    margin: 0;
    padding-left: 1.1rem;
    font-size: 0.85rem;
  }
  .tables li {
    margin-bottom: 0.25rem;
  }
  .muted {
    color: var(--muted, #9aa);
  }
  .pad {
    padding: 0.5rem;
  }
  .quest-dest-banner.arrived {
    border-color: rgba(52, 211, 153, 0.7);
    background: rgba(16, 74, 52, 0.35);
  }
  .qbadge.arrived {
    color: #6ee7b7;
  }
  .qpreflight {
    font-family: var(--mono, monospace);
    font-size: 0.72rem;
    font-weight: 700;
    padding: 0.1rem 0.42rem;
    border-radius: 4px;
    border: 1px solid rgba(251, 191, 36, 0.55);
    background: rgba(78, 56, 12, 0.45);
    color: #fde68a;
  }
  .qpreflight.ok {
    border-color: rgba(52, 211, 153, 0.65);
    background: rgba(16, 74, 52, 0.45);
    color: #6ee7b7;
  }
  .qarrived-btn {
    background: rgba(16, 94, 62, 0.85);
    border: 1px solid rgba(52, 211, 153, 0.75);
    color: #d1fae5;
    border-radius: 4px;
    padding: 0.18rem 0.55rem;
    font-size: 0.78rem;
    font-weight: 700;
    cursor: pointer;
  }
</style>
