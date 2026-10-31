//! **The market desk** (17g cut C3b; the C3 plan §2.4): the connection-local
//! request state behind the lettered market screen `m` and the legal-actions
//! panel's typed submit.
//!
//! It holds the two market keyframes decoded closed (`mind.market`, the page
//! of listings with the actor's own quotes and waiting purchases, and
//! `mind.holdings`, one section of the actor's own licences), the cursors the
//! shown pages were asked with, the section, the one open form, the one
//! pending market request and the legal-action attempts. Like
//! [`super::mind_workshop::MindWorkshop`], it owns no transport: a form's
//! Enter pushes one typed request to an outbox the output pump drains
//! (`TuiApp::take_invokes`), and the re-queries after a staged action are
//! handed back to `TuiApp::apply_event` the same way.
//!
//! **No command text** (17g §3 C3, D-I-18): nothing here builds a `+minds`
//! line. A bare key opens a form or moves a page and sends nothing; Enter
//! inside a form sends exactly one `MindRequest::Market` (or, for the actions
//! panel, one `LegalActionRequest::SubmitSelf`) after
//! `InvokeRequest::validate` passes. The server builds every parameter from
//! the actor's own holdings; the request carries only what a player types
//! (17g C3a's `MarketAction`).
//!
//! **Ids** (the C3 plan §2.4, C3-FEAS-4): every request the desk sends takes
//! an id from its own counter, which starts at `1 << 40` and never passes
//! `MAX_VIEW_WIRE_INTEGER` — the gateway drops a `LegalAction` invoke whose id
//! is past that bound with no reply, which would leave the desk pending for
//! the session. The workshop counts from `1 << 62`, so the two ranges are
//! disjoint and a late reply can never reach the wrong owner.
//!
//! **What reaches the screen** is the server's bytes (`Staged.line`,
//! `Error.message`, `Shown.lines`), the web's ten legal-action sentences
//! (`web/src/lib/stores/action-requests.svelte.ts` `actionMessage`), and the
//! desk's own short sentences below. A keyframe's unknown keys are ignored and
//! its token-shaped fields are never kept: a listing is its 8-character code.

use std::collections::BTreeMap;

use mush_proto::{
    InvokeRequest, InvokeResult, LegalActionInteger, LegalActionParameters, LegalActionRequest,
    LegalActionResponse, LegalActionScalar, ListingCode, MAX_VIEW_WIRE_INTEGER, MarketAction,
    MarketListingForm, MindRequest, MindResponse, ProjectedOfferId,
};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use serde_json::{Value, json};

use super::keys::Key;
use super::screen::LocalAction;
use crate::line::LineBuffer;

/// The listings feed and the query of the same name (17g C3a,
/// `tacsim::minds::market_commands::query::MARKET_FEED`).
pub(super) const MARKET_FEED: &str = "mind.market";
/// The holdings feed and the query of the same name (17g C3a).
pub(super) const HOLDINGS_FEED: &str = "mind.holdings";
/// The schema version both keyframes carry (17g C3a `SCHEMA_VERSION`).
const SCHEMA_VERSION: u64 = 1;
/// What a pane shows for a keyframe of the wrong shape: the server's own
/// sentence for a page it would not build (17g C3a `NO_PAGE`).
pub(super) const NO_PAGE: &str = "No such market page.";
/// The first id the desk draws (the C3 plan §2.4): far below the workshop's
/// `1 << 62`, and far below `MAX_VIEW_WIRE_INTEGER`.
pub(super) const FIRST_ID: u64 = 1 << 40;
/// The most rows of one keyframe the desk keeps (the web's `parseActions`
/// cap; a server page is 20).
const MAX_ROWS: usize = 128;
/// The most legal-action attempts held at once (the C3 plan §7).
pub(super) const MAX_ATTEMPTS: usize = 16;
/// The most characters a typed player name or price may run to in a form
/// (`MAX_MARKET_PLAYER_CHARS` is 32 and `MAX_MARKET_PRICE_BYTES` 16; the form
/// stops a little past them so `validate` gives the refusal).
const MAX_FIELD_CHARS: usize = 40;
/// A digits field (a licence or filed number, a royalty).
const MAX_DIGITS: usize = 20;

/// The one-pending rule's refusal (the workshop's sentence).
pub(super) const PENDING: &str = "An operation is pending; wait for its result.";
/// While a request is out.
pub(super) const WAITING: &str = "Waiting for the server...";
/// The counter ran past the bound (never in a session's life).
pub(super) const IDS_EXHAUSTED: &str = "Request IDs exhausted; reconnect.";
/// 17g C2's sentence for a reply the client never saw: shown when a session
/// boundary forgets a pending market request (the TUI pump has no timeout of
/// its own, the C3 plan §2.4).
pub(super) const LOST: &str = "A request whose result you have not seen is not a refusal; check your holdings before trying again.";
/// `k` on a holding with no renewal action (the C3 plan §2.4).
pub(super) const NOT_RENEWABLE: &str = "This licence is not renewable now.";
/// `t` on a holding with neither `resell` nor `charter` (the C3 plan §2.4).
pub(super) const NOT_OFFERABLE: &str = "This holding cannot be offered now.";
/// `l` on a holding without `publish`.
pub(super) const NOT_LISTABLE: &str = "This filed mind cannot be listed now.";
/// `x` on a holding without `withdraw`.
pub(super) const NOT_WITHDRAWABLE: &str = "This filed mind cannot be withdrawn now.";
/// A row key was asked of a pane with no row.
pub(super) const NO_ROW: &str = "Select a row first.";
/// `]` on the last page.
pub(super) const LAST_PAGE: &str = "This is the last page.";
/// The accept form's number field is empty.
pub(super) const NO_NUMBER: &str = "Enter the number the seller named.";
/// A reply of a kind the request cannot have (never from a conforming server).
pub(super) const UNEXPECTED: &str = "Unexpected response; refresh the market.";
/// `a` on an offer with targets or a subject other than oneself (the C3 plan
/// §2.4).
pub(super) const NEEDS_TARGET: &str = "This action needs a target; use +act.";
/// `a` on an offer whose parameter schema no control here can fill (the web's
/// `ActionRequestsStore` notice, verbatim).
pub(super) const NO_CONTROL: &str = "This action needs another control surface.";
/// `a` with no offer highlighted.
pub(super) const NO_ACTION: &str = "No action is selected.";
/// Every attempt slot holds an unresolved action (the web's sentence).
pub(super) const TOO_MANY: &str =
    "Too many unresolved actions. Check their transaction status first.";
/// A market form's foot while its request is out: Esc only closes the
/// window, and saying "cancels" there would read as a refund (common rule
/// "a refused or unknown outcome is never reported as a refund").
pub(super) const SENT_FOOT: &str = "Sent; Esc closes this window but does not cancel the request.";

// ---- the two keyframes, decoded closed -----------------------------------

/// One listing of a `mind.market` page (17g C3a `listing_row`): the fields
/// the screen paints or a form shows. `price_minor`, the levy and the venue
/// shares are checked and not kept: the client paints `price_text` only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ListingRow {
    pub(super) code: String,
    pub(super) name: String,
    pub(super) class: String,
    pub(super) form: String,
    pub(super) transferable: bool,
    pub(super) derived: bool,
    pub(super) term_seconds: i64,
    pub(super) grace_seconds: i64,
    pub(super) price_text: String,
    pub(super) terms_line: String,
    pub(super) split_line: String,
}

/// One of the actor's own quotes (the `own.quotes` strip).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct OwnQuote {
    pub(super) code: String,
    pub(super) price_text: String,
    pub(super) expires_in_seconds: u64,
    pub(super) expired: bool,
}

/// A `mind.market` keyframe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct MarketView {
    pub(super) open: bool,
    pub(super) notice: Option<String>,
    pub(super) rows: Vec<ListingRow>,
    /// The next page's cursor (the last row's code), passed back as `after`.
    pub(super) after: Option<String>,
    pub(super) quotes: Vec<OwnQuote>,
    pub(super) waiting: Vec<String>,
}

/// The holdings section a `mind.holdings` page shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Section {
    /// The actor's filed minds (owner and dependent licences).
    #[default]
    Owner,
    /// The actor's paid licences.
    Paid,
}

impl Section {
    /// The wire word (`MindHoldingsSection`).
    pub(super) fn word(self) -> &'static str {
        match self {
            Section::Owner => "owner",
            Section::Paid => "paid",
        }
    }
    fn other(self) -> Section {
        match self {
            Section::Owner => Section::Paid,
            Section::Paid => Section::Owner,
        }
    }
}

