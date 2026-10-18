//! The market's five document families and the caps every one of them re-runs on
//! load (E6c, 08f §5.1–§5.2).
//!
//! | family | record | corrupt → |
//! |---|---|---|
//! | `market:catalog:{pack}:{code}` | [`CatalogEntry`] | **skipped loudly**; the entry is re-loadable from its source pack (`@market/catalog load`) |
//! | `market:listing:{id}` | [`Listing`] | **skipped loudly**; a listing is an offer to sell, never an escrow — the vendor lists it again |
//! | `market:quote:{id}` | [`Quote`] | **skipped loudly**; an unsettled quote is a price anyone can ask for again, and a settled one is a *trade tape row* whose authority is the economy's own `ContractState::Settled` |
//! | `market:vendor:{id}` | [`Vendor`] | **skipped loudly**; staff re-register it, and its money and goods are the organization's, never this record's |
//! | `market:source:{id}` | [`MarketSource`] | **skipped loudly**; a maker that will not load simply stops quoting, which is the fail-closed direction |
//!
//! Every one follows the E5a fold rule exactly: the persisted shape is a separate
//! `…Wire` struct and the live record is reached only through a `TryFrom` that
//! re-runs the constructor invariants with documented caps. **No document failure
//! ever fails the boot** — the direction of each loss is stated above, and in every
//! case the money is the economy's and survives.
//!
//! ## What no record here holds
//!
//! No balance, no hold, no journal line and no authority. A listing is a *published
//! intention*; a quote is a *price with an expiry*; a vendor and a maker are
//! *pointers at an organization's own account and vault*. Everything that moves money
//! moves it through E4 under an E4 authority.

use std::collections::BTreeSet;

use mush_core::ObjId;
use mush_economy::{AccountId, Amount, CatalogRef, ContractId, EconomicDeadline, VaultId};
use mush_org::{OrgId, PrincipalId};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::ids::{ListingId, MarketId, MarketSourceId, QuoteId, VendorId};

// ---- caps -----------------------------------------------------------------

/// The most entries one content pack may carry (08f §9).
pub const MAX_CATALOG_ENTRIES_PER_PACK: usize = 512;
/// The most catalog entries the whole store holds, across every pack.
pub const MAX_CATALOG_ENTRIES: usize = 4_096;
/// The most listings one vendor may hold open (08f §9).
pub const MAX_LISTINGS_PER_VENDOR: usize = 64;
/// The most listings the whole store holds.
pub const MAX_LISTINGS: usize = 1_024;
/// The most quotes the whole store holds — open offers and trade-tape rows alike.
pub const MAX_QUOTES: usize = 2_048;
/// The most **settled trade-tape rows** the whole store holds, across every pair.
///
/// The tape's own global ceiling (independent verify fold, R4). A settled quote *is*
/// the tape row (§5.6 deviation 3), and it used to count against [`MAX_QUOTES`] with no
/// ceiling of its own: `MAX_TRADES_PER_PAIR` 32 trimmed a pair only when a **new** trade
/// landed in that same pair, so 2 048 / 32 = 64 traded pairs filled the store — three
/// venues of `sr.core`'s 27 codes — and `+market/quote` then answered "The market is
/// busy; try later." to every buyer for ever, with no new trade possible to evict
/// anything. Half of `MAX_QUOTES`, so a full tape always leaves room for the open
/// offers; past it the globally oldest **settlement** is evicted.
pub const MAX_TRADE_ROWS: usize = 1_024;

/// The tape's ceiling is **below** the quote family's, so a store full of history always
/// leaves room for open offers. Checked at compile time, because it is the whole of R4's
/// fix and a later edit that inverted it would restore the defect silently.
const _: () = assert!(MAX_TRADE_ROWS < MAX_QUOTES);
/// The most **open** quotes one buyer may hold at a time.
pub const MAX_OPEN_QUOTES_PER_BUYER: usize = 8;
/// The most registered vendors one server holds.
pub const MAX_VENDORS: usize = 64;
/// The most registered NPC market makers one server holds.
pub const MAX_SOURCES: usize = 32;
/// The longest pack name.
pub const MAX_PACK_BYTES: usize = 32;
/// The longest catalog code (`CatalogRef.code`) — E6b's own ceiling.
pub const MAX_CATALOG_CODE_BYTES: usize = 64;
/// The longest catalog entry display name.
pub const MAX_ENTRY_NAME_BYTES: usize = 64;
/// The most tags one entry carries.
pub const MAX_ENTRY_TAGS: usize = 8;
/// The longest tag.
pub const MAX_TAG_BYTES: usize = 24;
/// The longest book/page citation an entry carries. It is a citation and never
/// mechanics: nothing in this crate reads it.
pub const MAX_RULES_REF_BYTES: usize = 64;
/// The longest vendor or maker display name.
pub const MAX_VENDOR_NAME_BYTES: usize = 48;
/// The longest availability-test skill label.
pub const MAX_SKILL_BYTES: usize = 32;
/// The most dice an availability test may declare as its threshold.
pub const MAX_AVAILABILITY_THRESHOLD: u8 = 24;
/// The longest note an availability outcome records.
pub const MAX_OUTCOME_NOTE_BYTES: usize = 120;
/// The most units one listing may carry, and the most one quote may ask for.
pub const MAX_QUANTITY: u64 = 100_000;

/// What went wrong loading a document. A `&'static str` on purpose: the caps are
/// source-controlled, so every reason is a fixed sentence and no document's own bytes
/// can ever reach a refusal string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordError(pub &'static str);

impl std::fmt::Display for RecordError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for RecordError {}

// ---- the catalog (§5.1, D-E6-12, answers 9-10) -----------------------------

/// **The policy decision** — what a purchase of this entry becomes (D-E6-12).
///
/// Decided per entry by category and gameplay significance, with price only as a
/// hint (answer 9): important, serialized, restricted, upgradeable or stealable gear
/// is [`Representation::Asset`]; ammunition, drugs, reagents and commodities are
/// [`Representation::Instrument`]; licences and programs are
/// [`Representation::Entitlement`]; cyberware is [`Representation::Installed`].
///
/// [`Representation::Lot`] is **policy only in E6c** and no `Lot` record exists:
/// doc 10b §3.2 (Q2D) puts the lot in `mush-inventory` as the single source of
/// quantity truth, with a different field list (`commodity: CanonicalId`, a
/// `Lineage`, and explicitly **not** `legal` or `custody`), and E7 cut 2 owns it.
/// A listing whose entry declares `Lot` is therefore refused at creation rather than
/// producing a second, divergent definition of the same concept.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Representation {
    /// A fungible commodity: an `InstrumentKind::Commodity` account line.
    Instrument,
    /// Provenance-bearing stock. **E7 cut 2's** (`mush-inventory`); policy only here.
    Lot,
    /// An `EconomicAsset` with title, custody and encumbrances (E3).
    Asset,
    /// An `InstrumentKind::Entitlement` line — licences, programs.
    Entitlement,
    /// Becomes an installed record on installation (08f §4.4, D-E6-16).
    Installed,
}

impl Representation {
    /// The config/pack word.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Representation::Instrument => "instrument",
            Representation::Lot => "lot",
            Representation::Asset => "asset",
            Representation::Entitlement => "entitlement",
            Representation::Installed => "installed",
        }
    }
    /// Whether E6c can actually deliver this representation (OPEN 1: `Lot` cannot).
    ///
    /// **E7r2 P1 — LIFTED, not retired, and the comment above is why.** 10b §15.5
    /// called this "the `is_stockable` retirement"; it overstates the change. The
    /// predicate keeps its definition and its meaning — *E6c on its own cannot
    /// deliver a lot* — and its **one** call site (`api::list_goods`) now asks it
    /// only when the composition has **not** wired a lot supply
    /// (`MarketModule::with_inventory`). A server that composes no
    /// `mush-inventory` therefore refuses a `Representation::Lot` listing
    /// **byte-identically to E6c**, which is what
    /// `ruleset-shadowrun/tests/e6c.rs`'s `"not stockable"` assertion and
    /// `mush-market/tests/e7r2.rs`'s own pin both check.
    #[must_use]
    pub fn is_stockable(self) -> bool {
        !matches!(self, Representation::Lot)
    }
}

/// How legal the goods are where they are sold. Recorded, rendered and filtered on;
/// **never** enforced by this crate — jurisdiction is a policy test at transaction
/// time (10b §3.2), and the ruleset owns it.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Legality {
    /// Openly sold.
    #[default]
    Legal,
    /// Sold under a licence.
    Restricted,
    /// Not sold openly at all.
    Forbidden,
}

