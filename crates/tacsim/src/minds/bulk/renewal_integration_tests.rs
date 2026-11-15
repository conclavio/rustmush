//! Real D birth -> registered Market/Economy/Roll renewal. Every grant below
//! comes from the actual composed provider and owner installation. This is an
//! in-process ACK/replay fixture, not a server writer/Unknown transport test.
use super::*;
use mush_core::authoritative::ReplayWrite;
use mush_market::mind_renewal::{MindRenewalOperationV1, MindRenewalReceiptV1};
use mush_roll::commerce::{
    TermGeneration,
    renewal::{RenewalAttemptV1, RenewalHeadV1, RenewalOperationV1, RenewalStateV1},
};

use crate::minds::bulk::registry_integration_tests::original_purchase::paid_checkout::renewal_twins;
use mush_core::CommitToken;

#[path = "renewal_service_integration_tests.rs"]
pub(super) mod service_tests;

const START: u128 = 9_000_000;
const DRAFT: &str = "draft_mind_renewal";
const FUND: &str = "fund_mind_renewal";
const GRANT: &str = "settle_mind_renewal";
const REFUND: &str = "refund_mind_renewal";
const CLOSE: &str = "close_mind_renewal";
const ROLL_OP: &str = "roll_mind_renewal_operation";
const HEAD: &str = "roll_mind_renewal_head";
const ECONOMICS: &str = "roll_mind_paid_term_economics";

