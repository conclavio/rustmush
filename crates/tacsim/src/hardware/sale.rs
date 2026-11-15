//! **`+core/sell`: a stored core is sold back to the fitting shop with its title** (doc 47 unit
//! CO.13; R-47-5, "non-destroyed parts removed go to storage, to be fitted again or sold").
//!
//! A governed core's title is a managed title: no generic contract may move it
//! (`mush_economy::managed_title::require_unmanaged_mutations`), so the market's custody-only
//! `+market/sell` cannot sell one (it refuses a managed core outright). This is the closed path,
//! on the Registry seizure's shape: **one act, one union.** The provider here reads the core in
//! the seller's own hands, at the fitting shop, prices it from the market's core catalogue at the
//! shop's buy-back rate, and binds two closed legs in the same coordinator union:
//!
//! * the economy's closed sale plan (`EconomyRequest::CoreSale`): the shop's integrator pays the
//!   price into the seller's own credit account and takes the core's title, audited and replayed
//!   at boot (`mush_economy::registry_filing::core_sale`);
//! * the kernel's closed sale image (`mush_core::core_sale`): the core leaves the seller's hands
//!   for the shop's completed-work container, its record kept in this crate's `core_sales`
//!   family.
//!
//! A boot pass pairs the two record sets one to one (`boot_audit`).
//!
//! **Who buys, and at what**: the governed hardware's integrator (the fitting shop's
//! organization, its factory scope and account), at [`BUY_BACK_PERMILLE`] of the catalogue's
//! baseline (`CoreClass::finished_minor`) unless the deployment names another rate. Composed only
//! where the hardware deployment names a buy-back (`HardwareDeployment::buy_back_permille`).
//!
//! **Refused**: a core that is fitted (it is in a hull), a core someone else holds, a core whose
//! title is not the seller's alone (another's, a claim on it, impounded), a destroyed core (the
//! refit that displaced it found it destroyed: CO.4's rule), and a seller not at the shop.

use std::sync::{Arc, Mutex};

use mush_core::action::{
    ActionError, ActionErrorCode, ActionProvider, AuthoritativeActionOutput, PrincipalId,
};
use mush_core::core_sale::{CoreSalePolicy, CoreSales, SaleFacts, SaleRequest, SoldCore};
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::participant::{ParticipantError, ParticipantHandle};
use mush_core::{
    ActionInvocation, ActionTxn, CmdError, Ctx, DefinitionRef, ModError, ModuleCtx, ObjId, ObjKind,
    PerceivedRef, PrepareCtx, Registrar, World,
};
use mush_economy::core_integration::ScopedCoreAccount;
use mush_economy::registry_filing::core_sale::{CoreSaleConfig, RegistryCoreSale};
use mush_economy::registry_filing::{EconomyRequest, InitialFilingParticipant};
use mush_economy::{AccountId, Amount, CatalogRef, EconomyStore};
use mush_industry::core_production::refit::CoreRefitFactoryConfig;
use serde::Deserialize;
use serde_json::json;

/// The verb.
pub const SALE_VERB: &str = "+core";
/// The sale action's id (the kernel's and the economy's closed legs name it).
pub const SALE_ACTION: &str = mush_core::core_sale::ACTION;
/// The sale's outcome id.
pub const SALE_OUTCOME: &str = "stored_core_sold";
/// **The fitting shop's buy-back rate**, permille of a core's catalogue baseline: half its
/// finished price. Provisional, like every economy number (the owner's rule): tuned in play.
pub const BUY_BACK_PERMILLE: u32 = 500;
/// The market pack the core catalogue entries load under.
pub const CORE_PACK: &str = "tacsim.cores";
/// The three governed cores' catalogue codes, Class I, II, III (ruleset `tacsim`): the codes the
/// core integration titles each core with (`CorePaymentPolicy::catalogs`).
pub const CORE_CATALOG_CODES: [&str; 3] = [
    "tacsim.core.doctrine",
    "tacsim.core.line",
    "tacsim.core.command",
];

