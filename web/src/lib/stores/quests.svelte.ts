// Live WoW-style Quest Store (`tacsim.quests` panel feed + `RoomView.ambient.quest_markers`).
// Tracks NPC quest markers (`!`, gold `?`, silver `?`), active quests & objective progress,
// available room offers with server-signed `+quest/accept` tokens, completed quest history,
// collapsible HUD tracker state, and NPC quest dialog state.

import type { RoomView } from "../proto";

export const QUESTS_FEED = "tacsim.quests";

export type QuestMarkerKind = "turn_in" | "available" | "in_progress";

export interface NpcQuestMarker {
  npcId: string;
  npcName: string;
  marker: QuestMarkerKind;
  symbol: "!" | "?";
  questKey: string;
  questTitle: string;
  category: string;
  questCount: number;
}

export interface QuestCheckpointView {
  id: string;
  title: string;
  instructions: string;
  roomEntry: string;
  roomName: string;
  checked: boolean;
  liveReady: boolean;
  detailText: string | null;
}

export interface ActiveQuestView {
  key: string;
  title: string;
  category: string;
  introduction: string;
  rewardSummary: string;
  giverId: string;
  giverName: string;
  turnInId: string;
  turnInName: string;
  turnInHere: boolean;
  allowRestart: boolean;
  readyToFinish: boolean;
  nextStep: string;
  nextInstructions: string;
  nextRoomEntry: string;
  nextRoomName: string;
  nextObjectiveReady: boolean;
  canCheckNow: boolean;
  canFinishNow: boolean;
  checkCmd: string;
  finishCmd: string;
  abandonCmd: string;
  checkpoints: QuestCheckpointView[];
}

export interface AvailableQuestView {
  key: string;
  title: string;
  category: string;
  introduction: string;
  rewardSummary: string;
  giverId: string;
  giverName: string;
  turnInId: string;
  turnInName: string;
  allowRestart: boolean;
  acceptCmd: string;
  declineCmd: string;
  checkpoints: QuestCheckpointView[];
}

export interface CompletedQuestView {
  key: string;
  title: string;
  category: string;
  giverName: string;
  turnInName: string;
  completion: string;
  rewardSummary: string;
  completedAtUtc: number;
}

export interface QuestTradeNeed {
  questTitle: string;
  checkpointTitle: string;
  commoditySlug: string;
  targetQuantity: number;
  wantsBuy: boolean;
  wantsSell: boolean;
}

export interface QuestStowageReadiness {
  commoditySlug: string;
  label: string;
  currentQty: number;
  targetQty: number;
  unit: string;
  ready: boolean;
  gaugeKind: "propellant" | "provisions" | "cargo";
}

export interface QuestRefitHoldReadiness {
  cargoEmpty: boolean;
  provisionsEmpty: boolean;
  cargoKg: number;
  provisionsCrewDays: number;
  propellantTonnes: number;
  ready: boolean;
}

export function evaluateRefitHoldReadiness(
  stowage:
    | {
        cargoKg: number;
        propellantKg: number;
        provisionsCrewDays: number;
        cargoLines?: ReadonlyArray<{ qty: number }>;
      }
    | null
    | undefined,
): QuestRefitHoldReadiness | null {
  if (!stowage) return null;
  const cargoKg = Math.max(0, stowage.cargoKg);
  const hasCargoLines = (stowage.cargoLines ?? []).some((l) => l.qty > 0);
  const cargoEmpty = cargoKg === 0 && !hasCargoLines;
  const provisionsCrewDays = Math.max(0, Math.floor(stowage.provisionsCrewDays));
  const provisionsEmpty = provisionsCrewDays === 0;
  const propellantTonnes = Math.floor(Math.max(0, stowage.propellantKg) / 1000);
  return {
    cargoEmpty,
    provisionsEmpty,
    cargoKg,
    provisionsCrewDays,
    propellantTonnes,
    ready: cargoEmpty && provisionsEmpty,
  };
}

export function extractSurfaceMiningPrepActions(
  mining: QuestSurfaceMiningStatus | null,
  rigSlot = 1,
): QuestSuggestedAction[] {
  if (!mining) return [];
  const safeSlot = Number.isFinite(rigSlot) && rigSlot >= 1 ? Math.floor(rigSlot) : 1;
  const recoverCmd = `+tv/${safeSlot} recover go`;
  if (!mining.scanDone) {
    return [
      { label: "License", command: "+license" },
      { label: "Lease Rig", command: "+tv/lease" },
      { label: "Ground Chart", command: "+chart/ground" },
    ];
  }
  if (!mining.mineDone) {
    return [
      { label: "Ground Chart", command: "+chart/ground" },
      { label: "Recover Rig", command: recoverCmd },
    ];
  }
  return [{ label: "Recover Rig", command: recoverCmd }];
}

export function evaluateQuestStowageReadiness(
  need: QuestTradeNeed | null,
  stowage:
    | {
        propellantKg: number;
        provisionsCrewDays: number;
        cargoLines: ReadonlyArray<{ good: string; label: string; qty: number }>;
      }
    | null
    | undefined,
): QuestStowageReadiness | null {
  if (!need || !stowage) return null;
  const targetQty = Math.max(1, need.targetQuantity);
  const label = need.commoditySlug.replaceAll("_", " ").toUpperCase();
  if (need.commoditySlug === "propellant") {
    const currentQty = Math.floor(Math.max(0, stowage.propellantKg) / 1000);
    return {
      commoditySlug: need.commoditySlug,
      label,
      currentQty,
      targetQty,
      unit: "t",
      ready: currentQty >= targetQty,
      gaugeKind: "propellant",
    };
  }
  if (need.commoditySlug === "provisions") {
    const currentQty = Math.floor(Math.max(0, stowage.provisionsCrewDays));
    return {
      commoditySlug: need.commoditySlug,
      label,
      currentQty,
      targetQty,
      unit: "d",
      ready: currentQty >= targetQty,
      gaugeKind: "provisions",
    };
  }
  let currentQty = 0;
  for (const line of stowage.cargoLines) {
    if (
      normalizeGoodSlug(line.good) === need.commoditySlug ||
      normalizeGoodSlug(line.label) === need.commoditySlug
    ) {
      currentQty += Math.max(0, line.qty);
    }
  }
  return {
    commoditySlug: need.commoditySlug,
    label,
    currentQty,
    targetQty,
    unit: "u",
    ready: currentQty >= targetQty,
    gaugeKind: "cargo",
  };
}

export interface QuestHoldReadinessSummary {
  kind: "refit" | "trade";
  tag: string;
  text: string;
  ready: boolean;
  refit?: QuestRefitHoldReadiness;
  primary?: QuestStowageReadiness;
  secondary?: QuestStowageReadiness[];
}

