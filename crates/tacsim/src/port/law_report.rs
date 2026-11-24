//! **The receipt of a private sale, and the party who reports it** (doc 50 unit LW.3; Q-50-35,
//! Q-50-43, Q-50-66, Q-50-67 as ruled, doc 50a B-5, C-6).
//!
//! - **The receipt** ([`SaleReceipt`], Q-50-66): when a good the venue's law restricts is sold
//!   privately, the market's settlement tells the proceeds policy
//!   (`mush_market::ProceedsPolicy::private_sale`), and each party gets a receipt in their own
//!   book (`tacsim:sale_receipts:{party}`): the seller, the buyer, the venue, the good, the units,
//!   the price, the IC time, the polity whose law it was, and whether each party held the papers
//!   at that moment. The receipt alone fines nobody.
//! - **The report** (`+law/report <quote> go`, Q-50-35): a party brings their receipt, no more
//!   than one IC day old, against the other party who lacked the papers at the sale. Nobody else
//!   can witness a private sale.
//!   - **A reporting buyer hands over the goods** (Q-50-67): the bought lot (or the units, for a
//!     pooled sale) goes to the authority's vault (LW.5's `crate::port::seized`), in one
//!     submitted action, [`REPORT_ACTION`], whose prepare moves the goods and writes the report's
//!     row ([`REPORT_FAMILY`]) together. A buyer who no longer holds them cannot report.
//!   - **The offense** is recorded on the port tick from the landed row: against the other party,
//!     under the polity on the receipt, proven by the reporter, and fined at once.
//!   - **The reporter's payout** (Q-50-43, Q-50-67): when the fine is collected, the price a
//!     reporting buyer paid and a fifth of the fine, both out of the fine and never more than it
//!     (`crate::surface::security::pay_reporters`). The offender is never told who reported.

use std::collections::BTreeMap;

use mush_core::mechanics::OwnerId;
use mush_core::{
    ActionErrorCode, ActionInvocation, ActionProvider, ActionTxn, AuthoritativeActionOutput,
    CanonicalHash, CanonicalId, CmdError, Ctx, DefinitionDescriptor, DefinitionId, DefinitionKind,
    DefinitionRef, IdempotencyKeyText, ModError, ObjId, PerceivedRef, ProjectedParameter,
    ProjectedParameterKind, ProjectedParameterSchema, ProjectedTargetSchema, ProviderOfferDraft,
    Registrar, RevisionKey, RevisionKind, RevisionSet, RevisionStamp, RevisionValidator,
    RevisionValidatorContext, RulesValue, StateFamilyHandle, SubjectKey, World,
};
use mush_economy::VaultId;
use mush_inventory::InventoryFamilies;
use mush_inventory::record::{EndpointRef, LotId};
use mush_org::{OrgId, PrincipalId};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use smol_str::SmolStr;

use crate::offense::{Evidence, Offender, OffenseKey, OffenseKind, Payout, Proof, ProvenOffense};
use crate::port_tick::port_error;
use crate::store::TacsimStore;

/// The verb.
pub const LAW_VERB: &str = "+law";
/// The book's key prefix: `tacsim:sale_receipts:{party}`.
pub const RECEIPTS_PREFIX: &str = "tacsim:sale_receipts:";
/// The book's schema.
pub const RECEIPTS_SCHEMA: u16 = 1;
/// The most receipts a party's book keeps; past it the oldest goes.
pub const MAX_RECEIPTS: usize = 16;
/// **How old a receipt may be when it is reported**, IC seconds (Q-50-35, Q-50-66: one IC day).
pub const REPORT_WINDOW_IC_S: i64 = 86_400;
/// The report's action.
pub const REPORT_ACTION: &str = "tacsim_law_report";
/// Its outcome.
pub const REPORT_OUTCOME: &str = "tacsim_law_report_result";
/// The family of landed reports, keyed by the sale's quote.
pub const REPORT_FAMILY: &str = "tacsim.law_report";
/// The revision kind guarding projected law-report offers against live ownership/locality changes
/// (P2-TSW-9).
const REPORT_REVISION: &str = "tacsim.law_report.v1";
/// The most reports one offer carries.
const MAX_OFFER_ROWS: usize = 16;
/// Sentinel `ObjId` stored in `SaleReceipt::seller`/`buyer` when that side was an organization
/// (`P2-TSW-21`), so `seller_org`/`buyer_org` carries the real `OrgId` and no real character id
/// (including `#0`) ever aliases it.
const ORG_PARTY_SENTINEL: ObjId = ObjId(u32::MAX);
/// Hard cap on the RAM-only `law_reports_landed` deduplication cache (`P2-TSW-49`).
const MAX_LANDED_CACHE: usize = 4_096;

/// The usage line.
pub const LAW_USAGE: &str = "Usage: +law  |  +law/report <quote> [go]  |  +law/audit #<hull>";
/// The help.
pub const LAW_HELP: &str = "+law                       your receipts of private sales of restricted goods\n\
+law/report <quote> [go]   report the other party to a private sale of a restricted good who\n\
                           lacked the papers. Your receipt must be no more than one IC day old.\n\
                           A buyer hands over the goods. When the fine is collected you are paid\n\
                           a fifth of it, and a buyer the price they paid; you are never named.\n\
+law/audit #<hull>         (staff) audit a docked hull's hold now.";
/// No receipts.
pub const NO_RECEIPTS: &str = "You hold no receipts of private sales of restricted goods.";
/// The list head.
pub const RECEIPTS_HEAD: &str = "Your receipts of private sales of restricted goods:";
/// No such receipt of the actor's (`{quote}`).
pub const NO_SUCH_RECEIPT: &str = "You hold no receipt for sale {quote}.";
/// Too old.
pub const REPORT_TOO_OLD: &str =
    "That receipt is more than one IC day old; the authority acts only on a fresh record.";