/// `+core`'s help.
pub const SALE_HELP: &str = concat!(
    "+core/sell <core>  --  Quotes what the fitting shop pays for a governed core in your hands:\n",
    "a core a refit took out of a hull, or one never fitted. Say it at the shop.\n",
    "+core/sell <core>=go  sells it: the shop pays the quote into your own credit account and\n",
    "takes the core and its title. A fitted core, a destroyed core and a core you do not hold\n",
    "are not bought."
);
/// The grammar.
pub const SALE_USAGE: &str = "Usage: +core/sell <core>[=go]";
/// No buy-back on this world.
pub const SALE_OFF: &str = "No fitting shop buys cores on this world.";
/// Not at the shop.
pub const SALE_NOT_HERE: &str = "The fitting shop buys cores only across its own counter.";
/// Not a governed core.
pub const SALE_NOT_CORE: &str = "That is not a governed core.";
/// Fitted.
pub const SALE_FITTED: &str = "That core is fitted to a hull; a refit takes it out first.";
/// Not in the seller's hands.
pub const SALE_NOT_HELD: &str = "You are not holding that core.";
/// Not the seller's title alone.
pub const SALE_NOT_YOURS: &str =
    "That core's title is not yours alone, so it is not yours to sell.";
/// Destroyed.
pub const SALE_DESTROYED: &str = "The fitting shop does not buy a destroyed core.";
/// No catalogue price.
pub const SALE_NO_PRICE: &str = "The fitting shop has no price for that core.";
/// The shop's till cannot cover the price.
pub const SALE_SHOP_SHORT: &str = "The fitting shop cannot pay for that core just now.";
/// No account to pay into.
pub const SALE_NO_ACCOUNT: &str = "You hold no credit account for the shop to pay into.";
/// The quote.
pub const SALE_QUOTE: &str = "The fitting shop pays {price} for core #{core} and takes its title. \
     +core/sell #{core}=go sells it.";
/// Submitted.
pub const SALE_SELLING: &str = "You hand core #{core} across the counter.";
/// Done (the provider's receipt).
pub const SALE_DONE: &str =
    "The fitting shop takes core #{core} and its title, and pays you {price}.";
/// Busy.
pub const SALE_BUSY: &str = "You are already doing something; try again in a moment.";

/// **What the sale composes with** (the governed hardware's deployment): the rate, who buys, the
/// shop the seller stands in, the container the core goes into, the cores' catalogue codes, and
/// the refit (whose history says whether a stored core is destroyed).
#[derive(Clone)]
pub(crate) struct SaleTerms {
    pub(crate) permille: u32,
    pub(crate) vendor: ScopedCoreAccount,
    pub(crate) factory: ObjId,
    pub(crate) shop: ObjId,
    pub(crate) catalogs: [CatalogRef; 3],
    pub(crate) refit: CoreRefitFactoryConfig,
}
impl std::fmt::Debug for SaleTerms {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SaleTerms")
            .field("permille", &self.permille)
            .field("vendor", &self.vendor)
            .field("factory", &self.factory)
            .field("shop", &self.shop)
            .finish_non_exhaustive()
    }
}

fn bad(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}

/// The sale's action descriptor (tacsim, version 1), as the crate's action validator holds it.
///
/// # Errors
/// A descriptor that does not build.
pub(crate) fn action_descriptor() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new("action").map_err(bad)?,
        DefinitionId::new(SALE_ACTION).map_err(bad)?,
        1,
        json!({"format":1,"protocol":"stored_core_sale_v1","subject":"self_actor","targets":0,
            "buyer":"fitting_shop_integrator","settlement":"native_money_title_and_object_same_union"}),
    )
    .map(DefinitionDescriptor::as_current)
    .map_err(bad)
}
/// The sale's outcome descriptor.
///
/// # Errors
/// A descriptor that does not build.
pub(crate) fn outcome_descriptor() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new("outcome").map_err(bad)?,
        DefinitionId::new(SALE_OUTCOME).map_err(bad)?,
        1,
        json!({"format":1}),
    )
    .map(DefinitionDescriptor::as_current)
    .map_err(bad)
}
/// The registered action reference, for the economy's closed leg.
///
/// # Errors
/// A descriptor that does not register.
pub(crate) fn action_ref() -> Result<DefinitionRef, ModError> {
    DefinitionRegistryBuilder::new()
        .register(OwnerId::new(crate::NS).map_err(bad)?, action_descriptor()?)
        .map_err(bad)
}
impl SaleTerms {
    /// The economy's closed leg: the action and the vendor.
    ///
    /// # Errors
    /// A descriptor that does not register.
    pub(crate) fn economy_config(&self) -> Result<CoreSaleConfig, ModError> {
        Ok(CoreSaleConfig {
            action: action_ref()?,
            vendor: self.vendor.clone(),
        })
    }
}

