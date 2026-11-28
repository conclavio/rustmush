// @vitest-environment jsdom
//
// The two strips around the scene, mounted (doc 43 cut TS.19): the room band
// and the vitals strip. They replace the source probes `lib/roomband.test.ts`
// and `stores/unit.test.ts` carried for the halves a node test cannot render.

import { fireEvent, render, screen } from "@testing-library/svelte";
import { describe, expect, it, vi } from "vitest";

import ChannelTabs from "./ChannelTabs.svelte";
import ChannelView from "./ChannelView.svelte";
import RoomBand from "./RoomBand.svelte";
import VitalsStrip from "./VitalsStrip.svelte";
import type { GameApp } from "$lib/game.svelte";
import type { RoomView } from "$lib/proto";
import type { ChannelMeta } from "$lib/stores/channels.svelte";
import { buildFallbackDray, buildFallbackFreeport } from "$lib/tacsim/spatial";

const HALL: RoomView = {
  name: "The Hall",
  id: "#2",
  desc: [],
  contents: [],
  exits: [],
  zone: null,
};

function mountBand(roomView: RoomView): void {
  const app = {
    roomView,
    screens: { exits: new Map() },
    presence: { prune: vi.fn(), thinking: [], composing: [] },
  } as unknown as GameApp;
  render(RoomBand, { props: { app, onCmd: vi.fn() } });
}

describe("the room band's weather chip (I4-C20)", () => {
  it("shows the module's prose when it sent any", () => {
    mountBand({
      ...HALL,
      ambient: { weather: { state: "rain", icon: "rain", line: "Overhead: rain." } },
    });
    expect(screen.getByText(/Overhead: rain\./)).toBeTruthy();
  });

  it("falls back to the bare state, never an empty chip", () => {
    mountBand({ ...HALL, ambient: { weather: { state: "rain" } } });
    const chip = screen.getByTitle("rain");
    expect(chip.textContent?.trim()).toContain("rain");
  });
});

describe("the vitals strip", () => {
  it("names its subject visibly, not only as a tooltip", () => {
    // One feed can follow a viewer from subject to subject; a strip that
    // repoints without saying so reads as the old subject's gauges.
    const app = {
      vitals: {
        list: [
          {
            feed: "wod.vitals",
            frame: { subject: { id: "#9", name: "Mara" }, revision: 1, vitals: [] },
          },
        ],
      },
    } as unknown as GameApp;
    render(VitalsStrip, { props: { app } });
    const name = screen.getByText("Mara");
    expect(name.closest("[title]")?.getAttribute("title")).toBe("Mara");
    expect(name.hasAttribute("title")).toBe(false);
  });
});