/// Nothing to report.
pub const REPORT_NOTHING: &str =
    "The other party held the papers at that sale; there is nothing to report.";
/// Already reported.
pub const REPORT_ALREADY: &str = "That sale has already been reported.";
/// Reporter is not at the port where the sale took place (P2-TSW-9).
pub const REPORT_NOT_HERE: &str = "You must be at that port to file a report with its authority.";
/// Reporting buyer no longer controls the receiving vault (P2-TSW-9).
pub const REPORT_NO_GOODS: &str = "You no longer control the vault that received those goods.";
/// The preview (`{who}`, `{goods}`).
pub const REPORT_PREVIEW: &str =
    "Report {who} for that sale. {goods}To file the report, type: +law/report <quote> go";
/// A buyer's hand-over, in the preview.
pub const REPORT_HANDOVER: &str = "You hand over the goods you bought. ";
/// Filed.
pub const REPORT_FILED: &str = "Your report is filed with the port's authority.";
/// Taken (on the tick, to the reporter).
pub const REPORT_TAKEN: &str = "The port's authority takes your report; the other party is fined, and you are paid your share when the fine is collected.";
/// Staff only.
pub const LAW_STAFF_ONLY: &str = "Only staff order an audit.";

/// **One party's receipt of a private sale** (Q-50-66).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaleReceipt {
    /// The sale's quote, `QuoteId.0`.
    pub quote: u64,
    /// The seller (character id, or `ORG_PARTY_SENTINEL` when `seller_org` is `Some`).
    pub seller: ObjId,
    /// The buyer (character id, or `ORG_PARTY_SENTINEL` when `buyer_org` is `Some`).
    pub buyer: ObjId,
    /// The selling organization, when the seller was `PrincipalId::Organization` (`P2-TSW-21`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seller_org: Option<OrgId>,
    /// The buying organization, when the buyer was `PrincipalId::Organization` (`P2-TSW-21`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub buyer_org: Option<OrgId>,
    /// The venue.
    pub venue: CanonicalId,
    /// The good.
    pub good: CanonicalId,
    /// How many units.
    pub units: u64,
    /// The price a unit, minor units.
    pub price_minor: i64,
    /// When it settled, IC.
    pub at_ic: i64,
    /// The polity whose law it was (`FactionId.0`).
    pub polity: u64,
    /// Whether the seller held the papers at that moment.
    pub seller_papers: bool,
    /// Whether the buyer held the papers at that moment.
    pub buyer_papers: bool,
    /// Where the goods went, `VaultId.0`.
    pub destination: u64,
    /// The exact lot handed over, for a hold-backed sale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lot: Option<LotId>,
    /// Whether it has been reported.
    #[serde(default)]
    pub reported: bool,
}

impl SaleReceipt {
    /// Whether `who` is the character buyer on this receipt.
    #[must_use]
    pub const fn is_buyer(&self, who: ObjId) -> bool {
        self.buyer_org.is_none() && self.buyer.0 == who.0
    }

    /// Whether `who` is the character seller on this receipt.
    #[must_use]
    pub const fn is_seller(&self, who: ObjId) -> bool {
        self.seller_org.is_none() && self.seller.0 == who.0
    }

    /// The other party as an [`Offender`], from `who`'s side; `None` when `who` is neither
    /// (`P2-TSW-21`).
    #[must_use]
    pub const fn other_offender(&self, who: ObjId) -> Option<Offender> {
        if self.is_buyer(who) {
            Some(match self.seller_org {
                Some(org) => Offender::Organization(org),
                None => Offender::Character(self.seller),
            })
        } else if self.is_seller(who) {
            Some(match self.buyer_org {
                Some(org) => Offender::Organization(org),
                None => Offender::Character(self.buyer),
            })
        } else {
            None
        }
    }

    /// The other character party, from `who`'s side; `None` when `who` is neither or the other
    /// party is an organization.
    #[must_use]
    pub const fn other(&self, who: ObjId) -> Option<ObjId> {
        match self.other_offender(who) {
            Some(Offender::Character(id)) => Some(id),
            _ => None,
        }
    }

    /// Whether the other party, from `who`'s side, lacked the papers at the sale.
    #[must_use]
    pub const fn other_lacked(&self, who: ObjId) -> bool {
        if self.is_buyer(who) {
            !self.seller_papers
        } else if self.is_seller(who) {
            !self.buyer_papers
        } else {
            false
        }
    }
}

/// **`tacsim:sale_receipts:{party}`**: one party's receipts, oldest first.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiptBook {
    /// [`RECEIPTS_SCHEMA`].
    pub schema: u16,
    /// The party.
    pub party: ObjId,
    /// The receipts, at most [`MAX_RECEIPTS`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub receipts: Vec<SaleReceipt>,
}

