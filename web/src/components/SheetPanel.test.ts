// @vitest-environment jsdom

import { fireEvent, render, screen } from "@testing-library/svelte";
import { describe, expect, it, vi } from "vitest";

import SheetPanel from "./SheetPanel.svelte";
import type { GameApp } from "$lib/game.svelte";
import { GUIDED_TOUR_KEYS, QuestsStore } from "$lib/stores/quests.svelte";

describe("SheetPanel guided corridor tour summary", () => {
  it("renders Tour progress badge, clickable 7-step roadmap, and between-steps Next Guided locator", async () => {
    const quests = new QuestsStore();
    quests.applyFrame({
      schema: 1,
      completed: [
        { key: GUIDED_TOUR_KEYS[0], title: "First Papers" },
        { key: GUIDED_TOUR_KEYS[1], title: "Stock Locker" },
      ],
      active: [
        {
          key: GUIDED_TOUR_KEYS[2],
          title: "Metals Down the Corridor",
          next_step: "Buy 5t metals at Exchange",
          checkpoints: [{ id: "deliver_metals", title: "Deliver 5t metals", checked: false, live_ready: false }],
        },
      ],
      available: [],
    });

    const app = {
      sheet: {
        loaded: true,
        unavailable: false,
        approval: null,
        schema: { sections: [] },
        data: {},
      },
      quests,
      refreshSheet: vi.fn(),
    } as unknown as GameApp;

    const first = render(SheetPanel, { props: { app } });
    expect(screen.getByText(/Tour 2\/7/i)).toBeTruthy();
    const step3Btn = screen.getByRole("button", { name: /3\. Metals Haul/i });
    await fireEvent.click(step3Btn);
    expect(quests.dialogOpen).toBe(true);
    expect(quests.dialogQuestKey).toBe(GUIDED_TOUR_KEYS[2]);
    first.unmount();

    // Between steps (Steps 1–3 complete, no active or room offers) -> shows Next Guided (4/7) Jonas
    quests.applyFrame({
      schema: 1,
      completed: [
        { key: GUIDED_TOUR_KEYS[0], title: "S1" },
        { key: GUIDED_TOUR_KEYS[1], title: "S2" },
        { key: GUIDED_TOUR_KEYS[2], title: "S3" },
      ],
      active: [],
      available: [],
    });
    render(SheetPanel, { props: { app } });
    expect(screen.getByText(/Tour 3\/7/i)).toBeTruthy();
    expect(screen.getByText(/Next Guided \(4\/7\):/i)).toBeTruthy();
    expect(screen.getByText(/Jonas Achterberg-Nkemelu/i)).toBeTruthy();
  });
});