/// **The market's core catalogue entries** (doc 47 CO.13): the three governed cores, each at its
/// finished price (`CoreClass::finished_minor`), an `asset` (one titled object, not a lot). A
/// reference price, never a charge: the shop's buy-back reads it.
///
/// # Errors
/// An entry that does not decode.
pub fn core_catalog_entries() -> Result<Vec<mush_market::CatalogEntry>, String> {
    use mush_economy::core_integration::CoreClass;
    [
        (CoreClass::I, "Class I governed core"),
        (CoreClass::II, "Class II governed core"),
        (CoreClass::III, "Class III governed core"),
    ]
    .into_iter()
    .map(|(class, name)| {
        let value = json!({
            "catalog": {"ruleset": crate::NS, "code": CORE_CATALOG_CODES[class.catalog_index()]},
            "pack": CORE_PACK,
            "definition_version": 1,
            "name": name,
            "baseline": class.finished_minor(),
            "legality": "legal",
            "representation": "asset",
            "stackable": false,
            "tags": ["governed_core"],
        });
        serde_json::from_value(value).map_err(|error| error.to_string())
    })
    .collect()
}

/// Why a sale is refused, in the seller's words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    NotHere,
    NotCore,
    Fitted,
    NotHeld,
    NotYours,
    Destroyed,
    NoPrice,
    ShopShort,
    NoAccount,
}
impl Refusal {
    pub(crate) const fn line(self) -> &'static str {
        match self {
            Self::NotHere => SALE_NOT_HERE,
            Self::NotCore => SALE_NOT_CORE,
            Self::Fitted => SALE_FITTED,
            Self::NotHeld => SALE_NOT_HELD,
            Self::NotYours => SALE_NOT_YOURS,
            Self::Destroyed => SALE_DESTROYED,
            Self::NoPrice => SALE_NO_PRICE,
            Self::ShopShort => SALE_SHOP_SHORT,
            Self::NoAccount => SALE_NO_ACCOUNT,
        }
    }
}

/// **What the shop pays, and from what read**: the price, the title revision read and the
/// seller's own credit account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Quote {
    pub(crate) price: Amount,
    pub(crate) title_revision: u64,
    pub(crate) seller_account: AccountId,
}

/// **The one read** the command's quote, the provider and the kernel policy share.
pub(crate) fn quote(
    world: &World,
    terms: &SaleTerms,
    seller: ObjId,
    core: ObjId,
) -> Result<Quote, Refusal> {
    let at = world.get(seller).and_then(|o| o.location);
    if at != Some(terms.factory) {
        return Err(Refusal::NotHere);
    }
    let economy = world
        .module_state::<EconomyStore>()
        .ok_or(Refusal::NotCore)?;
    let object = world.get(core).ok_or(Refusal::NotCore)?;
    if object.kind != ObjKind::Thing || !economy.is_managed_core(core) {
        return Err(Refusal::NotCore);
    }
    let place = object.location.ok_or(Refusal::NotHeld)?;
    if place != seller {
        let fitted = crate::minds::registry_inspection::seizure::fitted_core(world, place);
        return Err(if fitted == Some(core) {
            Refusal::Fitted
        } else {
            Refusal::NotHeld
        });
    }
    let title = economy.snapshot().asset(core).ok_or(Refusal::NotYours)?;
    if title.title_owner != mush_org::PrincipalId::Object(seller)
        || title.scope.is_some()
        || !title.encumbrances.is_empty()
        || title.frozen
        || title.stolen.is_some()
        || economy.core_impounded(core)
    {
        return Err(Refusal::NotYours);
    }
    match terms.refit.stored_core_destroyed(world, core) {
        Some(false) => {}
        Some(true) | None => return Err(Refusal::Destroyed),
    }
    let catalog = title.catalog.as_ref().ok_or(Refusal::NoPrice)?;
    if !terms.catalogs.contains(catalog) {
        return Err(Refusal::NoPrice);
    }
    let baseline = world
        .module_state::<mush_market::MarketStore>()
        .and_then(|market| market.entry_of(catalog))
        .and_then(|entry| entry.baseline)
        .filter(|b| b.minor() > 0)
        .ok_or(Refusal::NoPrice)?;
    let price = baseline
        .minor()
        .checked_mul(i64::from(terms.permille))
        .map(|minor| minor / 1_000)
        .filter(|minor| *minor > 0)
        .ok_or(Refusal::NoPrice)?;
    let till = economy
        .snapshot()
        .account(terms.vendor.account)
        .ok_or(Refusal::ShopShort)?;
    if till
        .posted_balance
        .minor()
        .checked_sub(till.held_amount.minor())
        .is_none_or(|free| free < price)
    {
        return Err(Refusal::ShopShort);
    }
    let seller_account = crate::port::credit::own_account(world, seller)
        .ok_or(Refusal::NoAccount)?
        .id;
    Ok(Quote {
        price: Amount::from_minor(price),
        title_revision: title.revision,
        seller_account,
    })
}

