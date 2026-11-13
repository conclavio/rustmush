#![cfg(test)]

//! Doc 46 CT.0: the personal renewal twin's committed snapshot. The walk is
//! the one both renewal twins share (`mind_house_renewal::tests::ct0_snapshot`'s
//! `renewal_walk!`), over this twin's own fixture helpers and types.
use super::*;

/// The personal twin's one configuration.
fn ct0_personal_configs(
    row: &MindRenewalOperationV1,
    policy: super::super::RenewalPolicy,
) -> TestResult<Vec<(&'static str, super::super::RenewalConfig)>> {
    let [economy, retention] = row.participants();
    Ok(vec![(
        "personal",
        super::super::RenewalConfig::new(
            row.manifest().clone(),
            PurchaseParticipants::new(economy.clone(), retention.clone())?,
            mush_roll::commerce::renewal::writer::RenewalWriter::new(),
            policy,
        )?,
    )])
}

crate::mind_house_renewal::tests::ct0_snapshot::renewal_walk! {
    test: ct0_personal_renewal_snapshot,
    twin: "personal",
    file: "tests/ct0/renewal_personal.snap",
    configs: ct0_personal_configs,
}
