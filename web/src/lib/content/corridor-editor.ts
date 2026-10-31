// Pure data models, entry filters, and JSON patchers for Cut CS.4 & CS.5b —
// Corridor (`lane/ceres_phobos.json`) & Relays (`relays/luna_ceres.json`,
// `relays/manufacture-v1.json`) Studio (doc 34 §5.2, D-34-6).
import type { JsonValue } from "../proto";

export type CorridorFilter =
  | "all"
  | "port"
  | "route"
  | "location"
  | "body"
  | "relay";

export interface CorridorEntrySummary {
  id: string;
  kind: string;
  label: string;
  subtitle: string;
  readOnly: boolean;
  def: Record<string, unknown>;
}

export interface PortOrgDraft {
  id: string;
  name: string;
  capital_major: number;
  faction: string;
}

export interface VendorStockLine {
  good: string;
  qty: number;
  grade: number;
  issue: number;
}

export interface VendorAskLine {
  good: string;
  price_major: number;
  boards: string[];
}

export interface VendorBidLine {
  good: string;
  price_major: number;
  intake: number;
  boards: string[];
}

export interface VendorMillLine {
  ore: string;
  into: string;
  ratio_ore: number;
  ratio_product: number;
  batch_t: number;
  per_ic_day_t: number;
}

export interface PortVendorDraft {
  org: string;
  storefront: string;
  agent: string;
  signatory: string;
  stock: VendorStockLine[];
  asks: VendorAskLine[];
  bids: VendorBidLine[];
  mill: VendorMillLine[];
}

export interface SiteBlockDraft {
  id: string;
  tenure: string;
  holder: string;
  prospectivity: number;
  worked_permille: number;
}

export interface PortSiteDraft {
  name: string;
  issuer: string;
  zone: string;
  defaultTenure: string;
  blocks: SiteBlockDraft[];
  hasYard: boolean;
  yardOrg: string;
  yardName: string;
  yardUnit: string;
  yardRigs: number;
  yardOre: string;
  yardRearmSecs: number;
}

export interface RelayNodeDraft {
  id: string;
  label: string;
  placePack: string;
  placeEntry: string;
  coverageCount: number;
}

export interface RelayPairDraft {
  id: string;
  capacityUnitsPerIcSecond: number;
  classPolicy: string;
  nodeA: string;
  nodeB: string;
}

export interface RelayRecipeLine {
  commodity: string;
  units: number;
}

export interface RelayRecipeDraft {
  label: string;
  workUnits: number;
  requires: string[];
  inputs: RelayRecipeLine[];
  outputs: RelayRecipeLine[];
}

function asRecord(val: unknown): Record<string, unknown> {
  return val && typeof val === "object" && !Array.isArray(val)
    ? (val as Record<string, unknown>)
    : {};
}

