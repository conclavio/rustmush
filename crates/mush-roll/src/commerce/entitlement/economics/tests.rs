//! Codec and immutable-evidence boundaries, not paid grants or authority.
use super::super::super::rating::RatingClass;
use super::*;
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use serde_json::json;
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
fn hash(byte: u8) -> DefinitionDigest {
    DefinitionDigest::from_bytes([byte; 32])
}
fn identity(number: u64) -> TestResult<RevisionIdentity> {
    Ok(serde_json::from_value(json!({"format":1,
        "revision":{"key":{"character":1,"mind":number,"number":1},"digest":hash(1)},
        "source":hash(2)}))?)
}
fn definition(kind: &str, id: &str, version: u32) -> TestResult<DefinitionRef> {
    Ok(DefinitionRegistryBuilder::new().register(
        OwnerId::new("market")?,
        DefinitionDescriptor::new(
            DefinitionKind::new(kind)?,
            DefinitionId::new(id)?,
            version,
            json!({"format":version}),
        )?,
    )?)
}
fn edge(number: u64, rate: u16) -> TestResult<RoyaltyEdgeV1> {
    Ok(RoyaltyEdgeV1 {
        parent: ParentContent {
            filed: FiledRevisionId::new(number)?,
            revision: identity(number)?,
        },
        charter: CharterId::new(u32::try_from(number)?)?,
        line: LineId::new(number)?,
        acquisition: AcquisitionV1 {
            licence: LicenceId::new(number)?,
            term: TermGeneration::new(1)?,
            operation: CommitToken::from_bytes([u8::try_from(number)?; 16]),
            product_digest: hash(3),
        },
        royalty_permille: rate,
        basis: RoyaltyBasisV1::ExplicitTermEconomicsV1 { digest: hash(4) },
    })
}
fn chain() -> TestResult<FiledRoyaltyTermsV1> {
    Ok(FiledRoyaltyTermsV1::capture(
        FiledRevisionId::new(10)?,
        identity(10)?,
        CommitToken::from_bytes([10; 16]),
        vec![edge(1, 100)?, edge(2, 100)?, edge(3, 100)?],
    )?)
}
fn grant(form: PaidForm) -> TestResult<GrantEconomicsV1> {
    let chain = FiledRoyaltyTermsV1::capture(
        FiledRevisionId::new(10)?,
        identity(10)?,
        CommitToken::from_bytes([10; 16]),
        Vec::new(),
    )?;
    let terms = PaidTerms::original_personal_form(RatingClass::I, form, false)?;
    let renewal = RenewalTemplateV1::new(
        InstrumentId::new("cr").ok_or("instrument")?,
        Amount::from_minor(400),
        MAX_TERM_SECONDS,
        MAX_GRACE_SECONDS,
        terms,
        RevisionPolicy::Pin {
            filed: chain.filed,
            revision: chain.revision,
        },
        definition("entitlement_policy", "personal", 1)?,
    )?;
    Ok(GrantEconomicsV1::new(
        match form {
            PaidForm::Sealed {} => DownstreamTermsV1::Sealed {},
            _ => DownstreamTermsV1::Source {
                royalty_permille: 0,
            },
        },
        renewal,
        chain.reference()?,
    )?)
}
fn term() -> TestResult<PaidTermEconomicsV1> {
    Ok(PaidTermEconomicsV1::capture(
        LicenceId::new(12)?,
        TermGeneration::new(1)?,
        CommitToken::from_bytes([12; 16]),
        hash(12),
        grant(PaidForm::OpenResale {})?,
    )?)
}

