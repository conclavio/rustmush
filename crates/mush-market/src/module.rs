//! The [`MarketModule`] — the cut that attaches the market to a running world, and
//! the three **wiring-frozen** tables a ruleset registers into (E6c, B2).
//!
//! Every one of the three follows `mush-economy`'s own `with_custody` / `with_facts`
//! / `with_title_issuers` pattern exactly: a table that could change at runtime would
//! make "who could sell you this, and at what odds" a function of when you asked.
//!
//! | table | what it answers | why it is not in this crate |
//! |---|---|---|
//! | [`MarketModule::with_catalog_sources`] | "what is in pack `<name>`" | content is the ruleset's (answer 20) |
//! | [`MarketModule::with_availability`] | "can one be got hold of" | the **dice** are the ruleset's; this crate holds the rule shape and the recorded outcome, and rolls nothing (B7) |
//! | [`MarketModule::with_delivery`] | "the money settled — hand the goods over" | what a commodity line, an entitlement or an installed record *means* is the ruleset's (D-E6-16) |
//!
//! `Representation::Asset` needs no delivery provider at all: title moves as an
//! ordinary E4b2 `Deliverable::TitleTransfer` the settlement already executes.

use std::sync::Arc;

use mush_core::module::{GameModule, ModError, ModuleBootData, ModuleCtx, Registrar, TickCadence};
use mush_core::{Ctx, ObjId};
use mush_economy::{CatalogRef, ContractId, EconomicEvent, EconomyObserver};
use mush_org::PrincipalId;

use crate::record::{AvailabilityOutcome, AvailabilityRule, CatalogEntry, Quote, Representation};
use crate::store::{MarketStore, NS};

/// The market sweep's tick tag.
pub const SWEEP_TAG: &str = "market.sweep";

/// How often the market sweep fires, in seconds.
pub const SWEEP_PERIOD_SECS: u64 = 60;

/// Something that can hand the market a content pack's entries (B3, B9).
///
/// Frozen at wiring. A source answers `None` for a pack it does not own, so the
/// composition can carry one per ruleset without either knowing about the other.
pub trait CatalogSource: Send + Sync + std::fmt::Debug {
    /// The entries of `pack`, or `None` when this source does not define it.
    /// `Some(Err(..))` is a pack this source owns and could not read — the message
    /// names the file, and the **pack** fails, never the server.
    fn pack(&self, pack: &str) -> Option<Result<Vec<CatalogEntry>, String>>;
}

/// Something that can resolve an availability test (B7, answer 11).
///
/// The engine is the ruleset's — Shadowrun's is `mush_dice::sr5::{Sr5Pool, Sr5Opts}`
/// under a `RollCtx`, the non-rendering seam, deliberately **not** `roll.rs`'s
/// `roll_labeled`, which renders to the actor. Rule 2 (08i §4) is satisfied by
/// construction: a provider is composition-frozen deterministic policy, and no LLM
/// decides a price, a budget or an availability outcome.
pub trait AvailabilityProvider: Send + Sync + std::fmt::Debug {
    /// Resolve one test for `buyer` against `catalog`, or `None` when this provider
    /// does not own that catalogue's ruleset.
    fn resolve(
        &self,
        ctx: &mut Ctx<'_>,
        rule: &AvailabilityRule,
        catalog: &CatalogRef,
        buyer: ObjId,
    ) -> Option<AvailabilityOutcome>;
}

/// What a settled purchase asks a provider to hand over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeliveryOrder {
    /// What the catalog says this becomes.
    pub representation: Representation,
    /// What was bought.
    pub catalog: CatalogRef,
    /// Who bought it.
    pub buyer: PrincipalId,
    /// Who sold it.
    pub seller: PrincipalId,
    /// How many units.
    pub quantity: u64,
    /// The contract that settled.
    pub contract: ContractId,
}

/// Something that can hand over the goods once the money has settled (B6).
///
/// **Not settlement machinery**: E4 settles the money and executes the contract's own
/// `TitleTransfer`; this is the delivery act that follows, and it runs through the
/// economy's existing posting seams under the seller's own authority — exactly as
/// E6b's `+fence/deliver` moves custody outside the settlement.
pub trait DeliveryProvider: Send + Sync + std::fmt::Debug {
    /// Deliver one order, or `None` when this provider does not own that
    /// representation or that ruleset. `Some(Err(..))` says what went wrong.
    fn deliver(&self, ctx: &mut Ctx<'_>, order: &DeliveryOrder) -> Option<Result<(), String>>;

    /// **Could this order be delivered right now?** — asked at `+market/accept`,
    /// before any money moves (independent verify fold, M1).
    ///
    /// `None` from every provider means nothing on this server owns the question, and
    /// the accept proceeds exactly as it did: this is a pre-check, never a second
    /// authority, and a provider that cannot answer cheaply should keep the default.
    /// `Some(Err(why))` refuses the purchase with the goods still undelivered and the
    /// buyer's money still theirs — which is the whole point, because the money moves
    /// at settlement and the goods leg is asked only afterwards.
    ///
    /// The default answers `None`, so every existing provider keeps working unchanged.
    fn can_deliver(&self, _ctx: &Ctx<'_>, _order: &DeliveryOrder) -> Option<Result<(), String>> {
        None
    }
}

// ---- doc 18 cut IC7: the content-port seams ---------------------------------
//
// The market adds three composition-frozen hooks of its own — the presence gate, the
// capacity hook and (cut IC10) the proceeds policy — in the same `with_*` shape as the
// tables above, and game-neutral in the same way: none of them names a game, and with
// none installed the market behaves exactly as it did before IC7. The tacsim
// implementations are cuts IC8's and IC10's.

/// Doc 18 §4.3 "Presence" (D-18-12 extended): how the actor is present for a trade.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Presence {
    /// A typed verb: the actor stands at the counter (the gate judges what that means).
    Here,
    /// A queued order (IC11): the hull's locality and the author's lease; the room is
    /// ignored.
    Ship(ObjId),
}