fn row<T: serde::de::DeserializeOwned>(
    world: &PlayerWorld,
    owner: &str,
    family: &str,
    field: &str,
    exact: Value,
) -> TestResult<T> {
    let mut matched = rows(&disk(world), owner, family)
        .into_iter()
        .filter(|value| value.get(field) == Some(&exact));
    let value = matched.next().ok_or("missing actual renewal row")?;
    check!(matched.next().is_none(), "renewal key is unique");
    Ok(serde_json::from_value(value)?)
}
fn licence(world: &PlayerWorld, id: LicenceId) -> TestResult<PersonalPaidLicence> {
    row(
        world,
        "roll",
        "roll_mind_personal_paid_licence",
        "id",
        json!(id),
    )
}
fn term(
    world: &PlayerWorld,
    id: LicenceId,
    generation: TermGeneration,
) -> TestResult<PaidTermRecord> {
    let mut found = rows(&disk(world), "roll", "roll_mind_paid_term")
        .into_iter()
        .filter(|v| {
            v.get("licence") == Some(&json!(id)) && v.get("generation") == Some(&json!(generation))
        });
    let actual = found.next().ok_or("actual retained generation")?;
    check!(found.next().is_none(), "term generation is unique");
    Ok(serde_json::from_value(actual)?)
}
fn economics(
    world: &PlayerWorld,
    id: LicenceId,
    generation: TermGeneration,
) -> TestResult<PaidTermEconomicsV1> {
    let mut found = rows(&disk(world), "roll", ECONOMICS)
        .into_iter()
        .filter(|v| {
            v.get("licence") == Some(&json!(id)) && v.get("generation") == Some(&json!(generation))
        });
    let actual = found.next().ok_or("actual retained economics")?;
    check!(found.next().is_none(), "economics generation is unique");
    Ok(serde_json::from_value(actual)?)
}
fn operation(world: &PlayerWorld, id: CommitToken) -> TestResult<RenewalOperationV1> {
    row(world, "roll", ROLL_OP, "operation", json!(id))
}
fn market_operation(world: &PlayerWorld, id: CommitToken) -> TestResult<MindRenewalOperationV1> {
    row(
        world,
        "market",
        MindRenewalOperationV1::FAMILY,
        "operation",
        json!(id),
    )
}
fn head(world: &PlayerWorld, id: LicenceId) -> TestResult<RenewalHeadV1> {
    row(world, "roll", HEAD, "licence", json!(id))
}
fn paired(world: &PlayerWorld, id: CommitToken) -> TestResult<RenewalOperationV1> {
    let actual = operation(world, id)?;
    let market = market_operation(world, id)?;
    market.matches_roll(&actual)?;
    let economy = world
        .engine
        .world
        .module_state::<EconomyStore>()
        .ok_or("real Economy")?;
    let audit = economy.renewal_audit()?;
    let proof = audit.get(id)?;
    actual.verify_history(&proof)?;
    market.verify_history(&proof)?;
    let receipts = rows(&disk(world), "market", MindRenewalReceiptV1::FAMILY);
    let receipts: Vec<_> = receipts
        .iter()
        .filter(|v| v.get("operation") == Some(&json!(id)))
        .collect();
    if actual.state().pending() {
        check!(receipts.is_empty(), "pending offer has no terminal receipt");
    } else {
        check_eq!(receipts.len(), 1);
        let receipt: MindRenewalReceiptV1 =
            serde_json::from_value((*receipts.first().ok_or("receipt")?).clone())?;
        receipt.verify_history(&market, &proof)?;
    }
    Ok(actual)
}
fn prepare_step(
    world: &mut PlayerWorld,
    id: &str,
    actor: ObjId,
    number: u128,
    params: Value,
) -> TestResult<(
    PreparedComposedTxn,
    mush_core::action::PreparedAuthoritativeAction,
)> {
    super::prepare(world, "market", id, 1, actor, number, params)
}
fn step(world: &mut PlayerWorld, id: &str, number: u128, params: Value) -> TestResult {
    super::commit(world, "market", id, 1, GOD, number, params)
}
fn step_for(world: &mut PlayerWorld, id: &str, number: u128, operation: CommitToken) -> TestResult {
    step(world, id, number, json!({"operation":operation}))
}
fn assert_disk(world: &PlayerWorld, before: &Disk) {
    assert!(
        disk(world) == *before,
        "pre-ACK or refused action changed retained images"
    );
}
fn typed_families(writes: &[ReplayWrite]) -> Vec<String> {
    let mut out: Vec<_> = writes
        .iter()
        .filter_map(|write| match write {
            ReplayWrite::ModuleJsonSet { value, .. } => value
                .pointer("/family/id")
                .and_then(Value::as_str)
                .map(str::to_owned),
            _ => None,
        })
        .collect();
    out.sort();
    out
}
fn assert_current_unchanged(
    world: &PlayerWorld,
    old: &PersonalPaidLicence,
    saved_term: &PaidTermRecord,
    saved_economics: &PaidTermEconomicsV1,
) -> TestResult {
    check_eq!(licence(world, old.id())?, *old);
    check_eq!(term(world, old.id(), old.term())?, *saved_term);
    check_eq!(economics(world, old.id(), old.term())?, *saved_economics);
    Ok(())
}
fn due_world(
    f: &PurchaseFixture,
    saved: &Durable,
    birth: &EntitlementAllocationV1,
) -> TestResult<PlayerWorld> {
    let mut world = Rig::personal(f, Compose::RENEWALS).boot(saved)?;
    let current = licence(&world, birth.licence())?;
    let predecessor = term(&world, current.id(), current.term())?;
    let due = predecessor.window().term_end().get();
    world
        .engine
        .world
        .set_now(world.engine.world.now().max(due));
    Ok(world)
}
#[inline(never)]
fn check_early_renewal(
    world: &mut PlayerWorld,
    birth: &EntitlementAllocationV1,
    old_term: &PaidTermRecord,
) -> TestResult {
    let before = disk(world);
    check!(world.engine.world.now() < old_term.window().term_end().get());
    check!(
        prepare_step(
            world,
            DRAFT,
            GOD,
            START + 89,
            json!({"licence":birth.licence().get()})
        )
        .is_err(),
        "early renewal cannot truncate a paid term"
    );
    world.engine.world.set_now(
        old_term
            .window()
            .term_end()
            .get()
            .checked_sub(1)
            .ok_or("due boundary")?,
    );
    check!(
        prepare_step(
            world,
            DRAFT,
            GOD,
            START + 89,
            json!({"licence":birth.licence().get()})
        )
        .is_err(),
        "one second before term_end remains too early"
    );
    assert_disk(world, &before);
    world
        .engine
        .world
        .set_now(old_term.window().term_end().get());
    Ok(())
}

