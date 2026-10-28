//! 17g C2c (C2 plan §2.4): the market's bounded read behind the line-mode
//! `+minds` commands, and the entitlement actions those commands wrap.
//!
//! **The read.** `MarketCommandReader` is an `OwnerReadService` over the
//! entitlement owner's own listing family (descriptor `owner_read`
//! `mind_market_commands` v1), granted to TacSim alone. It is registered
//! whenever the market composes personal entitlements (`MarketModule::
//! with_mind_entitlements`). Every kind is free of the seller, the author,
//! any account, revision, rating or digest: a listing row becomes a
//! `PublicListingView` (`EntitlementListingV1::command_view`) and nothing
//! else of it leaves.
//!
//! - `AtCounter`: whether the actor stands at a venue the entitlement policy
//!   admits, by exactly the gate the List, Quote, Fund and Grant offers use
//!   (`EntitlementGuard::current`).
//! - `Listings { after }`: up to `LISTINGS_PER_PAGE` (20) policy-current
//!   listings of both protocols in key order, after an optional listing, and
//!   the cursor of the next page. "Current" is `current_policy` only: the
//!   price, cuts and policy are the deployment's now. It is not
//!   seller-specific (a quote still runs the whole of `current_listing`
//!   against Roll's facts). Away from a counter it is `Refused`.
//! - `ListingByCode { code }`: exactly `CODE_CHARACTERS` (8) characters of a
//!   publication token's text. One scan of `CODE_ROWS` (2) rows after the
//!   code, in which only current rows count (17g C2e): no current key
//!   starting with it is `Refused`; one current is `One`, whatever stale row
//!   sits beside it; two current are `Ambiguous`. When both scanned rows
//!   share the code and the family goes on, one probe of the next row tells
//!   whether a third shares it too (then `Ambiguous`), so a `One` is always
//!   the only current listing with the code (`by_code`). No other length is
//!   answered, so no more than the 32 bits `+minds` prints of a token is ever
//!   answerable. It needs no counter (`+minds/buy/refund` works anywhere).
//! - `ListingsAfter { code }` (the C2c fix round, reviewer F4): the page after
//!   the listing a `MORE` cursor names, for `+minds/next`. A `Listings` read
//!   that spends its scan budget on listings that are no longer current
//!   (insert-only rows a policy change left behind) hands back the last
//!   *scanned* key as its cursor, which may be such a listing, so this kind
//!   resolves its code to the first key starting with it, current or not
//!   (one scan of `CODE_ROWS`; none is `Refused`), then pages on with one
//!   scan fewer. A stale listing's code is no secret: it was a public listing
//!   at a counter, and only its existence is answered, never its terms.
//!   Away from a counter it is `Refused`.
//!
//! **The actor's own records (17g C2g, the C2e plan §4.1 items 1 and 4),**
//! over 17g C2f's owner indexes, each written inside the money transaction
//! it follows. Every one answers for `ctx.actor` alone, from the actor's own
//! index row and one batched point read of the rows it names, so another
//! actor's records are never read, and are absent exactly as none are:
//! - `OwnPurchase { code }`: the actor's own quote and funded purchase on the
//!   listing `code` names, matched against the listing token each of the
//!   actor's OWN indexed quotes and operations names
//!   (`market_mind_entitlement_actors`), current listing or not, so a
//!   listing withdrawn after Fund can still be completed or refunded. It
//!   never calls `by_code`: another actor's purchase is the same `None` as
//!   none. At most `MAX_BUYER_QUOTES` + `MAX_BUYER_PURCHASES` rows, three
//!   reads. Its view also names the actor's most recently quoted own record
//!   there (`OwnPurchaseView::latest`), `+minds/buy`'s key subject.
//! - `OwnOpen`: every funded purchase the actor has open (the `+minds/
//!   holdings` waiting lines), with its listing's code, whatever the
//!   listing's currency (a withdrawn listing's escrow is still the actor's).
//! - `OwnQuotes` (17g C3a, the C3 plan §2.3): every quote on the actor's own
//!   index row (at most `MAX_BUYER_QUOTES`), live or expired, newest first
//!   (by `quoted_at_utc`, then token), each with its listing's code, frozen
//!   price and expiry: the `mind.market` panel's own strip, which no other
//!   kind answers (`OwnPurchase` is keyed by a code the panel does not
//!   have). No token, seller, account or digest; it needs no counter.
//! - `ResaleOffers { licence }`: the live seller offers naming the actor on
//!   that licence (`market_mind_paid_resale_buyer_offers` keyed by the
//!   actor and the licence, then each offer row, whose named buyer and
//!   licence are checked), expired ones left out; a licence nobody offered
//!   the actor (a foreign one included) answers the empty list, as a missing
//!   one does. Each view carries the acceptance step (`ResaleKind::Accept`
//!   or `Fund`) by the provider's own claim test, taken from Roll
//!   (`mush_roll::commerce::paid_resale::reader::resale_step`) only for a
//!   licence on which an offer naming the actor was found, so the step is
//!   built on the offer's committed evidence and is no oracle for anyone
//!   else (the C2d fix round's reviewers C2D-R2 and C2D-R3).
//!
//! No own view carries the seller, an account, a digest or another buyer.
//!
//! **Capacity and the grant.** This is a second market-side owner-read
//! service: on the format-2 Active named launch it is one of its 33 services. 17g
//! C2e raised both owner-read caps to 48 (the lead's ruling R1) and TacSim
//! requests the read through `MarketCommandReader::request` in its
//! `register`, beside the entitlement holdings' grants, whenever it composes
//! the line-mode mind commands (the format-2 Active launch only); the launch
//! then holds 33 services and 35 grants (with the scoped passenger facts). A House-only launch requests
//! nothing.

use crate::mind_entitlement::{
    EntitlementActorIndex, EntitlementConfig, EntitlementGuard, EntitlementListingV1,
    EntitlementPurchaseV1, EntitlementQuoteV1, Families, current_policy,
};
use crate::mind_paid_resale::{MindPaidResaleOffer, ResaleBuyerOffers};
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistry,
    DefinitionRegistryBuilder, OwnerId,
};
use mush_core::read_service::{
    OwnerReadContext, OwnerReadError, OwnerReadHandle, OwnerReadService,
};
use mush_core::{CommitToken, DefinitionRef, ModError, ObjId, Registrar};
use mush_economy::managed_entitlement::EntitlementProtocol;
use mush_roll::commerce::paid::PaidForm;
use mush_roll::commerce::purchase::PurchasePhase;
use mush_roll::commerce::rating::RatingClass;
use mush_roll::commerce::{CharterId, LicenceId};
use serde::Serialize;
use std::collections::BTreeSet;
use std::sync::Arc;

