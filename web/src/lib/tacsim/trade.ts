// Structured Corridor Port Trade (`port.trade`), One-Click Quote-to-Accept state
// machine, Sailing Plan route presets, and NPC dialogue prompt builders.
// Mirrors Unity's `TacSimTradeAndShipState.cs` and `TacSimGameCoordinator.cs`.

import {
  parseStowageNode,
  type DockedHullSummary,
  type SimulatorSlotInfo,
  type StowageSnapshot,
} from "./spatial";

export const PORT_TRADE_FEED = "port.trade";
export const TACSIM_CONTENT_FEED = "tacsim.content";

export interface GoodItemDef {
  id: string;
  label: string;
  goodClass: string;
  ledgerUnit: string;
  massKgPerUnit: number;
  volumeLPerUnit: number;
  stowage: string;
  isConsumable: boolean;
  referencePriceCr: number;
}

export interface PortListingRow {
  listingId: number;
  side: "ask" | "bid";
  goodId: string;
  itemName: string;
  vendorName: string;
  venue: string;
  quantity: number;
  unit: string;
  unitPriceMinor: number;
  unitPriceText: string;
  funded: boolean;
}

export interface OpenBuyerQuote {
  quoteId: number;
  listingId: number;
  quantity: number;
  unitPriceMinor: number;
  totalMinor: number;
}

export interface BerthGallerySnapshot {
  portEntry: string | null;
  dockedCount: number;
  dockedHulls: DockedHullSummary[];
  ownHull: string | null;
  ownHullName: string | null;
  ownHullDockedHere: boolean;
}

export interface PortTradeSnapshot {
  venueId: string;
  venueLabel: string;
  portName: string;
  creditsMinor: number;
  creditsText: string;
  asks: PortListingRow[];
  bids: PortListingRow[];
  hasMore: boolean;
  nextAfterId: number | null;
  quotes: OpenBuyerQuote[];
  stowage: StowageSnapshot;
  /** The helm hull's dbref as the server writes it (`#123`); `null` with no hull. */
  hullId: string | null;
  hullName: string | null;
  hullDocked: boolean;
  hullPort: string | null;
  berth: BerthGallerySnapshot | null;
  simulator: {
    matchState: string;
    aiFill: number;
    myTeam: string;
    myUnit: string;
    slots: SimulatorSlotInfo[];
  } | null;
}

export interface PendingQuoteLatch {
  listingId: number;
  quantity: number;
  maxUnitPriceMinor: number;
  maxTotalMinor: number;
  preExistingQuoteIds: ReadonlySet<number>;
  requestedAtMs: number;
}

/**
 * The lane's venue words (48e L8, clients:G7): every `port` entry's `venue` in
 * `crates/tacsim/content/lane/ceres_phobos.json`, with its location's label.
 * `+ship/undock`, `+haul/post to=` and `+escort/post to=` take exactly these;
 * `kirkwood`, `vesta` and `aitken` name no venue and were refused.
 */
export const CORRIDOR_DESTINATIONS: ReadonlyArray<{ id: string; label: string; body: string }> = [
  { id: "ceres", label: "Ceres Freeport", body: "Ceres · port" },
  { id: "stickney", label: "Stickney Yards", body: "Phobos · port" },
  { id: "luna", label: "Aitken Highport", body: "Luna · port" },
  { id: "deimos", label: "Voltaire Anchorage", body: "Deimos · port" },
  { id: "titan", label: "Ligeia Terminal", body: "Titan · port" },
  { id: "commons", label: "Freeport Commons", body: "Ceres · extraction" },
  { id: "kerwan", label: "Kerwan Floor", body: "Ceres · extraction" },
  { id: "ahuna", label: "Ahuna Flank", body: "Ceres · extraction" },
  { id: "juling", label: "Juling Shadow", body: "Ceres · extraction" },
  { id: "occator", label: "Occator Rim", body: "Ceres · extraction" },
  { id: "rheasilvia", label: "Rheasilvia Commons", body: "Vesta · extraction" },
  { id: "urvara", label: "Urvara Reach", body: "Ceres · extraction" },
];