export function formatQuestHoldReadinessSummary(
  quest: ActiveQuestView | null,
  stowage:
    | {
        cargoKg: number;
        propellantKg: number;
        provisionsCrewDays: number;
        cargoLines: ReadonlyArray<{ good: string; label: string; qty: number }>;
      }
    | null
    | undefined,
): QuestHoldReadinessSummary | null {
  if (!quest || !stowage) return null;
  const suggested = extractQuestSuggestedAction(quest);
  if (suggested?.command.startsWith("+coop/refit")) {
    const refit = evaluateRefitHoldReadiness(stowage);
    if (!refit) return null;
    const cargoText = refit.cargoEmpty
      ? "✓ Cargo empty"
      : `✗ Cargo ${(refit.cargoKg / 1000).toFixed(1)}t`;
    const storesText = refit.provisionsEmpty
      ? "✓ Stores empty"
      : `✗ Stores ${refit.provisionsCrewDays}d`;
    return {
      kind: "refit",
      tag: refit.ready ? "[READY]" : "[REFIT CHECK]",
      text: `${cargoText} · ${storesText} · Tank ${refit.propellantTonnes}t OK`,
      ready: refit.ready,
      refit,
    };
  }

  const needs = extractActiveTradeNeeds([quest], quest.key);
  if (needs.length === 0) {
    const mining = extractActiveSurfaceMiningStatus([quest], quest.key);
    if (!mining) return null;
    const want = normalizeGoodSlug(mining.targetOreSlug);
    let currentQty = 0;
    for (const line of stowage.cargoLines) {
      if (normalizeGoodSlug(line.good) === want || normalizeGoodSlug(line.label) === want) {
        currentQty += Math.max(0, Math.floor(line.qty));
      }
    }
    const targetQty = 1;
    const ready = currentQty >= targetQty;
    const label = mining.targetOreSlug.replaceAll("_", " ").toUpperCase();
    const primary: QuestStowageReadiness = {
      commoditySlug: mining.targetOreSlug,
      label,
      currentQty,
      targetQty,
      unit: "t",
      ready,
      gaugeKind: "cargo",
    };
    return {
      kind: "trade",
      tag: ready ? "[ORE HOLD READY]" : "[ORE HOLD]",
      text: `${ready ? "✓" : "○"} ${label}: ${currentQty}/${targetQty} t`,
      ready,
      primary,
      secondary: [],
    };
  }
  const items: QuestStowageReadiness[] = [];
  for (const need of needs) {
    const item = evaluateQuestStowageReadiness(need, stowage);
    if (item) items.push(item);
  }
  if (items.length === 0) return null;

  const primary = items.find((r) => !r.ready) ?? items[0]!;
  const secondary = items.filter((r) => r !== primary);
  const ready = items.every((r) => r.ready);
  const primaryText = `${primary.ready ? "✓" : "○"} ${primary.label}: ${primary.currentQty}/${primary.targetQty} ${primary.unit}`;
  const secondaryText = secondary
    .map((s) => `${s.label} ${s.currentQty}/${s.targetQty}${s.unit}${s.ready ? " OK" : ""}`)
    .join(" · ");

  return {
    kind: "trade",
    tag: ready ? "[HOLD READY]" : "[HOLD]",
    text: secondaryText ? `${primaryText} | ${secondaryText}` : primaryText,
    ready,
    primary,
    secondary,
  };
}

export interface QuestSailingPreflightReadiness {
  propellantTonnes: number;
  propellantOk: boolean;
  provisionsDays: number;
  provisionsOk: boolean;
  cargoRequired: boolean;
  cargoLabel: string;
  cargoQty: number;
  cargoTarget: number;
  cargoOk: boolean;
  ready: boolean;
}

export function evaluateSailingPreflightReadiness(
  dest: QuestDestinationHint | null,
  need: QuestTradeNeed | null,
  stowage:
    | {
        propellantKg: number;
        provisionsCrewDays: number;
        cargoLines: ReadonlyArray<{ good: string; label: string; qty: number }>;
      }
    | null
    | undefined,
): QuestSailingPreflightReadiness | null {
  if (!dest || !stowage) return null;
  const propellantTonnes = Math.floor(Math.max(0, stowage.propellantKg) / 1000);
  const propellantOk = stowage.propellantKg > 0;
  const provisionsDays = Math.floor(Math.max(0, stowage.provisionsCrewDays));
  const provisionsOk = stowage.provisionsCrewDays > 0;

  let cargoRequired = false;
  let cargoLabel = "";
  let cargoQty = 0;
  let cargoTarget = 0;
  let cargoOk = true;
  if (need && need.commoditySlug !== "propellant" && need.commoditySlug !== "provisions") {
    cargoRequired = true;
    cargoLabel = need.commoditySlug.replaceAll("_", " ");
    cargoTarget = Math.max(1, need.targetQuantity);
    for (const line of stowage.cargoLines) {
      if (
        normalizeGoodSlug(line.good) === need.commoditySlug ||
        normalizeGoodSlug(line.label) === need.commoditySlug
      ) {
        cargoQty += Math.max(0, line.qty);
      }
    }
    cargoOk = cargoQty >= cargoTarget;
  }

  return {
    propellantTonnes,
    propellantOk,
    provisionsDays,
    provisionsOk,
    cargoRequired,
    cargoLabel,
    cargoQty,
    cargoTarget,
    cargoOk,
    ready: propellantOk && provisionsOk && cargoOk,
  };
}

/**
 * True when the tracked quest is a cross-port buy+deliver trade run (e.g. Step 3 metals
 * or Step 4 industrial_parts) whose required cargo is not yet loaded in the ship's hold,
 * so station wayfinding should direct the player to the local Exchange before Berth Gallery.
 */
export function needsOriginPortProcurement(
  active: readonly ActiveQuestView[],
  trackedKey: string | null,
  stowage:
    | {
        propellantKg: number;
        provisionsCrewDays: number;
        cargoLines: ReadonlyArray<{ good: string; label: string; qty: number }>;
      }
    | null
    | undefined,
): boolean {
  if (!stowage) return false;
  const tracked = orderedActiveQuests(active, trackedKey)[0] ?? null;
  if (!tracked || tracked.readyToFinish || tracked.canFinishNow) return false;
  const need = extractActiveTradeNeed([tracked], tracked.key);
  if (!need || !need.wantsBuy || !need.wantsSell) return false;
  if (need.commoditySlug === "propellant" || need.commoditySlug === "provisions") return false;
  const readiness = evaluateQuestStowageReadiness(need, stowage);
  return readiness !== null && !readiness.ready;
}

export interface QuestDestinationHint {
  questTitle: string;
  portId: string;
  portLabel: string;
}

export interface QuestSurfaceMiningStatus {
  questTitle: string;
  targetOreSlug: string;
  recommendedFamily: string;
  hasScanStep: boolean;
  scanDone: boolean;
  hasSoundStep: boolean;
  soundDone: boolean;
  hasMineStep: boolean;
  mineDone: boolean;
  nextStepText: string;
}

export interface QuestMilestone {
  kind: "accepted" | "checkpoint" | "ready" | "completed" | "abandoned";
  key: string;
  title: string;
  detail?: string;
  tourAllDone?: boolean;
  restartable?: boolean;
}

