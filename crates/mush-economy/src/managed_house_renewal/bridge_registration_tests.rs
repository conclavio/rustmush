//! The House family's registration scenarios: every one of
//! `crate::renewal_twin::bridge_registration_tests` run for the House issuer, whose renewals
//! stand on the House filing and the House purchases.
use crate::EconomyModule;
use crate::renewal_twin::House;
use crate::renewal_twin::bridge_registration_tests::{self as twin, Composition};
use mush_core::mechanics::DefinitionRef;

impl Composition for House {
    const BIRTH: &'static [(&'static str, u32)] = &[
        ("purchase_house_mind", 1),
        ("settle_house_mind", 1),
        ("refund_house_mind", 1),
    ];
    const BIRTH_FILING: Option<&'static str> = Some("house_initial_filing");

    fn with_birth(
        economy: EconomyModule,
        birth: &[DefinitionRef],
        filing: Option<&DefinitionRef>,
    ) -> Result<EconomyModule, Box<dyn std::error::Error>> {
        let [fund, grant, refund] = birth else {
            return Err("three House purchase actions".into());
        };
        Ok(economy
            .with_house_initial_filing(filing.ok_or("House filing action")?.clone())
            .with_house_mind_purchases([fund.clone(), grant.clone(), refund.clone()]))
    }
    fn with_renewals(
        economy: EconomyModule,
        manifest: DefinitionRef,
        participants: [DefinitionRef; 2],
        actions: [DefinitionRef; 5],
    ) -> EconomyModule {
        economy.with_house_mind_renewals(manifest, participants, actions)
    }
}

twin::instantiate!(crate::renewal_twin::House);