export function extractCorridorEntries(
  doc: JsonValue | null,
): CorridorEntrySummary[] {
  const root = asRecord(doc);
  const entries = asRecord(root.entries);
  const out: CorridorEntrySummary[] = [];

  for (const [id, raw] of Object.entries(entries)) {
    const entry = asRecord(raw);
    const kind = typeof entry.kind === "string" ? entry.kind : "entry";
    const def = asRecord(entry.def);

    if (kind === "port") {
      const venue = typeof def.venue === "string" ? def.venue : id;
      const area = typeof def.area === "string" ? def.area : "";
      const vendors = Array.isArray(def.vendors) ? def.vendors.length : 0;
      const orgs = Array.isArray(def.orgs) ? def.orgs.length : 0;
      const hasSite = Boolean(def.site);
      out.push({
        id,
        kind,
        label: venue,
        subtitle: `${area} · ${orgs} orgs · ${vendors} vendors${hasSite ? " · mining site" : ""}`,
        readOnly: false,
        def,
      });
    } else if (kind === "route") {
      const origin =
        typeof def.origin === "string"
          ? def.origin
          : typeof def.from === "string"
            ? def.from
            : "?";
      const dest =
        typeof def.destination === "string"
          ? def.destination
          : typeof def.to === "string"
            ? def.to
            : "?";
      const legs = Array.isArray(def.legs) ? def.legs : [];
      const legsDur = legs.reduce((acc, l) => {
        const rec = asRecord(l);
        return acc + (typeof rec.base_s === "number" ? rec.base_s : 0);
      }, 0);
      const durS =
        typeof def.duration_seconds === "number"
          ? def.duration_seconds
          : legsDur;
      const days = (durS / 86400).toFixed(1);
      out.push({
        id,
        kind,
        label: `${origin} → ${dest}`,
        subtitle: `${days} IC days (${durS} s)`,
        readOnly: false,
        def,
      });
    } else if (kind === "location") {
      const role = typeof def.role === "string" ? def.role : "place";
      const region =
        typeof def.region === "string"
          ? def.region
          : typeof def.body === "string"
            ? def.body
            : "sol";
      out.push({
        id,
        kind,
        label: typeof def.label === "string" ? def.label : id,
        subtitle: `role: ${role} · region: ${region}`,
        readOnly: false,
        def,
      });
    } else if (kind === "body") {
      const elements = Array.isArray(def.elements) ? def.elements.length : 0;
      out.push({
        id,
        kind,
        label: id,
        subtitle: `JPL Horizons ephemeris (${elements} epochs, read-only)`,
        readOnly: true,
        def,
      });
    } else {
      // Relay network (`network`), recipe (`private_pair`), or `region` (`sol`)
      const nodes = Object.keys(asRecord(def.nodes)).length;
      const pairs = Object.keys(asRecord(def.pairs)).length;
      const label =
        typeof def.label === "string"
          ? def.label
          : nodes > 0
            ? `Relay Network (${nodes} nodes, ${pairs} pairs)`
            : id;
      out.push({
        id,
        kind:
          nodes > 0 || kind === "recipe" || id === "network" ? "relay" : kind,
        label,
        subtitle: `kind: ${kind}`,
        readOnly: false,
        def,
      });
    }
  }
  return out;
}

/** Update a single entry's `def` inside `lane/ceres_phobos.json` or `relays/*.json`. */
export function updateCorridorEntryDef(
  doc: JsonValue,
  entryId: string,
  nextDef: Record<string, unknown>,
): JsonValue {
  const clone = JSON.parse(JSON.stringify(doc)) as Record<string, unknown>;
  const entries = asRecord(clone.entries);
  const existing = asRecord(entries[entryId]);
  entries[entryId] = {
    ...existing,
    def: nextDef,
  };
  clone.entries = entries;
  return clone as JsonValue;
}

/** Extract structured `orgs[]` from a port `def`. */
export function extractPortOrgs(def: Record<string, unknown>): PortOrgDraft[] {
  const rawOrgs = Array.isArray(def.orgs) ? def.orgs : [];
  return rawOrgs.map((item) => {
    const rec = asRecord(item);
    return {
      id: typeof rec.id === "string" ? rec.id : "",
      name: typeof rec.name === "string" ? rec.name : "",
      capital_major:
        typeof rec.capital_major === "number" ? rec.capital_major : 50000,
      faction: typeof rec.faction === "string" ? rec.faction : "freeport",
    };
  });
}

/** Update an org in `def.orgs[]` while preserving any extra fields (such as `raises`). */
export function updatePortOrg(
  def: Record<string, unknown>,
  index: number,
  patch: Partial<PortOrgDraft>,
): Record<string, unknown> {
  const next = JSON.parse(JSON.stringify(def)) as Record<string, unknown>;
  const orgs = Array.isArray(next.orgs) ? [...next.orgs] : [];
  const existing = asRecord(orgs[index]);
  orgs[index] = {
    ...existing,
    ...patch,
  };
  next.orgs = orgs;
  return next;
}

/** Append a new org desk to `def.orgs[]`. */
export function addPortOrg(
  def: Record<string, unknown>,
  org?: Partial<PortOrgDraft>,
): Record<string, unknown> {
  const next = JSON.parse(JSON.stringify(def)) as Record<string, unknown>;
  const orgs = Array.isArray(next.orgs) ? [...next.orgs] : [];
  orgs.push({
    id: org?.id ?? `org_${orgs.length + 1}`,
    name: org?.name ?? "New Port Organization",
    capital_major: org?.capital_major ?? 50000,
    faction: org?.faction ?? "freeport",
  });
  next.orgs = orgs;
  return next;
}