/// A gate's refusal, in the gate's own words; rendered verbatim, nothing charged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VenueRefusal {
    /// The line the actor reads.
    pub line: String,
}

/// Doc 18 §4.3: the presence gate a content composition installs
/// ([`MarketModule::with_venue_gate`]).
///
/// Game-neutral: it names no game. Every method is total and reads `w` only. With no
/// gate installed the market admits every venue exactly as it did before IC7, and the
/// `market=` / `to=` defaults are simply absent. The tacsim implementation is IC8's.
pub trait VenueGate: Send + Sync + std::fmt::Debug {
    /// The venue a room belongs to — the `market=` default (`venue_of_room`).
    fn venue_of_room(&self, w: &mush_core::World, room: ObjId) -> Option<crate::MarketId>;
    /// The endpoint a no-destination accept delivers into for `actor` at `venue` (in
    /// IC8, the berthed hold) — the `to=` default; IC10's `from=` reads the same answer.
    fn endpoint_for(
        &self,
        w: &mush_core::World,
        actor: ObjId,
        venue: &crate::MarketId,
        presence: Presence,
    ) -> Option<mush_inventory::EndpointRef>;
    /// May `actor` trade at `venue` (delivering into `endpoint`, when one is known)
    /// under `presence`? `Ok` for a venue this gate does not govern.
    ///
    /// # Errors
    /// The gate's own refusal line, rendered verbatim.
    fn admit(
        &self,
        w: &mush_core::World,
        actor: ObjId,
        venue: &crate::MarketId,
        endpoint: Option<mush_inventory::EndpointRef>,
        presence: Presence,
    ) -> Result<(), VenueRefusal>;
    /// Whether `endpoint` is governed (in IC8, a hull's hold): only a port listing
    /// delivers there (lead note 2e).
    fn governed(&self, w: &mush_core::World, endpoint: mush_inventory::EndpointRef) -> bool;
    /// Whether `seller` is one of `venue`'s own port institutions (L7).
    fn port_seller(
        &self,
        w: &mush_core::World,
        venue: &crate::MarketId,
        seller: &PrincipalId,
    ) -> bool;
    /// The name of the person who signs for `seller` at `market`, when the gate
    /// knows one (doc 18 cut IC14 row 52; §4.3; Q-18-47): `+market/accept`'s success
    /// line names them. Default: nobody, so every gate that does not answer keeps
    /// the line unchanged.
    fn signatory(
        &self,
        _w: &mush_core::World,
        _market: &crate::MarketId,
        _seller: &PrincipalId,
    ) -> Option<String> {
        None
    }
}

/// What a governed endpoint's limits are, resolved where a World exists and carried,
/// bounded and opaque to the market, to the Draw prepare where none does (doc 18 §4.3
/// "The capacity function"; L3).
///
/// The market never reads inside it: the provider that made it is the one that checks
/// against it, and the offer carries it in its server-authenticated fixed parameters.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CapacityPlan(pub serde_json::Value);

/// The largest plan, in canonical JSON bytes; a larger one refuses (fail closed).
pub const MAX_CAPACITY_PLAN_BYTES: usize = 2_048;

impl CapacityPlan {
    /// Whether the plan is inside [`MAX_CAPACITY_PLAN_BYTES`] once serialized. A plan
    /// that will not serialize counts as oversized, so it refuses.
    #[must_use]
    pub fn within_bounds(&self) -> bool {
        serde_json::to_vec(&self.0).is_ok_and(|bytes| bytes.len() <= MAX_CAPACITY_PLAN_BYTES)
    }
}

/// A pool the capacity rule needs could not be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapacityReadError;

/// Why a capacity check refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CapacityRefusal {
    /// It does not fit; `fits` more would, and `line` says so in the provider's words.
    Refused {
        /// How much more of the commodity would fit.
        fits: u64,
        /// The line the actor reads, verbatim.
        line: String,
    },
    /// A pool the rule needs could not be read: refuse, never read as zero.
    Unreadable,
}

/// Doc 18 §4.3 "The capacity function", the market-side hook (D-18-28 as replaced by
/// R7), installed with [`MarketModule::with_capacity_provider`].
///
/// Split in two (L3) because the Draw prepare has no World: `plan` maps an endpoint to
/// its limits where a World exists (the accept pre-check, the offer projection at
/// drain), and `check` runs against that plan wherever the pools can be read — in the
/// accept pre-check through the command's typed-state reader, and again as the backstop
/// inside the Draw's own `ActionTxn`. No provider = no capacity rule. The tacsim
/// implementation is IC8's.
pub trait CapacityProvider: Send + Sync + std::fmt::Debug {
    /// `None` for an ungoverned endpoint (no rule; nothing more is asked).
    fn plan(
        &self,
        w: &mush_core::World,
        endpoint: mush_inventory::EndpointRef,
    ) -> Option<CapacityPlan>;
    /// Would `quantity` more of `commodity` fit? `read_pool` answers the endpoint's pool
    /// of a commodity: `Ok(0)` when absent, `Err` when unreadable (which must refuse).
    ///
    /// # Errors
    /// [`CapacityRefusal`] when it does not fit or a pool could not be read.
    fn check(
        &self,
        plan: &CapacityPlan,
        commodity: &mush_core::CanonicalId,
        quantity: u64,
        read_pool: &mut dyn FnMut(&mush_core::CanonicalId) -> Result<u64, CapacityReadError>,
    ) -> Result<(), CapacityRefusal>;
}

// ---- doc 18 cut IC10: the proceeds seam ------------------------------------------

/// The longest sale line a proceeds policy may return (cut IC10).
pub const MAX_SALE_LINE_BYTES: usize = 512;
/// The longest memo a proceeds policy may append to the sale contract's name (cut
/// IC10) — the ledger entry's own record of, e.g., a baseline reference.
pub const MAX_PROCEEDS_MEMO_BYTES: usize = 24;

