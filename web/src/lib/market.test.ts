// The mind market's closed decoders and the screen's store (17g cut C3c; the C3 plan §5): the
// C3a keyframes decode, a wrong schema id or version and a numeric `price_minor` do not,
// unknown keys are ignored and unknown action words dropped; the store's forms take the TUI's
// defaults (17g cut C3b) and build exactly one typed `MarketAction`, and its reply contract is
// the TUI's — `staged` closes, `shown` paints in the open accept form, `error` closes with the
// server's message, and an unseen reply is never a refusal.
//
// The fixtures under `fixtures/market/` are the TUI's (`mush-net-ssh/tests/market_screen.rs`)
// one for one, with a publication token, an account and a digest planted in fields the
// decoders must not keep.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it } from "vitest";

import {
  LAST_PAGE,
  LOST,
  NOT_LISTABLE,
  NOT_OFFERABLE,
  NOT_RENEWABLE,
  NO_NUMBER,
  NO_PAGE,
  NO_ROW,
  OFFLINE,
  PENDING,
  MAX_MARKET_ROWS,
  decodeHoldingsView,
  decodeMarketView,
  holdingFlags,
  holdingKind,
  isListingCode,
  listingFlags,
  marketActionError,
  ownStrip,
  phaseCell,
  termCell,
} from "./market";
import { MarketStore, formTitle, offerClosing, withdrawText } from "./stores/market.svelte";

const TOKEN = "f00dcafef00dcafef00dcafef00dcafef00dcafef00dcafef00dcafef00dcafe";
const ACCOUNT = "acct-7f3a9e";
const DIGEST = "sha256:9b2d4e6f8a0c1e3f5a7b9c1d3e5f7a9b";

function fixture(name: string): Record<string, unknown> {
  return JSON.parse(
    readFileSync(resolve(process.cwd(), "src/lib/fixtures/market", name), "utf8"),
  ) as Record<string, unknown>;
}

/** A deep copy with `edit` applied. */
function edited(name: string, edit: (v: Record<string, any>) => void): Record<string, unknown> {
  const v = fixture(name) as Record<string, any>;
  edit(v);
  return v;
}

