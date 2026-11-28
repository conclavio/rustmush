<script lang="ts">
  // 3D Station & Ship Interior Viewport + Contextual Quick Actions (`SceneScreen.svelte`).
  // Toggleable 3D split above `Scrollback`; when seated at the Bridge Helm, hands off
  // seamlessly into `PictureView.svelte` (`mush-pixel-wasm` SpaceCockpit & 60 Hz Docking).

  import { untrack } from "svelte";
  import type { GameApp } from "$lib/game.svelte";
  import { PICTURE_PREF_KEY } from "$lib/picture";
  import { storageGet, storageSet } from "$lib/storage";
  import { tacsimAudio } from "$lib/tacsim/audio";
  import {
    extractActiveDestinationPort,
    needsOriginPortProcurement,
    needsSurfaceExpeditionBoarding,
    resolveNextGuidedOfferDestination,
    resolveQuestShipTargetCompartment,
  } from "$lib/stores/quests.svelte";
  import { normalizePortVenueTarget, resolveLocalStationQuestTarget } from "$lib/tacsim/spatial";
  import {
    formatHeldLockSummary,
    formatMountAtLockEffect,
    formatMountMeta,
    formatMountStateLabel,
  } from "$lib/tacsim/space-fire";
  import { TacsimStage3D, type WorldInteractable3D } from "$lib/tacsim/stage3d";
  import PictureView from "./PictureView.svelte";
  import TacsimPortModals from "./TacsimPortModals.svelte";

  let { app, onCmd }: { app: GameApp; onCmd: (cmd: string) => void } = $props();

  let canvasEl: HTMLCanvasElement | undefined = $state();
  let wrapEl: HTMLDivElement | undefined = $state();
  let stage = $state.raw<TacsimStage3D | null>(null);
  let resizing = $state(false);
  let dragStartY = 0;
  let dragStartH = 380;

  function sendTacCmd(cmd: string): void {
    tacsimAudio.playForCommand(cmd);
    onCmd(cmd);
  }

  function handleAction(act: string): void {
    stage?.notifyExplicitCommandSent();
    if (act.startsWith("ui:") || act === "seat:helm") {
      if (act === "ui:sit_helm" || act === "seat:helm") {
        // Ensure PictureView renders immediately on helm handoff
        if (storageGet(PICTURE_PREF_KEY) !== "on") {
          storageSet(PICTURE_PREF_KEY, "on");
        }
      }
      app.tacsim.executeUiOrMushAction(act);
    } else {
      onCmd(act);
    }
  }

  function handleInteractable(wi: WorldInteractable3D): void {
    switch (wi.kind) {
      case "npc":
        if (wi.npcMeta) {
          const qm =
            wi.npcMeta.questMarker ??
            app.quests.roomMarkers.find((m) => m.npcId === wi.npcMeta?.id) ??
            null;
          if (qm && (qm.marker === "available" || qm.marker === "turn_in")) {
            app.quests.openNpcDialog(wi.npcMeta.id);
          } else {
            app.tacsim.openNpcModal(wi.npcMeta);
          }
        }
        break;
      case "plaque":
        app.tacsim.openDetailModal(
          wi.title.toUpperCase(),
          wi.detailBody || "Inspecting...",
          wi.actionOrKey,
        );
        break;
      case "helm_seat":
        handleAction("ui:sit_helm");
        break;
      default:
        handleAction(wi.actionOrKey);
        break;
    }
  }

  // Mount / teardown Three.js stage when the 3D walk canvas is active
  $effect(() => {
    const c = canvasEl;
    if (!c || !app.tacsim.view3dOpen || app.tacsim.seatedAtBridgeHelm) {
      return;
    }
    const inst = new TacsimStage3D(c, {
      onSendCommand: (cmd) => onCmd(cmd),
      onInteractableTriggered: handleInteractable,
      onFocusedPromptChanged: (p) => {
        app.tacsim.focusedPrompt = p;
      },
      onPhysicalLocationChanged: (loc) => {
        app.tacsim.physicalLocationKey = loc;
      },
      isUiBlockingInput: () =>
        app.tacsim.activeModal !== null ||
        app.quests.hasOpenOverlay ||
        app.requests.current !== null ||
        app.screens.helpOpen ||
        app.screens.paletteOpen,
    });
    stage = inst;

    const wrap = wrapEl;
    const ro =
      typeof ResizeObserver !== "undefined" && wrap
        ? new ResizeObserver((entries) => {
            const rect = entries[0]?.contentRect;
            if (rect) inst.resize(rect.width, rect.height);
          })
        : null;
    if (ro && wrap) ro.observe(wrap);
    if (wrap) {
      const initialHeight = untrack(() => app.tacsim.viewportHeightPx);
      inst.resize(wrap.clientWidth || 720, wrap.clientHeight || initialHeight);
    }

    return () => {
      ro?.disconnect();
      inst.dispose();
      stage = null;
      app.tacsim.focusedPrompt = null;
    };
  });

  // Resize the live stage in-place without tearing down WebGL state when viewport height changes
  $effect(() => {
    const s = stage;
    const h = app.tacsim.viewportHeightPx;
    const wrap = wrapEl;
    if (s && wrap) {
      s.resize(wrap.clientWidth || 720, h);
    }
  });

  // Push reactive room/pack/actor/stowage/holotank state into the live Three.js stage
  $effect(() => {
    const s = stage;
    const _rev = app.tacsim.packsRevision;
    void _rev;
    if (!s) return;
    s.syncWorldState({
      stationPack: app.tacsim.stationPack,
      shipTemplate: app.tacsim.shipTemplate,
      isAboardShip: app.tacsim.isAboardShip,
      serverStationRoomEntry: app.tacsim.serverStationRoomEntry,
      serverShipCompartment: app.tacsim.serverShipCompartment,
      roomRevision: app.tacsim.roomRevision,
      ambient: app.tacsim.ambient,
      berthDockedHulls: app.tacsim.berthDockedHulls,
      berthOwnHullName:
        app.tacsim.trade.berth?.ownHullName ?? app.tacsim.ambient.ownHullName,
      berthOwnHullDockedHere:
        app.tacsim.trade.berth?.ownHullDockedHere ?? app.tacsim.ambient.ownHullDockedHere,
      visibleActors: app.tacsim.visibleActors,
      actorSpeech: app.tacsim.actorSpeech,
      actorComposing: app.tacsim.actorComposing,
      roomMarkers: app.quests.roomMarkers,
      questRoomEntry: resolvedQuestStationTarget,
      questShipCompartment: resolvedQuestShipTarget,
      stowageCargoKg: app.tacsim.trade.stowage.cargoKg,
      holotankData: app.picture.spaceHolotankData,
    });
  });

  const resolvedQuestStationTarget = $derived.by((): string | null => {
    const quests = app.quests;
    const pack = app.tacsim.stationPack;
    if (!quests || !pack) return null;
    const rawTarget =
      trackedQuest?.nextRoomEntry ||
      trackedQuest?.checkpoints.find((c) => !c.checked && !c.liveReady && c.roomEntry)?.roomEntry ||
      (!trackedQuest ? quests.nextGuidedOffer?.giverRoomEntry : null) ||
      null;
    const liveStowage =
      app.tacsim.ambient?.ownStowage ??
      app.tacsim.ambient?.stowage ??
      (app.tacsim.trade?.hullId ? app.tacsim.trade.stowage : null);
    const resolved = resolveLocalStationQuestTarget(
      pack,
      rawTarget,
      needsOriginPortProcurement(quests.active, quests.trackedKey, liveStowage),
      needsSurfaceExpeditionBoarding(quests.active, quests.trackedKey),
    );
    return resolved?.roomEntry ?? null;
  });

  const resolvedQuestShipTarget = $derived.by((): string | null => {
    const quests = app.quests;
    if (!quests || !app.tacsim.isAboardShip || app.tacsim.seatedAtBridgeHelm) return null;
    const settledPad = Boolean(app.tacsim.ambient?.settledPad);
    const dockedPortId =
      app.tacsim.ambient?.docked && !settledPad
        ? normalizePortVenueTarget(app.tacsim.ambient?.port ?? app.tacsim.trade?.hullPort)
        : null;
    return resolveQuestShipTargetCompartment(
      quests.active,
      quests.trackedKey,
      dockedPortId,
      settledPad,
      quests.nextGuidedOffer,
    );
  });

  function onResizeDown(ev: PointerEvent): void {
    ev.preventDefault();
    resizing = true;
    dragStartY = ev.clientY;
    dragStartH = app.tacsim.viewportHeightPx;
    (ev.currentTarget as HTMLElement | null)?.setPointerCapture?.(ev.pointerId);
  }

  function onResizeMove(ev: PointerEvent): void {
    if (!resizing) return;
    const next = Math.max(220, Math.min(680, Math.round(dragStartH + (ev.clientY - dragStartY))));
    app.tacsim.viewportHeightPx = next;
    if (stage && wrapEl) {
      stage.resize(wrapEl.clientWidth || 720, next);
    }
  }

  function onResizeUp(ev: PointerEvent): void {
    if (!resizing) return;
    resizing = false;
    (ev.currentTarget as HTMLElement | null)?.releasePointerCapture?.(ev.pointerId);
  }

  const trackedQuest = $derived(
    app.quests?.active.find((q) => q.key === app.quests?.trackedKey) ?? app.quests?.active[0] ?? null,
  );
  const questDest = $derived(
    app.quests
      ? (extractActiveDestinationPort(app.quests.active, app.quests.trackedKey) ??
          resolveNextGuidedOfferDestination(
            app.quests.nextGuidedOffer,
            app.tacsim.ambient?.port || app.tacsim.trade?.hullPort || app.tacsim.trade?.venueId || null,
          ))
      : null,
  );
  const questMining = $derived(app.quests?.surfaceMiningStatus ?? null);
  const inVehicleBay = $derived(
    app.tacsim.isAboardShip &&
      (app.tacsim.serverShipCompartment === "bay" || app.tacsim.physicalLocationKey === "bay"),
  );
  const roomOfferMarker = $derived(
    !trackedQuest
      ? ((app.quests?.prioritizedRoomMarkers ?? app.quests?.roomMarkers ?? []).find(
          (m) => m.marker === "turn_in" || m.marker === "available",
        ) ?? null)
      : null,
  );
  const nextGuidedOffer = $derived(
    !trackedQuest && !roomOfferMarker ? (app.quests?.nextGuidedOffer ?? null) : null,
  );
  const dockingActive = $derived(Boolean(app.picture?.spaceDockingData));
  const dockTargetMatchesQuest = $derived(
    Boolean(
      questDest &&
        normalizePortVenueTarget(app.tacsim.dockTarget) === questDest.portId.toLowerCase(),
    ),
  );

  function planQuestDestination(): void {
    if (!questDest) return;
    app.tacsim.selectedDestination = questDest.portId;
    onCmd(`+ship/plan ${questDest.portId}`);
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
    onCmd("+chart/ground");
  }
