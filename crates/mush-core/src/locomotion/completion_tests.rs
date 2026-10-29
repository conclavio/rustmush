#![cfg(test)]

use super::*;
use crate::action::{ActionError, ActionProvider};
use crate::participant::{
    BoundParticipant, OwnerPreparation, OwnerPrepareContext, ParticipantError,
};
use crate::{
    ActionInvocation, Engine, GameModule, Registrar, Registry, ReplayWrite, World, WorldEventKind,
};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Clone)]
struct Cell<const N: u8>;
struct Adapter<const N: u8>;
struct Bound;
impl BoundParticipant for Bound {
    fn apply(self) {}
}
impl<const N: u8> OwnerParticipant for Adapter<N> {
    type State = Cell<N>;
    type Request = ();
    type Output = ();
    type Patch = ();
    type Bound<'a> = Bound;
    fn prepare(
        &self,
        _: &Cell<N>,
        _: &OwnerPrepareContext<'_>,
        _: (),
    ) -> Result<OwnerPreparation<(), ()>, ParticipantError> {
        Err(ParticipantError::Unavailable)
    }
    fn recheck(
        &self,
        _: &Cell<N>,
        _: &OwnerPrepareContext<'_>,
        _: &(),
    ) -> Result<(), ParticipantError> {
        Err(ParticipantError::Unavailable)
    }
    fn bind(self: Arc<Self>, _: &mut Cell<N>, _: ()) -> Result<Bound, ParticipantError> {
        Err(ParticipantError::Unavailable)
    }
    fn accepts_write(&self, _: &ReplayWrite) -> bool {
        false
    }
}
struct Provider;
impl ActionProvider for Provider {
    fn validate_invocation(&self, _: &ActionInvocation) -> Result<(), ActionError> {
        Ok(())
    }
}
fn descriptor(kind: &str, id: &str) -> DefinitionDescriptor {
    DefinitionDescriptor::new(
        DefinitionKind::new(kind).expect("kind"),
        DefinitionId::new(id).expect("id"),
        1,
        json!({}),
    )
    .expect("descriptor")
    .as_current()
}
fn reference(owner: &str, kind: &str, id: &str) -> DefinitionRef {
    crate::DefinitionRegistryBuilder::new()
        .register(OwnerId::new(owner).expect("owner"), descriptor(kind, id))
        .expect("reference")
}
fn action() -> DefinitionRef {
    reference("sim", "action", "complete_move")
}
fn participant(id: &str) -> DefinitionRef {
    reference("owner", "owner_participant", id)
}
fn observer(id: &str) -> DefinitionRef {
    crate::DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new("owner").expect("owner"),
            crate::moved_observer::descriptor(id, 1).expect("observer"),
        )
        .expect("observer reference")
}