describe("decodeMarketView", () => {
  it("decodes the C3a page, the own strip and the cursor, and keeps no planted field", () => {
    const view = decodeMarketView(fixture("market_page.json"));
    expect(view).not.toBeNull();
    expect(view!.open).toBe(true);
    expect(view!.notice).toBeNull();
    expect(view!.after).toBe("99aa77bb");
    expect(view!.rows.map((r) => r.code)).toEqual(["0a1b2c3d", "99aa77bb"]);
    expect(view!.rows[0]).toEqual({
      code: "0a1b2c3d",
      name: "Drift Kestrel",
      class: "II",
      form: "open",
      transferable: true,
      derived: false,
      termSeconds: 2592000,
      graceSeconds: 604800,
      priceMinor: "125000",
      priceText: "1250cr",
      levyPermille: 50,
      venuePermille: 20,
      parentRoyaltyPermille: 0,
      termsLine: "One seat for 30 IC days plus 7 days of Grace; the licence can be resold.",
      splitLine: "Price 1250cr: Registry 5%, venue 2%, charter owner the rest.",
    });
    expect(view!.own).toEqual({
      quotes: [{ code: "0a1b2c3d", priceMinor: "125000", priceText: "1250cr", expiresInSeconds: 240, expired: false }],
      waiting: ["99aa77bb"],
    });
    const kept = JSON.stringify(view);
    for (const planted of [TOKEN, ACCOUNT, DIGEST]) {
      expect(kept).not.toContain(planted);
    }
  });

  it("decodes the closed and the empty page", () => {
    expect(decodeMarketView(fixture("closed_market.json"))).toEqual({
      open: false,
      notice: "The mind market is not open here.",
      rows: [],
      after: null,
      own: { quotes: [], waiting: [] },
    });
    const third = decodeMarketView(fixture("third_page.json"));
    expect(third?.own.quotes[0]).toEqual({
      code: "deadbeef",
      priceMinor: "7000",
      priceText: "70cr",
      expiresInSeconds: 0,
      expired: true,
    });
  });

  it("refuses a wrong schema id or version, a numeric price, a bad code, a bad cursor", () => {
    expect(decodeMarketView(edited("market_page.json", (v) => (v.schema.id = "mind.holdings")))).toBeNull();
    expect(decodeMarketView(edited("market_page.json", (v) => (v.schema.version = 2)))).toBeNull();
    expect(decodeMarketView(edited("market_page.json", (v) => delete v.schema))).toBeNull();
    expect(decodeMarketView(edited("market_page.json", (v) => (v.rows[0].price_minor = 125000)))).toBeNull();
    expect(decodeMarketView(edited("market_page.json", (v) => (v.rows[0].price_minor = "12.5")))).toBeNull();
    expect(decodeMarketView(edited("market_page.json", (v) => (v.own.quotes[0].price_minor = 125000)))).toBeNull();
    expect(decodeMarketView(edited("market_page.json", (v) => (v.rows[1].code = "99AA77BB")))).toBeNull();
    expect(decodeMarketView(edited("market_page.json", (v) => (v.rows[1].code = "gggggggg")))).toBeNull();
    expect(decodeMarketView(edited("market_page.json", (v) => (v.rows[1].code = "99aa77b")))).toBeNull();
    expect(decodeMarketView(edited("market_page.json", (v) => (v.after = "short")))).toBeNull();
    expect(decodeMarketView(edited("market_page.json", (v) => (v.own.waiting = [{ code: 7 }])))).toBeNull();
    expect(decodeMarketView(edited("market_page.json", (v) => delete v.rows[0].terms_line))).toBeNull();
    expect(decodeMarketView(edited("market_page.json", (v) => (v.open = "yes")))).toBeNull();
    expect(decodeMarketView(edited("market_page.json", (v) => delete v.own))).toBeNull();
    expect(decodeMarketView(null)).toBeNull();
    expect(decodeMarketView([])).toBeNull();
  });

  it("keeps the first 128 rows, quotes and waiting codes of a longer list (the C3 plan §7)", () => {
    const code = (i: number): string => i.toString().padStart(8, "0");
    const view = decodeMarketView(edited("market_page.json", (v) => {
      const row = v.rows[0];
      const quote = v.own.quotes[0];
      v.rows = Array.from({ length: MAX_MARKET_ROWS + 1 }, (_, i) => ({ ...row, code: code(i) }));
      v.own.quotes = Array.from({ length: MAX_MARKET_ROWS + 1 }, (_, i) => ({ ...quote, code: code(i) }));
      v.own.waiting = Array.from({ length: MAX_MARKET_ROWS + 1 }, (_, i) => ({ code: code(i) }));
    }));
    expect(MAX_MARKET_ROWS).toBe(128);
    expect(view).not.toBeNull();
    const kept = Array.from({ length: MAX_MARKET_ROWS }, (_, i) => code(i));
    expect(view!.rows.map((r) => r.code)).toEqual(kept);
    expect(view!.own.quotes.map((q) => q.code)).toEqual(kept);
    expect(view!.own.waiting).toEqual(kept);
    // A bad shape past the cap is never read, so it cannot refuse the page.
    const past = decodeMarketView(edited("market_page.json", (v) => {
      v.rows = [...Array.from({ length: MAX_MARKET_ROWS }, () => v.rows[0]), { code: 7 }];
    }));
    expect(past?.rows).toHaveLength(MAX_MARKET_ROWS);
  });

  it("ignores unknown keys", () => {
    const view = decodeMarketView(edited("market_page.json", (v) => {
      v.extra = { nested: true };
      v.rows[0].future_field = 1;
    }));
    expect(view?.rows).toHaveLength(2);
  });
});

