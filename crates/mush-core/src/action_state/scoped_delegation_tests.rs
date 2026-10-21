#![cfg(test)]

use std::error::Error;
use std::sync::Mutex;

use super::*;
use crate::{Engine, GameModule, ModError, Registrar, Registry, World};

type TestResult = Result<(), Box<dyn Error>>;
type Handle = StateFamilyHandle<u64, u64>;

fn descriptor(name: &str) -> Result<StateFamilyDescriptor, Box<dyn Error>> {
    Ok(StateFamilyDescriptor::new(
        StateFamilyId::new(name)?,
        CanonicalId::new("decimal.v1")?,
        CanonicalId::new("u64.v1")?,
        1,
        CanonicalId::new("u64.decode.v1")?,
        CanonicalId::new("u64.validate.v1")?,
        CanonicalId::new("u64.encode.v1")?,
    )?)
}

fn register(
    builder: &mut StateFamilyRegistryBuilder,
    owner: &str,
    name: &str,
) -> Result<Handle, Box<dyn Error>> {
    Ok(builder.register(
        OwnerId::new(owner)?,
        descriptor(name)?,
        |key: &u64| Ok(key.to_string()),
        |value: &Value| value.as_u64().ok_or_else(|| "expected u64".to_owned()),
        |_: &u64| Ok(()),
        |value: &u64| Ok(json!(value)),
    )?)
}

fn freeze(
    builder: StateFamilyRegistryBuilder,
    owners: &[&str],
) -> Result<StateFamilyRegistry, Box<dyn Error>> {
    let owners = owners
        .iter()
        .map(|owner| OwnerId::new(*owner))
        .collect::<Result<BTreeSet<_>, _>>()?;
    let readiness = MechanicalOwnerReadiness::for_owners(&owners);
    Ok(builder.freeze_for_owners_with_readiness(&owners, readiness)?)
}

#[test]
fn scoped_access_covers_reads_scans_writes_and_emptiness_without_leaking_other_families()
-> TestResult {
    let mut builder = StateFamilyRegistryBuilder::new();
    let paid = register(&mut builder, "roll", "paid")?;
    let private = register(&mut builder, "roll", "private")?;
    let market = register(&mut builder, "market", "own")?;
    builder.delegate_family(OwnerId::new("roll")?, &paid, OwnerId::new("market")?)?;
    let mut store = ActionStateStore::new(freeze(builder, &["roll", "market"])?);

    let mut own = store.transaction(&private)?;
    own.insert(&private, &1, 99)?;
    let (_, installation) = own.prepare()?.into_parts();
    store.install(installation)?;
    let mut txn = store.transaction(&market)?;
    assert!(txn.family_is_empty(&paid)?);
    txn.insert(&paid, &1, 7)?;
    txn.insert(&market, &1, 3)?;
    assert_eq!(txn.get(&paid, &1)?, Some(&7));
    let (writes, installation) = txn.prepare()?.into_parts();
    assert_eq!(writes.len(), 2);
    assert_eq!(installation.touched_families().len(), 2);
    assert!(
        installation
            .touched_families()
            .iter()
            .any(|(owner, family, _)| { owner.as_str() == "roll" && family.as_str() == "paid" })
    );
    store.install(installation)?;
    assert_eq!(store.get(&paid, &1)?, Some(&7));

    let view = ModuleActionState::for_module(TypedStateLens::Live(&store), Some("market"));
    assert_eq!(view.get(&paid, &1)?, Some(&7));
    assert_eq!(view.scan_family(&paid, None, 1)?.entries().len(), 1);
    assert!(matches!(
        view.get(&private, &1),
        Err(ModuleActionStateReadError::ForeignOwner { .. })
    ));
    assert!(matches!(
        view.scan_family(&private, None, 1),
        Err(ModuleActionStateReadError::ForeignOwner { .. })
    ));

    for operation in 0..4 {
        let mut txn = store.transaction(&market)?;
        txn.insert(&paid, &2, 8)?;
        let refused = match operation {
            0 => txn.get(&private, &1).map(|_| ()),
            1 => txn.insert(&private, &2, 100),
            2 => txn.family_is_empty(&private).map(|_| ()),
            _ => txn.remove(&private, &1),
        };
        assert!(matches!(
            refused,
            Err(ActionStateError::OwnerMismatch { .. })
        ));
        assert!(matches!(
            txn.prepare(),
            Err(ActionStateError::TransactionPoisoned { .. })
        ));
        assert_eq!(store.get(&paid, &2)?, None);
        assert_eq!(store.get(&private, &1)?, Some(&99));
    }
    Ok(())
}