fn grant_chain(
    f: &PurchaseFixture,
    granted: &Durable,
    birth: &EntitlementAllocationV1,
) -> TestResult<(Durable, Durable)> {
    let mut world = Rig::personal(f, Compose::RENEWALS).boot(granted)?;
    let old = licence(&world, birth.licence())?;
    let old_term = term(&world, old.id(), old.term())?;
    let old_economics = economics(&world, old.id(), old.term())?;
    check_early_renewal(&mut world, birth, &old_term)?;
    let old_balances = balances(&world)?;
    let before = disk(&world);
    let params = json!({"licence":birth.licence().get()});
    // No host-supplied price/payee/form and no other character's request.
    for (actor, bad) in [
        (BUYER, params.clone()),
        (GOD, json!({"licence":birth.licence().get(),"price":1})),
        (
            GOD,
            json!({"licence":birth.licence().get(),"previous_operation":token(START)}),
        ),
    ] {
        check!(prepare_step(&mut world, DRAFT, actor, START + 90, bad).is_err());
        assert_disk(&world, &before);
    }
    let (prepared, _) = prepare_step(&mut world, DRAFT, GOD, START, params)?;
    assert_disk(&world, &before);
    check_eq!(balances(&world)?, old_balances);
    assert_current_unchanged(&world, &old, &old_term, &old_economics)?;
    let (writes, install) = prepared.into_parts();
    apply(&mut world, &writes, install)?;
    let draft = paired(&world, token(START))?;
    check!(matches!(draft.state(), RenewalStateV1::Drafted {}));
    check_eq!(draft.attempt(), RenewalAttemptV1::Automatic {});
    check_eq!(draft.binding().birth_operation(), birth.operation());
    check_eq!(draft.binding().predecessor(), old.term());
    check_eq!(head(&world, old.id())?.pending(), Some(token(START)));
    check_eq!(balances(&world)?, old_balances);
    assert_current_unchanged(&world, &old, &old_term, &old_economics)?;
    source_answer(&mut world, GOD, old.id(), Some(birth.revision().source()))?;
    let drafted = capture(&world, &disk(&world));
    // Pending automatic/retry overlap must not replace the current head.
    for params in [
        json!({"licence":old.id().get()}),
        json!({"previous_operation":token(START)}),
    ] {
        let before = disk(&world);
        check!(prepare_step(&mut world, DRAFT, GOD, START + 91, params).is_err());
        assert_disk(&world, &before);
    }
    let before = disk(&world);
    let (prepared, _) = prepare_step(
        &mut world,
        FUND,
        GOD,
        START + 1,
        json!({"operation":token(START)}),
    )?;
    assert_disk(&world, &before);
    check_eq!(balances(&world)?, old_balances);
    let (writes, install) = prepared.into_parts();
    check_eq!(
        typed_families(&writes),
        vec![
            MindRenewalOperationV1::FAMILY.to_owned(),
            ROLL_OP.to_owned()
        ]
    );
    apply(&mut world, &writes, install)?;
    let funded = paired(&world, token(START))?;
    check!(matches!(funded.state(), RenewalStateV1::Funded { .. }));
    assert_current_unchanged(&world, &old, &old_term, &old_economics)?;
    source_answer(&mut world, GOD, old.id(), Some(birth.revision().source()))?;
    for (id, posted, held) in &old_balances {
        let all = balances(&world)?;
        let actual = all.iter().find(|v| v.0 == *id).ok_or("wallet")?;
        check_eq!(actual.1, *posted);
        check_eq!(
            actual.2,
            held.checked_add(Amount::from_minor(if id.0 == 0 { 73 } else { 0 }))
                .ok_or("held")?
        );
    }
    let funded_image = capture(&world, &disk(&world));
    // Cold funded recovery keeps the old pointer and escrow; no synthetic grant.
    let mut cold = Rig::personal(f, Compose::RENEWALS).boot(&funded_image)?;
    paired(&cold, token(START))?;
    assert_current_unchanged(&cold, &old, &old_term, &old_economics)?;
    cold.engine
        .world
        .set_now(cold.engine.world.now().checked_add(31).ok_or("time")?);
    let before = disk(&cold);
    let held = balances(&cold)?;
    let (prepared, _) = prepare_step(
        &mut cold,
        GRANT,
        GOD,
        START + 2,
        json!({"operation":token(START)}),
    )?;
    assert_disk(&cold, &before);
    check_eq!(balances(&cold)?, held);
    let (writes, install) = prepared.into_parts();
    apply(&mut cold, &writes, install)?;
    let renewed = paired(&cold, token(START))?;
    check!(renewed.state().granted());
    let current = licence(&cold, old.id())?;
    check_eq!(current.operation(), birth.operation());
    check_eq!(current.term().get(), old.term().get() + 1);
    let new_term = term(&cold, current.id(), current.term())?;
    let new_economics = economics(&cold, current.id(), current.term())?;
    check_eq!(new_term.operation(), token(START));
    check_eq!(new_economics.operation(), token(START));
    check_eq!(new_economics.grant(), old_economics.grant());
    check_eq!(
        new_economics.product_digest(),
        old_economics.product_digest()
    );
    check_eq!(term(&cold, old.id(), old.term())?, old_term);
    check_eq!(economics(&cold, old.id(), old.term())?, old_economics);
    check_eq!(new_term.window().issued_at().get(), cold.engine.world.now());
    check_eq!(head(&cold, old.id())?.current(), current.term());
    check_eq!(head(&cold, old.id())?.pending(), None);
    for (id, posted, held) in &old_balances {
        let all = balances(&cold)?;
        let actual = all.iter().find(|v| v.0 == *id).ok_or("wallet")?;
        let delta = match id.0 {
            0 => -73,
            1 => 7,
            2 => 66,
            _ => 0,
        };
        check_eq!(
            actual.1,
            posted
                .checked_add(Amount::from_minor(delta))
                .ok_or("payout")?
        );
        check_eq!(actual.2, *held);
    }
    let snapshot = capture(&cold, &disk(&cold));
    let replay = Rig::personal(f, Compose::RENEWALS).boot(&snapshot)?;
    paired(&replay, token(START))?;
    check_eq!(licence(&replay, old.id())?, current);
    check_eq!(balances(&replay)?, balances(&cold)?);
    check!(
        crate::test_checks::refused(Rig::personal(f, Compose::ENTITLEMENTS).boot(&snapshot))?,
        "disabled renewal composition rejects retained history"
    );
    Ok((snapshot, drafted))
}