impl Legality {
    /// The config/pack word.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Legality::Legal => "legal",
            Legality::Restricted => "restricted",
            Legality::Forbidden => "forbidden",
        }
    }
    /// Parse a pack or filter word.
    #[must_use]
    pub fn parse(s: &str) -> Option<Legality> {
        match s.trim().to_ascii_lowercase().as_str() {
            "legal" => Some(Legality::Legal),
            "restricted" => Some(Legality::Restricted),
            "forbidden" => Some(Legality::Forbidden),
            _ => None,
        }
    }
}

/// Whether getting hold of this at all is itself uncertain (answer 11).
///
/// `mush-market` holds the **shape** and the recorded outcome and rolls nothing: the
/// dice are the ruleset's, behind the wiring-frozen
/// [`AvailabilityProvider`](crate::AvailabilityProvider) table.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AvailabilityRule {
    /// Stocked legal goods: no test at all.
    #[default]
    None,
    /// Scarce or restricted goods: an engine-resolved, **recorded** test.
    Test {
        /// The ruleset's own skill label (opaque here).
        skill: SmolStr,
        /// The hits the test must reach.
        threshold: u8,
        /// The seconds a failed test bars a retry for; `0` means none.
        interval: i64,
    },
    /// Exceptional or authored availability: a storyteller attests it. Rides the
    /// contract's existing `Attestation` condition, never a new one.
    Attested,
}

impl AvailabilityRule {
    /// Whether the buyer has to pass a test before a quote binds.
    #[must_use]
    pub fn needs_test(&self) -> bool {
        matches!(self, AvailabilityRule::Test { .. })
    }
}

/// The **recorded** result of one availability test — stored on the quote and
/// binding thereafter (answer 11). A re-read never re-rolls.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "AvailabilityOutcomeWire")]
pub struct AvailabilityOutcome {
    /// Whether the goods can be got at all.
    pub passed: bool,
    /// The hits the engine recorded.
    pub hits: u8,
    /// The threshold it was against.
    pub threshold: u8,
    /// When, on the wall clock.
    pub at: i64,
    /// A short, bounded engine note ("6 dice, 2 hits").
    pub note: SmolStr,
}

#[derive(Clone, Debug, Deserialize)]
struct AvailabilityOutcomeWire {
    passed: bool,
    #[serde(default)]
    hits: u8,
    #[serde(default)]
    threshold: u8,
    #[serde(default)]
    at: i64,
    #[serde(default)]
    note: SmolStr,
}

impl TryFrom<AvailabilityOutcomeWire> for AvailabilityOutcome {
    type Error = RecordError;

    fn try_from(w: AvailabilityOutcomeWire) -> Result<AvailabilityOutcome, RecordError> {
        if w.threshold > MAX_AVAILABILITY_THRESHOLD {
            return Err(RecordError("an availability threshold is out of range"));
        }
        if w.note.len() > MAX_OUTCOME_NOTE_BYTES {
            return Err(RecordError("an availability note is too long"));
        }
        Ok(AvailabilityOutcome {
            passed: w.passed,
            hits: w.hits,
            threshold: w.threshold,
            at: w.at,
            note: w.note,
        })
    }
}

/// `market:catalog:{pack}:{code}` — one catalog entry (§5.1, answers 9–10).
///
/// A vendor produces offers **from** an entry and never mutates one. `baseline` is a
/// **reference price, never an intrinsic one** (doc 08 §9.4 / D9): nothing in this
/// crate charges it, and a listing names its own price.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "CatalogEntryWire")]
pub struct CatalogEntry {
    /// The economy's opaque catalog reference — `{ ruleset, code }`.
    pub catalog: CatalogRef,
    /// The content pack this entry came from.
    pub pack: SmolStr,
    /// Bumped by the pack; a quote records the version it priced against.
    pub definition_version: u32,
    /// Display name.
    pub name: SmolStr,
    /// A **reference** price. Never charged by this crate.
    pub baseline: Option<Amount>,
    /// Whether getting hold of it is itself uncertain.
    pub availability: AvailabilityRule,
    /// How legal it is. Recorded and filtered on; never enforced here.
    pub legality: Legality,
    /// **The policy decision** (D-E6-12).
    pub representation: Representation,
    /// Whether many of it are one line.
    pub stackable: bool,
    /// Bounded, sorted tags.
    pub tags: BTreeSet<SmolStr>,
    /// A book/page citation, never mechanics.
    pub rules_ref: Option<SmolStr>,
}

impl CatalogEntry {
    /// The document key component this entry occupies: `{pack}:{code}`.
    #[must_use]
    pub fn slug(&self) -> String {
        format!("{}:{}", self.pack, self.catalog.code)
    }
}

#[derive(Clone, Debug, Deserialize)]
struct CatalogEntryWire {
    catalog: CatalogRef,
    pack: SmolStr,
    #[serde(default = "one")]
    definition_version: u32,
    #[serde(default)]
    name: SmolStr,
    #[serde(default)]
    baseline: Option<Amount>,
    #[serde(default)]
    availability: AvailabilityRule,
    #[serde(default)]
    legality: Legality,
    representation: Representation,
    #[serde(default)]
    stackable: bool,
    #[serde(default)]
    tags: BTreeSet<SmolStr>,
    #[serde(default)]
    rules_ref: Option<SmolStr>,
}

fn one() -> u32 {
    1
}

/// The one place a catalog entry's invariants live — run by the wire on load and by
/// the pack loader on write, so a document and a pack cannot disagree about what a
/// legal entry is.
///
/// # Errors
///
/// [`RecordError`] naming the first violated cap.
pub fn check_entry(e: &CatalogEntry) -> Result<(), RecordError> {
    if e.pack.trim().is_empty() || e.pack.len() > MAX_PACK_BYTES {
        return Err(RecordError("a catalog entry names no usable pack"));
    }
    if !e
        .pack
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
    {
        return Err(RecordError("a pack name holds a character a key cannot"));
    }
    if e.catalog.ruleset.trim().is_empty()
        || e.catalog.ruleset.len() > MAX_CATALOG_CODE_BYTES
        || e.catalog.code.trim().is_empty()
        || e.catalog.code.len() > MAX_CATALOG_CODE_BYTES
    {
        return Err(RecordError(
            "a catalog entry names no usable catalogue code",
        ));
    }
    if !e
        .catalog
        .code
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
    {
        return Err(RecordError(
            "a catalogue code holds a character a key cannot",
        ));
    }
    if e.name.trim().is_empty() || e.name.len() > MAX_ENTRY_NAME_BYTES {
        return Err(RecordError("a catalog entry's name is empty or too long"));
    }
    if e.definition_version == 0 {
        return Err(RecordError("a catalog entry's definition version is zero"));
    }
    if e.baseline.is_some_and(|b| b.minor() < 0) {
        return Err(RecordError("a catalog entry's baseline is negative"));
    }
    if e.tags.len() > MAX_ENTRY_TAGS
        || e.tags
            .iter()
            .any(|t| t.trim().is_empty() || t.len() > MAX_TAG_BYTES)
    {
        return Err(RecordError("a catalog entry's tags are malformed"));
    }
    if e.rules_ref
        .as_ref()
        .is_some_and(|r| r.len() > MAX_RULES_REF_BYTES)
    {
        return Err(RecordError("a catalog entry's rules reference is too long"));
    }
    if let AvailabilityRule::Test {
        skill,
        threshold,
        interval,
    } = &e.availability
    {
        if skill.trim().is_empty() || skill.len() > MAX_SKILL_BYTES {
            return Err(RecordError("an availability test names no usable skill"));
        }
        if *threshold == 0 || *threshold > MAX_AVAILABILITY_THRESHOLD {
            return Err(RecordError("an availability threshold is out of range"));
        }
        if *interval < 0 || *interval > 365 * 86_400 {
            return Err(RecordError("an availability interval is out of range"));
        }
    }
    Ok(())
}

impl TryFrom<CatalogEntryWire> for CatalogEntry {
    type Error = RecordError;

    fn try_from(w: CatalogEntryWire) -> Result<CatalogEntry, RecordError> {
        let e = CatalogEntry {
            catalog: w.catalog,
            pack: w.pack,
            definition_version: w.definition_version,
            name: w.name,
            baseline: w.baseline,
            availability: w.availability,
            legality: w.legality,
            representation: w.representation,
            stackable: w.stackable,
            tags: w.tags,
            rules_ref: w.rules_ref,
        };
        check_entry(&e)?;
        Ok(e)
    }
}

// ---- listings (§5.2) -------------------------------------------------------