/// An owner licence's or a paid licence's phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    Active(u64),
    Grace(u64),
    Lapsed,
}

/// A paid licence's renewal head.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Renewal {
    Offered,
    Paid,
}

/// The actions the server offers on a holding row (17g C3a `owner_actions`,
/// `paid_actions`); an unknown word is dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HoldingAct {
    Publish,
    Withdraw,
    Charter,
    Renew,
    RenewPay,
    RenewDecline,
    Resell,
}

impl HoldingAct {
    fn of(word: &str) -> Option<HoldingAct> {
        Some(match word {
            "publish" => HoldingAct::Publish,
            "withdraw" => HoldingAct::Withdraw,
            "charter" => HoldingAct::Charter,
            "renew" => HoldingAct::Renew,
            "renew_pay" => HoldingAct::RenewPay,
            "renew_decline" => HoldingAct::RenewDecline,
            "resell" => HoldingAct::Resell,
            _ => return None,
        })
    }
}

/// One row of a `mind.holdings` page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Holding {
    /// A filed mind the actor holds the charter of (or a dependent licence).
    Owner {
        filed: u64,
        name: String,
        dependent: bool,
        phase: Phase,
        selling: bool,
        actions: Vec<HoldingAct>,
    },
    /// A licence the actor paid for (or the House Pin).
    Paid {
        licence: u64,
        house: bool,
        name: String,
        form: String,
        transferable: bool,
        phase: Phase,
        revoked: bool,
        selling: bool,
        renewal: Option<Renewal>,
        actions: Vec<HoldingAct>,
    },
}

impl Holding {
    /// The row's number: the filed number or the licence number (the row key).
    pub(super) fn number(&self) -> u64 {
        match self {
            Holding::Owner { filed, .. } => *filed,
            Holding::Paid { licence, .. } => *licence,
        }
    }
    fn name(&self) -> &str {
        match self {
            Holding::Owner { name, .. } | Holding::Paid { name, .. } => name,
        }
    }
    fn offers(&self, act: HoldingAct) -> bool {
        match self {
            Holding::Owner { actions, .. } | Holding::Paid { actions, .. } => {
                actions.contains(&act)
            }
        }
    }
}

/// A `mind.holdings` keyframe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct HoldingsView {
    pub(super) open: bool,
    pub(super) notice: Option<String>,
    pub(super) section: Section,
    pub(super) rows: Vec<Holding>,
    /// The next page's cursor, passed back as `after`.
    pub(super) after: Option<u64>,
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_owned)
}
fn flag(v: &Value, key: &str) -> Option<bool> {
    v.get(key).and_then(Value::as_bool)
}
fn count(v: &Value, key: &str) -> Option<u64> {
    v.get(key).and_then(Value::as_u64)
}
fn seconds(v: &Value, key: &str) -> Option<i64> {
    v.get(key).and_then(Value::as_i64)
}
/// `null` or a string: `Some(None)` / `Some(Some(..))`; anything else `None`.
fn maybe_text(v: &Value, key: &str) -> Option<Option<String>> {
    match v.get(key) {
        Some(Value::Null) => Some(None),
        Some(Value::String(s)) => Some(Some(s.clone())),
        _ => None,
    }
}
fn is_code(code: &str) -> bool {
    ListingCode::try_from(code).is_ok()
}
fn decimal(v: &Value, key: &str) -> Option<()> {
    let s = v.get(key)?.as_str()?;
    (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())).then_some(())
}
fn schema_is(v: &Value, id: &str) -> bool {
    let schema = v.get("schema");
    schema.and_then(|s| s.get("id")).and_then(Value::as_str) == Some(id)
        && schema
            .and_then(|s| s.get("version"))
            .and_then(Value::as_u64)
            == Some(SCHEMA_VERSION)
}

/// Decode a `mind.market` keyframe closed: the schema id and version, every
/// field the screen reads typed, a code that is not 8 digits or letters a
/// to f refused; unknown keys ignored. `None` is a bad shape.
pub(super) fn decode_market(data: &Value) -> Option<MarketView> {
    if !schema_is(data, MARKET_FEED) {
        return None;
    }
    let mut rows = Vec::new();
    for row in data.get("rows")?.as_array()?.iter().take(MAX_ROWS) {
        let code = text(row, "code").filter(|c| is_code(c))?;
        decimal(row, "price_minor")?;
        for share in ["levy_permille", "venue_permille", "parent_royalty_permille"] {
            count(row, share)?;
        }
        rows.push(ListingRow {
            code,
            name: text(row, "name")?,
            class: text(row, "class")?,
            form: text(row, "form")?,
            transferable: flag(row, "transferable")?,
            derived: flag(row, "derived")?,
            term_seconds: seconds(row, "term_seconds")?,
            grace_seconds: seconds(row, "grace_seconds")?,
            price_text: text(row, "price_text")?,
            terms_line: text(row, "terms_line")?,
            split_line: text(row, "split_line")?,
        });
    }
    let own = data.get("own")?;
    let mut quotes = Vec::new();
    for quote in own.get("quotes")?.as_array()?.iter().take(MAX_ROWS) {
        decimal(quote, "price_minor")?;
        quotes.push(OwnQuote {
            code: text(quote, "code").filter(|c| is_code(c))?,
            price_text: text(quote, "price_text")?,
            expires_in_seconds: count(quote, "expires_in_seconds")?,
            expired: flag(quote, "expired")?,
        });
    }
    let mut waiting = Vec::new();
    for purchase in own.get("waiting")?.as_array()?.iter().take(MAX_ROWS) {
        waiting.push(text(purchase, "code").filter(|c| is_code(c))?);
    }
    let after = maybe_text(data, "after")?;
    if after.as_deref().is_some_and(|c| !is_code(c)) {
        return None;
    }
    Some(MarketView {
        open: flag(data, "open")?,
        notice: maybe_text(data, "notice")?,
        rows,
        after,
        quotes,
        waiting,
    })
}

fn phase_of(v: &Value) -> Option<Phase> {
    let phase = v.get("phase")?;
    let days = phase.get("days_left");
    match phase.get("state")?.as_str()? {
        "active" => Some(Phase::Active(days?.as_u64()?)),
        "grace" => Some(Phase::Grace(days?.as_u64()?)),
        "lapsed" if days.is_none_or(Value::is_null) => Some(Phase::Lapsed),
        _ => None,
    }
}

fn actions_of(v: &Value) -> Option<Vec<HoldingAct>> {
    let mut actions = Vec::new();
    for word in v.get("actions")?.as_array()? {
        if let Some(act) = HoldingAct::of(word.as_str()?) {
            actions.push(act);
        }
    }
    Some(actions)
}

/// Decode a `mind.holdings` keyframe closed (as [`decode_market`]); an
/// unknown row kind or an unknown `actions` entry that is not a string is a
/// bad shape, an unknown action word is dropped.
pub(super) fn decode_holdings(data: &Value) -> Option<HoldingsView> {
    if !schema_is(data, HOLDINGS_FEED) {
        return None;
    }
    let section = match data.get("section")?.as_str()? {
        "owner" => Section::Owner,
        "paid" => Section::Paid,
        _ => return None,
    };
    let mut rows = Vec::new();
    for row in data.get("rows")?.as_array()?.iter().take(MAX_ROWS) {
        rows.push(match row.get("kind")?.as_str()? {
            "owner" => Holding::Owner {
                filed: count(row, "filed")?,
                name: text(row, "name")?,
                dependent: flag(row, "dependent")?,
                phase: phase_of(row)?,
                selling: flag(row, "selling")?,
                actions: actions_of(row)?,
            },
            "paid" => Holding::Paid {
                licence: count(row, "licence")?,
                house: flag(row, "house")?,
                name: text(row, "name")?,
                form: text(row, "form")?,
                transferable: flag(row, "transferable")?,
                phase: phase_of(row)?,
                revoked: flag(row, "revoked")?,
                selling: flag(row, "selling")?,
                renewal: match maybe_text(row, "renewal")?.as_deref() {
                    None => None,
                    Some("offered") => Some(Renewal::Offered),
                    Some("paid") => Some(Renewal::Paid),
                    Some(_) => return None,
                },
                actions: actions_of(row)?,
            },
            _ => return None,
        });
    }
    let after = match data.get("after")? {
        Value::Null => None,
        v => Some(v.as_u64()?),
    };
    Some(HoldingsView {
        open: flag(data, "open")?,
        notice: maybe_text(data, "notice")?,
        section,
        rows,
        after,
    })
}