</script>

{#if app.tacsim.available}
  <div class="port-bar" role="toolbar" aria-label="port and ship actions">
    <button
      type="button"
      class="btn3d"
      class:on={app.tacsim.view3dOpen}
      onclick={() => app.tacsim.toggle3dView()}
      title="Toggle 3D station & ship interior viewport"
    >
      {app.tacsim.view3dOpen ? "3D ON" : "3D VIEW"}
    </button>

    {#if app.tacsim.isAboardShip && app.tacsim.ambient.hullName}
      <span class="hull-pill">
        {app.tacsim.ambient.hullName} · {app.tacsim.ambient.docked ? "Docked" : "Under Way"}
      </span>
    {:else if app.tacsim.ambient.ownHullName}
      <span class="hull-pill">
        {app.tacsim.ambient.ownHullName} · {app.tacsim.ambient.ownHullDockedHere ? "Cradled Here" : "Away"}
      </span>
    {/if}

    {#if inVehicleBay && questMining}
      <button
        type="button"
        class="quest-dest-chip"
        onclick={() => handleAction("ui:open_rig")}
        title="Open Dray-P Prospector Rig Cradle for {questMining.questTitle}"
      >
        <span class="qdest-tag">[QUEST]</span>{" "}Rig: {questMining.scanDone ? "✓" : "○"} Scan · {questMining.soundDone ? "✓" : "○"} Sound · {questMining.mineDone ? "✓" : "○"} {questMining.targetOreSlug}
      </button>
    {:else if app.tacsim.isAboardShip && questDest}
      <button
        type="button"
        class="quest-dest-chip"
        onclick={planQuestDestination}
        title="Plan quest route to {questDest.portLabel} (+ship/plan {questDest.portId})"
      >
        <span class="qdest-tag">[QUEST]</span>{" "}Dest: {questDest.portLabel}
      </button>
    {:else if app.tacsim.isAboardShip && questMining && !questMining.mineDone}
      <button
        type="button"
        class="quest-dest-chip"
        onclick={openQuestGroundChart}
        title="Open Ground Chart ({questMining.recommendedFamily.toUpperCase()} wash) to pick a landing site"
      >
        <span class="qdest-tag">[QUEST]</span>{" "}Mine {questMining.targetOreSlug} · Ground Chart
      </button>
    {/if}

    <div class="quick-actions">
      {#each app.tacsim.quickActions as qa (qa.label)}
        <button
          type="button"
          class="qbtn"
          class:accent={qa.accent}
          onclick={() => handleAction(qa.action)}
        >
          {qa.label}
        </button>
      {/each}
    </div>
  </div>

  {#if app.tacsim.view3dOpen}
    {#if app.tacsim.seatedAtBridgeHelm}
      <div class="helm-visor">
        <div class="visor-bar">
          <span class="visor-title">Bridge Flight Visor · Orbital & 60 Hz Docking</span>
          {#if questDest}
            <button
              type="button"
              class="quest-dest-chip"
              onclick={planQuestDestination}
              title="Plan quest route to {questDest.portLabel} (+ship/plan {questDest.portId})"
            >
              <span class="qdest-tag">[QUEST]</span>{" "}Dest: {questDest.portLabel}
            </button>
          {:else if questMining && !questMining.mineDone}
            <button
              type="button"
              class="quest-dest-chip"
              onclick={openQuestGroundChart}
              title="Open Ground Chart ({questMining.recommendedFamily.toUpperCase()} wash) to pick a landing site"
            >
              <span class="qdest-tag">[QUEST]</span>{" "}Mine {questMining.targetOreSlug} · Ground Chart
            </button>
          {/if}
          <div class="visor-btns">
            <button type="button" class="qbtn" onclick={() => onCmd("+helm/thrust 0")}>0%</button>
            <button type="button" class="qbtn" onclick={() => onCmd("+helm/thrust 500")}>50%</button>
            <button type="button" class="qbtn" onclick={() => onCmd("+helm/thrust 1000")}>100%</button>
            <button type="button" class="qbtn" onclick={() => handleAction("ui:open_sailing")}>Undock / Route</button>
            <button
              type="button"
              class="qbtn"
              class:quest-dock={dockTargetMatchesQuest}
              onclick={() => handleAction("ui:dock_destination")}
            >
              {#if dockTargetMatchesQuest}[Q]{" "}{/if}60 Hz Dock ({app.tacsim.dockTarget})
            </button>
            <button type="button" class="qbtn accent" onclick={() => app.tacsim.standFromBridgeHelm()}>
              Stand From Helm
            </button>
          </div>
        </div>
        {#if dockingActive && questDest}
          <div class="quest-dock-banner" role="status" aria-label="quest docking approach">
            <span class="qdest-tag">[QUEST DOCK]</span>
            <strong>{questDest.portLabel}</strong>
            <span class="qd-sep">—</span>
            <span>{questDest.questTitle}</span>
            {#if trackedQuest?.nextStep}
              <span class="qd-step">· {trackedQuest.nextStep}</span>
            {/if}
          </div>
        {/if}

        <div class="visor-tac-strip" aria-label="space fire control and mounts">
          <div class="visor-lock-row">
            <span class="lock-pill" class:active={app.tacsim.spaceFire.lock !== null}>
              {formatHeldLockSummary(app.tacsim.spaceFire)}
            </span>
            <div class="visor-btns">
              <button type="button" class="qbtn" onclick={() => sendTacCmd("+tac/scan")}>Scan</button>
              <button type="button" class="qbtn danger" onclick={() => sendTacCmd("+tac/beam")}>Fire Beam</button>
              <button type="button" class="qbtn" onclick={() => sendTacCmd("+tac/railgun")}>Fire Railgun</button>
              <button type="button" class="qbtn" onclick={() => sendTacCmd("+tac/torp")}>Launch Torpedo</button>
              <button type="button" class="qbtn" onclick={() => sendTacCmd("+tac/pdc offense")}>Laser Offense</button>
              <button type="button" class="qbtn" onclick={() => sendTacCmd("+tac/pdc point")}>Laser Defense</button>
              <button type="button" class="qbtn" onclick={() => sendTacCmd("+tac/pdc auto")}>Auto Point Defense</button>
              <button type="button" class="qbtn" onclick={() => onCmd("+helm/submit")}>Customs Submit</button>
            </div>
          </div>

          {#if app.tacsim.spaceFire.mounts.length > 0}
            <div class="visor-mounts-row">
              {#each app.tacsim.spaceFire.mounts as m (m.id)}
                {@const effect = formatMountAtLockEffect(m)}
                <div
                  class="mount-chip"
                  class:ready={m.state === "ready" && m.mode !== "defense"}
                  class:warn={m.state === "cycling" || m.state === "too_hot" || m.state === "past_reach"}
                  class:down={m.state === "empty" || m.state === "down"}
                >
                  <span class="m-id">{m.id}</span>
                  <span class="m-meta">{formatMountMeta(m)}</span>
                  <span class="m-state">{formatMountStateLabel(m)}</span>
                  {#if effect}
                    <span class="m-effect">{effect}</span>
                  {/if}
                </div>
              {/each}
            </div>
          {/if}
        </div>

        <div class="visor-canvas">
          <PictureView {app} headingDeg={0} prefill={(ln) => app.focusComposer(ln)} />
        </div>
      </div>
    {:else}
      <div
        class="stage-wrap"
        bind:this={wrapEl}
        style="height: {app.tacsim.viewportHeightPx}px;"
      >
        <canvas bind:this={canvasEl} class="stage-canvas"></canvas>

        <div class="hud-hint">
          WASD move · Shift sprint · Drag orbit · Scroll zoom · E / Click interact
        </div>

        {#if trackedQuest}
          <button
            type="button"
            class="quest-hud-pill"
            data-ready={trackedQuest.readyToFinish || trackedQuest.canFinishNow ? "true" : "false"}
            onclick={() => app.quests?.openQuestDialog(trackedQuest.key)}
            title="Open quest dialog"
          >
            <span class="qhud-crest">{trackedQuest.readyToFinish || trackedQuest.canFinishNow ? "?" : "▸"}</span>
            <span class="qhud-body">
              <strong>{trackedQuest.title}</strong>
              <span>{trackedQuest.nextStep}</span>
            </span>
          </button>
        {:else if roomOfferMarker}
          <button
            type="button"
            class="quest-hud-pill"
            data-ready={roomOfferMarker.marker === "turn_in" ? "true" : "false"}
            onclick={() => app.quests?.openNpcDialog(roomOfferMarker.npcId)}
            title="Speak with {roomOfferMarker.npcName}"
          >
            <span class="qhud-crest">{roomOfferMarker.symbol}</span>
            <span class="qhud-body">
              <strong>{roomOfferMarker.npcName}</strong>
              <span>{roomOfferMarker.questTitle}</span>
            </span>
          </button>
        {:else if nextGuidedOffer}
          <button
            type="button"
            class="quest-hud-pill"
            data-ready="false"
            onclick={() => app.quests?.openLog("active")}
            title="Next Guided Tour Step ({nextGuidedOffer.step}/7)"
          >
            <span class="qhud-crest">!</span>
            <span class="qhud-body">
              <strong>Next Guided ({nextGuidedOffer.step}/7): {nextGuidedOffer.shortLabel}</strong>
              <span>Speak with {nextGuidedOffer.giverLabel} ({nextGuidedOffer.giverRoomName})</span>
            </span>
          </button>
        {/if}

        {#if app.tacsim.focusedPrompt}
          <button
            type="button"
            class="interact-banner"
            onclick={() => {
              // Trigger focused interactable on click too
              const ev = new KeyboardEvent("keydown", { code: "KeyE", key: "e", bubbles: true });
              window.dispatchEvent(ev);
            }}
          >
            <span class="ititle">{app.tacsim.focusedPrompt.title}</span>
            <span class="isub">{app.tacsim.focusedPrompt.subtitle}</span>
          </button>
        {/if}

        {#if stage && !stage.supported}
          <div class="webgl-fallback">
            WebGL is unavailable in this browser context — using text & diegetic port modals.
          </div>
        {/if}
      </div>
      <div
        class="resize-grip"
        role="separator"
        aria-orientation="horizontal"
        onpointerdown={onResizeDown}
        onpointermove={onResizeMove}
        onpointerup={onResizeUp}
      ></div>
    {/if}
  {/if}

  <TacsimPortModals {app} {onCmd} />
{/if}

<style>
  .port-bar {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.4rem;
    padding: 0.3rem 0.75rem;
    border-bottom: 1px solid var(--border);
    background: color-mix(in srgb, var(--bg-panel) 88%, #0c2238 12%);
    flex: none;
  }

  .btn3d {
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--fg-muted);
    font: inherit;
    font-family: var(--font-mono);
    font-size: 0.74rem;
    font-weight: 700;
    padding: 0.16rem 0.55rem;
    cursor: pointer;
  }

  .btn3d.on {
    background: #144a6e;
    border-color: #3da8e8;
    color: #e8f7ff;
  }

  .hull-pill {
    font-family: var(--font-mono);
    font-size: 0.74rem;
    color: #88d4ff;
    padding: 0.1rem 0.5rem;
    border-radius: 999px;
    border: 1px solid rgb(80 170 230 / 35%);
    background: rgb(16 42 68 / 45%);
  }

  .quest-dest-chip {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    font-family: var(--font-mono);
    font-size: 0.73rem;
    font-weight: 600;
    color: #fef3c7;
    padding: 0.12rem 0.55rem;
    border-radius: 999px;
    border: 1px solid #fbbf24;
    background: rgba(251, 191, 36, 0.18);
    cursor: pointer;
    box-shadow: 0 0 8px rgba(251, 191, 36, 0.25);
  }

  .quest-dest-chip:hover {
    filter: brightness(1.14);
  }

  .qdest-tag {
    color: #fbbf24;
    font-weight: 800;
  }

  .quick-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem;
    margin-left: auto;
  }

  .qbtn {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--fg);
    font: inherit;
    font-size: 0.76rem;
    padding: 0.16rem 0.58rem;
    cursor: pointer;
  }

  .qbtn:hover {
    border-color: var(--accent);
  }

  .qbtn.accent {
    background: #164866;
    border-color: #3ca6e0;
    color: #effaff;
    font-weight: 600;
  }

  .stage-wrap {
    position: relative;
    width: 100%;
    background: #06090f;
    overflow: hidden;
    flex: none;
  }

  .stage-canvas {
    width: 100%;
    height: 100%;
    display: block;
    touch-action: none;
  }

  .hud-hint {
    position: absolute;
    top: 0.45rem;
    right: 0.6rem;
    padding: 0.15rem 0.5rem;
    border-radius: 5px;
    background: rgb(6 12 22 / 72%);
    color: #9bb8d4;
    font-family: var(--font-mono);
    font-size: 0.7rem;
    pointer-events: none;
  }

  .interact-banner {
    position: absolute;
    left: 50%;
    bottom: 0.75rem;
    transform: translateX(-50%);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.1rem;
    padding: 0.38rem 1.05rem;
    border-radius: 8px;
    background: rgb(8 20 34 / 88%);
    border: 1px solid #44d492;
    cursor: pointer;
    box-shadow: 0 4px 16px rgb(0 0 0 / 45%);
  }

  .ititle {
    color: #66ffaa;
    font-weight: 700;
    font-size: 0.86rem;
  }

  .isub {
    color: #b8d8ee;
    font-size: 0.74rem;
  }

  .webgl-fallback {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--fg-muted);
    font-size: 0.85rem;
    padding: 1rem;
    text-align: center;
  }

  .resize-grip {
    height: 5px;
    background: var(--border);
    cursor: ns-resize;
    flex: none;
  }

  .resize-grip:hover {
    background: var(--accent);
  }

  .helm-visor {
    display: flex;
    flex-direction: column;
    border-bottom: 1px solid var(--border);
    background: #060a12;
    flex: none;
  }

  .visor-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 0.4rem;
    padding: 0.35rem 0.75rem;
    background: #0c1828;
    border-bottom: 1px solid #1e3d5c;
  }

  .visor-title {
    font-family: var(--font-mono);
    font-size: 0.76rem;
    font-weight: 700;
    color: #77d4ff;
    text-transform: uppercase;
  }

  .visor-btns {
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem;
  }

  .qbtn.danger {
    background: #521a24;
    border-color: #c44a62;
    color: #ffeef2;
    font-weight: 600;
  }

  .qbtn.quest-dock {
    background: #332609;
    border-color: #eab308;
    color: #ffe270;
    font-weight: 700;
  }

  .quest-dock-banner {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.4rem;
    padding: 0.28rem 0.75rem;
    background: #231a06;
    border-bottom: 1px solid #8a6a1a;
    color: #f8e9b5;
    font-family: var(--font-mono);
    font-size: 0.74rem;
  }

  .quest-dock-banner .qd-sep,
  .quest-dock-banner .qd-step {
    color: #d6c38d;
  }

  .visor-tac-strip {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    padding: 0.35rem 0.75rem;
    background: #08121f;
    border-bottom: 1px solid #18324c;
  }

  .visor-lock-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 0.4rem;
  }

  .lock-pill {
    font-family: var(--font-mono);
    font-size: 0.73rem;
    padding: 0.14rem 0.55rem;
    border-radius: 6px;
    border: 1px solid #2b4a68;
    background: #0e1d30;
    color: #9ec0dd;
  }

  .lock-pill.active {
    border-color: #e69b38;
    background: #2b1d0c;
    color: #ffd588;
    font-weight: 600;
  }

  .visor-mounts-row {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
  }

  .mount-chip {
    display: inline-flex;
    align-items: center;
    gap: 0.38rem;
    padding: 0.16rem 0.52rem;
    border-radius: 6px;
    border: 1px solid #24405c;
    background: #0c1a2a;
    font-family: var(--font-mono);
    font-size: 0.71rem;
    color: #d8ecfc;
  }

  .mount-chip.ready {
    border-color: #2f8f66;
    background: #0b241c;
  }

  .mount-chip.warn {
    border-color: #9e7428;
    background: #241b0c;
  }

  .mount-chip.down {
    border-color: #8c353e;
    background: #261015;
  }

  .m-id {
    font-weight: 700;
    color: #eff8ff;
    text-transform: uppercase;
  }

  .m-meta {
    color: #9ac4e2;
  }

  .m-state {
    color: #77f2b4;
    font-weight: 600;
  }

  .mount-chip.warn .m-state {
    color: #ffcf6e;
  }

  .mount-chip.down .m-state {
    color: #ff858f;
  }

  .m-effect {
    color: #ffdd8a;
    font-weight: 700;
  }

  .visor-canvas {
    padding: 0.35rem 0.6rem;
    max-height: 440px;
    overflow: auto;
  }

  .quest-hud-pill {
    position: absolute;
    top: 0.55rem;
    right: 0.65rem;
    z-index: 4;
    display: inline-flex;
    align-items: center;
    gap: 0.45rem;
    max-width: min(320px, 46%);
    padding: 0.32rem 0.6rem;
    border-radius: 8px;
    border: 1px solid #d4a64a;
    background: rgba(12, 18, 28, 0.88);
    color: #fef3c7;
    text-align: left;
    font: inherit;
    font-size: 0.75rem;
    cursor: pointer;
    box-shadow: 0 4px 14px rgba(0, 0, 0, 0.45);
  }

  .quest-hud-pill[data-ready="true"] {
    border-color: #fbbf24;
    background: rgba(34, 24, 6, 0.92);
    box-shadow: 0 0 12px rgba(251, 191, 36, 0.35);
  }

  .quest-hud-pill:hover {
    filter: brightness(1.12);
  }

  .qhud-crest {
    flex: none;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.25rem;
    height: 1.25rem;
    border-radius: 999px;
    background: #fbbf24;
    color: #161103;
    font-weight: 800;
    font-size: 0.78rem;
  }

  .qhud-body {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .qhud-body strong {
    color: #fde68a;
    font-size: 0.76rem;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .qhud-body span {
    color: #cbd5e1;
    font-size: 0.7rem;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