export interface GuidedTourProgress {
  completed: number;
  total: number;
  currentStep: number | null;
  currentTitle: string;
  allDone: boolean;
}

export type GuidedTourStepStatus = "completed" | "active" | "available" | "upcoming";

export interface GuidedTourRoadmapStep {
  step: number;
  key: string;
  shortLabel: string;
  giverLabel: string;
  giverRoomEntry: string;
  giverRoomName: string;
  locationLabel: string;
  status: GuidedTourStepStatus;
  readyToTurnIn: boolean;
}

export interface QuestSuggestedAction {
  label: string;
  command: string;
}

export const GUIDED_TOUR_ROADMAP_META: ReadonlyArray<{
  readonly key: string;
  readonly shortLabel: string;
  readonly giverLabel: string;
  readonly giverRoomEntry: string;
  readonly giverRoomName: string;
  readonly locationLabel: string;
}> = [
  {
    key: "tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll",
    shortLabel: "Co-op Charter",
    giverLabel: "Maren Halvorsen",
    giverRoomEntry: "room.c03_arrivals",
    giverRoomName: "Port Watch Arrivals",
    locationLabel: "Ceres Arrivals -> Registry",
  },
  {
    key: "tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel",
    shortLabel: "Locker & Tanks",
    giverLabel: "Ilse Oduya-Brandt",
    giverRoomEntry: "room.c05_registry",
    giverRoomName: "Registry & Co-op Desk",
    locationLabel: "Ceres Chandlery & Exchange",
  },
  {
    key: "tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery",
    shortLabel: "Metals Haul",
    giverLabel: "Ilse Oduya-Brandt",
    giverRoomEntry: "room.c05_registry",
    giverRoomName: "Registry & Co-op Desk",
    locationLabel: "Ceres -> Stickney Receiving",
  },
  {
    key: "tacsim.quests.guided_onboarding/quest.guide_04_return_haul",
    shortLabel: "Return Load",
    giverLabel: "Jonas Achterberg-Nkemelu",
    giverRoomEntry: "room.p07_receiving",
    giverRoomName: "Receiving Counter",
    locationLabel: "Stickney -> Ceres Chandlery",
  },
  {
    key: "tacsim.quests.guided_onboarding/quest.guide_05_refit_prospector",
    shortLabel: "Dray-P Refit",
    giverLabel: "Ilse Oduya-Brandt",
    giverRoomEntry: "room.c05_registry",
    giverRoomName: "Registry & Co-op Desk",
    locationLabel: "Ceres Registry -> Exchange",
  },
  {
    key: "tacsim.quests.guided_onboarding/quest.guide_06_surface_mining",
    shortLabel: "Surface Mining",
    giverLabel: "Rosalind Qureshi-Mbeki",
    giverRoomEntry: "room.c07_exchange",
    giverRoomName: "Exchange Hall",
    locationLabel: "Ceres Surface Landing Site",
  },
  {
    key: "tacsim.quests.guided_onboarding/quest.guide_07_sell_mined_ore",
    shortLabel: "Sell Mined Ore",
    giverLabel: "Rosalind Qureshi-Mbeki",
    giverRoomEntry: "room.c07_exchange",
    giverRoomName: "Exchange Hall",
    locationLabel: "Ceres -> Stickney Service",
  },
];

export const GUIDED_TOUR_KEYS: readonly string[] = GUIDED_TOUR_ROADMAP_META.map((m) => m.key);

export function buildGuidedTourRoadmap(
  active: readonly ActiveQuestView[],
  available: readonly AvailableQuestView[],
  completed: readonly CompletedQuestView[],
): GuidedTourRoadmapStep[] {
  const doneSet = new Set(completed.map((c) => c.key));
  const activeByKey = new Map(active.map((a) => [a.key, a]));
  const availSet = new Set(available.map((av) => av.key));

  return GUIDED_TOUR_ROADMAP_META.map((meta, idx) => {
    const activeMatch = activeByKey.get(meta.key);
    let status: GuidedTourStepStatus;
    if (doneSet.has(meta.key)) {
      status = "completed";
    } else if (activeMatch) {
      status = "active";
    } else if (availSet.has(meta.key)) {
      status = "available";
    } else {
      status = "upcoming";
    }
    return {
      step: idx + 1,
      key: meta.key,
      shortLabel: meta.shortLabel,
      giverLabel: meta.giverLabel,
      giverRoomEntry: meta.giverRoomEntry,
      giverRoomName: meta.giverRoomName,
      locationLabel: meta.locationLabel,
      status,
      readyToTurnIn: Boolean(activeMatch && (activeMatch.canFinishNow || activeMatch.readyToFinish)),
    };
  });
}

/**
 * Between-steps guided tour locator: when the player has completed at least one guided step
 * (and has not yet completed all 7) but currently has no active quest and no offer in the
 * current room, returns the next upcoming guided step so tracker, Quest Log, and station/ship
 * wayfinding guide them straight to the next NPC giver.
 */
export function findNextGuidedOffer(
  active: readonly ActiveQuestView[],
  available: readonly AvailableQuestView[],
  completed: readonly CompletedQuestView[],
): GuidedTourRoadmapStep | null {
  if (active.length > 0 || available.length > 0) return null;
  const roadmap = buildGuidedTourRoadmap(active, available, completed);
  const completedCount = roadmap.filter((s) => s.status === "completed").length;
  if (completedCount === 0 || completedCount >= roadmap.length) return null;
  return roadmap.find((s) => s.status === "upcoming") ?? null;
}

/**
 * Resolves a corridor destination hint for a between-steps Next Guided Offer when the pilot
 * is currently at a different port/body than the next guided NPC giver (returns null if already there).
 */
export function resolveNextGuidedOfferDestination(
  nextOffer: GuidedTourRoadmapStep | null,
  currentPortOrBody?: string | null,
): QuestDestinationHint | null {
  if (!nextOffer) return null;
  const targetPort = inferStationPortFromRoomEntry(nextOffer.giverRoomEntry);
  if (!targetPort) return null;
  const found = KNOWN_DESTINATIONS.find(([id]) => id === targetPort);
  if (!found) return null;
  const cur = (currentPortOrBody ?? "").trim().toLowerCase();
  if (
    cur &&
    (cur === found[0] ||
      cur.includes(found[0]) ||
      found[1].toLowerCase().includes(cur))
  ) {
    return null;
  }
  return {
    questTitle: `Next Guided (${nextOffer.step}/7): ${nextOffer.shortLabel} — ${nextOffer.giverLabel}`,
    portId: found[0],
    portLabel: found[1],
  };
}

function guidedStepNumber(key: string): number | null {
  const idx = GUIDED_TOUR_KEYS.indexOf(key);
  return idx >= 0 ? idx + 1 : null;
}