/// Who may see a listing at all.
///
/// `Hidden` is the projection's own boundary: a hidden listing contributes **nothing**
/// to band, depth, volume or last trade, not even a rounding difference.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarketVisibility {
    /// Anyone may see it, and it is in the public projection.
    #[default]
    Public,
    /// Only the seller (and staff) may see it, and it is in **no** projection.
    Hidden,
}

// ---- E7r2: P1 — the lot supply's two opaque handles ------------------------

/// **An opaque handle on one `mush-inventory` endpoint.**
///
/// Opaque in the sense that matters: the market never renders it, never offers it as
/// a choice to a buyer and never lets a player name one. It is written at
/// `@market/vendor stock` from the seller's own custody check
/// (`mush_economy::may_take_from_vault`) and read only by the market's own
/// authoritative action, which is the one place a quantity mutation may happen at
/// all. A viewer of `+market/show` or `+market/depth` learns nothing from it.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LotEndpoint(pub mush_inventory::EndpointRef);

impl LotEndpoint {
    /// The vault the endpoint is.
    #[must_use]
    pub const fn vault(self) -> VaultId {
        self.0.vault()
    }
}

/// **An opaque handle on the commodity a lot listing sells.**
///
/// The second half of the `(endpoint, commodity)` pair `mush_inventory::pool_resource`
/// and `ReservationId::derive` actually key on. Like [`LotEndpoint`] it is written at
/// stock time and never rendered: the *catalogue entry* is what a buyer reads.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LotCommodity(pub mush_core::CanonicalId);

/// Where the goods come from (§5.2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Supply {
    /// Held in the vendor's storefront vault, right now.
    Stock {
        /// The vault.
        vault: VaultId,
    },
    /// Quoted by a registered NPC market maker inside its budgets.
    Market {
        /// Which maker.
        source: MarketSourceId,
    },
    /// A delayed delivery: the contract carries the lead time as a `Before`
    /// deadline, and nothing is stocked today.
    Procurement {
        /// When it can be delivered by.
        lead: EconomicDeadline,
    },
    /// **E7r2 P1 — provenance-bearing stock in `mush-inventory`** (§15.1 item 3;
    /// 10b §15.5's delegated path).
    ///
    /// The only `Supply` arm that **reserves**: a deliberate divergence from this
    /// record's own opening sentence, recorded in 08f §5.6 and in 10b §3.2's "As
    /// built". E6c's other three arms advertise a *counter* — a catalogue promise the
    /// vendor is expected to be able to honour — and over-advertising one is a
    /// commercial mistake. This arm advertises **finite matter another owner is the
    /// truth for**, and there is no honest way to publish it without holding it: the
    /// reservation is placed on `mush-inventory`'s own `ReservationBook` at
    /// `(endpoint, commodity)` when the listing is written, consumed when a contract
    /// comes into force, and released when the listing is withdrawn.
    Lot {
        /// Which inventory endpoint the stock sits in.
        endpoint: LotEndpoint,
        /// Which commodity's pool at that endpoint.
        commodity: LotCommodity,
        /// The grade the listing advertises, carried onto the buyer's lot.
        grade: u8,
    },
    /// **Doc 18 cut IC8 — a standing bid** (§4.3 "Bids that take real lots"; lead
    /// ruling L14). The listing's `seller` is the **bidding** org, its `price` the bid
    /// per unit and its `quantity` the intake still wanted. An exhausted bid stands at
    /// quantity 0 — not buying — and is the readiness report's exhausted marker (doc 18
    /// cut IC10); the sweep never deletes it, and it is the one listing the wire admits
    /// at quantity 0. It sells nothing: it is not a lot sale
    /// ([`crate::MarketStore::lot_matter`] answers `None`), it is never quoted or
    /// accepted ([`crate::WANTED_IS_A_BID`]), it is not supply in the projection, and
    /// selling into it is cut IC10's lot sell arm ([`crate::sell_into_bid`]).
    ///
    /// A recorded deviation from doc 18 §4.3's `Supply::Wanted{good, min_grade, price,
    /// remaining}` (lead ruling L21): the price and the remaining intake ride on the
    /// listing's own `price` and `quantity`, as every listing's do, so there is one
    /// truth for each.
    Wanted {
        /// The bidder's own vault, where bought matter would be received (its
        /// storefront).
        vault: VaultId,
        /// The commodity it buys.
        commodity: LotCommodity,
        /// The lowest grade it takes, `0..=mush_inventory::MAX_GRADE`.
        min_grade: u8,
    },
}

impl Supply {
    /// The lot endpoint, commodity and grade of a `Supply::Lot`, and `None` for every
    /// other arm — the one accessor every lot-aware path reads, so a new arm cannot
    /// be reached by accident. A standing bid (`Supply::Wanted`) is not a lot sale.
    #[must_use]
    pub fn lot(&self) -> Option<(mush_inventory::EndpointRef, &mush_core::CanonicalId, u8)> {
        match self {
            Supply::Lot {
                endpoint,
                commodity,
                grade,
            } => Some((endpoint.0, &commodity.0, *grade)),
            Supply::Stock { .. }
            | Supply::Market { .. }
            | Supply::Procurement { .. }
            | Supply::Wanted { .. } => None,
        }
    }

    /// Whether this is a standing bid (doc 18 cut IC8).
    #[must_use]
    pub const fn is_wanted(&self) -> bool {
        matches!(self, Supply::Wanted { .. })
    }
}

/// **What matter a lot-represented listing sells, and how it moves** (doc 18 §4.3
/// "Asks that draw from the pool", D-18-26; cut IC7, lead note 2b).
///
/// The one accessor every path reads ([`crate::MarketStore::lot_matter`]), so which op
/// a listing takes is decided in one place and never re-derived inline:
///
/// - [`LotMatter::Hold`] — a `Supply::Lot` listing (stocked with `endpoint=`), the
///   player / hold-backed listing: a reservation placed at stock time, consumed whole by
///   its one sale (`LotOp::Consume`, IC0's rules unchanged);
/// - [`LotMatter::Draw`] — a `Supply::Stock { vault }` listing on a
///   `Representation::Lot` catalogue entry: no reservation, the listing's quantity is the
///   availability counter, and the matter leg transfers from the vendor's storefront pool
///   (`LotOp::Draw`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LotMatter {
    /// The hold-backed listing: IC0's `Supply::Lot`.
    Hold {
        /// The endpoint the reservation sits in.
        endpoint: mush_inventory::EndpointRef,
        /// The commodity's pool there.
        commodity: mush_core::CanonicalId,
        /// The grade the listing advertises.
        grade: u8,
    },
    /// The drawn listing: `Supply::Stock` on a lot entry.
    Draw {
        /// The vendor's own storefront vault, as an endpoint.
        source: mush_inventory::EndpointRef,
        /// The catalogue code, as a commodity id.
        commodity: mush_core::CanonicalId,
    },
}

impl LotMatter {
    /// The endpoint the goods leave from.
    #[must_use]
    pub fn source(&self) -> mush_inventory::EndpointRef {
        match self {
            LotMatter::Hold { endpoint, .. } => *endpoint,
            LotMatter::Draw { source, .. } => *source,
        }
    }
    /// The commodity that moves.
    #[must_use]
    pub fn commodity(&self) -> &mush_core::CanonicalId {
        match self {
            LotMatter::Hold { commodity, .. } | LotMatter::Draw { commodity, .. } => commodity,
        }
    }
    /// The paid kind this matter makes (the quote's frozen terms).
    #[must_use]
    pub fn kind(&self) -> PaidKind {
        match self {
            LotMatter::Hold { .. } => PaidKind::Hold,
            LotMatter::Draw { .. } => PaidKind::Draw,
        }
    }
}

/// `market:listing:{id}` — one published intention to sell (§5.2).
///
/// It holds no money and reserves nothing. `quantity` decrements only at contract
/// **activation** (OPEN 4), which is the single serialization point that makes
/// oversell unreachable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "ListingWire")]
pub struct Listing {
    /// Identity (never reused).
    pub id: ListingId,
    /// Who is selling.
    pub seller: PrincipalId,
    /// The vendor record behind the seller.
    pub vendor: VendorId,
    /// The venue — the projection's second key.
    pub market: MarketId,
    /// What is for sale.
    pub catalog: CatalogRef,
    /// The catalog entry version this listing was published against.
    pub definition_version: u32,
    /// Where the goods come from.
    pub source: Supply,
    /// The **specific** titled object this listing conveys, for a
    /// `Representation::Asset`. `None` for every other representation.
    #[serde(default)]
    pub object: Option<ObjId>,
    /// The asking price, per unit.
    pub price: Amount,
    /// How many are left.
    pub quantity: u64,
    /// Whether it is in the public projection at all.
    pub visibility: MarketVisibility,
    /// When the listing lapses, on its own clock.
    pub expires: EconomicDeadline,
    /// Bumped by every write; a quote is bound to it.
    pub revision: u64,
    /// Doc 18 cut IC14 row 14 (lead ruling L8): whether this `Supply::Stock` listing
    /// sells by Draw, decided once at publication by `draw_listing_precheck` and never
    /// re-derived from the live catalogue entry. `None` only on a listing written
    /// before IC14, whose kind is still derived from the live catalogue entry as IC7
    /// built it, until its next write stamps it (`store::put_listing`). Every other
    /// supply writes `Some(false)`. Optional on the wire because the market's boot
    /// **skips** a listing that does not decode, so a required field would silently
    /// drop every kept listing.
    #[serde(default)]
    pub draw: Option<bool>,
}

