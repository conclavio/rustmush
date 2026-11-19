// Reactive Svelte 5 store for the Web 3D Station/Ship Viewport & Diegetic Port UI.
// Lazily syncs `tacsim.content` packs and `port.trade` over the existing WebSocket
// without altering `GameApp`'s pinned `hydrationQueries` sequence.

import type { NarrativeKind, ObjRef, RichText, RoomView } from "../proto";
import { plainText } from "../render";
import { storageGet, storageSet } from "../storage";
import { tacsimAudio } from "../tacsim/audio";
import { isValidHullDbref } from "../tacsim/economy";
import {
  areRoomsConnectedWithinHops,
  buildFallbackDray,
  buildFallbackFreeport,
  buildFallbackOocArea,
  contextualQuickActions,
  decodeTacsimRoomAmbient,
  filterVisibleRoomActors,
  findShipExitPath,
  mergeCastPackIntoNeighborhood,
  normalizeUnitId,
  parseNeighborhoodPack,
  parseShipUnitJson,
  resolveDynamicDockTarget,
  resolveShipCompartmentFromRoom,
  resolveStationPackAndEntry,
  type AtlasNeighborhoodPack,
  type DecodedRoomAmbient,
  type DockedHullSummary,
  type QuickActionItem,
  type ShipUnitTemplate,
  type VisibleRoomActor,
} from "../tacsim/spatial";
import {
  beginOneClickBuy,
  buildFallbackGoodsTable,
  CORRIDOR_DESTINATIONS,
  decodePortTradeFeed,
  emptyPortTradeSnapshot,
  formatOneClickSellCommand,
  parseGoodsJson,
  PORT_TRADE_FEED,
  resolvePendingQuoteFromFeed,
  suggestUniqueHullName,
  TACSIM_CONTENT_FEED,
  type GoodItemDef,
  type PendingQuoteLatch,
  type PortListingRow,
  type PortTradeSnapshot,
} from "../tacsim/trade";
import {
  decodeSpaceFireFeed,
  emptySpaceFireSnapshot,
  type SpaceFireSnapshot,
} from "../tacsim/space-fire";

export const SCENE_3D_PREF_KEY = "rustmush.scene3d";
export const PACK_IN_FLIGHT_TIMEOUT_MS = 5_000;
export const PACK_RETRY_BASE_DELAY_MS = 1_000;
export const PACK_MAX_RETRIES = 4;

export type TacsimModalKind =
  | "coop"
  | "trade"
  | "sailing"
  | "npc"
  | "sim"
  | "rig"
  | "detail"
  | null;

export interface TacsimTransportHooks {
  hasPanel: (feed: string) => boolean;
  subscribePanel: (feed: string, params?: Record<string, unknown>) => boolean;
  sendCommand: (line: string) => void;
  selfCharacterId: () => string | null;
  /** Optional session/server/character scope key to partition pack caches across switches. */
  sessionScopeKey?: () => string | null;
}

function unrefTimer(timer: ReturnType<typeof setTimeout>): ReturnType<typeof setTimeout> {
  if (typeof timer === "object" && timer !== null && "unref" in timer) {
    (timer as { unref?: () => void }).unref?.();
  }
  return timer;
}

function asObj(v: unknown): Record<string, unknown> | null {
  return typeof v === "object" && v !== null && !Array.isArray(v)
    ? (v as Record<string, unknown>)
    : null;
}

export class TacsimPortStore {
  /** Whether the 3D Scene Viewport split is open on Screen 1. */
  view3dOpen = $state<boolean>(storageGet(SCENE_3D_PREF_KEY) === "1");
  /** Viewport height in pixels (user-resizable). */
  viewportHeightPx = $state<number>(380);
  /** Whether the pilot is currently seated in the Bridge Helm crash couch. */
  seatedAtBridgeHelm = $state<boolean>(false);

  /** Active diegetic modal over the Scene screen. */
  activeModal = $state<TacsimModalKind>(null);
  focusedNpc = $state<{ id: string; name: string; bio: string; isNpc: boolean } | null>(null);
  detailModal = $state<{ title: string; body: string } | null>(null);
  modalLog = $state<string[]>([]);
  coopSuggestedHull = $state<string>(suggestUniqueHullName());
  selectedDestination = $state<string>(CORRIDOR_DESTINATIONS[1]?.id ?? "stickney");
  selectedQtyPreset = $state<number | "max">(5);