// ---- the forms -----------------------------------------------------------

/// Which market pane a page key acts on: the focused one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MarketPane {
    Listings,
    Holdings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BuyStep {
    Quote,
    Pay,
    Complete,
    Refund,
}
const BUY_STEPS: [(BuyStep, &str); 4] = [
    (BuyStep::Quote, "quote"),
    (BuyStep::Pay, "pay"),
    (BuyStep::Complete, "complete"),
    (BuyStep::Refund, "refund"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RenewStep {
    Request,
    Pay,
    Decline,
}
const RENEW_STEPS: [(RenewStep, &str); 3] = [
    (RenewStep::Request, "request"),
    (RenewStep::Pay, "pay"),
    (RenewStep::Decline, "decline"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Licence,
    Filed,
}
impl Kind {
    fn word(self) -> &'static str {
        match self {
            Kind::Licence => "licence",
            Kind::Filed => "filed",
        }
    }
}
const KINDS: [(Kind, &str); 2] = [(Kind::Licence, "licence"), (Kind::Filed, "filed")];
const FORMS: [(MarketListingForm, &str); 3] = [
    (MarketListingForm::Sealed, "sealed"),
    (MarketListingForm::Open, "open"),
    (MarketListingForm::OpenResale, "open-resale"),
];
const YES_NO: [(bool, &str); 2] = [(true, "yes"), (false, "no")];

/// One legal-action parameter's control (the web's four, `actionControl`).
#[derive(Clone, Debug, PartialEq, Eq)]
enum ActionKind {
    Boolean,
    Integer {
        minimum: Option<i64>,
        maximum: Option<i64>,
    },
    Text {
        maximum_bytes: usize,
    },
    Choice {
        choices: Vec<String>,
    },
}

#[derive(Clone, Debug)]
struct ActionField {
    id: String,
    required: bool,
    kind: ActionKind,
    /// The typed text (integer, text).
    line: LineBuffer,
    /// Whether the text was ever edited (an optional text field the player
    /// never touched is left out; one cleared to empty is sent empty).
    touched: bool,
    /// The pick (boolean: `0` yes, `1` no; choice: the index).
    pick: Option<usize>,
}

#[derive(Clone, Debug)]
struct ActionForm {
    offer: ProjectedOfferId,
    label: String,
    fields: Vec<ActionField>,
    field: usize,
}

#[derive(Clone, Debug)]
enum Form {
    /// `b`: the four purchase steps of one listing.
    Buy {
        code: String,
        name: String,
        step: usize,
        confirm: Vec<String>,
    },
    /// `k`: the three renewal steps of one paid licence.
    Renew {
        licence: u64,
        name: String,
        step: usize,
    },
    /// `t`: a resale offer (`Licence`) or a charter offer (`Filed`).
    Offer {
        kind: Kind,
        number: u64,
        name: String,
        field: usize,
        player: LineBuffer,
        price: LineBuffer,
    },
    /// `a`: accept an offer, or list the offers with an empty price.
    Accept {
        kind: usize,
        field: usize,
        number: LineBuffer,
        price: LineBuffer,
        shown: Vec<String>,
    },
    /// `l`: list a filed mind.
    Publish {
        filed: u64,
        name: String,
        field: usize,
        form: usize,
        royalty: LineBuffer,
        transfer: usize,
    },
    /// `x`: the withdrawal's confirm.
    Withdraw { filed: u64, name: String },
    /// The legal-actions panel's parameter form.
    Action(ActionForm),
    /// Read-only lines (the offer's closing instruction, an unexpected
    /// `Shown`); Enter or Esc closes it.
    Notice { title: String, lines: Vec<String> },
}

impl Form {
    fn is_market(&self) -> bool {
        !matches!(self, Form::Action(_))
    }
    /// The number of focusable fields.
    fn fields(&self) -> usize {
        match self {
            Form::Buy { .. } | Form::Renew { .. } => 1,
            Form::Offer { .. } => 2,
            Form::Accept { .. } | Form::Publish { .. } => 3,
            Form::Action(a) => a.fields.len(),
            Form::Withdraw { .. } | Form::Notice { .. } => 0,
        }
    }
    fn field_mut(&mut self) -> Option<&mut usize> {
        match self {
            Form::Offer { field, .. }
            | Form::Accept { field, .. }
            | Form::Publish { field, .. } => Some(field),
            Form::Action(a) => Some(&mut a.field),
            _ => None,
        }
    }
}

fn cycle(at: usize, len: usize, back: bool) -> usize {
    if len == 0 {
        return 0;
    }
    if back {
        (at + len - 1) % len
    } else {
        (at + 1) % len
    }
}

fn clean(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

/// The one pending market request: its id and, for an offer, the closing
/// instruction its staged reply shows.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Pending {
    id: u64,
    closing: Option<String>,
}

/// One legal-action attempt (the web's `ActionAttempt`): the offer, the id
/// of a submission still in flight, the last sentence, and whether the
/// offer is blocked.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Attempt {
    offer: String,
    id: Option<u64>,
    message: String,
    blocked: bool,
}

/// The web's `actionMessage`, verbatim, and whether the state blocks the
/// offer until the `legal.actions` keyframe is replaced (the C3 plan §2.4:
/// `queued`, `owned`, `pending`, `unavailable`, `unknown`, `expired` block;
/// `committed`, `rejected`, `invalid`, `busy` unblock at once).
pub(super) fn action_sentence(response: &LegalActionResponse) -> (&'static str, bool) {
    match response {
        LegalActionResponse::Committed { .. } => ("Action committed.", false),
        LegalActionResponse::Owned { .. } => (
            "Transaction accepted. Check its status for completion.",
            true,
        ),
        LegalActionResponse::Pending { .. } => ("Transaction is still pending.", true),
        LegalActionResponse::Queued {} => (
            "Submission queued. Wait for a transaction result before submitting again.",
            true,
        ),
        LegalActionResponse::Unavailable { .. } => (
            "Completion is unavailable. This does not mean the action failed.",
            true,
        ),
        LegalActionResponse::Unknown {} => (
            "No accessible transaction was found. This does not prove the action failed.",
            true,
        ),
        LegalActionResponse::Expired {} => (
            "The transaction reference expired. Check your current state before taking another action.",
            true,
        ),
        LegalActionResponse::Busy {} => ("The server is busy. Try again later.", false),
        LegalActionResponse::Rejected {} => {
            ("The offer was refused. Refresh available actions.", false)
        }
        LegalActionResponse::Invalid {} => (
            "The inputs were refused. Check them and refresh available actions.",
            false,
        ),
    }
}

/// A parameter id the kernel accepts (`LegalActionParameters::to_json`'s
/// rule and the web's `canonical`).
fn canonical(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 96
        && !id.starts_with('.')
        && !id.ends_with('.')
        && !id.contains("..")
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'.'))
}

