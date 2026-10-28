// The player mind market on the web (17g cut C3c; the C3 plan §2.5): the two
// panel feeds 17g cut C3a answers — `mind.market` (one page of the counter's
// listings, with the actor's own quotes and waiting purchases) and
// `mind.holdings` (one section of the actor's own filed minds and licences) —
// decoded CLOSED, on `mind-bulk.ts` `decodeProvingView`'s pattern: the schema
// id and version are checked, every field the screen reads is typed, a price
// in minor units is a string of digits, a listing code is exactly the eight
// characters `+minds` prints, unknown keys are ignored and an unknown
// `actions` word is dropped. Anything else is a bad shape and decodes to
// `null`: the screen then paints `No such market page.` and opens no form.
//
// This is the hand-kept twin of the TUI's `mush-net-ssh/src/tui/market.rs`
// (`decode_market`, `decode_holdings`, the desk's sentences): the two clients
// read the same keyframes and say the same things.
//
// Nothing here builds command text (17g §3 C3, D-I-18): a market mutation is
// one typed `MarketAction` (`GameApp.submitMarket`), and what it carries is
// only what a player types — a code, a number, a name, a price in the line's
// `price=` spelling, a form, a royalty, the author's transfer mark. The server
// derives every other parameter from the actor's own holdings.

import type { MarketAction, MindHoldingsSection } from "./proto";

/** The listings feed and the query of the same name (17g C3a
 * `tacsim::minds::market_commands::query::MARKET_FEED`). */
export const MARKET_FEED = "mind.market";
/** The holdings feed and the query of the same name (17g C3a). */
export const HOLDINGS_FEED = "mind.holdings";
/** The schema version both keyframes carry (17g C3a `SCHEMA_VERSION`). */
export const MARKET_SCHEMA_VERSION = 1;
/** The most rows of one keyframe kept per pane (the C3 plan §7, the cap
 * `parseActions` keeps; a server page is 20). */
export const MAX_MARKET_ROWS = 128;

// ---- the client's sentences: the TUI desk's, verbatim ----------------------

/** What a pane shows for a keyframe of the wrong shape: the server's own
 * sentence for a page it would not build (17g C3a `NO_PAGE`). */
export const NO_PAGE = "No such market page.";
/** The one-pending rule's refusal (the workshop's sentence). */
export const PENDING = "An operation is pending; wait for its result.";
/** While a request is out. */
export const WAITING = "Waiting for the server...";
/** 17g C2's sentence for a reply the client never saw (a timeout, a dropped
 * connection, a character switch): never a refusal, never a refund. */
export const LOST =
  "A request whose result you have not seen is not a refusal; check your holdings before trying again.";
/** `k` on a holding with no renewal action. */
export const NOT_RENEWABLE = "This licence is not renewable now.";
/** `t` on a holding with neither `resell` nor `charter`. */
export const NOT_OFFERABLE = "This holding cannot be offered now.";
/** `l` on a holding without `publish`. */
export const NOT_LISTABLE = "This filed mind cannot be listed now.";
/** `x` on a holding without `withdraw`. */
export const NOT_WITHDRAWABLE = "This filed mind cannot be withdrawn now.";
/** A row key was asked of a pane with no row selected. */
export const NO_ROW = "Select a row first.";
/** `]` on the last page. */
export const LAST_PAGE = "This is the last page.";
/** The accept form's number is empty or not a number. */
export const NO_NUMBER = "Enter the number the seller named.";
/** A reply of a kind the request cannot have (never from a conforming server). */
export const UNEXPECTED = "Unexpected response; refresh the market.";
/** The renew form's note: the renewal service answers on the ordinary output
 * (17g C2h; the C3 plan §0a item 1). */
export const RENEW_NOTE =
  "The renewal service answers on the scene screen; read the price there before you pay.";
/** The accept form's hint. */
export const ACCEPT_HINT = "An empty price lists the offers that name you.";
/** The form's foot while its request is out: closing it never cancels, and
 * "cancels" there would read as a refund (the common rule). */