  /** Latest decoded `RoomView` & `ambient`. */
  roomView = $state<RoomView | null>(null);
  /** Monotonic revision incremented strictly on server `Room` events (never on chat/typing/holotank). */
  roomRevision = $state<number>(0);
  ambient = $state<DecodedRoomAmbient>(decodeTacsimRoomAmbient(null));
  isAboardShip = $state<boolean>(false);
  serverStationRoomEntry = $state<string | null>(null);
  serverShipCompartment = $state<string | null>(null);
  /** Physical room/compartment tracked by 3D avatar walking (may lead `serverStationRoomEntry` by 1-2 hops). */
  physicalLocationKey = $state<string | null>(null);

  /** Loaded 3D packs & goods catalog. */
  stationPack = $state.raw<AtlasNeighborhoodPack>(buildFallbackFreeport());
  shipTemplate = $state.raw<ShipUnitTemplate>(buildFallbackDray("tacsim.unit.dray"));
  goodsById = $state.raw<Map<string, GoodItemDef>>(buildFallbackGoodsTable());
  packsRevision = $state<number>(0);

  /** Structured `port.trade` snapshot. */
  trade = $state<PortTradeSnapshot>(emptyPortTradeSnapshot());
  /** Authoritative Doc 49 (`CC.9`) `space.cockpit` mounts, magazines & 5-band lock snapshot. */
  spaceFire = $state<SpaceFireSnapshot>(emptySpaceFireSnapshot());

  /** Active speech bubbles & composing flags keyed by actor id or lowercase name. */
  actorSpeech = $state<Record<string, { text: string; expiresAt: number }>>({});
  actorComposing = $state<Record<string, boolean>>({});

  /** Focused 3D interactable prompt (`[E] ...`). */
  focusedPrompt = $state<{ title: string; subtitle: string } | null>(null);

  #hooks: TacsimTransportHooks;
  #rawPacks = new Map<string, { json: string; sha256: string | null; canonicalId: string }>();
  #manifestDigests = new Map<string, string>();
  #packsInFlight = new Map<string, { sentAtMs: number; sha256: string | null }>();
  #packsFailed = new Map<string, string | null>();
  #packRetryCounts = new Map<string, number>();
  #packRetryAfterMs = new Map<string, number>();
  #manifestReceived = false;
  #manifestInFlightAtMs: number | null = null;
  #manifestRetryCount = 0;
  #desiredStationPackId = "neighborhood.ceres_freeport";
  #desiredShipUnitId = "tacsim.unit.dray";
  #activeSessionScopeKey: string | null = null;
  #packQueue: string[] = [];
  #packDrainTimer: ReturnType<typeof setTimeout> | null = null;
  #packRetryTimer: ReturnType<typeof setTimeout> | null = null;
  #pendingQuote: PendingQuoteLatch | null = null;
  #awaitingAppendTradePage = false;
  #lastAutoTradeRefreshAt = 0;

  constructor(hooks: TacsimTransportHooks) {
    this.#hooks = hooks;
  }

  /** Whether the current room or server advertises Corridor port/ship/sim features. */
  get available(): boolean {
    return (
      this.#hooks.hasPanel(PORT_TRADE_FEED) ||
      this.#hooks.hasPanel(TACSIM_CONTENT_FEED) ||
      this.ambient.atlasPack !== null ||
      this.ambient.tacsimMode !== null ||
      this.isAboardShip
    );
  }

  /** Docked hulls for the Berth Gallery, preferring live `port.trade.berth.docked_hulls` (`W3D-H4`). */
  get berthDockedHulls(): DockedHullSummary[] {
    if (this.trade.berth !== null) {
      return this.trade.berth.dockedHulls;
    }
    return this.ambient.dockedHulls;
  }

  /** Active hull `#<dbref>` required by `+escort/post` (`C-21`) and supported by `+bay`. */
  get activeHullRef(): string | null {
    const candidates = [
      this.ambient.hullId,
      this.trade.hullId,
      this.trade.berth?.ownHull ?? null,
      this.ambient.ownHull,
    ];
    for (const c of candidates) {
      if (isValidHullDbref(c)) return c!.trim();
    }
    return null;
  }

  /** Dynamic docking target venue derived from orbital site, current port, or selected route (`H-40`). */
  get dockTarget(): string {
    return resolveDynamicDockTarget({
      cockpitSiteId: this.spaceFire.siteId,
      ambientPort: this.ambient.port,
      ambientPortEntry: this.ambient.portEntry,
      tradeHullPort: this.trade.hullPort,
      tradeVenueId: this.trade.venueId,
      stationPackId: this.stationPack.packId,
      selectedDestination: this.selectedDestination,
    });
  }