export function computeGuidedTourProgress(
  active: readonly ActiveQuestView[],
  available: readonly AvailableQuestView[],
  completed: readonly CompletedQuestView[],
): GuidedTourProgress | null {
  const doneSet = new Set(completed.map((c) => c.key).filter((k) => GUIDED_TOUR_KEYS.includes(k)));
  const activeGuided = active.find((q) => guidedStepNumber(q.key) !== null) ?? null;
  const availGuided = available.find((q) => guidedStepNumber(q.key) !== null) ?? null;
  if (doneSet.size === 0 && !activeGuided && !availGuided) {
    return null;
  }
  const current = activeGuided ?? availGuided;
  const currentStep = current ? guidedStepNumber(current.key) : null;
  return {
    completed: doneSet.size,
    total: GUIDED_TOUR_KEYS.length,
    currentStep,
    currentTitle: current?.title ?? "",
    allDone: doneSet.size >= GUIDED_TOUR_KEYS.length,
  };
}

export function extractQuestSuggestedAction(
  quest: ActiveQuestView | null,
): QuestSuggestedAction | null {
  if (!quest || quest.canFinishNow || quest.canCheckNow || quest.readyToFinish) {
    return null;
  }
  const pending = quest.checkpoints.find((cp) => !cp.checked && !cp.liveReady) ?? null;
  const idLow = (pending?.id ?? "").toLowerCase();
  const textLow = `${idLow} ${pending?.title ?? ""} ${pending?.instructions ?? ""} ${quest.nextStep}`.toLowerCase();

  if (textLow.includes("+coop/refit") || idLow.includes("refit_dray")) {
    return { label: "Refit Dray-P", command: "+coop/refit prospector" };
  }
  if (textLow.includes("+coop/enroll") || idLow.includes("enroll_coop")) {
    return { label: "Enroll Co-op", command: "+coop/enroll " };
  }
  if (idLow.includes("scan") || textLow.includes("scan go")) {
    return { label: "Scan Site", command: "+tv/1 scan go" };
  }
  if (idLow.includes("sound") || textLow.includes("sound go")) {
    return { label: "Sound Pocket", command: "+tv/1 sound go" };
  }
  if (idLow.startsWith("mine_") || textLow.includes("mine go")) {
    return { label: "Mine Ore", command: "+tv/1 mine go" };
  }
  const dest = extractActiveDestinationPort([quest], quest.key);
  if (dest) {
    return { label: `Plan ${dest.portLabel}`, command: `+ship/plan ${dest.portId}` };
  }
  if (textLow.includes("+market/")) {
    return { label: "Market Board", command: "+market/list" };
  }
  return null;
}

const KNOWN_TRADE_COMMODITIES: ReadonlyArray<readonly [string, readonly string[]]> = [
  ["industrial_parts", ["industrial_parts", "industrial parts"]],
  ["carbon_ore", ["carbon_ore", "carbon ore"]],
  ["structural_alloys", ["structural_alloys", "structural alloys"]],
  ["rare_earths", ["rare_earths", "rare earths"]],
  ["water_ice", ["water_ice", "water ice"]],
  ["provisions", ["provisions"]],
  ["propellant", ["propellant"]],
  ["volatiles", ["volatiles"]],
  ["silicates", ["silicates"]],
  ["metals", ["metals"]],
];

const KNOWN_DESTINATIONS: ReadonlyArray<readonly [string, string, readonly string[]]> = [
  ["stickney", "Stickney Yards", ["+ship/plan stickney", "+ship/order depart stickney", "stickney yards", "stickney"]],
  ["ceres", "Ceres Freeport", ["+ship/plan ceres", "+ship/order depart ceres", "ceres freeport", "back to ceres", "return to ceres"]],
  ["luna", "Aitken Highport", ["+ship/plan luna", "+ship/order depart luna", "aitken highport"]],
  ["deimos", "Voltaire Anchorage", ["+ship/plan deimos", "+ship/order depart deimos", "voltaire anchorage"]],
  ["titan", "Ligeia Terminal", ["+ship/plan titan", "+ship/order depart titan", "ligeia terminal"]],
  ["isidis", "Isidis Concession", ["+ship/plan isidis", "+ship/order depart isidis", "isidis concession"]],
  ["commons", "Freeport Commons", ["+ship/plan commons", "freeport commons"]],
  ["ahuna", "Ahuna Flank", ["+ship/plan ahuna", "ahuna flank"]],
  ["occator", "Occator Rim", ["+ship/plan occator", "occator rim"]],
  ["kerwan", "Kerwan Floor", ["+ship/plan kerwan", "kerwan floor"]],
  ["juling", "Juling Shadow", ["+ship/plan juling", "juling shadow"]],
  ["urvara", "Urvara Reach", ["+ship/plan urvara", "urvara reach"]],
  ["rheasilvia", "Rheasilvia Commons", ["+ship/plan rheasilvia", "rheasilvia commons"]],
];

export function normalizeGoodSlug(raw: string): string {
  let s = raw.trim().toLowerCase();
  const colon = s.lastIndexOf(":");
  if (colon >= 0 && colon < s.length - 1) s = s.slice(colon + 1);
  return s.replace(/[-\s]+/g, "_").replace(/_+/g, "_");
}

export function matchesQuestTradeRow(
  need: QuestTradeNeed | null,
  row: { goodCode?: string; itemName?: string },
  isBid: boolean,
): boolean {
  if (!need) return false;
  if (isBid ? !need.wantsSell : !need.wantsBuy) return false;
  const code = row.goodCode ? normalizeGoodSlug(row.goodCode) : "";
  if (code && code === need.commoditySlug) return true;
  const item = row.itemName ? normalizeGoodSlug(row.itemName) : "";
  return Boolean(item && item === need.commoditySlug);
}

function extractLeadingPositiveInt(text: string): number {
  const m = /\b(\d{1,4})(?:t\b|\b)/i.exec(text);
  if (!m) return 0;
  const n = Number.parseInt(m[1]!, 10);
  return Number.isFinite(n) && n > 0 ? n : 0;
}

function orderedActiveQuests(
  active: readonly ActiveQuestView[],
  trackedKey: string | null,
): ActiveQuestView[] {
  if (active.length === 0) return [];
  const tracked = trackedKey ? active.find((q) => q.key === trackedKey) : undefined;
  return tracked ? [tracked, ...active.filter((q) => q !== tracked)] : [...active];
}