#[test]
fn scoped_grants_require_exact_registered_owned_handle_and_foreign_reads_still_refuse() -> TestResult
{
    let mut builder = StateFamilyRegistryBuilder::new();
    let paid = register(&mut builder, "roll", "paid")?;
    let mut other = StateFamilyRegistryBuilder::new();
    let foreign = register(&mut other, "roll", "paid")?;
    let absent = register(&mut other, "roll", "absent")?;
    let roll = OwnerId::new("roll")?;
    let market = OwnerId::new("market")?;
    assert!(matches!(
        builder.delegate_family(market.clone(), &paid, roll.clone()),
        Err(StateFamilyRegistryError::DelegationOwnerMismatch { .. })
    ));
    assert!(matches!(
        builder.delegate_family(roll.clone(), &foreign, market.clone()),
        Err(StateFamilyRegistryError::ForeignDelegatedFamily { .. })
    ));
    assert!(matches!(
        builder.delegate_family(roll.clone(), &absent, market.clone()),
        Err(StateFamilyRegistryError::UnknownDelegatedFamily { .. })
    ));
    assert!(matches!(
        builder.delegate_family(roll.clone(), &paid, roll.clone()),
        Err(StateFamilyRegistryError::SelfDelegation { .. })
    ));
    builder.delegate_family(roll, &paid, market)?;
    let store = ActionStateStore::new(freeze(builder, &["roll", "market"])?);
    let view = ModuleActionState::for_module(TypedStateLens::Live(&store), Some("market"));
    assert!(matches!(
        view.get(&foreign, &1),
        Err(ModuleActionStateReadError::State(
            ActionStateError::ForeignHandle { .. }
        ))
    ));
    Ok(())
}

#[test]
fn scoped_and_legacy_grants_share_the_existing_distinct_grantee_cap() -> TestResult {
    let mut builder = StateFamilyRegistryBuilder::new();
    let first = register(&mut builder, "roll", "first")?;
    let second = register(&mut builder, "roll", "second")?;
    let owner = OwnerId::new("roll")?;
    for index in 0..MAX_STATE_FAMILY_DELEGATES {
        let grantee = OwnerId::new(format!("grantee{index}"))?;
        if index % 2 == 0 {
            builder.delegate_family(owner.clone(), &first, grantee)?;
        } else {
            builder.delegate(owner.clone(), grantee)?;
        }
    }
    // More families, repeated grants and the legacy grant for the SAME module
    // do not consume a fifth grantee or silently remove the explicit grants.
    let existing = OwnerId::new("grantee0")?;
    builder.delegate_family(owner.clone(), &second, existing.clone())?;
    builder.delegate_family(owner.clone(), &second, existing.clone())?;
    builder.delegate(owner.clone(), existing)?;
    let fifth = OwnerId::new("fifth")?;
    assert!(matches!(
        builder.delegate_family(owner.clone(), &first, fifth.clone()),
        Err(StateFamilyRegistryError::OwnerDelegateLimit {
            max: MAX_STATE_FAMILY_DELEGATES,
            ..
        })
    ));
    assert!(matches!(
        builder.delegate(owner, fifth),
        Err(StateFamilyRegistryError::OwnerDelegateLimit {
            max: MAX_STATE_FAMILY_DELEGATES,
            ..
        })
    ));
    Ok(())
}

#[test]
fn scoped_grants_require_composed_grantees_and_are_not_symmetric_or_transitive() -> TestResult {
    let mut builder = StateFamilyRegistryBuilder::new();
    let paid = register(&mut builder, "roll", "paid")?;
    builder.delegate_family(OwnerId::new("roll")?, &paid, OwnerId::new("ghost")?)?;
    let owners = BTreeSet::from([OwnerId::new("roll")?]);
    let readiness = MechanicalOwnerReadiness::for_owners(&owners);
    assert!(
        matches!(builder.freeze_for_owners_with_readiness(&owners, readiness),
        Err(StateFamilyRegistryError::UnknownOwner { owner }) if owner.as_str() == "ghost")
    );

    let mut builder = StateFamilyRegistryBuilder::new();
    let paid = register(&mut builder, "roll", "paid")?;
    let roll = OwnerId::new("roll")?;
    let market = OwnerId::new("market")?;
    let other = OwnerId::new("other")?;
    builder.delegate_family(roll.clone(), &paid, market.clone())?;
    builder.delegate(market.clone(), other.clone())?;
    let registry = freeze(builder, &["roll", "market", "other"])?;
    assert!(!registry.delegates(&roll, &market));
    assert!(registry.delegates_family(&roll, &paid.descriptor().id, &market));
    assert!(!registry.delegates_family(&roll, &paid.descriptor().id, &other));
    assert!(!registry.delegates_family(&market, &paid.descriptor().id, &roll));
    assert_eq!(
        registry.family_delegations(),
        vec![(roll, paid.descriptor().id.clone(), market)]
    );
    Ok(())
}