/// The web's `actionControl`: only an offer on oneself with no targets is
/// filled in here, one control per declared parameter. `Err` is the
/// sentence the status shows.
fn action_control(offer: &Value) -> Result<ActionForm, &'static str> {
    let id = offer
        .get("offer")
        .and_then(Value::as_str)
        .and_then(|s| ProjectedOfferId::new(s.to_owned()).ok())
        .ok_or(NO_CONTROL)?;
    let kind_of = |key: &str| {
        offer
            .get(key)
            .and_then(|v| v.get("kind"))
            .and_then(Value::as_str)
    };
    match (kind_of("subject"), kind_of("targets")) {
        (Some("self_actor"), Some("none")) => {}
        (Some(_), Some(_)) => return Err(NEEDS_TARGET),
        _ => return Err(NO_CONTROL),
    }
    let parameters = offer
        .get("parameters")
        .and_then(Value::as_array)
        .filter(|p| p.len() <= mush_proto::MAX_LEGAL_ACTION_PARAMETERS)
        .ok_or(NO_CONTROL)?;
    let mut fields: Vec<ActionField> = Vec::new();
    for parameter in parameters {
        let id = text(parameter, "id")
            .filter(|id| canonical(id))
            .ok_or(NO_CONTROL)?;
        if fields.iter().any(|f| f.id == id) {
            return Err(NO_CONTROL);
        }
        let required = flag(parameter, "required").ok_or(NO_CONTROL)?;
        let spec = parameter.get("kind").ok_or(NO_CONTROL)?;
        let kind = match spec.get("kind").and_then(Value::as_str) {
            Some("boolean") => ActionKind::Boolean,
            Some("integer") => {
                let minimum = spec.get("minimum").and_then(Value::as_i64);
                let maximum = spec.get("maximum").and_then(Value::as_i64);
                match (minimum, maximum) {
                    (Some(lo), Some(hi)) if lo <= hi => ActionKind::Integer {
                        minimum: Some(lo),
                        maximum: Some(hi),
                    },
                    _ => return Err(NO_CONTROL),
                }
            }
            Some("text") => match count(spec, "maximum_bytes") {
                Some(n @ 1..=4096) => ActionKind::Text {
                    maximum_bytes: usize::try_from(n).map_err(|_| NO_CONTROL)?,
                },
                _ => return Err(NO_CONTROL),
            },
            Some("choice") => {
                let choices: Vec<String> = spec
                    .get("choices")
                    .and_then(Value::as_array)
                    .ok_or(NO_CONTROL)?
                    .iter()
                    .map(|c| c.as_str().map(str::to_owned))
                    .collect::<Option<_>>()
                    .ok_or(NO_CONTROL)?;
                let distinct = choices
                    .iter()
                    .enumerate()
                    .all(|(i, c)| !choices.iter().take(i).any(|d| d == c));
                if choices.is_empty()
                    || choices.len() > 256
                    || !distinct
                    || choices.iter().any(|c| {
                        c.is_empty()
                            || c.len() > mush_proto::MAX_LEGAL_ACTION_TEXT_BYTES
                            || c.chars().any(char::is_control)
                    })
                {
                    return Err(NO_CONTROL);
                }
                ActionKind::Choice { choices }
            }
            _ => return Err(NO_CONTROL),
        };
        fields.push(ActionField {
            id,
            required,
            kind,
            line: LineBuffer::new(0),
            touched: false,
            pick: None,
        });
    }
    let presentation = offer.get("presentation");
    let label = presentation
        .and_then(|p| p.get("label"))
        .or_else(|| presentation.and_then(|p| p.get("verb")))
        .and_then(Value::as_str)
        .map(clean)
        .unwrap_or_default();
    Ok(ActionForm {
        offer: id,
        label,
        fields,
        field: 0,
    })
}

/// The web's `submitSelf`: the typed values as exact scalars, or the
/// sentence that refuses them.
fn action_parameters(form: &ActionForm) -> Result<LegalActionParameters, String> {
    let mut values = BTreeMap::new();
    for field in &form.fields {
        let named = field.id.replace('_', " ");
        let value = match &field.kind {
            ActionKind::Boolean => match field.pick {
                Some(pick) => LegalActionScalar::Boolean { value: pick == 0 },
                None if field.required => return Err("Choose Yes or No.".into()),
                None => continue,
            },
            ActionKind::Choice { choices } => match field.pick.and_then(|i| choices.get(i)) {
                Some(choice) => LegalActionScalar::Text {
                    value: choice.clone(),
                },
                None if field.required => return Err("Choose an available option.".into()),
                None => continue,
            },
            ActionKind::Integer { minimum, maximum } => {
                let typed = field.line.text();
                if typed.is_empty() {
                    if field.required {
                        return Err(format!("Enter {named}."));
                    }
                    continue;
                }
                let number = LegalActionInteger::new(typed.clone()).map_err(|_| {
                    "Enter a whole number without spaces or leading zeros.".to_owned()
                })?;
                let exact = number.value().map_err(|e| e.to_owned())?;
                if minimum.is_some_and(|lo| exact < lo) || maximum.is_some_and(|hi| exact > hi) {
                    return Err("That number is outside the allowed range.".into());
                }
                LegalActionScalar::Integer { value: number }
            }
            ActionKind::Text { maximum_bytes } => {
                if !field.required && !field.touched {
                    continue;
                }
                let typed = field.line.text();
                if typed.len() > *maximum_bytes || typed.chars().any(char::is_control) {
                    return Err("Text is too long or contains a control character.".into());
                }
                LegalActionScalar::Text { value: typed }
            }
        };
        values.insert(field.id.clone(), value);
    }
    LegalActionParameters::new(values).map_err(|e| format!("Request refused: {e}"))
}

// ---- the desk ------------------------------------------------------------

/// Bounded market and legal-action request state owned by one attached SSH
/// connection (17g cut C3b).
pub(super) struct MarketDesk {
    market: Option<MarketView>,
    holdings: Option<HoldingsView>,
    /// The cursor the shown listings page was asked with (`None`: the first).
    listings_at: Option<String>,
    /// The cursor the shown holdings page was asked with.
    holdings_at: Option<u64>,
    /// The section asked for last, or shown by the last keyframe.
    section: Section,
    form: Option<Form>,
    pending: Option<Pending>,
    attempts: Vec<Attempt>,
    next_id: u64,
    outbox: Vec<(u64, InvokeRequest)>,
    queries: Vec<(String, Value)>,
    /// The market's status sentence (the market screen's status line).
    status: String,
    /// The legal actions' status sentence (screens 1 and 7).
    action_status: String,
    /// How many times [`Self::status`] was set, and [`Self::action_status`]:
    /// a form paints only a sentence set while it was open (`form_from`), so
    /// an older answer — another listing's refusal, the targeted offer's
    /// `+act` sentence — never reads as the new form's (the C3b review).
    said: u64,
    action_said: u64,
    /// The count of the form's own status kind when it opened.
    form_from: u64,
}

impl Default for MarketDesk {
    fn default() -> Self {
        Self::new()
    }
}

impl MarketDesk {
    /// An empty desk whose first request id is [`FIRST_ID`].
    pub(super) fn new() -> Self {
        Self {
            market: None,
            holdings: None,
            listings_at: None,
            holdings_at: None,
            section: Section::Owner,
            form: None,
            pending: None,
            attempts: Vec::new(),
            next_id: FIRST_ID,
            outbox: Vec::new(),
            queries: Vec::new(),
            status: String::new(),
            action_status: String::new(),
            said: 0,
            action_said: 0,
            form_from: 0,
        }
    }

    /// Set the market's status sentence.
    fn say(&mut self, sentence: &str) {
        sentence.clone_into(&mut self.status);
        self.said = self.said.saturating_add(1);
    }

    /// Set the legal actions' status sentence.
    fn say_action(&mut self, sentence: &str) {
        sentence.clone_into(&mut self.action_status);
        self.action_said = self.action_said.saturating_add(1);
    }

    /// Open `form`, remembering which status sentences predate it.
    fn open(&mut self, form: Form) {
        self.form_from = if form.is_market() {
            self.said
        } else {
            self.action_said
        };
        self.form = Some(form);
    }

    /// Forget everything on a session or character boundary — the views,
    /// the form, the pending request and every attempt — keeping only the id
    /// counter, so a late reply to a forgotten id is ignored and no id is
    /// reused. A market request that was pending leaves C2's "not a refusal"
    /// sentence behind: its result will never be seen here.
    pub(super) fn reset(&mut self) {
        let lost = self.pending.is_some();
        *self = Self {
            next_id: self.next_id,
            ..Self::new()
        };
        if lost {
            self.say(LOST);
        }
    }

    /// Whether a market request is out.
    pub(super) fn pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Whether a form is open (it owns the keys while it is).
    pub(super) fn form_open(&self) -> bool {
        self.form.is_some()
    }

    /// The market's status sentence.
    pub(super) fn status(&self) -> &str {
        &self.status
    }

    /// The legal actions' status sentence.
    pub(super) fn action_status(&self) -> &str {
        &self.action_status
    }

    /// The typed requests a form's Enter queued, for the output pump.
    pub(super) fn take_invokes(&mut self) -> Vec<(u64, InvokeRequest)> {
        std::mem::take(&mut self.outbox)
    }

    /// The queries a key or a staged reply queued, `(name, args)`.
    pub(super) fn take_queries(&mut self) -> Vec<(String, Value)> {
        std::mem::take(&mut self.queries)
    }

    /// A `mind.market` or `mind.holdings` keyframe arrived: decode it closed.
    /// A bad shape is held as none, so no form opens on it (its pane paints
    /// [`NO_PAGE`] from the same decoder).
    pub(super) fn apply_keyframe(&mut self, feed: &str, data: &Value) {
        match feed {
            MARKET_FEED => self.market = decode_market(data),
            HOLDINGS_FEED => {
                self.holdings = decode_holdings(data);
                if let Some(view) = &self.holdings {
                    self.section = view.section;
                }
            }
            _ => {}
        }
    }