/// **The kernel's trusted sale policy**: the same read, then the shop's container.
pub(crate) struct SalePolicy {
    terms: Arc<SaleTerms>,
}
impl CoreSalePolicy for SalePolicy {
    fn read(
        &self,
        world: &World,
        seller: ObjId,
        request: &SaleRequest,
    ) -> Result<SaleFacts, ParticipantError> {
        quote(world, &self.terms, seller, request.core())
            .map_err(|_| ParticipantError::refused("The core sale is unavailable or changed."))?;
        Ok(SaleFacts::new(request.core(), seller, self.terms.shop))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Parameters {
    core: u32,
}

/// **The provider**: read, bind the economy's sale, bind the kernel's image, answer the receipt.
struct Provider {
    action: DefinitionRef,
    outcome: DefinitionRef,
    economy: ParticipantHandle<InitialFilingParticipant>,
    sales: CoreSales<SalePolicy>,
    terms: Arc<SaleTerms>,
    refusal: ActionError,
}
impl Provider {
    fn request(&self, inv: &ActionInvocation) -> Result<ObjId, ActionError> {
        if inv.principal() != &(PrincipalId::Object { id: inv.actor() })
            || inv.subject() != &PerceivedRef::SelfActor
            || !inv.targets().is_empty()
        {
            return Err(self.refusal.clone());
        }
        let p: Parameters =
            serde_json::from_value(inv.parameters().clone()).map_err(|_| self.refusal.clone())?;
        if p.core == 0 {
            return Err(self.refusal.clone());
        }
        Ok(ObjId(p.core))
    }
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, inv: &ActionInvocation) -> Result<(), ActionError> {
        self.request(inv).map(|_| ())
    }

    fn prepare_authoritative_with_context(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let fail = || self.refusal.clone();
        let core = self.request(inv)?;
        let seller = inv.actor();
        let quoted = {
            let world = ctx.world().map_err(|_| fail())?;
            quote(world, &self.terms, seller, core).map_err(|_| fail())?
        };
        let native = ctx
            .prepare_owner(
                &self.economy,
                EconomyRequest::CoreSale(RegistryCoreSale {
                    core,
                    title_revision: quoted.title_revision,
                    seller_account: quoted.seller_account,
                    price: quoted.price,
                }),
            )
            .map_err(|_| fail())?
            .core_sale()
            .map_err(|_| fail())?;
        if native.action() != &self.action || native.commit_token() != ctx.commit_token() {
            return Err(fail());
        }
        let sold = ctx
            .sell_stored_core(&self.sales, txn, SaleRequest::new(core))
            .map_err(|_| fail())?;
        if sold.commit() != ctx.commit_token() || sold.core() != core || sold.seller() != seller {
            return Err(fail());
        }
        let summary = SALE_DONE
            .replace("{core}", &core.0.to_string())
            .replace("{price}", &credits(quoted.price.minor()));
        let result = mush_core::mechanics::RulesValue::new(
            self.outcome.clone(),
            json!({
                "core": core.0,
                "seller": seller.0,
                "price_minor": quoted.price.minor(),
                "summary": summary,
            }),
        )
        .map_err(|_| fail())?;
        Ok(AuthoritativeActionOutput::new(result)
            .with_definitions(vec![self.action.clone(), self.outcome.clone()])
            .with_eligible_witnesses(vec![inv.principal().clone()]))
    }

    /// **Offered to a character standing in the fitting shop**: which core is the parameter;
    /// whether that core may be sold is the prepare's read.
    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<mush_core::ProviderOfferDraft>, ActionError> {
        let here = engine
            .world
            .get(query.actor())
            .is_some_and(|o| o.kind == ObjKind::Player && o.location == Some(self.terms.factory));
        if action != &self.action
            || query.truth_subject() != &(mush_core::SubjectKey::Object { id: query.actor() })
            || query.projected_subject() != &PerceivedRef::SelfActor
            || !here
        {
            return Ok(None);
        }
        let fail = || self.refusal.clone();
        let definitions = engine.registry.definitions().ok_or_else(fail)?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(mush_core::RevisionSet::default())
            .map_err(|_| fail())?;
        for r in [&self.action, &self.outcome] {
            guards
                .add_definition(r.clone(), definitions)
                .map_err(|_| fail())?;
        }
        let parameters = vec![mush_core::ProjectedParameter {
            id: mush_core::mechanics::CanonicalId::new("core").map_err(|_| fail())?,
            required: true,
            kind: mush_core::ProjectedParameterKind::Integer {
                minimum: 1,
                maximum: i64::from(u32::MAX),
            },
        }];
        Ok(Some(mush_core::ProviderOfferDraft {
            fixed_parameters: json!({}),
            parameters: mush_core::ProjectedParameterSchema::new(parameters).map_err(|_| fail())?,
            targets: mush_core::ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: mush_core::ActionPresentation::new(
                "tacsim-core-sell",
                "Sell a stored core",
                "Sell a governed core in your own hands to the fitting shop, with its title.",
            )
            .map_err(|_| fail())?,
            guards: guards.finish().map_err(|_| fail())?,
        }))
    }

    /// The receipt the prepare rendered, for the seller only.
    fn project_receipt(
        &self,
        action: &DefinitionRef,
        outcome: &mush_core::mechanics::RulesValue,
        viewer: &PrincipalId,
    ) -> Result<Option<mush_proto::ProjectedActionResult>, ActionError> {
        if action != &self.action || outcome.schema() != &self.outcome {
            return Err(self.refusal.clone());
        }
        let data = outcome.data();
        let seller = data
            .get("seller")
            .and_then(serde_json::Value::as_u64)
            .and_then(|id| u32::try_from(id).ok())
            .map(ObjId)
            .ok_or_else(|| self.refusal.clone())?;
        if viewer != &(PrincipalId::Object { id: seller }) {
            return Err(self.refusal.clone());
        }
        let summary = data
            .get("summary")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| self.refusal.clone())?;
        mush_proto::ProjectedActionResult::new(SALE_ACTION, summary, data.clone())
            .map(Some)
            .map_err(|_| self.refusal.clone())
    }
}

