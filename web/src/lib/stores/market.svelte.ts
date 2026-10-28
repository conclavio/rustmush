// The market screen's store (17g cut C3c; the C3 plan §2.5): the two keyframes
// decoded closed (`$lib/market`), the cursors the shown pages were asked with,
// the holdings section, the selected rows, the one open form, the one pending
// market request and the screen's status sentence. It owns no transport:
// `GameApp` asks the pages (`requestMarket`, `enterMarket`) and sends the one
// typed request (`submitMarket`); this store says what to ask and what a
// reply means.
//
// The web twin of the TUI's `MarketDesk` (`mush-net-ssh/src/tui/market.rs`,
// 17g cut C3b): the same forms, the same defaults, the same sentences and the
// same reply contract — `staged` closes the form and asks the two pages once,
// `shown` paints the lines inside the open accept form, `error` closes it
// with the server's message, and a reply this client never saw is never a
// refusal (`LOST`). One market request at a time; a bare key opens a form or
// moves a page and sends nothing.

import {
  HOLDINGS_FEED,
  LOST,
  MARKET_FEED,
  NOT_LISTABLE,
  NOT_OFFERABLE,
  NOT_RENEWABLE,
  NOT_WITHDRAWABLE,
  NO_NUMBER,
  NO_PAGE,
  NO_ROW,
  LAST_PAGE,
  PENDING,
  UNEXPECTED,
  WAITING,
  decodeHoldingsView,
  decodeMarketView,
  holdingNumber,
  marketActionError,
  type HoldingRow,
  type HoldingsView,
  type MarketView,
} from "../market";
import type {
  MarketAction,
  MarketListingForm,
  MindHoldingsSection,
  MindMarketRequest,
  MindResponse,
} from "../proto";

/** Which market pane a page key acts on: the one last clicked. */
export type MarketPane = "listings" | "holdings";

/** The purchase's four steps, in the TUI's order. */
export const BUY_STEPS = ["quote", "pay", "complete", "refund"] as const;
export type BuyStep = (typeof BUY_STEPS)[number];
/** The renewal's three steps (17g C2h: each hands the step to the service). */
export const RENEW_STEPS = ["request", "pay", "decline"] as const;
export type RenewStep = (typeof RENEW_STEPS)[number];
/** What an offer or an accept names: a paid licence or a filed mind's charter. */
export const OFFER_KINDS = ["licence", "filed"] as const;
export type OfferKind = (typeof OFFER_KINDS)[number];
/** The listing's three forms: the wire word and the word the screen shows. */
export const LISTING_FORMS: readonly { value: MarketListingForm; label: string }[] = [
  { value: "sealed", label: "sealed" },
  { value: "open", label: "open" },
  { value: "open_resale", label: "open-resale" },
];

/** The one open form (the TUI's `Form`, field for field). */
export type MarketForm =
  /** `b`: the four purchase steps of one listing. */
  | { kind: "buy"; code: string; name: string; step: BuyStep; confirm: string[] }
  /** `k`: the three renewal steps of one paid licence. */
  | { kind: "renew"; licence: number; name: string; step: RenewStep }
  /** `t`: a resale offer (`licence`) or a charter offer (`filed`). */
  | { kind: "offer"; target: OfferKind; number: number; name: string; player: string; price: string }
  /** `a`: accept an offer, or list the offers that name you with an empty price. */
  | { kind: "accept"; target: OfferKind; number: string; price: string; shown: string[] }
  /** `l`: list a filed mind. */
  | { kind: "publish"; filed: number; name: string; form: MarketListingForm; royalty: string; transfer: boolean }
  /** `x`: the withdrawal's confirm. */
  | { kind: "withdraw"; filed: number; name: string }
  /** Read-only lines (the offer's closing instruction, an unexpected `shown`). */
  | { kind: "notice"; title: string; lines: string[] };

/** The form's title (the TUI's titles, verbatim). */
export function formTitle(form: MarketForm): string {
  switch (form.kind) {
    case "buy":
      // A code the own strip names, off this page, has no name.
      return form.name === "" ? `Buy listing ${form.code}` : `Buy listing ${form.code}: ${form.name}`;
    case "renew":
      return `Renew licence ${form.licence}: ${form.name}`;
    case "offer":
      return form.target === "licence"
        ? `Offer licence ${form.number}: ${form.name}`
        : `Offer filed mind ${form.number} and its charter: ${form.name}`;
    case "accept":
      return "Accept an offer";
    case "publish":
      return `List filed mind ${form.filed}: ${form.name}`;
    case "withdraw":
      return `Withdraw filed mind ${form.filed}`;
    case "notice":
      return form.title;
  }
}