/// The largest `Listings` page.
pub const LISTINGS_PER_PAGE: usize = 20;
/// The exact length of a listing code: the first characters of the
/// publication token's text, as `+minds` prints them.
pub const CODE_CHARACTERS: usize = 8;
/// The rows one code lookup reads: enough to tell one from two.
pub const CODE_ROWS: usize = 2;
/// One listing scan's rows, and the most scans one `Listings` read makes
/// (`SCAN_ROWS * SCANS` = 512 = `MAX_OWNER_READ_ROWS`). `ListingsAfter`
/// spends `CODE_ROWS` on its cursor first, so it pages with `SCANS - 1`.
const SCAN_ROWS: usize = 64;
const SCANS: usize = 8;

fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}

/// The two entitlement protocols a command can wrap: original v3 and
/// derived v1 (17g C2 plan §2.2: `@mind/publish` lists v3 only).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    Original,
    Derived,
}
impl Protocol {
    pub(crate) const fn of(protocol: EntitlementProtocol) -> Self {
        match protocol {
            EntitlementProtocol::OriginalV3 => Self::Original,
            EntitlementProtocol::DerivedV1 => Self::Derived,
        }
    }
    /// The definition id prefix and version of this protocol's actions.
    const fn key(self) -> (&'static str, u32) {
        match self {
            Self::Original => ("original", 3),
            Self::Derived => ("derived", 1),
        }
    }
}

/// One step of the entitlement purchase a command submits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    List,
    Quote,
    Fund,
    Grant,
    Refund,
}
impl Step {
    /// The registered action id's verb (`mind_entitlement::actions`'
    /// presentation table: fund is `purchase_*`, grant is `settle_*`).
    const fn id(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Quote => "quote",
            Self::Fund => "purchase",
            Self::Grant => "settle",
            Self::Refund => "refund",
        }
    }
}

/// A listing's paid form, as the commands print it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ListingForm {
    Sealed,
    Open,
    OpenResale,
}
impl ListingForm {
    pub(crate) const fn of(form: PaidForm) -> Self {
        match form {
            PaidForm::Sealed {} => Self::Sealed,
            PaidForm::Open {} => Self::Open,
            PaidForm::OpenResale {} => Self::OpenResale,
        }
    }
}

/// 17g C2c: the entitlement actions the market commands wrap, captured from
/// the live definition registry. A captured reference is a selector only:
/// `Ctx::submit_authoritative` resolves it against the actor's own projected
/// offers and the provider rechecks every authority (C2 plan §2.1).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MarketCommandActions {
    actions: Vec<(Protocol, Step, DefinitionRef)>,
    /// 17g C2d: the paid-resale Offer, Accept and Fund, in that order.
    resale: [Option<DefinitionRef>; 3],
}

/// 17g C2d (C2 plan §2.6): one step of a named paid-licence resale the
/// commands submit. The seller's `+minds/resell` is `Offer`; the named
/// buyer's `+minds/resell/accept` is `Accept` when the licence has no open
/// paid claim and `Fund` otherwise (the provider's own test; Roll's
/// `ResaleStep` read that answers which is held with the accept, the C2d
/// fix round).
///
/// 17g C2g: a `ResaleOffers` view carries the named buyer's step, `Accept`
/// or `Fund`, by the provider's own claim test (Roll's
/// `mush_roll::commerce::paid_resale::reader::resale_step`, asked only once
/// an offer naming the actor was found).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResaleKind {
    Offer,
    Accept,
    Fund,
}
impl ResaleKind {
    /// The registered action id (`mush_roll::commerce::paid_resale::protocol`).
    const fn id(self) -> &'static str {
        use mush_roll::commerce::paid_resale::protocol;
        match self {
            Self::Offer => protocol::OFFER,
            Self::Accept => protocol::ACCEPT,
            Self::Fund => protocol::FUND,
        }
    }
    const fn index(self) -> usize {
        match self {
            Self::Offer => 0,
            Self::Accept => 1,
            Self::Fund => 2,
        }
    }
}
impl MarketCommandActions {
    /// Read each wrapped action from the registry: owner `market`, kind
    /// `action`, `<step>_<protocol>_mind` at the protocol's version. A key
    /// the market did not register, or registered twice, is absent.
    #[must_use]
    pub fn capture(definitions: &DefinitionRegistry) -> Self {
        let mut actions = Vec::new();
        for protocol in [Protocol::Original, Protocol::Derived] {
            let (product, version) = protocol.key();
            for step in [
                Step::List,
                Step::Quote,
                Step::Fund,
                Step::Grant,
                Step::Refund,
            ] {
                let id = format!("{}_{product}_mind", step.id());
                let mut found = definitions
                    .iter()
                    .map(|entry| entry.reference())
                    .filter(|r| {
                        let key = r.key();
                        key.owner.as_str() == crate::NS
                            && key.kind.as_str() == "action"
                            && key.id.as_str() == id
                            && r.version() == version
                    });
                if let (Some(reference), None) = (found.next(), found.next()) {
                    actions.push((protocol, step, reference.clone()));
                }
            }
        }
        // 17g C2d: the paid-resale steps, owner `market`, version 1.
        let mut resale: [Option<DefinitionRef>; 3] = [None, None, None];
        for kind in [ResaleKind::Offer, ResaleKind::Accept, ResaleKind::Fund] {
            let mut found = definitions
                .iter()
                .map(|entry| entry.reference())
                .filter(|r| {
                    let key = r.key();
                    key.owner.as_str() == crate::NS
                        && key.kind.as_str() == "action"
                        && key.id.as_str() == kind.id()
                        && r.version() == 1
                });
            if let (Some(reference), None) = (found.next(), found.next())
                && let Some(slot) = resale.get_mut(kind.index())
            {
                *slot = Some(reference.clone());
            }
        }
        Self { actions, resale }
    }
    /// 17g C2d: the registered paid-resale action of `kind`, or `None` when
    /// this composition does not register it (the command refuses
    /// `NOT_OPEN`).
    #[must_use]
    pub fn resale(&self, kind: ResaleKind) -> Option<DefinitionRef> {
        self.resale.get(kind.index()).cloned().flatten()
    }
    /// The registered action of `step` in `protocol`, or `None` when this
    /// composition does not register it (the command refuses `NOT_OPEN`).
    #[must_use]
    pub fn entitlement(&self, protocol: Protocol, step: Step) -> Option<DefinitionRef> {
        self.actions
            .iter()
            .find(|(p, s, _)| *p == protocol && *s == step)
            .map(|(_, _, reference)| reference.clone())
    }
}