/// What a sale into a standing bid is, as the proceeds policy is asked about it (doc 18
/// §4.1 "The share", D-18-24; cut IC10). Every refusal of the market's own has already
/// passed when the policy is asked.
#[derive(Clone, Debug)]
pub struct SaleTerms<'a> {
    /// The seller — the player who typed the sale (or IC11's executor's principal).
    pub seller: ObjId,
    /// The endpoint the goods leave from.
    pub source: mush_inventory::EndpointRef,
    /// The bidding house (the bid listing's seller).
    pub house: &'a PrincipalId,
    /// The venue.
    pub market: &'a crate::MarketId,
    /// The bid listing.
    pub listing: crate::ListingId,
    /// The commodity sold.
    pub commodity: &'a mush_core::CanonicalId,
    /// How much.
    pub quantity: u64,
    /// The bid's price per unit.
    pub price: mush_economy::Amount,
    /// `price × quantity` — the house's escrow.
    pub gross: mush_economy::Amount,
}

/// What the proceeds policy answers for one sale (cut IC10): its cuts (each an `Exact`
/// payout beside the seller's), the basis it measured them on, the sale line the seller
/// reads (or `None` for the market's own), and an optional memo for the contract name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proceeds {
    /// Who besides the seller is paid, at most [`crate::MAX_PROCEEDS_CUTS`].
    pub cuts: Vec<crate::ProceedsCut>,
    /// What the cuts were measured on (the margin over reference for the Co-op).
    pub basis: mush_economy::Amount,
    /// The line the seller reads, at most [`MAX_SALE_LINE_BYTES`].
    pub line: Option<String>,
    /// A memo appended to the contract's name, at most [`MAX_PROCEEDS_MEMO_BYTES`].
    pub memo: Option<smol_str::SmolStr>,
}

impl Default for Proceeds {
    /// No cuts, a zero basis, the market's own line, no memo — the unwired answer.
    fn default() -> Proceeds {
        Proceeds {
            cuts: Vec::new(),
            basis: mush_economy::Amount::ZERO,
            line: None,
            memo: None,
        }
    }
}

/// A proceeds policy's refusal, in its own words; nothing is offered (cut IC10).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProceedsRefusal {
    /// The line the seller reads.
    pub line: String,
}

/// One sale into a bid settled (cut IC10): what the policy's `settled` hook is told.
#[derive(Clone, Debug)]
pub struct SettledSale<'a> {
    /// The sale's quote.
    pub quote: crate::QuoteId,
    /// Its contract.
    pub contract: ContractId,
    /// The seller (the quote's seller principal).
    pub seller: &'a PrincipalId,
    /// The endpoint the goods left.
    pub source: mush_inventory::EndpointRef,
    /// The commodity.
    pub commodity: &'a mush_core::CanonicalId,
    /// How much.
    pub quantity: u64,
    /// The bid price per unit.
    pub price: mush_economy::Amount,
    /// `price × quantity`.
    pub gross: mush_economy::Amount,
    /// The cuts frozen on the quote at the offer.
    pub cuts: &'a [crate::ProceedsCut],
    /// The basis frozen on the quote at the offer.
    pub basis: mush_economy::Amount,
}

/// Doc 18 §4.1 "The share" (D-18-24), game-neutral: who besides the seller is paid out
/// of a sale into a standing bid, and the sale's line (cut IC10), installed with
/// [`MarketModule::with_proceeds_policy`]. **Unwired is: no cuts, the market's own
/// line.** The tacsim implementation is `CoopProceeds`.
pub trait ProceedsPolicy: Send + Sync + std::fmt::Debug {
    /// Asked once per sale, before any contract is offered, reading `w` only.
    ///
    /// # Errors
    /// The policy's own refusal line; the sale is refused before any contract.
    fn proceeds(
        &self,
        w: &mush_core::World,
        sale: &SaleTerms<'_>,
    ) -> Result<Proceeds, ProceedsRefusal>;
    /// One sale into a bid settled: called exactly once per sale, from the market's
    /// settlement ear after its own idempotency check. Default: nothing.
    fn settled(&self, _ctx: &mut Ctx<'_>, _sale: &SettledSale<'_>) {}
}

/// The market [`GameModule`].
///
/// Boots **after** `mush-economy` (it validates listings against real accounts,
/// assets and vaults) and **before** the rulesets, which register into its three
/// tables.
#[derive(Default)]
pub struct MarketModule {
    catalogs: Vec<Arc<dyn CatalogSource>>,
    availability: Vec<Arc<dyn AvailabilityProvider>>,
    delivery: Vec<Arc<dyn DeliveryProvider>>,
    issuer: Option<mush_org::SystemPrincipalId>,
    inventory: Option<mush_inventory::InventoryHandleCell>,
    /// Doc 18 §4.3 (cut IC7): the presence gate. `None` = pre-IC7 behaviour exactly.
    venue_gate: Option<Arc<dyn VenueGate>>,
    /// Doc 18 §4.3 (cut IC7): the capacity hook. `None` = no capacity rule.
    capacity: Option<Arc<dyn CapacityProvider>>,
    /// Doc 18 §4.1 (cut IC10): the proceeds policy. `None` = no cuts, the market's line.
    proceeds: Option<Arc<dyn ProceedsPolicy>>,
    original_mind_quotes: Option<crate::mind_trade::OriginalQuoteConfig>,
    original_mind_families: Option<crate::mind_trade::Families>,
    original_mind_purchases: Option<crate::mind_trade::OriginalPurchaseConfig>,
    original_purchase_families: Option<crate::mind_trade::PurchaseFamilies>,
    mind_entitlements: Option<crate::mind_entitlement::EntitlementConfig>,
    house_minds: Option<crate::mind_house::HouseConfig>,
    house_families: Option<crate::mind_house::Families>,
    entitlement_families: Option<crate::mind_entitlement::Families>,
    mind_renewals: Option<crate::mind_renewal::RenewalConfig>,
    renewal_families: Option<crate::mind_renewal::Families>,
    house_renewals: Option<crate::mind_house_renewal::RenewalConfig>,
    house_renewal_families: Option<crate::mind_house_renewal::Families>,
}