#[derive(Clone, Debug, Deserialize)]
struct ListingWire {
    id: u64,
    seller: PrincipalId,
    vendor: u64,
    market: MarketId,
    catalog: CatalogRef,
    #[serde(default = "one")]
    definition_version: u32,
    source: Supply,
    #[serde(default)]
    object: Option<ObjId>,
    price: Amount,
    #[serde(default)]
    quantity: u64,
    #[serde(default)]
    visibility: MarketVisibility,
    expires: EconomicDeadline,
    #[serde(default)]
    revision: u64,
    #[serde(default)]
    draw: Option<bool>,
}

impl TryFrom<ListingWire> for Listing {
    type Error = RecordError;

    fn try_from(w: ListingWire) -> Result<Listing, RecordError> {
        // D-E6-4's rule applied one layer up: a `System` principal can never be a
        // contract party, so a listing whose seller is one could never settle. It is
        // refused **on the wire**, so no document can re-introduce it.
        if matches!(w.seller, PrincipalId::System(_)) {
            return Err(RecordError("a listing names a system seller"));
        }
        if w.price.minor() <= 0 {
            return Err(RecordError("a listing asks nothing"));
        }
        // Doc 18 cut IC10: an exhausted standing bid stands at quantity 0 (the
        // readiness report's exhausted marker); every other arm still refuses 0.
        if (w.quantity == 0 && !w.source.is_wanted()) || w.quantity > MAX_QUANTITY {
            return Err(RecordError("a listing's quantity is out of range"));
        }
        if w.catalog.code.trim().is_empty() || w.catalog.code.len() > MAX_CATALOG_CODE_BYTES {
            return Err(RecordError("a listing names no usable catalogue code"));
        }
        if w.definition_version == 0 {
            return Err(RecordError("a listing's definition version is zero"));
        }
        // Doc 18 cut IC14 row 14: only a storefront listing can draw.
        if w.draw == Some(true) && !matches!(w.source, Supply::Stock { .. }) {
            return Err(RecordError(
                "a listing that is not stock cannot sell by Draw",
            ));
        }
        // An `Asset` listing conveys exactly one named object; every other shape
        // names none. That is the invariant `TitleTransfer` needs and the reason a
        // quantity above one can never mint a second title.
        if w.object.is_some() && w.quantity != 1 {
            return Err(RecordError(
                "a listing that names an object sells exactly one",
            ));
        }
        // ---- E7r2: P1 — the lot arm re-runs its own caps on the wire ----
        //
        // A hand-edited grade past `MAX_GRADE` would make the buyer's lot refuse
        // `Grade::new` inside the settling bundle, i.e. **after** the money leg is in
        // force. It is refused ON LOAD instead, where a refused document is skipped
        // loudly and the seller re-lists.
        if let Supply::Lot {
            endpoint, grade, ..
        } = &w.source
        {
            if *grade > mush_inventory::MAX_GRADE {
                return Err(RecordError("a lot listing's grade is out of range"));
            }
            if w.object.is_some() {
                return Err(RecordError("a lot listing names no object"));
            }
            // The endpoint is a handle on another owner's record, never a claim: the
            // vault it names may be gone, and the listing is then reconciled away by
            // the sweep's own arm rather than refused here.
            let _ = endpoint;
        }
        // Doc 18 cut IC8: a standing bid re-runs its own caps on the wire too — a
        // grade floor past `MAX_GRADE` could never be met, and a bid conveys no object.
        if let Supply::Wanted { min_grade, .. } = &w.source {
            if *min_grade > mush_inventory::MAX_GRADE {
                return Err(RecordError("a standing bid's grade floor is out of range"));
            }
            if w.object.is_some() {
                return Err(RecordError("a standing bid names no object"));
            }
        }
        Ok(Listing {
            id: ListingId(w.id),
            seller: w.seller,
            vendor: VendorId(w.vendor),
            market: w.market,
            catalog: w.catalog,
            definition_version: w.definition_version,
            source: w.source,
            object: w.object,
            price: w.price,
            quantity: w.quantity,
            visibility: w.visibility,
            expires: w.expires,
            revision: w.revision,
            draw: w.draw,
        })
    }
}

// ---- quotes (§5.2, D-E6-13) ------------------------------------------------

/// The declared state a quote was priced against (D-E6-13, scenario 29; OPEN 2).
///
/// `mush_core::RevisionSet` — which 08f §5.2 named — is a `BTreeMap<RevisionKey, u64>`
/// in K's **action-gate** vocabulary, not an economy or market type; E6b hit exactly
/// this and landed per-record stamps
/// ([`QuoteObserved`](../../ruleset_shadowrun/economy/fence_record/struct.QuoteObserved.html)).
/// This is that decision reused: three declared stamps, each compared for
/// **equality**, and a difference refuses deterministically rather than re-pricing.
///
/// The `artefact` half is deliberately **not** an `EconomicAsset.revision`. E6b's
/// verify fold (D1) proved that construct is an oracle — every asset op bumps the
/// revision, a **covert** theft report included, so binding to it publishes a hidden
/// fact as a state change the holder can read. It is a stable hash of the
/// **declared** catalogue code instead, which no hidden fact can move.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedRevisions {
    /// The catalog entry's version at pricing time.
    pub definition_version: u32,
    /// The listing's own revision at pricing time.
    pub listing: u64,
    /// A stable hash of the declared catalogue code the quote conveys, when the
    /// listing conveys a titled artefact at all.
    pub artefact: Option<u64>,
    /// The **market maker's** own revision at pricing time, for a `Supply::Market`
    /// listing and `None` otherwise (independent verify fold, M7).
    ///
    /// `spread` is a live pricing input: `price_for` reads it at quote time and
    /// `@market/source budget` moves it. Without this stamp a re-spread maker went on
    /// honouring quotes at the old price for the whole quote window, so re-pricing
    /// through the listing lapsed a quote and re-pricing through the spread did not.
    /// It is a **declared** number bumped only by a declared staff act, so it is no
    /// more an oracle than the listing's own revision is.
    #[serde(default)]
    pub source: Option<u64>,
}

/// A stable, declaration-only hash — the same FNV-1a E6b's fence uses, so the two
/// layers stamp the same declared fact the same way.
#[must_use]
pub fn declared_hash(s: &str) -> u64 {
    let mut h: u64 = 1_469_598_103_934_665_603;
    for b in s.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(1_099_511_628_211);
    }
    h
}

/// What a settled quote records — the **trade tape row** the aggregate projection
/// folds (§5.3).
///
/// The row is an *index*, never the authority: `projection` re-asks the economy
/// whether the named contract really is `ContractState::Settled { transaction }` with
/// this transaction, so last trade and volume come from settled journal transactions
/// and from nothing else. An unsettled or withdrawn offer is invisible by
/// construction, because it carries no row at all.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settlement {
    /// The transaction the contract's terminal transition landed on.
    pub transaction: mush_economy::TransactionId,
    /// When, on the wall clock.
    pub at: i64,
}