    /// The screen asked these queries itself (its `on_enter`, a hello's
    /// re-hydrate): remember the cursors and the section they name, so a
    /// staged reply re-asks the pages actually shown.
    pub(super) fn note_asks(&mut self, queries: &[(String, Value)]) {
        for (name, args) in queries {
            match name.as_str() {
                MARKET_FEED => {
                    self.listings_at = args.get("after").and_then(Value::as_str).map(str::to_owned);
                }
                HOLDINGS_FEED => {
                    self.holdings_at = args.get("after").and_then(Value::as_u64);
                    match args.get("section").and_then(Value::as_str) {
                        Some("owner") => self.section = Section::Owner,
                        Some("paid") => self.section = Section::Paid,
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }

    fn listings_ask(&self) -> (String, Value) {
        (MARKET_FEED.to_owned(), json!({ "after": self.listings_at }))
    }

    fn holdings_ask(&self) -> (String, Value) {
        (
            HOLDINGS_FEED.to_owned(),
            json!({ "section": self.section.word(), "after": self.holdings_at }),
        )
    }

    /// The two pages the screen is on, asked again (`r`, and once after a
    /// staged action).
    pub(super) fn page_queries(&self) -> Vec<(String, Value)> {
        vec![self.listings_ask(), self.holdings_ask()]
    }

    fn draw_id(&mut self) -> Option<u64> {
        let id = self.next_id;
        if id > MAX_VIEW_WIRE_INTEGER {
            return None;
        }
        self.next_id = id.saturating_add(1);
        Some(id)
    }

    fn holding(&self, key: Option<&str>) -> Result<&Holding, &'static str> {
        let view = self.holdings.as_ref().ok_or(NO_PAGE)?;
        let number = key.and_then(|k| k.parse::<u64>().ok()).ok_or(NO_ROW)?;
        view.rows
            .iter()
            .find(|h| h.number() == number)
            .ok_or(NO_ROW)
    }

    /// One of the market screen's local keys. `focus` is the focused pane;
    /// `listing` and `holding` are the row keys the two panes highlight (a
    /// listing's code, a holding's number). Nothing here sends: a form opens,
    /// a page is asked for, or the status says why not.
    pub(super) fn local(
        &mut self,
        action: LocalAction,
        focus: MarketPane,
        listing: Option<&str>,
        holding: Option<&str>,
    ) {
        // The one-pending rule (the C3 plan §2.4) holds at the keys too: a
        // form opened while a request is out could not send, would be titled
        // as sent, and would be closed by the earlier request's reply.
        let form_key = matches!(
            action,
            LocalAction::MarketBuy
                | LocalAction::MarketRenew
                | LocalAction::MarketOffer
                | LocalAction::MarketAccept
                | LocalAction::MarketPublish
                | LocalAction::MarketWithdraw
        );
        if form_key && self.pending.is_some() {
            self.say(PENDING);
            return;
        }
        let opened = match action {
            LocalAction::MarketBuy => self.open_buy(listing),
            LocalAction::MarketRenew => self.open_renew(holding),
            LocalAction::MarketOffer => self.open_offer(holding),
            LocalAction::MarketAccept => {
                self.open(Form::Accept {
                    kind: 0,
                    field: 0,
                    number: LineBuffer::new(0),
                    price: LineBuffer::new(0),
                    shown: Vec::new(),
                });
                Ok(())
            }
            LocalAction::MarketPublish => self.open_publish(holding),
            LocalAction::MarketWithdraw => self.open_withdraw(holding),
            LocalAction::MarketSection => {
                self.section = self.section.other();
                self.holdings_at = None;
                self.queries.push(self.holdings_ask());
                Ok(())
            }
            LocalAction::MarketPageNext => self.page_next(focus),
            LocalAction::MarketPageFirst => {
                match focus {
                    MarketPane::Listings => {
                        self.listings_at = None;
                        self.queries.push(self.listings_ask());
                    }
                    MarketPane::Holdings => {
                        self.holdings_at = None;
                        self.queries.push(self.holdings_ask());
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        };
        if let Err(sentence) = opened {
            self.say(sentence);
        }
    }

    fn page_next(&mut self, focus: MarketPane) -> Result<(), &'static str> {
        match focus {
            MarketPane::Listings => {
                let view = self.market.as_ref().ok_or(NO_PAGE)?;
                let after = view.after.clone().ok_or(LAST_PAGE)?;
                self.listings_at = Some(after);
                self.queries.push(self.listings_ask());
            }
            MarketPane::Holdings => {
                let view = self.holdings.as_ref().ok_or(NO_PAGE)?;
                let after = view.after.ok_or(LAST_PAGE)?;
                self.holdings_at = Some(after);
                self.queries.push(self.holdings_ask());
            }
        }
        Ok(())
    }

    fn open_buy(&mut self, listing: Option<&str>) -> Result<(), &'static str> {
        let view = self.market.as_ref().ok_or(NO_PAGE)?;
        let code = listing.ok_or(NO_ROW)?;
        let row = view.rows.iter().find(|r| r.code == code);
        let quote = view.quotes.iter().find(|q| q.code == code);
        let waiting = view.waiting.iter().any(|w| w == code);
        // A code off this page is still the actor's own when the own strip
        // names it (the C3b review): a purchase paid into escrow whose listing
        // was withdrawn, or one bought at a counter the actor has walked away
        // from, must still reach `complete` and `refund` from the screen.
        if row.is_none() && quote.is_none() && !waiting {
            return Err(NO_ROW);
        }
        // The default step follows the actor's own state (the C3 plan §2.4):
        // a waiting purchase completes, a live quote pays, and no quote or an
        // expired one asks for a quote.
        let step = if waiting {
            BuyStep::Complete
        } else if quote.is_some_and(|q| !q.expired) {
            BuyStep::Pay
        } else {
            BuyStep::Quote
        };
        let mut confirm: Vec<String> = row
            .map(|row| vec![clean(&row.terms_line), clean(&row.split_line)])
            .unwrap_or_default();
        if let Some(quote) = quote {
            confirm.push(if quote.expired {
                "Your quote has expired.".to_owned()
            } else {
                format!(
                    "Your quote: {}, {}s left.",
                    clean(&quote.price_text),
                    quote.expires_in_seconds
                )
            });
        }
        let form = Form::Buy {
            code: code.to_owned(),
            name: row.map(|row| clean(&row.name)).unwrap_or_default(),
            step: BUY_STEPS.iter().position(|(s, _)| *s == step).unwrap_or(0),
            confirm,
        };
        self.open(form);
        Ok(())
    }

    fn open_renew(&mut self, holding: Option<&str>) -> Result<(), &'static str> {
        let row = self.holding(holding)?;
        let Holding::Paid {
            licence,
            name,
            renewal,
            ..
        } = row
        else {
            return Err(NOT_RENEWABLE);
        };
        if ![
            HoldingAct::Renew,
            HoldingAct::RenewPay,
            HoldingAct::RenewDecline,
        ]
        .iter()
        .any(|a| row.offers(*a))
        {
            return Err(NOT_RENEWABLE);
        }
        let step = match renewal {
            Some(Renewal::Offered) => RenewStep::Pay,
            _ => RenewStep::Request,
        };
        let form = Form::Renew {
            licence: *licence,
            name: clean(name),
            step: RENEW_STEPS
                .iter()
                .position(|(s, _)| *s == step)
                .unwrap_or(0),
        };
        self.open(form);
        Ok(())
    }

    fn open_offer(&mut self, holding: Option<&str>) -> Result<(), &'static str> {
        let row = self.holding(holding)?;
        let kind = match row {
            Holding::Paid { .. } if row.offers(HoldingAct::Resell) => Kind::Licence,
            Holding::Owner { .. } if row.offers(HoldingAct::Charter) => Kind::Filed,
            _ => return Err(NOT_OFFERABLE),
        };
        let form = Form::Offer {
            kind,
            number: row.number(),
            name: clean(row.name()),
            field: 0,
            player: LineBuffer::new(0),
            price: LineBuffer::new(0),
        };
        self.open(form);
        Ok(())
    }

    fn open_publish(&mut self, holding: Option<&str>) -> Result<(), &'static str> {
        let row = self.holding(holding)?;
        if !matches!(row, Holding::Owner { .. }) || !row.offers(HoldingAct::Publish) {
            return Err(NOT_LISTABLE);
        }
        let form = Form::Publish {
            filed: row.number(),
            name: clean(row.name()),
            field: 0,
            form: 0,
            royalty: LineBuffer::new(0),
            transfer: 0,
        };
        self.open(form);
        Ok(())
    }

    fn open_withdraw(&mut self, holding: Option<&str>) -> Result<(), &'static str> {
        let row = self.holding(holding)?;
        if !matches!(row, Holding::Owner { .. }) || !row.offers(HoldingAct::Withdraw) {
            return Err(NOT_WITHDRAWABLE);
        }
        let form = Form::Withdraw {
            filed: row.number(),
            name: clean(row.name()),
        };
        self.open(form);
        Ok(())
    }

