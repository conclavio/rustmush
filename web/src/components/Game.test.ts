// @vitest-environment jsdom
//
// The game shell, mounted on a real `GameApp` (doc 43 cut TS.19). These
// replace the source probes `lib/game.test.ts` carried for `Game.svelte`:
// where the command box mounts, what the unit tab's badge counts. The app is
// driven through a fake socket, `lib/game.test.ts`'s harness in miniature,
// so every fact below is what a connected client would draw.

import { render, screen, within } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import Game from "./Game.svelte";
import { COCKPIT_FEED, NOT_IN_A_COCKPIT } from "$lib/cockpit";
import { GameApp } from "$lib/game.svelte";
import { REPLAY_FEED, WAITING_FOR_THE_REPLAY } from "$lib/replay";
import type { Envelope, OutputEvent } from "$lib/proto";
import type { WebSocketLike } from "$lib/ws";

/** A scriptable WebSocket: opened by hand, fed envelopes by hand. */
class FakeSocket implements WebSocketLike {
  readyState = 0;
  sent: string[] = [];
  onopen: (() => void) | null = null;
  onclose: ((ev: { code: number; wasClean: boolean }) => void) | null = null;
  onerror: (() => void) | null = null;
  onmessage: ((ev: { data: unknown }) => void) | null = null;

  constructor(public url: string) {}

  send(data: string): void {
    this.sent.push(data);
  }

  close(code = 1000): void {
    this.readyState = 3;
    this.onclose?.({ code, wasClean: true });
  }

  deliver(event: OutputEvent, seq: number): void {
    const env = { seq, ts: 1756300000000 + seq, ...event } as Envelope;
    this.onmessage?.({ data: JSON.stringify(env) });
  }
}

/** The manifest: the unit feed, the channels, and screen 0's two feeds. */
const PANELS = ["tactics.unit", "comsys", COCKPIT_FEED, REPLAY_FEED];

// jsdom draws no `<dialog>`: the shell's modals call these on mount.
HTMLDialogElement.prototype.showModal ??= function (this: HTMLDialogElement) {
  this.open = true;
};
HTMLDialogElement.prototype.close ??= function (this: HTMLDialogElement) {
  this.open = false;
};

/** A connected app past its hello. */
async function mountGame(): Promise<{ app: GameApp; ws: FakeSocket }> {
  let ws: FakeSocket | null = null;
  const app = new GameApp({
    url: "ws://test/ws",
    webSocketFactory: (url) => (ws = new FakeSocket(url)),
    random: () => 1,
  });
  app.client.connect("tok", "7");
  const socket = ws as unknown as FakeSocket;
  socket.readyState = 1;
  socket.onopen?.();
  socket.deliver(
    { t: "hello", d: { proto: 1, server: "test", features: [], panels: PANELS } } as OutputEvent,
    0,
  );
  render(Game, { props: { app } });
  await tick();
  return { app, ws: socket };
}

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

/** The one shell command box, or `null`. */
function commandBox(): HTMLElement | null {
  return screen.queryByRole("textbox", { name: "command input" });
}

/** The tab whose label is `label`. */
function tab(label: string): HTMLElement {
  const nav = screen.getByRole("navigation", { name: "screens" });
  const hit = within(nav)
    .getAllByRole("button")
    .find((b) => b.title.startsWith(`${label} (`));
  if (hit === undefined) {
    throw new Error(`no ${label} tab`);
  }
  return hit;
}

describe("the shell's one command box (16b cut B.1, `B-08`)", () => {
  it("mounts under the screen and above the key legend, and registers with the app", async () => {
    const { app } = await mountGame();
    const boxes = screen.getAllByRole("textbox", { name: "command input" });
    expect(boxes, "one box, the shell's: the scene screen mounts none").toHaveLength(1);
    const box = boxes[0]!;
    const shown = document.querySelector(".screen")!;
    const legend = screen.getByRole("contentinfo", { name: "keys" });
    expect(shown.compareDocumentPosition(box) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(box.compareDocumentPosition(legend) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(shown.contains(box)).toBe(false);
    expect(app.composer).not.toBeNull();
  });

  it("is not on the channel screen, and the app lets go of it there", async () => {
    const { app } = await mountGame();
    app.switchScreen("channels");
    await tick();
    expect(commandBox()).toBeNull();
    expect(app.composer).toBeNull();
    app.switchScreen("scene");
    await tick();
    expect(commandBox()).not.toBeNull();
    expect(app.composer).not.toBeNull();
  });
});

describe("the unit tab's badge counts only rows the panel can open", () => {
  it("is nothing until the server answers, then the ring's length", async () => {
    const { app } = await mountGame();
    app.unit.journal = [
      { object: "#7", rev: 4, cause: "gauss", at: 0, entry: { kind: "cell" } },
      { object: "#7", rev: 4, cause: "gauss", at: 0, entry: { kind: "section" } },
    ];
    await tick();
    expect(tab("unit").querySelector(".badge")).toBeNull();
    app.unit.answered = true;
    await tick();
    expect(tab("unit").querySelector(".badge")?.textContent).toBe("2");
  });
});

describe("screen 0 draws a replay while one is open and no seat is held (16b cut I.2)", () => {
  it("has no tab for a viewer with neither, and draws the replay, not the cockpit, once one opens", async () => {
    const { app } = await mountGame();
    expect(app.switchScreen("cockpit")).toBe(false);
    app.replay.open = true;
    await tick();
    expect(app.switchScreen("cockpit")).toBe(true);
    await tick();
    expect(screen.getByText(WAITING_FOR_THE_REPLAY)).toBeTruthy();
    expect(screen.queryByText(NOT_IN_A_COCKPIT)).toBeNull();
  });
});