fn scoped_fingerprint(
    reverse: bool,
    family: &str,
    grantee: &str,
) -> Result<DefinitionDigest, Box<dyn Error>> {
    let mut builder = StateFamilyRegistryBuilder::new();
    let names = if reverse {
        ["second", "first"]
    } else {
        ["first", "second"]
    };
    let mut handles = BTreeMap::new();
    for name in names {
        handles.insert(name, register(&mut builder, "roll", name)?);
    }
    let selected = handles.get(family).ok_or("missing test family")?;
    let first = handles.get("first").ok_or("missing first family")?;
    let grants = if reverse {
        [(first, "other"), (selected, grantee)]
    } else {
        [(selected, grantee), (first, "other")]
    };
    for (handle, grantee) in grants {
        builder.delegate_family(OwnerId::new("roll")?, handle, OwnerId::new(grantee)?)?;
        builder.delegate_family(OwnerId::new("roll")?, handle, OwnerId::new(grantee)?)?;
    }
    Ok(freeze(builder, &["roll", "market", "other", "third"])?.fingerprint())
}

#[test]
fn scoped_fingerprint_is_canonical_and_names_the_exact_family_and_grantee() -> TestResult {
    let normal = scoped_fingerprint(false, "second", "market")?;
    assert_eq!(normal, scoped_fingerprint(true, "second", "market")?);
    assert_ne!(normal, scoped_fingerprint(false, "first", "market")?);
    assert_ne!(normal, scoped_fingerprint(false, "second", "third")?);
    Ok(())
}

#[test]
fn legacy_grants_keep_exact_v1_fingerprint_and_owner_wide_access() -> TestResult {
    for grant in [false, true] {
        let mut builder = StateFamilyRegistryBuilder::new();
        let first = register(&mut builder, "roll", "first")?;
        let second = register(&mut builder, "roll", "second")?;
        let roll = OwnerId::new("roll")?;
        let market = OwnerId::new("market")?;
        if grant {
            builder.delegate(roll.clone(), market.clone())?;
        }
        let pairs = if grant {
            vec![json!({"owner":"roll","grantee":"market"})]
        } else {
            vec![]
        };
        let expected = DefinitionDigest::for_canonical_value(
            "state_family_registry.v1",
            &json!({
                "families": [
                    {"owner":"roll","descriptor":first.descriptor().canonical_value()},
                    {"owner":"roll","descriptor":second.descriptor().canonical_value()}
                ],
                "delegations": pairs
            }),
        )?;
        let registry = freeze(builder, &["roll", "market"])?;
        assert_eq!(registry.fingerprint(), expected);
        assert!(registry.family_delegations().is_empty());
        assert_eq!(
            registry.delegates_family(&roll, &first.descriptor().id, &market),
            grant
        );
        assert_eq!(
            registry.delegates_family(&roll, &second.descriptor().id, &market),
            grant
        );
    }
    Ok(())
}

struct ScopedOwner {
    captured: Arc<Mutex<Option<Handle>>>,
}
impl GameModule for ScopedOwner {
    fn name(&self) -> &'static str {
        "roll"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let handle = reg.state_family(
            descriptor("paid").map_err(|error| ModError::msg(error.to_string()))?,
            |key: &u64| Ok(key.to_string()),
            |value: &Value| value.as_u64().ok_or_else(|| "expected u64".to_owned()),
            |_: &u64| Ok(()),
            |value: &u64| Ok(json!(value)),
        )?;
        reg.delegate_state_family(&handle, "market")?;
        *self
            .captured
            .lock()
            .map_err(|_| ModError::msg("capture poisoned"))? = Some(handle);
        Ok(())
    }
}
struct Market;
impl GameModule for Market {
    fn name(&self) -> &'static str {
        "market"
    }
    fn register(&mut self, _: &mut Registrar<'_>) -> Result<(), ModError> {
        Ok(())
    }
}

struct AttemptForeignGrant {
    captured: Arc<Mutex<Option<Handle>>>,
}
impl GameModule for AttemptForeignGrant {
    fn name(&self) -> &'static str {
        "market"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let handle = self
            .captured
            .lock()
            .map_err(|_| ModError::msg("capture poisoned"))?
            .clone()
            .ok_or_else(|| ModError::msg("owner did not register first"))?;
        reg.delegate_state_family(&handle, "third")
    }
}