function asObj(v: unknown): Record<string, unknown> | null {
  return typeof v === "object" && v !== null && !Array.isArray(v)
    ? (v as Record<string, unknown>)
    : null;
}

function asStr(v: unknown, fallback = ""): string {
  return typeof v === "string" ? v : fallback;
}

function asNum(v: unknown, fallback = 0): number {
  return typeof v === "number" && Number.isFinite(v) ? v : fallback;
}

function asBool(v: unknown, fallback = false): boolean {
  return typeof v === "boolean" ? v : fallback;
}

function asArr(v: unknown): unknown[] | null {
  return Array.isArray(v) ? v : null;
}

export function buildFallbackGoodsTable(): Map<string, GoodItemDef> {
  const list: GoodItemDef[] = [
    { id: "metals", label: "metal concentrate", goodClass: "trade", ledgerUnit: "t", massKgPerUnit: 1000, volumeLPerUnit: 400, stowage: "stow.bulk", isConsumable: false, referencePriceCr: 200 },
    { id: "volatiles", label: "water ice", goodClass: "trade", ledgerUnit: "t", massKgPerUnit: 1000, volumeLPerUnit: 1400, stowage: "stow.bulk", isConsumable: false, referencePriceCr: 100 },
    { id: "structural_alloys", label: "structural alloy stock", goodClass: "trade", ledgerUnit: "t", massKgPerUnit: 1000, volumeLPerUnit: 300, stowage: "stow.bulk", isConsumable: false, referencePriceCr: 400 },
    { id: "industrial_parts", label: "yard fittings, crated", goodClass: "trade", ledgerUnit: "t", massKgPerUnit: 1000, volumeLPerUnit: 2300, stowage: "stow.bulk", isConsumable: false, referencePriceCr: 400 },
    { id: "propellant", label: "reaction mass", goodClass: "trade", ledgerUnit: "t", massKgPerUnit: 1000, volumeLPerUnit: 1000, stowage: "stow.propellant", isConsumable: true, referencePriceCr: 40 },
    { id: "provisions", label: "voyage provisions", goodClass: "ship_supply", ledgerUnit: "crew_day", massKgPerUnit: 4, volumeLPerUnit: 10, stowage: "stow.provisions", isConsumable: true, referencePriceCr: 10 },
  ];
  return new Map(list.map((g) => [g.id, g]));
}

export function parseGoodsJson(rawJson: string): Map<string, GoodItemDef> {
  let parsed: unknown;
  try {
    parsed = JSON.parse(rawJson);
  } catch {
    return buildFallbackGoodsTable();
  }
  const root = asObj(parsed);
  const goods = root ? asArr(root["goods"]) : null;
  if (!goods || goods.length === 0) return buildFallbackGoodsTable();

  const out = new Map<string, GoodItemDef>();
  for (const gVal of goods) {
    const g = asObj(gVal);
    if (!g) continue;
    const id = asStr(g["id"]);
    if (!id) continue;
    const measures = asObj(g["measures"]);
    const consumable = asStr(g["consumable"]);
    out.set(id, {
      id,
      label: asStr(g["label"], id),
      goodClass: asStr(g["class"], "trade"),
      ledgerUnit: asStr(g["ledger_unit"], "t"),
      massKgPerUnit: measures ? asNum(measures["si.mass_kg"], 0) : 0,
      volumeLPerUnit: measures ? asNum(measures["si.volume_l"], 0) : 0,
      stowage: asStr(g["stowage"], "stow.bulk"),
      isConsumable: Boolean(consumable && consumable !== "null"),
      referencePriceCr: asNum(g["reference_major"], 100),
    });
  }
  return out.size > 0 ? out : buildFallbackGoodsTable();
}