/** Remove an org desk from `def.orgs[]`. */
export function removePortOrg(
  def: Record<string, unknown>,
  index: number,
): Record<string, unknown> {
  const next = JSON.parse(JSON.stringify(def)) as Record<string, unknown>;
  const orgs = Array.isArray(next.orgs) ? [...next.orgs] : [];
  orgs.splice(index, 1);
  next.orgs = orgs;
  return next;
}

/** Extract structured `vendors[]` from a port `def`. */
export function extractPortVendors(
  def: Record<string, unknown>,
): PortVendorDraft[] {
  const rawVendors = Array.isArray(def.vendors) ? def.vendors : [];
  return rawVendors.map((item) => {
    const rec = asRecord(item);
    const stock = (Array.isArray(rec.stock) ? rec.stock : []).map((s) => {
      const sr = asRecord(s);
      return {
        good: typeof sr.good === "string" ? sr.good : "metals",
        qty: typeof sr.qty === "number" ? sr.qty : 100,
        grade: typeof sr.grade === "number" ? sr.grade : 50,
        issue: typeof sr.issue === "number" ? sr.issue : 1,
      };
    });
    const asks = (Array.isArray(rec.asks) ? rec.asks : []).map((a) => {
      const ar = asRecord(a);
      return {
        good: typeof ar.good === "string" ? ar.good : "metals",
        price_major: typeof ar.price_major === "number" ? ar.price_major : 100,
        boards: Array.isArray(ar.boards)
          ? ar.boards.filter((b): b is string => typeof b === "string")
          : ["exchange"],
      };
    });
    const bids = (Array.isArray(rec.bids) ? rec.bids : []).map((b) => {
      const br = asRecord(b);
      return {
        good: typeof br.good === "string" ? br.good : "metal_ore",
        price_major: typeof br.price_major === "number" ? br.price_major : 50,
        intake: typeof br.intake === "number" ? br.intake : 100,
        boards: Array.isArray(br.boards)
          ? br.boards.filter((x): x is string => typeof x === "string")
          : ["exchange"],
      };
    });
    const mill = (Array.isArray(rec.mill) ? rec.mill : []).map((m) => {
      const mr = asRecord(m);
      const ratio = asRecord(mr.ratio);
      return {
        ore: typeof mr.ore === "string" ? mr.ore : "metal_ore",
        into: typeof mr.into === "string" ? mr.into : "metals",
        ratio_ore: typeof ratio.ore === "number" ? ratio.ore : 5,
        ratio_product: typeof ratio.product === "number" ? ratio.product : 2,
        batch_t: typeof mr.batch_t === "number" ? mr.batch_t : 20,
        per_ic_day_t:
          typeof mr.per_ic_day_t === "number" ? mr.per_ic_day_t : 40,
      };
    });
    return {
      org:
        typeof rec.org === "string"
          ? rec.org
          : typeof rec.id === "string"
            ? rec.id
            : "",
      storefront: typeof rec.storefront === "string" ? rec.storefront : "",
      agent: typeof rec.agent === "string" ? rec.agent : "",
      signatory: typeof rec.signatory === "string" ? rec.signatory : "",
      stock,
      asks,
      bids,
      mill,
    };
  });
}

/** Update a vendor in `def.vendors[]` while preserving nested fields like `refresh` or `as`. */
export function updatePortVendor(
  def: Record<string, unknown>,
  index: number,
  draft: PortVendorDraft,
): Record<string, unknown> {
  const next = JSON.parse(JSON.stringify(def)) as Record<string, unknown>;
  const vendors = Array.isArray(next.vendors) ? [...next.vendors] : [];
  const existing = asRecord(vendors[index]);
  const existingBids = Array.isArray(existing.bids) ? existing.bids : [];

  const updated: Record<string, unknown> = {
    ...existing,
    org: draft.org,
    storefront: draft.storefront,
    agent: draft.agent,
    signatory: draft.signatory,
  };

  if (draft.stock.length > 0 || existing.stock !== undefined) {
    updated.stock = draft.stock.map((s) => ({
      good: s.good,
      qty: s.qty,
      grade: s.grade,
      issue: s.issue,
    }));
  }
  if (draft.asks.length > 0 || existing.asks !== undefined) {
    updated.asks = draft.asks.map((a) => ({
      good: a.good,
      price_major: a.price_major,
      boards: a.boards,
    }));
  }
  if (draft.bids.length > 0 || existing.bids !== undefined) {
    updated.bids = draft.bids.map((b, i) => {
      const prevBid = asRecord(existingBids[i]);
      return {
        ...prevBid,
        good: b.good,
        price_major: b.price_major,
        intake: b.intake,
        boards: b.boards,
      };
    });
  }
  if (draft.mill.length > 0 || existing.mill !== undefined) {
    updated.mill = draft.mill.map((m) => ({
      ore: m.ore,
      into: m.into,
      ratio: { ore: m.ratio_ore, product: m.ratio_product },
      batch_t: m.batch_t,
      per_ic_day_t: m.per_ic_day_t,
    }));
  }

  vendors[index] = updated;
  next.vendors = vendors;
  return next;
}

