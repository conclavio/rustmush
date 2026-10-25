#![cfg(test)]

//! Real registered-provider preparation and engine installation across one
//! typed family and three distinct owner slots belonging to two modules.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::*;
use crate::action::{
    ActionError, ActionErrorCode, ActionInvocation, ActionInvocationDraft, ActionOfferId,
    ActionProvider, AuthoritativeActionOutput, CanonicalHash, CommitToken, IdempotencyKey,
    IdempotencyScope, PerceivedRef, PrincipalId, RequestUid, ScopedIdempotencyKey,
};
use crate::action_state::{ActionTxn, StateFamilyDescriptor, StateFamilyHandle};
use crate::authoritative::AuthoritativeKey;
use crate::mechanics::{
    CanonicalId, DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef,
    DefinitionValidationContext, RulesValue,
};
use crate::participant::{
    BoundParticipant, OwnerParticipant, OwnerPreparation, OwnerPrepareContext, ParticipantHandle,
};
use crate::resolution::PrepareCtx;
use crate::{Engine, GameModule, ModError, ObjId, Registrar, Registry};

type Trace = Arc<Mutex<Vec<(u8, &'static str)>>>;
type Counter = StateFamilyHandle<u64, u64>;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Cell<const N: u8>(u64);
struct Adapter<const N: u8> {
    trace: Trace,
}
struct Patch {
    before: u64,
    after: u64,
    refuse: bool,
}
struct Bound<'a, const N: u8> {
    cell: &'a mut Cell<N>,
    after: u64,
    trace: Trace,
}
fn owner<const N: u8>() -> OwnerId {
    OwnerId::new(if N == 3 { "writer" } else { "legacy" }).unwrap()
}
fn key<const N: u8>() -> AuthoritativeKey {
    AuthoritativeKey::parse_canonical(format!("slot{N}")).unwrap()
}
impl<const N: u8> BoundParticipant for Bound<'_, N> {
    fn apply(self) {
        self.trace.lock().unwrap().push((N, "apply"));
        self.cell.0 = self.after;
    }
}
impl<const N: u8> OwnerParticipant for Adapter<N> {
    type State = Cell<N>;
    type Request = Patch;
    type Output = u64;
    type Patch = Patch;
    type Bound<'a> = Bound<'a, N>;
    fn prepare(
        &self,
        state: &Cell<N>,
        _: &OwnerPrepareContext<'_>,
        patch: Patch,
    ) -> Result<OwnerPreparation<u64, Patch>, ParticipantError> {
        if state.0 != patch.before {
            return Err(ParticipantError::Slot);
        }
        let writes = vec![ReplayWrite::ModuleJsonSet {
            namespace: owner::<N>(),
            key: key::<N>(),
            value: json!(patch.after),
        }];
        Ok(OwnerPreparation::new(patch.after, patch, writes))
    }
    fn recheck(
        &self,
        state: &Cell<N>,
        _: &OwnerPrepareContext<'_>,
        patch: &Patch,
    ) -> Result<(), ParticipantError> {
        if state.0 == patch.before {
            Ok(())
        } else {
            Err(ParticipantError::Slot)
        }
    }
    fn bind<'a>(
        self: Arc<Self>,
        cell: &'a mut Cell<N>,
        patch: Patch,
    ) -> Result<Bound<'a, N>, ParticipantError> {
        self.trace.lock().unwrap().push((N, "bind"));
        if cell.0 != patch.before || patch.refuse {
            return Err(ParticipantError::Slot);
        }
        Ok(Bound {
            cell,
            after: patch.after,
            trace: Arc::clone(&self.trace),
        })
    }
    fn accepts_write(&self, write: &ReplayWrite) -> bool {
        matches!(write, ReplayWrite::ModuleJsonSet {namespace, key: actual, value}
            if namespace == &owner::<N>() && actual == &key::<N>() && value.is_u64())
    }
}