export const SENT_FOOT = "Sent; Esc closes this window but does not cancel the request.";
/** The form's foot before it is sent. */
export const FORM_FOOT = "Enter sends this request; Esc cancels. Tab moves between fields; Left/Right chooses.";
/** The withdrawal's foot. */
export const WITHDRAW_FOOT = "Enter withdraws; Esc cancels.";
/** A read-only notice's foot. */
export const NOTICE_FOOT = "Enter or Esc closes.";

// ---- the web's own sentences (the TUI desk has none of these) --------------

/** A submit with no socket to send it on: nothing left this client, so this is never `LOST`
 * (the web's own "Not connected — … dropped." voice, `game.svelte.ts`). */
export const OFFLINE = "Not connected — request dropped.";
/** A pane no page has reached yet (before the entry asks are answered, or after a reset). */
export const NO_PAGE_YET = "No page yet: r asks the server.";
/** A holdings section with no rows and no notice. */
export const EMPTY_SECTION = "Nothing in this section.";
/** Under the FOCUSED pane when its page has a next one: `]` pages the focused pane only, so
 * the other pane never says this. */
export const MORE_PAGES = "More: ] shows the next page.";

// ---- the decoded views ------------------------------------------------------

/** One listing of a `mind.market` page (17g C3a `listing_row`). The screen
 * paints `priceText` only; `priceMinor` is kept typed and never shown. */
export interface MarketRow {
  code: string;
  name: string;
  class: string;
  form: string;
  transferable: boolean;
  derived: boolean;
  termSeconds: number;
  graceSeconds: number;
  priceMinor: string;
  priceText: string;
  levyPermille: number;
  venuePermille: number;
  parentRoyaltyPermille: number;
  termsLine: string;
  splitLine: string;
}

/** One of the actor's own quotes (the `own.quotes` strip). */
export interface OwnQuote {
  code: string;
  priceMinor: string;
  priceText: string;
  expiresInSeconds: number;
  expired: boolean;
}

/** A `mind.market` keyframe. */
export interface MarketView {
  open: boolean;
  notice: string | null;
  rows: MarketRow[];
  /** The next page's cursor, passed back as `after`; `null` on the last page. */
  after: string | null;
  own: { quotes: OwnQuote[]; waiting: string[] };
}

/** An owner licence's or a paid licence's phase; `daysLeft` is `null` only
 * when lapsed. */
export interface HoldingPhase {
  state: "active" | "grace" | "lapsed";
  daysLeft: number | null;
}

/** The actions the server offers on an owner row (17g C3a `owner_actions`). */
export type OwnerAction = "publish" | "withdraw" | "charter";
/** The actions the server offers on a paid row (17g C3a `paid_actions`). */
export type PaidAction = "renew" | "renew_pay" | "renew_decline" | "resell";

const OWNER_ACTIONS: readonly OwnerAction[] = ["publish", "withdraw", "charter"];
const PAID_ACTIONS: readonly PaidAction[] = ["renew", "renew_pay", "renew_decline", "resell"];

/** One row of a `mind.holdings` page. */
export type HoldingRow =
  | {
      kind: "owner";
      filed: number;
      name: string;
      dependent: boolean;
      phase: HoldingPhase;
      selling: boolean;
      actions: OwnerAction[];
    }
  | {
      kind: "paid";
      licence: number;
      house: boolean;
      name: string;
      form: string;
      transferable: boolean;
      phase: HoldingPhase;
      revoked: boolean;
      selling: boolean;
      renewal: "offered" | "paid" | null;
      actions: PaidAction[];
    };

/** A `mind.holdings` keyframe. */
export interface HoldingsView {
  open: boolean;
  notice: string | null;
  section: MindHoldingsSection;
  rows: HoldingRow[];
  /** The next page's cursor (a charter asset id or a licence number), passed
   * back as `after`; `null` on the last page. */
  after: number | null;
}

/** A holding row's number: the filed number or the licence number. */
export function holdingNumber(row: HoldingRow): number {
  return row.kind === "owner" ? row.filed : row.licence;
}

// ---- the closed decoders ---------------------------------------------------

const object = (value: unknown): value is Record<string, unknown> =>
  value !== null && typeof value === "object" && !Array.isArray(value);
