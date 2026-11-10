// Doc 43 cut TS.9 (D-43-8, §7 TS.9, §10.1): the one browser smoke.
//
// Everything below the page has its own suites — the REST auth, the socket's
// resume, the picture module's parity with the Rust renderer — but none of
// them runs the page itself: `world_journeys/web_e2e.rs` drives the same
// server with reqwest and tungstenite, and vitest mounts components in jsdom
// with a mock socket. This is the one run where a real browser loads the
// bundle `mush-net-web` serves, opens the real socket, and the real picture
// module paints a real canvas. One journey, in the order a player meets it;
// each numbered step is TS.9's row.
import { expect, test, type Locator, type Page, type WebSocketRoute } from "@playwright/test";

const BASE = process.env.MUSHD_BASE_URL ?? "";

test.use({ baseURL: BASE });

test.afterEach(async ({}, info) => {
  // The server's side of a failure is in its log; attach it to the report.
  if (info.status !== info.expectedStatus && process.env.MUSHD_LOG) {
    await info.attach("mushd.log", { path: process.env.MUSHD_LOG, contentType: "text/plain" });
  }
});

/** A server envelope as it crosses the wire: `{ seq, ts, t, d }`. */
type Frame = { seq: number; t: string; d?: unknown };

/** Every socket the page opens goes through here and on to the server
 * unchanged, so the smoke can drop one the way a network does (step 7) and
 * read what crossed the wire. */
class SocketTap {
  urls: string[] = [];
  routes: WebSocketRoute[] = [];
  fromServer: Frame[] = [];
  fromPage: Array<{ t: string; d?: unknown }> = [];

  async install(page: Page): Promise<void> {
    await page.routeWebSocket(/\/ws(\?|$)/, (ws) => {
      this.urls.push(ws.url());
      this.routes.push(ws);
      const server = ws.connectToServer();
      ws.onMessage((m) => {
        if (typeof m === "string") this.fromPage.push(JSON.parse(m) as { t: string; d?: unknown });
        server.send(m);
      });
      server.onMessage((m) => {
        if (typeof m === "string") this.fromServer.push(JSON.parse(m) as Frame);
        ws.send(m);
      });
      ws.onClose((code, reason) => server.close({ code, reason }));
      server.onClose((code, reason) => ws.close({ code, reason }));
    });
  }

  /** The last settled legal-action answer (`committed` or `busy`) the server
   * sent from index `from` on, or `""` while none has arrived. */
  actionSettled(from: number): string {
    const states = this.fromServer
      .slice(from)
      .map((f) => (f.d as { result?: { service?: string; result?: { state?: string } } } | undefined)?.result)
      .filter((r) => r?.service === "legal_action")
      .map((r) => r?.result?.state ?? "")
      .filter((st) => st === "committed" || st === "busy");
    return states.at(-1) ?? "";
  }

  /** Server frames of event type `t`, from index `from` on. */
  events(t: string, from = 0): Frame[] {
    return this.fromServer.slice(from).filter((e) => e.t === t);
  }
}

/** The shell's command box: type a line and send it, as a player does. */
async function run(page: Page, line: string): Promise<void> {
  const box = page.getByRole("textbox", { name: "command input", exact: true });
  await box.fill(line);
  await box.press("Enter");
}

/** The world's one transient refusal: a commit is in flight. A player types
 * the line again; so does the smoke (as `sim_lenses/sim_range.rs` does). */
const COMMIT_PENDING = "please retry in a moment";

/** Send `line` until the output that arrives after it shows `expected`,
 * re-sending only on the transient refusal. Only the new text is read, so an
 * exit name already on screen never answers for a room not yet entered. */
async function runUntil(page: Page, scrollback: Locator, line: string, expected: string): Promise<void> {
  for (let attempt = 0; attempt < 20; attempt++) {
    const from = (await scrollback.innerText()).length;
    await run(page, line);
    let answer = "";
    await expect
      .poll(
        async () => {
          const fresh = (await scrollback.innerText()).slice(from);
          answer = fresh.includes(expected) ? "ok" : fresh.includes(COMMIT_PENDING) ? "retry" : "";
          return answer;
        },
        { message: `${line} -> ${expected}`, timeout: 30_000 },
      )
      .not.toBe("");
    if (answer === "ok") return;
    await page.waitForTimeout(250);
  }
  throw new Error(`${line}: never answered ${JSON.stringify(expected)}`);
}

