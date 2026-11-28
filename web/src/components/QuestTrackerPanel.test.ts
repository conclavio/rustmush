// @vitest-environment jsdom

import { fireEvent, render, screen } from "@testing-library/svelte";
import { describe, expect, it, vi } from "vitest";

import QuestTrackerPanel from "./QuestTrackerPanel.svelte";
import type { GameApp } from "$lib/game.svelte";
import { GUIDED_TOUR_KEYS, QuestsStore } from "$lib/stores/quests.svelte";
import { buildFallbackDray, buildFallbackFreeport } from "$lib/tacsim/spatial";

describe("QuestTrackerPanel route step, guided tour pill, and quick-accept", () => {
  it("shows Tour progress pill and 1-click Walk route button along shortest station path", async () => {
    const quests = new QuestsStore();
    const pack = buildFallbackFreeport();
    const rooms = [...pack.rooms.keys()];
    const startEntry = rooms[0]!;
    const destEntry = rooms[rooms.length - 1]!;

    quests.applyFrame({
      schema: 1,
      room_id: "#10",
      room_name: pack.rooms.get(startEntry)!.name,
      room_markers: [],
      available: [],
      completed: [
        { key: GUIDED_TOUR_KEYS[0], title: "First Papers" },
      ],
      active: [
        {
          key: GUIDED_TOUR_KEYS[1],
          title: "Stock the Locker and Tanks",
          next_step: "Walk to Exchange to buy provisions",
          next_room_entry: destEntry,
          next_room_name: pack.rooms.get(destEntry)!.name,
          checkpoints: [
            {
              id: "buy_provisions",
              title: "Buy 10 provisions",
              room_entry: destEntry,
              room_name: pack.rooms.get(destEntry)!.name,
              checked: false,
              live_ready: false,
            },
          ],
        },
      ],
    });

    const onCmd = vi.fn();
    const app = {
      quests,
      tacsim: {
        isAboardShip: false,
        view3dOpen: false,
        physicalLocationKey: null,
        serverStationRoomEntry: startEntry,
        stationPack: pack,
      },
      focusComposer: vi.fn(),
    } as unknown as GameApp;

    render(QuestTrackerPanel, { props: { app, onCmd } });

    expect(screen.getByRole("button", { name: "Tour 1/7" })).toBeTruthy();
    const walkBtn = screen.getByRole("button", { name: /^Walk → / });
    await fireEvent.click(walkBtn);
    expect(onCmd).toHaveBeenCalledTimes(1);
    expect(String(onCmd.mock.calls[0]![0]).toLowerCase().startsWith("goto ")).toBe(true);
  });

  it("offers 1-click Accept Quest alongside Open Quest Dialog for room offers", async () => {
    const quests = new QuestsStore();
    quests.applyFrame({
      schema: 1,
      room_id: "#10",
      room_name: "Port Watch Arrivals",
      room_markers: [],
      active: [],
      completed: [],
      available: [
        {
          key: GUIDED_TOUR_KEYS[0],
          title: "First Papers",
          giver_id: "#99",
          giver_name: "Maren Halvorsen",
          accept_cmd: "+quest/accept #99 tok123",
          checkpoints: [],
        },
      ],
    });

    const onCmd = vi.fn();
    const app = { quests, tacsim: null } as unknown as GameApp;
    render(QuestTrackerPanel, { props: { app, onCmd } });

    expect(screen.getByText("Tour 0/7")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Accept Quest" }));
    expect(onCmd).toHaveBeenCalledWith("+quest/accept #99 tok123");
  });

  it("renders Corridor Captain mastery strip once all 7 guided quests are complete", async () => {
    const quests = new QuestsStore();
    quests.applyFrame({
      schema: 1,
      room_id: "#22",
      room_name: "Service Counter",
      room_markers: [],
      active: [],
      available: [],
      completed: GUIDED_TOUR_KEYS.map((key, i) => ({ key, title: `Step ${i + 1}` })),
    });

    const app = { quests, tacsim: null } as unknown as GameApp;
    render(QuestTrackerPanel, { props: { app, onCmd: vi.fn() } });

    const section = screen.getByLabelText("Guided tour complete");
    expect(section.textContent).toContain("Corridor Captain");
    expect(section.textContent).toContain("(7/7)");
    await fireEvent.click(screen.getByRole("button", { name: "Completed (7)" }));
    expect(quests.logOpen).toBe(true);
    expect(quests.logTab).toBe("completed");
  });

  it("guides aboard-ship players from airlock to bridge for flight quests and hold to vehicle bay for mining", async () => {
    const quests = new QuestsStore();
    const drayP = buildFallbackDray("tacsim.unit.dray_p");
    if (!drayP.compartments.has("bay")) {
      drayP.compartments.set("bay", {
        id: "bay",
        name: "Vehicle Bay",
        shortDesc: "Two-rig bay",
        desc: "",
        localCenter: { x: 0, y: -3.6, z: -20 },
        localSize: { x: 9, y: 3.6, z: 11 },
        details: {},
      });
      drayP.exits.push({
        from: "hold",
        to: "bay",
        forwardLabel: "Vehicle Bay",
        forwardCommand: "+tv/bay in",
        reverseLabel: "Cargo Hold",
        reverseCommand: "+tv/bay out",
        isStaticAction: true,
      });
    }

    // 1. Flight destination quest aboard ship in airlock -> guides toward Bridge (2 hops via Passage)
    quests.applyFrame({
      schema: 1,
      room_id: "#50",
      room_name: "M/V Starling - Airlock",
      active: [
        {
          key: GUIDED_TOUR_KEYS[2],
          title: "Metals Down the Corridor",
          next_step: "Sail to Stickney Yards (+ship/plan stickney)",
          checkpoints: [
            {
              id: "sail_stickney",
              title: "Sail to Stickney Yards",
              instructions: "Run +ship/plan stickney and depart.",
              checked: false,
              live_ready: false,
            },
          ],
        },
      ],
    });
    const onCmd = vi.fn();
    const appFlight = {
      quests,
      tacsim: {
        isAboardShip: true,
        seatedAtBridgeHelm: false,
        view3dOpen: false,
        physicalLocationKey: null,
        serverShipCompartment: "airlock",
        shipTemplate: drayP,
      },
      focusComposer: vi.fn(),
    } as unknown as GameApp;

    const first = render(QuestTrackerPanel, { props: { app: appFlight, onCmd } });
    const bridgeWalk = screen.getByRole("button", { name: /^Walk → / });
    await fireEvent.click(bridgeWalk);
    expect(onCmd).toHaveBeenCalledTimes(1);
    expect(String(onCmd.mock.calls[0]![0]).toLowerCase().startsWith("goto ")).toBe(true);
    first.unmount();

    // 2. Surface mining quest aboard Dray-P in cargo hold -> guides directly into Vehicle Bay (+tv/bay in)
    quests.applyFrame({
      schema: 1,
      room_id: "#54",
      room_name: "M/V Starling - Cargo Hold",
      active: [
        {
          key: GUIDED_TOUR_KEYS[5],
          title: "Scan and Mine Carbon Ore",
          next_step: "Launch rig and run +tv/1 scan go",
          checkpoints: [
            {
              id: "scan_site",
              title: "Run surface scan",
              instructions: "Run +tv/1 scan go.",
              checked: false,
              live_ready: false,
            },
            {
              id: "mine_carbon_ore",
              title: "Mine 1t carbon ore",
              instructions: "Run +tv/1 mine go.",
              checked: false,
              live_ready: false,
            },
          ],
        },
      ],
    });
    const onCmdMine = vi.fn();
    const appMine = {
      quests,
      tacsim: {
        isAboardShip: true,
        seatedAtBridgeHelm: false,
        view3dOpen: false,
        physicalLocationKey: null,
        serverShipCompartment: "hold",
        shipTemplate: drayP,
      },
      focusComposer: vi.fn(),
    } as unknown as GameApp;

    render(QuestTrackerPanel, { props: { app: appMine, onCmd: onCmdMine } });
    const bayBtn = screen.getByRole("button", { name: /Walk → Vehicle Bay \(1h\)/i });
    await fireEvent.click(bayBtn);
    expect(onCmdMine).toHaveBeenCalledWith("+tv/bay in");
  });

  it("guides cross-port quests to Berth Gallery (+ship/board) and docked ships to Airlock (+ship/leave)", async () => {
    const quests = new QuestsStore();
    const ceres = buildFallbackFreeport();
    const dray = buildFallbackDray("tacsim.unit.dray");
    const berthEntry = [...ceres.rooms.keys()].find((k) => k.includes("berth_gallery"))!;
    const otherEntry = [...ceres.rooms.keys()].find((k) => k !== berthEntry)!;

    quests.applyFrame({
      schema: 1,
      room_id: "#15",
      room_name: ceres.rooms.get(otherEntry)!.name,
      active: [
        {
          key: GUIDED_TOUR_KEYS[2],
          title: "Metals Down the Corridor",
          next_step: "Sell 5t metals at Stickney Yards",
          next_room_entry: "room.p07_receiving",
          next_room_name: "Receiving & Customs",
          checkpoints: [
            {
              id: "deliver_metals",
              title: "Deliver 5t metals to Stickney Yards",
              instructions: "Fly to Stickney (+ship/plan stickney) and sell metals.",
              room_entry: "room.p07_receiving",
              room_name: "Receiving & Customs",
              checked: false,
              live_ready: false,
            },
          ],
        },
      ],
    });

    // 1. Ashore on Ceres outside Berth Gallery -> walks toward Berth Gallery
    const onCmdWalk = vi.fn();
    const r1 = render(QuestTrackerPanel, {
      props: {
        app: {
          quests,
          tacsim: {
            isAboardShip: false,
            view3dOpen: false,
            physicalLocationKey: null,
            serverStationRoomEntry: otherEntry,
            stationPack: ceres,
          },
          focusComposer: vi.fn(),
        } as unknown as GameApp,
        onCmd: onCmdWalk,
      },
    });
    const walkToBerth = screen.getByRole("button", { name: /^Walk → / });
    await fireEvent.click(walkToBerth);
    expect(String(onCmdWalk.mock.calls[0]![0]).toLowerCase().startsWith("goto ")).toBe(true);
    r1.unmount();

    // 2. Ashore at Ceres Berth Gallery -> 1-click Board Ship (+ship/board)
    const onCmdBoard = vi.fn();
    const r2 = render(QuestTrackerPanel, {
      props: {
        app: {
          quests,
          tacsim: {
            isAboardShip: false,
            view3dOpen: false,
            physicalLocationKey: null,
            serverStationRoomEntry: berthEntry,
            stationPack: ceres,
          },
          focusComposer: vi.fn(),
        } as unknown as GameApp,
        onCmd: onCmdBoard,
      },
    });
    await fireEvent.click(screen.getByRole("button", { name: /Board Ship/i }));
    expect(onCmdBoard).toHaveBeenCalledWith("+ship/board");
    r2.unmount();

    // 3. Aboard ship docked at Stickney on Bridge -> routes toward Airlock
    const onCmdToLock = vi.fn();
    const r3 = render(QuestTrackerPanel, {
      props: {
        app: {
          quests,
          tacsim: {
            isAboardShip: true,
            seatedAtBridgeHelm: false,
            view3dOpen: false,
            physicalLocationKey: null,
            serverShipCompartment: "bridge",
            shipTemplate: dray,
            ambient: { docked: true, port: "stickney" },
          },
          focusComposer: vi.fn(),
        } as unknown as GameApp,
        onCmd: onCmdToLock,
      },
    });
    const walkLock = screen.getByRole("button", { name: /^Walk → / });
    await fireEvent.click(walkLock);
    expect(String(onCmdToLock.mock.calls[0]![0]).toLowerCase().startsWith("goto ")).toBe(true);
    r3.unmount();

    // 4. Aboard ship docked at Stickney in Airlock -> 1-click Cycle Ashore (+ship/leave)
    const onCmdLeave = vi.fn();
    render(QuestTrackerPanel, {
      props: {
        app: {
          quests,
          tacsim: {
            isAboardShip: true,
            seatedAtBridgeHelm: false,
            view3dOpen: false,
            physicalLocationKey: null,
            serverShipCompartment: "airlock",
            shipTemplate: dray,
            ambient: { docked: true, port: "stickney" },
          },
          focusComposer: vi.fn(),
        } as unknown as GameApp,
        onCmd: onCmdLeave,
      },
    });
    await fireEvent.click(screen.getByRole("button", { name: /Cycle Ashore/i }));
    expect(onCmdLeave).toHaveBeenCalledWith("+ship/leave");
  });
});