impl std::fmt::Debug for MarketModule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MarketModule")
            .field("catalog_sources", &self.catalogs.len())
            .field("availability_providers", &self.availability.len())
            .field("delivery_providers", &self.delivery.len())
            .field("venue_gate", &self.venue_gate.is_some())
            .field("capacity_provider", &self.capacity.is_some())
            .field("proceeds_policy", &self.proceeds.is_some())
            .finish()
    }
}

impl MarketModule {
    /// Register the typed original-personal listing/quote actions. Roll must
    /// explicitly grant the matching reader and a real venue gate is required.
    /// This does not activate managed purchase, money movement or paid grants.
    #[must_use]
    pub fn with_original_mind_quotes(
        mut self,
        config: crate::mind_trade::OriginalQuoteConfig,
    ) -> Self {
        self.original_mind_quotes = Some(config);
        self
    }

    /// Opt into the closed paired purchase actions using Roll's exact frozen
    /// writer and Economy participant. This requires the same quote policy.
    #[must_use]
    pub fn with_original_mind_purchases(
        mut self,
        config: crate::mind_trade::OriginalPurchaseConfig,
    ) -> Self {
        self.original_mind_quotes = Some(config.quotes.clone());
        self.original_mind_purchases = Some(config);
        self
    }

    /// Explicit original-v3 / derived-v1 personal Pin commerce. The host must
    /// supply the matching Roll writer, Economy actions and current manifest.
    #[must_use]
    pub fn with_house_minds(mut self, config: crate::mind_house::HouseConfig) -> Self {
        self.house_minds = Some(config);
        self
    }

    pub fn with_mind_entitlements(
        mut self,
        config: crate::mind_entitlement::EntitlementConfig,
    ) -> Self {
        self.mind_entitlements = Some(config);
        self
    }

    /// Explicit paired renewal providers. The optional service adds bounded
    /// continuations and controls; acceptance is always explicit.
    pub fn with_mind_renewals(mut self, config: crate::mind_renewal::RenewalConfig) -> Self {
        self.mind_renewals = Some(config);
        self
    }

    /// Explicit House renewal composition; buyers always accept new money holds.
    #[must_use]
    pub fn with_house_mind_renewals(
        mut self,
        config: crate::mind_house_renewal::RenewalConfig,
    ) -> Self {
        self.house_renewals = Some(config);
        self
    }

    /// A fresh module with every table empty — a server with no ruleset content
    /// registers the verbs, loads no pack, resolves no availability test and delivers
    /// nothing but title. Fail-closed in all three directions.
    #[must_use]
    pub fn new() -> MarketModule {
        MarketModule::default()
    }
    /// Freeze the content-pack sources.
    #[must_use]
    pub fn with_catalog_sources(mut self, t: Vec<Arc<dyn CatalogSource>>) -> MarketModule {
        self.catalogs = t;
        self
    }
    /// Freeze the availability engines.
    #[must_use]
    pub fn with_availability(mut self, t: Vec<Arc<dyn AvailabilityProvider>>) -> MarketModule {
        self.availability = t;
        self
    }
    /// Freeze the delivery providers.
    #[must_use]
    pub fn with_delivery(mut self, t: Vec<Arc<dyn DeliveryProvider>>) -> MarketModule {
        self.delivery = t;
        self
    }
    /// Freeze the system principal a stocked asset's title is minted under (B11,
    /// §2 item 8, D-E3-13).
    ///
    /// The **server** names it, exactly as it names housing's, and
    /// `mush_economy::issue_title` refuses an issuer that is not also on
    /// `EconomyModule::with_title_issuers` — so a mismatch is a loud refusal at
    /// `@market/vendor stock` rather than a silent capability, and this crate stays
    /// game-neutral by being told rather than by knowing.
    #[must_use]
    pub fn with_issuer(mut self, issuer: mush_org::SystemPrincipalId) -> MarketModule {
        self.issuer = Some(issuer);
        self
    }

    /// ---- E7r2: P1 — take cut 2's family handles from the composition root ----
    ///
    /// `IndustryModule::with_inventory`'s shape exactly, for the same reason: module
    /// state is installed **after** every module has registered, so the handles cannot
    /// be read from `&World` while this module registers — and the lot action's
    /// provider needs them then. The composition root fills the cell by composing
    /// `mush-inventory` first (the boot order it already fixes: inventory, industry,
    /// …, market), and it must **also** name `"market"` in
    /// `InventoryModule::with_quantity_delegates`: the cell carries a **capability**,
    /// and the delegation is what lets a market-owned `ActionTxn` write with it.
    ///
    /// **Unwired is fail-closed and byte-identical to E6c**: no lot action is
    /// registered at all, `Representation::is_stockable` keeps its one call site, and
    /// `@market/vendor stock` refuses a lot-represented good with the sentence it has
    /// refused it with since E6c.
    #[must_use]
    pub fn with_inventory(mut self, cell: mush_inventory::InventoryHandleCell) -> MarketModule {
        self.inventory = Some(cell);
        self
    }

    /// Freeze the **presence gate** (doc 18 §4.3 "Presence", D-18-12 extended; cut
    /// IC7). Consulted at quote, accept and sell; it supplies the no-argument `market=`
    /// and `to=` defaults, and it says which endpoints are governed so a non-port
    /// listing never delivers into one. **Unwired is the pre-IC7 behaviour exactly.**
    /// The tacsim implementation is IC8's.
    #[must_use]
    pub fn with_venue_gate(mut self, gate: Arc<dyn VenueGate>) -> MarketModule {
        self.venue_gate = Some(gate);
        self
    }

