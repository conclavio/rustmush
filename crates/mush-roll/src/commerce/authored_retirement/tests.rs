use super::*;
use mush_core::CanonicalHash;
use mush_core::action::{
    ActionInvocationDraft, ActionOfferId, IdempotencyKey, IdempotencyScope, RequestUid,
    ScopedIdempotencyKey,
};
use mush_core::mechanics::CanonicalId;
type TestResult = Result<(), Box<dyn std::error::Error>>;
fn invocation(
    parameters: serde_json::Value,
) -> Result<ActionInvocation, Box<dyn std::error::Error>> {
    let id = RequestUid::from_bytes([1; 16]);
    let actor = ObjId(2);
    let principal = PrincipalId::Object { id: actor };
    Ok(ActionInvocation::new(ActionInvocationDraft {
        request: id,
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
                namespace: CanonicalId::new("retirement_test")?,
            },
            1,
            IdempotencyKey::Request(id),
        ),
    })?)
}
#[test]
fn captured_retirement_request_requires_exact_stable_mind_and_preserves_v1() -> TestResult {
    let old = action_reference()?;
    let new = captured_action_reference()?;
    check_eq!(old.version(), 1);
    check_eq!(new.version(), 2);
    check_ne!(old, new);
    let legacy = json!({"name":"mine","edit":"1","revision":"1"});
    let captured = json!({"mind":"7","name":"mine","edit":"1","revision":"1"});
    check!(
        Request::parse(&invocation(legacy.clone())?, &old)?
            .mind
            .is_none()
    );
    check_eq!(
        Request::parse(&invocation(captured.clone())?, &new)?.mind,
        Some(7)
    );
    check!(Request::parse(&invocation(legacy)?, &new).is_err());
    check!(Request::parse(&invocation(captured.clone())?, &old).is_err());
    for mind in [
        json!("0"),
        json!("07"),
        json!("+7"),
        json!("18446744073709551616"),
        json!(7),
        json!(null),
    ] {
        let mut invalid = captured.clone();
        *invalid.get_mut("mind").ok_or("mind")? = mind;
        check!(Request::parse(&invocation(invalid)?, &new).is_err());
    }
    for field in ["actor", "account", "source", "validated"] {
        let mut invalid = captured.clone();
        invalid
            .as_object_mut()
            .ok_or("object")?
            .insert(field.into(), json!(1));
        check!(Request::parse(&invocation(invalid)?, &new).is_err());
    }
    let old_definition = captured_definition()?;
    let definition = DefinitionDescriptor::new(
        old_definition.kind().clone(),
        old_definition.id().clone(),
        3,
        old_definition.payload().clone(),
    )?
    .with_references(old_definition.references().to_vec());
    let foreign =
        DefinitionRegistryBuilder::new().register(OwnerId::new(crate::NS)?, definition)?;
    check!(!known_action(&foreign)?);
    check!(Request::parse(&invocation(captured)?, &foreign).is_err());
    Ok(())
}

/// 17g C2a (C2 plan §2.3): the two registered retirement versions project two
/// verbs, so the gateway's `+act` can name either: version 1 its own
/// `roll-retire-authored-draft-v1`, version 2 the original verb. The verb is
/// presentation only; the two definitions are unchanged.
#[test]
fn retirement_versions_project_distinct_verbs() -> TestResult {
    let old = action_reference()?;
    let new = captured_action_reference()?;
    check_eq!(super::provider::verb(&old), "roll-retire-authored-draft-v1");
    check_eq!(super::provider::verb(&new), "roll-retire-authored-draft");
    check_ne!(super::provider::verb(&old), super::provider::verb(&new));
    Ok(())
}
