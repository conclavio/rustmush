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
  kind: "accepted" | "ready" | "completed";
  key: string;
  title: string;
}

export interface GuidedTourProgress {
  completed: number;
  total: number;
  currentStep: number | null;
  currentTitle: string;
  allDone: boolean;
}

export interface QuestSuggestedAction {
  label: string;
  command: string;
}

export const GUIDED_TOUR_KEYS: readonly string[] = [
  "tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll",
  "tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel",
  "tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery",
  "tacsim.quests.guided_onboarding/quest.guide_04_return_haul",
  "tacsim.quests.guided_onboarding/quest.guide_05_refit_prospector",
  "tacsim.quests.guided_onboarding/quest.guide_06_surface_mining",
  "tacsim.quests.guided_onboarding/quest.guide_07_sell_mined_ore",
];

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
  const m = /\b(\d{1,4})\b/.exec(text);
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

export function extractActiveTradeNeed(
  active: readonly ActiveQuestView[],
  trackedKey: string | null,
): QuestTradeNeed | null {
  for (const q of orderedActiveQuests(active, trackedKey)) {
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

      const qty =
        extractLeadingPositiveInt(cp.title) || extractLeadingPositiveInt(cp.instructions) || 1;
      return {
        questTitle: q.title,
        checkpointTitle: cp.title,
        commoditySlug: slug,
        targetQuantity: qty,
        wantsBuy,
        wantsSell,
      };
    }
  }
  return null;
}

export function extractActiveDestinationPort(
  active: readonly ActiveQuestView[],
  trackedKey: string | null,
): QuestDestinationHint | null {
  for (const q of orderedActiveQuests(active, trackedKey)) {
    for (const cp of q.checkpoints) {
      if (cp.checked || cp.liveReady) continue;
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

export function resolveQuestShipTargetCompartment(
  active: readonly ActiveQuestView[],
  trackedKey: string | null,
  dockedPortId: string | null = null,
): "airlock" | "bay" | "bridge" | null {
  const mining = extractActiveSurfaceMiningStatus(active, trackedKey);
  const dockedNorm = dockedPortId?.trim().toLowerCase() ?? "";
  if (dockedNorm) {
    const tracked = orderedActiveQuests(active, trackedKey)[0] ?? null;
    if (tracked && (!mining || mining.mineDone || tracked.readyToFinish || tracked.canFinishNow)) {
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
  if (mining && !mining.mineDone) {
    return "bay";
  }
  const dest = extractActiveDestinationPort(active, trackedKey);
  if (dest) {
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
  #seenReady = new Set<string>();
  #seenCompleted = new Set<string>();

  get trackedQuest(): ActiveQuestView | null {
    if (this.active.length === 0) return null;
    if (this.trackedKey !== null) {
      const found = this.active.find((q) => q.key === this.trackedKey);
      if (found) return found;
    }
    return this.active[0] ?? null;
  }

  get badgeCount(): number {
    return this.active.length + this.available.length;
  }

  get hasOpenOverlay(): boolean {
    return this.dialogOpen || this.logOpen;
  }

  get activeTradeNeed(): QuestTradeNeed | null {
    return extractActiveTradeNeed(this.active, this.trackedKey);
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

  get suggestedAction(): QuestSuggestedAction | null {
    return extractQuestSuggestedAction(this.trackedQuest);
  }

  applyFrame(data: unknown): QuestMilestone[] {
    const o = asObj(data);
    if (!o) return [];
    const nextActive = parseActive(o["active"]);
    const nextAvailable = parseAvailable(o["available"]);
    const nextCompleted = parseCompleted(o["completed"]);

    const milestones: QuestMilestone[] = [];
    if (this.#hydrated) {
      for (const c of nextCompleted) {
        if (c.key && !this.#seenCompleted.has(c.key)) {
          milestones.push({ kind: "completed", key: c.key, title: c.title });
        }
      }
      for (const a of nextActive) {
        if (!a.key) continue;
        const isReady = a.readyToFinish || a.canFinishNow;
        if (!this.#seenActive.has(a.key)) {
          milestones.push({ kind: "accepted", key: a.key, title: a.title });
        } else if (isReady && !this.#seenReady.has(a.key)) {
          milestones.push({ kind: "ready", key: a.key, title: a.title });
        }
      }
    }

    this.#hydrated = true;
    this.#seenActive = new Set(nextActive.map((a) => a.key).filter(Boolean));
    this.#seenReady = new Set(
      nextActive.filter((a) => a.readyToFinish || a.canFinishNow).map((a) => a.key).filter(Boolean),
    );
    for (const c of nextCompleted) {
      if (c.key) this.#seenCompleted.add(c.key);
    }

    this.loaded = true;
    this.roomId = asStr(o["room_id"]);
    this.roomName = asStr(o["room_name"]);
    this.roomMarkers = parseQuestMarkers(o["room_markers"]);
    this.active = nextActive;
    this.available = nextAvailable;
    this.completed = nextCompleted;

    if (this.trackedKey !== null && !this.active.some((q) => q.key === this.trackedKey)) {
      this.trackedKey = this.active[0]?.key ?? null;
    } else if (this.trackedKey === null && this.active.length > 0) {
      this.trackedKey = this.active[0]?.key ?? null;
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
    this.#seenReady.clear();
    this.#seenCompleted.clear();
  }
}