    /// The actions panel's `a` (the C3 plan §2.4): `view` is the stored
    /// `legal.actions` view, `ordinal` the highlighted row's place among the
    /// offers the panel lists. An offer whose last answer blocks it shows
    /// that sentence and opens nothing; an offer with targets is sent to
    /// `+act`.
    pub(super) fn open_action(&mut self, view: Option<&Value>, ordinal: Option<usize>) {
        let offer = view
            .and_then(Value::as_array)
            .zip(ordinal)
            .and_then(|(offers, at)| {
                offers
                    .iter()
                    .filter(|o| o.get("presentation").is_some())
                    .nth(at)
            });
        let Some(offer) = offer else {
            self.say_action(NO_ACTION);
            return;
        };
        if let Some(message) = offer
            .get("offer")
            .and_then(Value::as_str)
            .and_then(|token| self.attempts.iter().find(|a| a.offer == token))
            .filter(|a| a.blocked || a.id.is_some())
            .map(|a| a.message.clone())
        {
            self.say_action(&message);
            return;
        }
        match action_control(offer) {
            Ok(form) => self.open(Form::Action(form)),
            Err(sentence) => self.say_action(sentence),
        }
    }

    /// The `legal.actions` view was replaced by a keyframe: every resolved
    /// attempt is forgotten, which unblocks its offer; a submission still in
    /// flight stays blocked until its answer lands.
    pub(super) fn actions_replaced(&mut self) {
        self.attempts.retain(|a| a.id.is_some());
    }

    /// A key while a form is open. Esc closes it; Enter sends
    /// ([`Self::enter`]); Tab, Shift-Tab, Up and Down move between fields;
    /// Left, Right and Space change a choice; the rest edit the focused text.
    /// While the market request is out, a market form only closes.
    pub(super) fn key(&mut self, key: Key) {
        let Some(form) = self.form.as_mut() else {
            return;
        };
        match key {
            Key::Esc => {
                self.form = None;
                return;
            }
            Key::Enter => {
                self.enter();
                return;
            }
            _ => {}
        }
        if form.is_market() && self.pending.is_some() {
            return;
        }
        let fields = form.fields();
        match key {
            Key::Tab | Key::Down => {
                if let Some(field) = form.field_mut() {
                    *field = cycle(*field, fields, false);
                }
            }
            Key::BackTab | Key::Up => {
                if let Some(field) = form.field_mut() {
                    *field = cycle(*field, fields, true);
                }
            }
            Key::Left | Key::Right if typing(form) => edit(form, key),
            Key::Left => choose(form, true),
            Key::Right => choose(form, false),
            Key::Char(' ') if !typing(form) => choose(form, false),
            _ => edit(form, key),
        }
    }

    /// Enter in the open form: build its one request, check it with
    /// `InvokeRequest::validate`, and queue it — or say why not. A notice
    /// closes.
    pub(super) fn enter(&mut self) {
        let Some(form) = self.form.as_ref() else {
            return;
        };
        let (action, closing) = match form {
            Form::Notice { .. } => {
                self.form = None;
                return;
            }
            Form::Action(action) => {
                let action = action.clone();
                self.send_action(&action);
                return;
            }
            Form::Buy { code, step, .. } => {
                let Ok(code) = ListingCode::try_from(code.as_str()) else {
                    self.say(NO_PAGE);
                    return;
                };
                let action = match BUY_STEPS.get(*step).map(|(s, _)| *s) {
                    Some(BuyStep::Pay) => MarketAction::Pay { code },
                    Some(BuyStep::Complete) => MarketAction::Complete { code },
                    Some(BuyStep::Refund) => MarketAction::Refund { code },
                    _ => MarketAction::Quote { code },
                };
                (action, None)
            }
            Form::Renew { licence, step, .. } => {
                let licence = *licence;
                let action = match RENEW_STEPS.get(*step).map(|(s, _)| *s) {
                    Some(RenewStep::Pay) => MarketAction::RenewPay { licence },
                    Some(RenewStep::Decline) => MarketAction::RenewDecline { licence },
                    _ => MarketAction::Renew { licence },
                };
                (action, None)
            }
            Form::Offer {
                kind,
                number,
                player,
                price,
                ..
            } => {
                let player = player.text().trim().to_owned();
                let price = price.text().trim().to_owned();
                // The seller's instruction, from the typed fields (the C3 plan
                // §2.7: `RESELL_TELL` / `CHARTER_TELL` are not carried).
                let closing = format!(
                    "They accept with: a, {} {number}, price {price}.",
                    kind.word()
                );
                let action = match kind {
                    Kind::Licence => MarketAction::Resell {
                        licence: *number,
                        player,
                        price,
                    },
                    Kind::Filed => MarketAction::Charter {
                        filed: *number,
                        player,
                        price,
                    },
                };
                (action, Some(closing))
            }
            Form::Accept {
                kind,
                number,
                price,
                ..
            } => {
                let Ok(number) = number.text().parse::<u64>() else {
                    self.say(NO_NUMBER);
                    return;
                };
                let price = Some(price.text().trim().to_owned()).filter(|p| !p.is_empty());
                let action = match KINDS.get(*kind).map(|(k, _)| *k) {
                    Some(Kind::Filed) => MarketAction::CharterAccept {
                        filed: number,
                        price,
                    },
                    _ => MarketAction::ResellAccept {
                        licence: number,
                        price,
                    },
                };
                (action, None)
            }
            Form::Publish {
                filed,
                form,
                royalty,
                transfer,
                ..
            } => {
                let typed = royalty.text();
                let royalty = if typed.is_empty() {
                    None
                } else if let Ok(royalty) = typed.parse::<u16>() {
                    Some(royalty)
                } else {
                    // Past a u16 is past the 300 bound: `validate`'s refusal.
                    self.say("Request refused: invalid market royalty");
                    return;
                };
                let action = MarketAction::Publish {
                    filed: *filed,
                    form: FORMS
                        .get(*form)
                        .map_or(MarketListingForm::Sealed, |(f, _)| *f),
                    royalty,
                    transfer: Some(YES_NO.get(*transfer).is_none_or(|(yes, _)| *yes)),
                };
                (action, None)
            }
            Form::Withdraw { filed, .. } => (MarketAction::Withdraw { filed: *filed }, None),
        };
        self.send_market(action, closing);
    }

    fn send_market(&mut self, action: MarketAction, closing: Option<String>) {
        if self.pending.is_some() {
            self.say(PENDING);
            return;
        }
        let invoke = InvokeRequest::Mind(MindRequest::Market { action });
        if let Err(error) = invoke.validate() {
            self.say(&format!("Request refused: {error}"));
            return;
        }
        let Some(id) = self.draw_id() else {
            self.say(IDS_EXHAUSTED);
            return;
        };
        self.pending = Some(Pending { id, closing });
        self.outbox.push((id, invoke));
        self.say(WAITING);
    }

    fn send_action(&mut self, form: &ActionForm) {
        let parameters = match action_parameters(form) {
            Ok(parameters) => parameters,
            Err(sentence) => {
                self.say_action(&sentence);
                return;
            }
        };
        let offer = form.offer.as_str().to_owned();
        let at = match self.attempts.iter().position(|a| a.offer == offer) {
            Some(at) => at,
            None => {
                if self.attempts.len() >= MAX_ATTEMPTS {
                    let Some(retired) = self
                        .attempts
                        .iter()
                        .position(|a| a.id.is_none() && !a.blocked)
                    else {
                        self.say_action(TOO_MANY);
                        return;
                    };
                    self.attempts.remove(retired);
                }
                self.attempts.push(Attempt {
                    offer: offer.clone(),
                    id: None,
                    message: String::new(),
                    blocked: false,
                });
                self.attempts.len() - 1
            }
        };
        let invoke = InvokeRequest::LegalAction(LegalActionRequest::SubmitSelf {
            offer: form.offer.clone(),
            parameters,
        });
        if let Err(error) = invoke.validate() {
            self.say_action(&format!("Request refused: {error}"));
            return;
        }
        let Some(id) = self.draw_id() else {
            self.say_action(IDS_EXHAUSTED);
            return;
        };
        if let Some(attempt) = self.attempts.get_mut(at) {
            attempt.id = Some(id);
            attempt.blocked = true;
            WAITING.clone_into(&mut attempt.message);
        }
        self.outbox.push((id, invoke));
        self.say_action(WAITING);
        self.form = None;
    }

