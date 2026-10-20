//! Runs only from the genuine purchase story after its actual measured
//! certificate and acknowledged E4 Grant. No structural certificate fixture.
use super::*;
use mush_core::{
    Inbound,
    read_service::{OwnerReadError, OwnerReadHandle},
};
use mush_roll::commerce::{
    LicenceId,
    paid::PaidForm,
    personal_filed_reader::{PersonalFiledHolding, PersonalFiledReader, PersonalFiledRequest},
};

type Answers = Arc<Mutex<Option<Result<PersonalFiledHolding, OwnerReadError>>>>;
struct Query {
    reader: OwnerReadHandle<PersonalFiledReader>,
    answers: Answers,
}
impl mush_core::module::QueryHandler for Query {
    fn query(
        &self,
        _: &World,
        _: ObjId,
        _: &Value,
    ) -> Result<mush_proto::OutputEvent, mush_proto::RichText> {
        Err(mush_proto::RichText::plain("actual read context required"))
    }
    fn query_with_context(
        &self,
        ctx: &mush_core::HookReadContext<'_>,
        _: ObjId,
        args: &Value,
    ) -> Result<mush_proto::OutputEvent, mush_proto::RichText> {
        let number = args
            .get("licence")
            .and_then(Value::as_u64)
            .and_then(|v| LicenceId::new(v).ok())
            .ok_or_else(|| mush_proto::RichText::plain("fixture licence"))?;
        let request = match args.get("holder").and_then(Value::as_u64) {
            Some(holder) => PersonalFiledRequest::admission(
                ObjId(
                    u32::try_from(holder)
                        .map_err(|_| mush_proto::RichText::plain("fixture holder"))?,
                ),
                number,
            ),
            None => PersonalFiledRequest::own(number),
        };
        *self
            .answers
            .lock()
            .map_err(|_| mush_proto::RichText::plain("fixture answer"))? =
            Some(ctx.read_owner(&self.reader, request));
        Ok(mush_proto::OutputEvent::Sys(mush_proto::RichText::plain(
            "observed",
        )))
    }
}

/// Adds a real registered read consumer around the existing actual source /
/// Registry participant fixture. It does not replace any owner load/final audit.
pub(super) struct TacsimReader(pub(super) Tacsim, pub(super) bool);
impl GameModule for TacsimReader {
    fn name(&self) -> &'static str {
        "tacsim"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        self.0.register(reg)?;
        let manifest = self
            .0
            .shared
            .lock()
            .map_err(module_error)?
            .manifest
            .clone()
            .ok_or_else(|| ModError::msg("actual manifest"))?;
        let reader = if self.1 {
            crate::match_::original_access::register_personal_source_forms(reg, &manifest)?
        } else {
            crate::match_::original_access::register_personal(reg, &manifest)?
        };
        let answers: Answers = Arc::new(Mutex::new(None));
        reg.state(answers.clone());
        reg.query_handler("paid-holding.test", Arc::new(Query { reader, answers }))
    }
    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        self.0.on_world_loaded(ctx, data)
    }
    fn on_all_loaded(&mut self, ctx: &mut ModuleCtx<'_>) -> Result<(), ModError> {
        self.0.on_all_loaded(ctx)
    }
}

pub(super) fn observe(
    f: &mut PurchaseFixture,
    actor: ObjId,
    licence: u64,
    holder: Option<ObjId>,
) -> TestResult<Result<PersonalFiledHolding, OwnerReadError>> {
    let before_disk = f.disk.clone();
    let before_money = f.balances()?;
    let before_rows = f.engine.action_state().revision();
    let before_time = f.engine.world.world_ic_now();
    f.engine.step(
        Inbound::Query {
            conn: 1,
            actor,
            id: 1,
            name: "paid-holding.test".into(),
            args: json!({"licence":licence,"holder":holder.map(|id|id.0)}),
        },
        &mut Outbox::new(),
        &mut mush_core::engine::Effects::new(),
    );
    let answer = f
        .engine
        .world
        .module_state::<Answers>()
        .ok_or("read answers")?
        .lock()
        .map_err(module_error)?
        .take()
        .ok_or("actual query answer")?;
    assert_eq!(f.disk, before_disk);
    assert_eq!(f.balances()?, before_money);
    assert_eq!(f.engine.action_state().revision(), before_rows);
    assert_eq!(f.engine.world.world_ic_now(), before_time);
    Ok(answer)
}

pub(super) fn own(
    f: &mut PurchaseFixture,
    actor: ObjId,
    licence: u64,
) -> TestResult<PersonalFiledHolding> {
    Ok(observe(f, actor, licence, None)??)
}