/// `market:quote:{id}` — one buyer's bound price (§5.2, D-E6-13, answer 11).
///
/// **A quote reserves nothing** (OPEN 4; 08i §4 rule 1's exemption explicitly does not
/// cover reservations or mechanical offers, and doc 10 T4 keeps E4 `Hold`s
/// money-only). The listing's `quantity` decrements at contract **activation** and
/// nowhere else, so two quotes on the last unit are a race the economy serializes:
/// the loser refuses at `+market/accept` with the same deterministic refusal a stale
/// quote gets, and nothing is charged.
///
/// The terms are **denormalized** onto the quote deliberately: a quote must price
/// identically after its listing moved or lapsed, and a settled quote is the trade
/// tape row the projection folds, which has to describe itself when the listing is
/// gone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "QuoteWire")]
pub struct Quote {
    /// Identity (never reused).
    pub id: QuoteId,
    /// The listing it was asked against.
    pub listing: ListingId,
    /// Who asked.
    pub buyer: PrincipalId,
    /// Who is selling (denormalized; the projection's public test asks about them).
    pub seller: PrincipalId,
    /// What is being bought (denormalized).
    pub catalog: CatalogRef,
    /// Where (denormalized) — the projection's second key.
    pub market: MarketId,
    /// Whether the listing was in the public projection (denormalized).
    pub visibility: MarketVisibility,
    /// How the listing was supplied (denormalized) — a maker's own budget is charged
    /// against this.
    pub source: Option<MarketSourceId>,
    /// The unit price, frozen.
    pub price: Amount,
    /// How many units.
    pub quantity: u64,
    /// **What the deal actually booked against the maker's loss budget** at
    /// activation (independent verify fold, M5), and `Amount::ZERO` while the quote is
    /// still an open offer.
    ///
    /// Denormalized for the same reason `price` and `quantity` are: `charge_budgets`
    /// used to recompute the loss from the catalogue's *live* baseline on both legs,
    /// so any pack reload between the spend and the return made the return differ from
    /// the spend — a raised baseline returned more than was spent (and `.max(0)` then
    /// destroyed every other deal's booked loss), a lowered one returned less and drove
    /// `loss_used` permanently upward. The return subtracts exactly this, so return
    /// equals spend by construction rather than by the catalogue holding still.
    #[serde(default)]
    pub booked_loss: Amount,
    /// Whether the deal actually **came into force** — the listing's quantity was
    /// committed and the maker's window charged (independent verify fold, R9).
    ///
    /// The contract link is written as soon as `offer_contract` returns, before either
    /// acceptance, so no transition can fire against a quote nothing can find. That
    /// makes a pre-activation `Cancelled` reachable, and the return leg must not give
    /// back stock that never left or budget that was never spent — so it keys on this
    /// rather than on the link.
    #[serde(default)]
    pub activated: bool,
    /// Whether the buyer **acts for the selling organization** — recorded at
    /// activation (independent verify fold, M12).
    ///
    /// A member of the vendor's own organization can buy from their own shop: the
    /// contract really settles, so the row is honest, but the price it prints as
    /// `last_trade` is one the seller chose and paid themselves. Such a row is a real
    /// trade and stays in the buyer's own order history; it contributes **nothing** to
    /// the public tape.
    #[serde(default)]
    pub self_dealt: bool,
    /// The catalog entry version priced against.
    pub definition_version: u32,
    /// Every declared stamp the price was bound to.
    pub observed: ObservedRevisions,
    /// When it lapses — **its clock is declared** (answer 11, D-E6-13).
    pub expires: EconomicDeadline,
    /// The recorded availability test, when the entry demanded one.
    #[serde(default)]
    pub availability: Option<AvailabilityOutcome>,
    /// The contract, once `+market/accept` drafted one.
    #[serde(default)]
    pub contract: Option<ContractId>,
    /// The trade tape row, once that contract settled.
    #[serde(default)]
    pub settled: Option<Settlement>,
    /// **The lot terms the money was taken for**, frozen at activation of every lot
    /// sale of both kinds (doc 18 cut IC7), and `None` on every other quote.
    ///
    /// Denormalized for the reason `price` is: a `Supply::Lot` sale deletes its listing,
    /// so nothing later may re-derive the terms from a listing. The paid-leg projection
    /// (`lot::project_offer`) reads them, and nothing else is the authority for what a
    /// paid matter leg may move.
    #[serde(default)]
    pub lot: Option<PaidLot>,
    /// **When the market learned that a Draw's goods moved** (real `World::now()`
    /// seconds; doc 18 cut IC7 item 7) — the mirror of the durable draw receipt the
    /// Draw wrote in its own bundle, set only by the market sweep's mirror arm, and
    /// `Some` only on a Draw quote. The `market.lot.drawn` fact answers from it.
    #[serde(default)]
    pub drawn_at: Option<i64>,
    /// **Who besides the seller is paid out of a sale into a standing bid** (doc 18
    /// §4.1 "The share", D-18-24; cut IC10): the proceeds policy's cuts, frozen at the
    /// offer, each an `Exact` payout of the contract. At most [`MAX_PROCEEDS_CUTS`], and
    /// only on a Deliver quote.
    #[serde(default)]
    pub cuts: Vec<ProceedsCut>,
    /// What the proceeds policy measured its cuts on — for the Co-op, the margin over
    /// reference; `Amount::ZERO` otherwise (cut IC10). Only a Deliver quote carries a
    /// non-zero basis.
    #[serde(default = "zero")]
    pub basis: Amount,
    /// Bumped by every write.
    pub revision: u64,
}

/// The most payees a sale into a bid pays beyond its two parties (doc 18 §4.1, cut
/// IC10, lead ruling (b)): one — the Co-op First Captain Trust's share.
pub const MAX_PROCEEDS_CUTS: usize = 1;
/// The longest label a proceeds cut carries (cut IC10).
pub const MAX_PROCEEDS_LABEL_BYTES: usize = 32;

/// **One payee of a sale into a standing bid beyond the seller** (doc 18 §4.1 "The
/// share", D-18-24; cut IC10): an `Exact` payout of `amount` to `to`, named `label` on
/// the sale line. Game-neutral: the market validates the split, the policy names it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProceedsCut {
    /// The account paid.
    pub to: AccountId,
    /// How much (positive).
    pub amount: Amount,
    /// The line's name for it, 1..=32 bytes.
    pub label: SmolStr,
}

impl ProceedsCut {
    /// The cut's invariants: a positive amount and a 1..=32-byte label.
    ///
    /// # Errors
    /// [`RecordError`] naming the broken invariant.
    pub fn check(&self) -> Result<(), RecordError> {
        if self.amount.minor() <= 0 {
            return Err(RecordError("a proceeds cut pays nothing"));
        }
        if self.label.trim().is_empty() || self.label.len() > MAX_PROCEEDS_LABEL_BYTES {
            return Err(RecordError("a proceeds cut's label is out of range"));
        }
        Ok(())
    }
}

/// Which kind of paid lot leg a quote's money was taken for (doc 18 cut IC7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaidKind {
    /// A hold-backed `Supply::Lot` sale: `LotOp::Consume`.
    Hold,
    /// A drawn `Supply::Stock` sale on a lot entry: `LotOp::Draw`.
    Draw,
    /// A sale into a standing bid: `LotOp::Deliver` (doc 18 §4.3 "Bids that take real
    /// lots", D-18-26; cut IC10). The quote's buyer is the bidding house, its seller the
    /// player, and the contract is gated on the leg's `inventory.transferred` marker.
    Deliver,
}

/// **The frozen terms of one paid lot sale** (doc 18 cut IC7, L4/L12), written on the
/// quote at activation.
///
/// Clocks (F9): `accepted_at` and `draw_by` are **real** `World::now()` seconds — the
/// clock of the contract's `Before { RealUtc }` and of [`crate::DEAL_WINDOW_SECS`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaidLot {
    /// Which leg.
    pub kind: PaidKind,
    /// Where the goods come from (the seller's endpoint).
    pub source: LotEndpoint,
    /// Where they go (the buyer's endpoint).
    pub destination: LotEndpoint,
    /// What moves.
    pub commodity: LotCommodity,
    /// The advertised grade (a hold-backed sale's; zero on a Draw).
    pub grade: u8,
    /// When the deal came into force (real seconds).
    pub accepted_at: i64,
    /// The leg's cutoff (a Draw's or a Deliver's; real seconds): no such leg is
    /// offered, and so none is prepared, after it (`DRAW_SETTLE_MARGIN_SECS` before the
    /// window shuts). `Some` exactly for a Draw and a Deliver.
    #[serde(default)]
    pub draw_by: Option<i64>,
    /// The `inventory.transferred` applied-marker id a Deliver's contract is gated on
    /// (`mush_inventory::derived_marker_id`, derived at offer time from the contract,
    /// the listing and the quantity; doc 18 §4.3, cut IC10). `Some` exactly for a
    /// Deliver, 1..=96 bytes of lowercase hex.
    #[serde(default)]
    pub marker: Option<SmolStr>,
}

