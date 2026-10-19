use super::*;
use mush_core::action::{
    ActionInvocationDraft, ActionOfferId, IdempotencyKey, IdempotencyScope, RequestUid,
    ScopedIdempotencyKey,
};
use mush_core::mechanics::CanonicalId;
use mush_core::{CanonicalHash, Engine, Registry};
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
fn invocation(parameters: serde_json::Value) -> TestResult<ActionInvocation> {
    let request = RequestUid::from_bytes([1; 16]);
    let actor = ObjId(2);
    let principal = PrincipalId::Object { id: actor };
    Ok(ActionInvocation::new(ActionInvocationDraft {
        request,
        principal: principal.clone(),
        actor,
        offered_action: ActionOfferId(CanonicalHash::from_bytes([2; 32])),
        subject: PerceivedRef::SelfActor,
        targets: vec![],
        parameters,
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal,
                owner: OwnerId::new(crate::NS)?,
                namespace: CanonicalId::new("withdrawal_test")?,
            },
            1,
            IdempotencyKey::Request(request),
        ),
    })?)
}
#[test]
fn withdrawal_request_has_only_canonical_filed_selector_and_exact_new_action() -> TestResult {
    let action = action_reference()?;
    assert_eq!(
        Request::parse(&invocation(json!({"filed":"7"}))?, &action)?,
        FiledRevisionId::new(7)?
    );
    for value in [
        json!("0"),
        json!("07"),
        json!("+7"),
        json!("18446744073709551616"),
        json!(7),
        json!(null),
    ] {
        assert!(Request::parse(&invocation(json!({"filed":value}))?, &action).is_err());
    }
    for field in ["actor", "title", "parent", "recall", "validated"] {
        let mut params = json!({"filed":"7"});
        params
            .as_object_mut()
            .ok_or("object")?
            .insert(field.into(), json!(true));
        assert!(Request::parse(&invocation(params)?, &action).is_err());
    }
    assert!(
        Request::parse(
            &invocation(json!({"filed":"7"}))?,
            &super::super::authored_retirement::action_reference()?
        )
        .is_err()
    );
    Ok(())
}
fn cause() -> TestResult<records::WithdrawalOperation> {
    let digest = mush_core::DefinitionDigest::from_bytes([3; 32]);
    Ok(serde_json::from_value(
        json!({"format":1,"operation":CommitToken::from_bytes([1;16]),"action":action_reference()?,"author":ObjId(2),
        "initial_filing":CommitToken::from_bytes([2;16]),"filed":FiledRevisionId::new(7)?,"charter":CharterId::new(8)?,"line":super::super::LineId::new(9)?,"title_revision":1,
        "revision":{"format":1,"revision":{"key":{"character":ObjId(2),"mind":1,"number":1},"digest":digest},"source":digest}}),
    )?)
}
#[test]
fn withdrawal_cause_is_strict_exact_and_root_bound_without_mutating_filing() -> TestResult {
    let row = cause()?;
    row.validate()?;
    assert_eq!(
        RootAnchor::parse(&row.root()?.anchor.record())?,
        row.root()?.anchor
    );
    assert_eq!(row.status().withdrawn_by, Some(row.operation));
    assert!(row.status().recalled_by.is_none());
    let mut changed = row.clone();
    changed.title_revision = 2;
    assert_ne!(row.digest()?, changed.digest()?);
    changed.author = ObjId(3);
    assert!(changed.validate().is_err());
    changed = row.clone();
    changed.operation = changed.initial_filing;
    assert!(changed.validate().is_err());
    for field in ["permission", "recall", "source_access"] {
        let mut wire = serde_json::to_value(&row)?;
        wire.as_object_mut()
            .ok_or("object")?
            .insert(field.into(), json!(true));
        assert!(serde_json::from_value::<records::WithdrawalOperation>(wire).is_err());
    }
    let mut wire = serde_json::to_value(&row)?;
    wire.get_mut("revision")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("revision")?
        .insert("authority".into(), json!(true));
    assert!(serde_json::from_value::<records::WithdrawalOperation>(wire).is_err());
    Ok(())
}
#[test]
fn withdrawal_is_explicit_active_registration_and_never_default_or_setup() -> TestResult {
    use super::super::bootstrap_tests::{Manifest, manifest_descriptor, reference};
    use super::super::{
        filing::InitialFilingTerms,
        module::{CommerceConfig, CommerceMode},
        rating::RatingCompatibility,
    };
    let manifest = reference("tacsim", manifest_descriptor());
    let make = |mode| {
        CommerceConfig::new(
            mode,
            InitialFilingTerms::new(90 * 86400, 3 * 86400).map_err(error)?,
            manifest.clone(),
            RatingCompatibility::combat_v1(manifest.digest()),
        )
    };
    assert!(
        make(CommerceMode::Initialize)?
            .with_author_withdrawal()
            .is_err()
    );
    for enabled in [false, true] {
        let config = make(CommerceMode::Active)?;
        let config = if enabled {
            config.with_author_withdrawal()?
        } else {
            config
        };
        let issuer = mush_org::SystemPrincipalId::new("roll.registry").ok_or("issuer")?;
        let policy = mush_economy::registry_filing::InitialFilingPolicy::new(
            issuer.clone(),
            mush_economy::AccountId(1),
            mush_economy::Amount::from_minor(100),
            mush_economy::CatalogRef {
                ruleset: "roll".into(),
                code: "mind_charter".into(),
            },
        )?;
        let filing = config.initial_filing_action()?;
        let engine = Engine::with_modules(
            World::new_minimal(),
            Registry::new(),
            vec![
                Box::new(Manifest),
                Box::new(crate::RollModule::with_writers(["tacsim"]).with_commerce(config)),
                Box::new(
                    mush_economy::EconomyModule::new()
                        .with_title_issuers(vec![issuer])
                        .with_initial_filing(policy, filing),
                ),
            ],
        )?;
        let expected = action_reference()?;
        let count = engine
            .registry
            .definitions()
            .ok_or("definitions")?
            .iter()
            .filter(|entry| entry.reference() == &expected)
            .count();
        assert_eq!(count, usize::from(enabled));
    }
    Ok(())
}