/** Add a new vendor desk to `def.vendors[]`. */
export function addPortVendor(
  def: Record<string, unknown>,
  portId: string,
): Record<string, unknown> {
  const next = JSON.parse(JSON.stringify(def)) as Record<string, unknown>;
  const vendors = Array.isArray(next.vendors) ? [...next.vendors] : [];
  const area = typeof next.area === "string" ? next.area : "neighborhood.ceres_freeport";
  vendors.push({
    org: "new_vendor",
    storefront: `${area}.item.exchange_manifold`,
    agent: `${portId}.clerk`,
    signatory: "",
    stock: [],
    asks: [],
    bids: [],
  });
  next.vendors = vendors;
  return next;
}

/** Remove a vendor desk from `def.vendors[]`. */
export function removePortVendor(
  def: Record<string, unknown>,
  index: number,
): Record<string, unknown> {
  const next = JSON.parse(JSON.stringify(def)) as Record<string, unknown>;
  const vendors = Array.isArray(next.vendors) ? [...next.vendors] : [];
  vendors.splice(index, 1);
  next.vendors = vendors;
  return next;
}

const CLAIM_BLOCK_IDS = [
  "A",
  "B",
  "C",
  "D",
  "E",
  "F",
  "G",
  "H",
  "I",
  "J",
  "K",
  "L",
  "M",
  "N",
  "O",
  "P",
] as const;

/** Extract mining site & claim blocks (`A`–`P` + `yard`) from a port `def`. */
export function extractPortSite(
  def: Record<string, unknown>,
): PortSiteDraft | null {
  if (!def.site || typeof def.site !== "object") return null;
  const site = asRecord(def.site);
  const tenure = asRecord(site.tenure);
  const rawBlocks = asRecord(site.blocks);
  const blocks: SiteBlockDraft[] = CLAIM_BLOCK_IDS.map((id) => {
    const b = asRecord(rawBlocks[id]);
    return {
      id,
      tenure:
        typeof b.tenure === "string"
          ? b.tenure
          : typeof tenure.default === "string"
            ? tenure.default
            : "open",
      holder: typeof b.holder === "string" ? b.holder : "",
      prospectivity:
        typeof b.prospectivity === "number" ? b.prospectivity : 0,
      worked_permille:
        typeof b.worked_permille === "number" ? b.worked_permille : 0,
    };
  });
  const yard = asRecord(site.yard);
  return {
    name: typeof site.name === "string" ? site.name : "",
    issuer:
      typeof site.issuer === "string" ? site.issuer : "freeport_registry",
    zone: typeof site.zone === "string" ? site.zone : "core",
    defaultTenure:
      typeof tenure.default === "string" ? tenure.default : "open",
    blocks,
    hasYard: Boolean(site.yard && typeof site.yard === "object"),
    yardOrg: typeof yard.org === "string" ? yard.org : "",
    yardName: typeof yard.name === "string" ? yard.name : "",
    yardUnit:
      typeof yard.unit === "string" ? yard.unit : "tacsim.unit.dray_p",
    yardRigs: typeof yard.rigs === "number" ? yard.rigs : 2,
    yardOre: typeof yard.ore === "string" ? yard.ore : "ice_ore",
    yardRearmSecs:
      typeof yard.rearm_ic_secs === "number" ? yard.rearm_ic_secs : 3600,
  };
}