#[derive(Clone, Copy, Default)]
enum Case {
    #[default]
    Normal,
    WrongType,
    WrongParticipant,
    WrongAction,
    WrongOwner,
    MissingSlot,
    RetainedObserver,
    DuplicateCoverage,
    Grouped,
    Noncontiguous,
    PermissiveFactory,
    MissingBinding,
}
#[derive(Clone, Copy)]
struct Config {
    case: Case,
    second: bool,
    reverse_bindings: bool,
    reverse_callbacks: bool,
    reverse_modules: bool,
    request: bool,
    require: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            case: Case::Normal,
            second: false,
            reverse_bindings: false,
            reverse_callbacks: false,
            reverse_modules: false,
            request: true,
            require: false,
        }
    }
}
struct Coordinator(Config);
impl GameModule for Coordinator {
    fn name(&self) -> &'static str {
        "sim"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.definition_validator(
            DefinitionKind::new("action").map_err(bad)?,
            |_: &DefinitionValidationContext<'_>| Ok(()),
        )?;
        for id in ["complete_move", "other_move"] {
            let action = reg.definition(descriptor("action", id))?;
            reg.action_provider(action, Provider)?;
        }
        if self.0.request {
            reg.request_owner_participant::<Adapter<0>>(action())?;
            reg.request_owner_participant::<Adapter<1>>(action())?;
        }
        if self.0.require {
            reg.require_moved_completion::<Adapter<0>>(
                action(),
                participant("primary"),
                vec![observer("first")],
            )?;
        }
        Ok(())
    }
}
struct OwnerModule(Config, Arc<AtomicUsize>);
impl OwnerModule {
    fn callback(&self) -> Arc<dyn crate::module::WorldObserver> {
        let calls = Arc::clone(&self.1);
        Arc::new(move |_: &mut crate::Ctx<'_>, _: &crate::WorldEvent| {
            calls.fetch_add(1, Ordering::SeqCst);
        })
    }
    fn first_binding(&self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let cfg = self.0;
        if matches!(cfg.case, Case::MissingBinding) {
            return Ok(());
        }
        let observers = if matches!(cfg.case, Case::Grouped | Case::Noncontiguous) {
            vec![observer("first"), observer("second")]
        } else {
            vec![observer("first")]
        };
        let definition = if matches!(cfg.case, Case::WrongParticipant) {
            participant("unregistered")
        } else {
            participant("primary")
        };
        let selected_action = if matches!(cfg.case, Case::WrongAction) {
            reference("sim", "action", "other_move")
        } else {
            action()
        };
        if matches!(cfg.case, Case::WrongType) {
            reg.register_moved_completion::<Adapter<1>>(
                "first_completion",
                1,
                selected_action,
                definition,
                observers,
            )?;
        } else {
            reg.register_moved_completion::<Adapter<0>>(
                "first_completion",
                1,
                selected_action.clone(),
                definition.clone(),
                observers.clone(),
            )?;
            if matches!(cfg.case, Case::DuplicateCoverage) {
                reg.register_moved_completion::<Adapter<0>>(
                    "duplicate_completion",
                    1,
                    selected_action,
                    definition,
                    observers,
                )?;
            }
        }
        Ok(())
    }
    fn second_binding(&self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        if self.0.second {
            reg.register_moved_completion::<Adapter<1>>(
                "second_completion",
                1,
                action(),
                participant("secondary"),
                vec![observer("second")],
            )?;
        }
        Ok(())
    }
}
impl GameModule for OwnerModule {
    fn name(&self) -> &'static str {
        "owner"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let cfg = self.0;
        if !matches!(cfg.case, Case::MissingSlot) {
            reg.state(Cell::<0>);
        }
        reg.state(Cell::<1>);
        reg.definition_validator(
            DefinitionKind::new("owner_participant").map_err(bad)?,
            |_: &DefinitionValidationContext<'_>| Ok(()),
        )?;
        let primary = reg.definition(descriptor("owner_participant", "primary"))?;
        let secondary = reg.definition(descriptor("owner_participant", "secondary"))?;
        reg.definition(descriptor("owner_participant", "unregistered"))?;
        reg.owner_participant(primary, BTreeSet::from([action()]), Adapter::<0>)?;
        reg.owner_participant(secondary, BTreeSet::from([action()]), Adapter::<1>)?;
        if matches!(cfg.case, Case::PermissiveFactory) {
            reg.definition_validator(kind()?, |_: &DefinitionValidationContext<'_>| Ok(()))?;
        }
        let ids = if cfg.reverse_callbacks {
            ["second", "first"]
        } else {
            ["first", "second"]
        };
        for (index, id) in ids.iter().enumerate() {
            if *id == "first" && matches!(cfg.case, Case::RetainedObserver) {
                reg.retain_moved_observer(id, 1)?;
            } else {
                reg.subscribe_moved(id, 1, self.callback())?;
            }
            if index == 0 && matches!(cfg.case, Case::Noncontiguous) {
                reg.subscribe(WorldEventKind::Moved, self.callback());
            }
        }
        reg.subscribe(WorldEventKind::Moved, self.callback());
        if matches!(cfg.case, Case::WrongOwner) {
            let foreign = reference("foreign", "owner_participant", "primary");
            reg.register_moved_completion::<Adapter<0>>(
                "foreign_completion",
                1,
                action(),
                foreign,
                vec![observer("first")],
            )?;
        }
        if cfg.reverse_bindings {
            self.second_binding(reg)?;
            self.first_binding(reg)?;
        } else {
            self.first_binding(reg)?;
            self.second_binding(reg)?;
        }
        Ok(())
    }
}
fn compose(cfg: Config) -> Result<(Engine, Arc<AtomicUsize>), ModError> {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut modules: Vec<Box<dyn GameModule>> = vec![
        Box::new(Coordinator(cfg)),
        Box::new(OwnerModule(cfg, Arc::clone(&calls))),
    ];
    if cfg.reverse_modules {
        modules.reverse();
    }
    let engine = Engine::with_modules(World::new_minimal(), Registry::new(), modules)?;
    Ok((engine, calls))
}
fn manifest(engine: &Engine) -> &FrozenMovementManifest {
    engine
        .registry
        .movement_completion_manifest(&action())
        .expect("frozen actual manifest")
}
fn canonical(value: &Value) -> Vec<u8> {
    canonical_json_bytes_with_limit(value, MAX_MOVEMENT_MANIFEST_BYTES).expect("bounded fixture")
}