/// What `+minds` may show of one current listing (C2 plan §2.8). The charter
/// is a public Roll entry; the name is read from Roll by the command. There
/// is no seller, author, account, revision, rating or digest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PublicListingView {
    pub(crate) publication: CommitToken,
    pub(crate) protocol: Protocol,
    pub(crate) charter: CharterId,
    pub(crate) class: RatingClass,
    pub(crate) form: ListingForm,
    pub(crate) transferable: bool,
    pub(crate) term_seconds: i64,
    pub(crate) grace_seconds: i64,
    pub(crate) price_minor: i64,
    pub(crate) levy_permille: u16,
    pub(crate) venue_permille: u16,
    pub(crate) parent_royalty_permille: u16,
    pub(crate) derived: bool,
}
impl PublicListingView {
    /// The listing's publication token (the Quote's `listing`).
    #[must_use]
    pub const fn publication(&self) -> CommitToken {
        self.publication
    }
    /// Its code: the first `CODE_CHARACTERS` characters of the token text.
    #[must_use]
    pub fn code(&self) -> String {
        self.publication
            .to_string()
            .chars()
            .take(CODE_CHARACTERS)
            .collect()
    }
    #[must_use]
    pub const fn protocol(&self) -> Protocol {
        self.protocol
    }
    #[must_use]
    pub const fn charter(&self) -> CharterId {
        self.charter
    }
    /// The class a buyer's licence carries ("class II and above").
    #[must_use]
    pub const fn class(&self) -> RatingClass {
        self.class
    }
    #[must_use]
    pub const fn form(&self) -> ListingForm {
        self.form
    }
    /// The frozen product's transferability (the author's mark under the
    /// deployment policy, 17g C2b).
    #[must_use]
    pub const fn transferable(&self) -> bool {
        self.transferable
    }
    #[must_use]
    pub const fn term_seconds(&self) -> i64 {
        self.term_seconds
    }
    #[must_use]
    pub const fn grace_seconds(&self) -> i64 {
        self.grace_seconds
    }
    #[must_use]
    pub const fn price_minor(&self) -> i64 {
        self.price_minor
    }
    #[must_use]
    pub const fn levy_permille(&self) -> u16 {
        self.levy_permille
    }
    #[must_use]
    pub const fn venue_permille(&self) -> u16 {
        self.venue_permille
    }
    #[must_use]
    pub const fn parent_royalty_permille(&self) -> u16 {
        self.parent_royalty_permille
    }
    #[must_use]
    pub const fn derived(&self) -> bool {
        self.derived
    }
}

/// One bounded request. There is deliberately no Deserialize: only the
/// granted TacSim command adapter builds one.
#[derive(Serialize)]
pub struct MarketCommandRequest {
    kind: ReadKind,
}
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ReadKind {
    AtCounter,
    Listings { after: Option<CommitToken> },
    ListingByCode { code: String },
    ListingsAfter { code: String },
    OwnPurchase { code: String },
    OwnOpen,
    ResaleOffers { licence: LicenceId },
    OwnQuotes,
}
impl MarketCommandRequest {
    /// 17g C3a: every quote on the actor's own index row, newest first.
    #[must_use]
    pub const fn own_quotes() -> Self {
        Self {
            kind: ReadKind::OwnQuotes,
        }
    }
    /// 17g C2g: the actor's own quote and funded purchase on the listing
    /// whose code is `code`, through the actor's own index only.
    #[must_use]
    pub fn own_purchase(code: &str) -> Self {
        Self {
            kind: ReadKind::OwnPurchase {
                code: code.to_owned(),
            },
        }
    }
    /// 17g C2g: every funded purchase the actor has open.
    #[must_use]
    pub const fn own_open() -> Self {
        Self {
            kind: ReadKind::OwnOpen,
        }
    }
    /// 17g C2g: the live resale offers naming the actor on `licence`.
    #[must_use]
    pub const fn resale_offers(licence: LicenceId) -> Self {
        Self {
            kind: ReadKind::ResaleOffers { licence },
        }
    }
    /// Whether the actor stands at an admitting counter.
    #[must_use]
    pub const fn at_counter() -> Self {
        Self {
            kind: ReadKind::AtCounter,
        }
    }
    /// The current listings after `after` (from the first with `None`).
    #[must_use]
    pub const fn listings(after: Option<CommitToken>) -> Self {
        Self {
            kind: ReadKind::Listings { after },
        }
    }
    /// The current listing whose token text starts with `code`.
    #[must_use]
    pub fn listing_by_code(code: &str) -> Self {
        Self {
            kind: ReadKind::ListingByCode {
                code: code.to_owned(),
            },
        }
    }
    /// The current listings after the listing `code` names, current or not:
    /// `+minds/next <code>` with the cursor a `MORE` line printed.
    #[must_use]
    pub fn listings_after(code: &str) -> Self {
        Self {
            kind: ReadKind::ListingsAfter {
                code: code.to_owned(),
            },
        }
    }
}

/// One `Listings` page and the listing to page after, when there is more.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ListingsPage {
    rows: Vec<PublicListingView>,
    next: Option<CommitToken>,
}
impl ListingsPage {
    #[must_use]
    pub fn rows(&self) -> &[PublicListingView] {
        &self.rows
    }
    /// Where the next page starts, `None` at the end: the last row's token
    /// when a further current listing exists, or, when the scan budget ended
    /// before the family did, the last listing scanned, current or not (so a
    /// page can be empty and still go on). `+minds` prints its code in `MORE`
    /// and `ListingsAfter` resolves it.
    #[must_use]
    pub const fn next(&self) -> Option<CommitToken> {
        self.next
    }
}