  get visibleActors(): VisibleRoomActor[] {
    return filterVisibleRoomActors(
      this.roomView,
      this.ambient,
      this.#hooks.selfCharacterId(),
      this.isAboardShip ? null : this.stationPack,
      this.berthDockedHulls,
    );
  }

  get quickActions(): QuickActionItem[] {
    const authoritativeLoc = this.isAboardShip
      ? this.serverShipCompartment
      : this.serverStationRoomEntry;
    const loc = this.view3dOpen
      ? (this.physicalLocationKey ?? authoritativeLoc)
      : authoritativeLoc;
    return contextualQuickActions(
      loc,
      this.isAboardShip,
      this.seatedAtBridgeHelm,
      this.dockTarget,
      this.#authoritativeStationPackId(),
    );
  }

  toggle3dView(force?: boolean): void {
    const next = force !== undefined ? force : !this.view3dOpen;
    this.view3dOpen = next;
    storageSet(SCENE_3D_PREF_KEY, next ? "1" : "0");
    if (next) {
      this.ensurePacksSynced();
      this.#maybeAutoRefreshTrade(Date.now());
    } else {
      this.physicalLocationKey = this.isAboardShip
        ? this.serverShipCompartment
        : this.serverStationRoomEntry;
    }
  }

  ensurePacksSynced(): void {
    this.#ensureSessionScope();
    if (!this.#hooks.hasPanel(TACSIM_CONTENT_FEED)) return;
    const now = Date.now();
    this.#expireTimedOutPacks(now);
    if (this.#manifestReceived) {
      this.#pumpPackQueue(now);
      return;
    }
    if (
      this.#manifestInFlightAtMs !== null &&
      now - this.#manifestInFlightAtMs < PACK_IN_FLIGHT_TIMEOUT_MS
    ) {
      return;
    }
    if (this.#manifestInFlightAtMs !== null) {
      this.#manifestRetryCount += 1;
      if (this.#manifestRetryCount > PACK_MAX_RETRIES) {
        return;
      }
    }
    const sent = this.#hooks.subscribePanel(TACSIM_CONTENT_FEED, { op: "manifest" });
    this.#manifestInFlightAtMs = sent ? now : null;
  }

  clear(): void {
    this.seatedAtBridgeHelm = false;
    this.activeModal = null;
    this.focusedNpc = null;
    this.detailModal = null;
    this.modalLog = [];
    this.roomView = null;
    this.roomRevision = 0;
    this.ambient = decodeTacsimRoomAmbient(null);
    this.isAboardShip = false;
    this.serverStationRoomEntry = null;
    this.serverShipCompartment = null;
    this.physicalLocationKey = null;
    this.trade = emptyPortTradeSnapshot();
    this.spaceFire = emptySpaceFireSnapshot();
    this.actorSpeech = {};
    this.actorComposing = {};
    this.focusedPrompt = null;
    this.#activeSessionScopeKey = null;
    this.#pendingQuote = null;
    this.#awaitingAppendTradePage = false;
    this.#resetPackCaches();
  }

  observeRoom(view: RoomView): void {
    this.#ensureSessionScope();
    const prevRoomId = this.roomView?.id ?? null;
    const wasAboard = this.isAboardShip;
    this.roomView = view;
    this.roomRevision += 1;
    if (prevRoomId !== null && prevRoomId !== view.id) {
      this.actorComposing = {};
    }
    const dec = decodeTacsimRoomAmbient(view);
    this.ambient = dec;

    if (dec.stowage) {
      this.trade = { ...this.trade, stowage: dec.stowage };
    } else if (dec.ownStowage) {
      this.trade = { ...this.trade, stowage: dec.ownStowage };
    }

    // Check if aboard a ship interior
    const desiredUnit = dec.unit ? normalizeUnitId(dec.unit) : this.#desiredShipUnitId;
    this.#desiredShipUnitId = desiredUnit;
    if (desiredUnit !== this.shipTemplate.unitId) {
      this.#rebuildShipTemplate(desiredUnit);
    }

    const shipComp = resolveShipCompartmentFromRoom(view, dec, this.shipTemplate);
    if (shipComp !== null) {
      this.isAboardShip = true;
      this.serverShipCompartment = shipComp;
      this.serverStationRoomEntry = null;
      if (shipComp !== "bridge" && this.seatedAtBridgeHelm) {
        this.seatedAtBridgeHelm = false;
      }
      if (
        !this.view3dOpen ||
        !wasAboard ||
        !this.physicalLocationKey ||
        (this.physicalLocationKey !== shipComp &&
          findShipExitPath(this.shipTemplate, shipComp, this.physicalLocationKey).length === 0)
      ) {
        this.physicalLocationKey = shipComp;
      }
      if (this.view3dOpen) this.ensurePacksSynced();
      return;
    }

    // Otherwise Station / OOC room
    this.isAboardShip = false;
    this.seatedAtBridgeHelm = false;
    this.serverShipCompartment = null;

    const { desiredPackId, entryId } = resolveStationPackAndEntry(view, dec, this.stationPack);
    if (dec.atlasPack || desiredPackId !== this.stationPack.packId) {
      this.#desiredStationPackId = desiredPackId;
    }
    const targetPackId = this.#authoritativeStationPackId();
    if (targetPackId !== this.stationPack.packId) {
      this.#rebuildStationPack(targetPackId);
    }
    const resolvedEntry =
      entryId ??
      resolveStationPackAndEntry(view, dec, this.stationPack).entryId;
    this.serverStationRoomEntry = resolvedEntry;

    if (
      !this.view3dOpen ||
      wasAboard ||
      !this.physicalLocationKey ||
      !resolvedEntry ||
      !areRoomsConnectedWithinHops(this.stationPack, resolvedEntry, this.physicalLocationKey, 3)
    ) {
      this.physicalLocationKey = resolvedEntry;
    }

    if (this.view3dOpen) {
      this.ensurePacksSynced();
      this.#maybeAutoRefreshTrade(Date.now());
    }
  }

  observeNarrative(ev: { kind: NarrativeKind; actor?: ObjRef | null; text: RichText | string }): void {
    const plain = typeof ev.text === "string" ? ev.text : plainText(ev.text);
    const now = Date.now();
    if (ev.actor && plain.trim().length > 0) {
      const next: Record<string, { text: string; expiresAt: number }> = {};
      for (const [k, v] of Object.entries(this.actorSpeech)) {
        if (v.expiresAt > now) next[k] = v;
      }
      const exp = now + 8_500;
      next[ev.actor.id] = { text: plain.trim(), expiresAt: exp };
      next[ev.actor.name.toLowerCase()] = { text: plain.trim(), expiresAt: exp };
      this.actorSpeech = next;
    }
    if (this.activeModal !== null && plain.trim().length > 0) {
      const who = ev.actor?.name ? `${ev.actor.name}: ` : "";
      this.#pushModalLine(`${who}${plain.trim()}`);
    }
    this.#maybeAutoRefreshTrade(now);
  }

  observeComposing(ev: { actor: ObjRef; active: boolean }): void {
    if (ev.actor.id === this.#hooks.selfCharacterId()) return;
    this.actorComposing = {
      ...this.actorComposing,
      [ev.actor.id]: ev.active,
      [ev.actor.name.toLowerCase()]: ev.active,
    };
  }

  observeSys(line: string): void {
    if (this.activeModal !== null && line.trim().length > 0) {
      this.#pushModalLine(line.trim());
    }
  }

  observePortTrade(data: unknown): void {
    const append = this.#awaitingAppendTradePage;
    this.#awaitingAppendTradePage = false;
    const updated = decodePortTradeFeed(data, this.goodsById, this.trade, append);
    this.trade = updated;

    const res = resolvePendingQuoteFromFeed(updated, this.#pendingQuote, Date.now());
    if (res.clearLatch) {
      this.#pendingQuote = null;
    }
    if (res.acceptCommand) {
      this.#pushModalLine(`[QUOTE VERIFIED] ${res.acceptCommand}`);
      tacsimAudio.playMarketFillChime();
      this.#hooks.sendCommand(res.acceptCommand);
      this.requestPortTrade();
    } else if (res.priceWarning) {
      this.#pushModalLine(`[PRICE MOVED] ${res.priceWarning}`);
    }
  }

  observeSpaceCockpit(data: unknown): void {
    this.spaceFire = decodeSpaceFireFeed(data);
  }

  observeContentFeed(data: unknown): void {
    this.#ensureSessionScope();
    const obj = asObj(data);
    if (!obj) return;

    const now = Date.now();
    this.#expireTimedOutPacks(now);

    const op = typeof obj["op"] === "string" ? obj["op"] : "";
    // Manifest frame (`op: "manifest"`, `packs: [{ id, category, sha256 }, ...]`)
    if (op === "manifest" || Array.isArray(obj["packs"])) {
      this.#manifestReceived = true;
      this.#manifestInFlightAtMs = null;
      this.#manifestRetryCount = 0;

      const packs = Array.isArray(obj["packs"]) ? obj["packs"] : [];
      const wanted: string[] = [];
      for (const item of packs) {
        const p = asObj(item);
        if (!p) continue;
        const id = typeof p["id"] === "string" ? p["id"] : "";
        const cat = typeof p["category"] === "string" ? p["category"] : "";
        const digest =
          typeof p["sha256"] === "string" && p["sha256"].trim().length > 0
            ? p["sha256"].trim()
            : null;
        if (!id) continue;
        const prevDigest = this.#manifestDigests.get(id) ?? null;
        if (digest !== null) {
          this.#manifestDigests.set(id, digest);
          if (prevDigest !== null && prevDigest !== digest) {
            this.#packRetryCounts.delete(id);
            this.#packRetryAfterMs.delete(id);
          }
        }
        if (
          cat === "neighborhood" ||
          cat === "cast" ||
          cat === "unit" ||
          cat === "market"
        ) {
          const cached = this.#rawPacks.get(id);
          if (cached && digest !== null && cached.sha256 !== digest) {
            this.#evictCachedPack(id);
            this.#packRetryCounts.delete(id);
            this.#packRetryAfterMs.delete(id);
          }

          if (this.#packsFailed.has(id)) {
            const failedDigest = this.#packsFailed.get(id) ?? null;
            if (digest !== null && failedDigest !== digest) {
              this.#packsFailed.delete(id);
              this.#packRetryCounts.delete(id);
              this.#packRetryAfterMs.delete(id);
            }
          }

          const inFlight = this.#packsInFlight.get(id);
          if (inFlight && digest !== null && inFlight.sha256 !== digest) {
            this.#packsInFlight.delete(id);
            this.#packRetryCounts.delete(id);
            this.#packRetryAfterMs.delete(id);
          }

          if (
            !this.#rawPacks.has(id) &&
            !this.#packsInFlight.has(id) &&
            !this.#packsFailed.has(id)
          ) {
            wanted.push(id);
          }
        }
      }
      this.#enqueuePackFetches(wanted, now);
      return;
    }

    // Individual pack frame (`op: "pack"`, `found`, `id`, `requested_id`, `sha256`, `json`)
    const id = typeof obj["id"] === "string" ? obj["id"] : "";
    const reqId = typeof obj["requested_id"] === "string" ? obj["requested_id"] : "";
    const packSha =
      typeof obj["sha256"] === "string" && obj["sha256"].trim().length > 0
        ? obj["sha256"].trim()
        : null;

    const clearPendingId = (key: string): void => {
      if (!key) return;
      this.#packsInFlight.delete(key);
      this.#packRetryCounts.delete(key);
      this.#packRetryAfterMs.delete(key);
      const qIdx = this.#packQueue.indexOf(key);
      if (qIdx >= 0) this.#packQueue.splice(qIdx, 1);
    };
    clearPendingId(id);
    clearPendingId(reqId);

    if (obj["found"] === false) {
      const failedDigest =
        (id ? this.#manifestDigests.get(id) : undefined) ??
        (reqId ? this.#manifestDigests.get(reqId) : undefined) ??
        null;
      if (id) this.#packsFailed.set(id, failedDigest);
      if (reqId) this.#packsFailed.set(reqId, failedDigest);
      this.#schedulePackMaintenance(now);
      return;
    }

    const jsonText =
      typeof obj["json"] === "string"
        ? obj["json"]
        : obj["pack"] && typeof obj["pack"] === "object"
          ? JSON.stringify(obj["pack"])
          : "";
    if (!id || !jsonText) {
      this.#schedulePackMaintenance(now);
      return;
    }

    const expectedDigest =
      this.#manifestDigests.get(id) ??
      (reqId ? this.#manifestDigests.get(reqId) : undefined) ??
      null;
    if (packSha !== null && expectedDigest !== null && packSha !== expectedDigest) {
      // Stale in-flight reply from an older manifest; re-fetch the current digest.
      this.#enqueuePackFetches([id], now);
      return;
    }

    const effectiveSha = packSha ?? expectedDigest;
    this.#rawPacks.set(id, { json: jsonText, sha256: effectiveSha, canonicalId: id });
    this.#packsFailed.delete(id);
    if (reqId && reqId !== id) {
      this.#rawPacks.set(reqId, { json: jsonText, sha256: effectiveSha, canonicalId: id });
      this.#packsFailed.delete(reqId);
    }

    if (id === "goods" || reqId === "goods") {
      this.goodsById = parseGoodsJson(jsonText);
    } else if (id.startsWith("neighborhood.") || reqId.startsWith("neighborhood.")) {
      const targetPackId = this.#authoritativeStationPackId();
      if (id === targetPackId || reqId === targetPackId) {
        this.#rebuildStationPack(targetPackId);
        if (!this.isAboardShip && this.roomView) {
          const resolvedEntry = resolveStationPackAndEntry(
            this.roomView,
            this.ambient,
            this.stationPack,
          ).entryId;
          this.serverStationRoomEntry = resolvedEntry;
          if (
            !this.view3dOpen ||
            !this.physicalLocationKey ||
            !this.stationPack.rooms.has(this.physicalLocationKey)
          ) {
            this.physicalLocationKey = resolvedEntry;
          }
        }
      }
    } else if (id.startsWith("cast.") || reqId.startsWith("cast.")) {
      this.#rebuildStationPack(this.#authoritativeStationPackId());
    } else {
      const normUnit = normalizeUnitId(id);
      const normReq = reqId ? normalizeUnitId(reqId) : null;
      const targetUnit = this.#desiredShipUnitId || this.shipTemplate.unitId;
      if (normUnit === targetUnit || normReq === targetUnit) {
        this.#rebuildShipTemplate(targetUnit);
        if (this.isAboardShip && this.roomView) {
          const resolvedComp = resolveShipCompartmentFromRoom(
            this.roomView,
            this.ambient,
            this.shipTemplate,
          );
          if (resolvedComp !== null) {
            this.serverShipCompartment = resolvedComp;
            if (!this.view3dOpen || !this.physicalLocationKey) {
              this.physicalLocationKey = resolvedComp;
            }
          }
        }
      }
    }
    this.packsRevision += 1;
    this.#schedulePackMaintenance(now);
  }

  requestPortTrade(afterListingId?: number): void {
    if (this.#hooks.hasPanel(PORT_TRADE_FEED)) {
      if (afterListingId && afterListingId > 0) {
        this.#awaitingAppendTradePage = true;
        this.#hooks.subscribePanel(PORT_TRADE_FEED, { after: afterListingId });
      } else {
        this.#awaitingAppendTradePage = false;
        this.#hooks.subscribePanel(PORT_TRADE_FEED);
      }
    } else {
      this.#hooks.sendCommand("+market/list");
      this.#hooks.sendCommand("+credit");
      this.#hooks.sendCommand("+ship");
    }
  }

  openModal(kind: TacsimModalKind): void {
    this.activeModal = kind;
    this.modalLog = [];
    this.ensurePacksSynced();
    if (kind === "coop") {
      this.coopSuggestedHull = suggestUniqueHullName();
    } else if (kind === "trade" || kind === "sailing" || kind === "sim") {
      this.requestPortTrade();
      if (kind === "sailing") {
        this.#hooks.sendCommand("+ship/orders");
      }
    }
  }

  openNpcModal(actor: { id: string; name: string; bio: string; isNpc: boolean }): void {
    this.focusedNpc = actor;
    this.modalLog = [];
    this.activeModal = "npc";
  }

  openDetailModal(title: string, body: string, lookCmd?: string): void {
    this.detailModal = { title, body };
    this.modalLog = [];
    this.activeModal = "detail";
    if (lookCmd) {
      this.#hooks.sendCommand(lookCmd);
    }
  }

  closeModal(): void {
    this.activeModal = null;
  }

  sitAtBridgeHelm(): void {
    this.seatedAtBridgeHelm = true;
    this.#hooks.sendCommand("+helm/station command");
    if (this.#hooks.hasPanel("space.cockpit")) {
      this.#hooks.subscribePanel("space.cockpit");
    }
    if (this.#hooks.hasPanel("space.holotank")) {
      this.#hooks.subscribePanel("space.holotank");
    }
  }

  standFromBridgeHelm(): void {
    this.seatedAtBridgeHelm = false;
  }

  executeUiOrMushAction(action: string): void {
    if (!action) return;
    if (action === "ui:open_coop") {
      this.openModal("coop");
      return;
    }
    if (action === "ui:open_trade") {
      this.openModal("trade");
      return;
    }
    if (action === "ui:open_sailing") {
      this.openModal("sailing");
      return;
    }
    if (action === "ui:open_sim") {
      this.openModal("sim");
      return;
    }
    if (action === "ui:open_rig") {
      this.openModal("rig");
      return;
    }
    if (action === "ui:sit_helm" || action === "seat:helm") {
      this.sitAtBridgeHelm();
      return;
    }
    if (action === "ui:stand_helm") {
      this.standFromBridgeHelm();
      return;
    }
    this.#hooks.sendCommand(action);
  }

  buyListing(row: PortListingRow, qty: number): void {
    const started = beginOneClickBuy(row, qty, Date.now(), this.trade.quotes);
    if (!started) return;
    this.#pendingQuote = started.latch;
    this.#pushModalLine(`> ${started.quoteCommand}`);
    this.#hooks.sendCommand(started.quoteCommand);
    this.requestPortTrade();
  }

  sellListing(row: PortListingRow, qty: number, sellAll = false): void {
    const cmd = formatOneClickSellCommand(row.listingId, qty, sellAll);
    this.#pushModalLine(`> ${cmd}`);
    tacsimAudio.playMarketFillChime();
    this.#hooks.sendCommand(cmd);
    this.requestPortTrade();
  }

  #pushModalLine(line: string): void {
    const next = [...this.modalLog, line];
    if (next.length > 64) next.splice(0, next.length - 64);
    this.modalLog = next;
  }

  #maybeAutoRefreshTrade(nowMs: number): void {
    if (this.isAboardShip || !this.#hooks.hasPanel(PORT_TRADE_FEED)) return;
    const entry = (this.serverStationRoomEntry ?? "").toLowerCase();
    const inLiveRoom =
      entry === "room.simulator_lobby" || entry.includes("berth_gallery");
    const modalWantsTrade =
      this.activeModal === "trade" ||
      this.activeModal === "sailing" ||
      this.activeModal === "sim";
    if ((inLiveRoom || modalWantsTrade) && nowMs - this.#lastAutoTradeRefreshAt >= 650) {
      this.#lastAutoTradeRefreshAt = nowMs;
      this.requestPortTrade();
    }
  }

  #currentSessionScopeKey(): string | null {
    return this.#hooks.sessionScopeKey?.() ?? this.#hooks.selfCharacterId();
  }

  #ensureSessionScope(): void {
    const scope = this.#currentSessionScopeKey();
    if (
      scope !== null &&
      this.#activeSessionScopeKey !== null &&
      scope !== this.#activeSessionScopeKey
    ) {
      this.#resetPackCaches();
    }
    if (scope !== null) {
      this.#activeSessionScopeKey = scope;
    }
  }

  #resetPackCaches(): void {
    this.#rawPacks.clear();
    this.#manifestDigests.clear();
    this.#packsInFlight.clear();
    this.#packsFailed.clear();
    this.#packRetryCounts.clear();
    this.#packRetryAfterMs.clear();
    this.#packQueue = [];
    this.#manifestReceived = false;
    this.#manifestInFlightAtMs = null;
    this.#manifestRetryCount = 0;
    this.#desiredStationPackId = "neighborhood.ceres_freeport";
    this.#desiredShipUnitId = "tacsim.unit.dray";
    this.stationPack = buildFallbackFreeport();
    this.shipTemplate = buildFallbackDray("tacsim.unit.dray");
    this.goodsById = buildFallbackGoodsTable();
    if (this.#packDrainTimer !== null) {
      clearTimeout(this.#packDrainTimer);
      this.#packDrainTimer = null;
    }
    if (this.#packRetryTimer !== null) {
      clearTimeout(this.#packRetryTimer);
      this.#packRetryTimer = null;
    }
    this.packsRevision += 1;
  }

  #evictCachedPack(id: string): void {
    this.#rawPacks.delete(id);
    const normUnit = normalizeUnitId(id);
    this.#rawPacks.delete(normUnit);
    for (const [k, v] of [...this.#rawPacks.entries()]) {
      if (v.canonicalId === id) {
        this.#rawPacks.delete(k);
      }
    }
  }

  #authoritativeStationPackId(): string {
    if (this.ambient.atlasPack) return this.ambient.atlasPack;
    if (this.roomView) {
      const res = resolveStationPackAndEntry(this.roomView, this.ambient, this.stationPack);
      if (res.desiredPackId !== this.stationPack.packId) {
        return res.desiredPackId;
      }
    }
    return this.#desiredStationPackId || this.stationPack.packId;
  }

  #schedulePackRetry(id: string, now: number): void {
    if (this.#rawPacks.has(id) || this.#packsFailed.has(id)) return;
    const attempts = (this.#packRetryCounts.get(id) ?? 0) + 1;
    this.#packRetryCounts.set(id, attempts);
    if (attempts > PACK_MAX_RETRIES) {
      this.#packRetryAfterMs.delete(id);
      return;
    }
    const delayMs = Math.min(16_000, PACK_RETRY_BASE_DELAY_MS * Math.pow(2, attempts - 1));
    this.#packRetryAfterMs.set(id, now + delayMs);
    if (!this.#packQueue.includes(id)) {
      this.#packQueue.push(id);
    }
  }

  #expireTimedOutPacks(now: number): void {
    for (const [id, entry] of [...this.#packsInFlight.entries()]) {
      if (now - entry.sentAtMs < PACK_IN_FLIGHT_TIMEOUT_MS) continue;
      this.#packsInFlight.delete(id);
      this.#schedulePackRetry(id, now);
    }
  }

  #schedulePackMaintenance(now: number): void {
    let nextWakeMs = Infinity;
    for (const entry of this.#packsInFlight.values()) {
      const exp = entry.sentAtMs + PACK_IN_FLIGHT_TIMEOUT_MS;
      if (exp < nextWakeMs) nextWakeMs = exp;
    }
    for (const id of this.#packQueue) {
      const retryAt = this.#packRetryAfterMs.get(id);
      if (retryAt !== undefined && retryAt < nextWakeMs) {
        nextWakeMs = retryAt;
      }
    }
    if (this.#packRetryTimer !== null) {
      clearTimeout(this.#packRetryTimer);
      this.#packRetryTimer = null;
    }
    if (!Number.isFinite(nextWakeMs)) return;
    const delay = Math.max(10, nextWakeMs - now);
    this.#packRetryTimer = unrefTimer(
      setTimeout(() => {
        this.#packRetryTimer = null;
        this.#pumpPackQueue(Date.now());
      }, delay),
    );
  }

  #enqueuePackFetches(ids: string[], now = Date.now()): void {
    for (const id of ids) {
      if ((this.#packRetryCounts.get(id) ?? 0) > PACK_MAX_RETRIES) continue;
      if (
        !this.#packQueue.includes(id) &&
        !this.#rawPacks.has(id) &&
        !this.#packsInFlight.has(id) &&
        !this.#packsFailed.has(id)
      ) {
        this.#packQueue.push(id);
      }
    }
    this.#pumpPackQueue(now);
  }

  #pumpPackQueue(now = Date.now()): void {
    this.#expireTimedOutPacks(now);
    if (this.#packDrainTimer === null && this.#packQueue.length > 0) {
      const readyBatch: string[] = [];
      const deferred: string[] = [];
      for (const id of this.#packQueue) {
        if (this.#rawPacks.has(id) || this.#packsInFlight.has(id) || this.#packsFailed.has(id)) {
          continue;
        }
        const retryAfter = this.#packRetryAfterMs.get(id) ?? 0;
        if (readyBatch.length < 3 && retryAfter <= now) {
          readyBatch.push(id);
        } else {
          deferred.push(id);
        }
      }
      this.#packQueue = deferred;

      for (const id of readyBatch) {
        const sent = this.#hooks.subscribePanel(TACSIM_CONTENT_FEED, { op: "pack", id });
        if (sent) {
          this.#packsInFlight.set(id, {
            sentAtMs: now,
            sha256: this.#manifestDigests.get(id) ?? null,
          });
          this.#packRetryAfterMs.delete(id);
        } else {
          this.#schedulePackRetry(id, now);
        }
      }

      const hasMoreReady = this.#packQueue.some(
        (id) => (this.#packRetryAfterMs.get(id) ?? 0) <= now,
      );
      if (hasMoreReady) {
        this.#packDrainTimer = unrefTimer(
          setTimeout(() => {
            this.#packDrainTimer = null;
            this.#pumpPackQueue(Date.now());
          }, 260),
        );
      }
    }
    this.#schedulePackMaintenance(now);
  }

  #rebuildStationPack(packId: string): void {
    const raw = this.#rawPacks.get(packId)?.json;
    const built = raw
      ? parseNeighborhoodPack(raw, packId)
      : packId.includes("ooc")
        ? buildFallbackOocArea()
        : buildFallbackFreeport();

    for (const [k, v] of this.#rawPacks) {
      if (k.startsWith("cast.")) {
        mergeCastPackIntoNeighborhood(built, v.json);
      }
    }
    this.stationPack = built;
  }

  #rebuildShipTemplate(unitId: string): void {
    const norm = normalizeUnitId(unitId);
    const stem = norm.replace("tacsim.unit.", "");
    const raw = this.#rawPacks.get(stem)?.json ?? this.#rawPacks.get(norm)?.json;
    this.shipTemplate = raw ? parseShipUnitJson(raw, norm) : buildFallbackDray(norm);
  }
}
