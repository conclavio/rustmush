//! Explicit named launch composition. Config values never stand in for durable
//! initialization, actual publisher authorization, certificates or paid rights.
mod admission;
mod content;
mod deployment;
// 17g C1b: format 2's typed player-market policies; C1c composes them in the
// Active branch of `LaunchComposition::prepare`.
mod market;
pub(crate) mod registration;
mod venue;
pub use content::LaunchRecipe;
pub use deployment::{
    HardwareCoordinates, HouseDeployment, LaunchDeployment, LaunchMode, LaunchVenue,
    MAX_DEPLOYMENT_BYTES, MarketTerms, NamedHouses, PlayerMarket, ProcurementPolicy,
    ProcurementRole, RegistryAuthority,
};
pub(crate) use registration::Registration;

/// Source-free instructions. Every publication still uses its actual service.
pub const HELP: &str = "Named launch: Meridiani Field Series, Coprates Drill and Safe Conduct.\nConfigured publishers: @mind/launch <meridiani|coprates|safe_conduct>=<configured name> imports an owned draft; @mind/check <name>; @mind/save <name>=<notes>. Wait for the real save result, inspect @mind/info, then @registry/rate <mind number>/<saved revision>.\nAfter actual Registry certification, reserve a filing shell and use the offered house filing action with the exact certificate and scoped House treasury. Publish at the configured counter through house-list-one-seat; buyers use house-quote-one-seat, house-fund-one-seat and house-grant-one-seat. +act <verb> with <JSON object> resolves the current signed offer.\nFiling costs 100cr; a one-seat sealed Pin costs 400cr for 90 IC days plus 3 days Grace. A fresh renewal costs 400cr on its separately accepted offer. Coprates requires current Republic membership for a new purchase or renewal. Existing accepted terms retain their own dates; refund/cleanup does not require membership.\n@mind/doctrine shows the free Stickney teaching pages. Safe Conduct: after actual filing and current certification, the configured service operator uses @conduct/run <filed> <profile> <escort|transit>; status/retry/cancel use the actual Conduct job. Only completed verified trials qualify. Hull coverage remains a separately configured product and genuine host service; importing source supplies neither qualification nor coverage.";

use content::bad;
use mush_core::ModError;
use mush_economy::{
    Amount, CatalogRef, InstrumentId, managed_entitlement::GrossRecipientCut,
    registry_filing::InitialFilingPolicy,
};
use mush_roll::commerce::{filing::InitialFilingTerms, module::CommerceMode};
use std::sync::Arc;

