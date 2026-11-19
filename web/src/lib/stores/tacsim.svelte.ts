// Reactive Svelte 5 store for the Web 3D Station/Ship Viewport & Diegetic Port UI.
// Lazily syncs `tacsim.content` packs and `port.trade` over the existing WebSocket
// without altering `GameApp`'s pinned `hydrationQueries` sequence.

import type { NarrativeKind, ObjRef, RichText, RoomView } from "../proto";
import { plainText } from "../render";
import { storageGet, storageSet } from "../storage";
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
  resolveShipCompartmentFromRoom,
  resolveStationPackAndEntry,
  type AtlasNeighborhoodPack,
  type DecodedRoomAmbient,
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

export const SCENE_3D_PREF_KEY = "rustmush.scene3d";

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

  /** Active speech bubbles & composing flags keyed by actor id or lowercase name. */
  actorSpeech = $state<Record<string, { text: string; expiresAt: number }>>({});
  actorComposing = $state<Record<string, boolean>>({});

  /** Focused 3D interactable prompt (`[E] ...`). */
  focusedPrompt = $state<{ title: string; subtitle: string } | null>(null);

  #hooks: TacsimTransportHooks;
  #rawPacks = new Map<string, string>();
  #manifestRequested = false;
  #packQueue: string[] = [];
  #packDrainTimer: ReturnType<typeof setTimeout> | null = null;
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

  get visibleActors(): VisibleRoomActor[] {
    return filterVisibleRoomActors(
      this.roomView,
      this.ambient,
      this.#hooks.selfCharacterId(),
      this.isAboardShip ? null : this.stationPack,
    );
  }

  get quickActions(): QuickActionItem[] {
    const loc =
      this.physicalLocationKey ??
      (this.isAboardShip ? this.serverShipCompartment : this.serverStationRoomEntry);
    return contextualQuickActions(loc, this.isAboardShip, this.seatedAtBridgeHelm);
  }

  toggle3dView(force?: boolean): void {
    const next = force !== undefined ? force : !this.view3dOpen;
    this.view3dOpen = next;
    storageSet(SCENE_3D_PREF_KEY, next ? "1" : "0");
    if (next) {
      this.ensurePacksSynced();
      this.#maybeAutoRefreshTrade(Date.now());
    }
  }

  ensurePacksSynced(): void {
    if (this.#manifestRequested) return;
    if (!this.#hooks.hasPanel(TACSIM_CONTENT_FEED)) return;
    const sent = this.#hooks.subscribePanel(TACSIM_CONTENT_FEED);
    if (sent) {
      this.#manifestRequested = true;
    }
  }

  clear(): void {
    this.seatedAtBridgeHelm = false;
    this.activeModal = null;
    this.focusedNpc = null;
    this.detailModal = null;
    this.modalLog = [];
    this.roomView = null;
    this.ambient = decodeTacsimRoomAmbient(null);
    this.isAboardShip = false;
    this.serverStationRoomEntry = null;
    this.serverShipCompartment = null;
    this.physicalLocationKey = null;
    this.trade = emptyPortTradeSnapshot();
    this.actorSpeech = {};
    this.actorComposing = {};
    this.focusedPrompt = null;
    this.#manifestRequested = false;
    this.#packQueue = [];
    this.#pendingQuote = null;
    this.#awaitingAppendTradePage = false;
    if (this.#packDrainTimer !== null) {
      clearTimeout(this.#packDrainTimer);
      this.#packDrainTimer = null;
    }
  }

  observeRoom(view: RoomView): void {
    const wasAboard = this.isAboardShip;
    this.roomView = view;
    const dec = decodeTacsimRoomAmbient(view);
    this.ambient = dec;

    if (dec.stowage) {
      this.trade = { ...this.trade, stowage: dec.stowage };
    } else if (dec.ownStowage) {
      this.trade = { ...this.trade, stowage: dec.ownStowage };
    }

    // Check if aboard a ship interior
    const desiredUnit = dec.unit ? normalizeUnitId(dec.unit) : this.shipTemplate.unitId;
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
    if (desiredPackId !== this.stationPack.packId) {
      this.#rebuildStationPack(desiredPackId);
    }
    const resolvedEntry =
      entryId ??
      resolveStationPackAndEntry(view, dec, this.stationPack).entryId;
    this.serverStationRoomEntry = resolvedEntry;

    if (
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
    if (ev.actor && plain.trim().length > 0) {
      const next = { ...this.actorSpeech };
      const exp = Date.now() + 8_500;
      next[ev.actor.id] = { text: plain.trim(), expiresAt: exp };
      next[ev.actor.name.toLowerCase()] = { text: plain.trim(), expiresAt: exp };
      this.actorSpeech = next;
    }
    if (this.activeModal !== null && plain.trim().length > 0) {
      const who = ev.actor?.name ? `${ev.actor.name}: ` : "";
      this.#pushModalLine(`${who}${plain.trim()}`);
    }
    this.#maybeAutoRefreshTrade(Date.now());
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
      this.#hooks.sendCommand(res.acceptCommand);
      this.requestPortTrade();
    } else if (res.priceWarning) {
      this.#pushModalLine(`[PRICE MOVED] ${res.priceWarning}`);
    }
  }

  observeContentFeed(data: unknown): void {
    const obj = asObj(data);
    if (!obj) return;

    // Manifest frame (`packs: [{ id, category, sha256 }, ...]`)
    if (Array.isArray(obj["packs"])) {
      const wanted: string[] = [];
      for (const item of obj["packs"]) {
        const p = asObj(item);
        if (!p) continue;
        const id = typeof p["id"] === "string" ? p["id"] : "";
        const cat = typeof p["category"] === "string" ? p["category"] : "";
        if (!id) continue;
        if (
          cat === "neighborhood" ||
          cat === "cast" ||
          cat === "unit" ||
          cat === "market"
        ) {
          if (!this.#rawPacks.has(id)) {
            wanted.push(id);
          }
        }
      }
      this.#enqueuePackFetches(wanted);
      return;
    }

    // Individual pack frame (`id`, `json`)
    const id = typeof obj["id"] === "string" ? obj["id"] : "";
    const jsonText =
      typeof obj["json"] === "string"
        ? obj["json"]
        : obj["pack"] && typeof obj["pack"] === "object"
          ? JSON.stringify(obj["pack"])
          : "";
    if (!id || !jsonText) return;

    this.#rawPacks.set(id, jsonText);
    if (id === "goods") {
      this.goodsById = parseGoodsJson(jsonText);
    } else if (id.startsWith("neighborhood.")) {
      if (id === this.stationPack.packId) {
        this.#rebuildStationPack(id);
      }
    } else if (id.startsWith("cast.")) {
      this.#rebuildStationPack(this.stationPack.packId);
    } else {
      const normUnit = normalizeUnitId(id);
      if (normUnit === this.shipTemplate.unitId) {
        this.#rebuildShipTemplate(normUnit);
      }
    }
    this.packsRevision += 1;
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
    const started = beginOneClickBuy(row, qty, Date.now());
    if (!started) return;
    this.#pendingQuote = started.latch;
    this.#pushModalLine(`> ${started.quoteCommand}`);
    this.#hooks.sendCommand(started.quoteCommand);
    this.requestPortTrade();
  }

  sellListing(row: PortListingRow, qty: number, sellAll = false): void {
    const cmd = formatOneClickSellCommand(row.listingId, qty, sellAll);
    this.#pushModalLine(`> ${cmd}`);
    this.#hooks.sendCommand(cmd);
    this.requestPortTrade();
  }

  #pushModalLine(line: string): void {
    const next = [...this.modalLog, line];
    if (next.length > 18) next.splice(0, next.length - 18);
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

  #enqueuePackFetches(ids: string[]): void {
    for (const id of ids) {
      if (!this.#packQueue.includes(id) && !this.#rawPacks.has(id)) {
        this.#packQueue.push(id);
      }
    }
    this.#pumpPackQueue();
  }

  #pumpPackQueue(): void {
    if (this.#packDrainTimer !== null || this.#packQueue.length === 0) return;
    const batch = this.#packQueue.splice(0, 3);
    for (const id of batch) {
      this.#hooks.subscribePanel(TACSIM_CONTENT_FEED, { pack: id });
    }
    if (this.#packQueue.length > 0) {
      this.#packDrainTimer = setTimeout(() => {
        this.#packDrainTimer = null;
        this.#pumpPackQueue();
      }, 260);
    }
  }

  #rebuildStationPack(packId: string): void {
    const raw = this.#rawPacks.get(packId);
    const built = raw
      ? parseNeighborhoodPack(raw, packId)
      : packId.includes("ooc")
        ? buildFallbackOocArea()
        : buildFallbackFreeport();

    for (const [k, v] of this.#rawPacks) {
      if (k.startsWith("cast.")) {
        mergeCastPackIntoNeighborhood(built, v);
      }
    }
    this.stationPack = built;
  }

  #rebuildShipTemplate(unitId: string): void {
    const norm = normalizeUnitId(unitId);
    const stem = norm.replace("tacsim.unit.", "");
    const raw = this.#rawPacks.get(stem) ?? this.#rawPacks.get(norm);
    this.shipTemplate = raw ? parseShipUnitJson(raw, norm) : buildFallbackDray(norm);
  }
}