#[derive(Clone)]
struct Handles {
    action: DefinitionRef,
    counter: Counter,
}
struct Writer {
    handles: Arc<Mutex<Option<Handles>>>,
    trace: Trace,
    include_owners: bool,
    refuse_last: bool,
}
struct Legacy {
    handles: Arc<Mutex<Option<Handles>>>,
    trace: Trace,
}
fn definition(reg: &mut Registrar<'_>, kind: &str, name: &str) -> Result<DefinitionRef, ModError> {
    reg.definition(
        DefinitionDescriptor::new(
            DefinitionKind::new(kind).unwrap(),
            DefinitionId::new(name).unwrap(),
            1,
            json!({}),
        )
        .unwrap(),
    )
}
/// The fixture definitions deliberately have no parameters or dependencies.
/// Validate that exact contract rather than bypassing semantic composition.
fn register_definition_kind(
    reg: &mut Registrar<'_>,
    kind: &str,
    ids: &'static [&'static str],
) -> Result<(), ModError> {
    reg.definition_validator(
        DefinitionKind::new(kind).unwrap(),
        move |ctx: &DefinitionValidationContext<'_>| {
            let entry = ctx.definition();
            let key = entry.reference().key();
            if entry.reference().version() == 1
                && ids.contains(&key.id.as_str())
                && entry.payload() == &json!({})
                && entry.references().is_empty()
            {
                Ok(())
            } else {
                Err("unexpected composed fixture definition".into())
            }
        },
    )
}
fn register_owner<const N: u8>(
    reg: &mut Registrar<'_>,
    action: &DefinitionRef,
    trace: &Trace,
) -> Result<(), ModError> {
    reg.state(Cell::<N>(u64::from(N) * 10));
    let definition = definition(reg, "participant", &format!("slot{N}"))?;
    reg.owner_participant(
        definition,
        BTreeSet::from([action.clone()]),
        Adapter::<N> {
            trace: Arc::clone(trace),
        },
    )
}
impl GameModule for Writer {
    fn name(&self) -> &'static str {
        "writer"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        register_definition_kind(reg, "action", &["commit"])?;
        register_definition_kind(reg, "outcome", &["committed"])?;
        register_definition_kind(reg, "participant", &["slot3"])?;
        let action = definition(reg, "action", "commit")?;
        let outcome = definition(reg, "outcome", "committed")?;
        let counter = reg.state_family(
            StateFamilyDescriptor::new(
                StateFamilyId::new("counter").unwrap(),
                CanonicalId::new("counter.key.v1").unwrap(),
                CanonicalId::new("counter.value.v1").unwrap(),
                1,
                CanonicalId::new("counter.decode.v1").unwrap(),
                CanonicalId::new("counter.validate.v1").unwrap(),
                CanonicalId::new("counter.encode.v1").unwrap(),
            )
            .unwrap(),
            |key: &u64| Ok(key.to_string()),
            |value: &Value| value.as_u64().ok_or_else(|| "expected u64".into()),
            |_: &u64| Ok(()),
            |value: &u64| Ok(json!(value)),
        )?;
        let first = reg.request_owner_participant::<Adapter<1>>(action.clone())?;
        let second = reg.request_owner_participant::<Adapter<2>>(action.clone())?;
        let third = reg.request_owner_participant::<Adapter<3>>(action.clone())?;
        register_owner::<3>(reg, &action, &self.trace)?;
        let notices = Arc::clone(&self.trace);
        reg.subscribe(
            crate::WorldEventKind::ActionStateChanged,
            Arc::new(move |ctx: &mut crate::Ctx<'_>, _: &crate::WorldEvent| {
                // Observers must see the complete owner state, not a prefix.
                assert_eq!(ctx.w.module_state::<Cell<1>>(), Some(&Cell(11)));
                assert_eq!(ctx.w.module_state::<Cell<2>>(), Some(&Cell(22)));
                assert_eq!(ctx.w.module_state::<Cell<3>>(), Some(&Cell(33)));
                notices.lock().unwrap().push((4, "notice"));
            }),
        );
        reg.action_provider(
            action.clone(),
            Provider {
                outcome,
                counter: counter.clone(),
                first,
                second,
                third,
                include_owners: self.include_owners,
                refuse_last: self.refuse_last,
            },
        )?;
        *self.handles.lock().unwrap() = Some(Handles { action, counter });
        Ok(())
    }
}
impl GameModule for Legacy {
    fn name(&self) -> &'static str {
        "legacy"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        register_definition_kind(reg, "participant", &["slot1", "slot2"])?;
        let action = self
            .handles
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .action
            .clone();
        register_owner::<1>(reg, &action, &self.trace)?;
        register_owner::<2>(reg, &action, &self.trace)
    }
}
struct Provider {
    outcome: DefinitionRef,
    counter: Counter,
    first: ParticipantHandle<Adapter<1>>,
    second: ParticipantHandle<Adapter<2>>,
    third: ParticipantHandle<Adapter<3>>,
    include_owners: bool,
    refuse_last: bool,
}
fn failed() -> ActionError {
    ActionError::new(ActionErrorCode::ProviderRejected, false, "fixture refused").unwrap()
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, _: &ActionInvocation) -> Result<(), ActionError> {
        Ok(())
    }
    fn prepare_authoritative_with_context(
        &self,
        _: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        context: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        txn.insert(&self.counter, &1, 7).map_err(|_| failed())?;
        if self.include_owners {
            // Intentionally opposite registration-key order. The composed
            // installer must bind all three before applying any of them.
            context
                .prepare_owner(
                    &self.third,
                    Patch {
                        before: 30,
                        after: 33,
                        refuse: self.refuse_last,
                    },
                )
                .map_err(|_| failed())?;
            context
                .prepare_owner(
                    &self.second,
                    Patch {
                        before: 20,
                        after: 22,
                        refuse: false,
                    },
                )
                .map_err(|_| failed())?;
            context
                .prepare_owner(
                    &self.first,
                    Patch {
                        before: 10,
                        after: 11,
                        refuse: false,
                    },
                )
                .map_err(|_| failed())?;
        }
        Ok(AuthoritativeActionOutput::new(
            RulesValue::new(self.outcome.clone(), json!({"changed":true})).unwrap(),
        ))
    }
}

