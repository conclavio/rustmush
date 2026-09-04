use mush_core::{
    CanonicalId, ClockProvider, ClockProviderContext, ClockProviderFailure, ClockProviderKind,
    ClockReadError, ClockRef, Engine, GameModule, ModError, OwnerId, Registrar, Registry,
    SceneClockProvider, World,
};

#[derive(Clone, Debug)]
struct TestClockState {
    at: i64,
    generation: u64,
}

struct TestClockProvider;

impl ClockProvider for TestClockProvider {
    fn now(
        &self,
        context: &ClockProviderContext<'_>,
        generation: u64,
    ) -> Result<i64, ClockProviderFailure> {
        let state = context
            .module_state::<TestClockState>()
            .ok_or(ClockProviderFailure::Integrity)?;
        if generation != state.generation {
            return Err(ClockProviderFailure::StaleGeneration {
                requested: generation,
                current: state.generation,
            });
        }
        Ok(state.at)
    }
}

struct TestSceneClockProvider;

impl SceneClockProvider for TestSceneClockProvider {
    fn now(
        &self,
        context: &ClockProviderContext<'_>,
        scene: u64,
        generation: u64,
    ) -> Result<i64, ClockProviderFailure> {
        if scene != 77 {
            return Err(ClockProviderFailure::Integrity);
        }
        TestClockProvider.now(context, generation)
    }
}

struct TestClockModule {
    duplicate: bool,
}

impl GameModule for TestClockModule {
    fn name(&self) -> &'static str {
        "clock_test"
    }

    fn register(&mut self, registrar: &mut Registrar<'_>) -> Result<(), ModError> {
        registrar.state(TestClockState {
            at: 41,
            generation: 7,
        });
        registrar.scene_clock_provider(TestSceneClockProvider)?;
        let id = CanonicalId::new("initiative").expect("canonical clock id");
        registrar.clock_provider(ClockProviderKind::Registered, id.clone(), TestClockProvider)?;
        if self.duplicate {
            registrar.clock_provider(ClockProviderKind::Registered, id, TestClockProvider)?;
        }
        Ok(())
    }
}

fn registered_clock(generation: u64) -> ClockRef {
    ClockRef::Registered {
        owner: OwnerId::new("clock_test").expect("canonical owner"),
        id: CanonicalId::new("initiative").expect("canonical clock id"),
        generation,
    }
}

#[test]
fn engine_reads_built_in_and_registered_clocks_without_relabelling() {
    let mut world = World::new_minimal();
    world.set_now(1_234);
    let engine = Engine::with_modules(
        world,
        Registry::new(),
        vec![Box::new(TestClockModule { duplicate: false })],
    )
    .expect("compose clock module");

    let utc = engine.clock_now(&ClockRef::Utc).expect("UTC read");
    assert_eq!(utc.clock, ClockRef::Utc);
    assert_eq!(utc.at, 1_234);

    let world_ic = engine
        .clock_now(&ClockRef::WorldIc { generation: 1 })
        .expect("current world-IC read");
    assert_eq!(world_ic.clock, ClockRef::WorldIc { generation: 1 });
    assert_eq!(world_ic.at, 1_234);

    let registered = engine
        .clock_now(&registered_clock(7))
        .expect("registered clock read");
    assert_eq!(registered.clock, registered_clock(7));
    assert_eq!(registered.at, 41);

    let scene = engine
        .clock_now(&ClockRef::SceneTurn {
            scene: 77,
            generation: 7,
        })
        .expect("scene clock read");
    assert_eq!(
        scene.clock,
        ClockRef::SceneTurn {
            scene: 77,
            generation: 7,
        }
    );
    assert_eq!(scene.at, 41);

    let providers = engine
        .registry
        .clock_providers()
        .expect("frozen clock registry");
    assert_eq!(providers.len(), 2);
    assert!(!providers.fingerprint().to_hex().is_empty());
}

#[test]
fn mutable_clock_generation_mismatches_fail_closed() {
    let engine = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![Box::new(TestClockModule { duplicate: false })],
    )
    .expect("compose clock module");

    assert_eq!(
        engine.clock_now(&ClockRef::WorldIc { generation: 2 }),
        Err(ClockReadError::StaleGeneration {
            requested: 2,
            current: 1,
        })
    );

    assert!(matches!(
        engine.clock_now(&registered_clock(6)),
        Err(ClockReadError::Provider {
            source: ClockProviderFailure::StaleGeneration {
                requested: 6,
                current: 7,
            },
            ..
        })
    ));

    assert!(matches!(
        engine.clock_now(&ClockRef::SceneTurn {
            scene: 77,
            generation: 6,
        }),
        Err(ClockReadError::SceneProvider {
            source: ClockProviderFailure::StaleGeneration {
                requested: 6,
                current: 7,
            },
            ..
        })
    ));

    assert!(matches!(
        engine.clock_now(&registered_clock(0)),
        Err(ClockReadError::ZeroGeneration { .. })
    ));
}

struct SceneOnlyModule(&'static str);

impl GameModule for SceneOnlyModule {
    fn name(&self) -> &'static str {
        self.0
    }

    fn register(&mut self, registrar: &mut Registrar<'_>) -> Result<(), ModError> {
        registrar
            .scene_clock_provider(|_context: &ClockProviderContext<'_>, _scene, _generation| Ok(0))
    }
}

#[test]
fn scene_clock_has_one_unambiguous_composition_owner() {
    let error = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![
            Box::new(SceneOnlyModule("scene_one")),
            Box::new(SceneOnlyModule("scene_two")),
        ],
    )
    .err()
    .expect("a second scene clock provider must fail composition");
    assert!(error.to_string().contains("already owned"));

    let engine = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![Box::new(SceneOnlyModule("scene_one"))],
    )
    .expect("one scene provider composes");
    assert!(matches!(
        engine.clock_now(&ClockRef::SceneTurn {
            scene: 1,
            generation: 0,
        }),
        Err(ClockReadError::ZeroGeneration { .. })
    ));
}

#[test]
fn duplicate_clock_provider_identity_fails_composition() {
    let error = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![Box::new(TestClockModule { duplicate: true })],
    )
    .err()
    .expect("duplicate registration must fail");

    assert!(error.to_string().contains("registered more than once"));
}