    /// Freeze the **capacity hook** (doc 18 §4.3 "The capacity function"; cut IC7).
    /// Asked in the accept pre-check and again as the backstop inside the Draw's
    /// prepare. **Unwired is no capacity rule.** The tacsim implementation is IC8's.
    #[must_use]
    pub fn with_capacity_provider(mut self, provider: Arc<dyn CapacityProvider>) -> MarketModule {
        self.capacity = Some(provider);
        self
    }

    /// Freeze the **proceeds policy** (doc 18 §4.1 "The share", D-18-24; cut IC10).
    /// Asked once per sale into a standing bid before any contract, and told once per
    /// settled sale. **Unwired is: no cuts, the market's own line.** The tacsim
    /// implementation is `CoopProceeds`.
    #[must_use]
    pub fn with_proceeds_policy(mut self, policy: Arc<dyn ProceedsPolicy>) -> MarketModule {
        self.proceeds = Some(policy);
        self
    }
}

/// The frozen tables, parked in the store so a handler can reach them.
#[derive(Clone, Default)]
pub(crate) struct MarketTables {
    pub(crate) catalogs: Vec<Arc<dyn CatalogSource>>,
    pub(crate) availability: Vec<Arc<dyn AvailabilityProvider>>,
    pub(crate) delivery: Vec<Arc<dyn DeliveryProvider>>,
    pub(crate) issuer: Option<mush_org::SystemPrincipalId>,
    /// E7r2 P1: cut 2's delegated handles, when the composition wired them. `None` is
    /// **fail-closed** — a lot listing is refused byte-identically to E6c.
    pub(crate) inventory: Option<mush_inventory::InventoryFamilies>,
    /// Doc 18 cut IC7: the presence gate, when the composition installed one.
    pub(crate) venue_gate: Option<Arc<dyn VenueGate>>,
    /// Doc 18 cut IC7: the capacity hook, when the composition installed one.
    pub(crate) capacity: Option<Arc<dyn CapacityProvider>>,
    /// Doc 18 cut IC10: the proceeds policy, when the composition installed one.
    pub(crate) proceeds: Option<Arc<dyn ProceedsPolicy>>,
    /// Doc 18 cut IC7 item 6: the market-owned draw-receipt family, registered with the
    /// lot action (so `None` exactly when no lot supply is composed).
    pub(crate) draws: Option<DrawReceipts>,
}

/// The draw-receipt family's handle type (doc 18 cut IC7 item 6).
pub type DrawReceipts = mush_core::StateFamilyHandle<ContractId, crate::record::DrawReceipt>;

impl std::fmt::Debug for MarketTables {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MarketTables")
            .field("proceeds_policy", &self.proceeds.is_some())
            .finish_non_exhaustive()
    }
}