/** Update a mining claim block (`A`–`P`) inside `def.site.blocks`. */
export function updatePortSiteBlock(
  def: Record<string, unknown>,
  blockId: string,
  patch: Partial<SiteBlockDraft>,
): Record<string, unknown> {
  const next = JSON.parse(JSON.stringify(def)) as Record<string, unknown>;
  const site = asRecord(next.site);
  const blocks = asRecord(site.blocks);
  const existing = asRecord(blocks[blockId]);

  const updated: Record<string, unknown> = { ...existing };
  if (patch.tenure !== undefined) {
    updated.tenure = patch.tenure;
  }
  if (patch.holder !== undefined) {
    if (patch.holder.trim()) {
      updated.holder = patch.holder.trim();
    } else {
      delete updated.holder;
    }
  }
  if (patch.prospectivity !== undefined) {
    if (patch.prospectivity > 0) {
      updated.prospectivity = patch.prospectivity;
    } else {
      delete updated.prospectivity;
    }
  }
  if (patch.worked_permille !== undefined) {
    if (patch.worked_permille > 0) {
      updated.worked_permille = patch.worked_permille;
    } else {
      delete updated.worked_permille;
    }
  }
  blocks[blockId] = updated;
  site.blocks = blocks;
  next.site = site;
  return next;
}

/** Update mining site top-level metadata (`name`, `issuer`, `zone`, `defaultTenure`, `yard`). */
export function updatePortSiteMeta(
  def: Record<string, unknown>,
  patch: Partial<Omit<PortSiteDraft, "blocks">>,
): Record<string, unknown> {
  const next = JSON.parse(JSON.stringify(def)) as Record<string, unknown>;
  const site = asRecord(next.site);
  if (patch.name !== undefined) site.name = patch.name;
  if (patch.issuer !== undefined) site.issuer = patch.issuer;
  if (patch.zone !== undefined) site.zone = patch.zone;
  if (patch.defaultTenure !== undefined) {
    const tenure = asRecord(site.tenure);
    tenure.default = patch.defaultTenure;
    site.tenure = tenure;
  }
  if (patch.hasYard !== undefined || site.yard) {
    if (patch.hasYard === false) {
      delete site.yard;
    } else {
      const yard = asRecord(site.yard);
      if (patch.yardOrg !== undefined) yard.org = patch.yardOrg;
      if (patch.yardName !== undefined) yard.name = patch.yardName;
      if (patch.yardUnit !== undefined) yard.unit = patch.yardUnit;
      if (patch.yardRigs !== undefined) yard.rigs = patch.yardRigs;
      if (patch.yardOre !== undefined) yard.ore = patch.yardOre;
      if (patch.yardRearmSecs !== undefined) {
        yard.rearm_ic_secs = patch.yardRearmSecs;
      }
      site.yard = yard;
    }
  }
  next.site = site;
  return next;
}

/** Extract relay network nodes from `relays/luna_ceres.json` (`network.def`). */
export function extractRelayNodes(
  def: Record<string, unknown>,
): RelayNodeDraft[] {
  const nodes = asRecord(def.nodes);
  return Object.entries(nodes).map(([id, raw]) => {
    const n = asRecord(raw);
    const place = asRecord(n.place);
    const coverage = Array.isArray(n.coverage) ? n.coverage.length : 0;
    return {
      id,
      label: typeof n.label === "string" ? n.label : id,
      placePack: typeof place.pack === "string" ? place.pack : "",
      placeEntry: typeof place.entry === "string" ? place.entry : "",
      coverageCount: coverage,
    };
  });
}

/** Update a relay node's `label` and `place` inside `def.nodes`. */
export function updateRelayNode(
  def: Record<string, unknown>,
  nodeId: string,
  patch: Partial<Pick<RelayNodeDraft, "label" | "placePack" | "placeEntry">>,
): Record<string, unknown> {
  const next = JSON.parse(JSON.stringify(def)) as Record<string, unknown>;
  const nodes = asRecord(next.nodes);
  const node = asRecord(nodes[nodeId]);
  if (patch.label !== undefined) node.label = patch.label;
  if (patch.placePack !== undefined || patch.placeEntry !== undefined) {
    const place = asRecord(node.place);
    if (patch.placePack !== undefined) place.pack = patch.placePack;
    if (patch.placeEntry !== undefined) place.entry = patch.placeEntry;
    node.place = place;
  }
  nodes[nodeId] = node;
  next.nodes = nodes;
  return next;
}