#[test]
fn actual_grantee_registrar_cannot_redelegate_the_owners_handle() -> TestResult {
    let captured = Arc::new(Mutex::new(None));
    let result = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![
            Box::new(ScopedOwner {
                captured: Arc::clone(&captured),
            }),
            Box::new(AttemptForeignGrant { captured }),
        ],
    );
    let error = match result {
        Ok(_) => return Err("foreign module granted a Roll family".into()),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("module market cannot delegate a state family owned by roll")
    );
    Ok(())
}

#[test]
fn registrar_only_grants_during_real_composition_and_frozen_tables_cannot_expand() -> TestResult {
    let captured = Arc::new(Mutex::new(None));
    let mut engine = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![
            Box::new(ScopedOwner {
                captured: Arc::clone(&captured),
            }),
            Box::new(Market),
        ],
    )?;
    let handle = captured
        .lock()
        .map_err(|_| "capture poisoned")?
        .clone()
        .ok_or("no handle")?;
    let before = engine
        .registry
        .state_families()
        .ok_or("no registry")?
        .fingerprint();
    let mut legacy = Registrar::new(&mut engine.registry, "roll");
    assert!(legacy.delegate_state_family(&handle, "other").is_err());
    let mut frozen = Registrar::for_composition(&mut engine.registry, "roll");
    assert!(frozen.delegate_state_family(&handle, "other").is_err());
    assert_eq!(
        engine
            .registry
            .state_families()
            .ok_or("no registry")?
            .fingerprint(),
        before
    );
    assert_eq!(
        engine
            .registry
            .state_families()
            .ok_or("no registry")?
            .family_delegations()
            .len(),
        1
    );
    Ok(())
}

#[test]
fn read_grants_permit_module_reads_never_writes_and_change_identity_only_when_present()
-> TestResult {
    // The same composition with and without a grant: no grant keeps the
    // exact existing fingerprint bytes; a grant is a different deployment.
    let plain = {
        let mut builder = StateFamilyRegistryBuilder::new();
        register(&mut builder, "roll", "history")?;
        register(&mut builder, "market", "own")?;
        freeze(builder, &["roll", "market"])?
    };
    let mut builder = StateFamilyRegistryBuilder::new();
    let history = register(&mut builder, "roll", "history")?;
    let market = register(&mut builder, "market", "own")?;
    let roll = OwnerId::new("roll")?;
    assert!(matches!(
        builder.grant_family_read(roll.clone(), &history, roll.clone()),
        Err(StateFamilyRegistryError::SelfDelegation { .. })
    ));
    assert!(matches!(
        builder.grant_family_read(OwnerId::new("market")?, &history, OwnerId::new("tacsim")?),
        Err(StateFamilyRegistryError::DelegationOwnerMismatch { .. })
    ));
    builder.grant_family_read(roll.clone(), &history, OwnerId::new("market")?)?;
    // Repeating a grant is idempotent.
    builder.grant_family_read(roll.clone(), &history, OwnerId::new("market")?)?;
    let granted = freeze(builder, &["roll", "market"])?;
    assert_ne!(plain.fingerprint(), granted.fingerprint());
    assert_eq!(granted.family_read_grants().len(), 1);
    assert!(granted.family_delegations().is_empty());
    let mut store = ActionStateStore::new(granted);

    let mut own = store.transaction(&history)?;
    own.insert(&history, &1, 42)?;
    let (_, installation) = own.prepare()?.into_parts();
    store.install(installation)?;

    // Market's own ModuleCtx may read and scan the granted family.
    let view = ModuleActionState::for_module(TypedStateLens::Live(&store), Some("market"));
    assert_eq!(view.get(&history, &1)?, Some(&42));
    assert_eq!(view.scan_family(&history, None, 4)?.entries().len(), 1);
    // A module without the grant still refuses.
    let other = ModuleActionState::for_module(TypedStateLens::Live(&store), Some("tacsim"));
    assert!(matches!(
        other.get(&history, &1),
        Err(ModuleActionStateReadError::ForeignOwner { .. })
    ));
    // The grant never reaches the write gate.
    let mut txn = store.transaction(&market)?;
    assert!(matches!(
        txn.insert(&history, &2, 7),
        Err(ActionStateError::OwnerMismatch { .. })
    ));
    assert_eq!(store.get(&history, &2)?, None);

    // An uncomposed reader is refused at freeze.
    let mut builder = StateFamilyRegistryBuilder::new();
    let history = register(&mut builder, "roll", "history")?;
    builder.grant_family_read(roll, &history, OwnerId::new("absent")?)?;
    assert!(freeze(builder, &["roll"]).is_err());
    Ok(())
}
