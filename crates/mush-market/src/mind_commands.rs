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
//!   code: no key starting with it, or one that is not current, is
//!   `Refused`; one current is `One`; two are `Ambiguous`. No other length is
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
//! **Not built here (the C2c builder's report):** the brief's `OwnPurchase`
//! and `OwnOpen` kinds read the per-buyer entitlement index
//! (`market_mind_entitlement_actors`, C2 plan §2.5 item 1), which C2b did not
//! land (it is held for the lead's mush-core ruling). Without it there is no
//! bounded lookup of "this actor's quote or funded operation", so they are
//! left for the sub-cut that lands the index. 17g C2d's `ResaleOffers
//! { licence }` (the live resale offers naming the actor on one licence) is
//! held for the same reason: it reads the per-(buyer, licence) offer index
//! (`market_mind_paid_resale_buyer_offers`, plan §2.5 item 3), and the
//! paid-resale offers are otherwise keyed only by their own commit token.
//! C2d adds the resale steps to `MarketCommandActions` (`ResaleKind`).
//!
//! **Capacity.** This is a second market-side owner-read service. The
//! format-2 Active named launch already holds 31 services and all 32 grants
//! before it (17g C2a's measurement), so registering it takes the 32nd
//! service and TacSim cannot request it without `OwnerReadError::Limit`
//! (C2 plan §2.4: the lead's fallback raises both caps; no builder does).
//! `MarketCommandReader::request` is TacSim's call once that lands.

use crate::mind_entitlement::{
    EntitlementConfig, EntitlementGuard, EntitlementListingV1, Families, current_policy,
};
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistry,
    DefinitionRegistryBuilder, OwnerId,
};
use mush_core::read_service::{
    OwnerReadContext, OwnerReadError, OwnerReadHandle, OwnerReadService,
};
use mush_core::{CommitToken, DefinitionRef, ModError, Registrar};
use mush_economy::managed_entitlement::EntitlementProtocol;
use mush_roll::commerce::CharterId;
use mush_roll::commerce::paid::PaidForm;
use mush_roll::commerce::rating::RatingClass;
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
}
impl MarketCommandRequest {
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

/// One result per request kind.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MarketCommandResult {
    AtCounter { here: bool },
    Listings(ListingsPage),
    ListingByCode(CodeLookup),
}
impl MarketCommandResult {
    /// The counter answer, or `None` if this answered another kind.
    #[must_use]
    pub const fn at_counter(&self) -> Option<bool> {
        match self {
            Self::AtCounter { here } => Some(*here),
            Self::Listings(_) | Self::ListingByCode(_) => None,
        }
    }
    #[must_use]
    pub fn into_listings(self) -> Option<ListingsPage> {
        match self {
            Self::Listings(page) => Some(page),
            Self::AtCounter { .. } | Self::ListingByCode(_) => None,
        }
    }
    #[must_use]
    pub fn into_listing(self) -> Option<CodeLookup> {
        match self {
            Self::ListingByCode(found) => Some(found),
            Self::AtCounter { .. } | Self::Listings(_) => None,
        }
    }
}

/// The registered read. No public constructor: the market builds it when it
/// composes entitlements.
pub struct MarketCommandReader {
    families: Families,
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
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    families: Families,
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
        };
        answer(
            &lens,
            || self.guard.current(ctx.world(), ctx.actor()).is_ok(),
            &self.config,
            request,
        )
    }
}

/// The read's answer over `source`: `here` is the venue gate for the actor
/// (`EntitlementGuard::current`), and "current" is `current_policy` against
/// the deployment's `config`. `read` calls this with the owner lens; the
/// tests call it over an in-memory family, so the counter rule and the policy
/// filter they check are the registered read's own.
pub(crate) fn answer<'a>(
    source: &impl ListingSource<'a>,
    here: impl Fn() -> bool,
    config: &EntitlementConfig,
    request: MarketCommandRequest,
) -> Result<MarketCommandResult, OwnerReadError> {
    let current = |row: &EntitlementListingV1| current_policy(config, row).is_ok();
    match request.kind {
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
pub(crate) fn by_code<'a>(
    source: &impl ListingSource<'a>,
    code: &str,
    current: impl Fn(&EntitlementListingV1) -> bool,
) -> Result<CodeLookup, OwnerReadError> {
    if !is_code(code) {
        return Err(OwnerReadError::Refused);
    }
    let (scanned, _) = source.scan(Some(code), CODE_ROWS)?;
    let matching: Vec<_> = scanned
        .into_iter()
        .filter(|row| row.publication().to_string().starts_with(code))
        .collect();
    match matching.as_slice() {
        [] => Err(OwnerReadError::Refused),
        [row] if current(row) => view(row).map(CodeLookup::One),
        [_] => Err(OwnerReadError::Refused),
        _ => Ok(CodeLookup::Ambiguous),
    }
}

#[cfg(test)]
#[path = "mind_commands/tests.rs"]
mod tests;