/// A code lookup's answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "found", rename_all = "snake_case")]
pub enum CodeLookup {
    One(PublicListingView),
    Ambiguous,
}

/// 17g C2g: the actor's own unfunded quote on a listing: its token (the
/// Fund's `quote`), its frozen price and when it stops being fundable (world
/// UTC seconds; the provider refuses a Fund at or after it). It may have
/// expired: an abandoned quote stays indexed until the actor's next Quote.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct OwnQuoteView {
    quote: CommitToken,
    price_minor: i64,
    expires_at_utc: i64,
}
impl OwnQuoteView {
    #[must_use]
    pub const fn quote(&self) -> CommitToken {
        self.quote
    }
    #[must_use]
    pub const fn price_minor(&self) -> i64 {
        self.price_minor
    }
    #[must_use]
    pub const fn expires_at_utc(&self) -> i64 {
        self.expires_at_utc
    }
}

/// 17g C2g: the actor's own record on one listing (`OwnPurchase`): the
/// listing's code and protocol, the NEWEST indexed quote on it (live or
/// expired) and the earliest-quoted funded purchase on it (its operation
/// token, the Grant's and Refund's `operation`). At least one of the two is
/// present. No seller, account, digest or other buyer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OwnPurchaseView {
    code: String,
    protocol: Protocol,
    quote: Option<OwnQuoteView>,
    funded: Option<CommitToken>,
    latest: CommitToken,
}
impl OwnPurchaseView {
    /// The listing's code, as the actor typed it.
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }
    /// The entitlement protocol the steps submit through.
    #[must_use]
    pub const fn protocol(&self) -> Protocol {
        self.protocol
    }
    #[must_use]
    pub const fn quote(&self) -> Option<OwnQuoteView> {
        self.quote
    }
    /// The funded purchase's operation token, when the actor has paid.
    #[must_use]
    pub const fn funded(&self) -> Option<CommitToken> {
        self.funded
    }
    /// 17g C2g (the C2g fix round 2, reviewers correctness/C2G-R2-1,
    /// conformance/F1 and tests/T-1): the token of the actor's most recently
    /// quoted own record on the listing, among ALL its indexed quotes and
    /// funded purchases there: a quote's own token or a funded purchase's
    /// operation token, ordered by the quote's `quoted_at_utc`, a quote
    /// before a purchase quoted in the same second (a purchase's quote was
    /// quoted before the purchase's Fund), then by token. Every Quote and
    /// every Fund the actor commits on the listing makes it a token never
    /// seen there before; a Grant or a Refund, which take a funded purchase
    /// out of the index, can make it an earlier record's token again.
    /// `+minds/buy`'s idempotency subject (tacsim's `buy_key`).
    #[must_use]
    pub const fn latest(&self) -> CommitToken {
        self.latest
    }
}

/// 17g C2g: one funded purchase the actor has open (`OwnOpen`): its
/// operation token, its listing's code and its protocol.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OwnOpenView {
    operation: CommitToken,
    code: String,
    protocol: Protocol,
}
impl OwnOpenView {
    #[must_use]
    pub const fn operation(&self) -> CommitToken {
        self.operation
    }
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }
    #[must_use]
    pub const fn protocol(&self) -> Protocol {
        self.protocol
    }
}

/// 17g C3a: one quote on the actor's own index row (`OwnQuotes`): its
/// listing's code, its frozen price and when it stops being fundable (world
/// UTC seconds; it may have expired). No quote token: a typed `Pay` names
/// the code, and `/buy/pay` resolves the newest live quote there itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OwnQuoteRowView {
    code: String,
    price_minor: i64,
    expires_at_utc: i64,
}
impl OwnQuoteRowView {
    /// The listing's code, as `+minds` prints it.
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }
    #[must_use]
    pub const fn price_minor(&self) -> i64 {
        self.price_minor
    }
    #[must_use]
    pub const fn expires_at_utc(&self) -> i64 {
        self.expires_at_utc
    }
}

/// 17g C2g: one live seller offer naming the actor on a licence
/// (`ResaleOffers`): its token (the Accept's and Fund's `offer`), the frozen
/// price, when it expires (world UTC seconds; the economy refuses an
/// acceptance at or after it) and the acceptance step by the provider's own
/// claim test. No seller, account, digest or other buyer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ResaleOfferView {
    offer: CommitToken,
    price_minor: i64,
    expires_at_utc: i64,
    step: ResaleKind,
}
impl ResaleOfferView {
    #[must_use]
    pub const fn offer(&self) -> CommitToken {
        self.offer
    }
    #[must_use]
    pub const fn price_minor(&self) -> i64 {
        self.price_minor
    }
    #[must_use]
    pub const fn expires_at_utc(&self) -> i64 {
        self.expires_at_utc
    }
    /// `ResaleKind::Accept` or `ResaleKind::Fund`.
    #[must_use]
    pub const fn step(&self) -> ResaleKind {
        self.step
    }
}

/// One result per request kind.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MarketCommandResult {
    AtCounter {
        here: bool,
    },
    Listings(ListingsPage),
    ListingByCode(CodeLookup),
    /// 17g C2g: `OwnPurchase`, `None` when the actor has no record on it.
    OwnPurchase {
        found: Option<OwnPurchaseView>,
    },
    /// 17g C2g: `OwnOpen`, in operation token order.
    OwnOpen {
        open: Vec<OwnOpenView>,
    },
    /// 17g C2g: `ResaleOffers`, in offer token order.
    ResaleOffers {
        offers: Vec<ResaleOfferView>,
    },
    /// 17g C3a: `OwnQuotes`, newest first.
    OwnQuotes {
        quotes: Vec<OwnQuoteRowView>,
    },
}
impl MarketCommandResult {
    /// The counter answer, or `None` if this answered another kind.
    #[must_use]
    pub const fn at_counter(&self) -> Option<bool> {
        match self {
            Self::AtCounter { here } => Some(*here),
            _ => None,
        }
    }
    #[must_use]
    pub fn into_listings(self) -> Option<ListingsPage> {
        match self {
            Self::Listings(page) => Some(page),
            _ => None,
        }
    }
    #[must_use]
    pub fn into_listing(self) -> Option<CodeLookup> {
        match self {
            Self::ListingByCode(found) => Some(found),
            _ => None,
        }
    }
    /// 17g C2g: the own-purchase answer (`Some(None)`: no own record), or
    /// `None` for another kind.
    #[must_use]
    pub fn into_own_purchase(self) -> Option<Option<OwnPurchaseView>> {
        match self {
            Self::OwnPurchase { found } => Some(found),
            _ => None,
        }
    }
    /// 17g C2g: the open purchases, or `None` for another kind.
    #[must_use]
    pub fn into_own_open(self) -> Option<Vec<OwnOpenView>> {
        match self {
            Self::OwnOpen { open } => Some(open),
            _ => None,
        }
    }
    /// 17g C2g: the live offers naming the actor, or `None` for another kind.
    #[must_use]
    pub fn into_resale_offers(self) -> Option<Vec<ResaleOfferView>> {
        match self {
            Self::ResaleOffers { offers } => Some(offers),
            _ => None,
        }
    }
    /// 17g C3a: the actor's own quotes, or `None` for another kind.
    #[must_use]
    pub fn into_own_quotes(self) -> Option<Vec<OwnQuoteRowView>> {
        match self {
            Self::OwnQuotes { quotes } => Some(quotes),
            _ => None,
        }
    }
}

