import { describe, expect, it } from "vitest";
import {
  applyAssistPatchToQuest,
  applyQuestModelToDoc,
  buildQuestDagEdges,
  extractQuestModels,
} from "./quest-editor";

const SAMPLE_QUEST_DOC = {
  brief: { prose: "Sample quest follow-up" },
  entries: {
    "quest.leila_trade": {
      def: {
        checkpoints: [
          {
            id: "fittings_trade",
            instructions: "Buy 1t of fittings at Stickney and sell at Ceres.",
            objective: {
              commodity: "industrial_parts",
              destination: "ceres",
              kind: "trade",
              origin: "stickney",
              quantity: 1,
            },
            room_entry: "room.c08_chandlery",
            room_pack: "neighborhood.ceres_freeport",
            title: "Account for the fittings trade",
          },
        ],
        choice: { group: "tacsim.leila_next_work", path: "trade" },
        completion: "The captain completed the fittings trade.",
        destination: "ceres",
        introduction: "Try a small fittings run.",
        npc: "npc.leila_mensah_kovacs",
        npc_pack: "tacsim.passengers.stickney",
        prerequisite: {
          completed: {
            quest: "tacsim.passengers.stickney/quest.leila_second_reading",
          },
        },
        title: "A useful consignment",
      },
      kind: "quest",
    },
  },
  manifest: {
    digest: "0000000000000000000000000000000000000000000000000000000000000000",
    id: "tacsim.quests.leila_followup",
    kind: "campaign_seed",
    requires: [],
    version: 1,
  },
};

describe("quest-editor (Cut CS.2)", () => {
  it("extracts quest models, choice groups, and cross-pack DAG edges", () => {
    const quests = extractQuestModels(SAMPLE_QUEST_DOC);
    expect(quests).toHaveLength(1);
    expect(quests[0]?.title).toBe("A useful consignment");
    expect(quests[0]?.choiceGroup).toBe("tacsim.leila_next_work");
    expect(quests[0]?.checkpoints[0]?.objective.kind).toBe("trade");

    const edges = buildQuestDagEdges("tacsim.quests.leila_followup", quests);
    expect(edges).toEqual([
      {
        from: "tacsim.passengers.stickney/quest.leila_second_reading",
        to: "tacsim.quests.leila_followup/quest.leila_trade",
        crossPack: true,
        choiceGroup: "tacsim.leila_next_work",
      },
    ]);
  });

  it("reconciles manifest.requires across npc_pack, room_pack, and prerequisite quests", () => {
    const [quest] = extractQuestModels(SAMPLE_QUEST_DOC);
    expect(quest).toBeDefined();
    const updatedDoc = applyQuestModelToDoc(SAMPLE_QUEST_DOC, quest!) as Record<
      string,
      unknown
    >;
    const manifest = updatedDoc.manifest as {
      requires: Array<{ id: string; version: number }>;
    };
    expect(manifest.requires).toEqual([
      { id: "neighborhood.ceres_freeport", version: 4 },
      { id: "tacsim.passengers.stickney", version: 2 },
    ]);
  });

  it("applies LLM assist patches for introduction and completion fields", () => {
    const [quest] = extractQuestModels(SAMPLE_QUEST_DOC);
    const patched = applyAssistPatchToQuest(quest!, "introduction", {
      introduction: "Updated introduction in Leila's voice.",
    });
    expect(patched.introduction).toBe("Updated introduction in Leila's voice.");
    expect(patched.completion).toBe("The captain completed the fittings trade.");
  });
});