    /// A correlated result. `true` when the id is the desk's (the market's
    /// pending request or an attempt in flight); a foreign or forgotten id is
    /// ignored and answered `false`, so the workshop may take it.
    pub(super) fn accept(&mut self, id: u64, result: &InvokeResult) -> bool {
        if self.pending.as_ref().is_some_and(|p| p.id == id) {
            let closing = self.pending.take().and_then(|p| p.closing);
            match result {
                InvokeResult::Mind(response) => self.market_reply(response, closing),
                InvokeResult::LegalAction(_)
                | InvokeResult::UnitDesign(_)
                | InvokeResult::ContentStudio(_) => {
                    self.close_market_form();
                    self.say(UNEXPECTED);
                }
            }
            return true;
        }
        let Some(attempt) = self.attempts.iter_mut().find(|a| a.id == Some(id)) else {
            return false;
        };
        attempt.id = None;
        let (sentence, blocked) = match result {
            InvokeResult::LegalAction(response) => action_sentence(response),
            // Never from a conforming gateway; the offer stays blocked, since
            // nothing proves the submission did not land.
            InvokeResult::Mind(_)
            | InvokeResult::UnitDesign(_)
            | InvokeResult::ContentStudio(_) => {
                let (sentence, _) =
                    action_sentence(&LegalActionResponse::Unavailable { reference: None });
                (sentence, true)
            }
        };
        sentence.clone_into(&mut attempt.message);
        attempt.blocked = blocked;
        self.say_action(sentence);
        true
    }

    fn close_market_form(&mut self) {
        if self.form.as_ref().is_some_and(Form::is_market) {
            self.form = None;
        }
    }

    fn market_reply(&mut self, response: &MindResponse, closing: Option<String>) {
        match response {
            MindResponse::Staged { line } => {
                self.say(&clean(line));
                self.close_market_form();
                if let Some(closing) = closing
                    && !self.form_open()
                {
                    self.open(Form::Notice {
                        title: "Offer sent".to_owned(),
                        lines: vec![closing],
                    });
                }
                let asks = self.page_queries();
                self.queries.extend(asks);
            }
            MindResponse::Shown { lines } => {
                self.say(&lines.first().map(|l| clean(l)).unwrap_or_default());
                let lines: Vec<String> = lines.iter().map(|l| clean(l)).collect();
                match self.form.as_mut() {
                    Some(Form::Accept { shown, .. }) => *shown = lines,
                    Some(Form::Action(_)) => {}
                    _ => {
                        self.open(Form::Notice {
                            title: "Offers".to_owned(),
                            lines,
                        });
                    }
                }
            }
            MindResponse::Error { message, .. } => {
                self.say(&clean(message));
                self.close_market_form();
            }
            _ => {
                self.say(UNEXPECTED);
                self.close_market_form();
            }
        }
    }

    /// The open form's title and rows, top to bottom, as plain text — what
    /// [`Self::draw`] paints, and what a test reads. Nothing here is an
    /// offer token or a command line.
    pub(super) fn form_lines(&self) -> Option<(String, Vec<String>)> {
        let form = self.form.as_ref()?;
        let mark = |on: bool| if on { "> " } else { "  " };
        let choices = |names: &[&str], at: usize| {
            names
                .iter()
                .enumerate()
                .map(|(i, n)| {
                    if i == at {
                        format!("[{n}]")
                    } else {
                        (*n).to_owned()
                    }
                })
                .collect::<Vec<_>>()
                .join("  ")
        };
        let typed = |line: &LineBuffer, on: bool| {
            let mut text = line.text();
            if on {
                text.push('_');
            }
            text
        };
        let mut rows = Vec::new();
        let title = match form {
            Form::Buy {
                code,
                name,
                step,
                confirm,
            } => {
                let names: Vec<&str> = BUY_STEPS.iter().map(|(_, n)| *n).collect();
                rows.push(format!("> Step      {}", choices(&names, *step)));
                rows.extend(confirm.iter().cloned());
                if name.is_empty() {
                    // A code the own strip names, off this page.
                    format!("Buy listing {code}")
                } else {
                    format!("Buy listing {code}: {name}")
                }
            }
            Form::Renew {
                licence,
                name,
                step,
            } => {
                let names: Vec<&str> = RENEW_STEPS.iter().map(|(_, n)| *n).collect();
                rows.push(format!("> Step      {}", choices(&names, *step)));
                rows.push(
                    "The renewal service answers on the scene screen; read the price there before you pay."
                        .to_owned(),
                );
                format!("Renew licence {licence}: {name}")
            }
            Form::Offer {
                kind,
                number,
                name,
                field,
                player,
                price,
            } => {
                rows.push(format!(
                    "{}Player    {}",
                    mark(*field == 0),
                    typed(player, *field == 0)
                ));
                rows.push(format!(
                    "{}Price     {}",
                    mark(*field == 1),
                    typed(price, *field == 1)
                ));
                match kind {
                    Kind::Licence => format!("Offer licence {number}: {name}"),
                    Kind::Filed => format!("Offer filed mind {number} and its charter: {name}"),
                }
            }
            Form::Accept {
                kind,
                field,
                number,
                price,
                shown,
            } => {
                let names: Vec<&str> = KINDS.iter().map(|(_, n)| *n).collect();
                rows.push(format!(
                    "{}Kind      {}",
                    mark(*field == 0),
                    choices(&names, *kind)
                ));
                rows.push(format!(
                    "{}Number    {}",
                    mark(*field == 1),
                    typed(number, *field == 1)
                ));
                rows.push(format!(
                    "{}Price     {}",
                    mark(*field == 2),
                    typed(price, *field == 2)
                ));
                rows.push("An empty price lists the offers that name you.".to_owned());
                rows.extend(shown.iter().cloned());
                "Accept an offer".to_owned()
            }
            Form::Publish {
                filed,
                name,
                field,
                form,
                royalty,
                transfer,
            } => {
                let forms: Vec<&str> = FORMS.iter().map(|(_, n)| *n).collect();
                let yes_no: Vec<&str> = YES_NO.iter().map(|(_, n)| *n).collect();
                rows.push(format!(
                    "{}Form      {}",
                    mark(*field == 0),
                    choices(&forms, *form)
                ));
                rows.push(format!(
                    "{}Royalty   {} per mille (empty is 0)",
                    mark(*field == 1),
                    typed(royalty, *field == 1)
                ));
                rows.push(format!(
                    "{}Transfer  {}",
                    mark(*field == 2),
                    choices(&yes_no, *transfer)
                ));
                format!("List filed mind {filed}: {name}")
            }
            Form::Withdraw { filed, name } => {
                rows.push(format!(
                    "Withdraw filed mind #{filed} {name}? New sales stop; licences already sold keep working. Enter withdraws; Esc cancels."
                ));
                format!("Withdraw filed mind {filed}")
            }
            Form::Action(action) => {
                for (i, field) in action.fields.iter().enumerate() {
                    let on = i == action.field;
                    let value = match &field.kind {
                        ActionKind::Boolean => {
                            choices(&["yes", "no"], field.pick.unwrap_or(usize::MAX))
                        }
                        ActionKind::Choice { choices: names } => {
                            let names: Vec<&str> = names.iter().map(String::as_str).collect();
                            choices(&names, field.pick.unwrap_or(usize::MAX))
                        }
                        ActionKind::Integer { minimum, maximum } => {
                            let range = match (minimum, maximum) {
                                (Some(lo), Some(hi)) => format!("  ({lo} to {hi})"),
                                _ => String::new(),
                            };
                            format!("{}{range}", typed(&field.line, on))
                        }
                        ActionKind::Text { .. } => typed(&field.line, on),
                    };
                    let optional = if field.required { "" } else { " (optional)" };
                    rows.push(format!(
                        "{}{}{optional}  {}",
                        mark(on),
                        clean(&field.id.replace('_', " ")),
                        value
                    ));
                }
                if action.fields.is_empty() {
                    rows.push("This action takes no inputs.".to_owned());
                }
                action.label.clone()
            }
            Form::Notice { title, lines } => {
                rows.extend(lines.iter().cloned());
                title.clone()
            }
        };
        // Only a sentence set while this form was open is its answer; an
        // older one stays on the status line alone.
        let (status, said) = if form.is_market() {
            (&self.status, self.said)
        } else {
            (&self.action_status, self.action_said)
        };
        if said > self.form_from && !status.is_empty() && !matches!(form, Form::Notice { .. }) {
            rows.push(String::new());
            rows.push(status.clone());
        }
        Some((title, rows))
    }