/// The registered read. No public constructor: the market builds it when it
/// composes entitlements.
pub struct MarketCommandReader {
    families: Families,
    /// 17g C2g: the paid-resale families `ResaleOffers` reads, when the
    /// market composes paid resale (`Unavailable` otherwise).
    resale: Option<crate::mind_paid_resale::Families>,
    guard: EntitlementGuard,
    config: EntitlementConfig,
}
impl MarketCommandReader {
    /// TacSim's grant, computed from its own frozen manifest; the market's
    /// registration names exactly the same definition.
    pub fn request(
        reg: &mut Registrar<'_>,
        manifest: &DefinitionRef,
    ) -> Result<OwnerReadHandle<Self>, ModError> {
        reg.request_owner_read::<Self>(definition(manifest)?)
    }
}

pub(crate) fn descriptor(manifest: &DefinitionRef) -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new("owner_read").map_err(error)?,
        DefinitionId::new("mind_market_commands").map_err(error)?,
        1,
        serde_json::json!({
            "format": 1,
            "listings_per_page": LISTINGS_PER_PAGE,
            "code_characters": CODE_CHARACTERS,
            "code_rows": CODE_ROWS,
            "scan_rows": SCAN_ROWS,
            "scans": SCANS,
            "seller": false,
            "account_ids": false,
            "digests": false,
            "manifest": manifest,
        }),
    )
    .map_err(error)
    .map(|d| d.with_references(vec![manifest.clone()]))
}
fn definition(manifest: &DefinitionRef) -> Result<DefinitionRef, ModError> {
    DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new(crate::NS).map_err(error)?,
            descriptor(manifest)?,
        )
        .map_err(error)
}

/// Register the read beside the entitlement families (the market's own
/// `register`, only when entitlements are composed). Granted to TacSim.
/// 17g C2g: `resale` is the paid-resale families when the market composes
/// paid resale, so the market registers the read after them.
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    families: Families,
    resale: Option<crate::mind_paid_resale::Families>,
    config: &EntitlementConfig,
    gate: Arc<dyn crate::VenueGate>,
) -> Result<(), ModError> {
    let declaration = descriptor(config.manifest())?;
    reg.definition_validator(
        DefinitionKind::new("owner_read").map_err(error)?,
        crate::lot::exact_validator(std::slice::from_ref(&declaration)),
    )?;
    let reference = reg.definition(declaration)?;
    let tacsim = OwnerId::new("tacsim").map_err(error)?;
    reg.owner_read_service(
        reference,
        BTreeSet::from([tacsim.clone()]),
        BTreeSet::from([tacsim]),
        MarketCommandReader {
            families,
            resale,
            guard: EntitlementGuard::new(config.clone(), gate)?,
            config: config.clone(),
        },
    )
}

impl OwnerReadService for MarketCommandReader {
    type State = crate::MarketStore;
    type Request = MarketCommandRequest;
    type Output = MarketCommandResult;
    fn read(
        &self,
        _: &crate::MarketStore,
        ctx: &OwnerReadContext<'_>,
        request: MarketCommandRequest,
    ) -> Result<MarketCommandResult, OwnerReadError> {
        let lens = Lens {
            ctx,
            families: &self.families,
            resale: self.resale.as_ref(),
        };
        answer(
            &lens,
            Own {
                actor: ctx.actor(),
                now: ctx.world().now(),
            },
            || self.guard.current(ctx.world(), ctx.actor()).is_ok(),
            &self.config,
            request,
        )
    }
}

/// 17g C2g: whom the own kinds answer for (always the dispatching actor,
/// from core) and the world clock (UTC seconds) a resale offer's expiry is
/// read against.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Own {
    pub(crate) actor: ObjId,
    pub(crate) now: i64,
}

/// The read's answer over `source`: `here` is the venue gate for the actor
/// (`EntitlementGuard::current`), and "current" is `current_policy` against
/// the deployment's `config`. `read` calls this with the owner lens; the
/// tests call it over an in-memory family, so the counter rule and the policy
/// filter they check are the registered read's own. 17g C2g: the own kinds
/// answer for `own.actor` only.
pub(crate) fn answer<'a>(
    source: &(impl ListingSource<'a> + OwnSource<'a>),
    own: Own,
    here: impl Fn() -> bool,
    config: &EntitlementConfig,
    request: MarketCommandRequest,
) -> Result<MarketCommandResult, OwnerReadError> {
    let current = |row: &EntitlementListingV1| current_policy(config, row).is_ok();
    match request.kind {
        ReadKind::OwnPurchase { code } => own_purchase(source, own.actor, &code)
            .map(|found| MarketCommandResult::OwnPurchase { found }),
        ReadKind::OwnOpen => {
            own_open(source, own.actor).map(|open| MarketCommandResult::OwnOpen { open })
        }
        ReadKind::ResaleOffers { licence } => resale_offers(source, own, licence)
            .map(|offers| MarketCommandResult::ResaleOffers { offers }),
        ReadKind::OwnQuotes => {
            own_quotes(source, own.actor).map(|quotes| MarketCommandResult::OwnQuotes { quotes })
        }
        ReadKind::AtCounter => Ok(MarketCommandResult::AtCounter { here: here() }),
        ReadKind::Listings { after } => {
            if !here() {
                return Err(OwnerReadError::Refused);
            }
            listings(source, after.map(|t| t.to_string()), SCANS, current)
                .map(MarketCommandResult::Listings)
        }
        ReadKind::ListingsAfter { code } => {
            if !here() {
                return Err(OwnerReadError::Refused);
            }
            listings_after(source, &code, current).map(MarketCommandResult::Listings)
        }
        ReadKind::ListingByCode { code } => {
            by_code(source, &code, current).map(MarketCommandResult::ListingByCode)
        }
    }
}