impl ReceiptBook {
    /// The record's own rules.
    ///
    /// # Errors
    ///
    /// The rule broken.
    pub fn check(&self) -> Result<(), String> {
        if self.schema != RECEIPTS_SCHEMA {
            return Err(format!("schema {} is not {RECEIPTS_SCHEMA}", self.schema));
        }
        if self.receipts.len() > MAX_RECEIPTS {
            return Err(format!(
                "{} receipts, over {MAX_RECEIPTS}",
                self.receipts.len()
            ));
        }
        if self
            .receipts
            .iter()
            .any(|r| r.other_offender(self.party).is_none())
        {
            return Err("a receipt names another party".to_owned());
        }
        Ok(())
    }
}

/// The book's key.
#[must_use]
pub fn receipts_key(party: ObjId) -> SmolStr {
    SmolStr::new(format!("{RECEIPTS_PREFIX}{}", party.0))
}

/// `party`'s book, when it reads.
#[must_use]
pub fn receipts_of(w: &World, party: ObjId) -> Option<ReceiptBook> {
    w.module_state::<TacsimStore>()?
        .sale_receipts
        .get(&party)
        .cloned()
}

fn put_book(ctx: &mut Ctx<'_>, book: &ReceiptBook) {
    if let Some(store) = ctx.w.module_state_mut::<TacsimStore>() {
        store.sale_receipts.insert(book.party, book.clone());
    }
    crate::port::owned::write(ctx, receipts_key(book.party).to_string(), book);
}

/// **A private sale settled** (the proceeds policy's `private_sale`, Q-50-66; `P2-TSW-21`): a
/// receipt for each character party when the venue's law restricts the good, including sales
/// where the counterparty is an `Organization` (organizations hold no prospecting license, so
/// their `*_papers` flag is `false` and reporting fines `Offender::Organization`).
pub(crate) fn on_private_sale(ctx: &mut Ctx<'_>, sale: &mush_market::PrivateSale<'_>) {
    let (seller, seller_org) = match sale.seller {
        PrincipalId::Object(id) => (*id, None),
        PrincipalId::Organization(org) => (ORG_PARTY_SENTINEL, Some(*org)),
        PrincipalId::System(_) => return,
    };
    let (buyer, buyer_org) = match sale.buyer {
        PrincipalId::Object(id) => (*id, None),
        PrincipalId::Organization(org) => (ORG_PARTY_SENTINEL, Some(*org)),
        PrincipalId::System(_) => return,
    };
    let Some(paper) = crate::port::law::restriction_at(ctx.w, sale.market, sale.commodity) else {
        return;
    };
    let Some(entry) = crate::port::law::port_entry_of(ctx.w, sale.market) else {
        return;
    };
    let Some(polity) = crate::offense::jurisdiction_at(ctx.w, &entry) else {
        return;
    };
    let now = ctx.w.world_ic_now().at;
    let papers = |who: ObjId, org: Option<OrgId>| {
        org.is_none()
            && paper.as_str() == crate::port::law::PICKET_PAPERS
            && crate::port::law::papers_clear(ctx.w, who, now)
    };
    let receipt = SaleReceipt {
        quote: sale.quote.0,
        seller,
        buyer,
        seller_org,
        buyer_org,
        venue: CanonicalId::new(sale.market.as_str()).unwrap_or_else(|_| entry.clone()),
        good: sale.commodity.clone(),
        units: sale.quantity,
        price_minor: sale.price.minor(),
        at_ic: now,
        polity: polity.0,
        seller_papers: papers(seller, seller_org),
        buyer_papers: papers(buyer, buyer_org),
        destination: sale.destination.vault().0,
        lot: sale.lot,
        reported: false,
    };
    for party in [
        seller_org.is_none().then_some(seller),
        buyer_org.is_none().then_some(buyer),
    ]
    .into_iter()
    .flatten()
    {
        let mut book = receipts_of(ctx.w, party).unwrap_or(ReceiptBook {
            schema: RECEIPTS_SCHEMA,
            party,
            receipts: Vec::new(),
        });
        if book.receipts.iter().any(|r| r.quote == receipt.quote) {
            continue;
        }
        book.receipts.push(receipt.clone());
        while book.receipts.len() > MAX_RECEIPTS {
            book.receipts.remove(0);
        }
        put_book(ctx, &book);
    }
}

// ---- the command ----------------------------------------------------------------------------

/// **`+law`'s spec**.
#[must_use]
pub(crate) fn law_spec() -> mush_core::CommandSpec {
    mush_core::CommandSpec {
        name: LAW_VERB.into(),
        shape: mush_core::ArgShape::Raw,
        switches: ["report", "audit"]
            .iter()
            .copied()
            .map(SmolStr::new)
            .collect(),
        help: LAW_HELP.into(),
        ..mush_core::CommandSpec::default()
    }
}

/// **`+law`** (doc 50 unit LW.3).
///
/// # Errors
///
/// The refusal.
pub(crate) fn law_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    crate::commands::require_enabled(ctx)?;
    let raw = ctx.args.raw().to_owned();
    let (rest, go) = crate::surface::retrieval::split_go(&raw);
    let line = if ctx.switches.has("audit") {
        audit_cmd(ctx, &rest)
    } else if ctx.switches.has("report") {
        report_cmd(ctx, &rest, go)
    } else if rest.is_empty() {
        Ok(list(ctx.w, ctx.actor))
    } else {
        Err(LAW_USAGE.to_owned())
    };
    match line {
        Ok(line) => {
            ctx.out.sys_to_actor(ctx.actor, "tacsim.law", line);
            Ok(())
        }
        Err(line) => Err(CmdError::msg(line)),
    }
}