export function extractActiveTradeNeeds(
  active: readonly ActiveQuestView[],
  trackedKey: string | null,
): QuestTradeNeed[] {
  for (const q of orderedActiveQuests(active, trackedKey)) {
    const out: QuestTradeNeed[] = [];
    const seen = new Set<string>();
    for (const cp of q.checkpoints) {
      if (cp.checked || cp.liveReady) continue;
      const idLow = cp.id.toLowerCase();
      const titleLow = cp.title.toLowerCase();
      const instrLow = cp.instructions.toLowerCase();
      if (
        idLow.startsWith("mine_") ||
        idLow.startsWith("scan_") ||
        idLow.startsWith("sound_") ||
        titleLow.startsWith("mine ")
      ) {
        continue;
      }
      const combined = `${idLow} ${titleLow} ${instrLow}`;
      let slug: string | null = null;
      for (const [candidate, pats] of KNOWN_TRADE_COMMODITIES) {
        if (pats.some((p) => combined.includes(p))) {
          slug = candidate;
          break;
        }
      }
      if (!slug) continue;

      const wantsBuy =
        idLow.startsWith("buy_") ||
        idLow.startsWith("deliver_") ||
        idLow.startsWith("return_") ||
        titleLow.includes("buy ") ||
        titleLow.includes("haul ") ||
        instrLow.includes("buy ");
      const wantsSell =
        idLow.startsWith("sell_") ||
        idLow.startsWith("deliver_") ||
        idLow.startsWith("return_") ||
        titleLow.includes("sell ") ||
        titleLow.includes("deliver ") ||
        titleLow.includes("haul ") ||
        instrLow.includes("+market/sell");
      if (!wantsBuy && !wantsSell) continue;

      const dedupKey = `${slug}|${wantsBuy ? 1 : 0}|${wantsSell ? 1 : 0}`;
      if (seen.has(dedupKey)) continue;
      seen.add(dedupKey);

      const qty =
        extractLeadingPositiveInt(cp.title) || extractLeadingPositiveInt(cp.instructions) || 1;
      out.push({
        questTitle: q.title,
        checkpointTitle: cp.title,
        commoditySlug: slug,
        targetQuantity: qty,
        wantsBuy,
        wantsSell,
      });
    }
    if (out.length > 0) return out;
  }
  return [];
}

export function extractActiveTradeNeed(
  active: readonly ActiveQuestView[],
  trackedKey: string | null,
): QuestTradeNeed | null {
  return extractActiveTradeNeeds(active, trackedKey)[0] ?? null;
}

/**
 * Resolves the most relevant Exchange trade need for the pilot's current stowage and venue book:
 * - For multi-item buy quests (e.g. Step 2: 10 provisions + 5 propellant), advances past items
 *   already satisfied in live stowage and prefers an unsatisfied item present on the current book.
 * - For dual buy+sell delivery runs (Steps 3 & 4), narrows to SELL-only once the required cargo
 *   quantity is already loaded in the ship's hold so only Standing Bids are highlighted.
 */
export function resolveContextualTradeNeed(
  needs: readonly QuestTradeNeed[],
  stowage:
    | {
        propellantKg: number;
        provisionsCrewDays: number;
        cargoLines: ReadonlyArray<{ good: string; label: string; qty: number }>;
      }
    | null
    | undefined,
  asks: ReadonlyArray<{ goodCode?: string; itemName?: string }> = [],
  bids: ReadonlyArray<{ goodCode?: string; itemName?: string }> = [],
): QuestTradeNeed | null {
  if (needs.length === 0) return null;

  const isBuyAlreadySatisfied = (n: QuestTradeNeed): boolean => {
    if (!n.wantsBuy || n.wantsSell || !stowage) return false;
    return evaluateQuestStowageReadiness(n, stowage)?.ready === true;
  };

  const unsatisfied = needs.filter((n) => !isBuyAlreadySatisfied(n));
  const pool = unsatisfied.length > 0 ? unsatisfied : needs;

  const matchesBook = (n: QuestTradeNeed): boolean =>
    asks.some((row) => matchesQuestTradeRow(n, row, false)) ||
    bids.some((row) => matchesQuestTradeRow(n, row, true));

  const picked =
    (asks.length > 0 || bids.length > 0 ? pool.find((n) => matchesBook(n)) : undefined) ?? pool[0]!;

  if (picked.wantsBuy && picked.wantsSell && stowage) {
    const r = evaluateQuestStowageReadiness(picked, stowage);
    if (r?.ready) {
      return { ...picked, wantsBuy: false, wantsSell: true };
    }
  }
  return picked;
}

export function extractActiveDestinationPort(
  active: readonly ActiveQuestView[],
  trackedKey: string | null,
): QuestDestinationHint | null {
  for (const q of orderedActiveQuests(active, trackedKey)) {
    let hasPendingCheckpoint = false;
    for (const cp of q.checkpoints) {
      if (cp.checked || cp.liveReady) continue;
      hasPendingCheckpoint = true;
      const text = `${cp.instructions} ${cp.title} ${q.nextInstructions} ${q.introduction}`.toLowerCase();
      // Prioritize explicit `+ship/plan <port>` or `+ship/order depart <port>` commands first
      const cmdMatch = /\+ship\/(?:plan|order\s+depart|undock)\s+([a-z0-9_]+)/i.exec(text);
      if (cmdMatch) {
        const slug = cmdMatch[1]!.toLowerCase();
        const found = KNOWN_DESTINATIONS.find(([id]) => id === slug);
        if (found) {
          return { questTitle: q.title, portId: found[0], portLabel: found[1] };
        }
      }
      for (const [id, label, pats] of KNOWN_DESTINATIONS) {
        if (pats.some((p) => text.includes(p))) {
          return { questTitle: q.title, portId: id, portLabel: label };
        }
      }
    }
    if (
      !q.canFinishNow &&
      !q.turnInHere &&
      (q.readyToFinish || (!hasPendingCheckpoint && q.checkpoints.length > 0))
    ) {
      const turnInRoom =
        q.nextRoomEntry || q.checkpoints[q.checkpoints.length - 1]?.roomEntry || null;
      const turnInPort = inferStationPortFromRoomEntry(turnInRoom);
      if (turnInPort) {
        const found = KNOWN_DESTINATIONS.find(([id]) => id === turnInPort);
        if (found) {
          return { questTitle: q.title, portId: found[0], portLabel: found[1] };
        }
      }
    }
  }
  return null;
}

export function inferStationPortFromRoomEntry(
  roomEntry: string | null | undefined,
): string | null {
  if (!roomEntry) return null;
  const low = roomEntry.trim().toLowerCase();
  if (/^room\.c\d/.test(low)) return "ceres";
  if (/^room\.p\d/.test(low)) return "stickney";
  if (/^room\.l\d/.test(low)) return "luna";
  if (/^room\.d\d/.test(low)) return "deimos";
  if (/^room\.t\d/.test(low)) return "titan";
  return null;
}

/**
 * True when the tracked quest requires an active surface mining expedition
 * (`scan` / `sound` / `mine` not yet complete) so a player standing ashore in a station
 * should head to Berth Gallery and board their Dray-P rather than staying at the turn-in desk.
 */
export function needsSurfaceExpeditionBoarding(
  active: readonly ActiveQuestView[],
  trackedKey: string | null,
): boolean {
  const tracked = orderedActiveQuests(active, trackedKey)[0] ?? null;
  if (!tracked || tracked.readyToFinish || tracked.canFinishNow) return false;
  const mining = extractActiveSurfaceMiningStatus([tracked], tracked.key);
  return mining !== null && !mining.mineDone;
}