/** The withdrawal's confirm sentence (the TUI's, verbatim). */
export function withdrawText(filed: number, name: string): string {
  return `Withdraw filed mind #${filed} ${name}? New sales stop; licences already sold keep working. Enter withdraws; Esc cancels.`;
}

/** The seller's instruction after a staged offer, from the typed fields (the
 * C3 plan §2.7: `RESELL_TELL` / `CHARTER_TELL` are not carried). */
export function offerClosing(kind: OfferKind, number: number, price: string): string {
  return `They accept with: a, ${kind} ${number}, price ${price}.`;
}

/** Drop control characters from server text the form repeats. */
const clean = (s: string): string => s.replace(/[\u0000-\u001f\u007f-\u009f]/g, "");

export class MarketStore {
  /** The listings page shown (`null`: none yet, or a bad shape). */
  market = $state<MarketView | null>(null);
  /** The last `mind.market` answer was a bad shape or a refusal. */
  marketBad = $state(false);
  /** The holdings page last shown for each section. */
  holdings = $state<Record<MindHoldingsSection, HoldingsView | null>>({ owner: null, paid: null });
  /** The last `mind.holdings` answer was a bad shape or a refusal. */
  holdingsBad = $state(false);
  /** The cursor the shown listings page was asked with (`null`: the first). */
  listingsAt = $state<string | null>(null);
  /** The cursor each section's shown page was asked with. */
  holdingsAt = $state<Record<MindHoldingsSection, number | null>>({ owner: null, paid: null });
  /** The section asked for last, or shown by the last keyframe. */
  section = $state<MindHoldingsSection>("owner");
  /** The highlighted listing code (a page row or an own-strip line). */
  selected = $state<string | null>(null);
  /** The highlighted holding's number (a filed number or a licence number). */
  selectedHolding = $state<number | null>(null);
  /** The pane `]` and `[` page. */
  focus = $state<MarketPane>("listings");
  /** The screen's status sentence: the server's `line`, `message` or first
   * shown line, or one of the client's own short sentences. */
  notice = $state("");
  /** Whether the one market request is out. */
  pending = $state(false);
  /** The one open form. */
  form = $state<MarketForm | null>(null);
  /** The pending request's action (the offer's closing reads it). */
  #sent: MarketAction | null = null;
  /** Bumped on every reset, so a reply to a forgotten request is ignored. */
  #generation = 0;

  /** Whether the shown listings page says the market is open here. */
  get open(): boolean {
    return this.market?.open ?? false;
  }

  /** The holdings page of the section asked for. */
  get shownHoldings(): HoldingsView | null {
    return this.holdings[this.section];
  }

  /** The market tab's badge: 1 while a live quote or a waiting purchase
   * exists (the C3 plan §7). */
  get badge(): number {
    const own = this.market?.own;
    if (!own) return 0;
    return own.quotes.some((q) => !q.expired) || own.waiting.length > 0 ? 1 : 0;
  }

  /** The reset counter a caller holds across an await. */
  get generation(): number {
    return this.#generation;
  }

  /**
   * A `mind.market` or `mind.holdings` keyframe arrived: decode it closed. A
   * bad shape is held as none (its pane paints `NO_PAGE`), so no form opens on
   * it. Returns whether it decoded.
   */
  apply(feed: string, data: unknown): boolean {
    if (feed === MARKET_FEED) {
      const view = decodeMarketView(data);
      this.market = view;
      this.marketBad = view === null;
      if (view !== null) {
        const codes = [...view.rows.map((r) => r.code), ...view.own.quotes.map((q) => q.code), ...view.own.waiting];
        if (this.selected === null || !codes.includes(this.selected)) {
          this.selected = codes[0] ?? null;
        }
      }
      return view !== null;
    }
    if (feed === HOLDINGS_FEED) {
      const view = decodeHoldingsView(data);
      this.holdingsBad = view === null;
      if (view !== null) {
        this.holdings = { ...this.holdings, [view.section]: view };
        this.section = view.section;
        const numbers = view.rows.map(holdingNumber);
        if (this.selectedHolding === null || !numbers.includes(this.selectedHolding)) {
          this.selectedHolding = numbers[0] ?? null;
        }
      } else {
        this.holdings = { ...this.holdings, [this.section]: null };
      }
      return view !== null;
    }
    return false;
  }