    /// Paint the open form at the foot of `area` (the screen's body), as
    /// the workshop paints its forms: a bordered box over the panes.
    pub(super) fn draw(&self, frame: &mut Frame<'_>, area: Rect) {
        let Some((title, rows)) = self.form_lines() else {
            return;
        };
        if area.width < 4 || area.height < 4 {
            return;
        }
        let Some(form) = self.form.as_ref() else {
            return;
        };
        let foot = match form {
            Form::Notice { .. } => "Enter or Esc closes.",
            _ if form.is_market() && self.pending() => SENT_FOOT,
            Form::Withdraw { .. } => "Enter withdraws; Esc cancels.",
            Form::Action(_) => {
                "Enter submits this action; Esc cancels. Tab moves between inputs; Left/Right chooses."
            }
            _ => {
                "Enter sends this request; Esc cancels. Tab moves between fields; Left/Right chooses."
            }
        };
        let mut lines: Vec<Line<'static>> = vec![Line::from(Span::styled(
            clean(&title),
            Style::default().add_modifier(Modifier::BOLD),
        ))];
        lines.extend(rows.iter().map(|r| Line::from(clean(r))));
        lines.push(Line::from(Span::styled(
            foot.to_owned(),
            Style::default().add_modifier(Modifier::DIM),
        )));
        let want = u16::try_from(lines.len().saturating_add(2)).unwrap_or(u16::MAX);
        let h = want.min(area.height);
        let rect = Rect::new(area.x, area.y + area.height - h, area.width, h);
        frame.render_widget(Clear, rect);
        let label = match form {
            Form::Notice { .. } => "Sent",
            _ if form.is_market() && self.pending() => "Sent - waiting for the server",
            _ => "Not sent",
        };
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .style(Style::default())
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Yellow))
                        .title(label),
                ),
            rect,
        );
    }
}

/// Whether the focused field takes typed text (so Space types a space).
fn typing(form: &Form) -> bool {
    match form {
        Form::Offer { .. } => true,
        Form::Accept { field, .. } => *field != 0,
        Form::Publish { field, .. } => *field == 1,
        Form::Action(a) => a.fields.get(a.field).is_some_and(|f| {
            matches!(f.kind, ActionKind::Integer { .. } | ActionKind::Text { .. })
        }),
        _ => false,
    }
}

/// Left/Right/Space on the focused choice field.
fn choose(form: &mut Form, back: bool) {
    match form {
        Form::Buy { step, .. } => *step = cycle(*step, BUY_STEPS.len(), back),
        Form::Renew { step, .. } => *step = cycle(*step, RENEW_STEPS.len(), back),
        Form::Accept { kind, field: 0, .. } => *kind = cycle(*kind, KINDS.len(), back),
        Form::Publish { form, field: 0, .. } => *form = cycle(*form, FORMS.len(), back),
        Form::Publish {
            transfer, field: 2, ..
        } => *transfer = cycle(*transfer, YES_NO.len(), back),
        Form::Action(action) => {
            if let Some(field) = action.fields.get_mut(action.field) {
                let len = match &field.kind {
                    ActionKind::Boolean => 2,
                    ActionKind::Choice { choices } => choices.len(),
                    _ => return,
                };
                field.pick = Some(match field.pick {
                    None if back => len.saturating_sub(1),
                    None => 0,
                    Some(at) => cycle(at, len, back),
                });
            }
        }
        _ => {}
    }
}

/// A typed key into the focused text field.
fn edit(form: &mut Form, key: Key) {
    let (line, digits, max, touched): (&mut LineBuffer, bool, usize, Option<&mut bool>) = match form
    {
        Form::Offer {
            field: 0, player, ..
        } => (player, false, MAX_FIELD_CHARS, None),
        Form::Offer { price, .. } => (price, false, MAX_FIELD_CHARS, None),
        Form::Accept {
            field: 1, number, ..
        } => (number, true, MAX_DIGITS, None),
        Form::Accept {
            field: 2, price, ..
        } => (price, false, MAX_FIELD_CHARS, None),
        Form::Publish {
            field: 1, royalty, ..
        } => (royalty, true, 5, None),
        Form::Action(action) => {
            let Some(field) = action.fields.get_mut(action.field) else {
                return;
            };
            let max = match field.kind {
                ActionKind::Integer { .. } => 20,
                ActionKind::Text { maximum_bytes } => maximum_bytes,
                _ => return,
            };
            let integer = matches!(field.kind, ActionKind::Integer { .. });
            if integer {
                // Digits and one leading minus; the canonical form is the
                // server's check (`LegalActionInteger`).
                if let Key::Char(c) = key
                    && !(c.is_ascii_digit() || (c == '-' && field.line.cursor() == 0))
                {
                    return;
                }
            }
            (&mut field.line, false, max, Some(&mut field.touched))
        }
        _ => return,
    };
    let changed = match key {
        Key::Char(c) if digits && !c.is_ascii_digit() => false,
        Key::Char(c) if line.len() < max && !c.is_control() => line.insert(c),
        Key::Backspace => line.backspace().is_some(),
        Key::Delete => line.delete().is_some(),
        Key::Left => {
            line.left();
            false
        }
        Key::Right => {
            line.right();
            false
        }
        Key::Home | Key::Ctrl('a') => {
            line.home();
            false
        }
        Key::End | Key::Ctrl('e') => {
            line.end();
            false
        }
        Key::Ctrl('u') => line.kill_to_start(),
        Key::Ctrl('w') => line.kill_word(),
        _ => false,
    };
    if changed && let Some(touched) = touched {
        *touched = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `n` offers on oneself taking no inputs; the `i`-th token is `i` in
    /// sixty-four lowercase digits.
    fn offers(n: usize) -> Value {
        Value::Array(
            (1..=n)
                .map(|i| {
                    json!({
                        "offer": format!("{i:064x}"),
                        "subject": { "kind": "self_actor" },
                        "targets": { "kind": "none" },
                        "parameters": [],
                        "presentation": { "verb": "wait", "label": "Wait", "help": "" }
                    })
                })
                .collect(),
        )
    }

    /// `a` and Enter on the offer at `at`: the id drawn, if one was.
    fn submit(desk: &mut MarketDesk, view: &Value, at: usize) -> Option<u64> {
        desk.open_action(Some(view), Some(at));
        desk.enter();
        desk.take_invokes().into_iter().next().map(|(id, _)| id)
    }

    fn answer(desk: &mut MarketDesk, id: u64, response: LegalActionResponse) {
        assert!(desk.accept(id, &InvokeResult::LegalAction(response)));
    }

    /// **17g cut C3b** (the C3 plan §7: at most sixteen attempts): with every
    /// slot holding an unresolved attempt a new offer's Enter sends nothing
    /// and shows the web's `Too many unresolved actions` sentence; a settled
    /// attempt (here `committed`) is the one retired to make room, and the
    /// unresolved ones are kept.
    #[test]
    fn sixteen_unresolved_attempts_refuse_another_and_a_settled_one_is_retired() {
        let view = offers(MAX_ATTEMPTS + 2);
        let mut desk = MarketDesk::new();
        let settled = format!("{:064x}", 4);
        for at in 0..MAX_ATTEMPTS {
            let id = submit(&mut desk, &view, at).expect("a free slot");
            let response = if at == 3 {
                LegalActionResponse::Rejected {}
            } else {
                LegalActionResponse::Queued {}
            };
            answer(&mut desk, id, response);
        }
        assert_eq!(desk.attempts.len(), MAX_ATTEMPTS);
        // The seventeenth offer retires the settled attempt (offer 4).
        let id = submit(&mut desk, &view, MAX_ATTEMPTS).expect("the settled slot is reused");
        assert!(desk.attempts.iter().all(|a| a.offer != settled));
        assert_eq!(desk.attempts.len(), MAX_ATTEMPTS);
        answer(&mut desk, id, LegalActionResponse::Queued {});
        // The eighteenth finds every slot unresolved: nothing is sent.
        assert_eq!(submit(&mut desk, &view, MAX_ATTEMPTS + 1), None);
        assert_eq!(desk.action_status(), TOO_MANY);
        assert!(desk.form_open(), "the form stays; nothing was sent");
        desk.key(Key::Esc);
        // A replaced view forgets the resolved attempts, which frees them all.
        desk.actions_replaced();
        assert!(desk.attempts.is_empty());
        assert!(submit(&mut desk, &view, MAX_ATTEMPTS + 1).is_some());
    }
}
