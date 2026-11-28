import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import {
  GUIDED_TOUR_KEYS,
  GUIDED_TOUR_ROADMAP_META,
  QuestsStore,
  evaluateRefitHoldReadiness,
  evaluateSailingPreflightReadiness,
  extractActiveDestinationPort,
  extractActiveSurfaceMiningStatus,
  extractActiveTradeNeed,
  extractActiveTradeNeeds,
  extractQuestSuggestedAction,
  extractSurfaceMiningPrepActions,
  formatQuestHoldReadinessSummary,
  inferStationPortFromRoomEntry,
  needsOriginPortProcurement,
  needsSurfaceExpeditionBoarding,
  resolveContextualTradeNeed,
  resolveQuestShipTargetCompartment,
} from "./quests.svelte";

interface RawCheckpoint {
  id: string;
  title: string;
  room_entry?: string;
  instructions?: string;
}

interface RawQuestDef {
  title: string;
  category: string;
  allow_restart: boolean;
  reward_summary: string;
  introduction: string;
  completion: string;
  npc: string;
  turn_in_npc?: string;
  checkpoints: RawCheckpoint[];
}

interface RawOnboardingPack {
  manifest: { id: string };
  entries: Record<string, { kind: string; def: Record<string, unknown> }>;
}

function loadCanonicalPack(): RawOnboardingPack {
  const jsonPath = resolve(
    process.cwd(),
    "../crates/tacsim/content/quests/guided_onboarding.json",
  );
  return JSON.parse(readFileSync(jsonPath, "utf8")) as RawOnboardingPack;
}

function makeActivePayload(
  packId: string,
  entryName: string,
  def: RawQuestDef,
  opts: {
    giverName: string;
    turnInName: string;
    checkedIds?: readonly string[];
    readyToFinish?: boolean;
    canFinishNow?: boolean;
    turnInHere?: boolean;
    nextRoomEntry?: string;
  },
) {
  const checked = new Set(opts.checkedIds ?? []);
  const checkpoints = def.checkpoints.map((cp) => ({
    id: cp.id,
    title: cp.title,
    instructions: cp.instructions ?? "",
    room_entry: cp.room_entry ?? "",
    room_name: cp.room_entry ?? "",
    checked: checked.has(cp.id),
    live_ready: checked.has(cp.id),
  }));
  const firstPending = checkpoints.find((cp) => !cp.checked);
  return {
    key: `${packId}/${entryName}`,
    title: def.title,
    category: def.category,
    introduction: def.introduction,
    reward_summary: def.reward_summary,
    giver_name: opts.giverName,
    turn_in_name: opts.turnInName,
    allow_restart: def.allow_restart,
    ready_to_finish: Boolean(opts.readyToFinish),
    can_finish_now: Boolean(opts.canFinishNow),
    turn_in_here: Boolean(opts.turnInHere),
    next_step: firstPending ? firstPending.title : `Turn in to ${opts.turnInName}`,
    next_instructions: firstPending?.instructions ?? "",
    next_room_entry:
      opts.nextRoomEntry ??
      firstPending?.room_entry ??
      checkpoints[checkpoints.length - 1]?.room_entry ??
      "",
    checkpoints,
  };
}