export function emptyStowageSnapshot(): StowageSnapshot {
  return {
    cargoKg: 0,
    cargoMaxKg: 100_000,
    cargoL: 0,
    cargoMaxL: 120_000,
    propellantKg: 0,
    propellantMaxKg: 80_000,
    provisionsCrewDays: 0,
    provisionsMaxCrewDays: 120,
    cargoLines: [],
  };
}

export function emptyPortTradeSnapshot(): PortTradeSnapshot {
  return {
    venueId: "",
    venueLabel: "Port Commodity Exchange",
    portName: "",
    creditsMinor: 0,
    creditsText: "0.00 Cr",
    asks: [],
    bids: [],
    hasMore: false,
    nextAfterId: null,
    quotes: [],
    stowage: emptyStowageSnapshot(),
    hullId: null,
    hullName: null,
    hullDocked: true,
    hullPort: null,
    berth: null,
    simulator: null,
  };
}

export function formatMinorCredits(minor: number): string {
  const cr = minor / 100;
  return `${cr.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 })} Cr`;
}

function parseListingRow(
  rowVal: unknown,
  goodsById: ReadonlyMap<string, GoodItemDef>,
  fallbackVenue: string,
  defaultSide: "ask" | "bid" = "ask",
): PortListingRow | null {
  const r = asObj(rowVal);
  if (!r) return null;
  const listingId = Math.trunc(asNum(r["id"], asNum(r["listing_id"], 0)));
  if (listingId <= 0) return null;

  const sideRaw = asStr(r["side"], defaultSide).toLowerCase();
  const side: "ask" | "bid" = sideRaw === "bid" ? "bid" : "ask";
  const goodId = asStr(r["good"], asStr(r["item_id"], asStr(r["sku"])));
  const goodDef =
    goodsById.get(goodId) ?? goodsById.get(goodId.replace(/^tacsim:/i, ""));
  const itemName =
    asStr(r["good_label"]) ||
    asStr(r["label"]) ||
    asStr(r["item_name"]) ||
    (goodDef ? goodDef.label : goodId || `Lot #${listingId}`);
  const unitPriceMinor = Math.round(
    asNum(
      r["unit_price_minor"],
      asNum(r["execution_price_minor"], asNum(r["price_minor"], 0)),
    ),
  );
  const unitPriceText =
    asStr(r["price_text"]) ||
    asStr(r["execution_price_text"]) ||
    formatMinorCredits(unitPriceMinor);

  return {
    listingId,
    side,
    goodId,
    itemName,
    vendorName:
      asStr(r["vendor_name"]) ||
      asStr(r["vendor"]) ||
      asStr(r["party"]) ||
      fallbackVenue,
    venue: asStr(r["venue"], fallbackVenue),
    quantity: Math.max(
      0,
      Math.trunc(asNum(r["available_qty"], asNum(r["qty"], asNum(r["quantity"], 0)))),
    ),
    unit: asStr(r["unit"], goodDef ? goodDef.ledgerUnit : "t"),
    unitPriceMinor,
    unitPriceText,
    funded: asBool(r["funded"], true),
  };
}

/**
 * Decodes a `port.trade` keyframe as the server builds it
 * (`tacsim::client_sync::build_port_trade_frame`; the shared fixture
 * `fixtures/port/port_trade.json` is a real frame, held to the live shape by
 * tacsim's `port_trade_frame_matches_the_clients_shared_fixture`). 48e L8
 * (clients:G2): reads the server's exact keys — `balance_minor`, `port_name`,
 * `listings[].available_qty`, `next_after`, `open_quotes`, `ship.hull_name`,
 * `ship.hull_id`, `ship.stowage`, `berth.docked_hulls` — cleanly.
 */
