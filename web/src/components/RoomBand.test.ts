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