#[test]
fn actual_adapter_coverage_retains_every_named_and_anonymous_callback() {
    let (engine, calls) = compose(Config {
        require: true,
        ..Config::default()
    })
    .expect("compose");
    let frozen = manifest(&engine);
    let snapshot = frozen.snapshot();
    assert_eq!(snapshot.entries().len(), 3);
    assert_eq!(snapshot.groups().len(), 1);
    assert_eq!(
        snapshot.entries()[0].coverage(),
        MovementCoverage::Prepared { group: 0 }
    );
    assert_eq!(
        snapshot.entries()[1].coverage(),
        MovementCoverage::LegacyBlocker
    );
    assert!(snapshot.entries()[2].observer().is_none());
    assert_eq!(
        snapshot.entries()[2].coverage(),
        MovementCoverage::LegacyBlocker
    );
    assert!(snapshot.has_blockers());
    assert_eq!(snapshot.groups()[0].participant(), &participant("primary"));
    assert_eq!(snapshot.groups()[0].ordinals(), &[0]);
    assert_eq!(
        snapshot.roster_digest(),
        engine
            .registry
            .moved_observer_roster()
            .expect("roster")
            .digest()
    );
    assert_eq!(frozen.digest(), snapshot.digest().expect("digest"));
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "freeze and inspection never invoke a callback"
    );
}

#[test]
fn metadata_cannot_replace_actual_type_slot_request_grant_or_callback() {
    for case in [
        Case::WrongType,
        Case::WrongParticipant,
        Case::WrongAction,
        Case::WrongOwner,
        Case::MissingSlot,
        Case::RetainedObserver,
        Case::DuplicateCoverage,
        Case::Noncontiguous,
        Case::PermissiveFactory,
    ] {
        assert!(
            compose(Config {
                case,
                ..Config::default()
            })
            .is_err()
        );
    }
    assert!(
        compose(Config {
            request: false,
            ..Config::default()
        })
        .is_err()
    );
    assert!(
        compose(Config {
            case: Case::MissingBinding,
            require: true,
            ..Config::default()
        })
        .is_err()
    );
}

#[test]
fn grouped_callbacks_share_one_real_slot_and_preserve_original_order() {
    let (engine, _) = compose(Config {
        case: Case::Grouped,
        ..Config::default()
    })
    .expect("group");
    let snapshot = manifest(&engine).snapshot();
    assert_eq!(snapshot.groups().len(), 1);
    assert_eq!(snapshot.groups()[0].ordinals(), &[0, 1]);
    assert_eq!(
        snapshot.entries()[0].coverage(),
        snapshot.entries()[1].coverage()
    );
    assert!(
        compose(Config {
            case: Case::Grouped,
            reverse_callbacks: true,
            ..Config::default()
        })
        .is_err(),
        "binding order cannot reorder actual callbacks"
    );
}

