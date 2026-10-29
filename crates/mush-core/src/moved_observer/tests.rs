#![cfg(test)]

use super::*;
use crate::{
    Ctx, Effects, Engine, GOD, GameModule, LIMBO, Outbox, Registrar, Registry, World, WorldEvent,
    WorldEventKind, WorldObserver,
};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
enum Registration {
    Current(&'static str, u32),
    Historical(&'static str, u32),
    Anonymous,
    Other,
    SwallowDuplicate,
    PermissiveThenAnonymous,
    BadSchema,
    BadReferences,
}

struct ObserverModule {
    owner: &'static str,
    registrations: Vec<Registration>,
    calls: Arc<Mutex<Vec<(usize, String)>>>,
}
impl GameModule for ObserverModule {
    fn name(&self) -> &'static str {
        self.owner
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        for (index, registration) in self.registrations.iter().enumerate() {
            let calls = self.calls.clone();
            let callback: Arc<dyn WorldObserver> =
                Arc::new(move |ctx: &mut Ctx<'_>, _: &WorldEvent| {
                    calls
                        .lock()
                        .unwrap()
                        .push((index, ctx.module.as_deref().unwrap_or("core").to_owned()));
                });
            match registration {
                Registration::Current(id, version) => {
                    reg.subscribe_moved(id, *version, callback)?;
                }
                Registration::Historical(id, version) => {
                    reg.retain_moved_observer(id, *version)?;
                }
                Registration::Anonymous => reg.subscribe(WorldEventKind::Moved, callback),
                Registration::Other => reg.subscribe(WorldEventKind::ObjDestroyed, callback),
                Registration::SwallowDuplicate => {
                    reg.subscribe_moved("first", 1, callback.clone())?;
                    if reg.subscribe_moved("first", 2, callback.clone()).is_ok() {
                        return Err(ModError::msg("second current callback was admitted"));
                    }
                    // A caller explicitly falling back to an ordinary subscription
                    // must remain visible, rather than acquire the failed identity.
                    reg.subscribe(WorldEventKind::Moved, callback);
                }
                Registration::PermissiveThenAnonymous => {
                    reg.definition_validator(
                        kind()?,
                        |_: &DefinitionValidationContext<'_>| Ok(()),
                    )?;
                    if reg.subscribe_moved("unsafe", 1, callback.clone()).is_ok() {
                        return Err(ModError::msg("external validator replaced Core's factory"));
                    }
                    reg.subscribe(WorldEventKind::Moved, callback);
                }
                Registration::BadSchema => {
                    reg.subscribe_moved("valid", 1, callback)?;
                    reg.definition(
                        DefinitionDescriptor::new(
                            kind()?,
                            DefinitionId::new("wrong").unwrap(),
                            1,
                            json!({"format":1,"event":"moved","delivery":"prepared"}),
                        )
                        .unwrap(),
                    )?;
                }
                Registration::BadReferences => {
                    let dependency = reg.retain_moved_observer("retained", 1)?;
                    reg.subscribe_moved("valid", 1, callback)?;
                    reg.definition(descriptor("wrong", 1)?.with_references(vec![dependency]))?;
                }
            }
        }
        Ok(())
    }
}

fn module(owner: &'static str, registrations: Vec<Registration>) -> ObserverModule {
    ObserverModule {
        owner,
        registrations,
        calls: Arc::new(Mutex::new(Vec::new())),
    }
}

fn engine(modules: Vec<ObserverModule>) -> Result<Box<Engine>, ModError> {
    Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        modules
            .into_iter()
            .map(|module| Box::new(module) as Box<dyn GameModule>)
            .collect(),
    )
    .map(Box::new)
}