impl PaidLot {
    /// The record's invariants: two distinct endpoints, a cutoff on exactly the Draw
    /// and Deliver kinds, never before the deal itself, and a well-formed marker on
    /// exactly the Deliver kind (cut IC10).
    ///
    /// # Errors
    /// [`RecordError`] naming the broken invariant.
    pub fn check(&self) -> Result<(), RecordError> {
        if self.source == self.destination {
            return Err(RecordError("a paid lot moves between two endpoints"));
        }
        if self.grade > mush_inventory::MAX_GRADE {
            return Err(RecordError("a paid lot's grade is out of range"));
        }
        match (self.kind, self.draw_by, self.marker.as_deref()) {
            (PaidKind::Draw, Some(by), None) if by >= self.accepted_at => Ok(()),
            (PaidKind::Deliver, Some(by), Some(marker))
                if by >= self.accepted_at && mush_inventory::is_marker_id(marker) =>
            {
                Ok(())
            }
            (PaidKind::Hold, None, None) => Ok(()),
            (PaidKind::Draw, _, Some(_)) => Err(RecordError("a paid draw carries no marker")),
            (PaidKind::Draw, _, None) => {
                Err(RecordError("a paid draw carries a cutoff after its deal"))
            }
            (PaidKind::Deliver, _, _) => Err(RecordError(
                "a paid delivery carries a cutoff after its deal and a marker",
            )),
            (PaidKind::Hold, _, _) => Err(RecordError(
                "a paid hold sale carries no cutoff and no marker",
            )),
        }
    }
}

/// The `market.lot.drawn` fact kind (doc 18 cut IC7, L10): true once the market has
/// mirrored a Draw's durable receipt. Answered by [`crate::MarketFactProvider`].
pub const FACT_LOT_DRAWN: &str = "market.lot.drawn";

/// **The durable proof a Draw moved its matter** (doc 18 cut IC7 item 6), written by the
/// Draw prepare in its own bundle under `market_lot_draws[contract]`.
///
/// It is the epoch-free replay guard (a second Draw for one contract finds it and
/// refuses — the inventory applied marker's rule) and the evidence the market sweep
/// mirrors onto the quote's `drawn_at`. `at_ic` is the IC instant of the bundle (the lot
/// lane's fixed `now`), never a wall clock.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrawReceipt {
    /// The purchase contract the Draw delivered against.
    pub contract: ContractId,
    /// Its quote.
    pub quote: QuoteId,
    /// The listing it was drawn from.
    pub listing: ListingId,
    /// The storefront vault.
    pub source: VaultId,
    /// The buyer's vault.
    pub destination: VaultId,
    /// The commodity id.
    pub commodity: SmolStr,
    /// How much moved.
    pub quantity: u64,
    /// Who submitted it.
    pub actor: ObjId,
    /// The IC second of the bundle.
    pub at_ic: i64,
}

impl DrawReceipt {
    /// The record's invariants (the family validator runs this).
    ///
    /// # Errors
    /// The broken invariant, as text.
    pub fn check(&self) -> Result<(), String> {
        if self.quantity == 0 {
            return Err("a draw receipt names a non-zero quantity".to_owned());
        }
        if self.source == self.destination {
            return Err("a draw receipt moves between two vaults".to_owned());
        }
        if mush_core::CanonicalId::new(self.commodity.as_str()).is_err() {
            return Err("a draw receipt names a canonical commodity id".to_owned());
        }
        Ok(())
    }
}

impl Quote {
    /// What the whole quote costs — unit price times quantity, saturating.
    #[must_use]
    pub fn total(&self) -> Amount {
        Amount::from_minor(
            self.price
                .minor()
                .saturating_mul(i64::try_from(self.quantity).unwrap_or(i64::MAX)),
        )
    }
    /// Whether this quote is a settled **trade**, not an open offer.
    #[must_use]
    pub fn is_trade(&self) -> bool {
        self.settled.is_some()
    }
}

#[derive(Clone, Debug, Deserialize)]
struct QuoteWire {
    id: u64,
    listing: u64,
    buyer: PrincipalId,
    seller: PrincipalId,
    catalog: CatalogRef,
    market: MarketId,
    #[serde(default)]
    visibility: MarketVisibility,
    #[serde(default)]
    source: Option<MarketSourceId>,
    price: Amount,
    #[serde(default)]
    quantity: u64,
    #[serde(default = "zero")]
    booked_loss: Amount,
    #[serde(default)]
    activated: bool,
    #[serde(default)]
    self_dealt: bool,
    #[serde(default = "one")]
    definition_version: u32,
    observed: ObservedRevisions,
    expires: EconomicDeadline,
    #[serde(default)]
    availability: Option<AvailabilityOutcome>,
    #[serde(default)]
    contract: Option<u64>,
    #[serde(default)]
    settled: Option<Settlement>,
    #[serde(default)]
    lot: Option<PaidLot>,
    #[serde(default)]
    drawn_at: Option<i64>,
    #[serde(default)]
    cuts: Vec<ProceedsCut>,
    #[serde(default = "zero")]
    basis: Amount,
    #[serde(default)]
    revision: u64,
}

impl TryFrom<QuoteWire> for Quote {
    type Error = RecordError;

    fn try_from(w: QuoteWire) -> Result<Quote, RecordError> {
        if matches!(w.buyer, PrincipalId::System(_)) || matches!(w.seller, PrincipalId::System(_)) {
            return Err(RecordError("a quote names a system principal"));
        }
        if w.price.minor() <= 0 {
            return Err(RecordError("a quote offers nothing"));
        }
        if w.quantity == 0 || w.quantity > MAX_QUANTITY {
            return Err(RecordError("a quote's quantity is out of range"));
        }
        if w.catalog.code.trim().is_empty() || w.catalog.code.len() > MAX_CATALOG_CODE_BYTES {
            return Err(RecordError("a quote names no usable catalogue code"));
        }
        if w.definition_version == 0 {
            return Err(RecordError("a quote's definition version is zero"));
        }
        // A settled quote is a trade-tape row and a trade needs the contract it
        // settled on: a row naming no contract could never be re-checked against the
        // economy, which is the one thing that makes the projection's inputs
        // *journal* transactions rather than market documents.
        if w.settled.is_some() && w.contract.is_none() {
            return Err(RecordError("a settled quote names no contract"));
        }
        // The booked loss is what a cancellation gives back, so a negative one would
        // *mint* budget on the return leg — the one direction `charge_budgets`'
        // saturation cannot recover from.
        if w.booked_loss.minor() < 0 {
            return Err(RecordError("a quote's booked loss is negative"));
        }
        // Doc 18 cut IC7: the frozen lot terms re-run their own invariants on load, and
        // the draw mirror exists only on a Draw quote.
        if let Some(lot) = &w.lot {
            lot.check()?;
        }
        if w.drawn_at.is_some() && !w.lot.as_ref().is_some_and(|l| l.kind == PaidKind::Draw) {
            return Err(RecordError("only a drawn sale records when it was drawn"));
        }
        // Doc 18 cut IC10: the proceeds split re-runs its caps on load — at most one
        // payee beyond the parties, every cut well-formed, cuts and a basis only on a
        // sale into a bid, and the cuts never more than the whole sale.
        check_proceeds(&w)?;
        Ok(Quote {
            id: QuoteId(w.id),
            listing: ListingId(w.listing),
            buyer: w.buyer,
            seller: w.seller,
            catalog: w.catalog,
            market: w.market,
            visibility: w.visibility,
            source: w.source,
            price: w.price,
            quantity: w.quantity,
            booked_loss: w.booked_loss,
            activated: w.activated,
            self_dealt: w.self_dealt,
            definition_version: w.definition_version,
            observed: w.observed,
            expires: w.expires,
            availability: w.availability,
            contract: w.contract.map(ContractId),
            settled: w.settled,
            lot: w.lot,
            drawn_at: w.drawn_at,
            cuts: w.cuts,
            basis: w.basis,
            revision: w.revision,
        })
    }
}

/// The proceeds half of a quote's wire check (doc 18 §4.1, cut IC10).
fn check_proceeds(w: &QuoteWire) -> Result<(), RecordError> {
    let delivers = w.lot.as_ref().is_some_and(|l| l.kind == PaidKind::Deliver);
    if w.cuts.len() > MAX_PROCEEDS_CUTS {
        return Err(RecordError("a quote pays too many proceeds cuts"));
    }
    if (!w.cuts.is_empty() || w.basis.minor() != 0) && !delivers {
        return Err(RecordError("only a sale into a bid carries proceeds cuts"));
    }
    if w.basis.minor() < 0 {
        return Err(RecordError("a quote's proceeds basis is negative"));
    }
    let mut cut_total: i64 = 0;
    for cut in &w.cuts {
        cut.check()?;
        cut_total = cut_total
            .checked_add(cut.amount.minor())
            .ok_or(RecordError("a quote's proceeds cuts overflow"))?;
    }
    if cut_total == 0 {
        return Ok(());
    }
    let gross = i64::try_from(w.quantity)
        .ok()
        .and_then(|quantity| w.price.minor().checked_mul(quantity))
        .ok_or(RecordError("a quote's total overflows"))?;
    if cut_total > gross {
        return Err(RecordError("a quote's proceeds cuts exceed the sale"));
    }
    Ok(())
}

