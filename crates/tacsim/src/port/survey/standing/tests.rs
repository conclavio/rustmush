#![cfg(test)]

use super::*;
use crate::surface::survey::{SURVEY_SCHEMA, SurveyKey};

fn record() -> SurveyRecord {
    SurveyRecord {
        schema: SURVEY_SCHEMA,
        offer: SurveyKey::parse("ahuna-a").unwrap(),
        revision: 1,
        account: mush_core::AccountId::mint("survey-pilot", 1).expect("account identity"),
        pilot: mush_core::ObjId(7),
        sponsor: PrincipalId::Organization(OrgId(2)),
        sponsor_account: 0,
        payee: 1,
        contract: 0,
        site: CanonicalId::new("ahuna_port").unwrap(),
        block: 0,
        reward_minor: 50_000,
        accepted_ic: 1_000,
        deadline_ic: 1_000 + 604_800,
        coverage: [0; 32],
        completed: None,
        cancelled: None,
        standing: Some(SurveyStandingTerms {
            beneficiary: OrgId(2),
            delta: 10,
            secondary: vec![SurveyStandingEffect {
                organization: OrgId(3),
                delta: -5,
            }],
            representative: Some(mush_core::ObjId(8)),
        }),
    }
}

fn consent(row: &SurveyRecord) -> [u8; 32] {
    mush_economy::consent_hash(
        &PrincipalId::Object(row.pilot),
        &super::super::expected_consent(row),
    )
}

#[test]
fn signed_effects_and_the_original_representative_are_frozen_into_consent() {
    let row = record();
    row.check().unwrap();
    let restored: SurveyRecord =
        serde_json::from_slice(&serde_json::to_vec(&row).unwrap()).unwrap();
    assert_eq!(restored, row);
    assert_eq!(fact_id(&restored), fact_id(&row));
    assert_eq!(restored.standing.as_ref().unwrap().secondary[0].delta, -5);
    assert!(
        line(&World::new_minimal(), row.standing.as_ref().unwrap()).contains("organization #3 -5")
    );
    let original = consent(&row);
    for change in 0..5 {
        let mut changed = row.clone();
        let terms = changed.standing.as_mut().unwrap();
        match change {
            0 => terms.delta += 1,
            1 => terms.secondary[0].delta -= 1,
            2 => terms.secondary[0].organization = OrgId(4),
            3 => terms.representative = Some(mush_core::ObjId(9)),
            _ => terms.secondary.clear(),
        }
        assert_ne!(
            consent(&changed),
            original,
            "an effect or representative changed"
        );
    }
}

#[test]
fn legacy_commissions_keep_their_original_fact_and_have_no_new_effects() {
    let mut row = record();
    row.standing = None;
    let value = serde_json::to_value(&row).unwrap();
    assert!(value.get("standing").is_none());
    let legacy: SurveyRecord = serde_json::from_value(value).unwrap();
    legacy.check().unwrap();
    assert!(legacy.standing.is_none());
    assert_eq!(fact_id(&legacy), "ahuna-a/0");
    assert!(!paid(&World::new_minimal(), &legacy));
}

#[test]
fn standing_terms_reject_implicit_beneficiaries_duplicate_orgs_and_unsigned_harm() {
    let row = record();
    for change in 0..5 {
        let mut changed = row.clone();
        let terms = changed.standing.as_mut().unwrap();
        match change {
            0 => terms.beneficiary = OrgId(9),
            1 => terms.delta = 0,
            2 => terms.secondary[0].organization = terms.beneficiary,
            3 => terms.secondary[0].delta = 5,
            _ => terms.secondary = vec![terms.secondary[0].clone(); 5],
        }
        assert!(changed.check().is_err());
    }
    assert!(
        super::super::offers()
            .iter()
            .all(|offer| offer.secondary_effects.is_empty())
    );
}

#[test]
fn a_completed_record_without_its_paid_contract_is_not_proof() {
    let mut row = record();
    row.coverage = [u8::MAX; 32];
    row.completed = Some(crate::surface::survey::SurveyCompletion {
        tv: mush_core::ObjId(10),
        ordinal: 1,
        at_ic: 2_000,
        at_utc: 2_000,
    });
    row.check().unwrap();
    assert!(!paid(&World::new_minimal(), &row));
}
