//! Exact real composition only. No title, claim, escrow or readiness is fabricated.
use super::*;
use mush_roll::commerce::PendingCharterTransferConfig;
use mush_roll::commerce::registry_revocation::RegistryRevocationPolicy;
fn config() -> Result<PendingCharterTransferConfig, ModError> {
    PendingCharterTransferConfig::new(mush_core::GOD, 60)
}
fn sanction() -> Result<RegistryRevocationPolicy, Box<dyn std::error::Error>> {
    Ok(RegistryRevocationPolicy::new(
        issuer()?,
        mush_org::OrgId(4),
        mush_org::OrgScopeId(5),
        vec![mush_core::ObjId(6)],
    )?)
}
#[test]
fn pending_charters_are_explicit_active_and_preserve_immediate_references() -> TestResult {
    assert!(PendingCharterTransferConfig::new(mush_core::ObjId(0), 60).is_err());
    for seconds in [0, 259201] {
        assert!(PendingCharterTransferConfig::new(mush_core::GOD, seconds).is_err());
    }
    for seconds in [1, 259200] {
        PendingCharterTransferConfig::new(mush_core::GOD, seconds)?;
    }
    assert!(
        prepare(CommerceMode::Initialize)?
            .with_pending_charter_transfers(config()?)
            .is_err()
    );
    assert!(
        prepare(CommerceMode::Active)?
            .with_pending_charter_transfers(config()?)
            .is_err()
    );
    let plain = prepare(CommerceMode::Active)?.with_charter_transfers()?;
    let before = engine(&plain)?;
    let selected = plain.with_pending_charter_transfers(config()?)?;
    let actual = engine(&selected)?;
    for old in mush_roll::commerce::charter_transfer::CharterTransferConfig::new().actions()? {
        assert!(
            before
                .registry
                .definitions()
                .ok_or("definitions")?
                .get_exact(&old)
                .is_ok()
        );
        assert!(
            actual
                .registry
                .definitions()
                .ok_or("definitions")?
                .get_exact(&old)
                .is_ok()
        );
    }
    for action in config()?.actions()? {
        assert!(
            before
                .registry
                .definitions()
                .ok_or("definitions")?
                .get_exact(&action)
                .is_err()
        );
        assert!(
            actual
                .registry
                .definitions()
                .ok_or("definitions")?
                .get_exact(&action)
                .is_ok()
        );
    }
    assert!(selected.with_pending_charter_transfers(config()?).is_err());
    Ok(())
}
#[test]
fn pending_compound_revocation_resolves_both_explicit_configuration_orders() -> TestResult {
    let policy = sanction()?;
    let first = prepare(CommerceMode::Active)?
        .with_charter_transfers()?
        .with_registry_revocation(policy.clone())?
        .with_pending_charter_transfers(config()?)?;
    let second = prepare(CommerceMode::Active)?
        .with_charter_transfers()?
        .with_pending_charter_transfers(config()?)?
        .with_registry_revocation(policy.clone())?;
    for cfg in [first, second] {
        let actual = engine(&cfg)?;
        let definitions = actual.registry.definitions().ok_or("definitions")?;
        assert!(definitions.get_exact(&policy.action()?).is_ok());
        for a in config()?.actions()? {
            assert!(definitions.get_exact(&a).is_ok());
        }
    }
    Ok(())
}
#[test]
fn pending_requires_matching_real_economy_and_roll_owner_configuration() -> TestResult {
    let plain = prepare(CommerceMode::Active)?.with_charter_transfers()?;
    let selected = prepare(CommerceMode::Active)?
        .with_charter_transfers()?
        .with_pending_charter_transfers(config()?)?;
    for missing_economy in [false, true] {
        let economy = if missing_economy { &plain } else { &selected };
        let roll = if missing_economy { &selected } else { &plain };
        let result =
            compose_boxed(
                World::new_minimal(),
                Registry::new(),
                vec![
                    Box::new(economy.configure_economy(
                        EconomyModule::new().with_title_issuers(vec![issuer()?]),
                    )?),
                    Box::new(roll.configure_roll(RollModule::with_writers([crate::NS]))),
                    Box::new(selected.configure_tacsim(layer())?),
                ],
            );
        assert!(result.is_err(), "mismatched owner composition accepted");
    }
    Ok(())
}

#[test]
fn pending_facts_and_measured_recall_share_one_exact_roll_guard_slot() -> TestResult {
    for pending in [false, true] {
        let selected = prepare(CommerceMode::Active)?
            .with_charter_transfers()?
            .with_minds_facts()?
            .with_registry_reconformance()?;
        let selected = if pending {
            selected.with_pending_charter_transfers(config()?)?
        } else {
            selected
        };
        let actual = engine(&selected)?;
        assert!(has(&actual, "roll", "action", "record_mind_term_lapse"));
        assert!(has(&actual, "roll", "action", "recall_nonconforming_mind"));
        for action in config()?.actions()? {
            assert_eq!(
                actual
                    .registry
                    .definitions()
                    .ok_or("definitions")?
                    .get_exact(&action)
                    .is_ok(),
                pending
            );
        }
    }
    Ok(())
}