// ---- vendors (§5.2, B5) ----------------------------------------------------

/// A vendor's lifecycle. A retired vendor stops quoting; its live contracts settle
/// through E4 untouched (B10).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VendorStatus {
    /// Trading.
    #[default]
    Active,
    /// Retired. Quotes nothing; live contracts still settle.
    Retired,
}

/// `market:vendor:{id}` — a staff-registered vendor **organization** (B5).
///
/// The neutral generalization of E6b's `@sr/fence <org>=<vault>`, and the table
/// `with_party_caps` is fed from: it holds no money and no authority of its own, and
/// all it adds is that the world knows this organization trades, from which vault,
/// into which account, in which market.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "VendorWire")]
pub struct Vendor {
    /// Identity (never reused).
    pub id: VendorId,
    /// The organization that is the vendor.
    pub org: OrgId,
    /// The venue it trades in.
    pub market: MarketId,
    /// Its own cash account.
    pub account: AccountId,
    /// Its storefront vault — where a delivered object lands.
    pub vault: VaultId,
    /// Its shop sign. The organization's own name renders through the economy's
    /// hidden-org renderer and never from here.
    pub name: SmolStr,
    /// The permille of an entry's baseline it pays a player for goods
    /// (`+market/sell`), clamped to 0..=1000.
    pub buy_permille: u32,
    /// Lifecycle.
    pub status: VendorStatus,
    /// Bumped by every write.
    pub revision: u64,
}

impl Vendor {
    /// Whether this vendor may issue new offers at all.
    #[must_use]
    pub fn is_trading(&self) -> bool {
        matches!(self.status, VendorStatus::Active)
    }
}

#[derive(Clone, Debug, Deserialize)]
struct VendorWire {
    id: u64,
    org: OrgId,
    market: MarketId,
    account: AccountId,
    vault: VaultId,
    #[serde(default)]
    name: SmolStr,
    #[serde(default)]
    buy_permille: u32,
    #[serde(default)]
    status: VendorStatus,
    #[serde(default)]
    revision: u64,
}

impl TryFrom<VendorWire> for Vendor {
    type Error = RecordError;

    fn try_from(w: VendorWire) -> Result<Vendor, RecordError> {
        if w.name.len() > MAX_VENDOR_NAME_BYTES {
            return Err(RecordError("a vendor's name is too long"));
        }
        if w.buy_permille > 1_000 {
            return Err(RecordError("a vendor's buy rate is out of range"));
        }
        Ok(Vendor {
            id: VendorId(w.id),
            org: w.org,
            market: w.market,
            account: w.account,
            vault: w.vault,
            name: w.name,
            buy_permille: w.buy_permille,
            status: w.status,
            revision: w.revision,
        })
    }
}

// ---- NPC market makers (§5.2, doc 10 7A, D-E6-15) --------------------------

/// `market:source:{id}` — an NPC market maker with **explicit, exhaustible** budgets
/// (doc 10 7A, D-E6-15).
///
/// Rejected by 7A and unreachable here: an infinite system market maker. A maker is
/// never a `System` party (D-E6-4), never holds an unlimited account, and nothing
/// recapitalizes it without an attributed act. "Mediocre prices is a trading policy,
/// not a substitute for funding": `spread` moves the price and never the funding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "MarketSourceWire")]
pub struct MarketSource {
    /// Identity (never reused).
    pub id: MarketSourceId,
    /// The organization behind it. Never a `System` principal.
    pub principal: OrgId,
    /// The venue it makes a market in.
    pub market: MarketId,
    /// Its cash account — the whole of its money.
    pub cash: AccountId,
    /// Its inventory vault.
    pub inventory: VaultId,
    /// Its price spread over the catalog baseline, in permille.
    pub spread: i64,
    /// The most it may sell, in minor units, per window.
    pub volume_budget: Amount,
    /// The most it may lose against baseline, in minor units, per window.
    pub loss_budget: Amount,
    /// The window length in seconds.
    pub window: i64,
    /// How much it has sold in the current window.
    pub volume_used: Amount,
    /// How much it has lost against baseline in the current window.
    pub loss_used: Amount,
    /// When the current window began, on the wall clock.
    pub window_started: i64,
    /// Whether staff halted it. A halted maker quotes nothing; its outstanding
    /// quotes expire and settle nothing.
    pub halted: bool,
    /// Bumped by every write.
    pub revision: u64,
}

impl MarketSource {
    /// Roll the window forward when it has lapsed, zeroing the per-window counters.
    /// Pure — the caller writes the record.
    #[must_use]
    pub fn rolled(&self, now: i64) -> MarketSource {
        if self.window <= 0 || now < self.window_started.saturating_add(self.window) {
            return self.clone();
        }
        MarketSource {
            volume_used: Amount::ZERO,
            loss_used: Amount::ZERO,
            window_started: now,
            ..self.clone()
        }
    }
    /// Whether the maker may quote `total` more minor units right now, and why not.
    ///
    /// Every budget is checked **before the quote is issued**, never at accept
    /// (D-E6-15): a maker that has run out simply stops quoting, and there is no
    /// half-committed state to unwind.
    #[must_use]
    pub fn refuses(&self, total: Amount, loss: Amount) -> Option<&'static str> {
        if self.halted {
            return Some("halted");
        }
        if self.volume_used.minor().saturating_add(total.minor()) > self.volume_budget.minor() {
            return Some("volume");
        }
        if self.loss_used.minor().saturating_add(loss.minor().max(0)) > self.loss_budget.minor() {
            return Some("loss");
        }
        None
    }
}

#[derive(Clone, Debug, Deserialize)]
struct MarketSourceWire {
    id: u64,
    principal: OrgId,
    market: MarketId,
    cash: AccountId,
    inventory: VaultId,
    #[serde(default)]
    spread: i64,
    #[serde(default = "zero")]
    volume_budget: Amount,
    #[serde(default = "zero")]
    loss_budget: Amount,
    #[serde(default)]
    window: i64,
    #[serde(default = "zero")]
    volume_used: Amount,
    #[serde(default = "zero")]
    loss_used: Amount,
    #[serde(default)]
    window_started: i64,
    #[serde(default)]
    halted: bool,
    #[serde(default)]
    revision: u64,
}

fn zero() -> Amount {
    Amount::ZERO
}

impl TryFrom<MarketSourceWire> for MarketSource {
    type Error = RecordError;