describe("room band exit buttons submit safe goto commands", () => {
  it("prefixes goto on click and refuses control-character exit names", async () => {
    const onCmd = vi.fn();
    const roomView: RoomView = {
      ...HALL,
      exits: [
        { name: "North", aliases: ["n"] },
        { name: "@destroy me", aliases: [] },
        { name: "Bad\nQUIT", aliases: [] },
      ],
    };
    const app = {
      roomView,
      screens: { exits: new Map([["n", "North"]]) },
      presence: { prune: vi.fn(), thinking: [], composing: [] },
    } as unknown as GameApp;
    render(RoomBand, { props: { app, onCmd } });

    await fireEvent.click(screen.getByRole("button", { name: /North/ }));
    await fireEvent.click(screen.getByRole("button", { name: "@destroy me" }));
    await fireEvent.click(screen.getByRole("button", { name: /Bad/ }));

    expect(onCmd.mock.calls).toEqual([["goto North"], ["goto @destroy me"]]);
  });

  it("P2-CLIENT-7: renders duplicate room exit names without each_key_duplicate crash", async () => {
    const onCmd = vi.fn();
    const roomView: RoomView = {
      ...HALL,
      exits: [
        { name: "North", aliases: ["n"] },
        { name: "North", aliases: [] },
        { name: "North", aliases: [] },
      ],
    };
    const app = {
      roomView,
      screens: { exits: new Map([["n", "North"]]) },
      presence: { prune: vi.fn(), thinking: [], composing: [] },
    } as unknown as GameApp;

    render(RoomBand, { props: { app, onCmd } });
    const buttons = screen.getAllByRole("button", { name: /North/ });
    expect(buttons).toHaveLength(3);
    await fireEvent.click(buttons[1]!);
    expect(onCmd).toHaveBeenCalledWith("goto North");
  });

  it("highlights the exact next exit along the tracked quest shortest path", () => {
    const pack = buildFallbackFreeport();
    const rooms = [...pack.rooms.keys()];
    const startEntry = rooms[0]!;
    const targetEntry = rooms[rooms.length - 1]!;
    const firstOut = pack.exits.find((ex) => ex.fromRoom === startEntry || ex.toRoom === startEntry);
    expect(firstOut).toBeDefined();
    const onPathLabel =
      firstOut!.fromRoom === startEntry ? firstOut!.forwardLabel : firstOut!.reverseLabel;
    const onPathCmd =
      firstOut!.fromRoom === startEntry ? firstOut!.forwardCommand : firstOut!.reverseCommand;

    const roomView: RoomView = {
      ...HALL,
      name: pack.rooms.get(startEntry)!.name,
      exits: [
        { name: onPathLabel, aliases: [onPathCmd] },
        { name: "Side Closet", aliases: ["closet"] },
      ],
    };
    const app = {
      roomView,
      screens: { exits: new Map() },
      presence: { prune: vi.fn(), thinking: [], composing: [] },
      quests: {
        roomMarkers: [],
        trackedKey: "q.walk",
        active: [
          {
            key: "q.walk",
            title: "Walk to Objective",
            nextRoomEntry: targetEntry,
            checkpoints: [],
          },
        ],
      },
      tacsim: {
        isAboardShip: false,
        view3dOpen: false,
        physicalLocationKey: null,
        serverStationRoomEntry: startEntry,
        stationPack: pack,
      },
    } as unknown as GameApp;

    const { container } = render(RoomBand, { props: { app, onCmd: vi.fn() } });
    const highlighted = Array.from(container.querySelectorAll("button.exit.quest-exit"));
    expect(highlighted.length).toBeGreaterThanOrEqual(1);
    expect(highlighted[0]!.getAttribute("data-quest-exit")).toBe("true");
    expect(highlighted[0]!.getAttribute("title")).toContain("Quest route");
    const sideBtn = screen.getByRole("button", { name: "Side Closet" });
    expect(sideBtn.getAttribute("data-quest-exit")).toBeNull();
  });

  it("highlights the aboard-ship hatch leading to the Bridge when a destination quest is active", () => {
    const dray = buildFallbackDray("tacsim.unit.dray");
    const airlockEx = dray.exits.find((e) => e.from === "airlock" || e.to === "airlock");
    expect(airlockEx).toBeDefined();
    const hatchLabel =
      airlockEx!.from === "airlock" ? airlockEx!.forwardLabel : airlockEx!.reverseLabel;
    const hatchCmd =
      airlockEx!.from === "airlock" ? airlockEx!.forwardCommand : airlockEx!.reverseCommand;

    const roomView: RoomView = {
      ...HALL,
      name: "M/V Starling - Airlock",
      exits: [
        { name: hatchLabel, aliases: [hatchCmd] },
        { name: "Cycle Ashore", aliases: ["out"] },
      ],
    };
    const app = {
      roomView,
      screens: { exits: new Map() },
      presence: { prune: vi.fn(), thinking: [], composing: [] },
      quests: {
        roomMarkers: [],
        trackedKey: "q.stickney",
        active: [
          {
            key: "q.stickney",
            title: "Run Metals to Stickney Yards",
            checkpoints: [
              {
                id: "sail_stickney",
                title: "Sail to Stickney Yards",
                instructions: "Run +ship/plan stickney",
                checked: false,
                liveReady: false,
              },
            ],
          },
        ],
      },
      tacsim: {
        isAboardShip: true,
        seatedAtBridgeHelm: false,
        view3dOpen: false,
        physicalLocationKey: null,
        serverShipCompartment: "airlock",
        shipTemplate: dray,
      },
    } as unknown as GameApp;

    const { container } = render(RoomBand, { props: { app, onCmd: vi.fn() } });
    const highlighted = Array.from(container.querySelectorAll("button.exit.quest-exit"));
    expect(highlighted).toHaveLength(1);
    expect(highlighted[0]!.getAttribute("title")).toContain("Quest route");
    expect(screen.getByRole("button", { name: "Cycle Ashore" }).getAttribute("data-quest-exit")).toBeNull();
  });

  it("prioritizes room NPC quest markers (turn-in -> offer -> in-progress) with role cues and opens NPC dialog", async () => {
    const openNpcDialog = vi.fn();
    const app = {
      roomView: HALL,
      screens: { exits: new Map() },
      presence: { prune: vi.fn(), thinking: [], composing: [] },
      quests: {
        roomMarkers: [
          {
            npcId: "#31",
            npcName: "Silver Clerk",
            marker: "in_progress",
            symbol: "?",
            questKey: "q.wip",
            questTitle: "In Progress Job",
            category: "optional",
            questCount: 1,
          },
          {
            npcId: "#11",
            npcName: "Maren Halvorsen",
            marker: "available",
            symbol: "!",
            questKey: "q.offer",
            questTitle: "First Papers",
            category: "guided",
            questCount: 2,
          },
          {
            npcId: "#22",
            npcName: "Ilse Oduya-Brandt",
            marker: "turn_in",
            symbol: "?",
            questKey: "q.turnin",
            questTitle: "Stock the Locker",
            category: "guided",
            questCount: 1,
          },
        ],
        active: [],
        trackedKey: null,
        openNpcDialog,
      },
    } as unknown as GameApp;

    const { container } = render(RoomBand, { props: { app, onCmd: vi.fn() } });
    const btns = Array.from(container.querySelectorAll("button.qmarker"));
    expect(btns.map((b) => b.getAttribute("data-marker"))).toEqual([
      "turn_in",
      "available",
      "in_progress",
    ]);
    expect(btns[0]!.textContent).toContain("Ilse Oduya-Brandt");
    expect(btns[0]!.textContent).toContain("Turn In");
    expect(btns[1]!.textContent).toContain("Maren Halvorsen");
    expect(btns[1]!.textContent).toContain("Offer");
    expect(btns[1]!.textContent).toContain("×2");

    await fireEvent.click(btns[0]!);
    expect(openNpcDialog).toHaveBeenCalledWith("#22");
  });
});