/// Credits, centicredit-exact, for a line no `Ctx` renders (the provider's receipt).
fn credits(minor: i64) -> String {
    let sign = if minor < 0 { "-" } else { "" };
    let minor = minor.unsigned_abs();
    format!("{sign}{}.{:02}cr", minor / 100, minor % 100)
}

/// **The slot** this crate registers when the sale composes: the action (the command's
/// submission), the terms (its quote), the kernel's service (the boot audit) and the command's
/// idempotency attempts (RAM only).
#[derive(Clone)]
pub struct SaleHost {
    action: DefinitionRef,
    terms: Arc<SaleTerms>,
    sales: CoreSales<SalePolicy>,
    attempts: Arc<Mutex<(i64, u64)>>,
}
impl SaleHost {
    /// The slot in `world`, owner-checked.
    #[must_use]
    pub fn of(world: &World) -> Option<&SaleHost> {
        if world
            .module_state_owner(std::any::TypeId::of::<SaleHost>())
            .map(|owner| owner.as_str())
            != Some(crate::NS)
        {
            return None;
        }
        world.module_state::<SaleHost>()
    }
}

/// **Register the provider, the kernel's service and the slot** over the registered `action` and
/// `outcome`.
///
/// # Errors
/// A registrar that is not this crate's, a reference that is not the sale's, or a refused
/// registration.
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    terms: &SaleTerms,
    action: DefinitionRef,
    outcome: DefinitionRef,
) -> Result<(), ModError> {
    if reg.module() != crate::NS || action != action_ref()? {
        return Err(bad("core sale registration differs from its composition"));
    }
    let terms = Arc::new(terms.clone());
    let sales = CoreSales::register(
        reg,
        action.clone(),
        SalePolicy {
            terms: Arc::clone(&terms),
        },
    )?;
    let economy = reg.request_owner_participant::<InitialFilingParticipant>(action.clone())?;
    reg.action_provider(
        action.clone(),
        Provider {
            action: action.clone(),
            outcome,
            economy,
            sales: sales.clone(),
            terms: Arc::clone(&terms),
            refusal: ActionError::new(
                ActionErrorCode::ProviderRejected,
                false,
                "The core sale is unavailable or changed.",
            )
            .map_err(bad)?,
        },
    )?;
    reg.state(SaleHost {
        action,
        terms,
        sales,
        attempts: Arc::new(Mutex::new((i64::MIN, 0))),
    });
    Ok(())
}