fn list(w: &World, who: ObjId) -> String {
    let Some(book) = receipts_of(w, who).filter(|b| !b.receipts.is_empty()) else {
        return NO_RECEIPTS.to_owned();
    };
    let mut out = vec![RECEIPTS_HEAD.to_owned()];
    for r in &book.receipts {
        let side = if r.is_buyer(who) { "bought" } else { "sold" };
        out.push(format!(
            "  sale {}: {side} {} {} at {}{}",
            r.quote,
            r.units,
            r.good,
            r.venue,
            if r.reported { " (reported)" } else { "" }
        ));
    }
    out.join("\n")
}

fn audit_cmd(ctx: &mut Ctx<'_>, rest: &str) -> Result<String, String> {
    if !crate::commands::is_staff(ctx.w, ctx.actor) {
        return Err(LAW_STAFF_ONLY.to_owned());
    }
    let hull = rest
        .trim()
        .trim_start_matches('#')
        .parse::<u32>()
        .map(ObjId)
        .map_err(|_| LAW_USAGE.to_owned())?;
    Ok(crate::port::audit::audit_now(ctx, hull))
}

/// The actor's own receipt for `quote`.
fn receipt(w: &World, who: ObjId, quote: u64) -> Option<SaleReceipt> {
    receipts_of(w, who)?
        .receipts
        .into_iter()
        .find(|r| r.quote == quote)
}

/// Why `who` may not report `r` now, if they may not.
fn refusal(w: &World, who: ObjId, r: &SaleReceipt) -> Option<String> {
    if r.reported || landed(w, r.quote) {
        return Some(REPORT_ALREADY.to_owned());
    }
    if w.world_ic_now().at > r.at_ic.saturating_add(REPORT_WINDOW_IC_S) {
        return Some(REPORT_TOO_OLD.to_owned());
    }
    if !r.other_lacked(who) {
        return Some(REPORT_NOTHING.to_owned());
    }
    // P2-TSW-9: the reporter must stand at the sale's port, and a reporting buyer must still
    // control the receiving vault while it (and any governing hull) is present/docked there.
    let Some(venue) = mush_market::MarketId::new(r.venue.as_str()) else {
        return Some(REPORT_NOT_HERE.to_owned());
    };
    let gate = crate::port::market::PortVenueGate;
    let here = w
        .get(who)
        .and_then(|o| o.location)
        .and_then(|room| mush_market::VenueGate::venue_of_room(&gate, w, room));
    if here.as_ref() != Some(&venue) {
        return Some(REPORT_NOT_HERE.to_owned());
    }
    if r.is_buyer(who) {
        let from = EndpointRef(VaultId(r.destination));
        if !mush_inventory::may_take(w, who, from)
            || !mush_market::seller::bidder_holds_vault(w, &PrincipalId::Object(who), from.vault())
        {
            return Some(REPORT_NO_GOODS.to_owned());
        }
        if let Err(why) = mush_market::VenueGate::admit(
            &gate,
            w,
            who,
            &venue,
            Some(from),
            mush_market::Presence::Here,
        ) {
            return Some(why.line);
        }
    }
    None
}

fn org_name_of(w: &World, org: OrgId) -> String {
    w.module_state::<mush_org::OrgStore>()
        .and_then(|store| store.state().org(org).map(|o| o.name.to_string()))
        .unwrap_or_else(|| format!("organization #{}", org.0))
}

fn report_cmd(ctx: &mut Ctx<'_>, rest: &str, go: bool) -> Result<String, String> {
    let who = ctx.actor;
    let word = rest.trim().trim_start_matches('#');
    let quote: u64 = word.parse().map_err(|_| LAW_USAGE.to_owned())?;
    let r = receipt(ctx.w, who, quote)
        .ok_or_else(|| NO_SUCH_RECEIPT.replace("{quote}", &quote.to_string()))?;
    if let Some(line) = refusal(ctx.w, who, &r) {
        return Err(line);
    }
    let buyer = r.is_buyer(who);
    if !go {
        let other = match r.other_offender(who) {
            Some(Offender::Character(o)) => crate::port::coop::name_of(ctx.w, o),
            Some(Offender::Organization(org)) => org_name_of(ctx.w, org),
            _ => String::new(),
        };
        return Ok(REPORT_PREVIEW
            .replace("{who}", &other)
            .replace("{goods}", if buyer { REPORT_HANDOVER } else { "" }));
    }
    // A reporting buyer's goods go to the authority's vault, made now on first use.
    if buyer {
        let authority = crate::port::law::port_entry_of(
            ctx.w,
            &mush_market::MarketId::new(r.venue.as_str()).ok_or_else(|| LAW_USAGE.to_owned())?,
        )
        .and_then(|entry| crate::port::seized::port_authority(ctx.w, &entry))
        .ok_or("The port has no authority to take your report.")?;
        crate::port::seized::ensure_vault(ctx, &authority)?;
    }
    let Some(action) = ctx
        .w
        .module_state::<ReportHandles>()
        .and_then(|h| h.action.clone())
    else {
        return Err("The authority cannot take a report here.".to_owned());
    };
    if ctx.out.authoritative_pending(who) {
        return Err("Wait for your last action to settle.".to_owned());
    }
    let key = IdempotencyKeyText::parse_canonical(format!("tacsim.law/report/{quote}"))
        .map_err(|e| e.to_string())?;
    ctx.submit_authoritative(
        action,
        PerceivedRef::SelfActor,
        vec![],
        json!({"quote": quote}),
        key,
    );
    Ok(REPORT_FILED.to_owned())
}