#[test]
fn actual_order_owner_and_delivery_are_preserved_without_inspection_callbacks() {
    let first = module(
        "alpha",
        vec![
            Registration::Current("one", 1),
            Registration::Other,
            Registration::Anonymous,
            Registration::Current("two", 3),
        ],
    );
    let first_calls = first.calls.clone();
    let second = module("beta", vec![Registration::Current("one", 1)]);
    let second_calls = second.calls.clone();
    let mut engine = engine(vec![first, second]).unwrap();
    let roster = engine.registry.moved_observer_roster().unwrap();
    assert_eq!(roster.entries().len(), 4);
    assert!(roster.has_blockers());
    assert_eq!(
        roster.digest(),
        engine.registry.moved_observer_roster().unwrap().digest()
    );
    for entry in roster.entries() {
        assert_eq!(entry.mode(), MovedObserverMode::LegacyPostMutation);
        if let Some(reference) = entry.definition() {
            assert_eq!(&reference.key().owner, entry.owner());
            assert_eq!(reference.key().kind.as_str(), "moved_observer");
        }
    }
    assert_eq!(roster.entries()[0].owner().as_str(), "alpha");
    assert!(roster.entries()[1].definition().is_none());
    assert_eq!(
        roster.entries()[2].definition().unwrap().key().id.as_str(),
        "two"
    );
    assert_eq!(roster.entries()[3].owner().as_str(), "beta");
    assert!(first_calls.lock().unwrap().is_empty());
    assert!(second_calls.lock().unwrap().is_empty());
    engine.emit_world_event(
        &WorldEvent::Moved {
            who: GOD,
            from: None,
            to: LIMBO,
        },
        &mut Outbox::new(),
        &mut Effects::new(),
    );
    assert_eq!(
        *first_calls.lock().unwrap(),
        vec![
            (0, "alpha".into()),
            (2, "alpha".into()),
            (3, "alpha".into())
        ]
    );
    assert_eq!(*second_calls.lock().unwrap(), vec![(0, "beta".into())]);
}

#[test]
fn digest_binds_actual_order_including_anonymous_entries() {
    let one = engine(vec![module(
        "alpha",
        vec![Registration::Current("one", 1), Registration::Anonymous],
    )])
    .unwrap();
    let two = engine(vec![module(
        "alpha",
        vec![Registration::Anonymous, Registration::Current("one", 1)],
    )])
    .unwrap();
    assert_ne!(
        one.registry.moved_observer_roster().unwrap().digest(),
        two.registry.moved_observer_roster().unwrap().digest()
    );
}

#[test]
fn retained_history_never_becomes_a_callback_or_current_adapter() {
    let only_history = engine(vec![module(
        "alpha",
        vec![Registration::Historical("one", 1)],
    )])
    .unwrap();
    let empty = only_history.registry.moved_observer_roster().unwrap();
    assert!(empty.entries().is_empty());
    assert!(!empty.has_blockers());
    let actual = engine(vec![module(
        "alpha",
        vec![
            Registration::Historical("one", 1),
            Registration::Current("one", 2),
        ],
    )])
    .unwrap();
    let roster = actual.registry.moved_observer_roster().unwrap();
    assert_eq!(roster.entries().len(), 1);
    assert_eq!(roster.entries()[0].definition().unwrap().version(), 2);
    let historical = crate::DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new("alpha").unwrap(),
            descriptor("one", 1).unwrap(),
        )
        .unwrap();
    let definitions = actual.registry.definitions().unwrap();
    assert!(definitions.inspect_exact(&historical).is_ok());
    assert!(
        super::roster(
            std::iter::once(("alpha", Some(&historical))),
            Some(definitions)
        )
        .is_err()
    );
}

#[test]
fn duplicate_exact_and_second_current_callbacks_refuse() {
    for version in [1, 2] {
        assert!(
            engine(vec![module(
                "alpha",
                vec![
                    Registration::Current("one", 1),
                    Registration::Current("one", version)
                ]
            )])
            .is_err()
        );
    }
}