pub struct LaunchComposition {
    commerce: crate::composition::mind_commerce::MindCommerce,
    hardware: crate::hardware::GovernedHardware,
    pub(crate) roster: crate::hardware::RosterPolicy,
    registration: Registration,
    venue: LaunchVenue,
}
impl LaunchComposition {
    pub fn prepare(
        layer: &crate::TacsimRuleset,
        deployment: LaunchDeployment,
    ) -> Result<Self, ModError> {
        deployment.validate()?;
        if !layer.config().enabled {
            return Err(bad("launch requires the actual enabled Tacsim layer"));
        }
        let filing = InitialFilingPolicy::new(
            deployment.registry_issuer.clone(),
            deployment.registry.account,
            Amount::from_minor(content::FILING_MINOR),
            CatalogRef {
                ruleset: crate::NS.into(),
                code: "mind.charter".into(),
            },
        )
        .map_err(bad)?;
        let (mode, origin) = match deployment.mode {
            LaunchMode::Initialize => (
                CommerceMode::Initialize,
                mush_industry::core_production::CoreOriginMode::Initialize,
            ),
            LaunchMode::Active => (
                CommerceMode::Active,
                mush_industry::core_production::CoreOriginMode::Active,
            ),
        };
        let hardware = crate::hardware::GovernedHardware::prepare(
            layer,
            origin,
            deployment.hardware.deployment(&deployment.registry_issuer),
            &filing,
        )?;
        let roster = hardware.roster_policy()?;
        // Capture the same finite hardware catalogue the actual layer will
        // register. Preserve the host's already verified packs (including the
        // Conduct carrier); this temporary configuration owns no live state.
        let mut prepared_layer = crate::TacsimRuleset::with_config(layer.config().clone());
        prepared_layer.loaded = layer.loaded.clone();
        let prepared_layer = hardware.configure_tacsim(prepared_layer)?;
        let admission = admission::CopratesAdmission::new(&deployment)?;
        // 17g C1c: format 2's typed player-market policies, built before the
        // deployment moves into its registration; `None` for format 1.
        let market = market::policies(&deployment)?;
        let mut commerce = crate::composition::mind_commerce::MindCommerce::prepare(
            &prepared_layer,
            mode,
            InitialFilingTerms::new(content::TERM_SECONDS, content::GRACE_SECONDS).map_err(bad)?,
            filing,
        )?;
        if mode == CommerceMode::Active {
            let registry = GrossRecipientCut {
                permille: content::LEVY_PERMILLE,
                recipient: Some(deployment.registry.clone()),
            };
            let venue = GrossRecipientCut {
                permille: content::VENUE_PERMILLE,
                recipient: Some(deployment.venue.recipient.clone()),
            };
            commerce = commerce
                .with_house_owner_terms(
                    InitialFilingTerms::new(
                        content::HOUSE_OWNER_TERM_SECONDS,
                        content::GRACE_SECONDS,
                    )
                    .map_err(bad)?,
                )?
                .with_house_filings()?
                .with_conduct_trials(&prepared_layer)?
                .with_registry_reconformance()?
                .with_minds_facts()?
                .with_house_minds_and_admission(
                    mush_market::mind_house::HousePolicy {
                        price: Amount::from_minor(content::HOUSE_MINOR),
                        instrument: InstrumentId::new(crate::economy::CREDIT)
                            .ok_or_else(|| bad("credit identity"))?,
                        registry: registry.clone(),
                        venue_name: deployment.venue.name.clone(),
                        venue: venue.clone(),
                        transferable: true,
                        term_seconds: content::TERM_SECONDS,
                        grace_seconds: content::GRACE_SECONDS,
                        quote_seconds: 300,
                        renewal_price: Amount::from_minor(content::HOUSE_MINOR),
                        renewal_term_seconds: content::TERM_SECONDS,
                        renewal_grace_seconds: content::GRACE_SECONDS,
                    },
                    Some(Arc::new(admission.clone())),
                )?
                .with_house_mind_renewals(mush_market::mind_house_renewal::RenewalPolicy {
                    registry,
                    venue: Some(venue),
                    offer_ic_seconds: content::OFFER_SECONDS,
                })?
                .with_house_mind_renewal_service()?
                // M8c F10 (the lead's ruling): the owner attestations (the
                // House attestation among them) are composed whenever the
                // House family is, market or not: B4 lifts C0's 365-day wall
                // for every world.
                .with_owner_attestations()?;
            // 17g C1c (§3 C1c, §6 risk 2): a format-2 deployment composes the
            // player market after the House chain, in the order C1a proved on
            // the production roster (C1 plan §2.2). The order is forced by the
            // builder's own guards:
            // - v2 source forms (`with_original_source_purchases`) before
            //   forks, because forks need source forms;
            // - forks, then the marked Registry, then dependent filing, all
            //   before entitlements;
            // - renewals after entitlements;
            // - pending charters after charter transfers;
            // - re-qualification after dependent owners;
            // - the House admission before paid resale (C1a: an admission
            //   given after paid resale refuses).
            // Two placements are conventions, not guards (C1 plan §2.3):
            // - revocation before pending charters and paid resale:
            //   `with_registry_revocation` would rebuild both anyway, so this
            //   order only means each is built once;
            // - paid resale after the personal purchases, so it takes their
            //   participants rather than falling back to the House ones.
            // The paid-resale writer inherits `CopratesAdmission` from
            // `with_house_minds_and_admission` above (C1a), so a resold
            // Coprates Pin's buyer needs Republic membership, as a new
            // buyer does. IC claims, hull cover and paid Follow are not
            // composed (17g §4). `Initialize` composes none of this, because
            // every owner here is Active-only, and a format-1 deployment
            // composes exactly the House chain it did before C1.
            if let Some(p) = market {
                commerce = commerce
                    .with_original_source_purchases(p.quote)?
                    .with_authored_forks()?
                    .with_marked_registry()?
                    .with_dependent_owner_filings()?
                    .with_mind_entitlements(p.entitlement)?
                    .with_compatible_updates()?
                    .with_derived_updates()?
                    .with_charter_transfers()?
                    .with_registry_revocation(p.revocation)?
                    .with_pending_charter_transfers(p.pending)?
                    .with_paid_mind_resales(p.clerk, p.resale_offer_seconds)?
                    .with_registry_recall(p.recall)?
                    .with_registry_inspection(p.inspection)?
                    .with_registry_fines(p.fines)?
                    .with_registry_principal_revocation(
                        prepared_layer.config().minds.revocation_ic_days,
                    )?
                    .with_mind_renewals(p.renewal)?
                    .with_mind_renewal_service()?
                    .with_author_withdrawal()?
                    .with_owner_parent_qualification()?;
            }
        }
        let venue = deployment.venue.clone();
        let registration = Registration::new(deployment, admission)?;
        Ok(Self {
            commerce,
            hardware,
            roster,
            registration,
            venue,
        })
    }
    pub fn commerce(&self) -> &crate::composition::mind_commerce::MindCommerce {
        &self.commerce
    }
    #[must_use]
    pub fn tactics(&self) -> mush_tactics::TacticsModule {
        self.roster.tactics()
    }
    #[must_use]
    pub fn configure_economy(
        &self,
        module: mush_economy::EconomyModule,
    ) -> mush_economy::EconomyModule {
        self.roster.economy(module)
    }
    #[must_use]
    pub fn configure_inventory(
        &self,
        module: mush_inventory::InventoryModule,
    ) -> mush_inventory::InventoryModule {
        self.roster.inventory(module)
    }
    #[must_use]
    pub fn configure_industry(
        &self,
        module: mush_industry::IndustryModule,
    ) -> mush_industry::IndustryModule {
        self.roster.industry(module)
    }
    #[must_use]
    pub fn configure_market(&self, module: mush_market::MarketModule) -> mush_market::MarketModule {
        module.with_venue_gate(Arc::new(venue::Gate(self.venue.clone())))
    }
    pub fn configure_tacsim(
        &self,
        layer: crate::TacsimRuleset,
    ) -> Result<crate::TacsimRuleset, ModError> {
        if layer.launch.is_some() {
            return Err(bad("launch already configured"));
        }
        let mut layer = self
            .commerce
            .configure_tacsim(self.hardware.configure_tacsim(layer)?)?;
        layer.launch = Some(Box::new(self.registration.clone()));
        Ok(layer)
    }
}