const natural = (value: unknown): value is number => Number.isSafeInteger(value) && Number(value) >= 0;
const integer = (value: unknown): value is number => Number.isSafeInteger(value);
const text = (value: unknown): value is string => typeof value === "string";
const flag = (value: unknown): value is boolean => typeof value === "boolean";
/** A price in minor units: a non-empty string of digits (never a JSON number,
 * which would round past 2^53). */
const digits = (value: unknown): value is string => typeof value === "string" && /^[0-9]+$/.test(value);
const nullableText = (value: unknown): value is string | null => value === null || typeof value === "string";

/** Exactly the 8 characters of a listing code: digits and the letters a to f
 * (17g C3a `ListingCode`, `mush_market::mind_commands::is_code`). */
export function isListingCode(value: unknown): value is string {
  return typeof value === "string" && /^[0-9a-f]{8}$/.test(value);
}

function schemaIs(value: Record<string, unknown>, id: string): boolean {
  return object(value.schema) && value.schema.id === id && value.schema.version === MARKET_SCHEMA_VERSION;
}

function decodeRow(row: unknown): MarketRow | null {
  if (
    !object(row) ||
    !isListingCode(row.code) ||
    !text(row.name) ||
    !text(row.class) ||
    !text(row.form) ||
    !flag(row.transferable) ||
    !flag(row.derived) ||
    !integer(row.term_seconds) ||
    !integer(row.grace_seconds) ||
    !digits(row.price_minor) ||
    !text(row.price_text) ||
    !natural(row.levy_permille) ||
    !natural(row.venue_permille) ||
    !natural(row.parent_royalty_permille) ||
    !text(row.terms_line) ||
    !text(row.split_line)
  ) {
    return null;
  }
  return {
    code: row.code,
    name: row.name,
    class: row.class,
    form: row.form,
    transferable: row.transferable,
    derived: row.derived,
    termSeconds: row.term_seconds,
    graceSeconds: row.grace_seconds,
    priceMinor: row.price_minor,
    priceText: row.price_text,
    levyPermille: row.levy_permille,
    venuePermille: row.venue_permille,
    parentRoyaltyPermille: row.parent_royalty_permille,
    termsLine: row.terms_line,
    splitLine: row.split_line,
  };
}

function decodeQuote(quote: unknown): OwnQuote | null {
  if (
    !object(quote) ||
    !isListingCode(quote.code) ||
    !digits(quote.price_minor) ||
    !text(quote.price_text) ||
    !natural(quote.expires_in_seconds) ||
    !flag(quote.expired)
  ) {
    return null;
  }
  return {
    code: quote.code,
    priceMinor: quote.price_minor,
    priceText: quote.price_text,
    expiresInSeconds: quote.expires_in_seconds,
    expired: quote.expired,
  };
}

/** Every element of `list` through `decode`, at most `MAX_MARKET_ROWS`;
 * `null` if any one is a bad shape. */
function decodeList<T>(list: unknown, decode: (item: unknown) => T | null): T[] | null {
  if (!Array.isArray(list)) {
    return null;
  }
  const out: T[] = [];
  for (const item of list.slice(0, MAX_MARKET_ROWS)) {
    const decoded = decode(item);
    if (decoded === null) {
      return null;
    }
    out.push(decoded);
  }
  return out;
}

/** Decode a `mind.market` keyframe closed; `null` is a bad shape. */
export function decodeMarketView(data: unknown): MarketView | null {
  if (!object(data) || !schemaIs(data, MARKET_FEED) || !flag(data.open) || !nullableText(data.notice)) {
    return null;
  }
  if (data.after !== null && !isListingCode(data.after)) {
    return null;
  }
  const rows = decodeList(data.rows, decodeRow);
  if (rows === null || !object(data.own)) {
    return null;
  }
  const quotes = decodeList(data.own.quotes, decodeQuote);
  const waiting = decodeList(data.own.waiting, (w) => (object(w) && isListingCode(w.code) ? w.code : null));
  if (quotes === null || waiting === null) {
    return null;
  }
  return { open: data.open, notice: data.notice, rows, after: data.after, own: { quotes, waiting } };
}