/// **The boot pass**: the kernel's sale records and the economy's replayed sale receipts pair one
/// to one, by operation, core and seller. A roster without the slot audits nothing.
///
/// # Errors
/// A record either side holds that the other does not, or one whose core or seller differs.
pub(crate) fn boot_audit(ctx: &ModuleCtx<'_>) -> Result<(), ModError> {
    let Some(host) = ctx.state::<SaleHost>() else {
        return Ok(());
    };
    let mut sold = std::collections::BTreeMap::new();
    host.sales
        .audit(ctx, |row: SoldCore| {
            sold.insert(row.commit(), row);
            Ok(())
        })
        .map_err(|_| bad("tacsim: the core sale records do not read"))?;
    let economy = ctx
        .w
        .module_state::<EconomyStore>()
        .ok_or_else(|| bad("tacsim: the core sale needs the economy"))?;
    let receipts = economy
        .core_sale_receipts()
        .map_err(|_| bad("tacsim: the economy's core sale receipts are not complete"))?;
    let mut paired = 0usize;
    for receipt in receipts {
        let row = sold
            .get(&receipt.commit)
            .ok_or_else(|| bad("tacsim: a core sale receipt has no sale record"))?;
        if row.core() != receipt.core || row.seller() != receipt.seller {
            return Err(bad("tacsim: a core sale record differs from its receipt"));
        }
        paired += 1;
    }
    if paired != sold.len() {
        return Err(bad("tacsim: a core sale record has no economy receipt"));
    }
    Ok(())
}

/// **`+core/sell <core>[=go]`**: the quote, or the sale. Every refusal is one sentence.
///
/// # Errors
/// The refusal line.
pub(crate) fn sale_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if !ctx.switches.has("sell") {
        return Err(CmdError::msg(SALE_USAGE));
    }
    let raw = ctx.args.raw().trim().to_owned();
    let (target, go) = match raw.split_once('=') {
        Some((target, word)) if word.trim().eq_ignore_ascii_case("go") => (target.trim(), true),
        Some(_) => return Err(CmdError::msg(SALE_USAGE)),
        None => (raw.as_str(), false),
    };
    let text = target.strip_prefix('#').unwrap_or(target);
    let core = text
        .parse::<u32>()
        .ok()
        .filter(|n| *n != 0)
        .map(ObjId)
        .ok_or_else(|| CmdError::msg(SALE_USAGE))?;
    let actor = ctx.actor;
    let (action, quoted, key) = {
        let w: &World = &*ctx.w;
        let host = SaleHost::of(w).ok_or_else(|| CmdError::msg(SALE_OFF))?;
        let quoted = quote(w, &host.terms, actor, core).map_err(|why| CmdError::msg(why.line()))?;
        if !go {
            (None, quoted, None)
        } else {
            if ctx.authoritative_pending(actor) || ctx.authoritative_queue_full() {
                return Err(CmdError::msg(SALE_BUSY));
            }
            let key = crate::minds::market_commands::next_key(
                &host.attempts,
                "core_sale",
                &core.0.to_string(),
                w.world_ic_now().at,
            )?;
            (Some(host.action.clone()), quoted, Some(key))
        }
    };
    let price = crate::port::coop::money_minor(ctx, quoted.price.minor());
    match (action, key) {
        (Some(action), Some(key)) => {
            ctx.submit_authoritative(
                action,
                PerceivedRef::SelfActor,
                Vec::new(),
                json!({"core": core.0}),
                key,
            );
            crate::port::coop::say(ctx, SALE_SELLING.replace("{core}", &core.0.to_string()));
        }
        _ => crate::port::coop::say(
            ctx,
            SALE_QUOTE
                .replace("{price}", &price)
                .replace("{core}", &core.0.to_string()),
        ),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The help and every line hold the 120-column floor; the entries decode at the class prices.
    #[test]
    fn co13_the_lines_fit_and_the_entries_are_the_class_prices() {
        for line in SALE_HELP.lines() {
            assert!(line.chars().count() <= 120, "{line}");
        }
        let entries = core_catalog_entries().expect("the entries decode");
        assert_eq!(entries.len(), 3);
        for (entry, class) in entries.iter().zip([
            mush_economy::core_integration::CoreClass::I,
            mush_economy::core_integration::CoreClass::II,
            mush_economy::core_integration::CoreClass::III,
        ]) {
            assert_eq!(
                entry.baseline.map(|b| b.minor()),
                Some(class.finished_minor())
            );
            assert_eq!(entry.pack.as_str(), CORE_PACK);
        }
        assert_eq!(credits(400_000), "4000.00cr");
    }
}