export function resolveQuestShipTargetCompartment(
  active: readonly ActiveQuestView[],
  trackedKey: string | null,
  dockedPortId: string | null = null,
  settledPad = false,
  nextGuidedOffer: GuidedTourRoadmapStep | null = null,
): "airlock" | "bay" | "bridge" | null {
  const tracked = orderedActiveQuests(active, trackedKey)[0] ?? null;
  const trackedOnly = tracked ? [tracked] : [];
  const mining = tracked ? extractActiveSurfaceMiningStatus(trackedOnly, tracked.key) : null;
  const dockedNorm = dockedPortId?.trim().toLowerCase() ?? "";
  if (dockedNorm && !settledPad && tracked) {
    if (!mining || mining.mineDone || tracked.readyToFinish || tracked.canFinishNow) {
      const targetEntry =
        tracked.nextRoomEntry ||
        tracked.checkpoints.find((c) => !c.checked && !c.liveReady && c.roomEntry)?.roomEntry ||
        null;
      const targetPort = inferStationPortFromRoomEntry(targetEntry);
      if (targetPort && targetPort === dockedNorm) {
        return "airlock";
      }
    }
  }
  if (mining) {
    if (!mining.mineDone) {
      // Docked inside an orbital station berth -> head to Bridge to undock and land at a surface site;
      // once settled on a surface landing pad (or when station-docked is unset) -> head to Vehicle Bay.
      return dockedNorm && !settledPad ? "bridge" : "bay";
    }
    // Mining done, not yet docked at turn-in station -> head to Bridge to lift off & return to port.
    return "bridge";
  }
  const dest = tracked ? extractActiveDestinationPort(trackedOnly, tracked.key) : null;
  if (dest) {
    return "bridge";
  }
  if (active.length === 0 && nextGuidedOffer) {
    const targetPort = inferStationPortFromRoomEntry(nextGuidedOffer.giverRoomEntry);
    if (dockedNorm && !settledPad && targetPort && targetPort === dockedNorm) {
      return "airlock";
    }
    return "bridge";
  }
  return null;
}

export function extractActiveSurfaceMiningStatus(
  active: readonly ActiveQuestView[],
  trackedKey: string | null,
): QuestSurfaceMiningStatus | null {
  for (const q of orderedActiveQuests(active, trackedKey)) {
    let hasScan = false;
    let scanDone = false;
    let hasSound = false;
    let soundDone = false;
    let hasMine = false;
    let mineDone = false;
    let oreSlug = "";

    for (const cp of q.checkpoints) {
      const idLow = cp.id.toLowerCase();
      const textLow = `${idLow} ${cp.title.toLowerCase()} ${cp.instructions.toLowerCase()}`;
      const done = cp.checked || cp.liveReady;

      if (idLow.includes("scan") || textLow.includes("surface scan") || textLow.includes("scan go")) {
        hasScan = true;
        scanDone = done;
      } else if (
        idLow.includes("sound") ||
        textLow.includes("sound a detected") ||
        textLow.includes("sound go")
      ) {
        hasSound = true;
        soundDone = done;
      } else if (
        idLow.startsWith("mine_") ||
        textLow.includes("mine at least") ||
        textLow.includes("mine go")
      ) {
        hasMine = true;
        mineDone = done;
        if (textLow.includes("carbon_ore") || textLow.includes("carbon ore")) {
          oreSlug = "carbon_ore";
        } else if (textLow.includes("rare_earths") || textLow.includes("rare earths")) {
          oreSlug = "rare_earths";
        } else if (textLow.includes("volatiles") || textLow.includes("water_ice")) {
          oreSlug = "volatiles";
        } else if (textLow.includes("metals")) {
          oreSlug = "metals";
        }
      }
    }

    if (hasScan || hasSound || hasMine) {
      const targetOreSlug = oreSlug || "carbon_ore";
      const recommendedFamily =
        targetOreSlug === "carbon_ore"
          ? "carbon"
          : targetOreSlug === "rare_earths"
            ? "strategic"
            : targetOreSlug === "metals"
              ? "metals"
              : "volatiles";
      return {
        questTitle: q.title,
        targetOreSlug,
        recommendedFamily,
        hasScanStep: hasScan,
        scanDone,
        hasSoundStep: hasSound,
        soundDone,
        hasMineStep: hasMine,
        mineDone,
        nextStepText: q.nextStep,
      };
    }
  }
  return null;
}

function asObj(v: unknown): Record<string, unknown> | null {
  return typeof v === "object" && v !== null && !Array.isArray(v)
    ? (v as Record<string, unknown>)
    : null;
}

function asStr(v: unknown, fallback = ""): string {
  return typeof v === "string" ? v : fallback;
}

function asBool(v: unknown, fallback = false): boolean {
  return typeof v === "boolean" ? v : fallback;
}

function asNum(v: unknown, fallback = 0): number {
  return typeof v === "number" && Number.isFinite(v) ? v : fallback;
}

export function parseQuestMarkers(raw: unknown): NpcQuestMarker[] {
  if (!Array.isArray(raw)) return [];
  const out: NpcQuestMarker[] = [];
  for (const item of raw) {
    const o = asObj(item);
    if (!o) continue;
    const npcId = asStr(o["npc_id"]);
    const npcName = asStr(o["npc_name"]);
    const markerRaw = asStr(o["marker"]);
    if (!npcId || !npcName) continue;
    if (markerRaw !== "turn_in" && markerRaw !== "available" && markerRaw !== "in_progress") {
      continue;
    }
    const symbol: "!" | "?" = markerRaw === "available" ? "!" : "?";
    out.push({
      npcId,
      npcName,
      marker: markerRaw,
      symbol,
      questKey: asStr(o["quest_key"]),
      questTitle: asStr(o["quest_title"]),
      category: asStr(o["category"], "optional"),
      questCount: Math.max(1, asNum(o["quest_count"], 1)),
    });
  }
  return out;
}

const MARKER_PRIORITY: Record<QuestMarkerKind, number> = {
  turn_in: 0,
  available: 1,
  in_progress: 2,
};

/**
 * Orders and coalesces NPC quest markers in the current room by action urgency:
 * gold turn-in (`?`) -> gold available offer (`!`) -> silver in-progress (`?`).
 * Duplicate entries for the same NPC are merged into one highest-priority marker
 * with an aggregated `questCount`.
 */
export function sortRoomQuestMarkers(
  markers: readonly NpcQuestMarker[],
): NpcQuestMarker[] {
  if (markers.length === 0) return [];
  const byNpc = new Map<
    string,
    { best: NpcQuestMarker; questKeys: Set<string>; maxCount: number }
  >();
  for (const m of markers) {
    const npcKey = m.npcId || m.npcName.trim().toLowerCase();
    const qKey = m.questKey || `${m.marker}:${m.questTitle}`;
    const existing = byNpc.get(npcKey);
    if (!existing) {
      byNpc.set(npcKey, {
        best: m,
        questKeys: new Set([qKey]),
        maxCount: Math.max(1, m.questCount),
      });
    } else {
      existing.questKeys.add(qKey);
      existing.maxCount = Math.max(existing.maxCount, m.questCount);
      if (MARKER_PRIORITY[m.marker] < MARKER_PRIORITY[existing.best.marker]) {
        existing.best = m;
      }
    }
  }
  return [...byNpc.values()]
    .map(({ best, questKeys, maxCount }) => {
      const count = Math.max(maxCount, questKeys.size);
      return count === best.questCount ? best : { ...best, questCount: count };
    })
    .sort((a, b) => MARKER_PRIORITY[a.marker] - MARKER_PRIORITY[b.marker]);
}