fn cleanup_after_parent_lapse(
    f: &PurchaseFixture,
    granted: &Durable,
    birth: &EntitlementAllocationV1,
) -> TestResult {
    let parent: PaidTermRecord = serde_json::from_value(
        rows(&granted.disk, "roll", "roll_mind_paid_term")
            .into_iter()
            .find(|v| v.get("licence") != Some(&json!(birth.licence())))
            .ok_or("actual issuer parent")?,
    )?;
    for funded in [false, true] {
        let number = START + 350 + u128::from(funded) * 10;
        let mut world = due_world(f, granted, birth)?;
        let old = licence(&world, birth.licence())?;
        let own = term(&world, old.id(), old.term())?;
        let money = balances(&world)?;
        step(
            &mut world,
            DRAFT,
            number,
            json!({"licence":birth.licence().get()}),
        )?;
        if funded {
            step_for(&mut world, FUND, number + 1, token(number))?;
        }
        world
            .engine
            .world
            .set_now(parent.window().grace_end().get());
        check_eq!(
            own.window()
                .phase_at(mush_roll::commerce::WorldIcSeconds::from_stamp(
                    &world.engine.world.world_ic_now()
                )?),
            mush_roll::commerce::TermPhase::Grace
        );
        source_answer(
            &mut world,
            GOD,
            birth.licence(),
            Some(birth.revision().source()),
        )?;
        let lapsed = capture(&world, &disk(&world));
        let mut cold = Rig::personal(f, Compose::RENEWALS).boot(&lapsed)?;
        // Fresh work is unavailable, while captured custody can still close.
        // Both offer expiry and issuer lapse are present in this long-gap case.
        check!(
            prepare_step(
                &mut cold,
                DRAFT,
                GOD,
                number + 2,
                json!({"licence":birth.licence().get()})
            )
            .is_err()
        );
        check!(
            prepare_step(
                &mut cold,
                if funded { GRANT } else { FUND },
                GOD,
                number + 2,
                json!({"operation":token(number)})
            )
            .is_err()
        );
        if funded {
            step_for(&mut cold, REFUND, number + 3, token(number))?;
        } else {
            step(
                &mut cold,
                CLOSE,
                number + 3,
                json!({"operation":token(number),"reason":"expired"}),
            )?;
        }
        paired(&cold, token(number))?;
        check_eq!(balances(&cold)?, money);
        check_eq!(licence(&cold, old.id())?, old);
        check!(
            prepare_step(
                &mut cold,
                DRAFT,
                GOD,
                number + 4,
                json!({"previous_operation":token(number)})
            )
            .is_err(),
            "retry requires fresh issuer eligibility"
        );
        let ended = capture(&cold, &disk(&cold));
        let replay = Rig::personal(f, Compose::RENEWALS).boot(&ended)?;
        paired(&replay, token(number))?;
    }
    Ok(())
}