fn sealed_arena(
    f: &PurchaseFixture,
    holding: PersonalFiledHolding,
) -> TestResult<(crate::arena::Arena, crate::arena::MindSeal, Arc<str>)> {
    use crate::arena::{Arena, Controller, CustodyOut, Team, test_support};
    use crate::match_::filed_holdings::FiledExecutable;
    use mush_sim::data::Pose;
    let executable = FiledExecutable::resolve(
        holding,
        f.engine
            .world
            .module_state::<TacsimStore>()
            .ok_or("source owner")?,
    )?;
    let source = executable.source_for_execution();
    let held = executable.into_held();
    let unit_id = ObjId(41);
    let mut unit = test_support::tank(
        unit_id,
        Team::Red,
        Pose::at(2_000, 2_000, 0, test_support::rules()),
    );
    unit.unit = test_support::tactical_unit_cored(unit_id, held.rated);
    unit.controller = Controller::Mind;
    let mut terms = crate::match_::seal::SealTerms::fixture();
    terms.authored = held.definition.clone();
    terms.selected_profiles = held.presets.clone();
    terms.identity = held.identity.clone();
    terms.rated = held.rated;
    terms.engine_for_minds = held.engine;
    terms.principal = Some(crate::deliberation::order::CharacterRef::Character { id: BUYER });
    unit.reseal(&terms)?;
    let full_seal = unit.seal.clone();
    let cfg = crate::config::ArenaConfig::default();
    let mut blue_pose = Pose::at(6_000, 6_000, 0, test_support::rules());
    blue_pose.heading_mdeg = 180_000;
    let mut arena = Arena::new(
        CustodyOut {
            definition: test_support::definition(),
            units: vec![unit, test_support::tank(ObjId(42), Team::Blue, blue_pose)],
            pilots: vec![],
            resolution: test_support::resolution(),
            seed: 93,
            mode: crate::arena::MatchMode::Skirmish,
        },
        &cfg,
        &crate::config::MatchConfig::default(),
        &crate::config::SensorsConfig::default(),
    )?;
    crate::minds::install_sealed_runners(&mut arena, &cfg, &|digest| {
        (*digest == full_seal.digest()).then(|| source.to_string())
    })?;
    arena.resume_at(0);
    for wall in [250, 500, 750, 1000] {
        let out = arena.step_settled(wall, &[], &[]);
        assert_eq!(out.motor.mind_faults, 0);
    }
    assert!(arena.minds.get(&unit_id).ok_or("actual mind")?.think_seq > 0);
    Ok((arena, full_seal, source))
}

#[inline(never)]
fn inspect_paid_access(paid: &mut PurchaseFixture) -> TestResult<PersonalFiledHolding> {
    let holding = own(paid, BUYER, 2)?;
    assert_eq!(holding.actor(), BUYER);
    assert_eq!(holding.licence().get(), 2);
    assert_eq!(
        holding.paid_terms().ok_or("paid terms")?.form(),
        PaidForm::Sealed {}
    );
    assert_eq!(
        holding.rating(),
        serde_json::from_value::<RatingReceipt>(one(paid.rows("roll", "rating_receipts"))?)?
            .reference()?
    );
    let wire = serde_json::to_string(&holding)?;
    assert!(!wire.contains(SOURCE));
    assert!(!wire.contains("\"memory\""));
    assert!(!wire.contains("\"account\""));
    assert!(wire.len() < mush_core::read_service::MAX_OWNER_READ_RESULT_BYTES);
    assert!(
        own(paid, GOD, 1)?.paid_terms().is_none(),
        "original route still resolves"
    );
    assert!(matches!(
        observe(paid, GOD, 2, None)?,
        Err(OwnerReadError::Refused)
    ));
    assert!(matches!(
        observe(paid, BUYER, 1, None)?,
        Err(OwnerReadError::Refused)
    ));
    assert!(matches!(
        observe(paid, BUYER, u64::MAX, None)?,
        Err(OwnerReadError::Refused)
    ));
    assert_eq!(
        observe(paid, GOD, 2, Some(BUYER))??,
        holding,
        "admission uses the real separate roster subject, not caller title"
    );
    assert!(matches!(
        observe(paid, BUYER, 2, Some(GOD))?,
        Err(OwnerReadError::Refused)
    ));

    let alt = paid
        .engine
        .world
        .create(ObjKind::Player, "Same account alt", GOD, Some(ROOM))?;
    {
        let mut accounts = paid.accounts.values.lock().map_err(module_error)?;
        let buyer_account = *accounts.get(&BUYER).ok_or("buyer account")?;
        paid.accounts.epoch.invalidate();
        accounts.insert(alt, buyer_account);
    }
    assert!(matches!(
        observe(paid, alt, 2, None)?,
        Err(OwnerReadError::Refused)
    ));

    Ok(holding)
}

