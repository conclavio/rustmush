<script lang="ts">
  // The room band (interface plan I4): name and id, zone, weather icon and
  // state, IC clock and season, the scene pill, the exits as buttons with
  // their nav keys (D-I-19), and the presence pills (NPCs thinking, people
  // typing) — all from `RoomView` plus `ambient`, read defensively.
  import type { GameApp } from "$lib/game.svelte";
  import { exitCommand } from "$lib/keys";
  import { clockText, roomBand } from "$lib/roomband";
  import {
    computeNextQuestExitBreadcrumb,
    computeShipQuestBreadcrumb,
    exitMatchesQuestBreadcrumb,
    normalizePortVenueTarget,
  } from "$lib/tacsim/spatial";
  import {
    needsOriginPortProcurement,
    resolveQuestShipTargetCompartment,
  } from "$lib/stores/quests.svelte";

  let { app, onCmd }: { app: GameApp; onCmd: (cmd: string) => void } = $props();

  function walkExit(name: string): void {
    const cmd = exitCommand(name);
    if (cmd !== null) {
      onCmd(cmd);
    }
  }

  const band = $derived(roomBand(app.roomView, app.screens.exits));

  const questBreadcrumb = $derived.by(() => {
    const quests = app.quests;
    const tacsim = app.tacsim;
    if (!quests || !tacsim) return null;
    if (tacsim.isAboardShip) {
      if (tacsim.seatedAtBridgeHelm) return null;
      const dockedPortId = tacsim.ambient?.docked
        ? normalizePortVenueTarget(tacsim.ambient?.port ?? tacsim.trade?.hullPort)
        : null;
      const targetComp = resolveQuestShipTargetCompartment(
        quests.active,
        quests.trackedKey,
        dockedPortId,
      );
      const currentComp =
        (tacsim.view3dOpen ? tacsim.physicalLocationKey : null) ?? tacsim.serverShipCompartment;
      return computeShipQuestBreadcrumb(tacsim.shipTemplate, currentComp, targetComp);
    }
    const tracked =
      quests.active.find((q) => q.key === quests.trackedKey) ?? quests.active[0] ?? null;
    const targetEntry =
      tracked?.nextRoomEntry ||
      tracked?.checkpoints.find((c) => !c.checked && !c.liveReady && c.roomEntry)?.roomEntry ||
      null;
    const currentEntry =
      (tacsim.view3dOpen ? tacsim.physicalLocationKey : null) ?? tacsim.serverStationRoomEntry;
    const liveStowage =
      tacsim.ambient?.ownStowage ??
      tacsim.ambient?.stowage ??
      (tacsim.trade?.hullId ? tacsim.trade.stowage : null);
    const needsOriginExchange = needsOriginPortProcurement(
      quests.active,
      quests.trackedKey,
      liveStowage,
    );
    return computeNextQuestExitBreadcrumb(
      tacsim.stationPack,
      currentEntry,
      targetEntry,
      needsOriginExchange,
    );
  });

  const WEATHER_ICONS: Record<string, string> = {
    clear: "☀",
    sunny: "☀",
    cloudy: "☁",
    overcast: "☁",
    rain: "🌧",
    drizzle: "🌦",
    storm: "⛈",
    thunder: "⛈",
    snow: "❄",
    fog: "🌫",
    mist: "🌫",
    wind: "🌬",
  };

  function weatherIcon(icon: string): string {
    return WEATHER_ICONS[icon.toLowerCase()] ?? "";
  }

  let now = $state(Date.now());
  $effect(() => {
    const timer = setInterval(() => {
      now = Date.now();
      app.presence.prune(now);
    }, 1000);
    return () => clearInterval(timer);
  });
</script>

