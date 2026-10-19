//! Public OOC checkout, entered only after the parent's genuine measured
//! certificate and actual acknowledged purchase Grant. No fabricated holding,
//! certificate, seat, seal or arena is used here. Like the parent, persistence
//! is an in-process installation boundary, not server writer/Unknown evidence.
use super::*;
#[path = "authored_fork_integration_tests.rs"]
mod fork_tests;
#[path = "personal_paid_source_tests.rs"]
mod source_tests;
use crate::arena::{Controller, MindSeal, Snapshot};
use crate::arena_bridge::{ArenaRegistry, ArenaTasks};
use crate::arena_task::{ArenaOut, MatchId};
use crate::match_::{self, MatchBook, MatchState};
use mush_core::{Audience, Inbound, WorldCommand};
use mush_proto::OutputEvent;

const BUYER_CONN: u64 = 401;
const OBSERVER_CONN: u64 = 402;
const WAIT: Duration = Duration::from_secs(30);

/// The actual module owns its state, commands and every boot hook. Registry
/// resources are the exact same frozen resources that measured this purchase.
/// In particular this wrapper does NOT register/replace Sim's AccountOracle.
struct PlayerTacsim {
    authored_forks: bool,
    marked_registry: bool,
    real: crate::TacsimRuleset,
    resources: Resources,
    shared: Shared,
}
impl GameModule for PlayerTacsim {
    fn name(&self) -> &'static str {
        "tacsim"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        self.real.register(reg)?;
        let (manifest, compatibility, factory) =
            self.resources.register_after_content(reg, true)?;
        if self.authored_forks {
            crate::minds::workshop::fork_material::register(reg, &manifest)?;
            crate::minds::workshop::authored_access::register(reg, &manifest)?;
            crate::minds::workshop::authored_access::probe::register(reg)?;
        }
        if self.marked_registry {
            crate::minds::bulk::registry_service::register_marked(reg, &manifest)?;
        }
        let mut shared = self.shared.lock().map_err(module_error)?;
        shared.manifest = Some(manifest);
        shared.compatibility = Some(compatibility);
        shared.factory = factory;
        Ok(())
    }
    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        self.real.on_world_loaded(ctx, data)
    }
    fn on_all_loaded(&mut self, ctx: &mut ModuleCtx<'_>) -> Result<(), ModError> {
        self.real.on_all_loaded(ctx)
    }
}

fn content() -> TestResult<Box<dyn GameModule>> {
    let mut appliers = mush_content::Appliers::new();
    mush_atlas::register_appliers(&mut appliers)?;
    mush_npc::content::register_appliers(&mut appliers)?;
    crate::register_appliers(&mut appliers)?;
    let mut packs = mush_atlas::starter::packs()?;
    packs.extend(crate::definitions::content::boot_packs()?);
    // Actual shipped OOC rooms, pod/tank objects, terrain and centre. The IC
    // station/lane packs are unrelated to this expressly OOC admission gate.
    Ok(Box::new(
        mush_content::ContentModule::new()
            .with_appliers(appliers)
            .with_boot_packs(packs),
    ))
}

