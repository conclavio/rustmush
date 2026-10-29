//! Doc 17h §3 M8f (the lead's rulings P3–P5): the market read's GOD-only
//! kinds behind tacsim's procurement tick.
//!
//! A module tick's owner reads run as GOD (core binds the read caller at
//! dispatch), so the tick cannot read "as" the House's procurement agent the
//! way `+minds` reads as the typing character. These kinds answer GOD alone
//! and refuse every other caller as `Refused`, which no player command
//! builds; nothing of them reaches a player surface (the tick prints nothing).
//!
//! - `ProcurementListings { after }`: the current listings, exactly as
//!   `Listings` pages them (the same scan budget, the same "current" rule),
//!   without the counter check (the tick stands nowhere), each with the listed
//!   revision and its rating reference, which the tick's score count needs.
//!   Only listings: a filed mind nobody listed is never in the family, so it
//!   is never a candidate (D-UM-54, P5).
//! - `ProcurementPurchase { agent, code, org }` and `ProcurementOpen { agent,
//!   org }`: `OwnPurchase` and `OwnOpen` over the organisation's buyer index
//!   row (`org/{org:020}`, M8c S1), read as the House's agent, whose grant
//!   the read re-asks as a read (`org_holder::reads_for`: a read moves no
//!   money, so the agent's budget-limited grant passes it, the lead's P2a
//!   ruling), so the tick finds the quote or funded purchase its agent's last
//!   step made.
//! - `ProcurementOperation { operation }`: one funded purchase's phase, price,
//!   listing, charter and reserved licence: whether the agent's Grant (or a
//!   Refund) has landed, and the facts the procurement ledger row records.
use super::{ListingSource, OwnSource, Protocol, PublicListingView, SCAN_ROWS, view};
use crate::mind_entitlement::{EntitlementListingV1, EntitlementPurchaseV1, EntitlementQuoteV1};
use mush_core::read_service::OwnerReadError;
use mush_core::{CommitToken, GOD, ObjId};
use mush_roll::commerce::purchase::PurchasePhase;
use mush_roll::commerce::rating::RatingRef;
use mush_roll::commerce::{CharterId, LicenceId, RevisionIdentity};
use serde::Serialize;

/// Listings one `ProcurementListings` page answers at most (the `Listings`
/// page's bound).
pub const PROCUREMENT_LISTINGS_PER_PAGE: usize = super::LISTINGS_PER_PAGE;

/// One current listing as the procurement tick ranks it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProcurementListing {
    view: PublicListingView,
    revision: RevisionIdentity,
    rating: RatingRef,
    instrument: mush_economy::InstrumentId,
}
impl ProcurementListing {
    /// What `+minds` would show of the listing (its token, form, charter,
    /// price, …).
    #[must_use]
    pub const fn view(&self) -> &PublicListingView {
        &self.view
    }
    /// The listed revision.
    #[must_use]
    pub const fn revision(&self) -> RevisionIdentity {
        self.revision
    }
    /// The listed revision's rating.
    #[must_use]
    pub const fn rating(&self) -> RatingRef {
        self.rating
    }
    /// The instrument the listing prices in (the buyer's treasury pays in it).
    #[must_use]
    pub const fn instrument(&self) -> &mush_economy::InstrumentId {
        &self.instrument
    }
}

/// One `ProcurementListings` page and where the next starts (as
/// `ListingsPage::next`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProcurementListingsPage {
    rows: Vec<ProcurementListing>,
    next: Option<CommitToken>,
}
impl ProcurementListingsPage {
    #[must_use]
    pub fn rows(&self) -> &[ProcurementListing] {
        &self.rows
    }
    #[must_use]
    pub const fn next(&self) -> Option<CommitToken> {
        self.next
    }
}

/// Where one funded purchase stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcurementPhase {
    Funded,
    Granted,
    Refunded,
}

/// One purchase as the procurement ledger records it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ProcurementOperationView {
    operation: CommitToken,
    listing: CommitToken,
    charter: CharterId,
    licence: LicenceId,
    price_minor: i64,
    phase: ProcurementPhase,
    org: Option<mush_org::OrgId>,
    acting: ObjId,
    protocol: Protocol,
}
impl ProcurementOperationView {
    #[must_use]
    pub const fn operation(&self) -> CommitToken {
        self.operation
    }
    #[must_use]
    pub const fn listing(&self) -> CommitToken {
        self.listing
    }
    #[must_use]
    pub const fn charter(&self) -> CharterId {
        self.charter
    }
    #[must_use]
    pub const fn licence(&self) -> LicenceId {
        self.licence
    }
    #[must_use]
    pub const fn price_minor(&self) -> i64 {
        self.price_minor
    }
    #[must_use]
    pub const fn phase(&self) -> ProcurementPhase {
        self.phase
    }
    /// The buying organisation, `None` for a character's purchase.
    #[must_use]
    pub const fn org(&self) -> Option<mush_org::OrgId> {
        self.org
    }
    /// The character that quoted it (for an organisation, its acting
    /// member): the House's tick settles only its own agent's purchases.
    #[must_use]
    pub const fn acting(&self) -> ObjId {
        self.acting
    }
    /// The entitlement protocol its terminal steps submit through.
    #[must_use]
    pub const fn protocol(&self) -> Protocol {
        self.protocol
    }
}