/** Extract relay entangled pairs from `relays/luna_ceres.json` (`network.def`). */
export function extractRelayPairs(
  def: Record<string, unknown>,
): RelayPairDraft[] {
  const pairs = asRecord(def.pairs);
  return Object.entries(pairs).map(([id, raw]) => {
    const p = asRecord(raw);
    const halves = Array.isArray(p.halves) ? p.halves : [];
    const h0 = asRecord(halves[0]);
    const h1 = asRecord(halves[1]);
    return {
      id,
      capacityUnitsPerIcSecond:
        typeof p.capacity_units_per_ic_second === "number"
          ? p.capacity_units_per_ic_second
          : 8,
      classPolicy:
        typeof p.class_policy === "string" ? p.class_policy : "exchange",
      nodeA: typeof h0.node === "string" ? h0.node : "",
      nodeB: typeof h1.node === "string" ? h1.node : "",
    };
  });
}

/** Update a relay entangled pair inside `def.pairs`. */
export function updateRelayPair(
  def: Record<string, unknown>,
  pairId: string,
  patch: Partial<Omit<RelayPairDraft, "id">>,
): Record<string, unknown> {
  const next = JSON.parse(JSON.stringify(def)) as Record<string, unknown>;
  const pairs = asRecord(next.pairs);
  const pair = asRecord(pairs[pairId]);
  if (patch.capacityUnitsPerIcSecond !== undefined) {
    pair.capacity_units_per_ic_second = patch.capacityUnitsPerIcSecond;
  }
  if (patch.classPolicy !== undefined) {
    pair.class_policy = patch.classPolicy;
  }
  const halves = Array.isArray(pair.halves) ? [...pair.halves] : [{}, {}];
  const h0 = asRecord(halves[0]);
  const h1 = asRecord(halves[1]);
  const nextNodeA = patch.nodeA ?? (typeof h0.node === "string" ? h0.node : "");
  const nextNodeB = patch.nodeB ?? (typeof h1.node === "string" ? h1.node : "");
  halves[0] = {
    ...h0,
    id: typeof h0.id === "string" && h0.id ? h0.id : `${pairId}.${nextNodeA}`,
    node: nextNodeA,
  };
  halves[1] = {
    ...h1,
    id: typeof h1.id === "string" && h1.id ? h1.id : `${pairId}.${nextNodeB}`,
    node: nextNodeB,
  };
  pair.halves = halves;
  pairs[pairId] = pair;
  next.pairs = pairs;
  return next;
}

/** Extract manufacture recipe (`private_pair`) from `relays/manufacture-v1.json`. */
export function extractRelayRecipe(
  def: Record<string, unknown>,
): RelayRecipeDraft | null {
  if (!Array.isArray(def.inputs) && !Array.isArray(def.outputs)) return null;
  const inputs = (Array.isArray(def.inputs) ? def.inputs : []).map((i) => {
    const r = asRecord(i);
    return {
      commodity: typeof r.commodity === "string" ? r.commodity : "",
      units: typeof r.units === "number" ? r.units : 1,
    };
  });
  const outputs = (Array.isArray(def.outputs) ? def.outputs : []).map((o) => {
    const r = asRecord(o);
    return {
      commodity: typeof r.commodity === "string" ? r.commodity : "",
      units: typeof r.units === "number" ? r.units : 1,
    };
  });
  const requires = Array.isArray(def.requires)
    ? def.requires.filter((x): x is string => typeof x === "string")
    : [];
  return {
    label: typeof def.label === "string" ? def.label : "",
    workUnits: typeof def.work_units === "number" ? def.work_units : 120,
    requires,
    inputs,
    outputs,
  };
}

/** Update manufacture recipe (`private_pair`) inside `relays/manufacture-v1.json`. */
export function updateRelayRecipe(
  def: Record<string, unknown>,
  patch: Partial<RelayRecipeDraft>,
): Record<string, unknown> {
  const next = JSON.parse(JSON.stringify(def)) as Record<string, unknown>;
  if (patch.label !== undefined) next.label = patch.label;
  if (patch.workUnits !== undefined) next.work_units = patch.workUnits;
  if (patch.requires !== undefined) next.requires = patch.requires;
  if (patch.inputs !== undefined) next.inputs = patch.inputs;
  if (patch.outputs !== undefined) next.outputs = patch.outputs;
  return next;
}
