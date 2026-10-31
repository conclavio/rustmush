import { describe, expect, it } from "vitest";
import { emptyCatalog } from "./store";
import {
  applyAssistPatchToNpc,
  applyLorePoolToDoc,
  applyNpcModelToDoc,
  extractLorePools,
  extractNpcModels,
  relocateNpcToPort,
} from "./npc-editor";

const SAMPLE_CREW_DOC = {
  brief: { prose: "Sample crew pack" },
  entries: {
    "crew_for_hire.sora_voss": {
      def: {
        daily_wage_minor: 2500,
        duties: ["recovery"],
        fee_minor: 50000,
        npc: "npc.sora_voss",
        port: "ceres_port",
        schema: 1,
      },
      kind: "crew_for_hire",
    },
    "npc.sora_voss": {
      def: {
        name: "Sora Voss",
        profile: {
          background: "Freeport hull tech",
          command_refs: ["+crew"],
          fallback: ["Give me a minute on this weld."],
          goals: ["Keep the drive sealed"],
          interfaces: ["hire"],
          knows: ["Dray hull seals"],
          persona: "Steady maintenance specialist",
          speech_style: "Direct and clipped",
        },
        room: "room.c10_slack_line",
        room_pack: "neighborhood.ceres_freeport",
        tags: ["corridor_trade", "port_ceres"],
        tier: "minor",
      },
      kind: "npc",
    },
    "lore.port_ceres": {
      def: {
        entries: [
          {
            fact: "Ceres Freeport is the Hub of the Belt.",
            subject: "Ceres",
            tier: "public",
          },
        ],
        tag: "port_ceres",
      },
      kind: "lore",
    },
  },
  manifest: {
    digest: "0000000000000000000000000000000000000000000000000000000000000000",
    id: "tacsim.crew.ceres",
    kind: "campaign_seed",
    requires: [{ id: "neighborhood.ceres_freeport", version: 4 }],
    version: 1,
  },
};

describe("npc-editor (Cut CS.1)", () => {
  it("extracts paired crew NPCs and lore pools from a pack document", () => {
    const npcs = extractNpcModels(SAMPLE_CREW_DOC);
    expect(npcs).toHaveLength(1);
    expect(npcs[0]?.name).toBe("Sora Voss");
    expect(npcs[0]?.role).toBe("crew");
    expect(npcs[0]?.crewTerms?.schema).toBe(1);
    expect(npcs[0]?.crewTerms?.duties).toEqual(["recovery"]);

    const pools = extractLorePools(SAMPLE_CREW_DOC);
    expect(pools).toHaveLength(1);
    expect(pools[0]?.tag).toBe("port_ceres");
  });

  it("relocates an NPC from Ceres to Stickney and updates manifest.requires on apply", () => {
    const catalog = emptyCatalog();
    catalog.ports = [
      {
        id: "ceres_port",
        venue: "Ceres Freeport",
        location: "ceres_freeport",
        area: "neighborhood.ceres_freeport",
        rooms: { hiring: "room.c10_slack_line" },
        orgs: [],
        vendors: [],
      },
      {
        id: "stickney_port",
        venue: "Stickney Yards",
        location: "stickney_yards",
        area: "neighborhood.stickney_yards",
        rooms: { hiring: "room.p10_shift_board" },
        orgs: [],
        vendors: [],
      },
    ];
    catalog.stations = [
      {
        pack_id: "neighborhood.stickney_yards",
        version: 4,
        latched_version: 4,
        rooms: [
          {
            id: "room.p10_shift_board",
            name: "Shift Board",
            zone: "public",
            tags: [],
            details: [],
          },
        ],
      },
    ];

    const [npc] = extractNpcModels(SAMPLE_CREW_DOC);
    expect(npc).toBeDefined();
    const relocated = relocateNpcToPort(npc!, "stickney_port", catalog);
    expect(relocated.roomPack).toBe("neighborhood.stickney_yards");
    expect(relocated.room).toBe("room.p10_shift_board");
    expect(relocated.crewTerms?.port).toBe("stickney_port");

    const updatedDoc = applyNpcModelToDoc(SAMPLE_CREW_DOC, relocated) as Record<
      string,
      unknown
    >;
    const manifest = updatedDoc.manifest as { requires: Array<{ id: string; version: number }> };
    expect(manifest.requires).toEqual([
      { id: "neighborhood.stickney_yards", version: 4 },
    ]);
  });

  it("applies LLM assist patches for both full drafts and single-field regenerations", () => {
    const [npc] = extractNpcModels(SAMPLE_CREW_DOC);
    const regen = applyAssistPatchToNpc(npc!, "persona", {
      persona: "Updated veteran hull inspector persona.",
    });
    expect(regen.persona).toBe("Updated veteran hull inspector persona.");
    expect(regen.background).toBe("Freeport hull tech");

    const pools = extractLorePools(SAMPLE_CREW_DOC);
    const nextDoc = applyLorePoolToDoc(SAMPLE_CREW_DOC, {
      ...pools[0]!,
      entries: [
        ...pools[0]!.entries,
        {
          tier: "guarded",
          subject: "Dock 4",
          fact: "Cradle 4 runs an extra shift on Thursdays.",
          source: "Harbourmaster log",
        },
      ],
    });
    expect(extractLorePools(nextDoc)[0]?.entries).toHaveLength(2);
  });
});