/// One bounded scan of the listing family: the rows in key order after
/// `after` and whether the family continues past them. Each row's key is its
/// own publication token (checked), so a row's code is its key's prefix.
pub(crate) trait ListingSource<'a> {
    fn scan(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<&'a EntitlementListingV1>, bool), OwnerReadError>;
}
struct Lens<'c, 'a> {
    ctx: &'c OwnerReadContext<'a>,
    families: &'c Families,
    resale: Option<&'c crate::mind_paid_resale::Families>,
}
impl<'a> ListingSource<'a> for Lens<'_, 'a> {
    fn scan(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<&'a EntitlementListingV1>, bool), OwnerReadError> {
        let page = self
            .ctx
            .scan_action_state(self.families.listings(), after, limit)?;
        let mut rows = Vec::with_capacity(page.entries().len());
        for entry in page.entries() {
            if entry.encoded_key() != entry.value().publication().to_string() {
                return Err(OwnerReadError::Integrity);
            }
            rows.push(entry.value());
        }
        Ok((rows, page.next_after().is_some()))
    }
}

/// 17g C2g: the point reads the own kinds make, each keyed by the actor or
/// by a token the actor's own index row names. `read` answers them through
/// the owner lens; the tests through in-memory rows.
pub(crate) trait OwnSource<'a> {
    /// The actor's own entitlement index row (`market_mind_entitlement_
    /// actors`, keyed by the actor).
    fn actor_row(&self, actor: ObjId) -> Result<Option<&'a EntitlementActorIndex>, OwnerReadError>;
    /// The quote rows `tokens` name, in order (one batched read).
    fn quotes(
        &self,
        tokens: &[CommitToken],
    ) -> Result<Vec<Option<&'a EntitlementQuoteV1>>, OwnerReadError>;
    /// The purchase rows `tokens` name, in order (one batched read).
    fn operations(
        &self,
        tokens: &[CommitToken],
    ) -> Result<Vec<Option<&'a EntitlementPurchaseV1>>, OwnerReadError>;
    /// The actor's own `(buyer, licence)` resale offer index row.
    fn buyer_offers(
        &self,
        buyer: ObjId,
        licence: LicenceId,
    ) -> Result<Option<&'a ResaleBuyerOffers>, OwnerReadError>;
    /// The seller offer rows `tokens` name, in order (one batched read).
    fn resale_offers(
        &self,
        tokens: &[CommitToken],
    ) -> Result<Vec<Option<&'a MindPaidResaleOffer>>, OwnerReadError>;
    /// The acceptance step for `licence` by the provider's own claim test
    /// (Roll's `resale_step`), asked only once an offer naming the actor on
    /// it was found.
    fn resale_step(&self, licence: LicenceId) -> Result<ResaleKind, OwnerReadError>;
}
impl<'a> OwnSource<'a> for Lens<'_, 'a> {
    fn actor_row(&self, actor: ObjId) -> Result<Option<&'a EntitlementActorIndex>, OwnerReadError> {
        self.ctx.action_state(self.families.actors(), &actor)
    }
    fn quotes(
        &self,
        tokens: &[CommitToken],
    ) -> Result<Vec<Option<&'a EntitlementQuoteV1>>, OwnerReadError> {
        if tokens.is_empty() {
            return Ok(Vec::new());
        }
        self.ctx.action_states(self.families.quotes(), tokens)
    }
    fn operations(
        &self,
        tokens: &[CommitToken],
    ) -> Result<Vec<Option<&'a EntitlementPurchaseV1>>, OwnerReadError> {
        if tokens.is_empty() {
            return Ok(Vec::new());
        }
        self.ctx.action_states(self.families.operations(), tokens)
    }
    fn buyer_offers(
        &self,
        buyer: ObjId,
        licence: LicenceId,
    ) -> Result<Option<&'a ResaleBuyerOffers>, OwnerReadError> {
        let resale = self.resale.ok_or(OwnerReadError::Unavailable)?;
        self.ctx
            .action_state(resale.buyer_offers(), &(buyer, licence))
    }
    fn resale_offers(
        &self,
        tokens: &[CommitToken],
    ) -> Result<Vec<Option<&'a MindPaidResaleOffer>>, OwnerReadError> {
        let resale = self.resale.ok_or(OwnerReadError::Unavailable)?;
        if tokens.is_empty() {
            return Ok(Vec::new());
        }
        self.ctx.action_states(resale.offers(), tokens)
    }
    fn resale_step(&self, licence: LicenceId) -> Result<ResaleKind, OwnerReadError> {
        use mush_roll::commerce::paid_resale::reader::{ResaleStep, resale_step};
        match resale_step(self.ctx.world(), licence) {
            Ok(ResaleStep::Accept) => Ok(ResaleKind::Accept),
            Ok(ResaleStep::Fund) => Ok(ResaleKind::Fund),
            Err(_) => Err(OwnerReadError::Unavailable),
        }
    }
}

/// The listing code of a listing token: its first `CODE_CHARACTERS`
/// characters, as `PublicListingView::code` prints it.
fn code_of(listing: CommitToken) -> String {
    listing.to_string().chars().take(CODE_CHARACTERS).collect()
}

/// The actor's own quotes and funded purchases.
type OwnRows<'a> = (Vec<&'a EntitlementQuoteV1>, Vec<&'a EntitlementPurchaseV1>);