describe("Canonical 7-step Guided Onboarding Walkthrough (guided_onboarding.json)", () => {
  it("matches all 7 canonical pack quests 1-for-1 and walks spawn through Corridor Captain (7/7)", () => {
    const pack = loadCanonicalPack();
    const packId = pack.manifest.id;
    expect(packId).toBe("tacsim.quests.guided_onboarding");

    // Verify spawn NPC Maren Halvorsen at Ceres Freeport Arrivals
    const maren = pack.entries["npc.maren_halvorsen"]?.def;
    expect(maren?.["name"]).toBe("Maren Halvorsen");
    expect(maren?.["room"]).toBe("room.c03_arrivals");

    const questEntryNames = [
      "quest.guide_01_coop_enroll",
      "quest.guide_02_provisions_fuel",
      "quest.guide_03_metals_delivery",
      "quest.guide_04_return_haul",
      "quest.guide_05_refit_prospector",
      "quest.guide_06_surface_mining",
      "quest.guide_07_sell_mined_ore",
    ] as const;

    const defs = questEntryNames.map((name, idx) => {
      const entry = pack.entries[name];
      expect(entry?.kind).toBe("quest");
      const def = entry!.def as unknown as RawQuestDef;
      expect(GUIDED_TOUR_KEYS[idx]).toBe(`${packId}/${name}`);
      expect(GUIDED_TOUR_ROADMAP_META[idx]?.key).toBe(`${packId}/${name}`);
      expect(def.category).toBe("guided");
      expect(def.allow_restart).toBe(true);
      return def;
    });

    const store = new QuestsStore();
    const completed: Array<{ key: string; title: string }> = [];

    // 0. Initial spawn hydration in room.c03_arrivals: Maren offers Step 1 with [!] marker
    const initMilestones = store.applyFrame({
      schema: 1,
      room_id: "#10",
      room_name: "Port Watch Arrivals",
      room_markers: [
        {
          npc_id: "#42",
          npc_name: "Maren Halvorsen",
          marker: "available",
          symbol: "!",
          quest_key: GUIDED_TOUR_KEYS[0],
          quest_title: defs[0]!.title,
          category: "guided",
        },
      ],
      available: [
        {
          key: GUIDED_TOUR_KEYS[0],
          title: defs[0]!.title,
          category: "guided",
          giver_id: "#42",
          giver_name: "Maren Halvorsen",
          turn_in_name: "Ilse Oduya-Brandt",
          allow_restart: true,
          accept_cmd: "+quest/accept #42=tok1",
          checkpoints: [],
        },
      ],
      active: [],
      completed: [],
    });
    expect(initMilestones).toEqual([]);
    expect(store.hasAvailableHere).toBe(true);
    expect(store.guidedTourProgress).toMatchObject({ completed: 0, total: 7, currentStep: 1, allDone: false });

    // 1. Step 1 — Accept, test abandon + restartable recovery, re-accept, enroll, and turn in to Ilse
    const s1Active = makeActivePayload(packId, questEntryNames[0], defs[0]!, {
      giverName: "Maren Halvorsen",
      turnInName: "Ilse Oduya-Brandt",
    });
    expect(store.applyFrame({ schema: 1, active: [s1Active], available: [], completed })).toEqual([
      { kind: "accepted", key: GUIDED_TOUR_KEYS[0], title: defs[0]!.title },
    ]);
    expect(extractQuestSuggestedAction(store.trackedQuest)?.command).toContain("+coop/enroll");

    // Abandon Step 1 before completing any step -> emits restartable abandon milestone with exact giver/room and keeps HUD opt-out clean
    expect(store.applyFrame({ schema: 1, active: [], available: [], completed })).toEqual([
      {
        kind: "abandoned",
        key: GUIDED_TOUR_KEYS[0],
        title: defs[0]!.title,
        restartable: true,
        restartHint: "Maren Halvorsen in Port Watch Arrivals",
      },
    ]);
    expect(store.nextGuidedOffer).toBeNull();

    // Re-accept & complete Step 1 -> emits completed milestone with Next (2/7) pointer
    store.applyFrame({ schema: 1, active: [s1Active], available: [], completed });
    completed.push({ key: GUIDED_TOUR_KEYS[0]!, title: defs[0]!.title });
    expect(store.applyFrame({ schema: 1, active: [], available: [], completed })).toEqual([
      {
        kind: "completed",
        key: GUIDED_TOUR_KEYS[0],
        title: defs[0]!.title,
        nextStepHint: "Next (2/7): Ilse Oduya-Brandt in Registry & Co-op Desk",
      },
    ]);
    expect(store.nextGuidedOffer).toMatchObject({ step: 2, giverRoomEntry: "room.c05_registry" });

    // 2. Step 2 — Accept, abandon mid-tour (points back to Step 2 Ilse), re-accept, and verify hold readiness
    const s2Active = makeActivePayload(packId, questEntryNames[1], defs[1]!, {
      giverName: "Ilse Oduya-Brandt",
      turnInName: "Ilse Oduya-Brandt",
    });
    store.applyFrame({ schema: 1, active: [s2Active], available: [], completed });
    expect(store.applyFrame({ schema: 1, active: [], available: [], completed })).toEqual([
      {
        kind: "abandoned",
        key: GUIDED_TOUR_KEYS[1],
        title: defs[1]!.title,
        restartable: true,
        restartHint: "Ilse Oduya-Brandt in Registry & Co-op Desk",
      },
    ]);
    expect(store.nextGuidedOffer).toMatchObject({
      step: 2,
      giverLabel: "Ilse Oduya-Brandt",
      giverRoomEntry: "room.c05_registry",
    });
    store.applyFrame({ schema: 1, active: [s2Active], available: [], completed });
    expect(extractActiveTradeNeeds(store.active, store.trackedKey).map((n) => n.commoditySlug)).toEqual([
      "provisions",
      "propellant",
    ]);
    const s2Partial = formatQuestHoldReadinessSummary(store.trackedQuest!, {
      cargoKg: 0,
      propellantKg: 2000,
      provisionsCrewDays: 10,
      cargoLines: [],
    });
    expect(s2Partial?.ready).toBe(false);
    expect(s2Partial?.text).toContain("PROPELLANT: 2/5 t");
    const s2Ready = formatQuestHoldReadinessSummary(store.trackedQuest!, {
      cargoKg: 0,
      propellantKg: 6000,
      provisionsCrewDays: 10,
      cargoLines: [],
    });
    expect(s2Ready?.ready).toBe(true);
    completed.push({ key: GUIDED_TOUR_KEYS[1]!, title: defs[1]!.title });
    store.applyFrame({ schema: 1, active: [], available: [], completed });

    // 3. Step 3 — Metals Down the Corridor (Ceres -> Stickney)
    const s3Active = makeActivePayload(packId, questEntryNames[2], defs[2]!, {
      giverName: "Ilse Oduya-Brandt",
      turnInName: "Jonas Achterberg-Nkemelu",
    });
    store.applyFrame({ schema: 1, active: [s3Active], available: [], completed });
    const emptyHold = { cargoKg: 0, propellantKg: 12_000, provisionsCrewDays: 12, cargoLines: [] };
    expect(needsOriginPortProcurement(store.active, store.trackedKey, emptyHold)).toBe(true);
    expect(extractActiveDestinationPort(store.active, store.trackedKey)?.portId).toBe("stickney");

    const loadedMetalsHold = {
      cargoKg: 5000,
      propellantKg: 12_000,
      provisionsCrewDays: 12,
      cargoLines: [{ good: "tacsim:metals", label: "Metals", qty: 5 }],
    };
    expect(needsOriginPortProcurement(store.active, store.trackedKey, loadedMetalsHold)).toBe(false);
    const s3NeedLoaded = resolveContextualTradeNeed(
      extractActiveTradeNeeds(store.active, store.trackedKey),
      loadedMetalsHold,
    );
    expect(s3NeedLoaded).toMatchObject({ commoditySlug: "metals", wantsBuy: false, wantsSell: true });
    expect(
      evaluateSailingPreflightReadiness(
        extractActiveDestinationPort(store.active, store.trackedKey),
        extractActiveTradeNeed(store.active, store.trackedKey),
        loadedMetalsHold,
      )?.ready,
    ).toBe(true);
    completed.push({ key: GUIDED_TOUR_KEYS[2]!, title: defs[2]!.title });
    store.applyFrame({ schema: 1, active: [], available: [], completed });
    expect(store.nextGuidedOffer).toMatchObject({
      step: 4,
      giverLabel: "Jonas Achterberg-Nkemelu",
      giverRoomEntry: "room.p07_receiving",
    });

    // 4. Step 4 — Return Load for Ceres (Stickney -> Ceres, 2t industrial_parts)
    const s4Active = makeActivePayload(packId, questEntryNames[3], defs[3]!, {
      giverName: "Jonas Achterberg-Nkemelu",
      turnInName: "Ilse Oduya-Brandt",
    });
    store.applyFrame({ schema: 1, active: [s4Active], available: [], completed });
    expect(extractActiveDestinationPort(store.active, store.trackedKey)?.portId).toBe("ceres");
    expect(extractActiveTradeNeed(store.active, store.trackedKey)?.commoditySlug).toBe("industrial_parts");
    completed.push({ key: GUIDED_TOUR_KEYS[3]!, title: defs[3]!.title });
    store.applyFrame({ schema: 1, active: [], available: [], completed });
    expect(store.nextGuidedOffer).toMatchObject({ step: 5, giverRoomEntry: "room.c05_registry" });

    // 5. Step 5 — Refit Dray to Dray-P Prospector (empty cargo & provisions; propellant stays)
    const s5Active = makeActivePayload(packId, questEntryNames[4], defs[4]!, {
      giverName: "Ilse Oduya-Brandt",
      turnInName: "Rosalind Qureshi-Mbeki",
    });
    store.applyFrame({ schema: 1, active: [s5Active], available: [], completed });
    expect(extractQuestSuggestedAction(store.trackedQuest)?.command).toBe("+coop/refit prospector");
    expect(
      evaluateRefitHoldReadiness({
        cargoKg: 2000,
        propellantKg: 15_000,
        provisionsCrewDays: 8,
        cargoLines: [{ qty: 2 }],
      })?.ready,
    ).toBe(false);
    expect(
      evaluateRefitHoldReadiness({
        cargoKg: 0,
        propellantKg: 15_000,
        provisionsCrewDays: 0,
        cargoLines: [],
      })?.ready,
    ).toBe(true);
    completed.push({ key: GUIDED_TOUR_KEYS[4]!, title: defs[4]!.title });
    store.applyFrame({ schema: 1, active: [], available: [], completed });
    expect(store.nextGuidedOffer).toMatchObject({
      step: 6,
      giverLabel: "Rosalind Qureshi-Mbeki",
      giverRoomEntry: "room.c07_exchange",
    });

    // 6. Step 6 — Surface Mining (Prep -> Scan -> Sound -> Mine 1t carbon_ore -> Post-mine Ceres Return)
    const s6Pre = makeActivePayload(packId, questEntryNames[5], defs[5]!, {
      giverName: "Rosalind Qureshi-Mbeki",
      turnInName: "Rosalind Qureshi-Mbeki",
    });
    store.applyFrame({ schema: 1, active: [s6Pre], available: [], completed });
    const mPre = extractActiveSurfaceMiningStatus(store.active, store.trackedKey);
    expect(mPre).toMatchObject({
      targetOreSlug: "carbon_ore",
      recommendedFamily: "carbon",
      scanDone: false,
      soundDone: false,
      mineDone: false,
    });
    expect(extractSurfaceMiningPrepActions(mPre).map((a) => a.command)).toEqual([
      "+license",
      "+tv/lease",
      "+chart/ground",
    ]);
    expect(needsSurfaceExpeditionBoarding(store.active, store.trackedKey)).toBe(true);
    expect(resolveQuestShipTargetCompartment(store.active, store.trackedKey, "ceres", false)).toBe("bridge");
    expect(resolveQuestShipTargetCompartment(store.active, store.trackedKey, null, true)).toBe("bay");

    const s6PostMine = makeActivePayload(packId, questEntryNames[5], defs[5]!, {
      giverName: "Rosalind Qureshi-Mbeki",
      turnInName: "Rosalind Qureshi-Mbeki",
      checkedIds: ["scan_landing_site", "sound_mineral_pocket", "mine_carbon_ore"],
      readyToFinish: true,
      canFinishNow: false,
      turnInHere: false,
    });
    store.applyFrame({ schema: 1, active: [s6PostMine], available: [], completed });
    expect(needsSurfaceExpeditionBoarding(store.active, store.trackedKey)).toBe(false);
    expect(resolveQuestShipTargetCompartment(store.active, store.trackedKey, null, true)).toBe("bridge");
    expect(extractActiveDestinationPort(store.active, store.trackedKey)?.portId).toBe("ceres");
    expect(inferStationPortFromRoomEntry(store.trackedQuest?.nextRoomEntry)).toBe("ceres");
    expect(
      formatQuestHoldReadinessSummary(store.trackedQuest!, {
        cargoKg: 1000,
        propellantKg: 10_000,
        provisionsCrewDays: 6,
        cargoLines: [{ good: "tacsim:carbon_ore", label: "Carbon Ore", qty: 1 }],
      }),
    ).toMatchObject({ tag: "[ORE HOLD READY]", ready: true });
    completed.push({ key: GUIDED_TOUR_KEYS[5]!, title: defs[5]!.title });
    store.applyFrame({ schema: 1, active: [], available: [], completed });
    expect(store.nextGuidedOffer).toMatchObject({ step: 7, giverRoomEntry: "room.c07_exchange" });

    // 7. Step 7 — Sell Mined Ore at Stickney Yards -> Corridor Captain (7/7) mastery
    const s7Active = makeActivePayload(packId, questEntryNames[6], defs[6]!, {
      giverName: "Rosalind Qureshi-Mbeki",
      turnInName: "Wren Castillo-Haugen",
    });
    store.applyFrame({ schema: 1, active: [s7Active], available: [], completed });
    expect(extractActiveDestinationPort(store.active, store.trackedKey)?.portId).toBe("stickney");
    expect(extractActiveTradeNeed(store.active, store.trackedKey)).toMatchObject({
      commoditySlug: "carbon_ore",
      wantsBuy: false,
      wantsSell: true,
    });
    const s7DryStores = evaluateSailingPreflightReadiness(
      extractActiveDestinationPort(store.active, store.trackedKey),
      extractActiveTradeNeed(store.active, store.trackedKey),
      {
        propellantKg: 12_000,
        provisionsCrewDays: 0,
        cargoLines: [{ good: "tacsim:carbon_ore", label: "Carbon Ore", qty: 2 }],
      },
    );
    expect(s7DryStores?.ready).toBe(false);
    expect(s7DryStores?.provisionsOk).toBe(false);

    const s7Ready = evaluateSailingPreflightReadiness(
      extractActiveDestinationPort(store.active, store.trackedKey),
      extractActiveTradeNeed(store.active, store.trackedKey),
      {
        propellantKg: 12_000,
        provisionsCrewDays: 15,
        cargoLines: [{ good: "tacsim:carbon_ore", label: "Carbon Ore", qty: 2 }],
      },
    );
    expect(s7Ready?.ready).toBe(true);

    // Complete Step 7 -> 7/7 Corridor Captain graduation milestone!
    completed.push({ key: GUIDED_TOUR_KEYS[6]!, title: defs[6]!.title });
    const finalMilestones = store.applyFrame({ schema: 1, active: [], available: [], completed });
    expect(finalMilestones).toEqual([
      { kind: "completed", key: GUIDED_TOUR_KEYS[6], title: defs[6]!.title, tourAllDone: true },
    ]);
    expect(store.guidedTourProgress).toMatchObject({ completed: 7, total: 7, allDone: true });
    expect(store.nextGuidedOffer).toBeNull();
  });
});
