//! Explicit named launch composition. Config values never stand in for durable
//! initialization, actual publisher authorization, certificates or paid rights.
mod admission;
mod content;
mod deployment;
pub(crate) mod registration;
mod venue;
pub use content::LaunchRecipe;
pub use deployment::{
    HardwareCoordinates, HouseDeployment, LaunchDeployment, LaunchMode, LaunchVenue,
    MAX_DEPLOYMENT_BYTES, NamedHouses,
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
                .with_house_mind_renewal_service()?;
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
    pub(crate) fn configure_market(
        &self,
        module: mush_market::MarketModule,
    ) -> mush_market::MarketModule {
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
        layer.launch = Some(self.registration.clone());
        Ok(layer)
    }
}