describe("decodeHoldingsView", () => {
  it("decodes both sections and drops an unknown or other-kind action word", () => {
    const owner = decodeHoldingsView(fixture("owner_page.json"));
    expect(owner).toEqual({
      open: true,
      notice: null,
      section: "owner",
      after: null,
      rows: [
        {
          kind: "owner",
          filed: 7,
          name: "Drift Kestrel",
          dependent: false,
          phase: { state: "active", daysLeft: 30 },
          selling: false,
          actions: ["publish", "withdraw", "charter"],
        },
      ],
    });
    const paid = decodeHoldingsView(fixture("paid_page.json"));
    expect(paid?.after).toBe(42);
    expect(paid?.rows[0]).toEqual({
      kind: "paid",
      licence: 41,
      house: false,
      name: "Kestrel Line",
      form: "open",
      transferable: true,
      phase: { state: "grace", daysLeft: 4 },
      revoked: false,
      selling: false,
      renewal: "offered",
      actions: ["renew_pay", "renew_decline", "resell"],
    });
    const kept = JSON.stringify([owner, paid]);
    for (const planted of [TOKEN, ACCOUNT, DIGEST, "4242"]) {
      expect(kept).not.toContain(planted);
    }
  });

  it("refuses a wrong schema, an unknown kind, a bad phase, a non-string action, a bad cursor", () => {
    expect(decodeHoldingsView(edited("owner_page.json", (v) => (v.schema.version = 0)))).toBeNull();
    expect(decodeHoldingsView(edited("owner_page.json", (v) => (v.schema.id = "mind.market")))).toBeNull();
    expect(decodeHoldingsView(edited("owner_page.json", (v) => (v.section = "both")))).toBeNull();
    expect(decodeHoldingsView(edited("owner_page.json", (v) => (v.rows[0].kind = "loan")))).toBeNull();
    expect(decodeHoldingsView(edited("owner_page.json", (v) => (v.rows[0].phase = { state: "active", days_left: null })))).toBeNull();
    expect(decodeHoldingsView(edited("owner_page.json", (v) => (v.rows[0].phase = { state: "lapsed", days_left: 3 })))).toBeNull();
    expect(decodeHoldingsView(edited("owner_page.json", (v) => (v.rows[0].actions = ["publish", 7])))).toBeNull();
    expect(decodeHoldingsView(edited("owner_page.json", (v) => (v.rows[0].filed = -1)))).toBeNull();
    expect(decodeHoldingsView(edited("owner_page.json", (v) => (v.after = "7")))).toBeNull();
    expect(decodeHoldingsView(edited("paid_page.json", (v) => (v.rows[0].renewal = "maybe")))).toBeNull();
    expect(decodeHoldingsView(edited("paid_page.json", (v) => (v.after = 2 ** 53)))).toBeNull();
    // Rows past 128 are cut (a page is 20); an action list past 128 words is no row the server
    // builds (each kind has at most four words), so it refuses the page instead.
    const many = decodeHoldingsView(
      edited("owner_page.json", (v) => (v.rows = Array.from({ length: MAX_MARKET_ROWS + 1 }, (_, i) => ({ ...v.rows[0], filed: i })))),
    );
    expect(many?.rows.map((r) => (r.kind === "owner" ? r.filed : -1))).toEqual(
      Array.from({ length: MAX_MARKET_ROWS }, (_, i) => i),
    );
    expect(
      decodeHoldingsView(edited("owner_page.json", (v) => (v.rows[0].actions = Array(MAX_MARKET_ROWS).fill("publish")))),
    ).not.toBeNull();
    expect(
      decodeHoldingsView(edited("owner_page.json", (v) => (v.rows[0].actions = Array(MAX_MARKET_ROWS + 1).fill("publish")))),
    ).toBeNull();
    expect(
      decodeHoldingsView(edited("owner_page.json", (v) => (v.rows[0].phase = { state: "lapsed", days_left: null }))),
    ).not.toBeNull();
  });
});

describe("the cells the screen paints", () => {
  it("TERM+GRACE, flags, kinds, phases and the own strip, as the TUI paints them", () => {
    const view = decodeMarketView(fixture("market_page.json"))!;
    expect(termCell(view.rows[0]!)).toBe("30+7");
    expect(listingFlags(view.rows[0]!)).toBe("");
    expect(listingFlags(view.rows[1]!)).toBe("no resale, derived");
    expect(ownStrip(view)).toEqual(["quote 0a1b2c3d 1250cr 240s", "waiting 99aa77bb"]);
    expect(ownStrip(decodeMarketView(fixture("third_page.json"))!)).toEqual([
      "quote deadbeef 70cr expired",
      "waiting 12345678",
    ]);
    const paid = decodeHoldingsView(fixture("paid_page.json"))!;
    expect(paid.rows.map(holdingKind)).toEqual(["paid", "House Pin"]);
    expect(paid.rows.map((r) => phaseCell(r.phase))).toEqual(["Grace, 4d left", "Active, 9d left"]);
    expect(paid.rows.map(holdingFlags)).toEqual(["renew offer", "no resale"]);
    expect(phaseCell({ state: "lapsed", daysLeft: null })).toBe("Lapsed");
  });
});