impl GameModule for MarketModule {
    fn name(&self) -> &'static str {
        NS
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        // S6: the economist's interface (doc 14 §4.6).
        reg.interface_provider(
            crate::interface::MARKET_INTERFACE,
            std::sync::Arc::new(crate::interface::MarketInterface),
        )?;
        // ---- E7r2: P1 — the lot supply's own handles, before anything else ----
        //
        // Read here and not later because `lot::register` needs them at registration
        // time and because `MarketStore`'s frozen table carries them for the command
        // layer's own pre-checks. `None` is the E6c composition: no action, no lot
        // listing, and the same refusal E6c printed.
        let inventory = self.inventory.as_ref().and_then(|cell| cell.get().cloned());
        // K8.5 module compatibility: the module's own documents are bounded
        // whole-document post-images on the compatibility lane (08i §4, rule 1). The
        // two player-facing verbs are classified **per group** in
        // `crate::commands::register`, which is where their own `reg.command`s are —
        // the discipline that failed this very crate's 140 tests at the K merge.
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        // ---- doc 18 cut IC7 item 6: the draw-receipt family, before the slot ----
        //
        // A typed family registered only when the composition wired a lot supply (it is
        // the lot action's own evidence), and before `reg.state` because its handle is
        // parked in the frozen tables. The action provider itself is still registered
        // further down, after the tick, exactly where it always was.
        let draws = match &inventory {
            Some(_) => Some(crate::lot::register_draws(reg)?),
            None => None,
        };
        reg.state(MarketStore {
            tables: MarketTables {
                catalogs: self.catalogs.clone(),
                availability: self.availability.clone(),
                delivery: self.delivery.clone(),
                issuer: self.issuer.clone(),
                inventory: inventory.clone(),
                venue_gate: self.venue_gate.clone(),
                capacity: self.capacity.clone(),
                proceeds: self.proceeds.clone(),
                draws: draws.clone(),
            },
            ..MarketStore::default()
        });
        // **The market has a clock of its own** (independent verify fold, R3/R4).
        // Nothing used to remove a lapsed listing or an aged trade-tape row: a listing
        // left `live` for ever, still holding one of its vendor's 64 slots and still
        // rendered as buyable, and 64 traded pairs' rows filled `MAX_QUOTES` so
        // `+market/quote` answered "The market is busy; try later." to every buyer for
        // ever, with no new trade possible to evict anything.
        reg.tick(
            SWEEP_TAG,
            TickCadence::Coarse(std::time::Duration::from_secs(SWEEP_PERIOD_SECS)),
            Arc::new(crate::boot::sweep_tick),
        )?;
        // ---- E7r2: P1 — the ONE market-owned authoritative action ----
        //
        // Registered **after** `reg.classify_commands(..)` and only when the
        // composition wired the delegated handles (§15.5 (c′)). It is the whole of the
        // lift's mechanic: `reserve` at `@market/vendor stock`, `settle_sale` at
        // `+market/accept`, `release` at `@market/vendor withdraw`, all on cut 2's own
        // exported verbs under `lot::listing_holder(ListingId)`.
        let mut quote_definitions = self
            .original_mind_quotes
            .as_ref()
            .map(crate::mind_trade::OriginalQuoteConfig::declarations)
            .transpose()?
            .unwrap_or_default();
        let purchase_definitions = self
            .original_mind_purchases
            .as_ref()
            .map(crate::mind_trade::OriginalPurchaseConfig::declarations)
            .transpose()?
            .unwrap_or_default();
        let entitlement_definitions = self
            .mind_entitlements
            .as_ref()
            .map(crate::mind_entitlement::EntitlementConfig::declarations)
            .transpose()?
            .unwrap_or_default();
        let house_definitions = self
            .house_minds
            .as_ref()
            .map(crate::mind_house::HouseConfig::declarations)
            .transpose()?
            .unwrap_or_default();
        let renewal_definitions = self
            .mind_renewals
            .as_ref()
            .map(crate::mind_renewal::RenewalConfig::declarations)
            .transpose()?
            .unwrap_or_default();
        if let Some(config) = &self.mind_renewals {
            let entitlement = self
                .mind_entitlements
                .as_ref()
                .ok_or_else(|| ModError::msg("renewals require entitlement composition"))?;
            if self.original_mind_purchases.is_none()
                || config.manifest() != entitlement.manifest()
                || config.participants() != entitlement.participants()
            {
                return Err(ModError::msg("renewal owner composition differs"));
            }
        }
        let house_renewal_definitions = self
            .house_renewals
            .as_ref()
            .map(crate::mind_house_renewal::RenewalConfig::declarations)
            .transpose()?
            .unwrap_or_default();
        if let Some(config) = &self.house_renewals {
            let house = self
                .house_minds
                .as_ref()
                .ok_or_else(|| ModError::msg("House renewals require House purchases"))?;
            if config.manifest() != house.manifest()
                || config.participants() != house.participants()
                || config.purchase_policy() != &house.policy_ref()?
            {
                return Err(ModError::msg("House renewal composition differs"));
            }
        }
        let mut purchase_definitions = purchase_definitions;
        purchase_definitions.extend(entitlement_definitions.iter().cloned());
        purchase_definitions.extend(house_definitions.iter().cloned());
        purchase_definitions.extend(renewal_definitions.iter().cloned());
        purchase_definitions.extend(house_renewal_definitions.iter().cloned());
        quote_definitions.extend(purchase_definitions.iter().cloned());
        crate::mind_entitlement::register_capabilities_with_house(
            reg,
            self.original_mind_purchases.as_ref(),
            self.mind_entitlements.as_ref(),
            self.house_minds.as_ref(),
            self.venue_gate.clone(),
        )?;
        if let Some(purchase) = &self.original_mind_purchases {
            let quotes = self
                .original_mind_quotes
                .as_ref()
                .ok_or_else(|| ModError::msg("paid purchase requires quote registration"))?;
            if quotes.declarations()? != purchase.quotes.declarations()? {
                return Err(ModError::msg("purchase and quote policies differ"));
            }
        }
        let lot_registered = inventory.is_some() && draws.is_some();
        if let (Some(families), Some(draws)) = (&inventory, draws) {
            crate::lot::register(
                reg,
                families,
                draws,
                self.capacity.clone(),
                &quote_definitions,
            )?;
        }
        if let Some(config) = &self.original_mind_quotes {
            let gate = self.venue_gate.clone().ok_or_else(|| {
                ModError::msg("original mind quotes require the actual venue gate")
            })?;
            let offers = if !purchase_definitions.is_empty() {
                crate::mind_trade::register_with_purchase_definitions(
                    reg,
                    config,
                    gate.clone(),
                    lot_registered,
                    &purchase_definitions,
                )?
            } else {
                crate::mind_trade::register(reg, config, gate.clone(), lot_registered)?
            };
            if let Some(purchase) = &self.original_mind_purchases {
                self.original_purchase_families = Some(crate::mind_trade::register_purchase(
                    reg,
                    purchase,
                    offers.clone(),
                    gate,
                )?);
            }
            self.original_mind_families = Some(offers);
        }
        if let Some(config) = &self.mind_entitlements {
            let gate = self
                .venue_gate
                .clone()
                .ok_or_else(|| ModError::msg("entitlements require the actual venue gate"))?;
            self.entitlement_families =
                Some(crate::mind_entitlement::register_with_house_definitions(
                    reg,
                    config,
                    gate,
                    lot_registered || self.original_mind_quotes.is_some(),
                    &house_definitions
                        .iter()
                        .chain(house_renewal_definitions.iter())
                        .cloned()
                        .collect::<Vec<_>>(),
                )?);
        }
        if let Some(config) = &self.house_minds {
            let gate = self
                .venue_gate
                .clone()
                .ok_or_else(|| ModError::msg("house purchase requires the actual venue gate"))?;
            self.house_families = Some(crate::mind_house::register_with_renewal_definitions(
                reg,
                config,
                gate,
                lot_registered
                    || self.original_mind_quotes.is_some()
                    || self.mind_entitlements.is_some(),
                self.mind_entitlements.is_some(),
                &house_renewal_definitions,
            )?);
        }
        if let Some(config) = &self.mind_renewals {
            self.renewal_families = Some(crate::mind_renewal::register(reg, config)?);
        }
        if let Some(config) = &self.house_renewals {
            self.house_renewal_families = Some(crate::mind_house_renewal::register(reg, config)?);
        }
        crate::commands::register(reg)?;
        Ok(())
    }

    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        let kv: Vec<(smol_str::SmolStr, serde_json::Value)> = data
            .kv
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let loaded = crate::store::load(&kv);
        {
            let Some(store) = ctx.w.module_state_mut::<MarketStore>() else {
                return Err(ModError::msg("market: no module state"));
            };
            let tables = store.tables.clone();
            *store = MarketStore { tables, ..loaded };
        }
        crate::boot::reconcile(ctx);
        if let Some(families) = &self.original_mind_families {
            families.audit(ctx)?;
        }
        Ok(())
    }
    fn finalize_owner_composition(&mut self, ctx: &mut ModuleCtx<'_>) -> Result<(), ModError> {
        if let Some(config) = &self.original_mind_purchases {
            let families = self
                .original_purchase_families
                .as_ref()
                .ok_or_else(|| ModError::msg("purchase families are unavailable"))?;
            let offers = self
                .original_mind_families
                .as_ref()
                .ok_or_else(|| ModError::msg("purchase quote families are unavailable"))?;
            families.audit_with_houses(
                ctx,
                config,
                offers,
                self.mind_entitlements.is_some(),
                self.house_minds.is_some(),
            )?;
            crate::mind_trade::enable_purchase_service(ctx)?;
        }
        if let Some(config) = &self.mind_entitlements {
            self.entitlement_families
                .as_ref()
                .ok_or_else(|| ModError::msg("entitlement families unavailable"))?
                .audit_with_house(
                    ctx,
                    config,
                    self.original_mind_purchases.is_some(),
                    self.house_minds.is_some(),
                )?;
        }
        if let Some(config) = &self.house_minds {
            self.house_families
                .as_ref()
                .ok_or_else(|| ModError::msg("house families unavailable"))?
                .audit(
                    ctx,
                    config,
                    self.original_mind_purchases.is_some(),
                    self.mind_entitlements.is_some(),
                )?;
        }
        if let Some(config) = &self.mind_renewals {
            self.renewal_families
                .as_ref()
                .ok_or_else(|| ModError::msg("renewal families unavailable"))?
                .audit(ctx, config)?;
            crate::mind_renewal::enable_service(ctx)?;
        } else {
            crate::mind_renewal::require_disabled(ctx)?;
        }
        if let Some(config) = &self.house_renewals {
            self.house_renewal_families
                .as_ref()
                .ok_or_else(|| ModError::msg("House renewal families unavailable"))?
                .audit(ctx, config)?;
            crate::mind_house_renewal::enable_service(ctx)?;
        } else {
            crate::mind_house_renewal::require_disabled(ctx)?;
        }
        Ok(())
    }
}