/// Doc 46 CT.8: the personal issuer's renewal twin, for the scenarios both issuers run.
struct Personal<'a> {
    f: &'a PurchaseFixture,
    granted: &'a Durable,
    birth: &'a EntitlementAllocationV1,
}
impl renewal_twins::Twin for Personal<'_> {
    const DRAFT: &'static str = DRAFT;
    const FUND: &'static str = FUND;
    const GRANT: &'static str = GRANT;
    const REFUND: &'static str = REFUND;
    const CLOSE: &'static str = CLOSE;
    const MARKET_OP: &'static str = MindRenewalOperationV1::FAMILY;
    const RECEIPT: &'static str = MindRenewalReceiptV1::FAMILY;
    const ROLL_OP: &'static str = ROLL_OP;
    const HEAD: &'static str = HEAD;
    const ECONOMICS: &'static str = ECONOMICS;
    const ECONOMY_OFFER: &'static str = "renewal-offer:";
    const LOGIN_PAIR: bool = true;
    fn licence(&self) -> LicenceId {
        self.birth.licence()
    }
    fn charter(&self) -> ObjId {
        self.birth.charter().object()
    }
    fn renewer(&self) -> ObjId {
        GOD
    }
    fn boot(&self, saved: &Durable) -> TestResult<PlayerWorld> {
        Rig::personal(self.f, Compose::RENEWALS).boot(saved)
    }
    fn due_world(&self) -> TestResult<PlayerWorld> {
        due_world(self.f, self.granted, self.birth)
    }
    fn set_time(&self, w: &mut PlayerWorld, at: i64) -> TestResult {
        w.engine.world.set_now(at);
        Ok(())
    }
    fn term_end(&self, w: &PlayerWorld) -> TestResult<i64> {
        let old = licence(w, self.birth.licence())?;
        Ok(term(w, old.id(), old.term())?.window().term_end().get())
    }
    fn disk(&self, w: &PlayerWorld) -> Disk {
        disk(w)
    }
    fn capture(&self, w: &PlayerWorld) -> Durable {
        capture(w, &disk(w))
    }
    fn balances(&self, w: &PlayerWorld) -> TestResult<Vec<(WalletId, Amount, Amount)>> {
        balances(w)
    }
    fn current(&self, w: &PlayerWorld) -> TestResult<Value> {
        let old = licence(w, self.birth.licence())?;
        Ok(json!([
            serde_json::to_value(&old)?,
            serde_json::to_value(term(w, old.id(), old.term())?)?,
            serde_json::to_value(economics(w, old.id(), old.term())?)?,
        ]))
    }
    fn term_at(&self, w: &PlayerWorld, generation: TermGeneration) -> TestResult<Value> {
        Ok(serde_json::to_value(term(w, self.birth.licence(), generation)?)?)
    }
    fn economics_at(&self, w: &PlayerWorld, generation: TermGeneration) -> TestResult<Value> {
        Ok(serde_json::to_value(economics(
            w,
            self.birth.licence(),
            generation,
        )?)?)
    }
    fn head(&self, w: &PlayerWorld) -> TestResult<(Option<CommitToken>, CommitToken)> {
        let head = head(w, self.birth.licence())?;
        Ok((head.pending(), head.last_operation()))
    }
    fn operation(&self, w: &PlayerWorld, n: u128) -> TestResult<renewal_twins::Operation> {
        let actual = paired(w, token(n))?;
        Ok(renewal_twins::Operation {
            pending: actual.state().pending(),
            retryable: actual.state().retryable(),
            attempt: actual.attempt(),
            next: actual.binding().next(),
            contract: serde_json::to_value(actual.contract())?,
            consent: serde_json::to_value(actual.consent_hash())?,
            binding: serde_json::to_value(actual.binding())?,
            expires_at: market_operation(w, token(n))?.offer().terms.expires_at.at,
        })
    }
    fn licence_operation(&self, w: &PlayerWorld) -> TestResult<CommitToken> {
        Ok(licence(w, self.birth.licence())?.operation())
    }
    fn birth_operation(&self) -> CommitToken {
        self.birth.operation()
    }
    fn title_owner(&self, w: &PlayerWorld) -> TestResult<Value> {
        Ok(serde_json::to_value(
            &w.engine
                .world
                .module_state::<EconomyStore>()
                .ok_or("Economy")?
                .snapshot()
                .asset(ObjId(self.birth.charter().get()))
                .ok_or("title")?
                .title_owner,
        )?)
    }
    fn prepare(
        &self,
        w: &mut PlayerWorld,
        id: &str,
        actor: ObjId,
        n: u128,
        params: Value,
    ) -> TestResult<renewal_twins::Prepared> {
        prepare_step(w, id, actor, n, params)
    }
    fn stage(
        &self,
        w: &mut PlayerWorld,
        id: &str,
        actor: ObjId,
        n: u128,
        params: Value,
        (prepared, output): renewal_twins::Prepared,
    ) -> TestResult {
        let definition = action(w, "market", id, 1)?;
        stage(w, definition, actor, n, params, prepared, output)
    }
    fn step(&self, w: &mut PlayerWorld, id: &str, n: u128, params: Value) -> TestResult {
        step(w, id, n, params)
    }
    fn merge_login_accounts(&self) -> TestResult<mush_core::AccountId> {
        merge_login_accounts(self.f)
    }
    fn restore_login_account(&self, seller: mush_core::AccountId) -> TestResult {
        restore_login_account(self.f, seller)
    }
}