function parseCheckpoints(raw: unknown): QuestCheckpointView[] {
  if (!Array.isArray(raw)) return [];
  const out: QuestCheckpointView[] = [];
  for (const item of raw) {
    const o = asObj(item);
    if (!o) continue;
    out.push({
      id: asStr(o["id"]),
      title: asStr(o["title"]),
      instructions: asStr(o["instructions"]),
      roomEntry: asStr(o["room_entry"]),
      roomName: asStr(o["room_name"]),
      checked: asBool(o["checked"]),
      liveReady: asBool(o["live_ready"]),
      detailText: typeof o["detail_text"] === "string" ? o["detail_text"] : null,
    });
  }
  return out;
}

function parseActive(raw: unknown): ActiveQuestView[] {
  if (!Array.isArray(raw)) return [];
  const out: ActiveQuestView[] = [];
  for (const item of raw) {
    const o = asObj(item);
    if (!o) continue;
    const key = asStr(o["key"]);
    if (!key) continue;
    out.push({
      key,
      title: asStr(o["title"], key),
      category: asStr(o["category"], "optional"),
      introduction: asStr(o["introduction"]),
      rewardSummary: asStr(o["reward_summary"]),
      giverId: asStr(o["giver_id"]),
      giverName: asStr(o["giver_name"]),
      turnInId: asStr(o["turn_in_id"]),
      turnInName: asStr(o["turn_in_name"]),
      turnInHere: asBool(o["turn_in_here"]),
      allowRestart: asBool(o["allow_restart"]),
      readyToFinish: asBool(o["ready_to_finish"]),
      nextStep: asStr(o["next_step"]),
      nextInstructions: asStr(o["next_instructions"]),
      nextRoomEntry: asStr(o["next_room_entry"]),
      nextRoomName: asStr(o["next_room_name"]),
      nextObjectiveReady: asBool(o["next_objective_ready"]),
      canCheckNow: asBool(o["can_check_now"]),
      canFinishNow: asBool(o["can_finish_now"]),
      checkCmd: asStr(o["check_cmd"]),
      finishCmd: asStr(o["finish_cmd"]),
      abandonCmd: asStr(o["abandon_cmd"]),
      checkpoints: parseCheckpoints(o["checkpoints"]),
    });
  }
  return out;
}

function parseAvailable(raw: unknown): AvailableQuestView[] {
  if (!Array.isArray(raw)) return [];
  const out: AvailableQuestView[] = [];
  for (const item of raw) {
    const o = asObj(item);
    if (!o) continue;
    const key = asStr(o["key"]);
    if (!key) continue;
    out.push({
      key,
      title: asStr(o["title"], key),
      category: asStr(o["category"], "optional"),
      introduction: asStr(o["introduction"]),
      rewardSummary: asStr(o["reward_summary"]),
      giverId: asStr(o["giver_id"]),
      giverName: asStr(o["giver_name"]),
      turnInId: asStr(o["turn_in_id"]),
      turnInName: asStr(o["turn_in_name"]),
      allowRestart: asBool(o["allow_restart"]),
      acceptCmd: asStr(o["accept_cmd"]),
      declineCmd: asStr(o["decline_cmd"]),
      checkpoints: parseCheckpoints(o["checkpoints"]),
    });
  }
  return out;
}

function parseCompleted(raw: unknown): CompletedQuestView[] {
  if (!Array.isArray(raw)) return [];
  const out: CompletedQuestView[] = [];
  for (const item of raw) {
    const o = asObj(item);
    if (!o) continue;
    const key = asStr(o["key"]);
    if (!key) continue;
    out.push({
      key,
      title: asStr(o["title"], key),
      category: asStr(o["category"], "optional"),
      giverName: asStr(o["giver_name"]),
      turnInName: asStr(o["turn_in_name"]),
      completion: asStr(o["completion"]),
      rewardSummary: asStr(o["reward_summary"]),
      completedAtUtc: asNum(o["completed_at_utc"]),
    });
  }
  return out;
}

function pickDefaultTrackedKey(active: readonly ActiveQuestView[]): string | null {
  if (active.length === 0) return null;
  const guided = active.find((q) => GUIDED_TOUR_KEYS.includes(q.key) || q.category === "guided");
  return (guided ?? active[0])?.key ?? null;
}

export class QuestsStore {
  loaded = $state(false);
  roomId = $state("");
  roomName = $state("");
  roomMarkers = $state<NpcQuestMarker[]>([]);
  active = $state<ActiveQuestView[]>([]);
  available = $state<AvailableQuestView[]>([]);
  completed = $state<CompletedQuestView[]>([]);

  trackedKey = $state<string | null>(null);
  trackerCollapsed = $state(false);
  logOpen = $state(false);
  logTab = $state<"active" | "available" | "completed">("active");
  dialogOpen = $state(false);
  dialogNpcId = $state<string | null>(null);
  dialogQuestKey = $state<string | null>(null);

  #hydrated = false;
  #seenActive = new Set<string>();
  #seenActiveMeta = new Map<string, { title: string; allowRestart: boolean }>();
  #seenCheckpoints = new Set<string>();
  #seenReady = new Set<string>();
  #seenCompleted = new Set<string>();
  #seenTourAllDone = false;

  get trackedQuest(): ActiveQuestView | null {
    if (this.active.length === 0) return null;
    if (this.trackedKey !== null) {
      const found = this.active.find((q) => q.key === this.trackedKey);
      if (found) return found;
    }
    const defKey = pickDefaultTrackedKey(this.active);
    return (defKey ? this.active.find((q) => q.key === defKey) : null) ?? this.active[0] ?? null;
  }

  get badgeCount(): number {
    return this.active.length + this.available.length;
  }

  get prioritizedRoomMarkers(): NpcQuestMarker[] {
    return sortRoomQuestMarkers(this.roomMarkers);
  }

  get hasTurnInHere(): boolean {
    return (
      this.active.some((q) => q.canFinishNow) ||
      this.roomMarkers.some((m) => m.marker === "turn_in")
    );
  }

  get hasAvailableHere(): boolean {
    return (
      this.available.length > 0 ||
      this.roomMarkers.some((m) => m.marker === "available")
    );
  }

  get hasOpenOverlay(): boolean {
    return this.dialogOpen || this.logOpen;
  }

  get activeTradeNeed(): QuestTradeNeed | null {
    return extractActiveTradeNeed(this.active, this.trackedKey);
  }

  get activeTradeNeeds(): QuestTradeNeed[] {
    return extractActiveTradeNeeds(this.active, this.trackedKey);
  }

  get activeDestinationPort(): QuestDestinationHint | null {
    return extractActiveDestinationPort(this.active, this.trackedKey);
  }