#[test]
fn swallowed_registration_errors_cannot_hide_actual_legacy_callbacks() {
    let duplicate = engine(vec![module("alpha", vec![Registration::SwallowDuplicate])]).unwrap();
    let roster = duplicate.registry.moved_observer_roster().unwrap();
    assert_eq!(roster.entries().len(), 2);
    assert!(roster.entries()[0].definition().is_some());
    assert!(roster.entries()[1].definition().is_none());
    assert!(roster.has_blockers());
    let permissive = engine(vec![module(
        "alpha",
        vec![Registration::PermissiveThenAnonymous],
    )])
    .unwrap();
    let roster = permissive.registry.moved_observer_roster().unwrap();
    assert_eq!(roster.entries().len(), 1);
    assert!(roster.entries()[0].definition().is_none());
    assert!(roster.has_blockers());
}

#[test]
fn exact_factory_validator_rejects_wrong_payload_and_extra_dependencies() {
    assert!(engine(vec![module("alpha", vec![Registration::BadSchema])]).is_err());
    assert!(engine(vec![module("alpha", vec![Registration::BadReferences])]).is_err());
}

#[test]
fn roster_refuses_unknown_owner_spoof_and_wrong_digest_without_callbacks() {
    let actual = engine(vec![module("alpha", vec![Registration::Current("one", 1)])]).unwrap();
    let roster = actual.registry.moved_observer_roster().unwrap();
    let reference = roster.entries()[0].definition().unwrap();
    let definitions = actual.registry.definitions();
    assert!(super::roster(std::iter::once(("beta", Some(reference))), definitions).is_err());
    assert!(super::roster(std::iter::once(("alpha", Some(reference))), None).is_err());
    let unknown = crate::DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new("alpha").unwrap(),
            descriptor("missing", 1).unwrap(),
        )
        .unwrap();
    assert!(super::roster(std::iter::once(("alpha", Some(&unknown))), definitions).is_err());
    let different = crate::DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new("alpha").unwrap(),
            DefinitionDescriptor::new(
                kind().unwrap(),
                DefinitionId::new("one").unwrap(),
                1,
                json!({"wrong":true}),
            )
            .unwrap(),
        )
        .unwrap();
    assert!(super::roster(std::iter::once(("alpha", Some(&different))), definitions).is_err());
}

#[test]
fn bounded_roster_never_returns_a_truncated_actual_census() {
    let boundary = engine(vec![
        module(
            "alpha",
            vec![Registration::Anonymous; MAX_MOVED_OBSERVERS - 1],
        ),
        module("beta", vec![Registration::Current("last", 1)]),
    ])
    .unwrap();
    assert_eq!(
        boundary
            .registry
            .moved_observer_roster()
            .unwrap()
            .entries()
            .len(),
        MAX_MOVED_OBSERVERS
    );
    assert!(
        engine(vec![
            module("alpha", vec![Registration::Anonymous; MAX_MOVED_OBSERVERS]),
            module("beta", vec![Registration::Current("excess", 1)])
        ])
        .is_err()
    );
    // Existing unbounded legacy registration still works, but cannot be
    // mistaken for a bounded complete roster by silently omitting callbacks.
    let legacy = engine(vec![module(
        "alpha",
        vec![Registration::Anonymous; MAX_MOVED_OBSERVERS + 1],
    )])
    .unwrap();
    assert!(legacy.registry.moved_observer_roster().is_err());
}

#[test]
fn invalid_identity_inputs_do_not_register_an_observer() {
    for (id, version) in [("not valid", 1), ("valid", 0)] {
        assert!(
            engine(vec![module(
                "alpha",
                vec![Registration::Current(id, version)]
            )])
            .is_err()
        );
    }
    let mut registry = Registry::new();
    let mut registrar = Registrar::new(&mut registry, "alpha");
    let callback: Arc<dyn WorldObserver> = Arc::new(|_: &mut Ctx<'_>, _: &WorldEvent| {});
    assert!(registrar.subscribe_moved("one", 1, callback).is_err());
    assert!(
        registry
            .moved_observer_roster()
            .unwrap()
            .entries()
            .is_empty()
    );
}
