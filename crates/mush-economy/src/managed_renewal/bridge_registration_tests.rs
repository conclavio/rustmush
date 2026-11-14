//! The personal family's registration scenarios: every one of
//! `crate::renewal_twin::bridge_registration_tests` run for the personal issuer, whose
//! renewals stand on the entitlement purchases.
use crate::EconomyModule;
use crate::renewal_twin::Personal;
use crate::renewal_twin::bridge_registration_tests::{self as twin, Composition};
use mush_core::mechanics::DefinitionRef;

impl Composition for Personal {
    const BIRTH: &'static [(&'static str, u32)] = &[
        ("purchase_original_mind", 3),
        ("settle_original_mind", 3),
        ("refund_original_mind", 3),
        ("purchase_derived_mind", 1),
        ("settle_derived_mind", 1),
        ("refund_derived_mind", 1),
    ];
    const BIRTH_FILING: Option<&'static str> = None;

    fn with_birth(
        economy: EconomyModule,
        birth: &[DefinitionRef],
        _: Option<&DefinitionRef>,
    ) -> Result<EconomyModule, Box<dyn std::error::Error>> {
        let [o1, o2, o3, d1, d2, d3] = birth else {
            return Err("six entitlement actions".into());
        };
        Ok(economy.with_mind_entitlement_purchases(
            [o1.clone(), o2.clone(), o3.clone()],
            [d1.clone(), d2.clone(), d3.clone()],
        ))
    }
    fn with_renewals(
        economy: EconomyModule,
        manifest: DefinitionRef,
        participants: [DefinitionRef; 2],
        actions: [DefinitionRef; 5],
    ) -> EconomyModule {
        economy.with_mind_renewals(manifest, participants, actions)
    }
}

twin::instantiate!(crate::renewal_twin::Personal);