  /** A page ask was answered with a refusal (`No such market page.`): the
   * pane paints the sentence and the status says it. */
  refused(feed: string, sentence: string): void {
    if (feed === MARKET_FEED) {
      this.market = null;
      this.marketBad = true;
    } else if (feed === HOLDINGS_FEED) {
      this.holdings = { ...this.holdings, [this.section]: null };
      this.holdingsBad = true;
    }
    this.notice = sentence || NO_PAGE;
  }

  /** A page was asked (by `GameApp.requestMarket`): remember the cursor and
   * the section it names, so a staged reply re-asks the pages actually shown. */
  noteAsk(req: MindMarketRequest): void {
    if (req.kind === "listings") {
      this.listingsAt = req.after;
    } else {
      this.section = req.section;
      this.holdingsAt = { ...this.holdingsAt, [req.section]: req.after };
    }
  }

  /** The listings page the screen is on. */
  listingsAsk(): MindMarketRequest {
    return { kind: "listings", after: this.listingsAt };
  }

  /** The holdings page the screen is on. */
  holdingsAsk(): MindMarketRequest {
    return { kind: "holdings", section: this.section, after: this.holdingsAt[this.section] };
  }

  /** `h`: the other section, from its first page. The focus stays where it was (the TUI's
   * `MarketSection` moves no pane focus either); only a click in a pane moves it. */
  flipSection(): MindMarketRequest {
    this.section = this.section === "owner" ? "paid" : "owner";
    this.holdingsAt = { ...this.holdingsAt, [this.section]: null };
    return this.holdingsAsk();
  }

  /** `]`: the focused pane's next page, or `null` with the status saying why. */
  pageNext(): MindMarketRequest | null {
    if (this.focus === "listings") {
      if (this.market === null) return this.#say(NO_PAGE);
      if (this.market.after === null) return this.#say(LAST_PAGE);
      return { kind: "listings", after: this.market.after };
    }
    const view = this.shownHoldings;
    if (view === null) return this.#say(NO_PAGE);
    if (view.after === null) return this.#say(LAST_PAGE);
    return { kind: "holdings", section: this.section, after: view.after };
  }

  /** `[`: the focused pane's first page. */
  pageFirst(): MindMarketRequest {
    return this.focus === "listings"
      ? { kind: "listings", after: null }
      : { kind: "holdings", section: this.section, after: null };
  }

