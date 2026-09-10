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
}

impl std::fmt::Debug for MarketModule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MarketModule")
            .field("catalog_sources", &self.catalogs.len())
            .field("availability_providers", &self.availability.len())
            .field("delivery_providers", &self.delivery.len())
            .finish()
    }
}

impl MarketModule {
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
}

impl std::fmt::Debug for MarketTables {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MarketTables").finish_non_exhaustive()
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
        reg.state(MarketStore {
            tables: MarketTables {
                catalogs: self.catalogs.clone(),
                availability: self.availability.clone(),
                delivery: self.delivery.clone(),
                issuer: self.issuer.clone(),
                inventory: inventory.clone(),
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
        if let Some(families) = &inventory {
            crate::lot::register(reg, families)?;
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
    if matches!(representation, Representation::Asset) {
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

/// **Ask the frozen table whether this purchase could be delivered at all**, before the
/// money moves (independent verify fold, M1). `Some(why)` refuses the accept.
///
/// `Representation::Asset` is never asked: its title moves inside the settlement as an
/// ordinary E4b2 deliverable, and the contract's own `TitleHeld` condition is the
/// pre-check.
pub(crate) fn cannot_deliver(
    ctx: &Ctx<'_>,
    q: &Quote,
    representation: Representation,
) -> Option<String> {
    if matches!(representation, Representation::Asset) {
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