function decodePhase(phase: unknown): HoldingPhase | null {
  if (!object(phase)) {
    return null;
  }
  switch (phase.state) {
    case "active":
    case "grace":
      return natural(phase.days_left) ? { state: phase.state, daysLeft: phase.days_left } : null;
    case "lapsed":
      return phase.days_left === null || phase.days_left === undefined
        ? { state: "lapsed", daysLeft: null }
        : null;
    default:
      return null;
  }
}

/** The row kind's own action words; an entry that is not a string is a bad
 * shape (`null`), an unknown word — or one of the other kind's — is dropped. */
function decodeActions<A extends string>(value: unknown, vocabulary: readonly A[]): A[] | null {
  if (!Array.isArray(value) || value.length > MAX_MARKET_ROWS) {
    return null;
  }
  const out: A[] = [];
  for (const word of value) {
    if (typeof word !== "string") {
      return null;
    }
    const known = vocabulary.find((v) => v === word);
    if (known !== undefined && !out.includes(known)) {
      out.push(known);
    }
  }
  return out;
}

function decodeHolding(row: unknown): HoldingRow | null {
  if (!object(row) || !text(row.name) || !flag(row.selling)) {
    return null;
  }
  const phase = decodePhase(row.phase);
  if (phase === null) {
    return null;
  }
  if (row.kind === "owner") {
    const actions = decodeActions(row.actions, OWNER_ACTIONS);
    if (!natural(row.filed) || !flag(row.dependent) || actions === null) {
      return null;
    }
    return {
      kind: "owner",
      filed: row.filed,
      name: row.name,
      dependent: row.dependent,
      phase,
      selling: row.selling,
      actions,
    };
  }
  if (row.kind === "paid") {
    const actions = decodeActions(row.actions, PAID_ACTIONS);
    const renewal = row.renewal;
    if (
      !natural(row.licence) ||
      !flag(row.house) ||
      !text(row.form) ||
      !flag(row.transferable) ||
      !flag(row.revoked) ||
      !(renewal === null || renewal === "offered" || renewal === "paid") ||
      actions === null
    ) {
      return null;
    }
    return {
      kind: "paid",
      licence: row.licence,
      house: row.house,
      name: row.name,
      form: row.form,
      transferable: row.transferable,
      phase,
      revoked: row.revoked,
      selling: row.selling,
      renewal,
      actions,
    };
  }
  return null;
}

/** Decode a `mind.holdings` keyframe closed; `null` is a bad shape. */
export function decodeHoldingsView(data: unknown): HoldingsView | null {
  if (!object(data) || !schemaIs(data, HOLDINGS_FEED) || !flag(data.open) || !nullableText(data.notice)) {
    return null;
  }
  if (data.section !== "owner" && data.section !== "paid") {
    return null;
  }
  if (data.after !== null && !natural(data.after)) {
    return null;
  }
  const rows = decodeList(data.rows, decodeHolding);
  if (rows === null) {
    return null;
  }
  return { open: data.open, notice: data.notice, section: data.section, rows, after: data.after };
}

// ---- what the screen paints -------------------------------------------------

/** Whole days, rounded up (the TUI's TERM+GRACE cell). */
function days(seconds: number): number {
  return Math.ceil(Math.max(0, seconds) / 86400);
}

/** A listing's TERM+GRACE cell, `{term days}+{grace days}`. */
export function termCell(row: MarketRow): string {
  return `${days(row.termSeconds)}+${days(row.graceSeconds)}`;
}

/** A listing's FLAGS cell: `no resale`, `derived`, or both. */
export function listingFlags(row: MarketRow): string {
  return [row.transferable ? null : "no resale", row.derived ? "derived" : null]
    .filter((f): f is string => f !== null)
    .join(", ");
}

/** A holding's KIND cell: `owner` / `dependent` / `paid` / `House Pin`. */
export function holdingKind(row: HoldingRow): string {
  if (row.kind === "owner") {
    return row.dependent ? "dependent" : "owner";
  }
  return row.house ? "House Pin" : "paid";
}