fn fixture(include_owners: bool, refuse_last: bool) -> (Engine, Handles, Trace, ActionInvocation) {
    let handles = Arc::new(Mutex::new(None));
    let trace = Arc::new(Mutex::new(Vec::new()));
    let engine = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![
            Box::new(Writer {
                handles: Arc::clone(&handles),
                trace: Arc::clone(&trace),
                include_owners,
                refuse_last,
            }),
            Box::new(Legacy {
                handles: Arc::clone(&handles),
                trace: Arc::clone(&trace),
            }),
        ],
    )
    .unwrap();
    let handles = handles.lock().unwrap().clone().unwrap();
    let request = RequestUid::from_bytes([1; 16]);
    let principal = PrincipalId::Object { id: ObjId(7) };
    let invocation = ActionInvocation::new(ActionInvocationDraft {
        request,
        principal: principal.clone(),
        actor: ObjId(7),
        offered_action: ActionOfferId(CanonicalHash::from_bytes([3; 32])),
        subject: PerceivedRef::SelfActor,
        targets: vec![],
        parameters: json!({}),
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal,
                owner: OwnerId::new("writer").unwrap(),
                namespace: CanonicalId::new("commit").unwrap(),
            },
            1,
            IdempotencyKey::Request(request),
        ),
    })
    .unwrap();
    (engine, handles, trace, invocation)
}
fn assert_before(engine: &Engine, handle: &Counter) {
    assert_eq!(engine.action_state().get(handle, &1).unwrap(), None);
    assert_eq!(engine.world.module_state::<Cell<1>>(), Some(&Cell(10)));
    assert_eq!(engine.world.module_state::<Cell<2>>(), Some(&Cell(20)));
    assert_eq!(engine.world.module_state::<Cell<3>>(), Some(&Cell(30)));
}

#[test]
fn last_of_three_owner_binds_refuses_without_any_ram_install_and_poison_is_retained() {
    let (mut engine, handles, trace, invocation) = fixture(true, true);
    let (prepared, _) = engine
        .prepare_registered_composed_authoritative_action(
            &handles.action,
            &invocation,
            CommitToken::from_bytes([9; 16]),
        )
        .unwrap();
    assert_before(&engine, &handles.counter);
    let (writes, installation) = prepared.into_parts();
    assert_eq!(
        writes.len(),
        4,
        "one typed and three registered owner images"
    );
    let mut out = crate::Outbox::new();
    let mut effects = crate::engine::Effects::new();
    assert!(matches!(
        engine.install_composed_action_announcing(installation, &mut out, &mut effects),
        Err(ComposedInstallError::Participant(ParticipantError::Slot))
    ));
    assert_before(&engine, &handles.counter);
    assert_eq!(
        *trace.lock().unwrap(),
        [(1, "bind"), (2, "bind"), (3, "bind")]
    );
    assert!(engine.action_state().reservation_lost());
    assert_eq!(engine.action_state().lost_installations(), 1);
    assert!(matches!(
        engine.action_transaction(&handles.counter),
        Err(ActionStateError::InstallationLost { .. })
    ));
}

