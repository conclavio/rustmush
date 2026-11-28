import { describe, expect, it } from "vitest";
import {
  QUESTS_FEED,
  QuestsStore,
  buildGuidedTourRoadmap,
  evaluateQuestStowageReadiness,
  evaluateRefitHoldReadiness,
  evaluateSailingPreflightReadiness,
  extractSurfaceMiningPrepActions,
  inferStationPortFromRoomEntry,
  matchesQuestTradeRow,
  needsOriginPortProcurement,
  needsSurfaceExpeditionBoarding,
  parseQuestMarkers,
  resolveQuestShipTargetCompartment,
} from "./quests.svelte";

describe("QuestsStore", () => {
  it("parses room ambient quest markers with priority symbols", () => {
    const markers = parseQuestMarkers([
      {
        npc_id: "#42",
        npc_name: "Maren Halvorsen",
        marker: "available",
        symbol: "!",
        quest_key: "tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll",
        quest_title: "First Papers at the Registry",
        category: "guided",
        quest_count: 1,
      },
      {
        npc_id: "#18",
        npc_name: "Ilse Oduya-Brandt",
        marker: "turn_in",
        symbol: "?",
        quest_key: "tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll",
        quest_title: "First Papers at the Registry",
        category: "guided",
        quest_count: 1,
      },
    ]);
    expect(markers).toHaveLength(2);
    expect(markers[0]?.symbol).toBe("!");
    expect(markers[1]?.marker).toBe("turn_in");
    expect(markers[1]?.symbol).toBe("?");
  });

  it("applies tacsim.quests keyframe, auto-tracks active quests, and drives dialog/log state", () => {
    expect(QUESTS_FEED).toBe("tacsim.quests");
    const store = new QuestsStore();
    store.applyFrame({
      schema: 1,
      room_id: "#12",
      room_name: "Port Watch Arrivals",
      room_markers: [
        {
          npc_id: "#42",
          npc_name: "Maren Halvorsen",
          marker: "available",
          symbol: "!",
          quest_key: "tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll",
          quest_title: "First Papers at the Registry",
          category: "guided",
          quest_count: 1,
        },
      ],
      active: [
        {
          key: "tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel",
          title: "Stock the Locker and Tanks",
          category: "guided",
          introduction: "Buy provisions and fuel.",
          reward_summary: "Guided Tour Step 2/7",
          giver_id: "#18",
          giver_name: "Ilse Oduya-Brandt",
          turn_in_id: "#18",
          turn_in_name: "Ilse Oduya-Brandt",
          turn_in_here: false,
          allow_restart: true,
          ready_to_finish: false,
          next_step: "Buy at least 10 provisions at Ceres Co-op Chandlery",
          next_instructions: "Use +market/list and +market/quote.",
          next_room_entry: "room.c08_chandlery",
          next_room_name: "Co-op Chandlery",
          next_objective_ready: true,
          can_check_now: true,
          can_finish_now: false,
          check_cmd: "+quest/check #18=tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel",
          finish_cmd: "+quest/finish #18=tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel",
          abandon_cmd: "+quest/abandon #18=tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel",
          checkpoints: [
            {
              id: "buy_provisions",
              title: "Buy provisions",
              instructions: "Buy 10 provisions",
              room_entry: "room.c08_chandlery",
              room_name: "Co-op Chandlery",
              checked: false,
              live_ready: true,
              detail_text: null,
            },
          ],
        },
      ],
      available: [
        {
          key: "tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll",
          title: "First Papers at the Registry",
          category: "guided",
          introduction: "Enroll in the Co-op.",
          reward_summary: "Guided Tour Step 1/7",
          giver_id: "#42",
          giver_name: "Maren Halvorsen",
          turn_in_id: "#18",
          turn_in_name: "Ilse Oduya-Brandt",
          allow_restart: true,
          accept_cmd: "+quest/accept #42=123.abc",
          decline_cmd: "+quest/decline #42=123.abc",
          checkpoints: [],
        },
      ],
      completed: [
        {
          key: "tacsim.quests.guided_onboarding/quest.guide_00",
          title: "Arrived at Ceres",
          category: "guided",
          giver_name: "Maren Halvorsen",
          turn_in_name: "Maren Halvorsen",
          completion: "Done.",
          reward_summary: "Intro",
          completed_at_utc: 1700000000,
        },
      ],
    });

    expect(store.loaded).toBe(true);
    expect(store.trackedQuest?.key).toBe("tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel");
    expect(store.badgeCount).toBe(2);
    expect(store.completed).toHaveLength(1);

    store.openNpcDialog("#42");
    expect(store.dialogOpen).toBe(true);
    expect(store.dialogNpcId).toBe("#42");
    expect(store.dialogQuestKey).toBe("tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll");

    store.openLog("completed");
    expect(store.dialogOpen).toBe(false);
    expect(store.logOpen).toBe(true);
    expect(store.logTab).toBe("completed");

    store.reset();
    expect(store.loaded).toBe(false);
    expect(store.active).toHaveLength(0);
    expect(store.roomMarkers).toHaveLength(0);
  });

  it("extracts active trade needs, corridor destination ports, and surface mining checklists", () => {
    const store = new QuestsStore();

    // 1. Step 2: Buy 10 provisions -> Buy 5 propellant
    store.applyFrame({
      schema: 1,
      room_id: "#18",
      room_name: "Co-op Chandlery",
      room_markers: [],
      available: [],
      completed: [],
      active: [
        {
          key: "tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel",
          title: "Stock the Locker and Tanks",
          checkpoints: [
            {
              id: "buy_provisions",
              title: "Buy at least 10 provisions at Ceres Co-op Chandlery",
              instructions: "Request quote for 10 provisions.",
              checked: false,
              live_ready: false,
            },
            {
              id: "buy_propellant",
              title: "Buy at least 5 tonnes of propellant at Ceres Exchange Hall",
              instructions: "Request quote for 5 propellant.",
              checked: false,
              live_ready: false,
            },
          ],
        },
      ],
    });
    expect(store.activeTradeNeed).toEqual({
      questTitle: "Stock the Locker and Tanks",
      checkpointTitle: "Buy at least 10 provisions at Ceres Co-op Chandlery",
      commoditySlug: "provisions",
      targetQuantity: 10,
      wantsBuy: true,
      wantsSell: false,
    });
    expect(
      matchesQuestTradeRow(
        store.activeTradeNeed,
        { goodCode: "tacsim:provisions", itemName: "Provisions" },
        false,
      ),
    ).toBe(true);
    expect(
      matchesQuestTradeRow(
        store.activeTradeNeed,
        { goodCode: "tacsim:provisions", itemName: "Provisions" },
        true,
      ),
    ).toBe(false);
    expect(store.surfaceMiningStatus).toBeNull();

    // 2. Step 3: Buy 5t metals and deliver to Stickney Yards -> both buy & sell metals + Stickney destination
    store.applyFrame({
      schema: 1,
      room_id: "#17",
      room_name: "Exchange Hall",
      room_markers: [],
      available: [],
      completed: [],
      active: [
        {
          key: "tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery",
          title: "Metals Down the Corridor",
          checkpoints: [
            {
              id: "deliver_metals",
              title: "Buy 5 tonnes of metals at Ceres and deliver them to Stickney Yards",
              instructions: "Check +ship/plan stickney, fly to Stickney Yards, and run +market/sell.",
              checked: false,
              live_ready: false,
            },
          ],
        },
      ],
    });
    expect(store.activeTradeNeed?.commoditySlug).toBe("metals");
    expect(store.activeTradeNeed?.targetQuantity).toBe(5);
    expect(store.activeTradeNeed?.wantsBuy).toBe(true);
    expect(store.activeTradeNeed?.wantsSell).toBe(true);
    expect(store.activeDestinationPort).toEqual({
      questTitle: "Metals Down the Corridor",
      portId: "stickney",
      portLabel: "Stickney Yards",
    });

    // 3. Step 6: Scan / Sound / Mine carbon_ore -> surface mining checklist (and NO trade banner)
    store.applyFrame({
      schema: 1,
      room_id: "#30",
      room_name: "Freeport Commons",
      room_markers: [],
      available: [],
      completed: [],
      active: [
        {
          key: "tacsim.quests.guided_onboarding/quest.guide_06_surface_mining",
          title: "Scan, Sound, and Mine a Landing Site",
          next_step: "Sound a detected mineral pocket on the surface",
          checkpoints: [
            {
              id: "scan_landing_site",
              title: "Complete a surface scan at a Ceres landing site",
              instructions: "Run +tv/1 scan go.",
              checked: true,
              live_ready: true,
            },
            {
              id: "sound_mineral_pocket",
              title: "Sound a detected mineral pocket on the surface",
              instructions: "Run +tv/1 sound go.",
              checked: false,
              live_ready: false,
            },
            {
              id: "mine_carbon_ore",
              title: "Mine at least 1 tonne of carbon ore on the surface",
              instructions: "Run +tv/1 mine go on carbon_ore.",
              checked: false,
              live_ready: false,
            },
          ],
        },
      ],
    });
    expect(store.activeTradeNeed).toBeNull();
    expect(store.surfaceMiningStatus).toEqual({
      questTitle: "Scan, Sound, and Mine a Landing Site",
      targetOreSlug: "carbon_ore",
      recommendedFamily: "carbon",
      hasScanStep: true,
      scanDone: true,
      hasSoundStep: true,
      soundDone: false,
      hasMineStep: true,
      mineDone: false,
      nextStepText: "Sound a detected mineral pocket on the surface",
    });
  });

  it("emits post-hydration quest milestones (accepted, ready, completed) without firing on initial load", () => {
    const store = new QuestsStore();
    // Initial hydration must never fire toasts
    expect(
      store.applyFrame({
        schema: 1,
        room_id: "#10",
        room_name: "Arrivals",
        room_markers: [],
        available: [],
        active: [],
        completed: [],
      }),
    ).toEqual([]);

    // Accepting fires "accepted"
    expect(
      store.applyFrame({
        schema: 1,
        room_id: "#10",
        room_name: "Arrivals",
        room_markers: [],
        available: [],
        active: [{ key: "q.enroll", title: "First Papers", ready_to_finish: false }],
        completed: [],
      }),
    ).toEqual([{ kind: "accepted", key: "q.enroll", title: "First Papers" }]);

    // Becoming ready fires "ready"
    expect(
      store.applyFrame({
        schema: 1,
        room_id: "#10",
        room_name: "Registry",
        room_markers: [],
        available: [],
        active: [{ key: "q.enroll", title: "First Papers", ready_to_finish: true }],
        completed: [],
      }),
    ).toEqual([{ kind: "ready", key: "q.enroll", title: "First Papers" }]);

    // Turning in fires "completed"
    expect(
      store.applyFrame({
        schema: 1,
        room_id: "#10",
        room_name: "Registry",
        room_markers: [],
        available: [],
        active: [],
        completed: [{ key: "q.enroll", title: "First Papers" }],
      }),
    ).toEqual([{ kind: "completed", key: "q.enroll", title: "First Papers" }]);

    // Reset clears hydration state cleanly
    store.reset();
    expect(
      store.applyFrame({
        schema: 1,
        room_id: "#10",
        room_name: "Registry",
        room_markers: [],
        available: [],
        active: [],
        completed: [{ key: "q.enroll", title: "First Papers" }],
      }),
    ).toEqual([]);
  });

  it("computes 7-step guided tour progress and extracts verified suggested commands", () => {
    const store = new QuestsStore();
    store.applyFrame({
      schema: 1,
      room_id: "#12",
      room_name: "Registry & Co-op Desk",
      room_markers: [],
      available: [],
      completed: [
        { key: "tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll", title: "Step 1" },
        { key: "tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel", title: "Step 2" },
        { key: "tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery", title: "Step 3" },
        { key: "tacsim.quests.guided_onboarding/quest.guide_04_return_haul", title: "Step 4" },
      ],
      active: [
        {
          key: "tacsim.quests.guided_onboarding/quest.guide_05_refit_prospector",
          title: "Refit the Dray for Surface Mining",
          category: "guided",
          next_step: "Run +coop/refit prospector at Registry & Co-op Desk",
          checkpoints: [
            {
              id: "refit_dray_p",
              title: "Refit your Dray into a Dray-P prospector",
              instructions: "Stand at Registry and run +coop/refit prospector.",
              checked: false,
              live_ready: false,
            },
          ],
        },
      ],
    });

    expect(store.guidedTourProgress).toEqual({
      completed: 4,
      total: 7,
      currentStep: 5,
      currentTitle: "Refit the Dray for Surface Mining",
      allDone: false,
    });
    expect(store.suggestedAction).toEqual({
      label: "Refit Dray-P",
      command: "+coop/refit prospector",
    });

    // Step 1 Co-op enrollment suggests +coop/enroll
    store.applyFrame({
      schema: 1,
      room_id: "#12",
      room_name: "Registry & Co-op Desk",
      room_markers: [],
      available: [],
      completed: [],
      active: [
        {
          key: "tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll",
          title: "First Papers at the Registry",
          category: "guided",
          checkpoints: [
            {
              id: "enroll_coop",
              title: "Enroll in the Belt Co-op",
              instructions: "Run +coop/enroll <ship name> at the Registry.",
              checked: false,
              live_ready: false,
            },
          ],
        },
      ],
    });
    expect(store.suggestedAction).toEqual({
      label: "Enroll Co-op",
      command: "+coop/enroll ",
    });
  });

  it("evaluates stowage readiness across propellant, provisions, and hold cargo lots", () => {
    const stowage = {
      propellantKg: 5400,
      provisionsCrewDays: 8,
      cargoLines: [
        { good: "tacsim:metals", label: "Metals", qty: 3 },
        { good: "tacsim:carbon_ore", label: "Carbon Ore", qty: 2 },
      ],
    };
    expect(
      evaluateQuestStowageReadiness(
        {
          questTitle: "Fuel",
          checkpointTitle: "Buy 5 propellant",
          commoditySlug: "propellant",
          targetQuantity: 5,
          wantsBuy: true,
          wantsSell: false,
        },
        stowage,
      ),
    ).toEqual({
      commoditySlug: "propellant",
      label: "PROPELLANT",
      currentQty: 5,
      targetQty: 5,
      unit: "t",
      ready: true,
      gaugeKind: "propellant",
    });

    expect(
      evaluateQuestStowageReadiness(
        {
          questTitle: "Provisions",
          checkpointTitle: "Buy 10 provisions",
          commoditySlug: "provisions",
          targetQuantity: 10,
          wantsBuy: true,
          wantsSell: false,
        },
        stowage,
      ),
    ).toEqual({
      commoditySlug: "provisions",
      label: "PROVISIONS",
      currentQty: 8,
      targetQty: 10,
      unit: "d",
      ready: false,
      gaugeKind: "provisions",
    });

    expect(
      evaluateQuestStowageReadiness(
        {
          questTitle: "Metals",
          checkpointTitle: "Buy 5 metals",
          commoditySlug: "metals",
          targetQuantity: 5,
          wantsBuy: true,
          wantsSell: true,
        },
        stowage,
      ),
    ).toEqual({
      commoditySlug: "metals",
      label: "METALS",
      currentQty: 3,
      targetQty: 5,
      unit: "u",
      ready: false,
      gaugeKind: "cargo",
    });
  });

  it("evaluates Dray-P refit empty-hold readiness (propellant OK) and surface mining prep/recovery chips", () => {
    expect(
      evaluateRefitHoldReadiness({
        cargoKg: 2000,
        propellantKg: 12000,
        provisionsCrewDays: 5,
        cargoLines: [{ qty: 2 }],
      }),
    ).toEqual({
      cargoEmpty: false,
      provisionsEmpty: false,
      cargoKg: 2000,
      provisionsCrewDays: 5,
      propellantTonnes: 12,
      ready: false,
    });

    // Empty cargo + empty provisions is ready even with propellant still in the tank
    expect(
      evaluateRefitHoldReadiness({
        cargoKg: 0,
        propellantKg: 15000,
        provisionsCrewDays: 0,
        cargoLines: [],
      }),
    ).toEqual({
      cargoEmpty: true,
      provisionsEmpty: true,
      cargoKg: 0,
      provisionsCrewDays: 0,
      propellantTonnes: 15,
      ready: true,
    });

    const beforeScan = extractSurfaceMiningPrepActions({
      questTitle: "Scan, Sound, and Mine",
      targetOreSlug: "carbon_ore",
      recommendedFamily: "carbon",
      hasScanStep: true,
      scanDone: false,
      hasSoundStep: true,
      soundDone: false,
      hasMineStep: true,
      mineDone: false,
      nextStepText: "Scan site",
    });
    expect(beforeScan.map((a) => a.command)).toEqual(["+license", "+tv/lease", "+chart/ground"]);

    const midMining = extractSurfaceMiningPrepActions(
      {
        questTitle: "Scan, Sound, and Mine",
        targetOreSlug: "carbon_ore",
        recommendedFamily: "carbon",
        hasScanStep: true,
        scanDone: true,
        hasSoundStep: true,
        soundDone: false,
        hasMineStep: true,
        mineDone: false,
        nextStepText: "Sound pocket",
      },
      2,
    );
    expect(midMining.map((a) => a.command)).toEqual(["+chart/ground", "+tv/2 recover go"]);

    const afterMine = extractSurfaceMiningPrepActions({
      questTitle: "Scan, Sound, and Mine",
      targetOreSlug: "carbon_ore",
      recommendedFamily: "carbon",
      hasScanStep: true,
      scanDone: true,
      hasSoundStep: true,
      soundDone: true,
      hasMineStep: true,
      mineDone: true,
      nextStepText: "Recover rig",
    });
    expect(afterMine.map((a) => a.command)).toEqual(["+tv/1 recover go"]);
  });

  it("infers corridor port from station room prefix and routes docked ships to airlock when objective is ashore", () => {
    expect(inferStationPortFromRoomEntry("room.c08_chandlery")).toBe("ceres");
    expect(inferStationPortFromRoomEntry("room.p07_receiving")).toBe("stickney");
    expect(inferStationPortFromRoomEntry("room.l01_berth_gallery")).toBe("luna");
    expect(inferStationPortFromRoomEntry("room.d02_concourse")).toBe("deimos");
    expect(inferStationPortFromRoomEntry("room.t04_deck")).toBe("titan");
    expect(inferStationPortFromRoomEntry("room.welcome_hall")).toBeNull();

    const store = new QuestsStore();
    store.applyFrame({
      schema: 1,
      active: [
        {
          key: "tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery",
          title: "Metals Down the Corridor",
          next_room_entry: "room.p07_receiving",
          checkpoints: [
            {
              id: "deliver_metals",
              title: "Deliver 5t metals to Stickney Yards",
              instructions: "Check +ship/plan stickney and sell at Receiving.",
              room_entry: "room.p07_receiving",
              checked: false,
              live_ready: false,
            },
          ],
        },
      ],
    });
    // Still docked at Ceres -> bridge to fly to Stickney
    expect(resolveQuestShipTargetCompartment(store.active, store.trackedKey, "ceres")).toBe("bridge");
    // Docked at Stickney -> airlock to cycle ashore and sell!
    expect(resolveQuestShipTargetCompartment(store.active, store.trackedKey, "stickney")).toBe("airlock");
  });

  it("evaluates Sailing Plan pre-flight readiness for propellant, provisions, and required quest cargo", () => {
    const dest = { questTitle: "Metals Down the Corridor", portId: "stickney", portLabel: "Stickney Yards" };
    const need = {
      questTitle: "Metals Down the Corridor",
      checkpointTitle: "Deliver 5t metals",
      commoditySlug: "metals",
      targetQuantity: 5,
      wantsBuy: true,
      wantsSell: true,
    };
    const missing = evaluateSailingPreflightReadiness(dest, need, {
      propellantKg: 10_000,
      provisionsCrewDays: 0,
      cargoLines: [{ good: "tacsim:metals", label: "Metals", qty: 3 }],
    });
    expect(missing).toEqual({
      propellantTonnes: 10,
      propellantOk: true,
      provisionsDays: 0,
      provisionsOk: false,
      cargoRequired: true,
      cargoLabel: "metals",
      cargoQty: 3,
      cargoTarget: 5,
      cargoOk: false,
      ready: false,
    });

    const ready = evaluateSailingPreflightReadiness(dest, need, {
      propellantKg: 15_000,
      provisionsCrewDays: 12,
      cargoLines: [{ good: "tacsim:metals", label: "Metals", qty: 5 }],
    });
    expect(ready?.ready).toBe(true);
    expect(ready?.cargoOk).toBe(true);

    // Non-cargo destination (e.g. return haul without trade cargo need) checks propellant & stores only
    const noCargo = evaluateSailingPreflightReadiness(dest, null, {
      propellantKg: 8_000,
      provisionsCrewDays: 6,
      cargoLines: [],
    });
    expect(noCargo?.cargoRequired).toBe(false);
    expect(noCargo?.ready).toBe(true);
  });

  it("detects when a cross-port buy+deliver quest requires local Exchange procurement before boarding", () => {
    const store = new QuestsStore();
    store.applyFrame({
      schema: 1,
      active: [
        {
          key: "tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery",
          title: "Metals Down the Corridor",
          next_room_entry: "room.p07_receiving",
          checkpoints: [
            {
              id: "deliver_metals",
              title: "Buy 5 tonnes of metals at Ceres and deliver them to Stickney Yards",
              instructions: "Buy metals at Ceres Exchange Hall, then fly to Stickney and +market/sell.",
              room_entry: "room.p07_receiving",
              checked: false,
              live_ready: false,
            },
          ],
        },
      ],
    });
    // Hold lacks 5t metals -> must visit local Exchange first
    expect(
      needsOriginPortProcurement(store.active, store.trackedKey, {
        propellantKg: 10_000,
        provisionsCrewDays: 10,
        cargoLines: [{ good: "tacsim:metals", label: "Metals", qty: 2 }],
      }),
    ).toBe(true);
    // Hold has 5t metals -> ready to head to Berth Gallery & board ship
    expect(
      needsOriginPortProcurement(store.active, store.trackedKey, {
        propellantKg: 10_000,
        provisionsCrewDays: 10,
        cargoLines: [{ good: "tacsim:metals", label: "Metals", qty: 5 }],
      }),
    ).toBe(false);
    // Unloaded stowage -> false (preserves default cross-port berth routing)
    expect(needsOriginPortProcurement(store.active, store.trackedKey, null)).toBe(false);

    // Sell-only quest (Step 7 carbon_ore) does NOT redirect to origin Exchange
    store.applyFrame({
      schema: 1,
      active: [
        {
          key: "tacsim.quests.guided_onboarding/quest.guide_07_sell_mined_ore",
          title: "Deliver Your Mined Ore to Market",
          next_room_entry: "room.p11_service",
          checkpoints: [
            {
              id: "sell_carbon_ore",
              title: "Sell at least 1 tonne of carbon ore at Stickney Yards",
              instructions: "Fly to Stickney Yards and +market/sell.",
              room_entry: "room.p11_service",
              checked: false,
              live_ready: false,
            },
          ],
        },
      ],
    });
    expect(
      needsOriginPortProcurement(store.active, store.trackedKey, {
        propellantKg: 10_000,
        provisionsCrewDays: 10,
        cargoLines: [],
      }),
    ).toBe(false);
  });

  it("routes Step 6 surface expeditions to Berth Gallery ashore, Bridge while docked at station, Bay on settled pad, Bridge post-mine, and Airlock back at Ceres", () => {
    const store = new QuestsStore();
    store.applyFrame({
      schema: 1,
      active: [
        {
          key: "tacsim.quests.guided_onboarding/quest.guide_06_surface_mining",
          title: "Scan, Sound, and Mine a Landing Site",
          next_room_entry: "room.c07_exchange",
          checkpoints: [
            { id: "scan_landing_site", title: "Surface scan", instructions: "+tv/1 scan go", room_entry: "room.c07_exchange", checked: false, live_ready: false },
            { id: "mine_carbon_ore", title: "Mine 1t carbon_ore", instructions: "+tv/1 mine go", room_entry: "room.c07_exchange", checked: false, live_ready: false },
          ],
        },
      ],
    });
    // Ashore -> must board ship for expedition
    expect(needsSurfaceExpeditionBoarding(store.active, store.trackedKey)).toBe(true);
    // Aboard Dray-P still docked at Ceres station -> bridge (fly to surface site first!)
    expect(resolveQuestShipTargetCompartment(store.active, store.trackedKey, "ceres", false)).toBe("bridge");
    // Aboard Dray-P settled on a surface landing pad -> bay (launch & operate rig!)
    expect(resolveQuestShipTargetCompartment(store.active, store.trackedKey, "ceres", true)).toBe("bay");

    // Once carbon_ore is mined (mineDone = true)
    store.applyFrame({
      schema: 1,
      active: [
        {
          key: "tacsim.quests.guided_onboarding/quest.guide_06_surface_mining",
          title: "Scan, Sound, and Mine a Landing Site",
          next_room_entry: "room.c07_exchange",
          checkpoints: [
            { id: "scan_landing_site", title: "Surface scan", instructions: "+tv/1 scan go", room_entry: "room.c07_exchange", checked: true, live_ready: true },
            { id: "mine_carbon_ore", title: "Mine 1t carbon_ore", instructions: "+tv/1 mine go", room_entry: "room.c07_exchange", checked: true, live_ready: true },
          ],
        },
      ],
    });
    expect(needsSurfaceExpeditionBoarding(store.active, store.trackedKey)).toBe(false);
    // Still on surface pad -> bridge (lift off & fly back to Ceres!)
    expect(resolveQuestShipTargetCompartment(store.active, store.trackedKey, null, true)).toBe("bridge");
    // Docked back at Ceres Freeport -> airlock (+ship/leave to turn in to Rosalind!)
    expect(resolveQuestShipTargetCompartment(store.active, store.trackedKey, "ceres", false)).toBe("airlock");
    // Turn-in port destination fallback guides Sailing Plan / Solar Orrery back to Ceres Freeport until standing with Rosalind
    expect(store.activeDestinationPort).toEqual({
      questTitle: "Scan, Sound, and Mine a Landing Site",
      portId: "ceres",
      portLabel: "Ceres Freeport",
    });

    // Once standing in Rosalind's room (turn_in_here: true) -> no corridor sailing destination banner
    store.applyFrame({
      schema: 1,
      active: [
        {
          key: "tacsim.quests.guided_onboarding/quest.guide_06_surface_mining",
          title: "Scan, Sound, and Mine a Landing Site",
          next_room_entry: "room.c07_exchange",
          turn_in_here: true,
          ready_to_finish: true,
          can_finish_now: true,
          checkpoints: [
            { id: "scan_landing_site", title: "Surface scan", instructions: "+tv/1 scan go", room_entry: "room.c07_exchange", checked: true, live_ready: true },
            { id: "mine_carbon_ore", title: "Mine 1t carbon_ore", instructions: "+tv/1 mine go", room_entry: "room.c07_exchange", checked: true, live_ready: true },
          ],
        },
      ],
    });
    expect(store.activeDestinationPort).toBeNull();
  });

  it("builds a 7-step Guided Corridor Tour roadmap with completed, active, available, and upcoming states", () => {
    const store = new QuestsStore();
    store.applyFrame({
      schema: 1,
      completed: [
        { key: "tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll", title: "Step 1" },
        { key: "tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel", title: "Step 2" },
      ],
      active: [
        {
          key: "tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery",
          title: "Metals Down the Corridor",
          ready_to_finish: true,
          checkpoints: [],
        },
      ],
      available: [
        {
          key: "tacsim.quests.guided_onboarding/quest.guide_04_return_haul",
          title: "A Return Load for Ceres",
          checkpoints: [],
        },
      ],
    });
    const roadmap = buildGuidedTourRoadmap(store.active, store.available, store.completed);
    expect(roadmap).toHaveLength(7);
    expect(roadmap[0]).toMatchObject({ step: 1, shortLabel: "Co-op Charter", status: "completed", readyToTurnIn: false });
    expect(roadmap[1]).toMatchObject({ step: 2, shortLabel: "Locker & Tanks", status: "completed" });
    expect(roadmap[2]).toMatchObject({ step: 3, shortLabel: "Metals Haul", status: "active", readyToTurnIn: true });
    expect(roadmap[3]).toMatchObject({ step: 4, shortLabel: "Return Load", status: "available" });
    expect(roadmap[4]).toMatchObject({ step: 5, shortLabel: "Dray-P Refit", status: "upcoming" });
    expect(roadmap[5]).toMatchObject({ step: 6, shortLabel: "Surface Mining", status: "upcoming" });
    expect(roadmap[6]).toMatchObject({ step: 7, shortLabel: "Sell Mined Ore", status: "upcoming" });
    expect(store.guidedTourRoadmap).toEqual(roadmap);
    // Active/available present -> nextGuidedOffer is null
    expect(store.nextGuidedOffer).toBeNull();

    // Between steps (Steps 1–3 complete, no active or room offers) -> points straight to Step 4 Jonas at Stickney Receiving
    store.applyFrame({
      schema: 1,
      completed: [
        { key: "tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll", title: "Step 1" },
        { key: "tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel", title: "Step 2" },
        { key: "tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery", title: "Step 3" },
      ],
      active: [],
      available: [],
    });
    expect(store.nextGuidedOffer).toMatchObject({
      step: 4,
      shortLabel: "Return Load",
      giverLabel: "Jonas Achterberg-Nkemelu",
      giverRoomEntry: "room.p07_receiving",
      giverRoomName: "Receiving Counter",
    });

    // Fresh character (0 completed) -> null so non-guided worlds stay quiet
    store.applyFrame({ schema: 1, completed: [], active: [], available: [] });
    expect(store.nextGuidedOffer).toBeNull();
  });
});
