// @vitest-environment jsdom
//
// MarketScreen mounted (17g cut C3c; the C3 plan §5): the listings, the own strip, the
// holdings sections and the notices paint from the closed keyframes; the buy form shows the
// server's `terms_line` and `split_line`; Submit hands `app.submitMarket` the exact typed
// `MarketAction` and nothing ever reaches the line (`app.submit`, the composer); a click on a
// row or a section button sends nothing typed; an `error` reply shows its message verbatim and
// a `shown` reply keeps the accept form open with the lines inside. The rig is a real
// `MarketStore` holding the TUI's fixtures and spies for everything else the screen reaches.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { fireEvent, render, screen } from "@testing-library/svelte";
import { flushSync } from "svelte";
import { describe, expect, it, vi } from "vitest";

import MarketScreen from "./MarketScreen.svelte";
import type { GameApp } from "$lib/game.svelte";
import { MORE_PAGES, NO_PAGE } from "$lib/market";
import { MarketStore } from "$lib/stores/market.svelte";

const TOKEN = "f00dcafef00dcafef00dcafef00dcafef00dcafef00dcafef00dcafef00dcafe";
const ACCOUNT = "acct-7f3a9e";
const DIGEST = "sha256:9b2d4e6f8a0c1e3f5a7b9c1d3e5f7a9b";

function fixture(name: string): unknown {
  return JSON.parse(readFileSync(resolve(process.cwd(), "src/lib/fixtures/market", name), "utf8")) as unknown;
}

interface Rig {
  market: MarketStore;
  submitMarket: ReturnType<typeof vi.fn>;
  localAction: ReturnType<typeof vi.fn>;
  submit: ReturnType<typeof vi.fn>;
  focusComposer: ReturnType<typeof vi.fn>;
}

function mount(holdings = "owner_page.json", market = "market_page.json"): Rig {
  const store = new MarketStore();
  store.apply("mind.market", fixture(market));
  store.apply("mind.holdings", fixture(holdings));
  const rig: Rig = {
    market: store,
    submitMarket: vi.fn(async () => {}),
    localAction: vi.fn(),
    submit: vi.fn(),
    focusComposer: vi.fn(),
  };
  render(MarketScreen, { props: { app: rig as unknown as GameApp } });
  return rig;
}

/** Nothing reached the line: no command text, no composer prefill. */
function noLine(rig: Rig): void {
  expect(rig.submit).not.toHaveBeenCalled();
  expect(rig.focusComposer).not.toHaveBeenCalled();
}

function formEl(): HTMLFormElement {
  const form = document.querySelector<HTMLFormElement>(".sheet form");
  if (!form) throw new Error("no open form");
  return form;
}

