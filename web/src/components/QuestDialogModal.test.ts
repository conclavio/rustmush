// @vitest-environment jsdom

import { fireEvent, render, screen } from "@testing-library/svelte";
import { describe, expect, it, vi } from "vitest";

import QuestDialogModal from "./QuestDialogModal.svelte";
import type { GameApp } from "$lib/game.svelte";
import { GUIDED_TOUR_KEYS, QuestsStore } from "$lib/stores/quests.svelte";

describe("QuestDialogModal guided corridor tour roadmap", () => {
  it("renders all 7 labeled roadmap steps and opens quest dialog when clicking an active or available step", async () => {
    const quests = new QuestsStore();
    quests.applyFrame({
      schema: 1,
      room_id: "#15",
      room_name: "Registry & Co-op Desk",
      room_markers: [],
      completed: [
        { key: GUIDED_TOUR_KEYS[0], title: "First Papers at the Registry" },
      ],
      active: [
        {
          key: GUIDED_TOUR_KEYS[1],
          title: "Stock the Locker and Tanks",
          introduction: "Buy 10 provisions and 5t propellant.",
          giver_name: "Ilse Oduya-Brandt",
          turn_in_name: "Ilse Oduya-Brandt",
          reward_summary: "Guided Tour Step 2/7",
          next_step: "Buy 10 provisions",
          checkpoints: [
            {
              id: "buy_provisions",
              title: "Buy 10 provisions",
              instructions: "Visit Co-op Chandlery.",
              room_entry: "room.c08_chandlery",
              room_name: "Co-op Chandlery",
              checked: false,
              live_ready: false,
            },
          ],
        },
      ],
      available: [
        {
          key: GUIDED_TOUR_KEYS[2],
          title: "Metals Down the Corridor",
          introduction: "Haul 5t metals to Stickney.",
          giver_name: "Ilse Oduya-Brandt",
          turn_in_name: "Jonas Achterberg-Nkemelu",
          reward_summary: "Guided Tour Step 3/7",
          accept_cmd: "+quest/accept #18 tok3",
          checkpoints: [],
        },
      ],
    });
    quests.openLog("active");

    const app = {
      quests,
      tacsim: { trade: null },
      focusComposer: vi.fn(),
    } as unknown as GameApp;

    render(QuestDialogModal, { app, onCmd: vi.fn() });

    const step1 = screen.getByText("1. Co-op Charter").closest(".tour-step");
    const step2Btn = screen.getByRole("button", { name: /2\. Locker & Tanks/ });
    const step3Btn = screen.getByRole("button", { name: /3\. Metals Haul/ });
    const step7 = screen.getByText("7. Sell Mined Ore").closest(".tour-step");

    expect(step1?.classList.contains("done")).toBe(true);
    expect(step2Btn.classList.contains("current")).toBe(true);
    expect(step3Btn.classList.contains("avail")).toBe(true);
    expect(step7?.classList.contains("upcoming")).toBe(true);

    // Clicking an available step opens its quest dialog directly
    await fireEvent.click(step3Btn);
    expect(quests.dialogOpen).toBe(true);
    expect(quests.dialogQuestKey).toBe(GUIDED_TOUR_KEYS[2]);
    expect(screen.getByRole("dialog", { name: "Quest dialog" })).toBeTruthy();
  });
});