export function decodePortTradeFeed(
  data: unknown,
  goodsById: ReadonlyMap<string, GoodItemDef>,
  previous: PortTradeSnapshot,
  appendPage = false,
): PortTradeSnapshot {
  const root = asObj(data);
  if (!root) return previous;

  const venueObj = asObj(root["venue"]);
  const venueId = venueObj
    ? asStr(venueObj["id"], asStr(root["venue_id"], previous.venueId))
    : typeof root["venue"] === "string"
      ? root["venue"]
      : "venue" in root && root["venue"] === null
        ? ""
        : previous.venueId;
  const hasPortField =
    "port_name" in root || "aboard_docked_port" in root || "port" in root;
  const portName =
    asStr(root["port_name"]) ||
    asStr(root["aboard_docked_port"]) ||
    asStr(root["port"]) ||
    (venueObj ? asStr(venueObj["port"]) : "") ||
    (hasPortField ? "" : previous.portName);
  const venueLabel =
    (venueObj ? asStr(venueObj["label"], asStr(venueObj["name"])) : "") ||
    asStr(root["venue_label"]) ||
    portName ||
    venueId ||
    previous.venueLabel;

  const creditObj = asObj(root["credit"]) ?? asObj(root["account"]);
  const creditsMinor =
    typeof root["balance_minor"] === "number" && Number.isFinite(root["balance_minor"])
      ? Math.round(root["balance_minor"])
      : creditObj
        ? Math.round(asNum(creditObj["minor"], asNum(creditObj["balance_minor"], previous.creditsMinor)))
        : Math.round(asNum(root["credits_minor"], previous.creditsMinor));
  const creditsText =
    asStr(root["balance_text"]) ||
    (creditObj ? asStr(creditObj["text"]) : "") ||
    asStr(root["credits_text"]) ||
    formatMinorCredits(creditsMinor);

  const hasListingsField =
    Array.isArray(root["listings"]) ||
    Array.isArray(root["asks"]) ||
    Array.isArray(root["bids"]);
  const asks: PortListingRow[] = appendPage || !hasListingsField ? [...previous.asks] : [];
  const bids: PortListingRow[] = appendPage || !hasListingsField ? [...previous.bids] : [];
  const seenIds = new Set<number>([...asks.map((a) => a.listingId), ...bids.map((b) => b.listingId)]);

  for (const item of asArr(root["listings"]) ?? []) {
    const parsed = parseListingRow(item, goodsById, venueLabel, "ask");
    if (!parsed || seenIds.has(parsed.listingId)) continue;
    seenIds.add(parsed.listingId);
    if (parsed.side === "bid") bids.push(parsed);
    else asks.push(parsed);
  }
  for (const item of asArr(root["asks"]) ?? []) {
    const parsed = parseListingRow(item, goodsById, venueLabel, "ask");
    if (parsed && !seenIds.has(parsed.listingId)) {
      seenIds.add(parsed.listingId);
      asks.push(parsed);
    }
  }
  for (const item of asArr(root["bids"]) ?? []) {
    const parsed = parseListingRow(item, goodsById, venueLabel, "bid");
    if (parsed && !seenIds.has(parsed.listingId)) {
      seenIds.add(parsed.listingId);
      bids.push(parsed);
    }
  }

  const quotes: OpenBuyerQuote[] = [];
  const quotesArr = asArr(root["open_quotes"]) ?? asArr(root["quotes"]);
  if (quotesArr) {
    for (const qVal of quotesArr) {
      const q = asObj(qVal);
      if (!q) continue;
      const quoteId = Math.trunc(asNum(q["quote_id"], asNum(q["id"], 0)));
      const listingId = Math.trunc(asNum(q["listing_id"], asNum(q["listing"], 0)));
      const quantity = Math.trunc(asNum(q["quantity"], asNum(q["qty"], 0)));
      if (quoteId <= 0 || listingId <= 0 || quantity <= 0) continue;
      const unitPriceMinor = Math.round(
        asNum(q["unit_price_minor"], asNum(q["execution_price_minor"], 0)),
      );
      const totalMinor = Math.round(asNum(q["total_minor"], unitPriceMinor * quantity));
      quotes.push({ quoteId, listingId, quantity, unitPriceMinor, totalMinor });
    }
  }

  const hullObj = asObj(root["ship"]) ?? asObj(root["hull"]);
  const shipCleared = "ship" in root && root["ship"] === null && !hullObj;
  const stowage =
    parseStowageNode(root["stowage"], goodsById) ??
    (hullObj ? parseStowageNode(hullObj["stowage"], goodsById) : null) ??
    previous.stowage;

  let berth: BerthGallerySnapshot | null = previous.berth;
  const berthObj = asObj(root["berth"]);
  if (berthObj) {
    const dockedHulls: DockedHullSummary[] = [];
    const dhArr = asArr(berthObj["docked_hulls"]);
    const ownHull = asStr(berthObj["own_hull"]) || null;
    if (dhArr) {
      for (const hVal of dhArr) {
        const h = asObj(hVal);
        if (!h) continue;
        const id = asStr(h["id"]);
        dockedHulls.push({
          id,
          name: asStr(h["name"]),
          mark: asStr(h["mark"]),
          unit: asStr(h["unit"], "dray"),
          captain: asStr(h["captain"]),
          yours: asBool(h["yours"], Boolean(ownHull && id === ownHull)),
        });
      }
    }
    berth = {
      portEntry: asStr(berthObj["port_entry"]) || null,
      dockedCount: Math.trunc(asNum(berthObj["docked_count"], dockedHulls.length)),
      dockedHulls,
      ownHull,
      ownHullName: asStr(berthObj["own_hull_name"]) || null,
      ownHullDockedHere: asBool(berthObj["own_hull_docked_here"], false),
    };
  } else if ("berth" in root && root["berth"] === null) {
    berth = null;
  }

  let simulator: PortTradeSnapshot["simulator"] = previous.simulator;
  const simObj = asObj(root["simulator"]);
  if (simObj) {
    const slots: SimulatorSlotInfo[] = [];
    const sArr = asArr(simObj["slots"]);
    if (sArr) {
      for (const sVal of sArr) {
        const s = asObj(sVal);
        if (!s) continue;
        slots.push({
          index: Math.trunc(asNum(s["index"], 1)),
          team: asStr(s["team"], "red"),
          pilotName: asStr(s["pilot_name"], asStr(s["name"])),
          unit: asStr(s["unit"], "line_tank"),
          seated: asBool(s["seated"], false),
        });
      }
    }
    simulator = {
      matchState: asStr(simObj["match_state"], "staging"),
      aiFill: Math.trunc(asNum(simObj["ai_fill"], 0)),
      myTeam: asStr(simObj["my_team"]),
      myUnit: asStr(simObj["my_unit"]).replace("tacsim.unit.", ""),
      slots,
    };
  } else if ("simulator" in root && root["simulator"] === null) {
    simulator = null;
  }

  let hasMore: boolean;
  let nextAfterId: number | null;
  if ("next_after" in root) {
    const na = root["next_after"];
    if (typeof na === "number" && Number.isFinite(na) && na > 0) {
      hasMore = true;
      nextAfterId = Math.trunc(na);
    } else {
      hasMore = false;
      nextAfterId = null;
    }
  } else {
    const allLoaded = [...asks, ...bids];
    const maxId = allLoaded.reduce((acc, r) => (r.listingId > acc ? r.listingId : acc), 0);
    hasMore = asBool(root["has_more"], false);
    nextAfterId = hasMore && maxId > 0 ? maxId : null;
  }

  return {
    venueId,
    venueLabel,
    portName,
    creditsMinor,
    creditsText,
    asks,
    bids,
    hasMore,
    nextAfterId,
    quotes,
    stowage,
    hullId: hullObj
      ? asStr(hullObj["hull_id"]) || previous.hullId
      : shipCleared
        ? null
        : previous.hullId,
    hullName: hullObj
      ? asStr(hullObj["hull_name"], asStr(hullObj["name"])) || previous.hullName
      : shipCleared
        ? null
        : previous.hullName,
    hullDocked: hullObj ? asBool(hullObj["docked"], previous.hullDocked) : previous.hullDocked,
    hullPort: hullObj
      ? asStr(hullObj["port"]) || null
      : shipCleared
        ? null
        : previous.hullPort,
    berth,
    simulator,
  };
}