/// The agent's own record on one listing (H1): its newest quote there and
/// whether it has a funded purchase there, among the organisation's records
/// the agent quoted (`acting == agent`); another member's never count.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct AgentPurchaseView {
    protocol: Protocol,
    quote: Option<CommitToken>,
    price_minor: i64,
    expires_at_utc: i64,
    funded: bool,
}
impl AgentPurchaseView {
    #[must_use]
    pub const fn protocol(&self) -> Protocol {
        self.protocol
    }
    /// The agent's newest quote on the listing, live or not.
    #[must_use]
    pub const fn quote(&self) -> Option<CommitToken> {
        self.quote
    }
    #[must_use]
    pub const fn price_minor(&self) -> i64 {
        self.price_minor
    }
    /// When that quote stops being fundable (world UTC seconds).
    #[must_use]
    pub const fn expires_at_utc(&self) -> i64 {
        self.expires_at_utc
    }
    /// Whether the agent holds a funded purchase on the listing.
    #[must_use]
    pub const fn funded(&self) -> bool {
        self.funded
    }
}

/// `ProcurementPurchase` over the organisation's rows `own_rows` read.
pub(super) fn agent_purchase(
    quotes: &[&EntitlementQuoteV1],
    operations: &[&EntitlementPurchaseV1],
    agent: ObjId,
    listing: CommitToken,
) -> Option<AgentPurchaseView> {
    let newest = quotes
        .iter()
        .filter(|q| q.listing() == listing && q.buyer().acting_character() == agent)
        .max_by_key(|q| (q.quoted_at_utc(), q.publication()));
    let funded = operations.iter().find(|p| {
        p.quote_row().listing() == listing && p.quote_row().buyer().acting_character() == agent
    });
    let protocol = newest
        .map(|q| q.protocol())
        .or_else(|| funded.map(|p| p.quote_row().protocol()))?;
    Some(AgentPurchaseView {
        protocol: Protocol::of(protocol),
        quote: newest.map(|q| q.publication()),
        price_minor: newest.map_or(0, |q| q.price_minor()),
        expires_at_utc: newest.map_or(0, |q| q.expires_at_utc()),
        funded: funded.is_some(),
    })
}

/// `Refused` unless the caller is GOD (the module note).
pub(super) fn god(actor: ObjId) -> Result<(), OwnerReadError> {
    if actor == GOD {
        Ok(())
    } else {
        Err(OwnerReadError::Refused)
    }
}

/// `ProcurementListings` (the module note): `listings`' scan, the rows
/// mapped with their revision and rating.
pub(super) fn listings<'a>(
    source: &impl ListingSource<'a>,
    after: Option<CommitToken>,
    scans: usize,
    current: impl Fn(&EntitlementListingV1) -> bool,
) -> Result<ProcurementListingsPage, OwnerReadError> {
    let mut cursor = after.map(|t| t.to_string());
    let mut last: Option<CommitToken> = None;
    let mut rows: Vec<ProcurementListing> = Vec::new();
    for _ in 0..scans {
        let (scanned, more) = source.scan(cursor.as_deref(), SCAN_ROWS)?;
        for row in &scanned {
            if !current(row) {
                continue;
            }
            if rows.len() == PROCUREMENT_LISTINGS_PER_PAGE {
                return Ok(ProcurementListingsPage {
                    next: rows.last().map(|r| r.view.publication()),
                    rows,
                });
            }
            let (revision, rating) = row.procurement_subject();
            rows.push(ProcurementListing {
                view: view(row)?,
                revision,
                rating,
                instrument: row.instrument().clone(),
            });
        }
        if !more {
            return Ok(ProcurementListingsPage { rows, next: None });
        }
        match scanned.last() {
            Some(row) => {
                last = Some(row.publication());
                cursor = Some(row.publication().to_string());
            }
            None => return Ok(ProcurementListingsPage { rows, next: None }),
        }
    }
    Ok(ProcurementListingsPage { rows, next: last })
}

/// `ProcurementOperation` (the module note); `None` for no such purchase.
pub(super) fn operation<'a>(
    source: &impl OwnSource<'a>,
    operation: CommitToken,
) -> Result<Option<ProcurementOperationView>, OwnerReadError> {
    let Some(row) = source
        .operations(&[operation])?
        .into_iter()
        .next()
        .flatten()
    else {
        return Ok(None);
    };
    if row.operation() != operation {
        return Err(OwnerReadError::Integrity);
    }
    let quote = row.quote_row();
    Ok(Some(ProcurementOperationView {
        operation,
        listing: quote.listing(),
        charter: quote.charter(),
        licence: row.licence(),
        price_minor: quote.price_minor(),
        phase: match row.phase() {
            PurchasePhase::Funded {} => ProcurementPhase::Funded,
            PurchasePhase::Granted { .. } => ProcurementPhase::Granted,
            PurchasePhase::Refunded { .. } => ProcurementPhase::Refunded,
        },
        org: quote.buyer().organization(),
        acting: quote.buyer().acting_character(),
        protocol: Protocol::of(quote.protocol()),
    }))
}