<div class="band">
  {#if !band}
    <span class="muted">No room yet — <code>look</code> when you are in.</span>
  {:else}
    <span class="name">{band.name}</span>
    <span class="dbref">{band.id}</span>
    {#if band.zone}<span class="sep">·</span><span class="zone">{band.zone}</span>{/if}
    {#if band.weather}
      <span class="sep">·</span>
      <!-- The chip shows the module's PROSE when it sent any, falling back to
           the bare state (I4-C20). The prose used to be a tooltip only, which
           is why I3's interim `Weather:` scrollback line could not be retired:
           the fact was visible nowhere else. -->
      <span class="weather" title={band.weather.state}>
        {#if weatherIcon(band.weather.icon)}<span aria-hidden="true">{weatherIcon(band.weather.icon)}</span>{/if}
        {band.weather.line ?? band.weather.state}{#if band.weather.temp !== null}<span class="muted"> {band.weather.temp}°</span>{/if}
      </span>
    {/if}
    {#if band.clock}
      <span class="sep">·</span>
      <span class="clock" title="in-character time">
        {clockText(band.clock)}{#if band.clock.season}<span class="muted"> {band.clock.season}</span>{/if}
      </span>
    {/if}
    {#if band.scene}
      <span class="pill scene" title={band.scene.status}>
        {band.scene.title}{#if band.scene.round !== null}<span class="muted"> r{band.scene.round}{#if band.scene.turn !== null}·t{band.scene.turn}{/if}</span>{/if}
      </span>
    {/if}
    {#each app.quests?.roomMarkers ?? [] as qm (qm.npcId)}
      <button
        type="button"
        class="qmarker"
        data-marker={qm.marker}
        title="{qm.npcName} — {qm.questTitle}"
        onclick={() => app.quests?.openNpcDialog(qm.npcId)}
      >
        <span class="qsym" aria-hidden="true">{qm.symbol}</span>{qm.npcName}
      </button>
    {/each}
    <span class="grow"></span>
    {#each app.presence.thinking as t (t.name)}
      <span class="pill thinking" title="the NPC is thinking">{t.name}…</span>
    {/each}
    {#each app.presence.composing as c (c.name)}
      <span class="pill typing">{c.name} is typing</span>
    {/each}
    {#if band.exits.length > 0}
      <span class="exits" aria-label="exits">
        {#each band.exits as exit, i (`${i}\0${exit.name}`)}
          {@const rawAliases = app.roomView?.exits[i]?.aliases}
          {@const isQuestExit = exitMatchesQuestBreadcrumb(exit.name, rawAliases, questBreadcrumb)}
          {@const baseTitle = exit.key ? `${exit.name} (key ${exit.key})` : exit.name}
          {@const exitTitle =
            isQuestExit && questBreadcrumb
              ? `Quest route (${questBreadcrumb.hops} ${questBreadcrumb.hops === 1 ? "hop" : "hops"} to ${questBreadcrumb.targetRoomName}) · ${baseTitle}`
              : baseTitle}
          <button
            type="button"
            class="exit"
            class:quest-exit={isQuestExit}
            data-quest-exit={isQuestExit ? "true" : undefined}
            onclick={() => walkExit(exit.name)}
            title={exitTitle}
          >
            {#if isQuestExit}<span class="qexit-badge" aria-hidden="true">★</span>{/if}{#if exit.key}<kbd>{exit.key}</kbd>{/if}{exit.name}
          </button>
        {/each}
      </span>
    {/if}
  {/if}
</div>

<style>
  .band {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.35rem;
    padding: 0.35rem 0.75rem;
    border-bottom: 1px solid var(--border);
    background: var(--bg-panel);
    font-size: 0.85rem;
    min-height: 2rem;
  }

  .name {
    color: var(--c-room);
    font-weight: 700;
  }

  .dbref {
    color: var(--fg-faint);
    font-size: 0.75rem;
  }

  .sep {
    color: var(--fg-faint);
  }

  .zone,
  .clock,
  .weather {
    color: var(--fg-muted);
  }

  .muted {
    color: var(--fg-muted);
  }

  code {
    font-family: var(--font-mono);
    background: var(--bg-input);
    border-radius: 4px;
    padding: 0.05em 0.3em;
  }

  .grow {
    flex: 1;
  }

  .pill {
    border-radius: 999px;
    padding: 0 0.55rem;
    font-size: 0.75rem;
    border: 1px solid var(--border);
    color: var(--fg-muted);
  }

  .pill.scene {
    border-color: var(--accent-dim);
    color: var(--accent);
  }

  .pill.thinking {
    border-color: var(--c-npc);
    color: var(--c-npc);
  }

  .pill.typing {
    border-color: var(--c-player);
    color: var(--c-player);
  }

  .qmarker {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    border-radius: 999px;
    padding: 0.08rem 0.55rem 0.08rem 0.35rem;
    font: inherit;
    font-size: 0.76rem;
    font-weight: 600;
    cursor: pointer;
    border: 1px solid #d4a64a;
    background: rgba(212, 166, 74, 0.14);
    color: #fde68a;
  }

  .qmarker[data-marker="turn_in"] {
    border-color: #fbbf24;
    background: rgba(251, 191, 36, 0.22);
    color: #fef3c7;
    box-shadow: 0 0 8px rgba(251, 191, 36, 0.35);
  }

  .qmarker[data-marker="in_progress"] {
    border-color: #64748b;
    background: rgba(100, 116, 139, 0.18);
    color: #cbd5e1;
  }

  .qmarker:hover {
    filter: brightness(1.12);
  }

  .qsym {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.05rem;
    height: 1.05rem;
    border-radius: 999px;
    font-weight: 800;
    font-size: 0.72rem;
    background: #fbbf24;
    color: #161103;
  }

  .qmarker[data-marker="in_progress"] .qsym {
    background: #475569;
    color: #e2e8f0;
  }

  .exits {
    display: inline-flex;
    gap: 0.3rem;
    flex-wrap: wrap;
  }

  .exit {
    background: none;
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--c-exit);
    cursor: pointer;
    font: inherit;
    font-size: 0.8rem;
    padding: 0.1rem 0.5rem;
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
  }

  .exit:hover {
    border-color: var(--c-exit);
  }

  .exit.quest-exit {
    border-color: #fbbf24;
    background: rgba(251, 191, 36, 0.16);
    color: #fef3c7;
    font-weight: 600;
    box-shadow: 0 0 8px rgba(251, 191, 36, 0.28);
  }

  .exit.quest-exit:hover {
    border-color: #fde68a;
    filter: brightness(1.1);
  }

  .qexit-badge {
    color: #fbbf24;
    font-size: 0.78rem;
    line-height: 1;
  }

  kbd {
    font-family: var(--font-mono);
    font-size: 0.7rem;
    background: var(--bg-input);
    border-radius: 3px;
    padding: 0 0.25rem;
    color: var(--fg-muted);
  }
</style>