#[test]
fn terms_are_required_closed_and_duplicate_fields_never_collapse() -> TestResult {
    let row = term()?;
    let raw = serde_json::to_vec(&row)?;
    assert_eq!(PaidTermEconomicsV1::decode(&raw)?, row);
    let mut unknown = serde_json::to_value(&row)?;
    unknown
        .as_object_mut()
        .ok_or("object")?
        .insert("authorized".into(), json!(true));
    assert!(serde_json::from_value::<PaidTermEconomicsV1>(unknown).is_err());
    let mut missing = serde_json::to_value(&row)?;
    missing.as_object_mut().ok_or("object")?.remove("grant");
    assert!(serde_json::from_value::<PaidTermEconomicsV1>(missing).is_err());
    let raw = String::from_utf8(raw)?;
    let duplicate = raw.replacen("{", "{\"format\":1,", 1);
    assert!(PaidTermEconomicsV1::decode(duplicate.as_bytes()).is_err());
    assert!(PaidTermEconomicsV1::decode(&vec![b' '; MAX_ECONOMICS_BYTES + 1]).is_err());
    Ok(())
}
#[test]
fn source_form_requires_explicit_matching_rate_and_sealed_stays_sealed() -> TestResult {
    for form in [PaidForm::Open {}, PaidForm::OpenResale {}] {
        let mut row = grant(form)?;
        row.downstream = DownstreamTermsV1::Source {
            royalty_permille: 300,
        };
        row.validate()?;
        row.downstream = DownstreamTermsV1::Source {
            royalty_permille: 301,
        };
        assert!(row.validate().is_err());
        row.downstream = DownstreamTermsV1::Sealed {};
        assert!(row.validate().is_err());
    }
    let mut row = grant(PaidForm::Sealed {})?;
    row.downstream = DownstreamTermsV1::Source {
        royalty_permille: 0,
    };
    assert!(row.validate().is_err());
    assert!(serde_json::from_str::<DownstreamTermsV1>(r#"{"kind":"source"}"#).is_err());
    assert!(
        serde_json::from_str::<DownstreamTermsV1>(r#"{"kind":"sealed","royalty_permille":0}"#)
            .is_err()
    );
    Ok(())
}
#[test]
fn royalty_depth_cycles_order_and_aggregate_are_immutable() -> TestResult {
    let row = chain()?;
    assert_eq!(row.total_permille()?, 300);
    assert_eq!(
        FiledRoyaltyTermsV1::decode(&serde_json::to_vec(&row)?)?,
        row
    );
    let mut over = row.clone();
    over.edges.0.first_mut().ok_or("edge")?.royalty_permille = 101;
    assert!(over.validate().is_err());
    let mut deep = row.clone();
    deep.edges.0.push(edge(4, 0)?);
    assert!(deep.validate().is_err());
    assert!(FiledRoyaltyTermsV1::decode(&serde_json::to_vec(&deep)?).is_err());
    let mut cycle = row.clone();
    cycle.edges.0.first_mut().ok_or("edge")?.parent.filed = row.filed;
    assert!(cycle.validate().is_err());
    let mut duplicate = row.clone();
    duplicate.edges.0.pop();
    duplicate.edges.0.push(edge(1, 100)?);
    assert!(duplicate.validate().is_err());
    let mut ordered = row.clone();
    ordered.edges.0.reverse();
    ordered.validate()?;
    assert_ne!(ordered.digest()?, row.digest()?);
    assert!(
        row.check_next_resale(0).is_err(),
        "depth bound remains separate from zero rate"
    );
    Ok(())
}
#[test]
fn historical_zero_is_explicit_and_cannot_be_repriced_or_retyped() -> TestResult {
    let mut edge = edge(1, 0)?;
    edge.basis = RoyaltyBasisV1::LegacySourceFormsV2Zero {
        effect: definition("action", "purchase_original_mind", 2)?,
    };
    edge.validate()?;
    let zero = serde_json::to_vec(&edge)?;
    assert_eq!(serde_json::from_slice::<RoyaltyEdgeV1>(&zero)?, edge);
    edge.royalty_permille = 1;
    assert!(edge.validate().is_err());
    edge.royalty_permille = 0;
    edge.basis = RoyaltyBasisV1::LegacySourceFormsV2Zero {
        effect: definition("action", "purchase_original_mind", 1)?,
    };
    assert!(edge.validate().is_err());
    edge.basis = RoyaltyBasisV1::LegacySourceFormsV2Zero {
        effect: definition("action", "purchase_derived_mind", 2)?,
    };
    assert!(edge.validate().is_err());
    // This codec does not claim the historical effect was granted or OpenResale;
    // the owner must separately prove the actual allocation/term/E4 history.
    Ok(())
}
#[test]
fn renewal_template_matches_exact_product_without_inventing_current_eligibility() -> TestResult {
    let row = grant(PaidForm::OpenResale {})?;
    let r = row.renewal();
    r.matches_product(r.terms(), r.revision(), r.instrument(), r.policy())?;
    let other = PaidTerms::original_personal_form(RatingClass::II, PaidForm::OpenResale {}, false)?;
    assert!(
        r.matches_product(other, r.revision(), r.instrument(), r.policy())
            .is_err()
    );
    assert!(
        r.matches_product(
            r.terms(),
            r.revision(),
            &InstrumentId::new("other").ok_or("instrument")?,
            r.policy()
        )
        .is_err()
    );
    let mut bad = r.clone();
    bad.price = Amount::ZERO;
    assert!(bad.validate().is_err());
    bad = r.clone();
    bad.term_seconds = MAX_TERM_SECONDS + 1;
    assert!(bad.validate().is_err());
    bad = r.clone();
    bad.grace_seconds = MAX_GRACE_SECONDS + 1;
    assert!(bad.validate().is_err());
    bad = r.clone();
    bad.revision = RevisionPolicy::Follow {
        line: LineId::new(1)?,
    };
    assert!(bad.validate().is_err());
    bad = r.clone();
    bad.policy = definition("entitlement_policy", "foreign", 1)?;
    assert!(bad.validate().is_err());
    Ok(())
}
#[test]
fn acquisition_generation_and_exact_issue_identity_change_their_own_digests() -> TestResult {
    let row = chain()?;
    let mut changed = row.clone();
    changed.edges.0.first_mut().ok_or("edge")?.acquisition.term = TermGeneration::new(2)?;
    assert_ne!(changed.digest()?, row.digest()?);
    let row = term()?;
    let mut changed = row.clone();
    changed.operation = CommitToken::from_bytes([90; 16]);
    assert_ne!(changed.digest()?, row.digest()?);
    assert_eq!(
        changed.grant.digest()?,
        row.grant.digest()?,
        "consented content precedes allocation"
    );
    assert_eq!(row.key(), "licence/000000000000000c/term/0000000000000001");
    Ok(())
}
#[test]
fn existing_paid_term_wire_is_not_an_economics_record() -> TestResult {
    use super::super::super::{TermWindow, WorldIcSeconds};
    let terms = PaidTerms::original_personal_form(RatingClass::I, PaidForm::OpenResale {}, false)?;
    let old = super::super::super::paid::PaidTermRecord::original(
        LicenceId::new(1)?,
        TermGeneration::new(1)?,
        TermWindow::new(
            WorldIcSeconds(1),
            WorldIcSeconds(1),
            WorldIcSeconds(2),
            WorldIcSeconds(3),
        )?,
        terms,
        RevisionPolicy::Pin {
            filed: FiledRevisionId::new(10)?,
            revision: identity(10)?,
        },
        CommitToken::from_bytes([1; 16]),
    )?;
    let before = serde_json::to_vec(&old)?;
    assert!(PaidTermEconomicsV1::decode(&before).is_err());
    assert!(
        !serde_json::to_value(&old)?
            .as_object()
            .ok_or("object")?
            .contains_key("economics")
    );
    let decoded = super::super::super::paid::PaidTermRecord::decode(&before)?;
    assert_eq!(serde_json::to_vec(&decoded)?, before);
    Ok(())
}
