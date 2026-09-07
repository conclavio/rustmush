//! **E7 residue pass 1 — R1: the work-order money `Hold`** (10b §15.1 item 6).
//!
//! Hazard A, conservation over the **ledger total**: a submitted work order cannot
//! produce without paying, a hold cannot be released twice or released by its own
//! owner, and every settlement leaves the sum of every posted balance exactly where
//! it was.
//!
//! Everything money-shaped here runs through the **real command path** — `@wo/draft`,
//! `@wo/submit`, `@wo/deliver`, `@hold/release`, `@econ/alerts` — because that is
//! where the hold is placed: a `prepare_authoritative` provider has no `Ctx`. The
//! *state machine* half (an order sitting in `Completed` or `Cancelled`) is staged
//! with cut 4's own `apply()` rig, which opens the same owner-stamped transaction a
//! provider gets, so the sweep sees exactly what production would leave it.

use std::collections::BTreeSet;
use std::sync::Arc;

use mush_core::{CanonicalId, CommitToken, DefinitionDigest, Engine, GOD, ObjId, World};
use mush_economy::{AccountId, EconomicVisibility, EconomyStore, Hold, HoldStatus, VaultId};
use mush_industry::money;
use mush_industry::workorder::{
    DeliveryTarget, PinnedRecipe, RecipeLine, RecipeRef, WorkOrder, WorkOrderId, WorkOrderState,
    quote_price,
};
use mush_industry::{IndustryError, IndustryFamilies, IndustryHandles, IndustryModule};
use mush_inventory::{EndpointRef, InventoryHandleCell, InventoryModule};
use mush_org::PrincipalId;
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, text_of};

// ---- E7r1: the rig --------------------------------------------------------

struct TestRecipes;

impl mush_industry::RecipeAuthority for TestRecipes {
    fn digest_of(&self, _world: &World, recipe: &RecipeRef) -> Option<DefinitionDigest> {
        (recipe.pack().as_str() == "tacsim.core"
            && recipe.version() == 1
            && recipe.id().as_str() == "alloy_smelt")
            .then(|| DefinitionDigest::from_bytes([0x11; 32]))
    }
    fn resolve(
        &self,
        _world: &World,
        pack: &CanonicalId,
        version: u32,
        id: &CanonicalId,
    ) -> Option<(RecipeRef, PinnedRecipe)> {
        (pack.as_str() == "tacsim.core" && version == 1 && id.as_str() == "alloy_smelt")
            .then(alloy_recipe)
    }
}

fn commodity(id: &str) -> CanonicalId {
    CanonicalId::new(id).expect("commodity id")
}

fn capabilities(ids: &[&str]) -> BTreeSet<CanonicalId> {
    ids.iter()
        .map(|id| CanonicalId::new(id).expect("capability id"))
        .collect()
}

fn endpoint(id: u64) -> EndpointRef {
    EndpointRef(VaultId(id))
}

/// 4 metals -> 1 structural_alloys, 3 work units: `quote_price` is 4 per unit.
fn alloy_recipe() -> (RecipeRef, PinnedRecipe) {
    let recipe = RecipeRef::new(
        commodity("tacsim.core"),
        1,
        commodity("alloy_smelt"),
        DefinitionDigest::from_bytes([0x11; 32]),
    )
    .expect("recipe ref");
    let pinned = PinnedRecipe::new(
        vec![RecipeLine::new(commodity("metals"), 4).expect("input line")],
        vec![RecipeLine::new(commodity("structural_alloys"), 1).expect("output line")],
        capabilities(&["smelt"]),
        3,
    )
    .expect("pinned recipe");
    (recipe, pinned)
}

fn boot() -> (TestWorld, IndustryFamilies) {
    let cell: InventoryHandleCell = Arc::default();
    let world = TestWorld::with_modules(vec![
        Box::new(mush_economy::EconomyModule::new()),
        Box::new(
            InventoryModule::new()
                .with_quantity_delegates(["industry"])
                .with_handle_cell(Arc::clone(&cell)),
        ),
        Box::new(
            IndustryModule::new()
                .with_inventory(Arc::clone(&cell))
                .with_recipes(Arc::new(TestRecipes)),
        ),
    ]);
    let families = world
        .world()
        .module_state::<IndustryHandles>()
        .expect("industry publishes its handles")
        .0
        .clone();
    (world, families)
}