struct PlayerWorld {
    engine: Engine,
    tasks: ArenaTasks,
    egress: tokio::sync::mpsc::Receiver<ArenaOut>,
    image: HashMap<SmolStr, ModuleBootData>,
    serial: u64,
    factory: Arc<Factory>,
    snapshots: BTreeMap<MatchId, Arc<Snapshot>>,
}
impl Drop for PlayerWorld {
    fn drop(&mut self) {
        self.tasks.signal_stop_all();
    }
}
impl PlayerWorld {
    fn boot(f: &PurchaseFixture, saved: &Durable) -> TestResult<Self> {
        Self::boot_forks(f, saved, false)
    }
    fn boot_forks(f: &PurchaseFixture, saved: &Durable, authored_forks: bool) -> TestResult<Self> {
        Self::boot_options(f, saved, authored_forks, false, false, false, false)
    }
    fn boot_marked(f: &PurchaseFixture, saved: &Durable) -> TestResult<Self> {
        Self::boot_options(f, saved, true, true, false, false, false)
    }
    fn boot_dependent(f: &PurchaseFixture, saved: &Durable) -> TestResult<Self> {
        Self::boot_options(f, saved, true, true, true, false, false)
    }
    fn boot_entitlements(f: &PurchaseFixture, saved: &Durable) -> TestResult<Self> {
        Self::boot_options(f, saved, true, true, true, true, false)
    }
    fn boot_renewals(f: &PurchaseFixture, saved: &Durable) -> TestResult<Self> {
        Self::boot_options(f, saved, true, true, true, true, true)
    }
    fn boot_options(
        f: &PurchaseFixture,
        saved: &Durable,
        authored_forks: bool,
        marked_registry: bool,
        dependent_owners: bool,
        entitlements: bool,
        renewals: bool,
    ) -> TestResult<Self> {
        let manifest = f
            .engine
            .registry
            .definitions()
            .ok_or("actual definitions")?
            .iter()
            .map(|entry| entry.reference())
            .find(|reference| {
                reference.key().owner.as_str() == "tacsim"
                    && reference.key().kind.as_str() == "registry_manifest"
                    && reference.key().id.as_str() == "combat"
                    && reference.version() == 1
            })
            .cloned()
            .ok_or("actual registered combat manifest")?;
        let shared: Shared = Arc::new(Mutex::new(Registered::default()));
        let purchase: PurchaseShared = Arc::new(Mutex::new(None));
        let mut cfg = f.resources.config();
        // Enable the real OOC layer and its lawful one-human/AI roster. Do not
        // change physics, class tables, metering, source or certificate policy.
        cfg.enabled = true;
        cfg.match_.ai_fill = true;
        let real = crate::TacsimRuleset::with_config(cfg);
        let real = if entitlements {
            real.with_mind_entitlement_holdings(manifest.clone())
        } else if dependent_owners {
            real.with_dependent_owner_holdings(manifest.clone())
        } else if f.source_forms {
            real.with_personal_filed_source_forms(manifest.clone())
        } else {
            real.with_personal_filed_holdings(manifest.clone())
        };
        let modules: Vec<Box<dyn GameModule>> = vec![
            Box::new(mush_org::OrgModule::new()),
            Box::new(mush_comsys::ComsysModule::new()),
            Box::new(mush_sim::SimulationModule::new().with_accounts(f.accounts.clone())),
            Box::new(mush_atlas::AtlasModule::new()),
            Box::new(mush_tactics::TacticsModule::with_catalog(
                crate::builtin_definitions()?.units,
            )),
            Box::new(mush_npc::NpcModule::new()),
            Box::new(PlayerTacsim {
                authored_forks,
                marked_registry,
                real,
                resources: f.resources.clone(),
                shared: shared.clone(),
            }),
            Box::new(PurchaseRoll {
                source_forms: f.source_forms,
                authored_forks,
                marked_registry,
                dependent_owners,
                entitlements,
                renewals,
                shared: shared.clone(),
                purchase: purchase.clone(),
                inner: None,
            }),
            Box::new(PurchaseEconomy {
                shared: shared.clone(),
                purchase: purchase.clone(),
                inner: None,
            }),
            Box::new(PurchaseMarket {
                purchase,
                inner: None,
            }),
            content()?,
        ];
        let mut engine = Engine::with_modules(saved.world()?, Registry::new(), modules)?;
        engine.set_account_identity(f.accounts.clone());
        load(&mut engine, &saved.disk)?;
        assert_eq!(
            shared.lock().map_err(module_error)?.manifest,
            Some(manifest)
        );
        let tasks = engine
            .world
            .module_state::<ArenaRegistry>()
            .ok_or("actual arena registry")?
            .tasks()
            .clone();
        let egress = tasks.take_egress().ok_or("unique actual arena egress")?;
        let mut image: HashMap<SmolStr, ModuleBootData> = HashMap::new();
        mush_testkit::fold_module_ops_into(
            &mut image,
            saved
                .disk
                .iter()
                .map(|((ns, key), value)| ModuleOp {
                    ns: ns.as_str().into(),
                    op: ModuleWrite::JsonSet {
                        key: key.as_str().into(),
                        value: value.clone(),
                    },
                })
                .collect(),
        );
        let factory = shared
            .lock()
            .map_err(module_error)?
            .factory
            .clone()
            .ok_or("actual Registry factory")?;
        let mut this = Self {
            engine,
            tasks,
            egress,
            image,
            serial: 0,
            factory,
            snapshots: BTreeMap::new(),
        };
        this.commit()?;
        let lobby = match_::centre_object(&this.engine.world, match_::LOBBY_ENTRY)
            .ok_or("shipped OOC centre")?;
        for (actor, conn) in [(BUYER, BUYER_CONN), (GOD, OBSERVER_CONN)] {
            this.engine.world.move_to(actor, lobby)?;
            this.step(Inbound::ConnUp {
                conn,
                actor,
                caps_structured: false,
                from_linkdead: false,
            })?;
        }
        assert_eq!(this.state()?, MatchState::Idle);
        Ok(this)
    }
    fn commit(&mut self) -> TestResult {
        // Preserve JSON, stream and ledger writes with the shared store-faithful
        // fold. Only after this modeled complete batch may egress advance its
        // committed ledger floor. No actual server transport claim is made.
        mush_testkit::fold_module_ops_into(&mut self.image, self.engine.take_module_ops());
        self.serial = self.serial.checked_add(1).ok_or("batch serial overflow")?;
        self.tasks.stamp_ledger_writes(self.serial);
        self.tasks.ack_ledger_upto(self.serial);
        Ok(())
    }
    fn step(&mut self, input: Inbound) -> TestResult<Vec<(Audience, OutputEvent)>> {
        let mut out = Outbox::new();
        self.engine
            .step(input, &mut out, &mut mush_core::engine::Effects::new());
        let events = out.take();
        // Inspect actual output variants, not a renderer which could omit source.
        for (_, event) in &events {
            let wire = serde_json::to_string(event)?;
            assert!(
                !wire.contains("this.landed"),
                "sealed source reached player output"
            );
            assert!(
                !wire.contains(SOURCE),
                "sealed source reached player output"
            );
        }
        self.commit()?;
        Ok(events)
    }
    fn say(&mut self, actor: ObjId, line: &str) -> TestResult<Vec<(Audience, OutputEvent)>> {
        self.step(Inbound::Cmd {
            conn: if actor == BUYER {
                BUYER_CONN
            } else {
                OBSERVER_CONN
            },
            actor,
            line: line.into(),
        })
    }
    fn private_text(events: &[(Audience, OutputEvent)], actor: ObjId) -> String {
        let conn = if actor == BUYER {
            BUYER_CONN
        } else {
            OBSERVER_CONN
        };
        events
            .iter()
            .filter(|(audience, _)| {
                matches!(audience, Audience::Actor(who) if *who == actor)
                    || matches!(audience, Audience::Conn(id) if *id == conn)
            })
            .map(|(_, event)| mush_testkit::text_of(event))
            .collect::<Vec<_>>()
            .join("\n")
    }
    fn state(&self) -> TestResult<MatchState> {
        Ok(match_::centre_of(&self.engine.world)
            .ok_or("actual centre")?
            .state)
    }
    fn select(&mut self) -> TestResult {
        let team = self.say(BUYER, "SIM TEAM red")?;
        assert!(Self::private_text(&team, BUYER).contains("red slot 1"));
        self.say(BUYER, "SIM AI 1")?;
        self.say(BUYER, "SIM UNIT heavy_mk1")?;
        let selected = self.say(BUYER, "SIM MIND filed:2 PROFILE Default")?;
        assert!(Self::private_text(&selected, BUYER).contains("Your slot will carry"));
        assert!(Self::private_text(&selected, BUYER).contains("filed:2"));
        // The choice itself is never broadcast, even to the author in the lobby.
        for (audience, event) in &selected {
            if mush_testkit::text_of(event).contains("Your slot will carry") {
                assert!(
                    matches!(audience, Audience::Actor(who) if *who == BUYER)
                        || matches!(audience, Audience::Conn(id) if *id == BUYER_CONN)
                );
            }
        }
        let centre = match_::centre_of(&self.engine.world).ok_or("selected centre")?;
        let own = centre
            .slots
            .iter()
            .find(|slot| slot.pilot == BUYER)
            .ok_or("claimed buyer slot")?;
        assert_eq!(
            own.mind
                .as_ref()
                .and_then(|mind| mind.filed)
                .map(|id| id.get()),
            Some(2)
        );
        assert_eq!(
            match_::lobby::lead_character_of(&self.engine.world, crate::arena::Team::Red),
            Some(BUYER)
        );
        assert_eq!(self.state()?, MatchState::Selection);
        Ok(())
    }
    fn start(&mut self) -> TestResult {
        let said = self.say(BUYER, "SIM START")?;
        assert_eq!(
            self.state()?,
            MatchState::Boarding,
            "{}",
            Self::private_text(&said, BUYER)
        );
        Ok(())
    }
    fn board(&mut self) -> TestResult<Vec<(Audience, OutputEvent)>> {
        let team_room = match_::centre_object(&self.engine.world, match_::RED_ROOM_ENTRY)
            .ok_or("shipped red room")?;
        // Only walking is arranged by the fixture. BOARD performs the actual
        // focus/seat activation, roster readiness, custody checkout and sealing.
        self.engine.world.move_to(BUYER, team_room)?;
        self.say(BUYER, "BOARD")
    }
    fn record(&self, id: MatchId) -> TestResult<&match_::MatchRecord> {
        self.engine
            .world
            .module_state::<MatchBook>()
            .and_then(|book| book.get(id))
            .ok_or_else(|| "actual checkout record".into())
    }
    fn pump(&mut self) -> TestResult {
        let mut count = 0;
        while count < crate::arena_bridge::MAX_EGRESS_PER_DRAIN {
            let message = match self.egress.try_recv() {
                Ok(message) => message,
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                Err(error) => return Err(error.into()),
            };
            if let ArenaOut::Snapshot { match_id, snapshot } = &message {
                self.snapshots.insert(*match_id, snapshot.clone());
            }
            assert!(self.tasks.stage(message), "bounded actual egress stage");
            count += 1;
        }
        if count > 0 {
            self.step(Inbound::Apply(WorldCommand::ModuleAsyncDone {
                module: crate::arena_bridge::MODULE.into(),
                actor: GOD,
                correlation: 0,
                payload: crate::arena_bridge::EGRESS_PAYLOAD.into(),
            }))?;
        }
        Ok(())
    }
    async fn read_sealed_mind(&mut self, name: &str) -> TestResult {
        let deadline = Instant::now() + WAIT;
        loop {
            self.pump()?;
            let events = self.say(BUYER, "MIND")?;
            let text = Self::private_text(&events, BUYER);
            if text.contains(name) {
                return Ok(());
            }
            assert!(text.contains(match_::cockpit::NO_FRAME_YET), "{text}");
            if Instant::now() >= deadline {
                return Err("cockpit never received its real sealed-mind projection".into());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    async fn progressed_snapshot(
        &mut self,
        id: MatchId,
        unit: ObjId,
        after_ms: i64,
        after_thinks: u64,
        after_landed: i64,
    ) -> TestResult<Arc<Snapshot>> {
        let deadline = Instant::now() + WAIT;
        let ground =
            crate::arena_bridge::ground_of(&self.engine.world, id).ok_or("actual frozen ground")?;
        let mut checked_ms = after_ms;
        loop {
            self.pump()?;
            if let Some(snapshot) = self
                .snapshots
                .get(&id)
                .filter(|snapshot| snapshot.at_ms > checked_ms)
            {
                checked_ms = snapshot.at_ms;
                let arena = Snapshot::decode(&snapshot.document, (*ground).clone())?;
                if arena
                    .units
                    .iter()
                    .any(|row| row.id == unit && row.controller == Controller::Mind)
                    && arena.minds.get(&unit).is_some_and(|mind| {
                        mind.think_seq > after_thinks
                            && landed_ms(&mind.memory).is_some_and(|landed| landed > after_landed)
                    })
                {
                    return Ok(snapshot.clone());
                }
            }
            if Instant::now() >= deadline {
                return Err(
                    "actual checked-out paid mind did not land a new answer within guard".into(),
                );
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

fn landed_ms(memory: &[u8]) -> Option<i64> {
    serde_json::from_slice::<Value>(memory)
        .ok()?
        .get("landed")?
        .as_i64()
}

fn assert_captured(
    world: &PlayerWorld,
    id: MatchId,
    unit: ObjId,
    seal: &MindSeal,
    snapshot: &Snapshot,
) -> TestResult<(u64, i64)> {
    let ground =
        crate::arena_bridge::ground_of(&world.engine.world, id).ok_or("actual frozen ground")?;
    let arena = Snapshot::decode(&snapshot.document, (*ground).clone())?;
    let actual = arena
        .units
        .iter()
        .find(|row| row.id == unit)
        .ok_or("actual checked-out unit")?;
    assert_eq!(actual.seal, seal.seal_ref());
    assert_eq!(actual.controller, Controller::Mind);
    let runtime = arena.minds.get(&unit).ok_or("actual running paid mind")?;
    assert_eq!(runtime.strikes, 0);
    assert!(runtime.dropped.is_none());
    assert!(runtime.think_seq > 0);
    assert_eq!(
        &world
            .record(id)?
            .minds
            .get(&unit)
            .ok_or("retained seal")?
            .seal,
        seal
    );
    Ok((
        runtime.think_seq,
        landed_ms(&runtime.memory).ok_or("actual source landing witness")?,
    ))
}

async fn live_and_expired(f: &PurchaseFixture, granted: &Durable, grace_end: i64) -> TestResult {
    let mut world = PlayerWorld::boot(f, granted)?;
    let tasks = world.tasks.clone();
    let outcome: TestResult = async {
        world.select()?;
        let term: PaidTermRecord =
            serde_json::from_value(one(f.rows("roll", "roll_mind_paid_term"))?)?;
        let mut refused_source = vec!["@mind/source Candidate@1"];
        if matches!(
            term.terms().form(),
            mush_roll::commerce::paid::PaidForm::Sealed {}
        ) {
            refused_source.push("@mind/source filed:2");
        }
        for line in refused_source {
            let refused = world.say(BUYER, line)?;
            let text = PlayerWorld::private_text(&refused, BUYER);
            assert!(!text.is_empty(), "source request must visibly refuse");
            assert!(text.contains("No such owned mind or revision."), "{text}");
        }
        world.start()?;
        let boarded = world.board()?;
        assert_eq!(
            world.state()?,
            MatchState::Running,
            "{}",
            PlayerWorld::private_text(&boarded, BUYER)
        );
        let id = match_::centre_of(&world.engine.world)
            .and_then(|centre| centre.match_id)
            .ok_or("running match")?;
        let record = world.record(id)?;
        let unit = record
            .teams
            .iter()
            .flat_map(|side| side.pilots.iter())
            .find_map(|(pilot, unit)| (*pilot == BUYER).then_some(*unit))
            .ok_or("actual buyer checkout")?;
        let seal = record
            .minds
            .get(&unit)
            .ok_or("actual checkout seal")?
            .seal
            .clone();
        assert_eq!(seal.origin.licence_label(), "2");
        assert_eq!(seal.engine, crate::arena::SealEngine::Scripted);
        let certificate = seal
            .requested
            .origin
            .certificate()
            .ok_or("actual filed certificate")?;
        let receipt: RatingReceipt =
            serde_json::from_value(one(f.rows("roll", "rating_receipts"))?)?;
        assert_eq!(certificate.rating_job, receipt.reference()?.job());
        assert_eq!(certificate.rating_id, receipt.reference()?.id().digest());
        assert_eq!(
            certificate.rating_digest,
            receipt.reference()?.receipt_digest()
        );
        assert!(seal.class >= certificate.minimum);
        // Public cockpit reads use this already checked-out seal. Expiry is
        // followed by EXIT, a causal arena input: every earlier snapshot still
        // has Pilot control. A queued pre-expiry snapshot therefore cannot
        // stand in for execution of the sealed mind after rights expire.
        world.read_sealed_mind(&seal.mind).await?;
        let deadline = Instant::now() + WAIT;
        while !world.snapshots.contains_key(&id) {
            world.pump()?;
            if Instant::now() >= deadline {
                return Err("actual opening snapshot did not arrive".into());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let initial = world.snapshots.get(&id).ok_or("actual opening snapshot")?;
        let ground =
            crate::arena_bridge::ground_of(&world.engine.world, id).ok_or("opening ground")?;
        let opening_arena = Snapshot::decode(&initial.document, (*ground).clone())?;
        let piloted = opening_arena
            .units
            .iter()
            .find(|row| row.id == unit)
            .ok_or("opening paid hull")?;
        assert_eq!(piloted.controller, Controller::Pilot);
        assert_eq!(piloted.seal, seal.seal_ref());
        world.engine.world.set_now(grace_end);
        world.read_sealed_mind(&seal.mind).await?;
        let before_exit = world
            .snapshots
            .get(&id)
            .map_or(-1, |snapshot| snapshot.at_ms);
        world.say(BUYER, "EXIT")?;
        let opening = world
            .progressed_snapshot(id, unit, before_exit, 0, -1)
            .await?;
        let (before_thinks, before_landed) = assert_captured(&world, id, unit, &seal, &opening)?;
        let later = world
            .progressed_snapshot(id, unit, opening.at_ms, before_thinks, before_landed)
            .await?;
        let (after_thinks, after_landed) = assert_captured(&world, id, unit, &seal, &later)?;
        assert!(
            after_thinks > before_thinks,
            "actual captured runner continues after own expiry"
        );
        assert!(after_landed > before_landed);
        assert_eq!(world.state()?, MatchState::Running);
        Ok(())
    }
    .await;
    tasks.shutdown(Duration::from_secs(5)).await;
    outcome
}

async fn fresh_refused(
    f: &PurchaseFixture,
    granted: &Durable,
    grace_end: i64,
    after_start: bool,
) -> TestResult {
    let mut world = PlayerWorld::boot(f, granted)?;
    let tasks = world.tasks.clone();
    let outcome: TestResult = async {
        world.select()?;
        if after_start {
            world.start()?;
        }
        world.engine.world.set_now(grace_end);
        let refused = if after_start {
            world.board()?
        } else {
            world.say(BUYER, "SIM START")?
        };
        assert!(
            PlayerWorld::private_text(&refused, BUYER)
                .contains("chosen mind is no longer held by its pilot"),
            "{}",
            PlayerWorld::private_text(&refused, BUYER)
        );
        assert_ne!(
            world.state()?,
            MatchState::Running,
            "{}",
            PlayerWorld::private_text(&refused, BUYER)
        );
        assert!(
            world.tasks.live_ids().is_empty(),
            "expired rights cannot launch a task"
        );
        if let Some(id) = match_::centre_of(&world.engine.world).and_then(|centre| centre.match_id)
        {
            assert!(
                world
                    .engine
                    .world
                    .module_state::<MatchBook>()
                    .and_then(|book| book.get(id))
                    .is_none_or(|record| record.minds.is_empty()),
                "failed checkout publishes no seal"
            );
        }
        assert!(world.snapshots.is_empty());
        if !after_start {
            let refused = world.say(BUYER, "SIM MIND filed:2 PROFILE Default")?;
            assert!(!PlayerWorld::private_text(&refused, BUYER).contains("Your slot will carry"));
            assert!(
                PlayerWorld::private_text(&refused, BUYER)
                    .contains("No such owned mind or revision.")
            );
        }
        Ok(())
    }
    .await;
    tasks.shutdown(Duration::from_secs(5)).await;
    outcome
}

pub(super) fn exercise(f: &PurchaseFixture, granted: &Durable) -> TestResult {
    let term: PaidTermRecord = serde_json::from_value(one(f.rows("roll", "roll_mind_paid_term"))?)?;
    assert_eq!(term.licence().get(), 2);
    assert_eq!(
        term.terms().form(),
        mush_roll::commerce::paid::PaidForm::Sealed {}
    );
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    runtime.block_on(async {
        live_and_expired(f, granted, term.window().grace_end().get()).await?;
        fresh_refused(f, granted, term.window().grace_end().get(), false).await?;
        fresh_refused(f, granted, term.window().grace_end().get(), true).await
    })
}

/// The same real module roster, with explicit v2 read grants when requested.
pub(super) fn exercise_source(f: &PurchaseFixture, saved: &Durable, permitted: bool) -> TestResult {
    source_tests::exercise(f, saved, permitted)?;
    if permitted {
        let term: PaidTermRecord =
            serde_json::from_value(one(f.rows("roll", "roll_mind_paid_term"))?)?;
        assert!(matches!(
            term.terms().form(),
            mush_roll::commerce::paid::PaidForm::Open {}
                | mush_roll::commerce::paid::PaidForm::OpenResale {}
        ));
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?;
        runtime.block_on(async {
            live_and_expired(f, saved, term.window().grace_end().get()).await?;
            fresh_refused(f, saved, term.window().grace_end().get(), false).await?;
            fresh_refused(f, saved, term.window().grace_end().get(), true).await
        })?;
    }
    Ok(())
}

/// The same genuine certificate/paid history, with an explicit fork opt-in.
pub(super) fn exercise_forks(f: &PurchaseFixture, saved: &Durable, permitted: bool) -> TestResult {
    fork_tests::exercise(f, saved, permitted)
}