/** A holding's PHASE cell: `Active, {n}d left` / `Grace, {n}d left` / `Lapsed`. */
export function phaseCell(phase: HoldingPhase): string {
  switch (phase.state) {
    case "active":
      return `Active, ${phase.daysLeft ?? 0}d left`;
    case "grace":
      return `Grace, ${phase.daysLeft ?? 0}d left`;
    default:
      return "Lapsed";
  }
}

/** A holding's FLAGS cell: owner `selling`; paid `no resale, revoked, selling,
 * renew offer, renew paid` (whichever hold). */
export function holdingFlags(row: HoldingRow): string {
  if (row.kind === "owner") {
    return row.selling ? "selling" : "";
  }
  return [
    row.transferable ? null : "no resale",
    row.revoked ? "revoked" : null,
    row.selling ? "selling" : null,
    row.renewal === "offered" ? "renew offer" : null,
    row.renewal === "paid" ? "renew paid" : null,
  ]
    .filter((f): f is string => f !== null)
    .join(", ");
}

/** One own-strip line: `quote {code} {price} {n}s` (or `expired`), or
 * `waiting {code}`. */
export function ownStrip(view: MarketView): string[] {
  return [
    ...view.own.quotes.map((q) =>
      q.expired ? `quote ${q.code} ${q.priceText} expired` : `quote ${q.code} ${q.priceText} ${q.expiresInSeconds}s`,
    ),
    ...view.own.waiting.map((code) => `waiting ${code}`),
  ];
}

// ---- the typed request's own bounds ----------------------------------------

/** The longest `player` a typed trade names (characters; mush-proto
 * `MAX_MARKET_PLAYER_CHARS`). */
export const MAX_MARKET_PLAYER_CHARS = 32;
/** The longest `price` (bytes; `MAX_MARKET_PRICE_BYTES`). */
export const MAX_MARKET_PRICE_BYTES = 16;
/** The largest parent royalty (per mille; `MAX_MARKET_ROYALTY_PERMILLE`). */
export const MAX_MARKET_ROYALTY_PERMILLE = 300;

/**
 * The bounds mush-proto's `InvokeRequest::validate` checks on a market action,
 * with its own words (`invalid market player`, `invalid market price`,
 * `invalid market royalty`, and a code or number the wire would refuse):
 * no state is consulted, and the grammar and the meaning stay the server's.
 * `null` when the action passes. The gateway refuses the same action before
 * forwarding it; checking here first only saves the round trip and names why.
 */
export function marketActionError(action: MarketAction): string | null {
  const player = (name: string): boolean =>
    name.length > 0 && [...name].length <= MAX_MARKET_PLAYER_CHARS && !/[\u0000-\u001f\u007f-\u009f]/.test(name);
  const price = (amount: string): boolean =>
    amount.length > 0 && new TextEncoder().encode(amount).length <= MAX_MARKET_PRICE_BYTES && /^[0-9.cr]+$/.test(amount);
  const number = (n: number): boolean => natural(n);
  switch (action.act) {
    case "quote":
    case "pay":
    case "complete":
    case "refund":
      return isListingCode(action.code) ? null : "invalid market code";
    case "renew":
    case "renew_pay":
    case "renew_decline":
      return number(action.licence) ? null : "invalid market number";
    case "resell":
      if (!number(action.licence)) return "invalid market number";
      if (!player(action.player)) return "invalid market player";
      return price(action.price) ? null : "invalid market price";
    case "charter":
      if (!number(action.filed)) return "invalid market number";
      if (!player(action.player)) return "invalid market player";
      return price(action.price) ? null : "invalid market price";
    case "resell_accept":
      if (!number(action.licence)) return "invalid market number";
      return action.price === null || price(action.price) ? null : "invalid market price";
    case "charter_accept":
      if (!number(action.filed)) return "invalid market number";
      return action.price === null || price(action.price) ? null : "invalid market price";
    case "publish":
      if (!number(action.filed)) return "invalid market number";
      return action.royalty === null ||
        (Number.isInteger(action.royalty) && action.royalty >= 0 && action.royalty <= MAX_MARKET_ROYALTY_PERMILLE)
        ? null
        : "invalid market royalty";
    case "withdraw":
      return number(action.filed) ? null : "invalid market number";
    default:
      return "invalid market action";
  }
}