/// 17g C1c (C1 plan §4.2): the player market's line for bare `@mind/launch` on
/// a format-2 launch, rendered from the deployment's terms and doc 17's
/// constants. Read-only and source-free; it names no holder and no mind.
/// - "the charter owner gets the rest" and "pays the seller the rest" follow
///   17f §2.2's payees: a sale's remainder goes to the charter-title owner, a
///   resale's to its seller.
/// - "does not renew": personal renewal reads only entitlement-born licences
///   (`Birth::Entitlement`), and the v2 quote policy has no renewal terms.
/// - "a market-original-mind-list listing": the v2 List offer's own verb
///   since 17g C2a split the shared verbs one per kind (C2 plan §2.3), so
///   every market verb names one offer and the gateway's `+act` can select it.
/// - "Read the listings here with +minds …": 17g C2c's last sentence, which
///   replaced C2a's "Until the +minds commands exist, use +actions and
///   `+act <verb>` with `<JSON object>`." once the market's `+minds` commands and
///   `docs/help/personal-minds.md`'s table of them arrived (the C2c brief §3;
///   the pin is `PROVISIONAL_MARKET_LINE`).
pub(crate) fn market_help(market: &PlayerMarket) -> String {
    let terms = &market.terms;
    format!(
        "Player market: personal filing costs {filing}. A personal Pin listed at the counter sells for {price} for {term} IC days plus {grace} days Grace; the Registry takes {levy}%, any parent royalty is paid next, and the charter owner gets the rest. It renews for {renewal} on its own accepted offer, open {renewal_window}; a Pin bought from a market-original-mind-list listing does not renew. A licence resale offer stays open {resale_window} and pays the seller the rest; buying a resold House Pin needs the same standing as buying it new. A pending charter transfer settles within {settle_window}. Registry recall and revocation belong to the configured Registry operators. Read the listings here with +minds and your licences with +minds/holdings; help personal-minds lists every market command.",
        filing = credits(content::FILING_MINOR),
        price = credits(terms.price_minor),
        term = content::TERM_SECONDS / 86_400,
        grace = content::GRACE_SECONDS / 86_400,
        levy = content::LEVY_PERMILLE / 10,
        renewal = credits(terms.renewal_price_minor),
        renewal_window = window(terms.renewal_offer_ic_seconds, "IC "),
        resale_window = window(terms.resale_offer_seconds, ""),
        settle_window = window(terms.charter_settle_ic_seconds, "IC "),
    )
}

