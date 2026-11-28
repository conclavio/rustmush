// @vitest-environment jsdom

import { fireEvent, render, screen } from "@testing-library/svelte";
import { describe, expect, it, vi } from "vitest";

import QuestTrackerPanel from "./QuestTrackerPanel.svelte";
import type { GameApp } from "$lib/game.svelte";
import { GUIDED_TOUR_KEYS, QuestsStore } from "$lib/stores/quests.svelte";
import { buildFallbackFreeport } from "$lib/tacsim/spatial";

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
});
