#![cfg(test)]

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::action_state::{StateFamilyDescriptor, StateFamilyHandle, StateFamilyId};
use crate::mechanics::CanonicalId;
use crate::{Engine, GameModule, ModError, ModuleCtx, Outbox, Registrar, Registry, World};

type Events = Arc<Mutex<Vec<(&'static str, &'static str)>>>;
type Handle = StateFamilyHandle<u64, u64>;
#[derive(Clone, Debug, Default)]
struct Ready<const N: u8>(bool);
struct Probe<const N: u8> {
    events: Events,
    own: Arc<Mutex<Option<Handle>>>,
    foreign: Arc<Mutex<Option<Handle>>>,
    load_error: bool,
    final_error: bool,
}
impl<const N: u8> GameModule for Probe<N> {
    fn name(&self) -> &'static str {
        if N == 1 { "first" } else { "second" }
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.state(Ready::<N>::default());
        let handle = reg.state_family(
            StateFamilyDescriptor::new(
                StateFamilyId::new("owned").unwrap(),
                CanonicalId::new("owned.key.v1").unwrap(),
                CanonicalId::new("owned.value.v1").unwrap(),
                1,
                CanonicalId::new("owned.decode.v1").unwrap(),
                CanonicalId::new("owned.validate.v1").unwrap(),
                CanonicalId::new("owned.encode.v1").unwrap(),
            )
            .unwrap(),
            |key: &u64| Ok(key.to_string()),
            |value: &Value| value.as_u64().ok_or_else(|| "expected u64".into()),
            |_: &u64| Ok(()),
            |value: &u64| Ok(json!(value)),
        )?;
        *self.own.lock().unwrap() = Some(handle);
        Ok(())
    }
    fn on_all_loaded(&mut self, _: &mut ModuleCtx<'_>) -> Result<(), ModError> {
        self.events.lock().unwrap().push((self.name(), "loaded"));
        if self.load_error {
            Err(ModError::msg("load refused"))
        } else {
            Ok(())
        }
    }
    fn finalize_owner_composition(&mut self, ctx: &mut ModuleCtx<'_>) -> Result<(), ModError> {
        let mut events = self.events.lock().unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|(_, phase)| *phase == "loaded")
                .count(),
            2,
            "every ordinary owner reconciliation precedes every final audit"
        );
        events.push((self.name(), "final"));
        drop(events);
        assert!(!ctx.state::<Ready<N>>().unwrap().0);
        assert!(
            ctx.action_state(self.own.lock().unwrap().as_ref().unwrap(), &1)
                .unwrap()
                .is_none()
        );
        assert!(
            ctx.action_state(self.foreign.lock().unwrap().as_ref().unwrap(), &1)
                .is_err(),
            "finalization retains the original owner-stamped typed lens"
        );
        if self.final_error {
            return Err(ModError::msg("final audit refused"));
        }
        ctx.state_mut::<Ready<N>>().unwrap().0 = true;
        Ok(())
    }
}
fn fixture(reverse: bool, load_error: bool, final_error: bool) -> (Engine, Events) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let first = Arc::new(Mutex::new(None));
    let second = Arc::new(Mutex::new(None));
    let mut modules: Vec<Box<dyn GameModule>> = vec![
        Box::new(Probe::<1> {
            events: Arc::clone(&events),
            own: Arc::clone(&first),
            foreign: Arc::clone(&second),
            load_error,
            final_error,
        }),
        Box::new(Probe::<2> {
            events: Arc::clone(&events),
            own: second,
            foreign: first,
            load_error: false,
            final_error: false,
        }),
    ];
    if reverse {
        modules.reverse();
    }
    (
        Engine::with_modules(World::new_minimal(), Registry::new(), modules).unwrap(),
        events,
    )
}

#[test]
fn final_audits_follow_all_reconciliation_in_either_roster_order_with_owner_lenses() {
    for reverse in [false, true] {
        let (mut engine, events) = fixture(reverse, false, false);
        engine
            .all_modules_loaded(&mut Outbox::default(), &mut crate::engine::Effects::new())
            .unwrap();
        let order = if reverse {
            ["second", "first"]
        } else {
            ["first", "second"]
        };
        let expected: Vec<_> = ["loaded", "final"]
            .into_iter()
            .flat_map(|phase| order.into_iter().map(move |owner| (owner, phase)))
            .collect();
        assert_eq!(*events.lock().unwrap(), expected);
        assert!(engine.world.module_state::<Ready<1>>().unwrap().0);
        assert!(engine.world.module_state::<Ready<2>>().unwrap().0);
    }
}

#[test]
fn failed_reconciliation_never_runs_a_final_audit_or_opens_readiness() {
    for reverse in [false, true] {
        let (mut engine, events) = fixture(reverse, true, false);
        let error = engine
            .all_modules_loaded(&mut Outbox::default(), &mut crate::engine::Effects::new())
            .unwrap_err();
        assert!(error.to_string().contains("load refused"));
        assert!(
            events
                .lock()
                .unwrap()
                .iter()
                .all(|(_, phase)| *phase == "loaded")
        );
        assert!(!engine.world.module_state::<Ready<1>>().unwrap().0);
        assert!(!engine.world.module_state::<Ready<2>>().unwrap().0);
    }
}

#[test]
fn failed_final_audit_stops_later_hooks_without_opening_their_readiness() {
    let (mut engine, events) = fixture(false, false, true);
    let error = engine
        .all_modules_loaded(&mut Outbox::default(), &mut crate::engine::Effects::new())
        .unwrap_err();
    assert!(error.to_string().contains("final audit refused"));
    assert_eq!(
        *events.lock().unwrap(),
        [
            ("first", "loaded"),
            ("second", "loaded"),
            ("first", "final")
        ]
    );
    assert!(!engine.world.module_state::<Ready<1>>().unwrap().0);
    assert!(!engine.world.module_state::<Ready<2>>().unwrap().0);
}