#[test]
fn binding_and_module_registration_order_are_irrelevant_but_dispatch_order_is_not() {
    let cfg = Config {
        second: true,
        ..Config::default()
    };
    let (first, _) = compose(cfg).expect("first");
    let (reordered, _) = compose(Config {
        reverse_bindings: true,
        reverse_modules: true,
        ..cfg
    })
    .expect("reordered registrations");
    assert_eq!(manifest(&first).digest(), manifest(&reordered).digest());
    let (changed, _) = compose(Config {
        reverse_callbacks: true,
        ..cfg
    })
    .expect("changed callbacks");
    assert_ne!(manifest(&first).digest(), manifest(&changed).digest());
    assert_eq!(
        manifest(&first).snapshot().action(),
        manifest(&changed).snapshot().action()
    );
}

#[test]
fn retained_partial_snapshot_stays_partial_after_current_coverage_expands() {
    let (earlier, _) = compose(Config::default()).expect("partial");
    let (current, _) = compose(Config {
        second: true,
        ..Config::default()
    })
    .expect("expanded");
    let old = manifest(&earlier).snapshot();
    let new = manifest(&current).snapshot();
    assert_eq!(old.action(), new.action());
    assert_ne!(manifest(&earlier).digest(), manifest(&current).digest());
    let retained = MovementManifestSnapshot::decode(
        &old.encode().expect("old snapshot"),
        current.registry.definitions().expect("definitions"),
    )
    .expect("retained exact definitions validate older coverage");
    assert_eq!(&retained, old);
    assert_eq!(
        retained.entries()[1].coverage(),
        MovementCoverage::LegacyBlocker
    );
    assert_eq!(
        new.entries()[1].coverage(),
        MovementCoverage::Prepared { group: 1 }
    );
}

#[test]
fn strict_codec_refuses_unknown_missing_reordered_and_forged_coverage() {
    let (engine, _) = compose(Config::default()).expect("compose");
    let definitions = engine.registry.definitions().expect("definitions");
    let snapshot = manifest(&engine).snapshot();
    let bytes = snapshot.encode().expect("encode");
    assert_eq!(
        MovementManifestSnapshot::decode(&bytes, definitions).expect("decode"),
        *snapshot
    );
    let base = serde_json::to_value(snapshot).expect("value");
    for field in ["format", "action", "roster_digest", "entries", "groups"] {
        let mut value = base.clone();
        value.as_object_mut().expect("object").remove(field);
        assert!(MovementManifestSnapshot::decode(&canonical(&value), definitions).is_err());
    }
    let mut bad_values = Vec::new();
    let mut value = base.clone();
    value["unknown"] = json!(true);
    bad_values.push(value);
    let mut value = base.clone();
    value["format"] = json!(2);
    bad_values.push(value);
    let mut value = base.clone();
    value["entries"][0]["ordinal"] = json!(1);
    bad_values.push(value);
    let mut value = base.clone();
    value["entries"][0]["coverage"] = json!({"kind":"legacy_blocker"});
    bad_values.push(value);
    let mut value = base.clone();
    value["entries"][2]["coverage"] = json!({"kind":"prepared","group":0});
    bad_values.push(value);
    let mut value = base.clone();
    value["groups"][0]["ordinals"] = json!([0, 1]);
    bad_values.push(value);
    let mut value = base.clone();
    value["groups"][0]["participant"] = json!(participant("secondary"));
    bad_values.push(value);
    let mut value = base;
    value["entries"][2]
        .as_object_mut()
        .expect("entry")
        .remove("observer");
    bad_values.push(value);
    for value in bad_values {
        assert!(MovementManifestSnapshot::decode(&canonical(&value), definitions).is_err());
    }
    let mut noncanonical = bytes;
    noncanonical.push(b' ');
    assert!(MovementManifestSnapshot::decode(&noncanonical, definitions).is_err());
    assert!(
        MovementManifestSnapshot::decode(&vec![b' '; MAX_MOVEMENT_MANIFEST_BYTES + 1], definitions)
            .is_err()
    );
}