/// The actor's own quotes and funded purchases from its index row, each row
/// checked against the index (its own token, this buyer, a funded phase).
fn own_rows<'a>(source: &impl OwnSource<'a>, actor: ObjId) -> Result<OwnRows<'a>, OwnerReadError> {
    let Some(row) = source.actor_row(actor)? else {
        return Ok((Vec::new(), Vec::new()));
    };
    if row.actor() != actor
        || row.quotes().len() > crate::mind_entitlement::MAX_BUYER_QUOTES
        || row.operations().len() > crate::mind_entitlement::MAX_BUYER_PURCHASES
    {
        return Err(OwnerReadError::Integrity);
    }
    let quote_tokens: Vec<CommitToken> = row.quotes().iter().copied().collect();
    let operation_tokens: Vec<CommitToken> = row.operations().iter().copied().collect();
    let mut quotes = Vec::with_capacity(quote_tokens.len());
    for (token, quote) in quote_tokens.iter().zip(source.quotes(&quote_tokens)?) {
        let quote = quote.ok_or(OwnerReadError::Integrity)?;
        if quote.publication() != *token || quote.buyer() != actor {
            return Err(OwnerReadError::Integrity);
        }
        quotes.push(quote);
    }
    let mut operations = Vec::with_capacity(operation_tokens.len());
    for (token, operation) in operation_tokens
        .iter()
        .zip(source.operations(&operation_tokens)?)
    {
        let operation = operation.ok_or(OwnerReadError::Integrity)?;
        if operation.operation() != *token
            || operation.quote_row().buyer() != actor
            || operation.phase() != (PurchasePhase::Funded {})
        {
            return Err(OwnerReadError::Integrity);
        }
        operations.push(operation);
    }
    Ok((quotes, operations))
}

/// 17g C2g: `OwnPurchase { code }` (module doc). Only the actor's own index
/// is read; `by_code` never is. The code's listing is the one whose token
/// starts with it among the actor's own records; two of the actor's own
/// listings sharing the 8 characters are `Refused` (the command then reads
/// as having no record), as is a malformed code.
pub(crate) fn own_purchase<'a>(
    source: &impl OwnSource<'a>,
    actor: ObjId,
    code: &str,
) -> Result<Option<OwnPurchaseView>, OwnerReadError> {
    if !is_code(code) {
        return Err(OwnerReadError::Refused);
    }
    let (quotes, operations) = own_rows(source, actor)?;
    let matches = |listing: CommitToken| listing.to_string().starts_with(code);
    let quotes: Vec<_> = quotes
        .into_iter()
        .filter(|q| matches(q.listing()))
        .collect();
    let operations: Vec<_> = operations
        .into_iter()
        .filter(|p| matches(p.quote_row().listing()))
        .collect();
    let listings: BTreeSet<CommitToken> = quotes
        .iter()
        .map(|q| q.listing())
        .chain(operations.iter().map(|p| p.quote_row().listing()))
        .collect();
    if listings.len() > 1 {
        return Err(OwnerReadError::Refused);
    }
    let newest = quotes
        .iter()
        .max_by_key(|q| (q.quoted_at_utc(), q.publication()));
    let earliest = operations
        .iter()
        .min_by_key(|p| (p.quote_row().quoted_at_utc(), p.operation()));
    // `OwnPurchaseView::latest`: a quote ranks after a purchase quoted in
    // the same second (`1` over `0`).
    let latest = quotes
        .iter()
        .map(|q| ((q.quoted_at_utc(), 1_u8), q.publication()))
        .chain(
            operations
                .iter()
                .map(|p| ((p.quote_row().quoted_at_utc(), 0_u8), p.operation())),
        )
        .max();
    let (protocol, latest) = match (newest, earliest, latest) {
        (Some(quote), _, Some((_, latest))) => (quote.protocol(), latest),
        (None, Some(operation), Some((_, latest))) => (operation.quote_row().protocol(), latest),
        _ => return Ok(None),
    };
    Ok(Some(OwnPurchaseView {
        code: code.to_owned(),
        protocol: Protocol::of(protocol),
        quote: newest.map(|q| OwnQuoteView {
            quote: q.publication(),
            price_minor: q.price_minor(),
            expires_at_utc: q.expires_at_utc(),
        }),
        funded: earliest.map(|p| p.operation()),
        latest,
    }))
}

/// 17g C2g: `OwnOpen` (module doc): every funded purchase on the actor's
/// own index row, current listing or not.
pub(crate) fn own_open<'a>(
    source: &impl OwnSource<'a>,
    actor: ObjId,
) -> Result<Vec<OwnOpenView>, OwnerReadError> {
    let (_, operations) = own_rows(source, actor)?;
    Ok(operations
        .into_iter()
        .map(|p| OwnOpenView {
            operation: p.operation(),
            code: code_of(p.quote_row().listing()),
            protocol: Protocol::of(p.quote_row().protocol()),
        })
        .collect())
}

/// 17g C3a: `OwnQuotes` (module doc): every quote on the actor's own index
/// row, live or expired, newest first by `quoted_at_utc` then token, each
/// with its listing's code. `own_rows` bounds them to `MAX_BUYER_QUOTES`.
pub(crate) fn own_quotes<'a>(
    source: &impl OwnSource<'a>,
    actor: ObjId,
) -> Result<Vec<OwnQuoteRowView>, OwnerReadError> {
    let (mut quotes, _) = own_rows(source, actor)?;
    quotes.sort_by_key(|q| std::cmp::Reverse((q.quoted_at_utc(), q.publication())));
    Ok(quotes
        .into_iter()
        .map(|q| OwnQuoteRowView {
            code: code_of(q.listing()),
            price_minor: q.price_minor(),
            expires_at_utc: q.expires_at_utc(),
        })
        .collect())
}

/// 17g C2g: `ResaleOffers { licence }` (module doc): the actor's own
/// `(buyer, licence)` index row and the offer rows it names, each checked to
/// name this buyer and licence; an offer at or past its expiry at `own.now`
/// is left out. The step is asked of Roll only when a live offer exists.
pub(crate) fn resale_offers<'a>(
    source: &impl OwnSource<'a>,
    own: Own,
    licence: LicenceId,
) -> Result<Vec<ResaleOfferView>, OwnerReadError> {
    let Some(row) = source.buyer_offers(own.actor, licence)? else {
        return Ok(Vec::new());
    };
    if row.buyer() != own.actor || row.licence() != licence {
        return Err(OwnerReadError::Integrity);
    }
    let tokens: Vec<CommitToken> = row.offers().iter().copied().collect();
    let mut live = Vec::new();
    for (token, offer) in tokens.iter().zip(source.resale_offers(&tokens)?) {
        let offer = offer.ok_or(OwnerReadError::Integrity)?;
        if offer.operation() != *token
            || offer.terms().buyer != own.actor
            || offer.binding().licence() != licence
        {
            return Err(OwnerReadError::Integrity);
        }
        if own.now < offer.terms().expires_at_utc {
            live.push((
                offer.operation(),
                offer.terms().price.minor(),
                offer.terms().expires_at_utc,
            ));
        }
    }
    if live.is_empty() {
        return Ok(Vec::new());
    }
    let step = source.resale_step(licence)?;
    Ok(live
        .into_iter()
        .map(|(offer, price_minor, expires_at_utc)| ResaleOfferView {
            offer,
            price_minor,
            expires_at_utc,
            step,
        })
        .collect())
}