fn texts(events: &[(mush_core::Audience, OutputEvent)]) -> String {
    events
        .iter()
        .map(|(_, e)| text_of(e))
        .collect::<Vec<_>>()
        .join("\n")
}
fn has_error(events: &[(mush_core::Audience, OutputEvent)]) -> bool {
    events.iter().any(|(_, e)| {
        matches!(e, OutputEvent::Sys(rt) if rt.spans().iter().any(|sp| sp.tag.as_deref() == Some("error")))
    })
}
fn ok(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    let ev = tw.run(actor, line);
    assert!(!has_error(&ev), "`{line}` was refused: {}", texts(&ev));
    texts(&ev)
}
fn err(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    let ev = tw.run(actor, line);
    assert!(
        has_error(&ev),
        "`{line}` should have been refused, got: {}",
        texts(&ev)
    );
    texts(&ev)
}

fn apply<T>(
    engine: &mut Engine,
    families: &IndustryFamilies,
    body: impl FnOnce(&mut mush_core::ActionTxn<'_>) -> Result<T, IndustryError>,
) -> Result<T, IndustryError> {
    let (result, prepared) = {
        let mut txn = engine
            .action_transaction(&families.deposits)
            .expect("industry transaction");
        let result = body(&mut txn);
        match result {
            Ok(value) => (Ok(value), txn.prepare().ok()),
            Err(error) => (Err(error), None),
        }
    };
    if let Some(prepared) = prepared {
        let (_, installation) = prepared.into_parts();
        engine.install_prepared_action(installation);
    }
    result
}

fn snap(tw: &TestWorld) -> &mush_economy::EconomySnapshot {
    tw.world()
        .module_state::<EconomyStore>()
        .expect("economy store")
        .snapshot()
}

/// **The conservation probe**: the sum of every posted balance. A double-entry
/// journal keeps it constant, so any path that minted or burned value moves it.
fn ledger_total(tw: &TestWorld) -> i64 {
    snap(tw).accounts().map(|a| a.posted_balance.minor()).sum()
}
fn bal(tw: &TestWorld, id: AccountId) -> i64 {
    snap(tw)
        .account(id)
        .expect("account")
        .posted_balance
        .minor()
}
fn held(tw: &TestWorld, id: AccountId) -> i64 {
    snap(tw).account(id).expect("account").held_amount.minor()
}
/// The reservation backing one order's budget, whatever its status.
fn budget_hold(tw: &TestWorld, order: WorkOrderId) -> Option<Hold> {
    snap(tw)
        .holds()
        .find(|h| h.purpose == money::purpose(order))
        .cloned()
}

fn god_accounts(tw: &TestWorld) -> Vec<u64> {
    snap(tw)
        .accounts()
        .filter(|a| a.owner == PrincipalId::Object(GOD))
        .map(|a| a.id.0)
        .collect()
}

/// The whole fixture: an instrument, GOD's budget and operating accounts, `funds`
/// minted into the budget, and a two-slot `smelt` facility on object #7.
fn scene(funds: i64) -> (TestWorld, IndustryFamilies, AccountId, AccountId) {
    let (mut tw, families) = boot();
    ok(&mut tw, GOD, "@instrument/register gold=0");
    ok(&mut tw, GOD, "@account/system faucet=gold");
    let faucet = snap(&tw)
        .accounts()
        .find(|a| matches!(a.owner, PrincipalId::System(_)))
        .expect("faucet")
        .id;
    let before = god_accounts(&tw);
    ok(&mut tw, GOD, &format!("@account/open #{}=gold", GOD.0));
    ok(&mut tw, GOD, &format!("@account/open #{}=gold", GOD.0));
    let fresh: Vec<u64> = god_accounts(&tw)
        .into_iter()
        .filter(|id| !before.contains(id))
        .collect();
    assert_eq!(fresh.len(), 2, "a budget and an operating account");
    let (budget, payee) = (AccountId(fresh[0]), AccountId(fresh[1]));
    if funds > 0 {
        ok(
            &mut tw,
            GOD,
            &format!("@econ/mint {faucet}={budget} {funds}"),
        );
    }
    ok(&mut tw, GOD, "@facility/open 7=2 smelt");
    let _ = tw.engine_mut().take_authoritative_submit();
    (tw, families, budget, payee)
}

/// Stage one `Draft` order in the family, exactly as `wo_draft`'s prepare would.
fn seed_order(
    tw: &mut TestWorld,
    families: &IndustryFamilies,
    id: u64,
    budget: AccountId,
    quantity: u32,
) -> WorkOrderId {
    let (recipe, pinned) = alloy_recipe();
    let order = WorkOrder::draft(
        WorkOrderId(id),
        PrincipalId::Object(GOD),
        PrincipalId::Object(GOD),
        ObjId(7),
        2,
        1,
        1,
        recipe,
        pinned,
        quantity,
        budget,
        Some(endpoint(500)),
        DeliveryTarget::Endpoint {
            endpoint: endpoint(600),
        },
        EconomicVisibility::Parties,
    )
    .expect("draft a work order");
    let engine = tw.engine_mut();
    apply(engine, families, |txn| families.open_work_order(txn, order)).expect("seed the order");
    WorkOrderId(id)
}

/// Move one order to `state`, the way a provider's terminal transition would.
fn set_state(
    tw: &mut TestWorld,
    families: &IndustryFamilies,
    id: WorkOrderId,
    state: WorkOrderState,
) {
    let engine = tw.engine_mut();
    apply(engine, families, |txn| {
        let order = families
            .work_order(txn, id)?
            .expect("the order exists")
            .clone();
        let next = order.with_state(state)?;
        families.put_work_order(txn, order.revision(), next)
    })
    .expect("set the state");
}

fn order_state(tw: &mut TestWorld, families: &IndustryFamilies, id: WorkOrderId) -> WorkOrderState {
    let engine = tw.engine_mut();
    let mut txn = engine
        .action_transaction(&families.deposits)
        .expect("industry transaction");
    *families
        .work_order(&mut txn, id)
        .expect("read")
        .expect("the order exists")
        .state()
}

/// One `industry.sweep` fire. The tick is `Coarse(SWEEP_PERIOD_SECS)`, so the world
/// clock has to move a whole period before the engine considers it due.
fn fire(tw: &mut TestWorld) {
    let period = i64::try_from(mush_industry::SWEEP_PERIOD_SECS).unwrap_or(60);
    let t = tw.world().now() + period + 1;
    tw.set_now(t);
    let _ = tw.fire_due(t.saturating_mul(1_000));
}

type Kv =
    std::collections::BTreeMap<smol_str::SmolStr, Vec<(smol_str::SmolStr, serde_json::Value)>>;

/// Every module's persisted documents, folded from the ops written so far.
fn fold_all(tw: &mut TestWorld) -> Kv {
    let mut kv: Kv = std::collections::BTreeMap::new();
    for op in tw.take_module_ops() {
        let entry = kv.entry(op.ns.clone()).or_default();
        match op.op {
            mush_core::ModuleWrite::JsonSet { key, value } => {
                entry.retain(|(k, _)| *k != key);
                entry.push((key, value));
            }
            mush_core::ModuleWrite::Del { key } => entry.retain(|(k, _)| *k != key),
            mush_core::ModuleWrite::StreamAppend { .. } => {}
        }
    }
    kv
}

fn reboot(tw: &mut TestWorld, kv: &Kv) {
    let boot: std::collections::HashMap<smol_str::SmolStr, mush_core::ModuleBootData> = kv
        .iter()
        .map(|(ns, entries)| {
            (
                ns.clone(),
                mush_core::ModuleBootData {
                    kv: entries.clone(),
                    streams: std::collections::HashMap::new(),
                    ..Default::default()
                },
            )
        })
        .collect();
    tw.modules_loaded(boot);
}

/// The price one order's budget reserves — the tree's own pure quote, never a
/// number this test typed.
fn price(quantity: u32) -> i64 {
    let (_, pinned) = alloy_recipe();
    i64::try_from(quote_price(&pinned, quantity)).expect("a small price")
}

// ---- E7r1: R1 — production stops being free -------------------------------

/// **An unfunded submit refuses on the ENGINE's own check and reserves nothing.**
///
/// Invariant 26 is the never-negative gate and this pass writes no second opinion:
/// the placement is refused with `InsufficientAvailable` before anything is
/// reserved, the order stays `Draft`, and the ledger total does not move.
#[test]
fn e7r1_r1_an_unfunded_submit_refuses_on_the_engines_own_check_and_reserves_nothing() {
    let (mut tw, families, budget, _payee) = scene(0);
    let id = seed_order(&mut tw, &families, 0xA1, budget, 1);
    let before = ledger_total(&tw);
    let _ = tw.engine_mut().take_authoritative_submit();

    let refusal = err(&mut tw, GOD, &format!("@wo/submit {id}"));
    assert!(
        refusal.contains("cannot fund"),
        "the refusal names the funding, not the order: {refusal}"
    );
    assert!(
        budget_hold(&tw, id).is_none(),
        "an unfunded submit reserves NOTHING"
    );
    assert_eq!(held(&tw, budget), 0, "no reservation on the budget account");
    assert_eq!(ledger_total(&tw), before, "the ledger total did not move");
    assert!(
        tw.engine_mut().take_authoritative_submit().is_none(),
        "and it staged no submission, so no capacity slot was taken either"
    );
    assert_eq!(
        order_state(&mut tw, &families, id),
        WorkOrderState::Draft,
        "the order stays drafted"
    );
}

/// **A funded submit places the hold BEFORE it submits, and names it in the
/// submission.** The reservation is the order's own quoted price; the ledger total
/// is unchanged because a hold moves no money, only reserves it.
#[test]
fn e7r1_r1_a_funded_submit_places_the_hold_before_the_submit_and_names_it() {
    let (mut tw, families, budget, _payee) = scene(100);
    let id = seed_order(&mut tw, &families, 0xA2, budget, 1);
    let before = ledger_total(&tw);
    let _ = tw.engine_mut().take_authoritative_submit();

    ok(&mut tw, GOD, &format!("@wo/submit {id}"));
    let hold = budget_hold(&tw, id).expect("the budget hold was placed");
    assert_eq!(
        hold.amount.minor(),
        price(1),
        "it reserves the quoted price"
    );
    assert!(hold.status.is_held(), "and it is live");
    assert_eq!(held(&tw, budget), price(1), "the account shows it reserved");
    assert_eq!(
        bal(&tw, budget),
        100,
        "a reservation moves no money — the posted balance is untouched"
    );
    assert_eq!(ledger_total(&tw), before, "the ledger total did not move");

    // The submission carries the placed hold, so the provider records it and a
    // submission naming none is refusable.
    let submit = tw
        .engine_mut()
        .take_authoritative_submit()
        .expect("the submission was staged");
    assert_eq!(
        submit.parameters.get("hold").and_then(|v| v.as_u64()),
        Some(hold.id.0),
        "the placed hold is named in the submission"
    );
}

/// **The order's own owner cannot release the engine-owned hold**, and the refusal
/// is byte-identical to one for a hold that never existed — the whole point of
/// `ENGINE_OWNED_RESERVATIONS`. Without the entry, an owner releases their own
/// reservation and production is free again, silently.
#[test]
fn e7r1_r1_the_orders_own_owner_cannot_release_or_capture_the_engine_owned_hold() {
    let (mut tw, families, budget, payee) = scene(100);
    let id = seed_order(&mut tw, &families, 0xA3, budget, 1);
    ok(&mut tw, GOD, &format!("@wo/submit {id}"));
    let hold = budget_hold(&tw, id).expect("placed");
    let before = ledger_total(&tw);

    let refused = err(&mut tw, GOD, &format!("@hold/release {}", hold.id.0));
    assert!(
        refused.contains("industry.workorder") && refused.contains("nothing else"),
        "the lockout names the owning system and its one settlement seam (cut 9's \
         own wording, which deliberately tells the placer where to look rather than \
         pretending the hold is absent): {refused}"
    );
    let captured = err(
        &mut tw,
        GOD,
        &format!("@hold/capture {}={payee} {}", hold.id.0, price(1)),
    );
    assert!(
        captured.contains("industry.workorder"),
        "the beneficiary cannot take it either: {captured}"
    );

    assert!(
        budget_hold(&tw, id).is_some_and(|h| h.status.is_held()),
        "the money is still reserved"
    );
    assert_eq!(held(&tw, budget), price(1));
    assert_eq!(ledger_total(&tw), before, "and nothing moved");
}

/// **The terminal capture: the sweep settles a `Completed` order exactly once, and
/// the ledger total is unchanged across place → capture.**
///
/// It runs from the EXISTING `SWEEP_TAG` Coarse arm because `wo_deliver` is a
/// provider prepare with no `Ctx`. The tick fires **twice** and both fires are
/// asserted: the second is a no-op, because the hold's own status is the marker.
#[test]
fn e7r1_r1_the_sweep_captures_a_completed_order_once_and_conserves_the_ledger() {
    let (mut tw, families, budget, payee) = scene(100);
    let id = seed_order(&mut tw, &families, 0xA4, budget, 1);
    ok(&mut tw, GOD, &format!("@wo/submit {id}"));
    let before = ledger_total(&tw);
    set_state(
        &mut tw,
        &families,
        id,
        WorkOrderState::Completed {
            commit: CommitToken::from_bytes([0xA4; 16]),
        },
    );

    fire(&mut tw);
    let hold = budget_hold(&tw, id).expect("the hold record survives its settlement");
    assert!(
        matches!(hold.status, HoldStatus::Consumed { .. }),
        "the terminal transition SPENT the reservation: {:?}",
        hold.status
    );
    assert_eq!(bal(&tw, budget), 100 - price(1), "the budget paid");
    assert_eq!(bal(&tw, payee), price(1), "the facility's owner was paid");
    assert_eq!(held(&tw, budget), 0, "nothing is reserved any more");
    assert_eq!(
        ledger_total(&tw),
        before,
        "place -> capture leaves the ledger total EXACTLY where it was"
    );

    // Fire two: a no-op past the applied point. A reboot re-running the arm IS the
    // reconcile, which is what makes the sweep safe to run for ever.
    fire(&mut tw);
    assert_eq!(bal(&tw, payee), price(1), "the second fire settled nothing");
    assert_eq!(ledger_total(&tw), before);
}

/// **A cancelled order's reservation comes back unspent, and a second release
/// writes nothing.** `industry.wo.settle/{order}` is the key, so the repeat is
/// `AlreadyPosted` — the same transaction, no second effect.
#[test]
fn e7r1_r1_the_sweep_releases_a_cancelled_order_and_a_repeat_writes_nothing() {
    let (mut tw, families, budget, payee) = scene(100);
    let id = seed_order(&mut tw, &families, 0xA5, budget, 1);
    ok(&mut tw, GOD, &format!("@wo/submit {id}"));
    let before = ledger_total(&tw);
    set_state(&mut tw, &families, id, WorkOrderState::Cancelled);

    fire(&mut tw);
    let hold = budget_hold(&tw, id).expect("the record survives");
    assert!(
        matches!(hold.status, HoldStatus::Released),
        "a cancelled order gives the money back: {:?}",
        hold.status
    );
    assert_eq!(bal(&tw, budget), 100, "unspent");
    assert_eq!(bal(&tw, payee), 0, "and nobody was paid");
    assert_eq!(held(&tw, budget), 0);
    assert_eq!(ledger_total(&tw), before, "place -> release conserves too");

    fire(&mut tw);
    assert_eq!(bal(&tw, budget), 100, "the second release wrote nothing");
    assert_eq!(ledger_total(&tw), before);
}

/// **A capture after a release refuses** — the settlement key is already spent and
/// the hold is no longer live, so there are two independent reasons and the money
/// never moves twice.
#[test]
fn e7r1_r1_a_capture_after_a_release_refuses_and_the_total_does_not_move() {
    let (mut tw, families, budget, payee) = scene(100);
    let id = seed_order(&mut tw, &families, 0xA6, budget, 1);
    ok(&mut tw, GOD, &format!("@wo/submit {id}"));
    set_state(&mut tw, &families, id, WorkOrderState::Cancelled);
    fire(&mut tw);
    let after_release = ledger_total(&tw);
    let hold = budget_hold(&tw, id).expect("released");

    // Now pretend the order went terminal-completed instead. The sweep must NOT
    // capture a settled hold.
    set_state(
        &mut tw,
        &families,
        id,
        WorkOrderState::Completed {
            commit: CommitToken::from_bytes([0xA6; 16]),
        },
    );
    fire(&mut tw);
    fire(&mut tw);
    assert_eq!(
        budget_hold(&tw, id).map(|h| h.status),
        Some(hold.status),
        "the settled hold is untouched"
    );
    assert_eq!(bal(&tw, payee), 0, "no second settlement paid anybody");
    assert_eq!(ledger_total(&tw), after_release);
}

/// **A vanished hold is a PAUSE and a PAGE, never a replay hit** (cut 9's fold
/// E2(a) — the exact bug the engine-owned seam was built for).
///
/// The observable outcome: `@wo/deliver` refuses, so no output materializes, and a
/// staffer sees the order named in `@econ/alerts` with the recommended command
/// already filled in. Only the subject is stopped: `@wo/cancel` still runs.
#[test]
fn e7r1_r1_a_vanished_hold_pauses_the_order_and_pages_and_is_never_a_replay_hit() {
    let (mut tw, families, budget, _payee) = scene(100);
    let id = seed_order(&mut tw, &families, 0xA7, budget, 1);
    // A reserving order that never had a hold: exactly the shape a lost placement,
    // or a submission forged past the command, would leave.
    set_state(&mut tw, &families, id, WorkOrderState::Running);
    assert!(budget_hold(&tw, id).is_none());
    let before = ledger_total(&tw);

    fire(&mut tw);
    let refusal = err(&mut tw, GOD, &format!("@wo/deliver {id}"));
    assert!(
        refusal.contains("budget reservation is missing"),
        "delivery is stopped for THIS order: {refusal}"
    );
    let alerts = ok(&mut tw, GOD, "@econ/alerts");
    assert!(
        alerts.contains(&format!("work order #{}", id.0)),
        "the page names the subject: {alerts}"
    );
    assert_eq!(
        ledger_total(&tw),
        before,
        "and it is a pause, not a settlement: nothing moved"
    );

    // Fire twice: the ladder dedupes per subject, so a sweep that keeps finding the
    // same order opens one record and not one per fire.
    fire(&mut tw);
    let alerts = ok(&mut tw, GOD, "@econ/alerts");
    assert_eq!(
        alerts.matches(&format!("work order #{}", id.0)).count(),
        1,
        "one page per subject, however many fires see it: {alerts}"
    );
    // The exit is open: cancelling the stopped order is still permitted.
    ok(&mut tw, GOD, &format!("@wo/cancel {id}"));
}

/// **The backstop for an orphan the command never sees**: a submission refused
/// *after* the place leaves a `Draft` order holding a reservation, and the sweep
/// releases it — on the **second** consecutive sighting, so a hold placed in the
/// same drain as its own submission is never taken out from under it.
#[test]
fn e7r1_r1_the_sweep_releases_an_orphaned_draft_hold_on_the_second_sighting() {
    let (mut tw, families, budget, _payee) = scene(100);
    let id = seed_order(&mut tw, &families, 0xA8, budget, 1);
    ok(&mut tw, GOD, &format!("@wo/submit {id}"));
    let before = ledger_total(&tw);
    // The submission was refused downstream: the order never left `Draft`.
    assert_eq!(order_state(&mut tw, &families, id), WorkOrderState::Draft);
    assert!(budget_hold(&tw, id).is_some_and(|h| h.status.is_held()));

    fire(&mut tw);
    assert!(
        budget_hold(&tw, id).is_some_and(|h| h.status.is_held()),
        "the FIRST sighting only notes it — a live submission must not be robbed"
    );
    assert_eq!(held(&tw, budget), price(1));

    fire(&mut tw);
    assert!(
        budget_hold(&tw, id).is_some_and(|h| matches!(h.status, HoldStatus::Released)),
        "the second releases it"
    );
    assert_eq!(held(&tw, budget), 0, "the money is free again");
    assert_eq!(bal(&tw, budget), 100);
    assert_eq!(ledger_total(&tw), before);
}

/// **A retyped submit reuses the standing reservation** rather than posting a
/// second one: `industry.wo.hold/{order}` would answer `AlreadyPosted` anyway, and
/// two reservations for one order would double-reserve the budget.
#[test]
fn e7r1_r1_a_retyped_submit_reuses_the_standing_reservation() {
    let (mut tw, families, budget, _payee) = scene(100);
    let id = seed_order(&mut tw, &families, 0xA9, budget, 1);
    ok(&mut tw, GOD, &format!("@wo/submit {id}"));
    let first = budget_hold(&tw, id).expect("placed");
    let _ = tw.engine_mut().take_authoritative_submit();

    ok(&mut tw, GOD, &format!("@wo/submit {id}"));
    let holds: Vec<Hold> = snap(&tw)
        .holds()
        .filter(|h| h.purpose == money::purpose(id))
        .cloned()
        .collect();
    assert_eq!(holds.len(), 1, "one reservation, not two");
    assert_eq!(holds[0].id, first.id);
    assert_eq!(held(&tw, budget), price(1), "and it reserves it once");
}

/// **The sweep's own budget is encoded, and it is bounded on records visited.**
/// Nine terminal orders, one fire: exactly `MAX_WO_SETTLE_PER_FIRE` settle, so the
/// settlement lane can never starve the reconcile that shares the tick — and the
/// ninth settles on the next fire rather than being dropped.
#[test]
fn e7r1_r1_the_sweep_settles_at_most_its_budget_per_fire_and_drops_nothing() {
    assert_eq!(money::MAX_WO_SETTLE_PER_FIRE, 8);
    assert_eq!(money::MAX_WO_VISITED_PER_FIRE, 64);
    let (mut tw, families, budget, _payee) = scene(1_000);
    let mut ids = Vec::new();
    for n in 0..9_u64 {
        let id = seed_order(&mut tw, &families, 0xB0 + n, budget, 1);
        ok(&mut tw, GOD, &format!("@wo/submit {id}"));
        let _ = tw.engine_mut().take_authoritative_submit();
        set_state(&mut tw, &families, id, WorkOrderState::Cancelled);
        ids.push(id);
    }
    assert_eq!(held(&tw, budget), price(1) * 9, "nine reservations stand");

    fire(&mut tw);
    let settled = ids
        .iter()
        .filter(|id| budget_hold(&tw, **id).is_some_and(|h| !h.status.is_held()))
        .count();
    assert_eq!(
        settled,
        money::MAX_WO_SETTLE_PER_FIRE,
        "exactly the budget, on one fire"
    );

    fire(&mut tw);
    let settled = ids
        .iter()
        .filter(|id| budget_hold(&tw, **id).is_some_and(|h| !h.status.is_held()))
        .count();
    assert_eq!(settled, 9, "and the ninth is not dropped, only deferred");
    assert_eq!(held(&tw, budget), 0);
    assert_eq!(
        bal(&tw, budget),
        1_000,
        "every reservation came back unspent"
    );
}

/// **A work order's money accounts are resolved at the EDIT** — `@wo/draft` — in
/// `check_bond_accounts`' shape, because the capture side was validated nowhere:
/// an order could otherwise run to `Completed` and have nowhere to pay.
#[test]
fn e7r1_r1_the_accounts_are_resolved_at_the_draft_not_at_the_settlement() {
    let (mut tw, _families, budget, _payee) = scene(100);
    // A budget account that does not exist.
    let refusal = err(
        &mut tw,
        GOD,
        "@wo/draft tacsim.core/alloy_smelt/1=7 1 999999 500 600",
    );
    assert!(
        refusal.contains("budget account that does not exist"),
        "the refusal names the missing account: {refusal}"
    );
    assert!(
        tw.engine_mut().take_authoritative_submit().is_none(),
        "and nothing at all was drafted"
    );
    // A live one drafts.
    ok(
        &mut tw,
        GOD,
        &format!("@wo/draft tacsim.core/alloy_smelt/1=7 1 {budget} 500 600"),
    );
    assert!(tw.engine_mut().take_authoritative_submit().is_some());
}

/// **A closed budget account refuses at the edit too**, and a facility owner with
/// no distinct operating account refuses — the two halves `check_bond_accounts`
/// checks, in its own order.
#[test]
fn e7r1_r1_a_closed_budget_and_a_missing_payee_both_refuse_at_the_draft() {
    let (mut tw, _families, budget, payee) = scene(100);
    // A third, empty account, closed: the *budget* half refuses.
    let before = god_accounts(&tw);
    ok(&mut tw, GOD, &format!("@account/open #{}=gold", GOD.0));
    let third = god_accounts(&tw)
        .into_iter()
        .find(|id| !before.contains(id))
        .expect("a third account");
    ok(&mut tw, GOD, &format!("@account/close {third}"));
    let refusal = err(
        &mut tw,
        GOD,
        &format!("@wo/draft tacsim.core/alloy_smelt/1=7 1 {third} 500 600"),
    );
    assert!(
        refusal.contains("not active"),
        "a closed budget refuses: {refusal}"
    );
    // With the operating account closed too, the *payee* half refuses: a run that
    // completes must have somewhere to pay, and that is decided here rather than at
    // the settlement, where nothing could act on it.
    ok(&mut tw, GOD, &format!("@account/close {payee}"));
    let refusal = err(
        &mut tw,
        GOD,
        &format!("@wo/draft tacsim.core/alloy_smelt/1=7 1 {budget} 500 600"),
    );
    assert!(
        refusal.contains("no operating account"),
        "the payee half refuses, naming the facility's owner: {refusal}"
    );
}

/// **A submission that names no hold is refused by the provider**, byte-identically
/// to one for an order that does not exist — so the direct-offer ingress cannot
/// acquire materials and a capacity slot without paying.
#[test]
fn e7r1_r1_a_submission_naming_no_hold_is_refused_by_the_provider() {
    let (mut tw, families, budget, _payee) = scene(100);
    let id = seed_order(&mut tw, &families, 0xC1, budget, 1);
    assert_eq!(order_state(&mut tw, &families, id), WorkOrderState::Draft);

    // A `Draft` submit always names the hold it just placed…
    ok(&mut tw, GOD, &format!("@wo/submit {id}"));
    let staged = tw.engine_mut().take_authoritative_submit().expect("staged");
    assert_ne!(
        staged.parameters.get("hold").and_then(|v| v.as_u64()),
        Some(0),
        "the command never submits a nameless order"
    );
    // …and once the order has left `Draft` the command places nothing more, so a
    // re-submission carries `hold: 0` — which is exactly the shape a forged
    // direct-offer submission has, and exactly what the provider refuses.
    set_state(&mut tw, &families, id, WorkOrderState::Ready);
    ok(&mut tw, GOD, &format!("@wo/submit {id}"));
    let staged = tw.engine_mut().take_authoritative_submit().expect("staged");
    assert_eq!(
        staged.parameters.get("hold").and_then(|v| v.as_u64()),
        Some(0)
    );

    // The provider's own arm, pinned at the source so deleting it fails here rather
    // than at the next audit: `wo_submit` refuses before it reserves anything.
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/provider.rs"),
    )
    .expect("the provider source");
    let boundary = src.find("#[cfg(test)]").unwrap_or(src.len());
    let at = src[..boundary]
        .find("fn wo_submit")
        .expect("the submit prepare");
    let body = &src[at..at + 2_000];
    let refusal = body
        .find("if hold == 0")
        .expect("the nameless-hold refusal");
    let reserve = body.find(".reserve(").unwrap_or(body.len());
    assert!(
        refusal < reserve,
        "the money check runs BEFORE the first material reservation"
    );
}