export function computeMaxFitQuantity(
  row: PortListingRow,
  goodsById: ReadonlyMap<string, GoodItemDef>,
  stowage: StowageSnapshot,
  creditsMinor: number,
): number {
  let cap = Math.max(1, row.quantity);
  if (row.unitPriceMinor > 0 && creditsMinor > 0) {
    cap = Math.min(cap, Math.max(1, Math.floor(creditsMinor / row.unitPriceMinor)));
  }
  const good =
    goodsById.get(row.goodId) ?? goodsById.get(row.goodId.replace(/^tacsim:/i, ""));
  if (good) {
    if (good.stowage === "stow.propellant") {
      const remKg = Math.max(0, stowage.propellantMaxKg - stowage.propellantKg);
      if (good.massKgPerUnit > 0) cap = Math.min(cap, Math.max(1, Math.floor(remKg / good.massKgPerUnit)));
    } else if (good.stowage === "stow.provisions") {
      const remDays = Math.max(0, stowage.provisionsMaxCrewDays - stowage.provisionsCrewDays);
      cap = Math.min(cap, Math.max(1, Math.floor(remDays)));
    } else {
      const remKg = Math.max(0, stowage.cargoMaxKg - stowage.cargoKg);
      const remL = Math.max(0, stowage.cargoMaxL - stowage.cargoL);
      if (good.massKgPerUnit > 0) cap = Math.min(cap, Math.max(1, Math.floor(remKg / good.massKgPerUnit)));
      if (good.volumeLPerUnit > 0) cap = Math.min(cap, Math.max(1, Math.floor(remL / good.volumeLPerUnit)));
    }
  }
  return Math.max(1, cap);
}