#[test]
fn public_legacy_registrar_cannot_claim_prepared_coverage() {
    let mut registry = Registry::new();
    assert!(
        Registrar::new(&mut registry, "owner")
            .register_moved_completion::<Adapter<0>>(
                "spoofed",
                1,
                action(),
                participant("primary"),
                vec![observer("first")],
            )
            .is_err()
    );
}

#[test]
fn named_callbacks_cannot_repeat_even_with_a_recomputed_roster_digest() {
    let (engine, _) = compose(Config::default()).expect("compose");
    let mut forged = manifest(&engine).snapshot().clone();
    forged.entries[1].observer = forged.entries[0].observer.clone();
    let rows: Vec<_> = forged
        .entries
        .iter()
        .map(|entry| {
            json!({
                "owner":entry.owner,"definition":entry.observer,"mode":"legacy_post_mutation"
            })
        })
        .collect();
    forged.roster_digest = DefinitionDigest::for_canonical_value_with_limit(
        "core.moved_observer_roster.v1",
        &json!(rows),
        16 * 1024,
    )
    .expect("recomputed digest");
    assert!(
        MovementManifestSnapshot::decode(
            &forged.encode().expect("canonical forged snapshot"),
            engine.registry.definitions().expect("definitions"),
        )
        .is_err()
    );
}

#[test]
fn declaration_and_requirement_caps_refuse_the_next_item() {
    let mut builder = MovementCompletionBuilder::default();
    for index in 0..=MAX_MOVEMENT_COMPLETION_BINDINGS {
        let action = reference("sim", "action", &format!("move{index}"));
        let participant = participant("primary");
        let observers = vec![observer("first")];
        let binding = crate::DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new("owner").expect("owner"),
                binding_descriptor(
                    &format!("binding{index}"),
                    1,
                    action.clone(),
                    participant.clone(),
                    observers.clone(),
                )
                .expect("descriptor"),
            )
            .expect("binding");
        let registration = builder.register::<Adapter<0>>(
            OwnerId::new("owner").expect("owner"),
            binding,
            action.clone(),
            participant.clone(),
            observers.clone(),
        );
        let requirement = builder.require::<Adapter<0>>(action, participant, observers);
        assert_eq!(
            registration.is_ok(),
            index < MAX_MOVEMENT_COMPLETION_BINDINGS
        );
        assert_eq!(
            requirement.is_ok(),
            index < MAX_MOVEMENT_COMPLETION_BINDINGS
        );
    }
    assert_eq!(
        builder.registrations.len(),
        MAX_MOVEMENT_COMPLETION_BINDINGS
    );
    assert_eq!(builder.requirements.len(), MAX_MOVEMENT_COMPLETION_BINDINGS);
}

#[test]
fn a_late_anonymous_callback_invalidates_current_coverage_without_rewriting_history() {
    let (mut engine, calls) = compose(Config::default()).expect("compose");
    let earlier = manifest(&engine).snapshot().clone();
    Registrar::new(&mut engine.registry, "owner").subscribe(
        WorldEventKind::Moved,
        Arc::new(|_: &mut crate::Ctx<'_>, _: &crate::WorldEvent| {}),
    );
    assert!(
        engine
            .registry
            .movement_completion_manifest(&action())
            .is_none()
    );
    assert_eq!(
        engine
            .registry
            .moved_observer_roster()
            .expect("actual roster")
            .entries()
            .len(),
        4
    );
    assert_eq!(
        MovementManifestSnapshot::decode(
            &earlier.encode().expect("earlier snapshot"),
            engine.registry.definitions().expect("definitions"),
        )
        .expect("retained snapshot"),
        earlier
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