  get surfaceMiningStatus(): QuestSurfaceMiningStatus | null {
    return extractActiveSurfaceMiningStatus(this.active, this.trackedKey);
  }

  get guidedTourProgress(): GuidedTourProgress | null {
    return computeGuidedTourProgress(this.active, this.available, this.completed);
  }

  get guidedTourRoadmap(): GuidedTourRoadmapStep[] {
    return buildGuidedTourRoadmap(this.active, this.available, this.completed);
  }

  get nextGuidedOffer(): GuidedTourRoadmapStep | null {
    return findNextGuidedOffer(this.active, this.available, this.completed);
  }

  get suggestedAction(): QuestSuggestedAction | null {
    return extractQuestSuggestedAction(this.trackedQuest);
  }

  applyFrame(data: unknown): QuestMilestone[] {
    const o = asObj(data);
    if (!o) return [];
    const nextActive = parseActive(o["active"]);
    const nextAvailable = parseAvailable(o["available"]);
    const nextCompleted = parseCompleted(o["completed"]);

    const nextActiveKeys = new Set(nextActive.map((a) => a.key).filter(Boolean));
    const nextCompletedKeys = new Set(nextCompleted.map((c) => c.key).filter(Boolean));
    const nextTourAllDone = GUIDED_TOUR_KEYS.every((k) => nextCompletedKeys.has(k));

    const milestones: QuestMilestone[] = [];
    if (this.#hydrated) {
      const newlyCompletedTour = nextTourAllDone && !this.#seenTourAllDone;
      for (const c of nextCompleted) {
        if (c.key && !this.#seenCompleted.has(c.key)) {
          const isMasteryCompletion = newlyCompletedTour && GUIDED_TOUR_KEYS.includes(c.key);
          milestones.push({
            kind: "completed",
            key: c.key,
            title: c.title,
            ...(isMasteryCompletion ? { tourAllDone: true } : {}),
          });
        }
      }
      for (const [prevKey, prevMeta] of this.#seenActiveMeta) {
        if (!nextActiveKeys.has(prevKey) && !nextCompletedKeys.has(prevKey)) {
          milestones.push({
            kind: "abandoned",
            key: prevKey,
            title: prevMeta.title,
            restartable: prevMeta.allowRestart,
          });
        }
      }
      for (const a of nextActive) {
        if (!a.key) continue;
        const isReady = a.readyToFinish || a.canFinishNow;
        if (!this.#seenActive.has(a.key)) {
          milestones.push({ kind: "accepted", key: a.key, title: a.title });
        } else if (isReady && !this.#seenReady.has(a.key)) {
          milestones.push({ kind: "ready", key: a.key, title: a.title });
        } else {
          for (const cp of a.checkpoints) {
            if (!cp.id || (!cp.checked && !cp.liveReady)) continue;
            const cpKey = `${a.key}::${cp.id}`;
            if (!this.#seenCheckpoints.has(cpKey)) {
              milestones.push({
                kind: "checkpoint",
                key: a.key,
                title: a.title,
                detail: cp.title,
              });
            }
          }
        }
      }
    }

    this.#hydrated = true;
    this.#seenActive = nextActiveKeys;
    this.#seenActiveMeta = new Map(
      nextActive
        .filter((a) => Boolean(a.key))
        .map((a) => [a.key, { title: a.title, allowRestart: a.allowRestart }]),
    );
    const nextCheckpoints = new Set<string>();
    for (const a of nextActive) {
      if (!a.key) continue;
      for (const cp of a.checkpoints) {
        if (cp.id && (cp.checked || cp.liveReady)) {
          nextCheckpoints.add(`${a.key}::${cp.id}`);
        }
      }
    }
    this.#seenCheckpoints = nextCheckpoints;
    this.#seenReady = new Set(
      nextActive.filter((a) => a.readyToFinish || a.canFinishNow).map((a) => a.key).filter(Boolean),
    );
    for (const c of nextCompleted) {
      if (c.key) this.#seenCompleted.add(c.key);
    }
    this.#seenTourAllDone = nextTourAllDone;

    this.loaded = true;
    this.roomId = asStr(o["room_id"]);
    this.roomName = asStr(o["room_name"]);
    this.roomMarkers = parseQuestMarkers(o["room_markers"]);
    this.active = nextActive;
    this.available = nextAvailable;
    this.completed = nextCompleted;

    if (this.trackedKey !== null && !this.active.some((q) => q.key === this.trackedKey)) {
      this.trackedKey = pickDefaultTrackedKey(this.active);
    } else if (this.trackedKey === null && this.active.length > 0) {
      this.trackedKey = pickDefaultTrackedKey(this.active);
    }
    return milestones;
  }

  observeRoom(view: RoomView): void {
    this.roomId = view.id;
    this.roomName = view.name;
    const amb = asObj(view.ambient);
    this.roomMarkers = parseQuestMarkers(amb?.["quest_markers"]);
  }

  track(key: string): void {
    this.trackedKey = key;
    this.trackerCollapsed = false;
  }

  cycleTracked(): string | null {
    if (this.active.length === 0) return null;
    if (this.active.length === 1) {
      this.trackedKey = this.active[0]!.key;
      return this.trackedKey;
    }
    const curIdx = this.active.findIndex((q) => q.key === this.trackedKey);
    const nextIdx = curIdx >= 0 ? (curIdx + 1) % this.active.length : 1;
    const next = this.active[nextIdx]!;
    this.trackedKey = next.key;
    this.trackerCollapsed = false;
    return next.key;
  }

  openNpcDialog(npcId: string): void {
    this.logOpen = false;
    this.dialogNpcId = npcId;
    const marker = this.roomMarkers.find((m) => m.npcId === npcId);
    this.dialogQuestKey = marker?.questKey || null;
    this.dialogOpen = true;
  }

  openQuestDialog(questKey: string): void {
    this.logOpen = false;
    this.dialogNpcId = null;
    this.dialogQuestKey = questKey;
    this.dialogOpen = true;
  }

  openLog(tab?: "active" | "available" | "completed"): void {
    this.dialogOpen = false;
    if (tab) {
      this.logTab = tab;
    } else if (this.active.length > 0) {
      this.logTab = "active";
    } else if (this.available.length > 0) {
      this.logTab = "available";
    } else {
      this.logTab = "completed";
    }
    this.logOpen = true;
  }

  closeModals(): void {
    this.dialogOpen = false;
    this.logOpen = false;
  }

  reset(): void {
    this.loaded = false;
    this.roomId = "";
    this.roomName = "";
    this.roomMarkers = [];
    this.active = [];
    this.available = [];
    this.completed = [];
    this.trackedKey = null;
    this.dialogOpen = false;
    this.dialogNpcId = null;
    this.dialogQuestKey = null;
    this.logOpen = false;
    this.#hydrated = false;
    this.#seenActive.clear();
    this.#seenActiveMeta.clear();
    this.#seenCheckpoints.clear();
    this.#seenReady.clear();
    this.#seenCompleted.clear();
    this.#seenTourAllDone = false;
  }
}