export function beginOneClickBuy(
  row: PortListingRow,
  quantity: number,
  nowMs: number,
  existingQuotes: ReadonlyArray<Pick<OpenBuyerQuote, "quoteId">> = [],
): { quoteCommand: string; latch: PendingQuoteLatch } | null {
  if (row.listingId <= 0 || row.unitPriceMinor <= 0) return null;
  const qty = Math.max(1, Math.min(row.quantity || quantity, Math.trunc(quantity)));
  return {
    quoteCommand: `+market/quote ${row.listingId}=${qty}`,
    latch: {
      listingId: row.listingId,
      quantity: qty,
      maxUnitPriceMinor: row.unitPriceMinor,
      maxTotalMinor: row.unitPriceMinor * qty,
      preExistingQuoteIds: new Set(existingQuotes.map((q) => q.quoteId)),
      requestedAtMs: nowMs,
    },
  };
}

export function resolvePendingQuoteFromFeed(
  snapshot: PortTradeSnapshot,
  latch: PendingQuoteLatch | null,
  nowMs: number,
): { acceptCommand: string | null; priceWarning: string | null; clearLatch: boolean } {
  if (!latch) return { acceptCommand: null, priceWarning: null, clearLatch: false };
  if (nowMs - latch.requestedAtMs > 12_000) {
    return { acceptCommand: null, priceWarning: null, clearLatch: true };
  }

  const freshMatches = snapshot.quotes.filter(
    (q) =>
      !latch.preExistingQuoteIds.has(q.quoteId) &&
      q.listingId === latch.listingId &&
      q.quantity === latch.quantity,
  );
  if (freshMatches.length === 0) {
    return { acceptCommand: null, priceWarning: null, clearLatch: false };
  }

  const matched = freshMatches.reduce((best, q) => (q.quoteId > best.quoteId ? q : best));
  const authorizedMaxTotal =
    typeof latch.maxTotalMinor === "number" && Number.isFinite(latch.maxTotalMinor)
      ? latch.maxTotalMinor
      : latch.maxUnitPriceMinor * latch.quantity;

  if (
    matched.unitPriceMinor <= 0 ||
    matched.totalMinor <= 0 ||
    matched.unitPriceMinor > latch.maxUnitPriceMinor ||
    matched.totalMinor > authorizedMaxTotal
  ) {
    return {
      acceptCommand: null,
      priceWarning: `Quote #${matched.quoteId} moved to ${formatMinorCredits(matched.unitPriceMinor)} (total ${formatMinorCredits(matched.totalMinor)}, above authorized ${formatMinorCredits(authorizedMaxTotal)}) — review before accepting.`,
      clearLatch: true,
    };
  }

  return {
    acceptCommand: `+market/accept ${matched.quoteId}`,
    priceWarning: null,
    clearLatch: true,
  };
}