describe("marketActionError (mush-proto's `validate` bounds)", () => {
  it("passes the well-formed thirteen and names what it refuses in validate's words", () => {
    expect(isListingCode("0a1b2c3d")).toBe(true);
    expect(isListingCode("0A1B2C3D")).toBe(false);
    expect(marketActionError({ act: "pay", code: "0a1b2c3d" })).toBeNull();
    expect(marketActionError({ act: "pay", code: "0a1b2c3" })).toBe("invalid market code");
    expect(marketActionError({ act: "resell", licence: 41, player: "Vex", price: "0.21cr" })).toBeNull();
    expect(marketActionError({ act: "resell", licence: 41, player: "", price: "1" })).toBe("invalid market player");
    expect(marketActionError({ act: "charter", filed: 7, player: "x".repeat(33), price: "1" })).toBe(
      "invalid market player",
    );
    expect(marketActionError({ act: "charter", filed: 7, player: "Vex", price: "1 cr" })).toBe("invalid market price");
    expect(marketActionError({ act: "resell_accept", licence: 41, price: null })).toBeNull();
    expect(marketActionError({ act: "charter_accept", filed: 7, price: "1".repeat(17) })).toBe("invalid market price");
    expect(marketActionError({ act: "publish", filed: 7, form: "open", royalty: 300, transfer: true })).toBeNull();
    expect(marketActionError({ act: "publish", filed: 7, form: "open", royalty: 301, transfer: true })).toBe(
      "invalid market royalty",
    );
    expect(marketActionError({ act: "renew", licence: -1 })).toBe("invalid market number");
    expect(marketActionError({ act: "withdraw", filed: 7 })).toBeNull();
  });
});

/** A store holding the C3a page and a holdings section. */
function stored(holdings = "owner_page.json", market = "market_page.json"): MarketStore {
  const store = new MarketStore();
  expect(store.apply("mind.market", fixture(market))).toBe(true);
  expect(store.apply("mind.holdings", fixture(holdings))).toBe(true);
  return store;
}