test("a player plays in the browser", async ({ page }) => {
  const tap = new SocketTap();
  await tap.install(page);
  const wasm: number[] = [];
  page.on("response", (r) => {
    if (/\.wasm(\?|$)/.test(r.url())) wasm.push(r.status());
  });
  const status = page.locator("header .status");
  const charname = page.locator("header .charname");
  const scrollback = page.locator(".scene .viewport");
  const tab = (name: string) => page.locator("nav.tabs button").filter({ hasText: name });
  // Unique per run, so a server left up by hand (`node e2e/boot-mushd.mjs`
  // and MUSHD_BASE_URL) can take the smoke more than once.
  const suffix = Date.now().toString(36).slice(-5);
  const account = `smoke${suffix}`;
  const password = "smoke-pass-1";
  const charName = `Ada${suffix}`;

  // (1) The server is up: `boot-mushd.mjs` booted `mushd` on the memory store
  // and waited for `/healthz`.
  // (2) Headless Chromium opens the SPA: the bundle the binary itself serves
  // (`web/dist` through rust-embed), not a dev server.
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "rustmush" })).toBeVisible();

  // (3) Register through the form, then sign out and back in, so both REST
  // routes (`/api/auth/register`, `/api/auth/login`) run from the page and
  // the token reaches the socket's subprotocol.
  await page.getByRole("button", { name: "New account" }).click();
  await page.getByLabel("Account").fill(account);
  await page.getByLabel("Password").fill(password);
  await page.getByLabel("Character name").fill(charName);
  await page.getByRole("button", { name: "Create account" }).click();
  // One character: the page goes straight into the game.
  await expect(status).toHaveAttribute("data-status", "connected");
  await expect(charname).toHaveText(charName);
  await page.getByRole("button", { name: "sign out" }).click();
  await expect(page.getByRole("button", { name: "Create account" })).toHaveCount(0);
  await page.getByLabel("Account").fill(account);
  await page.getByLabel("Password").fill(password);
  await page.locator("form button[type=submit]").click();
  await expect(status).toHaveAttribute("data-status", "connected");

  // (4) Character select: back out to it and pick the character from the grid
  // (login with one character skips the grid, so the header's `characters`
  // button is the way a player reaches it).
  await page.getByRole("button", { name: "characters" }).click();
  await expect(page.getByRole("heading", { name: "Choose a character" })).toBeVisible();
  await page.getByRole("button", { name: charName }).click();
  await expect(status).toHaveAttribute("data-status", "connected");
  await expect(charname).toHaveText(charName);

  // (5) One verb: `look`, typed into the command box, answered in the
  // scrollback. A fresh character stands in Limbo.
  await runUntil(page, scrollback, "look", "Obvious exits");
  await expect(scrollback).toContainText("Limbo");
  expect(tap.fromPage.some((c) => c.t === "line" && c.d === "look")).toBe(true);

  // (6) One typed action: `Brace` on the actions screen, the one offer a fresh
  // character holds that takes no argument and touches nothing anybody else
  // owns. It goes over the wire as a typed `invoke` (the legal-action RPC),
  // never as a typed line, and the panel's transaction list shows the commit.
  // `busy` is the documented transient (the world refused the commit while
  // another was in flight — common in the seconds after a debug boot, while
  // the arena and the economy seed): the panel says "Try again later" and
  // re-enables the button, and the smoke presses it again as a player would.
  await tab("actions").click();
  const brace = page.getByRole("form", { name: "Brace" });
  await expect(brace).toBeVisible();
  let state = "";
  for (let attempt = 0; attempt < 10 && state !== "committed"; attempt++) {
    const from = tap.fromServer.length;
    await brace.getByRole("button", { name: "Brace" }).click();
    await expect
      .poll(() => tap.actionSettled(from), { message: "the Brace invoke settles", timeout: 30_000 })
      .not.toBe("");
    state = tap.actionSettled(from);
    if (state === "busy") await page.waitForTimeout(1_000);
  }
  expect(state).toBe("committed");
  await expect(page.locator(".center .screen")).toContainText("Recent transactions");
  await expect(page.locator(".center .screen")).toContainText("Action committed.");
  expect(tap.fromPage.some((c) => c.t === "invoke")).toBe(true);

  // (7) Reconnect: drop the socket from under the page, as a network does.
  // The client reconnects by itself with `?resume=<last seq>`; the server
  // answers `resumed { replayed: true }` (the gap came from its ring, not a
  // fresh attach), the scrollback is kept, and the session is live again.
  await tab("scene").click();
  const before = tap.routes.length;
  const framesBefore = tap.fromServer.length;
  const live = tap.routes[before - 1]!;
  await live.close({ code: 4000, reason: "smoke: dropped" });
  await expect.poll(() => tap.routes.length, { timeout: 30_000 }).toBeGreaterThan(before);
  expect(tap.urls.at(-1)).toMatch(/[?&]resume=\d+/);
  await expect
    .poll(() => tap.events("resumed", framesBefore).map((f) => (f.d as { replayed?: boolean }).replayed))
    .toEqual([true]);
  await expect(status).toHaveAttribute("data-status", "connected");
  await expect(scrollback).toContainText("Obvious exits");
  await runUntil(page, scrollback, "ooc", "Welcome Hall");

  // (8) The picture: the browser's own WebAssembly module drawing the cockpit
  // into a canvas. Only a seated pilot has a cockpit, so the fresh character
  // walks to the simulator centre, sets up a range (one side, targets that do
  // not move or shoot, no second player), starts it and boards — the same
  // walk the bot's lobby makes (`mush-bot/src/arena/lobby.rs`). The match
  // starts the moment every claimed slot is seated.
  await runUntil(page, scrollback, "simulator", "Simulator Lobby");
  await runUntil(page, scrollback, "SIM TEAM red", "takes red slot");
  await runUntil(page, scrollback, "SIM RANGE 1", "Range set");
  await runUntil(page, scrollback, "SIM START", "is boarding");
  await runUntil(page, scrollback, "red", "Obvious exits");
  // The first seated frame opens the cockpit tab and the shell goes to it.
  // `BOARD` has one transient of its own besides the commit in flight — the
  // control-lease mirror is a 1 Hz projection (`control state unknown`) — so
  // it is typed again, a few seconds apart, until the seat is taken.
  const cockpit = tab("cockpit");
  for (let attempt = 0; attempt < 10; attempt++) {
    await run(page, "BOARD");
    try {
      await expect(cockpit).toHaveAttribute("aria-current", "page", { timeout: 6_000 });
      break;
    } catch {
      // Not seated yet: the next attempt types it again; the assertion
      // below reports the failure if none ever seats.
    }
  }
  await expect(cockpit).toHaveAttribute("aria-current", "page");
  await page.getByRole("button", { name: "Show the picture" }).click();
  const canvas = page.locator(".frame canvas");
  await expect(canvas).toBeVisible();
  // Painted: a 1280x720 frame, the size the picture opens at, with more
  // than a handful of colours (sky,
  // ground, the hull, the readouts). A canvas the module never reached is
  // transparent black: one colour.
  await expect
    .poll(
      () =>
        canvas.evaluate((c: HTMLCanvasElement) => {
          if (c.width === 0 || c.height === 0) return { w: 0, colours: 0 };
          const d = c.getContext("2d")!.getImageData(0, 0, c.width, c.height).data;
          const colours = new Set<number>();
          for (let i = 0; i < d.length; i += 4) {
            colours.add(((d[i]! << 24) | (d[i + 1]! << 16) | (d[i + 2]! << 8) | d[i + 3]!) >>> 0);
          }
          return { w: c.width, colours: Math.min(colours.size, 9) };
        }),
      { message: "the cockpit picture paints", timeout: 30_000 },
    )
    .toEqual({ w: 1280, colours: 9 });
  // And it was the module that drew it: the page fetched the `.wasm` itself.
  expect(wasm).toContain(200);
});