/// 17g C1c (C1 plan §1 item 4): God's read-only check that the runbook's
/// `@org/grant` lines took effect. Each configured operator, in ascending
/// order, is `#<n> ready` when `mush_org::authorize_in_world` allows it both
/// Registry capabilities in the configured scope now, and otherwise names what
/// it lacks. The authorizer weighs the grant's limit and validity window and
/// the org's and scope's status, so a limited, expired, not-yet-valid or
/// suspended grant, or a retired scope, reads as "lacks". It writes nothing
/// and reaches no state slot mutably.
pub(crate) fn registry_readiness(world: &mush_core::World, market: &PlayerMarket) -> String {
    let authority = &market.registry_authority;
    let allows = |actor: mush_core::ObjId, capability: &str| {
        mush_org::CapabilityId::new(capability).is_some_and(|capability| {
            mush_org::authorize_in_world(
                world,
                &mush_org::AuthRequest {
                    actor,
                    capability,
                    scope: authority.scope,
                    now: world.now(),
                    amount: None,
                },
            )
            .is_allow()
        })
    };
    let mut operators = authority.operators.clone();
    operators.sort_unstable();
    let items = operators
        .into_iter()
        .map(|operator| {
            // Doc 17h cut M8a: three capabilities (M8d: four); what is lacking is named in
            // this order, joined as prose (`lacks recall, revocation, inspection and fine`).
            let lacking: Vec<&str> = [
                (
                    mush_roll::commerce::registry_recall::REGISTRY_RECALL_CAPABILITY,
                    "recall",
                ),
                (
                    mush_roll::commerce::registry_revocation::REGISTRY_REVOCATION_CAPABILITY,
                    "revocation",
                ),
                (
                    mush_roll::commerce::registry_inspection::REGISTRY_INSPECTION_CAPABILITY,
                    "inspection",
                ),
                // Doc 17h cut M8d D3: the fine, the fourth.
                (
                    mush_roll::commerce::registry_inspection::REGISTRY_FINE_CAPABILITY,
                    "fine",
                ),
            ]
            .into_iter()
            .filter(|(capability, _)| !allows(operator, capability))
            .map(|(_, word)| word)
            .collect();
            let state = match lacking.as_slice() {
                [] => "ready".to_owned(),
                [one] => format!("lacks {one}"),
                [head @ .., last] => format!("lacks {} and {last}", head.join(", ")),
            };
            format!("#{} {state}", operator.0)
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("Registry operators: {items}.")
}

/// Whole credits when the minor amount divides by 100 (`400cr`), otherwise two
/// decimals (`400.50cr`). Decode bounds every market price to at least 1.
/// C1c's credit rendering (`400cr`, `0.21cr`). 17g C2c: the `+minds`
/// commands print every price with it.
pub(crate) fn credits(minor: i64) -> String {
    let (whole, cents) = (minor.div_euclid(100), minor.rem_euclid(100));
    if cents == 0 {
        format!("{whole}cr")
    } else {
        format!("{whole}.{cents:02}cr")
    }
}

/// 17g C2c: the personal term in whole IC days (doc 17's, not the
/// deployment's), the only `term=` `@mind/publish` accepts.
pub(crate) const fn personal_term_days() -> i64 {
    content::TERM_SECONDS / 86_400
}

/// A window in days when it divides by 86,400 (`1 IC day`, `3 IC days`), else
/// in hours when it divides by 3,600, else in seconds. `ic` is `"IC "` for an
/// IC-clock window and empty for a real-time one.
fn window(seconds: i64, ic: &str) -> String {
    let (count, unit) = if seconds % 86_400 == 0 {
        (seconds / 86_400, "day")
    } else if seconds % 3_600 == 0 {
        (seconds / 3_600, "hour")
    } else {
        (seconds, "second")
    };
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} {ic}{unit}{plural}")
}