describe("MarketScreen", () => {
  it("paints the rows, the own strip, the sections and no planted field", () => {
    mount();
    expect(screen.getAllByText("Drift Kestrel", { selector: "td" })).toHaveLength(2);
    expect(screen.getByText("Long Watch")).toBeTruthy();
    expect(screen.getByText("1250cr")).toBeTruthy();
    expect(screen.getByText("99.50cr")).toBeTruthy();
    expect(screen.getByText("no resale, derived")).toBeTruthy();
    expect(screen.getAllByText("30+7")).toHaveLength(2);
    expect(screen.getByText("quote 0a1b2c3d 1250cr 240s")).toBeTruthy();
    expect(screen.getByText("waiting 99aa77bb")).toBeTruthy();
    expect(screen.getByRole("button", { name: "owner" }).getAttribute("aria-pressed")).toBe("true");
    expect(screen.getByRole("button", { name: "paid" }).getAttribute("aria-pressed")).toBe("false");
    expect(screen.getByText("Active, 30d left")).toBeTruthy();
    // The price in minor units is never painted; a token, an account, a digest, a charter id
    // never reach the page at all.
    const html = document.body.innerHTML;
    for (const absent of ["125000", "9950", TOKEN, ACCOUNT, DIGEST, "4242"]) {
      expect(html).not.toContain(absent);
    }
  });

  it("paints the closed market's notice, and NO_PAGE for a bad shape", () => {
    const rig = mount("closed_holdings.json", "closed_market.json");
    expect(screen.getAllByText("The mind market is not open here.")).toHaveLength(2);
    rig.market.apply("mind.market", { schema: { id: "mind.market", version: 2 } });
    flushSync();
    expect(screen.getByText(NO_PAGE)).toBeTruthy();
  });

  it("the buy form shows the server's terms and split, and Submit sends the exact action", async () => {
    const rig = mount();
    rig.market.openBuy();
    flushSync();
    expect(screen.getByRole("dialog", { name: "Buy listing 0a1b2c3d: Drift Kestrel" })).toBeTruthy();
    expect(screen.getByText("One seat for 30 IC days plus 7 days of Grace; the licence can be resold.")).toBeTruthy();
    expect(screen.getByText("Price 1250cr: Registry 5%, venue 2%, charter owner the rest.")).toBeTruthy();
    expect(screen.getByText("Your quote: 1250cr, 240s left.")).toBeTruthy();
    // The default step is pay (a live quote), painted the TUI's way.
    expect(screen.getByRole("button", { name: "[pay]" }).getAttribute("aria-pressed")).toBe("true");
    await fireEvent.click(screen.getByRole("button", { name: "complete" }));
    expect(screen.getByRole("button", { name: "[complete]" })).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "pay" }));
    expect(rig.submitMarket).not.toHaveBeenCalled();
    await fireEvent.submit(formEl());
    expect(rig.submitMarket).toHaveBeenCalledTimes(1);
    expect(rig.submitMarket).toHaveBeenCalledWith({ act: "pay", code: "0a1b2c3d" });
    noLine(rig);
  });

  it("the publish form sends its form, royalty and transfer mark; a bad royalty sends nothing", async () => {
    const rig = mount();
    rig.market.openPublish();
    flushSync();
    expect(screen.getByRole("dialog", { name: "List filed mind 7: Drift Kestrel" })).toBeTruthy();
    expect(screen.getByText("per mille (empty is 0)")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "open-resale" }));
    await fireEvent.click(screen.getByRole("button", { name: "no" }));
    const royalty = formEl().querySelector<HTMLInputElement>("input")!;
    await fireEvent.input(royalty, { target: { value: "301" } });
    await fireEvent.submit(formEl());
    expect(rig.submitMarket).not.toHaveBeenCalled();
    expect(screen.getByRole("alert").textContent).toBe("Request refused: invalid market royalty");
    await fireEvent.input(royalty, { target: { value: "120" } });
    await fireEvent.submit(formEl());
    expect(rig.submitMarket).toHaveBeenCalledWith({
      act: "publish",
      filed: 7,
      form: "open_resale",
      royalty: 120,
      transfer: false,
    });
    noLine(rig);
  });

  it("a row click or a section button sends nothing typed", async () => {
    const rig = mount();
    await fireEvent.click(screen.getByRole("button", { name: "99aa77bb" }));
    expect(rig.market.selected).toBe("99aa77bb");
    expect(rig.market.focus).toBe("listings");
    await fireEvent.click(screen.getByRole("button", { name: "7" }));
    expect(rig.market.selectedHolding).toBe(7);
    expect(rig.market.focus).toBe("holdings");
    await fireEvent.click(screen.getByRole("button", { name: "paid" }));
    expect(rig.localAction).toHaveBeenCalledWith("marketSection");
    // The section already shown asks nothing.
    rig.localAction.mockClear();
    await fireEvent.click(screen.getByRole("button", { name: "owner" }));
    expect(rig.localAction).not.toHaveBeenCalled();
    expect(rig.submitMarket).not.toHaveBeenCalled();
    expect(document.querySelector(".sheet")).toBeNull();
    noLine(rig);
  });

  it("an error reply shows its message verbatim and closes the form", () => {
    const rig = mount();
    rig.market.openBuy();
    rig.market.begin({ act: "pay", code: "0a1b2c3d" });
    flushSync();
    expect(screen.getByText("Sent - waiting for the server")).toBeTruthy();
    expect(screen.getByText("Sent; Esc closes this window but does not cancel the request.")).toBeTruthy();
    rig.market.reply({ status: "error", code: "no_quote", message: "You have no quote on listing 0a1b2c3d." });
    flushSync();
    expect(screen.getByRole("status").textContent).toBe("You have no quote on listing 0a1b2c3d.");
    expect(document.querySelector(".sheet")).toBeNull();
  });

  it("a shown reply keeps the accept form open with the lines inside", async () => {
    const rig = mount("paid_page.json");
    rig.market.openAccept();
    flushSync();
    expect(screen.getByText("An empty price lists the offers that name you.")).toBeTruthy();
    const number = formEl().querySelector<HTMLInputElement>("input")!;
    await fireEvent.input(number, { target: { value: "41" } });
    await fireEvent.submit(formEl());
    expect(rig.submitMarket).toHaveBeenCalledWith({ act: "resell_accept", licence: 41, price: null });
    rig.market.begin({ act: "resell_accept", licence: 41, price: null });
    rig.market.reply({ status: "shown", lines: ["Vex offers licence 41 to you for 5cr.", "Accept it with: price=5"] });
    flushSync();
    expect(screen.getByRole("dialog", { name: "Accept an offer" })).toBeTruthy();
    expect(screen.getByText("Accept it with: price=5")).toBeTruthy();
    expect(screen.getByRole("status").textContent).toBe("Vex offers licence 41 to you for 5cr.");
    noLine(rig);
  });

  it("two listings sharing a code both paint (a code is a token's first 8 characters)", () => {
    const rig = mount();
    const page = fixture("market_page.json") as { rows: { code: string; name: string }[] };
    page.rows[1]!.code = page.rows[0]!.code;
    expect(rig.market.apply("mind.market", page)).toBe(true);
    flushSync();
    expect(screen.getAllByRole("button", { name: "0a1b2c3d" })).toHaveLength(2);
    expect(screen.getByText("Long Watch")).toBeTruthy();
  });

  it("a row or section pick lets go of the button, so the market's bare keys reach the shell", async () => {
    const rig = mount();
    for (const name of ["99aa77bb", "7", "paid"]) {
      const button = screen.getByRole("button", { name });
      button.focus();
      expect(document.activeElement).toBe(button);
      await fireEvent.click(button);
      expect(document.activeElement).not.toBe(button);
    }
    expect(rig.submitMarket).not.toHaveBeenCalled();
    noLine(rig);
  });

  it("the next-page hint shows under the focused pane only: ] pages that pane", async () => {
    mount("paid_page.json");
    const listings = (): HTMLElement => screen.getByRole("region", { name: "mind market" });
    const holdings = (): HTMLElement => screen.getByRole("region", { name: "holdings" });
    // Both pages have a next one; the listings pane has the focus.
    expect(listings().textContent).toContain(MORE_PAGES);
    expect(holdings().textContent).not.toContain(MORE_PAGES);
    await fireEvent.click(screen.getByRole("button", { name: "41" }));
    expect(listings().textContent).not.toContain(MORE_PAGES);
    expect(holdings().textContent).toContain(MORE_PAGES);
  });

  it("a staged offer's notice takes the focus, and Enter closes it as its foot says", async () => {
    const rig = mount();
    rig.market.openOffer();
    if (rig.market.form?.kind === "offer") {
      rig.market.form.player = "Vex";
      rig.market.form.price = "5";
    }
    rig.market.begin({ act: "charter", filed: 7, player: "Vex", price: "5" });
    flushSync();
    rig.market.reply({ status: "staged", line: "Offering filed mind 7 and its charter to Vex for 5cr." });
    flushSync();
    await Promise.resolve();
    expect(screen.getByRole("dialog", { name: "Offer sent" })).toBeTruthy();
    expect(screen.getByText("Enter or Esc closes.")).toBeTruthy();
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Close" }));
    // Enter from anywhere (the focus moved off, say) closes it.
    (document.activeElement as HTMLElement).blur();
    await fireEvent.keyDown(window, { key: "Enter" });
    expect(document.querySelector(".sheet")).toBeNull();
    expect(rig.submitMarket).not.toHaveBeenCalled();
    noLine(rig);
  });

  it("a shown reply hands the focus back to the accept form's first field", async () => {
    const rig = mount("paid_page.json");
    rig.market.openAccept();
    rig.market.begin({ act: "resell_accept", licence: 41, price: null });
    flushSync();
    rig.market.reply({ status: "shown", lines: ["Vex offers licence 41 to you for 5cr."] });
    flushSync();
    await Promise.resolve();
    expect(document.activeElement).toBe(formEl().querySelector("input"));
    // Enter in a form that is not a notice is the field's own (it submits the form).
    await fireEvent.keyDown(window, { key: "Enter" });
    expect(screen.getByRole("dialog", { name: "Accept an offer" })).toBeTruthy();
  });

  it("Esc closes the form and never sends", async () => {
    const rig = mount();
    rig.market.openWithdraw();
    flushSync();
    expect(
      screen.getByText(
        "Withdraw filed mind #7 Drift Kestrel? New sales stop; licences already sold keep working. Enter withdraws; Esc cancels.",
      ),
    ).toBeTruthy();
    await fireEvent.keyDown(window, { key: "Escape" });
    expect(document.querySelector(".sheet")).toBeNull();
    expect(rig.submitMarket).not.toHaveBeenCalled();
  });
});