async fn customer(
    f: &PurchaseFixture,
    granted: &Durable,
    owner: LicenceId,
    a: &EntitlementAllocationV1,
) -> TestResult {
    let parent: PaidTermRecord = serde_json::from_value(
        rows(&granted.disk, "roll", "roll_mind_paid_term")
            .into_iter()
            .find(|r| r.get("licence") != Some(&json!(a.licence())))
            .ok_or("parent")?,
    )?;
    let mut lapsed = granted.clone();
    lapsed.now = parent.window().grace_end().get();
    let mut active = Rig::personal(f, Compose::RENEWALS).boot(&lapsed)?;
    let current = licence(&active, a.licence())?;
    let own = term(&active, current.id(), current.term())?;
    assert_eq!(
        own.window()
            .phase_at(mush_roll::commerce::WorldIcSeconds::from_stamp(
                &active.engine.world.world_ic_now()
            )?),
        mush_roll::commerce::TermPhase::Active
    );
    source_answer(&mut active, GOD, a.licence(), Some(a.revision().source()))?;
    lapsed.now = own.window().term_end().get();
    assert!(lapsed.now >= parent.window().grace_end().get());
    drop(active);
    let mut world = Rig::personal(f, Compose::RENEWALS).boot(&lapsed)?;
    let tasks = world.tasks.clone();
    let result:TestResult=async {
        let current=licence(&world,a.licence())?;
        assert!(current.term().get()>1);
        let own=term(&world,a.licence(),current.term())?;
        assert_eq!(own.window().phase_at(mush_roll::commerce::WorldIcSeconds::from_stamp(&world.engine.world.world_ic_now())?),mush_roll::commerce::TermPhase::Grace);
        source_answer(&mut world,GOD,a.licence(),Some(a.revision().source()))?;
        assert!(prepare_step(&mut world,DRAFT,GOD,START+500,json!({"licence":a.licence().get()})).is_err(),"new renewal still needs current issuer parent");
        world.engine.world.move_to(BUYER,ROOM)?;world.commit()?;
        assert!(super::prepare(&mut world,"market","list_derived_mind",1,BUYER,START+501,listing_params(owner,"open_resale",100)).is_err(),"new sale still needs current issuer parent");
        source_answer(&mut world,BUYER,a.licence(),None)?;
        source_answer(&mut world,BUYER,owner,None)?;
        assert!(rows(&disk(&world),"roll","roll_mind_personal_paid_licence").iter().filter_map(|r|serde_json::from_value::<PersonalPaidLicence>(r.clone()).ok())
            .all(|licence|licence.holder()!=mush_economy::Holder::Character(GOD) || licence.id()==a.licence()),"customer bought no ancestor licence");
        // New sortie under the customer's own Grace still works after the
        // issuer's selected parent has ended; no ancestor is re-resolved.
        world.say(GOD,"SIM TEAM red")?;world.say(GOD,"SIM AI 1")?;world.say(GOD,"SIM UNIT heavy_mk1")?;
        let selected=world.say(GOD,&format!("SIM MIND filed:{} PROFILE Default",a.licence().get()))?;
        assert!(PlayerWorld::private_text(&selected,GOD).contains("Your slot will carry"));
        world.say(GOD,"SIM START")?;assert_eq!(world.state()?,match_::MatchState::Boarding);
        // The lead character is read during Boarding (`lead_character_of`).
        assert_eq!(match_::lobby::lead_character_of(&world.engine.world,crate::arena::PartyId::RED),Some(GOD));
        let room=match_::centre_object(&world.engine.world,match_::RED_ROOM_ENTRY).ok_or("red room")?;
        world.engine.world.move_to(GOD,room)?;world.say(GOD,"BOARD")?;assert_eq!(world.state()?,match_::MatchState::Running);
        let id=match_::centre_of(&world.engine.world).and_then(|c|c.match_id).ok_or("match")?;
        let record=world.record(id)?;let unit=record.teams.iter().flat_map(|t|t.pilots.iter()).find_map(|(p,u)|(*p==GOD).then_some(*u)).ok_or("customer unit")?;
        let seal=record.minds.get(&unit).ok_or("actual customer seal")?.seal.clone();
        let certificate=seal.requested.origin.certificate().ok_or("filed certificate")?;
        assert_eq!(certificate.rating_job,a.rating().job());assert_eq!(certificate.rating_id,a.rating().id().digest());
        assert_eq!(certificate.rating_digest,a.rating().receipt_digest());
        let deadline=Instant::now()+WAIT;while !world.snapshots.contains_key(&id){world.pump()?;if Instant::now()>=deadline{return Err("opening snapshot".into());}tokio::time::sleep(Duration::from_millis(10)).await;}
        let opening=world.snapshots.get(&id).ok_or("opening")?;
        let ground=crate::arena_bridge::ground_of(&world.engine.world,id).ok_or("ground")?;
        let arena=crate::arena::Snapshot::decode(&opening.document,(*ground).clone())?;
        assert_eq!(arena.units.iter().find(|u|u.id==unit).ok_or("pilot")?.controller,crate::arena::Controller::Pilot);
        // This causal public EXIT occurs only after own expiry. Future thinks
        // therefore cannot be a cached pre-expiry snapshot masquerading as use.
        world.engine.world.set_now(own.window().grace_end().get());source_answer(&mut world,GOD,a.licence(),None)?;
        let anchor=world.snapshots.get(&id).map_or(-1,|s|s.at_ms);world.say(GOD,"EXIT")?;
        let first=world.progressed_snapshot(id,unit,anchor,0,-1).await?;
        let (thinks,landed)=crate::minds::bulk::registry_integration_tests::original_purchase::paid_checkout::assert_captured(&world,id,unit,&seal,&first)?;
        let second=world.progressed_snapshot(id,unit,first.at_ms,thinks,landed).await?;
        crate::minds::bulk::registry_integration_tests::original_purchase::paid_checkout::assert_captured(&world,id,unit,&seal,&second)?;
        let mut ended=granted.clone();ended.now=own.window().grace_end().get();
        let mut fresh=Rig::personal(f, Compose::RENEWALS).boot(&ended)?;source_answer(&mut fresh,GOD,a.licence(),None)?;
        fresh.say(GOD,"SIM TEAM red")?;let refused=fresh.say(GOD,&format!("SIM MIND filed:{} PROFILE Default",a.licence().get()))?;
        assert!(!PlayerWorld::private_text(&refused,GOD).contains("Your slot will carry"));
        assert!(match_::centre_of(&fresh.engine.world).is_some_and(|c|c.match_id.is_none()));
        fresh.tasks.shutdown(Duration::from_secs(5)).await;Ok(())
    }.await;
    tasks.shutdown(Duration::from_secs(5)).await;
    result
}

pub(super) async fn exercise(
    f: &PurchaseFixture,
    granted: &Durable,
    owner: LicenceId,
    birth: &EntitlementAllocationV1,
) -> TestResult {
    eprintln!("M7_STORY phase=renewal.transitions event=begin");
    service_tests::exercise(f, granted, birth)?;
    let (renewed, _drafted) = grant_chain(f, granted, birth)?;
    let twin = Personal { f, granted, birth };
    renewal_twins::retries(&twin, START)?;
    renewal_twins::title_fences(&twin, START)?;
    renewal_twins::accounts_and_expiry(&twin, START)?;
    cleanup_after_parent_lapse(f, granted, birth)?;
    renewal_twins::corruptions(&twin, &renewed, START)?;
    story_phase("renewal.customer", || customer(f, &renewed, owner, birth))?.await?;
    eprintln!("M7_STORY phase=renewal.transitions event=complete");
    Ok(())
}
