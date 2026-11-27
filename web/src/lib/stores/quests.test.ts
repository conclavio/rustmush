import { describe, expect, it } from "vitest";
import { QUESTS_FEED, QuestsStore, parseQuestMarkers } from "./quests.svelte";

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
});