#[test]
fn registered_three_owner_success_installs_exact_union_only_after_all_binds() {
    let (mut engine, handles, trace, invocation) = fixture(true, false);
    let (prepared, _) = engine
        .prepare_registered_composed_authoritative_action(
            &handles.action,
            &invocation,
            CommitToken::from_bytes([9; 16]),
        )
        .unwrap();
    assert_before(&engine, &handles.counter);
    prepared.recheck(&engine.world).unwrap();
    let before = prepared.authoritative_replay_writes();
    let (writes, installation) = prepared.into_parts();
    assert_eq!(
        serde_json::to_value(&writes).unwrap(),
        serde_json::to_value(before).unwrap()
    );
    assert_eq!(writes.len(), 4);
    for (namespace, key, after) in [
        ("legacy", "slot1", 11),
        ("legacy", "slot2", 22),
        ("writer", "slot3", 33),
    ] {
        assert_eq!(
            writes
                .iter()
                .filter(
                    |w| matches!(w, ReplayWrite::ModuleJsonSet { namespace: ns, key: k, value }
            if ns.as_str() == namespace && k.as_str() == key && value == &json!(after))
                )
                .count(),
            1
        );
    }
    engine
        .install_composed_action_announcing(
            installation,
            &mut crate::Outbox::new(),
            &mut crate::engine::Effects::new(),
        )
        .unwrap();
    assert_eq!(
        engine.action_state().get(&handles.counter, &1).unwrap(),
        Some(&7)
    );
    assert_eq!(engine.world.module_state::<Cell<1>>(), Some(&Cell(11)));
    assert_eq!(engine.world.module_state::<Cell<2>>(), Some(&Cell(22)));
    assert_eq!(engine.world.module_state::<Cell<3>>(), Some(&Cell(33)));
    assert_eq!(
        *trace.lock().unwrap(),
        [
            (1, "bind"),
            (2, "bind"),
            (3, "bind"),
            (1, "apply"),
            (2, "apply"),
            (3, "apply"),
            (4, "notice")
        ]
    );
    assert!(!engine.action_state().reservation_lost());
    assert!(
        engine.action_transaction(&handles.counter).is_ok(),
        "reservation settled last"
    );
}

#[test]
fn zero_owner_registered_path_preserves_legacy_typed_bytes_and_installation() {
    let (mut composed, ch, ct, ci) = fixture(false, false);
    let (mut legacy, lh, lt, li) = fixture(false, false);
    let token = CommitToken::from_bytes([9; 16]);
    let (prepared, _) = composed
        .prepare_registered_composed_authoritative_action(&ch.action, &ci, token)
        .unwrap();
    let (old, _) = legacy
        .prepare_registered_authoritative_action(&lh.action, &li, token)
        .unwrap();
    assert_eq!(
        serde_json::to_value(prepared.authoritative_replay_writes()).unwrap(),
        serde_json::to_value(old.authoritative_replay_writes()).unwrap()
    );
    let (_, installation) = prepared.into_parts();
    let (_, old_installation) = old.into_parts();
    composed.install_composed_action(installation).unwrap();
    legacy.install_prepared_action(old_installation).unwrap();
    assert_eq!(
        composed.action_state().get(&ch.counter, &1).unwrap(),
        Some(&7)
    );
    assert_eq!(
        legacy.action_state().get(&lh.counter, &1).unwrap(),
        Some(&7)
    );
    assert!(ct.lock().unwrap().is_empty());
    assert!(lt.lock().unwrap().is_empty());
    assert_eq!(composed.world.module_state::<Cell<3>>(), Some(&Cell(30)));
    assert!(!composed.action_state().reservation_lost());
    assert!(!legacy.action_state().reservation_lost());
}