    fn try_from(w: MarketSourceWire) -> Result<MarketSource, RecordError> {
        if !(-1_000..=1_000).contains(&w.spread) {
            return Err(RecordError("a maker's spread is out of range"));
        }
        if w.volume_budget.minor() < 0 || w.loss_budget.minor() < 0 {
            return Err(RecordError("a maker's budget is negative"));
        }
        if w.volume_used.minor() < 0 || w.loss_used.minor() < 0 {
            return Err(RecordError("a maker's usage is negative"));
        }
        if !(60..=365 * 86_400).contains(&w.window) {
            return Err(RecordError("a maker's window is out of range"));
        }
        Ok(MarketSource {
            id: MarketSourceId(w.id),
            principal: w.principal,
            market: w.market,
            cash: w.cash,
            inventory: w.inventory,
            spread: w.spread,
            volume_budget: w.volume_budget,
            loss_budget: w.loss_budget,
            window: w.window,
            volume_used: w.volume_used,
            loss_used: w.loss_used,
            window_started: w.window_started,
            halted: w.halted,
            revision: w.revision,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry_json() -> serde_json::Value {
        serde_json::json!({
            "catalog": { "ruleset": "sr", "code": "ares.predator" },
            "pack": "sr.core", "definition_version": 1, "name": "Ares Predator V",
            "baseline": 725, "representation": "asset", "legality": "restricted",
        })
    }

    #[test]
    fn a_catalog_entry_round_trips_and_its_caps_are_on_the_wire() {
        let e: CatalogEntry = serde_json::from_value(entry_json()).expect("loads");
        assert_eq!(e.representation, Representation::Asset);
        assert_eq!(e.legality, Legality::Restricted);
        let mut bad = entry_json();
        bad["name"] = serde_json::json!("x".repeat(MAX_ENTRY_NAME_BYTES + 1));
        assert!(serde_json::from_value::<CatalogEntry>(bad).is_err());
        let mut bad = entry_json();
        bad["definition_version"] = serde_json::json!(0);
        assert!(serde_json::from_value::<CatalogEntry>(bad).is_err());
        let mut bad = entry_json();
        bad["catalog"]["code"] = serde_json::json!("has space");
        assert!(serde_json::from_value::<CatalogEntry>(bad).is_err());
    }

    #[test]
    fn a_system_seller_never_loads() {
        let bad = serde_json::json!({
            "id": 1, "seller": { "System": "sr.chargen" }, "vendor": 1, "market": "seattle",
            "catalog": { "ruleset": "sr", "code": "ammo" }, "source": { "stock": { "vault": 1 } },
            "price": 10, "quantity": 5, "expires": { "clock": "RealUtc", "at": 10 },
        });
        assert!(serde_json::from_value::<Listing>(bad).is_err());
    }

    #[test]
    fn a_settled_quote_naming_no_contract_never_loads() {
        let bad = serde_json::json!({
            "id": 1, "listing": 1, "buyer": { "Object": 5 }, "seller": { "Organization": 1 },
            "catalog": { "ruleset": "sr", "code": "ammo" }, "market": "seattle",
            "price": 10, "quantity": 1, "observed": { "definition_version": 1, "listing": 1, "artefact": null },
            "expires": { "clock": "RealUtc", "at": 10 },
            "settled": { "transaction": 3, "at": 1 },
        });
        assert!(serde_json::from_value::<Quote>(bad).is_err());
    }

    #[test]
    fn a_makers_budgets_refuse_before_the_quote_is_issued() {
        let m = MarketSource {
            id: MarketSourceId(1),
            principal: OrgId(1),
            market: MarketId::new("seattle").expect("valid"),
            cash: AccountId(1),
            inventory: VaultId(1),
            spread: 0,
            volume_budget: Amount::from_minor(100),
            loss_budget: Amount::from_minor(10),
            window: 3_600,
            volume_used: Amount::from_minor(95),
            loss_used: Amount::ZERO,
            window_started: 0,
            halted: false,
            revision: 1,
        };
        assert_eq!(m.refuses(Amount::from_minor(4), Amount::ZERO), None);
        assert_eq!(
            m.refuses(Amount::from_minor(6), Amount::ZERO),
            Some("volume")
        );
        assert_eq!(
            m.refuses(Amount::from_minor(1), Amount::from_minor(11)),
            Some("loss")
        );
        let halted = MarketSource {
            halted: true,
            ..m.clone()
        };
        assert_eq!(
            halted.refuses(Amount::ZERO, Amount::ZERO),
            Some("halted"),
            "a halted maker quotes nothing at all"
        );
        let rolled = m.rolled(m.window_started + m.window + 1);
        assert_eq!(rolled.volume_used, Amount::ZERO);
    }

    // ---- doc 18 cut IC10: the sale-into-a-bid wire rules ----

    const MARKER: &str =
        "8000000000000064aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn wanted_json(quantity: u64) -> serde_json::Value {
        serde_json::json!({
            "id": 4, "seller": { "Organization": 2 }, "vendor": 1, "market": "stickney",
            "catalog": { "ruleset": "tacsim", "code": "metals" },
            "source": { "wanted": { "vault": 9, "commodity": "metals", "min_grade": 0 } },
            "price": 28000, "quantity": quantity, "expires": { "clock": "RealUtc", "at": 10 },
        })
    }

    #[test]
    fn ic10_only_a_standing_bid_stands_at_quantity_zero() {
        let zero: Listing = serde_json::from_value(wanted_json(0)).expect("a zero bid loads");
        assert_eq!(zero.quantity, 0);
        assert!(serde_json::from_value::<Listing>(wanted_json(MAX_QUANTITY + 1)).is_err());
        let ask = serde_json::json!({
            "id": 1, "seller": { "Organization": 1 }, "vendor": 1, "market": "seattle",
            "catalog": { "ruleset": "sr", "code": "ammo" }, "source": { "stock": { "vault": 1 } },
            "price": 10, "quantity": 0, "expires": { "clock": "RealUtc", "at": 10 },
        });
        assert!(
            serde_json::from_value::<Listing>(ask).is_err(),
            "an ask at 0 never loads"
        );
    }

    fn paid(kind: &str, draw_by: Option<i64>, marker: Option<&str>) -> serde_json::Value {
        serde_json::json!({
            "kind": kind, "source": 11, "destination": 12, "commodity": "metals",
            "grade": 0, "accepted_at": 100, "draw_by": draw_by, "marker": marker,
        })
    }

    fn check(v: serde_json::Value) -> Result<(), RecordError> {
        serde_json::from_value::<PaidLot>(v).expect("shape").check()
    }

    #[test]
    fn ic10_a_paid_delivery_carries_a_cutoff_and_a_marker() {
        assert!(check(paid("deliver", Some(3_100), Some(MARKER))).is_ok());
        assert!(check(paid("deliver", None, Some(MARKER))).is_err());
        assert!(check(paid("deliver", Some(99), Some(MARKER))).is_err());
        assert!(check(paid("deliver", Some(3_100), None)).is_err());
        assert!(check(paid("deliver", Some(3_100), Some("XYZ"))).is_err());
        assert!(check(paid("deliver", Some(3_100), Some(&"a".repeat(97)))).is_err());
        // A Draw keeps its rule and carries no marker; a Hold carries neither.
        assert!(check(paid("draw", Some(3_100), None)).is_ok());
        assert!(check(paid("draw", Some(3_100), Some(MARKER))).is_err());
        assert!(check(paid("hold", None, None)).is_ok());
        assert!(check(paid("hold", None, Some(MARKER))).is_err());
        assert!(check(paid("hold", Some(3_100), None)).is_err());
    }

    fn sale_quote(
        lot: serde_json::Value,
        cuts: serde_json::Value,
        basis: i64,
    ) -> serde_json::Value {
        serde_json::json!({
            "id": 1, "listing": 4, "buyer": { "Organization": 2 }, "seller": { "Object": 5 },
            "catalog": { "ruleset": "tacsim", "code": "metals" }, "market": "stickney",
            "price": 28000, "quantity": 20,
            "observed": { "definition_version": 1, "listing": 1, "artefact": null },
            "expires": { "clock": "RealUtc", "at": 3_700 }, "contract": 3, "activated": true,
            "lot": lot, "cuts": cuts, "basis": basis,
        })
    }

    fn loads(v: serde_json::Value) -> bool {
        serde_json::from_value::<Quote>(v).is_ok()
    }

    #[test]
    fn ic10_a_quotes_proceeds_split_rides_only_a_sale_into_a_bid() {
        let deliver = paid("deliver", Some(3_100), Some(MARKER));
        let share = serde_json::json!([{ "to": 7, "amount": 40000, "label": "Co-op share" }]);
        let q: Quote = serde_json::from_value(sale_quote(deliver.clone(), share.clone(), 160_000))
            .expect("a sale with its share loads");
        assert_eq!(q.cuts.len(), 1);
        assert_eq!(q.basis, Amount::from_minor(160_000));
        // Two payees beyond the parties.
        let two = serde_json::json!([
            { "to": 7, "amount": 1, "label": "a" }, { "to": 8, "amount": 1, "label": "b" }
        ]);
        assert!(!loads(sale_quote(deliver.clone(), two, 0)));
        // A cut that pays nothing, names no label, carries a label over the cap, or an
        // unknown field.
        for bad in [
            serde_json::json!([{ "to": 7, "amount": 0, "label": "x" }]),
            serde_json::json!([{ "to": 7, "amount": 5, "label": " " }]),
            serde_json::json!([{ "to": 7, "amount": 5, "label": "x".repeat(33) }]),
            serde_json::json!([{ "to": 7, "amount": 5, "label": "x", "extra": 1 }]),
        ] {
            assert!(!loads(sale_quote(deliver.clone(), bad, 0)));
        }
        // Cuts over the whole sale (20 × 28,000 = 560,000).
        let over = serde_json::json!([{ "to": 7, "amount": 560_001, "label": "x" }]);
        assert!(!loads(sale_quote(deliver.clone(), over, 0)));
        let exact = serde_json::json!([{ "to": 7, "amount": 560_000, "label": "x" }]);
        assert!(loads(sale_quote(deliver.clone(), exact, 0)));
        // A negative basis.
        assert!(!loads(sale_quote(deliver, serde_json::json!([]), -1)));
        // Cuts or a basis on anything but a Deliver quote.
        let draw = paid("draw", Some(3_100), None);
        assert!(!loads(sale_quote(draw.clone(), share, 0)));
        assert!(!loads(sale_quote(draw.clone(), serde_json::json!([]), 5)));
        assert!(loads(sale_quote(draw, serde_json::json!([]), 0)));
    }
}