/// The market's ear on the economy — one observer, frozen at wiring beside the
/// others (`EconomyModule::with_observers`).
///
/// **Exhaustive over every `EconomicEvent` variant with no `?`, `unwrap`, `expect` or
/// panicking index**: it runs post-commit in the world task, where a panic takes the
/// world down. `SrBoardObserver`'s six-variant match is the shape.
#[derive(Debug, Default)]
pub struct MarketObserver;

impl EconomyObserver for MarketObserver {
    fn on_economic_event(&self, ctx: &mut Ctx<'_>, ev: &EconomicEvent) {
        match ev {
            EconomicEvent::ContractChanged { id, state, .. } => {
                crate::trade::on_contract_changed(ctx, *id, *state);
            }
            EconomicEvent::TransactionPosted { .. }
            | EconomicEvent::HoldChanged { .. }
            | EconomicEvent::ObligationChanged { .. }
            | EconomicEvent::AssetTitleChanged { .. }
            | EconomicEvent::EscalationOpened { .. } => {}
        }
    }
}

// ---- the table lookups ------------------------------------------------------

/// The frozen title issuer, when the composition named one.
pub(crate) fn title_issuer(ctx: &Ctx<'_>) -> Option<mush_org::SystemPrincipalId> {
    ctx.w.module_state::<MarketStore>()?.tables.issuer.clone()
}

/// **The delegated quantity handles, when the composition wired them** (E7r2 P1).
///
/// `None` on a server that composed no lot supply, which is the E6c composition and is
/// fail-closed in both directions: no lot listing may be published and no lot action
/// exists to move one.
#[must_use]
pub fn lot_supply(w: &mush_core::World) -> Option<mush_inventory::InventoryFamilies> {
    w.module_state::<MarketStore>()?.tables.inventory.clone()
}

/// **The presence gate, when the composition installed one** (doc 18 §4.3, cut IC7).
/// `None` is the pre-IC7 composition: every venue admitted, no defaults. The tacsim
/// implementation is IC8's.
#[must_use]
pub fn venue_gate(w: &mush_core::World) -> Option<Arc<dyn VenueGate>> {
    w.module_state::<MarketStore>()?.tables.venue_gate.clone()
}

/// **The capacity hook, when the composition installed one** (doc 18 §4.3, cut IC7).
/// `None` is no capacity rule. The tacsim implementation is IC8's.
#[must_use]
pub(crate) fn capacity_provider(w: &mush_core::World) -> Option<Arc<dyn CapacityProvider>> {
    w.module_state::<MarketStore>()?.tables.capacity.clone()
}

/// **The proceeds policy, when the composition installed one** (doc 18 §4.1, cut IC10).
/// `None` is no cuts and the market's own sale line.
#[must_use]
pub(crate) fn proceeds_policy(w: &mush_core::World) -> Option<Arc<dyn ProceedsPolicy>> {
    w.module_state::<MarketStore>()?.tables.proceeds.clone()
}

/// **The draw-receipt family's handle, when a lot supply is composed** (doc 18 cut IC7
/// item 6) — public so a reader holding the engine's typed state (a readiness report,
/// a test) can point-read one contract's receipt; it is a read capability only, since
/// every write needs an `ActionTxn` the market's own action prepares.
#[must_use]
pub fn draw_receipts(w: &mush_core::World) -> Option<DrawReceipts> {
    w.module_state::<MarketStore>()?.tables.draws.clone()
}

/// Ask the frozen sources for one pack. `None` when no source owns it.
pub(crate) fn pack_entries(ctx: &Ctx<'_>, pack: &str) -> Option<Result<Vec<CatalogEntry>, String>> {
    let sources = ctx.w.module_state::<MarketStore>()?.tables.catalogs.clone();
    sources.iter().find_map(|s| s.pack(pack))
}