describe("MarketStore", () => {
  it("holds the pages, selects the first rows, and paints NO_PAGE for a bad shape", () => {
    const store = stored();
    expect(store.open).toBe(true);
    expect(store.selected).toBe("0a1b2c3d");
    expect(store.selectedHolding).toBe(7);
    expect(store.badge).toBe(1);
    expect(store.apply("mind.market", edited("market_page.json", (v) => (v.schema.version = 9)))).toBe(false);
    expect(store.market).toBeNull();
    expect(store.marketBad).toBe(true);
    expect(store.badge).toBe(0);
    // A bad shape opens no form.
    store.openBuy();
    expect(store.form).toBeNull();
    expect(store.notice).toBe(NO_PAGE);
    expect(store.apply("mind.market", fixture("closed_market.json"))).toBe(true);
    expect(store.open).toBe(false);
    expect(store.badge).toBe(0);
  });

  it("the buy form's default step follows the actor's own state", () => {
    const store = stored();
    store.openBuy();
    expect(store.form).toEqual({
      kind: "buy",
      code: "0a1b2c3d",
      name: "Drift Kestrel",
      step: "pay",
      confirm: [
        "One seat for 30 IC days plus 7 days of Grace; the licence can be resold.",
        "Price 1250cr: Registry 5%, venue 2%, charter owner the rest.",
        "Your quote: 1250cr, 240s left.",
      ],
    });
    expect(formTitle(store.form!)).toBe("Buy listing 0a1b2c3d: Drift Kestrel");
    store.closeForm();
    store.selected = "99aa77bb";
    store.openBuy();
    expect(store.form).toMatchObject({ kind: "buy", step: "complete" });
    store.closeForm();
    store.apply("mind.market", fixture("third_page.json"));
    store.selected = "c0ffee00";
    store.openBuy();
    expect(store.form).toMatchObject({ kind: "buy", step: "quote" });
    store.closeForm();
    store.selected = "deadbeef";
    store.openBuy();
    expect(store.form).toMatchObject({ kind: "buy", step: "quote" });
    expect(store.form?.kind === "buy" && store.form.confirm.at(-1)).toBe("Your quote has expired.");
    store.closeForm();
    // A waiting purchase off this page is still reachable from the own strip.
    store.selected = "12345678";
    store.openBuy();
    expect(store.form).toMatchObject({ kind: "buy", code: "12345678", name: "", step: "complete", confirm: [] });
    expect(formTitle(store.form!)).toBe("Buy listing 12345678");
    store.closeForm();
    store.selected = "ffffffff";
    store.openBuy();
    expect(store.form).toBeNull();
    expect(store.notice).toBe(NO_ROW);
  });

  it("each form builds exactly one typed action from its fields", () => {
    const store = stored();
    store.openBuy();
    if (store.form?.kind === "buy") store.form.step = "refund";
    expect(store.build()).toEqual({ act: "refund", code: "0a1b2c3d" });

    store.openPublish();
    expect(formTitle(store.form!)).toBe("List filed mind 7: Drift Kestrel");
    expect(store.build()).toEqual({ act: "publish", filed: 7, form: "sealed", royalty: null, transfer: true });
    if (store.form?.kind === "publish") {
      store.form.form = "open_resale";
      store.form.royalty = "120";
      store.form.transfer = false;
    }
    expect(store.build()).toEqual({ act: "publish", filed: 7, form: "open_resale", royalty: 120, transfer: false });
    if (store.form?.kind === "publish") store.form.royalty = "301";
    expect(store.build()).toBe("Request refused: invalid market royalty");
    if (store.form?.kind === "publish") store.form.royalty = "70000";
    expect(store.build()).toBe("Request refused: invalid market royalty");

    store.openWithdraw();
    expect(formTitle(store.form!)).toBe("Withdraw filed mind 7");
    expect(store.build()).toEqual({ act: "withdraw", filed: 7 });
    expect(withdrawText(7, "Drift Kestrel")).toBe(
      "Withdraw filed mind #7 Drift Kestrel? New sales stop; licences already sold keep working. Enter withdraws; Esc cancels.",
    );

    store.openOffer();
    expect(formTitle(store.form!)).toBe("Offer filed mind 7 and its charter: Drift Kestrel");
    if (store.form?.kind === "offer") {
      store.form.player = "  Vex  ";
      store.form.price = " 0.21cr ";
    }
    expect(store.build()).toEqual({ act: "charter", filed: 7, player: "Vex", price: "0.21cr" });

    store.openAccept();
    expect(store.build()).toBe(NO_NUMBER);
    if (store.form?.kind === "accept") store.form.number = "41";
    expect(store.build()).toEqual({ act: "resell_accept", licence: 41, price: null });
    if (store.form?.kind === "accept") {
      store.form.target = "filed";
      store.form.price = "12.50";
    }
    expect(store.build()).toEqual({ act: "charter_accept", filed: 41, price: "12.50" });

    // The owner section offers no renewal: the paid one does, and `offered` → pay.
    store.openRenew();
    expect(store.notice).toBe(NOT_RENEWABLE);
    store.apply("mind.holdings", fixture("paid_page.json"));
    store.selectedHolding = 41;
    store.openRenew();
    expect(store.form).toEqual({ kind: "renew", licence: 41, name: "Kestrel Line", step: "pay" });
    expect(store.build()).toEqual({ act: "renew_pay", licence: 41 });
    store.selectedHolding = 42;
    store.openRenew();
    expect(store.form).toMatchObject({ step: "request" });
    expect(store.build()).toEqual({ act: "renew", licence: 42 });
    // The House Pin is not resaleable, and a paid row is never listed.
    store.openOffer();
    expect(store.notice).toBe(NOT_OFFERABLE);
    store.openPublish();
    expect(store.notice).toBe(NOT_LISTABLE);
    store.selectedHolding = 41;
    store.openOffer();
    expect(formTitle(store.form!)).toBe("Offer licence 41: Kestrel Line");
  });

  it("one pending request at a time, and the reply contract is the TUI's", () => {
    const store = stored();
    store.openOffer();
    if (store.form?.kind === "offer") {
      store.form.player = "Vex";
      store.form.price = "5";
    }
    const action = store.build();
    expect(typeof action).toBe("object");
    store.begin(action as Exclude<typeof action, string | null>);
    expect(store.pending).toBe(true);
    // The form keys answer PENDING and open nothing.
    store.openAccept();
    expect(store.notice).toBe(PENDING);
    expect(store.form?.kind).toBe("offer");
    expect(store.reply({ status: "staged", line: "Offering filed mind 7 and its charter to Vex for 5cr." })).toBe(
      "staged",
    );
    expect(store.pending).toBe(false);
    expect(store.notice).toBe("Offering filed mind 7 and its charter to Vex for 5cr.");
    expect(store.form).toEqual({ kind: "notice", title: "Offer sent", lines: [offerClosing("filed", 7, "5")] });
    expect(offerClosing("filed", 7, "5")).toBe("They accept with: a, filed 7, price 5.");
    expect(store.build()).toBeNull();
    expect(store.form).toBeNull();

    store.openAccept();
    if (store.form?.kind === "accept") store.form.number = "41";
    store.begin({ act: "resell_accept", licence: 41, price: null });
    expect(store.reply({ status: "shown", lines: ["Vex offers licence 41 to you for 5cr.", "Accept it with: price=5"] })).toBe("shown");
    expect(store.form).toMatchObject({ kind: "accept", shown: ["Vex offers licence 41 to you for 5cr.", "Accept it with: price=5"] });
    expect(store.notice).toBe("Vex offers licence 41 to you for 5cr.");

    store.begin({ act: "resell_accept", licence: 41, price: "5" });
    expect(store.reply({ status: "error", code: "no_offer_at", message: "No offer names you at that price." })).toBe("error");
    expect(store.form).toBeNull();
    expect(store.notice).toBe("No offer names you at that price.");

    store.begin({ act: "pay", code: "0a1b2c3d" });
    expect(store.reply({ status: "library", names: [] })).toBe("unexpected");
  });

  it("a reply never seen is not a refusal, and a reset forgets the pending request", () => {
    const store = stored();
    store.openBuy();
    store.begin({ act: "pay", code: "0a1b2c3d" });
    store.lost();
    expect(store.form).toBeNull();
    expect(store.notice).toBe(LOST);
    store.begin({ act: "pay", code: "0a1b2c3d" });
    const generation = store.generation;
    store.reset();
    expect(store.generation).not.toBe(generation);
    expect(store.pending).toBe(false);
    expect(store.notice).toBe(LOST);
    expect(store.market).toBeNull();
    // A second reset (the hello that follows a drop's reset) keeps the warning: it is sticky.
    store.reset();
    expect(store.notice).toBe(LOST);
    // The next sentence replaces it, and a reset with nothing pending then leaves nothing.
    store.openBuy();
    expect(store.notice).toBe(NO_PAGE);
    store.reset();
    expect(store.notice).toBe("");
    // A logout is an account boundary: the warning goes with the account.
    store.begin({ act: "pay", code: "0a1b2c3d" });
    store.reset();
    expect(store.notice).toBe(LOST);
    store.reset(true);
    expect(store.notice).toBe("");
  });

  it("a request that never left is not LOST: the store is free and the form stays open", () => {
    const store = stored();
    store.openBuy();
    store.begin({ act: "pay", code: "0a1b2c3d" });
    store.unsent(OFFLINE);
    expect(store.pending).toBe(false);
    expect(store.notice).toBe(OFFLINE);
    expect(store.form?.kind).toBe("buy");
    // Free again: the next key is not refused PENDING.
    store.openAccept();
    expect(store.form?.kind).toBe("accept");
  });

  it("a shown reply with no accept form open paints an Offers notice, never drops the lines", () => {
    const store = stored();
    store.openAccept();
    store.begin({ act: "resell_accept", licence: 41, price: null });
    // Esc on the pending form: it closes the window, not the request.
    store.closeForm();
    expect(store.pending).toBe(true);
    const lines = ["Vex offers licence 41 to you for 5cr.", "Accept it with: price=5"];
    expect(store.reply({ status: "shown", lines })).toBe("shown");
    expect(store.form).toEqual({ kind: "notice", title: "Offers", lines });
    expect(store.notice).toBe(lines[0]);
    expect(store.pending).toBe(false);
  });

  it("pages, sections and the cursors a staged reply re-asks", () => {
    const store = stored();
    expect(store.listingsAsk()).toEqual({ kind: "listings", after: null });
    expect(store.pageNext()).toEqual({ kind: "listings", after: "99aa77bb" });
    store.noteAsk({ kind: "listings", after: "99aa77bb" });
    expect(store.listingsAsk()).toEqual({ kind: "listings", after: "99aa77bb" });
    store.focus = "holdings";
    expect(store.pageNext()).toBeNull();
    expect(store.notice).toBe(LAST_PAGE);
    expect(store.flipSection()).toEqual({ kind: "holdings", section: "paid", after: null });
    store.apply("mind.holdings", fixture("paid_page.json"));
    expect(store.pageNext()).toEqual({ kind: "holdings", section: "paid", after: 42 });
    store.noteAsk({ kind: "holdings", section: "paid", after: 42 });
    expect(store.holdingsAsk()).toEqual({ kind: "holdings", section: "paid", after: 42 });
    expect(store.pageFirst()).toEqual({ kind: "holdings", section: "paid", after: null });
  });
});
