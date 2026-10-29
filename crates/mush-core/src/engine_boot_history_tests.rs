#![cfg(test)]

use super::*;
use std::sync::Mutex;

type Events = Arc<Mutex<Vec<(&'static str, &'static str)>>>;
struct Probe {
    name: &'static str,
    events: Events,
    refuse: bool,
}
impl Probe {
    fn record(&self, phase: &'static str) -> Result<(), ModError> {
        self.events
            .lock()
            .map_err(|e| ModError::msg(e.to_string()))?
            .push((self.name, phase));
        Ok(())
    }
}
impl GameModule for Probe {
    fn name(&self) -> &'static str {
        self.name
    }
    fn register(&mut self, _: &mut crate::Registrar<'_>) -> Result<(), ModError> {
        Ok(())
    }
    fn audit_typed_history(&self, ctx: &crate::ModuleCtx<'_>) -> Result<(), ModError> {
        if ctx.module.as_deref() != Some(self.name)
            || ctx.typed_state.module_owner().map(crate::OwnerId::as_str) != Some(self.name)
            || ctx.actor != GOD
        {
            return Err(ModError::msg("wrong audit identity"));
        }
        self.record("audit")?;
        if self.refuse {
            return Err(ModError::msg("history refused"));
        }
        Ok(())
    }
    fn audit_boot_history(
        &self,
        ctx: &crate::ModuleCtx<'_>,
        data: &ModuleBootData,
    ) -> Result<(), ModError> {
        if ctx.module.as_deref() != Some(self.name)
            || ctx.typed_state.module_owner().map(crate::OwnerId::as_str) != Some(self.name)
            || data.kv.iter().any(|(key, _)| key.starts_with("log:"))
        {
            return Err(ModError::msg(
                "wrong native audit context or unnormalized data",
            ));
        }
        self.record("native")?;
        if data.kv.iter().any(|(key, _)| key == "refuse_native") {
            return Err(ModError::msg("native history refused"));
        }
        Ok(())
    }
    fn on_world_loaded(
        &mut self,
        _: &mut crate::ModuleCtx<'_>,
        _: ModuleBootData,
    ) -> Result<(), ModError> {
        self.record("load")
    }
    fn on_all_loaded(&mut self, _: &mut crate::ModuleCtx<'_>) -> Result<(), ModError> {
        self.record("final")
    }
}
fn engine(events: &Events, refuse: bool) -> Box<Engine> {
    Box::new(
        Engine::with_modules(
            World::new_minimal(),
            Registry::new(),
            vec![
                Box::new(Probe {
                    name: "first",
                    events: events.clone(),
                    refuse: false,
                }),
                Box::new(Probe {
                    name: "second",
                    events: events.clone(),
                    refuse,
                }),
            ],
        )
        .expect("framework probes"),
    )
}

#[test]
fn every_read_only_owner_audit_precedes_the_first_load_callback() {
    let events = Events::default();
    let mut engine = engine(&events, false);
    engine.hydrate_action_state([]).unwrap();
    engine
        .modules_loaded(HashMap::new(), &mut Outbox::new(), &mut Effects::new())
        .unwrap();
    assert_eq!(
        *events.lock().unwrap(),
        vec![
            ("first", "audit"),
            ("second", "audit"),
            ("first", "native"),
            ("second", "native"),
            ("first", "load"),
            ("second", "load"),
            ("first", "final"),
            ("second", "final"),
        ]
    );
}

#[test]
fn a_later_owner_refusal_prevents_every_load_callback() {
    let events = Events::default();
    let mut engine = engine(&events, true);
    engine.hydrate_action_state([]).unwrap();
    let mut out = Outbox::new();
    assert!(
        engine
            .modules_loaded(HashMap::new(), &mut out, &mut Effects::new())
            .is_err()
    );
    assert_eq!(
        *events.lock().unwrap(),
        vec![("first", "audit"), ("second", "audit")]
    );
    assert!(out.is_empty());
    assert!(engine.take_module_ops().is_empty());
}

#[test]
fn later_native_refusal_prevents_every_load_and_normalizes_legacy_ledger_input() {
    let events = Events::default();
    let mut engine = engine(&events, false);
    let mut boot = HashMap::new();
    boot.insert(
        "second".into(),
        ModuleBootData {
            kv: vec![
                (
                    "log:test:0".into(),
                    serde_json::json!({"chunk":0,"sealed":false,"entries":[]}),
                ),
                ("refuse_native".into(), serde_json::Value::Bool(true)),
            ],
            ..Default::default()
        },
    );
    assert!(
        engine
            .modules_loaded(boot, &mut Outbox::new(), &mut Effects::new())
            .is_err()
    );
    assert_eq!(
        *events.lock().unwrap(),
        vec![
            ("first", "audit"),
            ("second", "audit"),
            ("first", "native"),
            ("second", "native"),
        ]
    );
    assert!(engine.take_module_ops().is_empty());
}