#[inline(never)]
fn inspect_frozen_execution(
    live: &mut PurchaseFixture,
    holding: &PersonalFiledHolding,
) -> TestResult {
    let (mut arena, full_seal, source) = sealed_arena(live, holding.clone())?;
    let captured = crate::arena::snapshot::Snapshot::encode(&arena)?;
    let captured_ref = arena.units.first().ok_or("first unit")?.seal.clone();
    let paid_term = one(live.rows("roll", "roll_mind_paid_term"))?;
    let dates = holding.window();
    live.engine.world.set_now(dates.issued_at().get() - 1);
    assert!(matches!(
        observe(live, BUYER, 2, None)?,
        Err(OwnerReadError::Refused)
    ));
    for now in [
        dates.issued_at().get(),
        dates.term_end().get(),
        dates.grace_end().get() - 1,
    ] {
        live.engine.world.set_now(now);
        assert_eq!(own(live, BUYER, 2)?, *holding);
        assert_eq!(one(live.rows("roll", "roll_mind_paid_term"))?, paid_term);
    }
    // Seller's original owner term starts 17 seconds before the buyer term.
    // Use that genuine gap: no synthetic term or derivative is invented.
    live.engine
        .world
        .set_now(1000 + TERM_SECONDS + GRACE_SECONDS);
    assert!(matches!(
        observe(live, GOD, 1, None)?,
        Err(OwnerReadError::Refused)
    ));
    assert_eq!(own(live, BUYER, 2)?, *holding);
    live.engine.world.set_now(dates.grace_end().get());
    assert!(matches!(
        observe(live, BUYER, 2, None)?,
        Err(OwnerReadError::Refused)
    ));
    assert_eq!(one(live.rows("roll", "roll_mind_paid_term"))?, paid_term);

    // World rights are absent from Arena/resume. The captured real paid seal
    // and actual scripted runner continue with exactly the same state/hash.
    let mut restored = crate::arena::snapshot::Snapshot::decode(
        &captured.document,
        crate::arena::test_support::definition(),
    )?;
    let cfg = crate::config::ArenaConfig::default();
    crate::minds::install_sealed_runners(&mut restored, &cfg, &|digest| {
        (*digest == full_seal.digest()).then(|| source.to_string())
    })?;
    assert_eq!(
        restored.units.first().ok_or("restored unit")?.seal,
        captured_ref
    );
    assert_eq!(crate::arena::state_hash(&restored), captured.state_hash);
    arena.resume_at(2_000);
    restored.resume_at(2_000);
    for wall in [2_250, 2_500, 2_750, 3_000] {
        assert_eq!(arena.step_settled(wall, &[], &[]).motor.mind_faults, 0);
        assert_eq!(restored.step_settled(wall, &[], &[]).motor.mind_faults, 0);
        assert_eq!(
            crate::arena::state_hash(&arena),
            crate::arena::state_hash(&restored)
        );
    }
    assert_eq!(
        restored.units.first().ok_or("restored unit")?.seal,
        captured_ref
    );

    Ok(())
}

/// The parent has already completed real Grant and Refund branches from this
/// same actual funded operation. All branches below replay that real history.
pub(super) fn exercise(
    f: &PurchaseFixture,
    funded: &Durable,
    granted: &Durable,
    refunded: &Durable,
) -> TestResult {
    let mut before = f.reboot(funded)?;
    assert!(matches!(
        observe(&mut before, BUYER, 2, None)?,
        Err(OwnerReadError::Refused)
    ));
    let mut refund = f.reboot(refunded)?;
    assert!(matches!(
        observe(&mut refund, BUYER, 2, None)?,
        Err(OwnerReadError::Refused)
    ));
    let mut paid = f.reboot(granted)?;
    let holding = inspect_paid_access(&mut paid)?;

    let mut live = f.reboot(granted)?;
    inspect_frozen_execution(&mut live, &holding)?;

    // Real paid history cannot back a fabricated withdrawal/recall cause.
    // Until those producers exist, the full audit must reject either edited
    // durable status instead of admitting it as a healthy live-read fixture.
    for field in ["withdrawn_by", "recalled_by"] {
        let mut status = granted.clone();
        change_one_family(&mut status, "roll", "roll_mind_status", |value| {
            *value.get_mut(field).ok_or("status field")? = json!(token(1_200_000));
            Ok(())
        })?;
        assert!(
            f.reboot(&status).is_err(),
            "a status field alone cannot supply a missing cause receipt"
        );
    }

    // Real signed native charter sale changes seller eligibility, not the
    // customer's already issued personal licence. No typed paid row is edited.
    let mut transferred = f.reboot(granted)?;
    let shell = holding.charter().object();
    transferred.transfer_charter(shell, BUYER, 49_001_000)?;
    assert_eq!(
        transferred
            .economy()?
            .snapshot()
            .asset(shell)
            .ok_or("charter title")?
            .title_owner,
        EconomicOwner::Object(BUYER)
    );
    assert!(matches!(
        observe(&mut transferred, GOD, 1, None)?,
        Err(OwnerReadError::Refused)
    ));
    assert_eq!(own(&mut transferred, BUYER, 2)?, holding);
    eprintln!(
        "M7_REAL_PAID_HOLDING own_term_issuer_independence_exact_rating_and_frozen_execution"
    );
    Ok(())
}