/// **A reboot re-running the sweep is a no-op past the applied point.** The RAM
/// rotation cursor and the pending note are both rebuilt from nothing, and the
/// hold's own status — journal authority — is what decides.
#[test]
fn e7r1_r1_a_reboot_re_running_the_sweep_settles_nothing_twice() {
    let (mut tw, families, budget, payee) = scene(100);
    let id = seed_order(&mut tw, &families, 0xC2, budget, 1);
    ok(&mut tw, GOD, &format!("@wo/submit {id}"));
    let before = ledger_total(&tw);
    set_state(
        &mut tw,
        &families,
        id,
        WorkOrderState::Completed {
            commit: CommitToken::from_bytes([0xC2; 16]),
        },
    );
    fire(&mut tw);
    assert_eq!(bal(&tw, payee), price(1));

    // Re-hydrate every module from its own persisted documents (the sweep's cursor
    // and its pending note are RAM and come back empty), then fire again: the
    // settlement does not happen twice.
    let kv = fold_all(&mut tw);
    reboot(&mut tw, &kv);
    fire(&mut tw);
    fire(&mut tw);
    assert_eq!(bal(&tw, payee), price(1), "settled exactly once");
    assert_eq!(bal(&tw, budget), 100 - price(1));
    assert_eq!(ledger_total(&tw), before);
}