// ---- the action ---------------------------------------------------------------------------

/// **One landed report**, keyed by the sale's quote: the replay guard and the tick's input.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportLanded {
    /// The sale's quote.
    pub quote: u64,
    /// The reporter.
    pub by: ObjId,
    /// Whether the goods were handed over.
    pub handed: bool,
}

/// **A report the offer projects**: the receipt and, for a buyer, the vault the goods go to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReportRow {
    receipt: SaleReceipt,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    into: Option<u64>,
}

/// The provider's handles, for the command and the tick.
#[derive(Clone, Debug)]
pub(crate) struct ReportHandles {
    pub action: Option<DefinitionRef>,
    pub reports: StateFamilyHandle<u64, ReportLanded>,
}

struct ReportProvider {
    action: DefinitionRef,
    outcome: DefinitionRef,
    reports: StateFamilyHandle<u64, ReportLanded>,
    inventory: InventoryFamilies,
}

fn invalid() -> mush_core::ActionError {
    port_error(
        ActionErrorCode::InvalidRequest,
        "a report is self-scoped, takes no targets and names one sale",
    )
}

/// The reports `actor` may make, keyed by quote: derived from the store at projection.
fn rows_for(w: &World, actor: ObjId) -> Map<String, Value> {
    let mut rows = Map::new();
    let Some(book) = receipts_of(w, actor) else {
        return rows;
    };
    for r in book.receipts {
        if rows.len() >= MAX_OFFER_ROWS {
            break;
        }
        if refusal(w, actor, &r).is_some() {
            continue;
        }
        let into = if r.is_buyer(actor) {
            let Some(vault) = mush_market::MarketId::new(r.venue.as_str())
                .and_then(|venue| crate::port::law::port_entry_of(w, &venue))
                .and_then(|entry| crate::port::seized::port_authority(w, &entry))
                .and_then(|a| {
                    w.module_state::<mush_market::MarketStore>()
                        .and_then(|s| s.vendor_of_org(a.org))
                        .map(|v| v.vault.0)
                })
            else {
                continue;
            };
            if vault == r.destination {
                continue;
            }
            Some(vault)
        } else {
            None
        };
        let quote = r.quote;
        if let Ok(value) = serde_json::to_value(ReportRow { receipt: r, into }) {
            rows.insert(quote.to_string(), value);
        }
    }
    rows
}

fn report_revision_key(actor: ObjId) -> Result<RevisionKey, mush_core::ActionError> {
    let integrity = |message: &'static str| port_error(ActionErrorCode::Integrity, message);
    let owner = OwnerId::new(crate::NS).map_err(|_| integrity("the report's owner is invalid"))?;
    let kind = RevisionKind::new(
        CanonicalId::new(REPORT_REVISION)
            .map_err(|_| integrity("the report's revision kind is invalid"))?,
    );
    Ok(RevisionKey {
        owner,
        kind,
        subject: SubjectKey::Object { id: actor },
        facet: None,
    })
}

/// Canonical digest of the actor's currently reportable receipts and live destination-vault
/// authority/revisions (P2-TSW-9): any post-projection change to actor presence, vault ownership,
/// hull berth, clearance, or on-sale state invalidates the offer's revision guard before commit.
fn report_generation(w: &World, actor: ObjId) -> Result<u64, mush_core::ActionError> {
    let integrity = |message: &'static str| port_error(ActionErrorCode::Integrity, message);
    let rows = rows_for(w, actor);
    let mut vaults = Map::new();
    if let Some(economy) = w.module_state::<mush_economy::EconomyStore>() {
        let snapshot = economy.snapshot();
        for (quote, row_val) in &rows {
            if let Ok(row) = serde_json::from_value::<ReportRow>(row_val.clone())
                && row.receipt.is_buyer(actor)
            {
                let stamp = snapshot.vault(VaultId(row.receipt.destination)).map(|v| {
                    json!({
                        "owner": v.owner,
                        "scope": v.scope,
                        "status": v.status,
                        "revision": v.revision,
                    })
                });
                vaults.insert(quote.clone(), json!(stamp));
            }
        }
    }
    let payload = json!({
        "actor": actor.0,
        "rows": Value::Object(rows),
        "vaults": Value::Object(vaults),
    });
    let hash = CanonicalHash::for_canonical_value("tacsim.law_report_world.v1", 1, &payload, 32768)
        .map_err(|_| integrity("the report's revision digest could not be encoded"))?;
    let hex = hash.to_hex();
    let prefix = hex
        .get(..16)
        .ok_or_else(|| integrity("the report's revision digest is malformed"))?;
    u64::from_str_radix(prefix, 16)
        .map_err(|_| integrity("the report's revision digest is malformed"))
}

struct LawReportValidator;

impl RevisionValidator for LawReportValidator {
    fn current_generation(
        &self,
        ctx: &RevisionValidatorContext<'_>,
        key: &RevisionKey,
    ) -> Result<Option<u64>, mush_core::ActionError> {
        let SubjectKey::Object { id: actor } = key.subject else {
            return Ok(None);
        };
        if key.facet.is_some() {
            return Ok(None);
        }
        report_generation(ctx.world(), actor).map(Some)
    }
}