/// Ask the frozen engines to resolve one availability test. `None` when nothing on
/// this server can answer, which is a **refusal** rather than a pass (B7, fail
/// closed).
pub(crate) fn resolve_availability(
    ctx: &mut Ctx<'_>,
    rule: &AvailabilityRule,
    catalog: &CatalogRef,
) -> Option<AvailabilityOutcome> {
    let providers = ctx
        .w
        .module_state::<MarketStore>()?
        .tables
        .availability
        .clone();
    let buyer = ctx.actor;
    for p in &providers {
        if let Some(o) = p.resolve(ctx, rule, catalog, buyer) {
            return Some(o);
        }
    }
    None
}

/// Hand the goods over after a settlement. `Representation::Asset` needs no provider:
/// its title moved as an ordinary E4b2 deliverable inside the settlement itself.
///
/// **`Representation::Lot` needs none either** (doc 18 cut IC0). Its matter leg is the
/// market's own `market_lot` action, staged at activation by `trade::accept` on the
/// same serialization point the money leg is in force at (E7r2 P1), so by the time the
/// contract settles the buyer's endpoint already holds the lot. Before IC0 this walked
/// the delivery table for a `'lot'` purchase, found no provider, and paged
/// `SettlementRefused` on **every** settled lot sale — a false alarm that said "the
/// buyer paid and holds nothing" about a buyer who held exactly what they paid for.
///
/// A refused delivery **reaches staff through the economy's own ladder** (independent
/// verify fold, M1): the money has already moved and the quote is written with
/// `settled: Some(..)`, so the trade tape counts a trade that delivered nothing, and a
/// bare `tracing::error!` left the paid-and-undelivered purchase in a log line nobody
/// is paged by. It opens a level-2 record on the **contract**, which is the ladder's own
/// per-subject dedupe key, with the filled-in commands a staffer runs.
pub(crate) fn deliver(ctx: &mut Ctx<'_>, q: &Quote, contract: ContractId) {
    let Some(representation) = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.entry_of(&q.catalog).map(|e| e.representation))
    else {
        undelivered(
            ctx,
            q,
            contract,
            "the catalogue entry it names is gone".to_string(),
        );
        return;
    };
    if settles_inside_the_contract(representation) {
        return;
    }
    let order = delivery_order(q, representation, contract);
    let providers = ctx
        .w
        .module_state::<MarketStore>()
        .map(|s| s.tables.delivery.clone())
        .unwrap_or_default();
    for p in &providers {
        match p.deliver(ctx, &order) {
            Some(Ok(())) => return,
            Some(Err(why)) => {
                undelivered(ctx, q, contract, why);
                return;
            }
            None => {}
        }
    }
    undelivered(
        ctx,
        q,
        contract,
        format!(
            "nothing on this server delivers a '{}' purchase",
            representation.label()
        ),
    );
}

/// The order one settled quote asks a provider for.
fn delivery_order(
    q: &Quote,
    representation: Representation,
    contract: ContractId,
) -> DeliveryOrder {
    DeliveryOrder {
        representation,
        catalog: q.catalog.clone(),
        buyer: q.buyer.clone(),
        seller: q.seller.clone(),
        quantity: q.quantity,
        contract,
    }
}

/// **The representations whose goods leg is the contract's own**, so neither the
/// delivery pre-check nor the post-settlement hand-over has a provider to ask:
///
/// - `Asset`: the title moves inside the settlement as an ordinary E4b2 deliverable,
///   and the contract's own `TitleHeld` condition is the pre-check;
/// - `Lot`: the matter moves through the market's own `market_lot` action, staged at
///   activation, and `trade::accept`'s own endpoint checks are the pre-check (doc 18
///   cut IC0).
///
/// One predicate for both callers, so `deliver` and `cannot_deliver` can never disagree
/// about which purchases the table is asked about.
#[must_use]
pub(crate) fn settles_inside_the_contract(representation: Representation) -> bool {
    matches!(representation, Representation::Asset | Representation::Lot)
}

/// **Ask the frozen table whether this purchase could be delivered at all**, before the
/// money moves (independent verify fold, M1). `Some(why)` refuses the accept.
///
/// `Representation::Asset` and `Representation::Lot` are never asked
/// ([`settles_inside_the_contract`]): their goods legs are the contract's and the lot
/// action's own, with their own pre-checks.
pub(crate) fn cannot_deliver(
    ctx: &Ctx<'_>,
    q: &Quote,
    representation: Representation,
) -> Option<String> {
    if settles_inside_the_contract(representation) {
        return None;
    }
    // The contract does not exist yet; the pre-check names none, and no provider may
    // read one from a `&Ctx` anyway.
    let order = delivery_order(q, representation, ContractId(0));
    let providers = ctx
        .w
        .module_state::<MarketStore>()
        .map(|s| s.tables.delivery.clone())
        .unwrap_or_default();
    for p in &providers {
        match p.can_deliver(ctx, &order) {
            Some(Ok(())) => return None,
            Some(Err(why)) => return Some(why),
            None => {}
        }
    }
    None
}

/// One paid-and-undelivered purchase: loud in the log **and** on the staff ladder.
fn undelivered(ctx: &mut Ctx<'_>, q: &Quote, contract: ContractId, why: String) {
    tracing::error!(
        contract = contract.0,
        quote = q.id.0,
        why,
        "market: a settled purchase could not be delivered; the buyer paid and holds nothing"
    );
    mush_economy::open_escalation(
        ctx,
        mush_economy::EscalationKind::SettlementRefused,
        mush_economy::EscalationSubject::Contract(contract),
        format!(
            "market quote #{} settled and its goods were not handed over ({why}). The money has \
             moved; the buyer holds nothing.",
            q.id
        ),
        vec![
            format!("@contract/show #{contract}"),
            format!(
                "@market/catalog show {}:{}",
                q.catalog.ruleset, q.catalog.code
            ),
        ],
    );
}