describe("channel colorHex inline style sanitization", () => {
  function makeChan(name: string, colorHex: string | null): ChannelMeta {
    return {
      name,
      alias: name.toLowerCase(),
      title: null,
      gagged: false,
      colorHex,
      seq: 0,
      readSeq: 0,
      loud: false,
      logged: false,
      ic: false,
      epoch: null,
    };
  }

  it("ChannelTabs allows valid hex colors and strips CSS injection payloads", () => {
    const valid = makeChan("Public", "#d1a458");
    const shortHex = makeChan("Short", "#0fa");
    const hostile = makeChan("Evil", "#ff0; background-image: url(https://evil.example)");
    const app = {
      channels: {
        active: null,
        totalUnread: 0,
        channelsWhere: (ic: boolean) => (ic ? [] : [valid, shortHex, hostile]),
        unreadOf: () => 0,
        activate: vi.fn(),
      },
      openChannelTab: vi.fn(),
    } as unknown as GameApp;

    const { container } = render(ChannelTabs, { props: { app } });
    const chips = Array.from(container.querySelectorAll(".chip"));
    expect(chips).toHaveLength(3);
    expect(chips[0]!.getAttribute("style")).toContain("background");
    expect(chips[1]!.getAttribute("style")).toContain("background");
    expect(chips[2]!.getAttribute("style") ?? "").toBe("");
  });

  it("ChannelView allows valid hex colors and strips CSS injection payloads", () => {
    const hostile = makeChan("Evil", "red; position:fixed");
    const app = {
      channels: {
        byName: () => hostile,
        messagesOf: () => [],
      },
      loadOlderChannel: vi.fn(),
      channelSend: vi.fn(),
    } as unknown as GameApp;

    const { container, unmount } = render(ChannelView, {
      props: { app, channel: "Evil", onCmd: vi.fn() },
    });
    const heading = container.querySelector(".head strong");
    expect(heading?.getAttribute("style") ?? "").toBe("");
    unmount();

    const safe = makeChan("Safe", "#8cc265");
    const appSafe = {
      channels: {
        byName: () => safe,
        messagesOf: () => [],
      },
      loadOlderChannel: vi.fn(),
      channelSend: vi.fn(),
    } as unknown as GameApp;
    const res = render(ChannelView, {
      props: { app: appSafe, channel: "Safe", onCmd: vi.fn() },
    });
    expect(res.container.querySelector(".head strong")?.getAttribute("style")).toContain("color");
  });
});