impl ActionProvider for ReportProvider {
    fn validate_invocation(
        &self,
        invocation: &ActionInvocation,
    ) -> Result<(), mush_core::ActionError> {
        quote_of(invocation).map(|_| ())
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<ProviderOfferDraft>, mush_core::ActionError> {
        let SubjectKey::Object { id: subject } = query.truth_subject() else {
            return Ok(None);
        };
        if action != &self.action
            || query.actor() != *subject
            || query.projected_subject() != &PerceivedRef::SelfActor
        {
            return Ok(None);
        }
        let rows = rows_for(&engine.world, query.actor());
        if rows.is_empty() {
            return Ok(None);
        }
        let integrity = |message: &'static str| port_error(ActionErrorCode::Integrity, message);
        let definitions = engine
            .registry
            .definitions()
            .ok_or_else(|| integrity("the report's definition registry is unavailable"))?;
        let revisions = RevisionSet::from_stamps([RevisionStamp {
            key: report_revision_key(query.actor())?,
            generation: report_generation(&engine.world, query.actor())?,
        }])
        .map_err(|_| integrity("the report's revision guard is invalid"))?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(revisions)
            .map_err(|_| integrity("the report's guard registry is unavailable"))?;
        guards
            .add_definition(action.clone(), definitions)
            .map_err(|_| integrity("the report's definition guard is unavailable"))?;
        Ok(Some(ProviderOfferDraft {
            fixed_parameters: json!({ "rows": Value::Object(rows) }),
            parameters: ProjectedParameterSchema::new(vec![ProjectedParameter {
                id: CanonicalId::new("quote")
                    .map_err(|_| integrity("the report's parameter id is invalid"))?,
                required: true,
                kind: ProjectedParameterKind::Integer {
                    minimum: 1,
                    maximum: i64::MAX,
                },
            }])
            .map_err(|_| integrity("the report's parameter schema is invalid"))?,
            targets: ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: mush_core::ActionPresentation::new(
                "tacsim-law-report",
                "Report a private sale",
                "Report the other party to a private sale of a restricted good.",
            )
            .map_err(|_| integrity("the report's presentation is invalid"))?,
            guards: guards
                .finish()
                .map_err(|_| integrity("the report's offer guards are invalid"))?,
        }))
    }

    fn prepare_authoritative(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, mush_core::ActionError> {
        let quote = quote_of(invocation)?;
        let row: ReportRow = invocation
            .parameters()
            .get("rows")
            .and_then(Value::as_object)
            .and_then(|rows| rows.get(&quote.to_string()))
            .cloned()
            .and_then(|row| serde_json::from_value(row).ok())
            .ok_or_else(invalid)?;
        let actor = invocation.actor();
        let r = &row.receipt;
        if r.other_offender(actor).is_none() || !r.other_lacked(actor) {
            return Err(port_error(
                ActionErrorCode::Unauthorized,
                "that sale is not yours to report",
            ));
        }
        let refused =
            |message: &'static str| port_error(ActionErrorCode::ProviderRejected, message);
        if r.reported
            || txn
                .get(&self.reports, &quote)
                .map_err(|_| port_error(ActionErrorCode::Integrity, "the reports are unavailable"))?
                .is_some()
        {
            return Err(refused("that sale has already been reported"));
        }
        // A reporting buyer hands the goods over to the authority's vault, in this bundle.
        let handed = if r.is_buyer(actor) {
            let into = EndpointRef(VaultId(row.into.ok_or_else(invalid)?));
            let from = EndpointRef(VaultId(r.destination));
            if from == into {
                return Err(invalid());
            }
            match r.lot {
                Some(lot) => {
                    self.inventory
                        .relocate_lots(txn, &[lot], from, into)
                        .map_err(|_| refused("you no longer hold the goods you bought"))?;
                }
                None => {
                    if r.units == 0 {
                        return Err(invalid());
                    }
                    let taken = self
                        .inventory
                        .seize(txn, from, &r.good, r.units, &[], Some(into))
                        .map_err(|_| refused("you no longer hold the goods you bought"))?;
                    if taken.taken != r.units {
                        return Err(refused("you no longer hold the goods you bought"));
                    }
                }
            }
            true
        } else {
            false
        };
        txn.insert(
            &self.reports,
            &quote,
            ReportLanded {
                quote,
                by: actor,
                handed,
            },
        )
        .map_err(|_| port_error(ActionErrorCode::Integrity, "the report did not stage"))?;
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({"actor": actor.0, "quote": quote, "handed": handed}),
        )
        .map_err(|_| {
            port_error(
                ActionErrorCode::BoundsExceeded,
                "the report's outcome exceeds canonical bounds",
            )
        })?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

fn quote_of(invocation: &ActionInvocation) -> Result<u64, mush_core::ActionError> {
    if invocation.subject() != &PerceivedRef::SelfActor || !invocation.targets().is_empty() {
        return Err(invalid());
    }
    invocation
        .parameters()
        .get("quote")
        .and_then(Value::as_u64)
        .filter(|q| *q > 0)
        .ok_or_else(invalid)
}

/// Whether a report of `quote` has landed.
fn landed(w: &World, quote: u64) -> bool {
    w.module_state::<TacsimStore>()
        .is_some_and(|store| store.law_reports_landed.contains(&quote))
}

/// **The port tick's half** (Q-50-35): each receipt whose report has landed and is not yet
/// recorded becomes an offense against the other party, fined at once, with the reporter's
/// payout on it.
pub(crate) fn tick(ctx: &mut Ctx<'_>) {
    let Some(handles) = ctx.w.module_state::<ReportHandles>().cloned() else {
        return;
    };
    let pending: Vec<(ObjId, SaleReceipt)> = ctx
        .w
        .module_state::<TacsimStore>()
        .map(|store| {
            store
                .sale_receipts
                .values()
                .flat_map(|b| b.receipts.iter().map(move |r| (b.party, r.clone())))
                .filter(|(_, r)| !r.reported)
                .collect()
        })
        .unwrap_or_default();
    for (party, r) in pending {
        let Ok(Some(row)) = ctx
            .action_state(&handles.reports, &r.quote)
            .map(|v| v.cloned())
        else {
            continue;
        };
        if row.by == party {
            apply(ctx, &row, &r);
        }
        mark_reported(ctx, party, r.quote);
    }
    prune_landed(ctx.w);
}

/// Prune `store.law_reports_landed` to quotes that still appear in a live receipt book within
/// [`REPORT_WINDOW_IC_S`] and enforce [`MAX_LANDED_CACHE`] (`P2-TSW-49`).
pub(crate) fn prune_landed(w: &mut World) {
    let now = w.world_ic_now().at;
    let Some(store) = w.module_state_mut::<TacsimStore>() else {
        return;
    };
    if store.law_reports_landed.is_empty() {
        return;
    }
    let live: std::collections::BTreeSet<u64> = store
        .sale_receipts
        .values()
        .flat_map(|book| book.receipts.iter())
        .filter(|r| now <= r.at_ic.saturating_add(REPORT_WINDOW_IC_S))
        .map(|r| r.quote)
        .collect();
    store.law_reports_landed.retain(|q| live.contains(q));
    while store.law_reports_landed.len() > MAX_LANDED_CACHE {
        if store.law_reports_landed.pop_first().is_none() {
            break;
        }
    }
}

fn mark_reported(ctx: &mut Ctx<'_>, party: ObjId, quote: u64) {
    let Some(mut book) = receipts_of(ctx.w, party) else {
        return;
    };
    let mut changed = false;
    for r in &mut book.receipts {
        if r.quote == quote && !r.reported {
            r.reported = true;
            changed = true;
        }
    }
    if changed {
        put_book(ctx, &book);
    }
    if let Some(store) = ctx.w.module_state_mut::<TacsimStore>() {
        store.law_reports_landed.insert(quote);
        while store.law_reports_landed.len() > MAX_LANDED_CACHE {
            if store.law_reports_landed.pop_first().is_none() {
                break;
            }
        }
    }
}

/// One landed report, recorded and fined.
fn apply(ctx: &mut Ctx<'_>, row: &ReportLanded, r: &SaleReceipt) {
    let Some(offender) = r.other_offender(row.by) else {
        return;
    };
    let buyer_reports = r.is_buyer(row.by);
    let key = OffenseKey {
        offender,
        polity: mush_faction::FactionId(r.polity),
    };
    let offense = ProvenOffense {
        kind: if buyer_reports {
            OffenseKind::RestrictedSale
        } else {
            OffenseKind::RestrictedPurchase
        },
        evidence: Evidence::Receipt {
            quote: r.quote,
            venue: r.venue.clone(),
            good: r.good.clone(),
            units: r.units,
            at_ic: r.at_ic,
        },
        proof: Proof::Reported { by: row.by },
        proven_ic: ctx.w.world_ic_now().at,
        debt: None,
        payout: Some(Payout {
            to: row.by,
            repay_minor: if buyer_reports {
                r.price_minor
                    .saturating_mul(i64::try_from(r.units).unwrap_or(i64::MAX))
            } else {
                0
            },
            paid: false,
        }),
    };
    if !crate::offense::record_offense(ctx, key, offense, crate::offense::is_settled) {
        return;
    }
    let entry = mush_market::MarketId::new(r.venue.as_str())
        .and_then(|venue| crate::port::law::port_entry_of(ctx.w, &venue));
    let notify = match offender {
        Offender::Character(who) => Some(who),
        Offender::Organization(_) | Offender::Account(_) => None,
    };
    if let Some(entry) = entry {
        crate::surface::security::fine_now(ctx, &key, &entry, notify);
    }
    // Doc 50 unit LW.4 (Q-50-69): a sale proven by a party's report costs standing.
    crate::port::record::lose_standing(
        ctx,
        &key,
        |row| matches!(&row.evidence, Evidence::Receipt { quote, .. } if *quote == r.quote),
    );
    ctx.out
        .sys_to_actor(row.by, "tacsim.law", REPORT_TAKEN.to_owned());
}

// ---- registration -------------------------------------------------------------------------

fn descriptor(kind: &str, id: &str, payload: Value) -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new(kind).map_err(|e| ModError::msg(e.to_string()))?,
        DefinitionId::new(id).map_err(|e| ModError::msg(e.to_string()))?,
        1,
        payload,
    )
    .map_err(|e| ModError::msg(e.to_string()))
}

/// The report action's descriptor.
///
/// # Errors
///
/// A descriptor that does not form.
pub(crate) fn action_descriptor(
    outcomes: &BTreeMap<String, DefinitionRef>,
) -> Result<DefinitionDescriptor, ModError> {
    outcomes
        .get(REPORT_OUTCOME)
        .ok_or_else(|| ModError::msg("the report's outcome did not register"))?;
    descriptor(
        "action",
        REPORT_ACTION,
        json!({"format": 1, "protocol": "law_report_v1", "subject": "self_actor", "targets": 0}),
    )
}

/// The report outcome's descriptor.
///
/// # Errors
///
/// A descriptor that does not form.
pub(crate) fn outcome_descriptor() -> Result<DefinitionDescriptor, ModError> {
    descriptor("outcome", REPORT_OUTCOME, json!({"format": 1}))
}

/// **Register the report**: its family always, and its provider when the inventory cell is
/// filled.
///
/// # Errors
///
/// A family or a provider that does not register.
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    actions: &BTreeMap<String, DefinitionRef>,
    outcomes: &BTreeMap<String, DefinitionRef>,
    inventory: Option<&InventoryFamilies>,
) -> Result<(), ModError> {
    use crate::port::families::family_descriptor;
    let reports = reg.state_family::<u64, ReportLanded, _, _, _, _>(
        family_descriptor(
            REPORT_FAMILY,
            "tacsim.law_report_key.v1",
            "tacsim.law_report",
        )?,
        |key: &u64| Ok(key.to_string()),
        |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
        |value: &ReportLanded| {
            if value.quote > 0 {
                Ok(())
            } else {
                Err("a report names a sale".to_owned())
            }
        },
        |value: &ReportLanded| serde_json::to_value(value).map_err(|e| e.to_string()),
    )?;
    reg.revision_validator(
        RevisionKind::new(
            CanonicalId::new(REPORT_REVISION).map_err(|e| ModError::msg(e.to_string()))?,
        ),
        LawReportValidator,
    )?;
    let action = match (actions.get(REPORT_ACTION), inventory) {
        (Some(action), Some(inventory)) => {
            let outcome = outcomes
                .get(REPORT_OUTCOME)
                .cloned()
                .ok_or_else(|| ModError::msg("the report's outcome did not register"))?;
            reg.action_provider(
                action.clone(),
                ReportProvider {
                    action: action.clone(),
                    outcome,
                    reports: reports.clone(),
                    inventory: inventory.clone(),
                },
            )?;
            Some(action.clone())
        }
        _ => None,
    };
    reg.state(ReportHandles { action, reports });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_receipt(
        quote: u64,
        seller: ObjId,
        seller_org: Option<OrgId>,
        buyer: ObjId,
        buyer_org: Option<OrgId>,
        at_ic: i64,
    ) -> SaleReceipt {
        SaleReceipt {
            quote,
            seller,
            buyer,
            seller_org,
            buyer_org,
            venue: CanonicalId::new("ceres").expect("venue"),
            good: CanonicalId::new("strategic_ore").expect("good"),
            units: 2,
            price_minor: 30_000,
            at_ic,
            polity: 1,
            seller_papers: false,
            buyer_papers: false,
            destination: 7,
            lot: None,
            reported: false,
        }
    }

    #[test]
    fn p2_tsw_21_org_private_sale_receipt_maps_to_org_offender_without_aliasing_god() {
        let buyer = ObjId(42);
        let org = OrgId(99);
        let r = sample_receipt(101, ORG_PARTY_SENTINEL, Some(org), buyer, None, 1_000);
        assert!(r.is_buyer(buyer));
        assert!(!r.is_seller(buyer));
        assert!(!r.is_seller(ObjId(0)));
        assert!(!r.is_seller(ORG_PARTY_SENTINEL));
        assert_eq!(r.other_offender(buyer), Some(Offender::Organization(org)));
        assert_eq!(r.other(buyer), None);
        assert!(r.other_lacked(buyer));
        let book = ReceiptBook {
            schema: RECEIPTS_SCHEMA,
            party: buyer,
            receipts: vec![r.clone()],
        };
        assert!(book.check().is_ok());

        // Buyer-as-org receipt on seller's book:
        let seller = ObjId(77);
        let r2 = sample_receipt(102, seller, None, ORG_PARTY_SENTINEL, Some(org), 1_000);
        assert!(r2.is_seller(seller));
        assert!(!r2.is_buyer(seller));
        assert_eq!(r2.other_offender(seller), Some(Offender::Organization(org)));
        assert!(r2.other_lacked(seller));
        let book2 = ReceiptBook {
            schema: RECEIPTS_SCHEMA,
            party: seller,
            receipts: vec![r2],
        };
        assert!(book2.check().is_ok());
    }

    #[test]
    fn p2_tsw_49_prune_landed_drops_expired_and_unreferenced_quotes() {
        let mut tw =
            mush_testkit::TestWorld::with_modules(vec![Box::new(crate::TacsimRuleset::default())]);
        let w = tw.world_mut();
        let party = ObjId(12);
        let now = w.world_ic_now().at;
        let fresh = sample_receipt(10, ObjId(5), None, party, None, now);
        let expired = sample_receipt(
            11,
            ObjId(5),
            None,
            party,
            None,
            now.saturating_sub(REPORT_WINDOW_IC_S + 1),
        );
        let store = w.module_state_mut::<TacsimStore>().expect("store");
        store.sale_receipts.insert(
            party,
            ReceiptBook {
                schema: RECEIPTS_SCHEMA,
                party,
                receipts: vec![fresh, expired],
            },
        );
        store.law_reports_landed.insert(10);
        store.law_reports_landed.insert(11);
        store.law_reports_landed.insert(999);
        prune_landed(w);
        let store = w.module_state::<TacsimStore>().expect("store");
        assert!(store.law_reports_landed.contains(&10));
        assert!(!store.law_reports_landed.contains(&11));
        assert!(!store.law_reports_landed.contains(&999));
    }
}