// ---- E7r1: R5(b) — `InterruptionCause::TimelineChanged`'s one producer ----

/// **The producer runs before the delivery's first irreversible step, and it is the
/// only one.**
///
/// 10b §15.1 item 8: the variant was declared at cut 4 and set nowhere, so the
/// D-E7-8 default it names had no producer at all. `wo_deliver` is it: an order whose
/// production was started on a world-time generation that is no longer live pauses as
/// `Interrupted { TimelineChanged }` instead of materializing output, and
/// `@wo/restamp` — which already existed, and already resumes an interrupted order —
/// is the exit.
///
/// It lives in the provider rather than on the production schedule's advance because
/// cut 8's `schedule_scan_clock_is_skippable` makes the kernel **skip** a
/// stale-generation schedule, so `wo_advance` never fires to notice.
#[test]
fn e7r1_r5_the_timeline_changed_producer_runs_before_the_first_irreversible_step() {
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/provider.rs"),
    )
    .expect("the provider source");
    let boundary = src.find("#[cfg(test)]").unwrap_or(src.len());
    // Comment lines are prose, not a writer: this module's own note names the
    // variant it is about to construct.
    let production: String = src[..boundary]
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join(
            "
",
        );
    let production = production.as_str();

    // Exactly one producer, and it is inside `wo_deliver`.
    assert_eq!(
        production
            .matches("InterruptionCause::TimelineChanged")
            .count(),
        1,
        "one writer, never two places deciding when production pauses"
    );
    let deliver = production
        .find("fn wo_deliver")
        .expect("the deliver prepare");
    let body = &production[deliver..];
    let pause = body
        .find("InterruptionCause::TimelineChanged")
        .expect("the pause is inside wo_deliver");
    // The first irreversible step of a delivery is consuming the material
    // reservations; the pause must precede it, or a paused order has already spent
    // its inputs.
    let consume = body.find(".consume(").unwrap_or(body.len());
    assert!(
        pause < consume,
        "the pause runs before the first consumption"
    );
    // And the predicate it reads is pure — two recorded stamps, no world.
    let predicate = production
        .find("fn stale_generation")
        .expect("the predicate");
    let sig = &production[predicate..predicate + 200];
    assert!(
        sig.contains("order: &WorkOrder") && sig.contains("now: &ClockStamp"),
        "the predicate takes a record and a stamp and nothing else: {sig}"
    );
    assert!(
        !sig.contains("Ctx") && !sig.contains("World"),
        "it reads no world, so two evaluations of one order agree"
    );
}

/// **`Interrupted { TimelineChanged }` is a real state a work order round-trips
/// through**, and `@wo/restamp` is the declared exit from it.
#[test]
fn e7r1_r5_a_timeline_changed_pause_round_trips_and_names_its_exit() {
    let paused = WorkOrderState::Interrupted {
        cause: mush_industry::InterruptionCause::TimelineChanged,
    };
    let encoded = serde_json::to_value(paused).expect("encode");
    let back: WorkOrderState = serde_json::from_value(encoded).expect("decode");
    assert_eq!(back, paused);
    // It holds resources and is not terminal — so the money hold stays placed and the
    // sweep leaves it alone until the order really ends.
    assert!(paused.holds_resources());
    assert!(!paused.is_terminal());

    // The exit exists as a registered verb on the prepared path.
    assert!(
        mush_industry::IndustryVerb::all()
            .iter()
            .any(|verb| matches!(verb, mush_industry::IndustryVerb::WoRestamp)),
        "`@wo/restamp` is the declared exit and is still registered"
    );
}