export function formatOneClickSellCommand(
  listingId: number,
  quantity: number,
  sellAll = false,
): string {
  if (sellAll) return `+market/sell ${listingId}=all`;
  return `+market/sell ${listingId}=${Math.max(1, Math.trunc(quantity))}`;
}

export function suggestUniqueHullName(seed = Date.now()): string {
  const prefixes = ["Drift", "Quiet", "Cinder", "Pale", "Copper", "Vesta", "Tether", "Iron"];
  const nouns = ["Lantern", "Kestrel", "Barge", "Moth", "Spindle", "Compass", "Anchor", "Cairn"];
  const n = Math.abs(Math.trunc(seed));
  const p = prefixes[n % prefixes.length] ?? "Drift";
  const noun = nouns[Math.floor(n / 13) % nouns.length] ?? "Lantern";
  const num = (n % 89) + 10;
  return `${p} ${noun} ${num}`;
}

export function buildNpcTopics(npcName: string): Array<{ label: string; command: string }> {
  const low = npcName.toLowerCase();
  if (low.includes("ilse")) {
    return [
      { label: "How do I enroll a Dray?", command: `say ${npcName}, how do I enroll for a Co-op Dray?` },
      { label: "Explain the lay & buyout", command: `say ${npcName}, how do the lay and buyout work?` },
      { label: "Check my statement", command: `say ${npcName}, can you check where my Co-op statement stands?` },
    ];
  }
  if (low.includes("qureshi")) {
    return [
      { label: "Ask about volatiles & fuel", command: `say ${npcName}, what are Occator's volatiles and reaction mass lines looking like?` },
      { label: "Ask about +survey work", command: `say ${npcName}, what survey commissions is Occator running?` },
    ];
  }
  if (low.includes("margit") || low.includes("søndergaard") || low.includes("sondergaard")) {
    return [
      { label: "Ask about concentrate & alloy", command: `say ${npcName}, how are the concentrate ask and Freeport Works alloy bid looking?` },
      { label: "Which claims consign here?", command: `say ${npcName}, which Belt claims consigned the concentrate on the board?` },
    ];
  }
  if (low.includes("dov")) {
    return [
      { label: "Ask about provisions cases", command: `say ${npcName}, how many cases of provisions do I need for Stickney?` },
      { label: "Ask about bunkering schedule", command: `say ${npcName}, how do I put reaction mass on my loading schedule?` },
    ];
  }
  if (low.includes("halvorsen")) {
    return [
      { label: "Ask about Clearing & +credit", command: `say ${npcName}, how does Ceres Clearing handle pilot accounts?` },
      { label: "What is dry money?", command: `say ${npcName}, what do people mean by dry money versus wet money?` },
    ];
  }
  if (low.includes("mbatha")) {
    return [
      { label: "Ask about hull valuation", command: `say ${npcName}, how does Tycho Mutual value a Dray on the board?` },
      { label: "Ask about Re-float Program", command: `say ${npcName}, how does the Re-float Program work?` },
    ];
  }
  return [
    { label: "Ask about First Captain hulls", command: `say ${npcName}, how do I charter a Co-op Dray?` },
    { label: "Ask about Stickney & Kirkwood lane", command: `say ${npcName}, how is the lane to Stickney looking?` },
  ];
}