  #say(sentence: string): null {
    this.notice = sentence;
    return null;
  }

  /** The one-pending rule holds at the keys too (C3b): a form opened while a
   * request is out could not send and would be closed by the earlier reply. */
  #free(): boolean {
    if (this.pending) {
      this.notice = PENDING;
      return false;
    }
    return true;
  }

  #holding(): HoldingRow | string {
    const view = this.shownHoldings;
    if (view === null) return NO_PAGE;
    const row = view.rows.find((h) => holdingNumber(h) === this.selectedHolding);
    return row ?? NO_ROW;
  }

  #open(form: MarketForm): void {
    this.form = form;
  }

  /** `b`: the purchase form over the selected code. The default step follows
   * the actor's own state: a waiting purchase completes, a live quote pays,
   * no quote or an expired one asks for a quote. */
  openBuy(): void {
    if (!this.#free()) return;
    const view = this.market;
    if (view === null) {
      this.notice = NO_PAGE;
      return;
    }
    const code = this.selected;
    if (code === null) {
      this.notice = NO_ROW;
      return;
    }
    const row = view.rows.find((r) => r.code === code);
    const quote = view.own.quotes.find((q) => q.code === code);
    const waiting = view.own.waiting.includes(code);
    // A code off this page is still the actor's own when the own strip names
    // it (the C3b review): an escrowed purchase must stay reachable.
    if (row === undefined && quote === undefined && !waiting) {
      this.notice = NO_ROW;
      return;
    }
    const step: BuyStep = waiting ? "complete" : quote !== undefined && !quote.expired ? "pay" : "quote";
    const confirm = row ? [clean(row.termsLine), clean(row.splitLine)] : [];
    if (quote !== undefined) {
      confirm.push(
        quote.expired
          ? "Your quote has expired."
          : `Your quote: ${clean(quote.priceText)}, ${quote.expiresInSeconds}s left.`,
      );
    }
    this.#open({ kind: "buy", code, name: row ? clean(row.name) : "", step, confirm });
  }

  /** `k`: the renewal form over the selected paid licence (`offered` → pay). */
  openRenew(): void {
    if (!this.#free()) return;
    const row = this.#holding();
    if (typeof row === "string") {
      this.notice = row;
      return;
    }
    if (row.kind !== "paid" || !row.actions.some((a) => a === "renew" || a === "renew_pay" || a === "renew_decline")) {
      this.notice = NOT_RENEWABLE;
      return;
    }
    this.#open({
      kind: "renew",
      licence: row.licence,
      name: clean(row.name),
      step: row.renewal === "offered" ? "pay" : "request",
    });
  }

  /** `t`: a resale offer over a paid licence, or a charter offer over a
   * filed mind, as the row's actions allow. */
  openOffer(): void {
    if (!this.#free()) return;
    const row = this.#holding();
    if (typeof row === "string") {
      this.notice = row;
      return;
    }
    let target: OfferKind;
    if (row.kind === "paid" && row.actions.includes("resell")) {
      target = "licence";
    } else if (row.kind === "owner" && row.actions.includes("charter")) {
      target = "filed";
    } else {
      this.notice = NOT_OFFERABLE;
      return;
    }
    this.#open({ kind: "offer", target, number: holdingNumber(row), name: clean(row.name), player: "", price: "" });
  }

  /** `a`: accept an offer by the number the seller named. */
  openAccept(): void {
    if (!this.#free()) return;
    this.#open({ kind: "accept", target: "licence", number: "", price: "", shown: [] });
  }

  /** `l`: list the selected filed mind. */
  openPublish(): void {
    if (!this.#free()) return;
    const row = this.#holding();
    if (typeof row === "string") {
      this.notice = row;
      return;
    }
    if (row.kind !== "owner" || !row.actions.includes("publish")) {
      this.notice = NOT_LISTABLE;
      return;
    }
    this.#open({ kind: "publish", filed: row.filed, name: clean(row.name), form: "sealed", royalty: "", transfer: true });
  }

  /** `x`: the withdrawal's confirm over the selected filed mind. */
  openWithdraw(): void {
    if (!this.#free()) return;
    const row = this.#holding();
    if (typeof row === "string") {
      this.notice = row;
      return;
    }
    if (row.kind !== "owner" || !row.actions.includes("withdraw")) {
      this.notice = NOT_WITHDRAWABLE;
      return;
    }
    this.#open({ kind: "withdraw", filed: row.filed, name: clean(row.name) });
  }

  /** Esc: close the form. It never cancels a request that is out. */
  closeForm(): void {
    this.form = null;
  }

  /**
   * The open form's one typed action, or the sentence that refuses it (the
   * TUI's `enter`). A notice has no action: `null` closes it.
   */
  build(): MarketAction | string | null {
    const form = this.form;
    if (form === null || form.kind === "notice") {
      this.form = null;
      return null;
    }
    let action: MarketAction;
    switch (form.kind) {
      case "buy":
        action =
          form.step === "pay"
            ? { act: "pay", code: form.code }
            : form.step === "complete"
              ? { act: "complete", code: form.code }
              : form.step === "refund"
                ? { act: "refund", code: form.code }
                : { act: "quote", code: form.code };
        break;
      case "renew":
        action =
          form.step === "pay"
            ? { act: "renew_pay", licence: form.licence }
            : form.step === "decline"
              ? { act: "renew_decline", licence: form.licence }
              : { act: "renew", licence: form.licence };
        break;
      case "offer": {
        const player = form.player.trim();
        const price = form.price.trim();
        action =
          form.target === "licence"
            ? { act: "resell", licence: form.number, player, price }
            : { act: "charter", filed: form.number, player, price };
        break;
      }
      case "accept": {
        const typed = form.number.trim();
        const number = /^[0-9]{1,16}$/.test(typed) ? Number(typed) : NaN;
        if (!Number.isSafeInteger(number)) {
          return NO_NUMBER;
        }
        const price = form.price.trim() === "" ? null : form.price.trim();
        action =
          form.target === "filed"
            ? { act: "charter_accept", filed: number, price }
            : { act: "resell_accept", licence: number, price };
        break;
      }
      case "publish": {
        const typed = form.royalty.trim();
        let royalty: number | null = null;
        if (typed !== "") {
          // Past a u16 is past the 300 bound: `validate`'s refusal.
          if (!/^[0-9]{1,5}$/.test(typed) || Number(typed) > 65535) {
            return "Request refused: invalid market royalty";
          }
          royalty = Number(typed);
        }
        // Always `Some(bool)`, so the request names what the player saw (C3b).
        action = { act: "publish", filed: form.filed, form: form.form, royalty, transfer: form.transfer };
        break;
      }
      case "withdraw":
        action = { act: "withdraw", filed: form.filed };
        break;
    }
    const error = marketActionError(action);
    return error === null ? action : `Request refused: ${error}`;
  }

  /** The one market request leaves: the store is pending until its reply,
   * and the returned generation tells the caller whether a reset intervened. */
  begin(action: MarketAction): number {
    this.pending = true;
    this.#sent = action;
    this.notice = WAITING;
    return this.#generation;
  }

  /** Close the open form if it is a market form (never another surface's). */
  #closeMarketForm(): void {
    this.form = null;
  }

  /**
   * The pending request's reply. `staged`: the status is `line`, the form
   * closes (an offer opens its closing instruction), and the caller asks the
   * two pages once. `shown`: the status is the first line and the lines paint
   * inside the open accept form. `error`: the status is `message` and the form
   * closes. Returns which it was.
   */
  reply(response: MindResponse): "staged" | "shown" | "error" | "unexpected" {
    const sent = this.#sent;
    this.pending = false;
    this.#sent = null;
    switch (response.status) {
      case "staged": {
        this.notice = clean(response.line);
        this.#closeMarketForm();
        if (sent !== null && (sent.act === "resell" || sent.act === "charter")) {
          const kind: OfferKind = sent.act === "resell" ? "licence" : "filed";
          const number = sent.act === "resell" ? sent.licence : sent.filed;
          this.#open({ kind: "notice", title: "Offer sent", lines: [offerClosing(kind, number, sent.price)] });
        }
        return "staged";
      }
      case "shown": {
        const lines = response.lines.map(clean);
        this.notice = lines[0] ?? "";
        if (this.form?.kind === "accept") {
          this.form = { ...this.form, shown: lines };
        } else {
          this.#open({ kind: "notice", title: "Offers", lines });
        }
        return "shown";
      }
      case "error":
        this.notice = clean(response.message);
        this.#closeMarketForm();
        return "error";
      default:
        this.notice = UNEXPECTED;
        this.#closeMarketForm();
        return "unexpected";
    }
  }

  /** The request never left this client (no socket, or the mind requests'
   * pending cap was full): no result of it exists, so this is no `LOST`. The
   * store is free again, the form stays open to send later, and the status
   * says why it was not sent. */
  unsent(sentence: string): void {
    this.pending = false;
    this.#sent = null;
    this.notice = sentence;
  }

  /** The reply was never seen (a timeout, a dropped connection): the form
   * closes with C2's "not a refusal" sentence and nothing retries. */
  lost(): void {
    this.pending = false;
    this.#sent = null;
    this.form = null;
    this.notice = LOST;
  }

  /** Forget everything on a connection or character boundary — the views,
   * the cursors, the form and the pending request. A request that was out
   * leaves `LOST` behind: its result will never be seen here. `LOST` is
   * STICKY across further resets — a drop runs one (`onStatus`) and the hello
   * after the reconnect runs another, and the second must not wipe the only
   * warning that a purchase may have been funded. It goes when the next
   * sentence replaces it (the player's next key or request), or on an
   * `account` boundary (a logout), where it is no longer the viewer's. */
  reset(account = false): void {
    const lost = !account && (this.pending || this.notice === LOST);
    this.#generation++;
    this.market = null;
    this.marketBad = false;
    this.holdings = { owner: null, paid: null };
    this.holdingsBad = false;
    this.listingsAt = null;
    this.holdingsAt = { owner: null, paid: null };
    this.section = "owner";
    this.selected = null;
    this.selectedHolding = null;
    this.focus = "listings";
    this.form = null;
    this.pending = false;
    this.#sent = null;
    this.notice = lost ? LOST : "";
  }
}