fn view(row: &EntitlementListingV1) -> Result<PublicListingView, OwnerReadError> {
    row.command_view().map_err(|_| OwnerReadError::Integrity)
}

/// Up to `LISTINGS_PER_PAGE` current listings after the key `after`, in key
/// order, scanning at most `scans` pages of `SCAN_ROWS`. `next` is the last
/// row's token when a further current listing was seen. When the scan budget
/// ends before the family does, `next` is the last key scanned, current or
/// not, so a long run of listings that are no longer current never ends the
/// listings early or hides the ones after it (reviewer F4): the page may then
/// be short, or empty, and still go on.
pub(crate) fn listings<'a>(
    source: &impl ListingSource<'a>,
    after: Option<String>,
    scans: usize,
    current: impl Fn(&EntitlementListingV1) -> bool,
) -> Result<ListingsPage, OwnerReadError> {
    let mut cursor = after;
    let mut last: Option<CommitToken> = None;
    let mut rows: Vec<PublicListingView> = Vec::new();
    for _ in 0..scans {
        let (scanned, more) = source.scan(cursor.as_deref(), SCAN_ROWS)?;
        for row in &scanned {
            if !current(row) {
                continue;
            }
            if rows.len() == LISTINGS_PER_PAGE {
                return Ok(ListingsPage {
                    next: rows.last().map(PublicListingView::publication),
                    rows,
                });
            }
            rows.push(view(row)?);
        }
        if !more {
            return Ok(ListingsPage { rows, next: None });
        }
        match scanned.last() {
            Some(row) => {
                last = Some(row.publication());
                cursor = Some(row.publication().to_string());
            }
            None => return Ok(ListingsPage { rows, next: None }),
        }
    }
    // The budget ended with the family going on: continue after the last
    // listing scanned (its key is its own token, `Lens::scan` checks).
    Ok(ListingsPage { rows, next: last })
}

/// `ListingsAfter`: the first listing whose token starts with `code`, current
/// or not, from one scan of `CODE_ROWS` rows after the code (none is
/// `Refused`), then `listings` after it with the budget that is left.
pub(crate) fn listings_after<'a>(
    source: &impl ListingSource<'a>,
    code: &str,
    current: impl Fn(&EntitlementListingV1) -> bool,
) -> Result<ListingsPage, OwnerReadError> {
    if !is_code(code) {
        return Err(OwnerReadError::Refused);
    }
    let (scanned, _) = source.scan(Some(code), CODE_ROWS)?;
    let start = scanned
        .into_iter()
        .map(|row| row.publication().to_string())
        .find(|key| key.starts_with(code))
        .ok_or(OwnerReadError::Refused)?;
    listings(source, Some(start), SCANS - 1, current)
}

/// Whether `code` is a listing code: exactly `CODE_CHARACTERS` characters of
/// a token's text (lowercase `0-9a-f`).
#[must_use]
pub fn is_code(code: &str) -> bool {
    code.len() == CODE_CHARACTERS
        && code
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The current listing whose token starts with `code`, from one scan of
/// `CODE_ROWS` rows after the code.
///
/// 17g C2e (C2e plan §2.1 item 4a, review FE-F13): only CURRENT rows count
/// toward ambiguity. Within the one scan, one current row beside any rows
/// that are no longer current resolves to the current row; two current rows
/// are `Ambiguous`; no current row (a stale exact match included) is
/// `Refused`, the not-found answer. `CODE_ROWS` stays 2 because it is in the
/// read's descriptor (widening it would version the definition), so a
/// current listing that sits beyond two stale rows sharing its prefix is
/// still not found: a stated residual, since stale rows exist only after a
/// policy change.
///
/// A `One` is never a guess (the C2e fix round, reviewer correctness/F1):
/// when both scanned rows share the code, one of them current, and the
/// family goes on, a further current row with the code could sit just past
/// the window (stale, current, current), and the current row in the window
/// is then not the only one. Keys sharing a prefix are adjacent in key
/// order, so one probe of the single row after the window decides it: a row
/// that shares the code makes the answer `Ambiguous` (current or not, since
/// past it the read cannot see), any other row or none leaves the `One`
/// proven. The probe is one more scan of one row, only in that case; the
/// window itself stays `CODE_ROWS`.
pub(crate) fn by_code<'a>(
    source: &impl ListingSource<'a>,
    code: &str,
    current: impl Fn(&EntitlementListingV1) -> bool,
) -> Result<CodeLookup, OwnerReadError> {
    if !is_code(code) {
        return Err(OwnerReadError::Refused);
    }
    let shares = |row: &EntitlementListingV1| row.publication().to_string().starts_with(code);
    let (scanned, more) = source.scan(Some(code), CODE_ROWS)?;
    let saturated = more && scanned.len() == CODE_ROWS && scanned.iter().all(|row| shares(row));
    let last = scanned.last().map(|row| row.publication().to_string());
    let matching: Vec<_> = scanned
        .into_iter()
        .filter(|row| shares(row) && current(row))
        .collect();
    match matching.as_slice() {
        [] => Err(OwnerReadError::Refused),
        [row] => {
            if saturated {
                let (next, _) = source.scan(last.as_deref(), 1)?;
                if next.first().is_some_and(|row| shares(row)) {
                    return Ok(CodeLookup::Ambiguous);
                }
            }
            view(row).map(CodeLookup::One)
        }
        _ => Ok(CodeLookup::Ambiguous),
    }
}

#[cfg(test)]
#[path = "mind_commands/tests.rs"]
mod tests;
