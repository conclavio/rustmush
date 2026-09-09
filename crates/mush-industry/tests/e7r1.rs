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
    fn catalog_revision(&self, _world: &World) -> u64 {
        1
    }
    fn catalog(&self, _world: &World) -> Vec<(RecipeRef, PinnedRecipe)> {
        vec![alloy_recipe()]
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
        engine
            .install_prepared_action(installation)
            .expect("the fixture installs");
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

/// The material source [`scene`] seeds.
///
/// ---- WS02-S5 ----
///
/// A work order's `source`, `destination` and `account` are no longer free text:
/// `wo_draft` refuses any that is not a member of the drafter's own
/// server-authored endpoint/account book, because `wo_submit` reserves material
/// out of `source` and `wo_deliver` mints into `destination` with no ownership
/// test of their own. So this corridor's endpoints are REAL vaults now instead of
/// two numbers typed into the command. Vault ids are handed out from 0 and id 0 is
/// the crate's own "absent" spelling, so `scene` opens a pad first and then these
/// two — and asserts it got them.
const SCENE_SOURCE: u64 = 1;
/// The delivery destination [`scene`] seeds. See [`SCENE_SOURCE`].
const SCENE_DEST: u64 = 2;

/// A real container-backed E3 vault owned by `who`, and the endpoint it is.
fn make_endpoint(tw: &mut TestWorld, name: &str, who: ObjId) -> EndpointRef {
    let object = tw
        .world_mut()
        .create(mush_core::ObjKind::Thing, name, who, Some(mush_core::LIMBO))
        .expect("a backing object");
    let before: Vec<u64> = snap(tw).vaults().map(|vault| vault.id.0).collect();
    ok(
        tw,
        GOD,
        &format!(
            "@vault/create {name}=container:#{} owner=#{}",
            object.0, who.0
        ),
    );
    let fresh = snap(tw)
        .vaults()
        .map(|vault| vault.id.0)
        .find(|id| !before.contains(id))
        .expect("a fresh vault");
    EndpointRef(VaultId(fresh))
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
    // WS02-S5: the corridor's two endpoints, as real vaults GOD owns, so
    // `live_endpoints` answers them and the draft's endpoint book carries them.
    let _pad = make_endpoint(&mut tw, "pad", GOD);
    let source = make_endpoint(&mut tw, "feed", GOD);
    let destination = make_endpoint(&mut tw, "yard", GOD);
    assert_eq!(
        (source.vault().0, destination.vault().0),
        (SCENE_SOURCE, SCENE_DEST),
        "the fixture's endpoint ids are the ones the corridor names"
    );
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
        Some(endpoint(SCENE_SOURCE)),
        DeliveryTarget::Endpoint {
            endpoint: endpoint(SCENE_DEST),
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
        &format!("@wo/draft tacsim.core/alloy_smelt/1=7 1 999999 {SCENE_SOURCE} {SCENE_DEST}"),
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
        &format!("@wo/draft tacsim.core/alloy_smelt/1=7 1 {budget} {SCENE_SOURCE} {SCENE_DEST}"),
    );
    assert!(tw.engine_mut().take_authoritative_submit().is_some());
}

/// **A closed budget account refuses at the edit too**, and a facility owner with
/// no payable account at all refuses — the two halves `check_bond_accounts` checks,
/// in its own order.
///
/// **Amended by the independent verify fold (C1/B9, owner decision F1(b)).** The
/// payee half used to refuse when the owner held no account *other than* the budget,
/// which made a one-account owner-operator unable to draft at all while a two-account
/// owner paid themselves — the same line read from two ends. `payee_of` now falls
/// back to the budget account when the facility's owner **is** the budget's owner, so
/// the refusal means what it says: the owner holds nothing payable in that
/// instrument. Both accounts are closed here to prove it.
#[test]
fn e7r1_r1_a_closed_budget_and_a_missing_payee_both_refuse_at_the_draft() {
    // Unfunded on purpose: a draft resolves accounts and reserves nothing, and an
    // account carrying a balance cannot be closed.
    let (mut tw, _families, budget, payee) = scene(0);
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
        &format!("@wo/draft tacsim.core/alloy_smelt/1=7 1 {third} {SCENE_SOURCE} {SCENE_DEST}"),
    );
    assert!(
        refusal.contains("not active"),
        "a closed budget refuses: {refusal}"
    );
    // With the second account closed, the owner-operator fall-back still drafts: the
    // budget account is its own payee and the capture is a recorded self-transfer.
    ok(&mut tw, GOD, &format!("@account/close {payee}"));
    ok(
        &mut tw,
        GOD,
        &format!("@wo/draft tacsim.core/alloy_smelt/1=7 1 {budget} {SCENE_SOURCE} {SCENE_DEST}"),
    );
    assert!(tw.engine_mut().take_authoritative_submit().is_some());
    // And with the budget closed as well the owner holds nothing payable at all, so
    // the *budget* half refuses first — as it must, since a budget that is not active
    // is a stronger and earlier fact than a payee that cannot be found. The genuine
    // `NoPayee` path needs a facility owner who is not the requester and is exercised
    // in `vf_c1d_the_draft_is_not_an_account_census_of_a_stranger`.
    ok(&mut tw, GOD, &format!("@account/close {budget}"));
    let refusal = err(
        &mut tw,
        GOD,
        &format!("@wo/draft tacsim.core/alloy_smelt/1=7 1 {budget} {SCENE_SOURCE} {SCENE_DEST}"),
    );
    assert!(
        refusal.contains("not active"),
        "a closed budget still refuses first: {refusal}"
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
    let end = src[at..]
        .find("\n    fn ")
        .map_or(src.len(), |offset| at + offset);
    let body = &src[at..end];
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

// ---- E7 residue pass 1 independent verify fold ----
//
// The C (conservation), D (durability) and B (bounds/claims) lenses' confirmed
// findings, each driving the **real** path and asserting the **observable** outcome.

/// Drive one authoritative submission through the REAL offer route —
/// `command_offer_submission` + `resolve_and_stage_offer`, exactly as `mush-server`'s
/// world task does — and install it.
///
/// This is the harness the pass never built, and its absence is why **Z2** shipped:
/// `TestWorld::run` *stages* a submission and never resolves it, so a parameter the
/// projection does not declare is invisible to every test in the tree while being
/// fatal on a real server.
fn land_parts(
    tw: &mut TestWorld,
    actor: ObjId,
    action: &mush_core::DefinitionRef,
    subject: &mush_core::PerceivedRef,
    parameters: serde_json::Value,
    token: u8,
) -> Result<Vec<mush_core::ModuleOp>, String> {
    tw.engine_mut().set_offer_keys(
        mush_core::OfferKeyRing::new(
            mush_core::OfferSecret::new(2, [2; 32]).map_err(|_| "secret".to_owned())?,
            [],
        )
        .map_err(|_| "key ring".to_owned())?,
    );
    let (action, resolved) = {
        let engine = tw.engine();
        let query = engine
            .legal_action_query(
                mush_core::PrincipalId::Object { id: actor },
                actor,
                subject.clone(),
            )
            .map_err(|error| error.to_string())?;
        let offers = engine
            .project_legal_actions(&query)
            .map_err(|error| error.to_string())?;
        let projected = offers
            .projected_for_action(action)
            .ok_or_else(|| "that action is not currently available".to_owned())?;
        let submission = mush_core::OfferSubmission {
            offer: projected.offer(),
            subject: projected.subject().clone(),
            targets: Vec::new(),
            parameters,
        };
        let execution = mush_core::OfferExecutionContext {
            request: mush_core::RequestUid::from_bytes([token; 16]),
            principal: mush_core::PrincipalId::Object { id: actor },
            actor,
            idempotency: mush_core::ScopedIdempotencyKey::new(
                mush_core::IdempotencyScope {
                    principal: mush_core::PrincipalId::Object { id: actor },
                    owner: action.key().owner.clone(),
                    namespace: CanonicalId::new(action.key().id.as_str())
                        .map_err(|_| "namespace".to_owned())?,
                },
                1,
                mush_core::IdempotencyKey::Request(mush_core::RequestUid::from_bytes([token; 16])),
            ),
            viewer_scope: query.viewer_scope(),
            now: None,
        };
        let resolved = engine
            .resolve_legal_action(&offers, execution, submission)
            .map_err(|error| format!("{error:?}"))?;
        let (action, resolved, _guards) = resolved.into_authoritative_parts();
        (action, resolved)
    };
    let (prepared, _) = tw
        .engine_mut()
        .prepare_registered_resolved_authoritative_action(
            &action,
            &resolved,
            CommitToken::from_bytes([token; 16]),
        )
        .map_err(|error| format!("{error:?}"))?;
    let (ops, installation) = prepared.into_parts();
    tw.engine_mut()
        .install_prepared_action(installation)
        .expect("the fixture installs");
    Ok(ops)
}

/// Resolve and install the one submission the last command staged.
fn land(tw: &mut TestWorld, token: u8) -> Result<Vec<mush_core::ModuleOp>, String> {
    let submit = tw
        .engine_mut()
        .take_authoritative_submit()
        .ok_or_else(|| "nothing was staged".to_owned())?;
    land_parts(
        tw,
        submit.actor,
        &submit.action,
        &submit.subject,
        submit.parameters.clone(),
        token,
    )
}

/// A staff object that is not `GOD`, so payer and payee can be genuinely distinct
/// principals. `@facility/open` is staff-only and hardcodes the opener as owner, so a
/// second staff object is the only way to reach the distinct-principal path in the
/// shipped tree (C1's own note; the real ownership split is pass 2's).
fn staff_object(tw: &mut TestWorld, name: &str) -> ObjId {
    let who = tw.spawn_player(name);
    tw.world_mut()
        .get_mut(who)
        .expect("the spawned staffer")
        .powers = mush_core::Powers::SIM_ADMIN;
    who
}

/// One fresh gold account for `owner`.
fn open_account(tw: &mut TestWorld, owner: ObjId) -> AccountId {
    let before: Vec<u64> = snap(tw)
        .accounts()
        .filter(|a| a.owner == PrincipalId::Object(owner))
        .map(|a| a.id.0)
        .collect();
    ok(tw, GOD, &format!("@account/open #{}=gold", owner.0));
    let fresh = snap(tw)
        .accounts()
        .filter(|a| a.owner == PrincipalId::Object(owner))
        .map(|a| a.id.0)
        .find(|id| !before.contains(id))
        .expect("a fresh account");
    AccountId(fresh)
}

/// The fixture C1's distinct-principal path needs: `boss` (a second staff object)
/// owns facility #8 and one gold account; `GOD` keeps the budget from `scene`.
fn two_principals(funds: i64) -> (TestWorld, IndustryFamilies, AccountId, ObjId, AccountId) {
    let (mut tw, families, budget, _payee) = scene(funds);
    let boss = staff_object(&mut tw, "Boss");
    let boss_account = open_account(&mut tw, boss);
    ok(&mut tw, boss, "@facility/open 8=2 smelt");
    let _ = tw.engine_mut().take_authoritative_submit();
    (tw, families, budget, boss, boss_account)
}

/// Stage a `Draft` order at `facility`, otherwise `seed_order`'s shape.
fn seed_order_at(
    tw: &mut TestWorld,
    families: &IndustryFamilies,
    id: u64,
    budget: AccountId,
    facility: ObjId,
) -> WorkOrderId {
    let (recipe, pinned) = alloy_recipe();
    let order = WorkOrder::draft(
        WorkOrderId(id),
        PrincipalId::Object(GOD),
        PrincipalId::Object(GOD),
        facility,
        2,
        1,
        1,
        recipe,
        pinned,
        1,
        budget,
        Some(endpoint(SCENE_SOURCE)),
        DeliveryTarget::Endpoint {
            endpoint: endpoint(SCENE_DEST),
        },
        EconomicVisibility::Parties,
    )
    .expect("draft a work order");
    let engine = tw.engine_mut();
    apply(engine, families, |txn| families.open_work_order(txn, order)).expect("seed the order");
    WorkOrderId(id)
}

/// Put metals in the source pool so a real submit can reserve them.
fn seed_metals(tw: &mut TestWorld, families: &IndustryFamilies, units: u64) {
    let metals = commodity("metals");
    let engine = tw.engine_mut();
    apply(engine, families, |txn| {
        families
            .inventory
            .credit(txn, endpoint(SCENE_SOURCE), &metals, units)
            .map(|_| ())
            .map_err(IndustryError::Inventory)
    })
    .expect("seed metals");
}

/// The whole order record, read out of the family.
fn order_record(tw: &mut TestWorld, families: &IndustryFamilies, id: WorkOrderId) -> WorkOrder {
    let engine = tw.engine_mut();
    let mut txn = engine
        .action_transaction(&families.deposits)
        .expect("industry transaction");
    families
        .work_order(&mut txn, id)
        .expect("read")
        .expect("the order exists")
        .clone()
}

/// The generation stamped on the order's own `ProductionSchedule` — the exact input
/// `provider::stale_generation` reads.
fn order_generation(
    tw: &mut TestWorld,
    families: &IndustryFamilies,
    id: WorkOrderId,
) -> mush_core::ClockRef {
    order_record(tw, families, id)
        .schedule()
        .expect("a live schedule")
        .started()
        .clock
        .clone()
}

/// Run production to completion, exactly as the schedule's own advances would.
fn run_production(tw: &mut TestWorld, families: &IndustryFamilies, id: WorkOrderId) {
    let total = order_record(tw, families, id)
        .schedule()
        .expect("a live schedule")
        .total_units();
    let engine = tw.engine_mut();
    apply(engine, families, |txn| {
        let order = families.work_order(txn, id)?.expect("the order").clone();
        let next = order.with_progress(total)?;
        families.put_work_order(txn, order.revision(), next)
    })
    .expect("advance production");
}

/// Every hold this order ever placed, in id order.
fn holds_of(tw: &TestWorld, order: WorkOrderId) -> Vec<Hold> {
    snap(tw)
        .holds()
        .filter(|h| h.purpose == money::purpose(order))
        .cloned()
        .collect()
}

/// Whether an idempotency key has already been spent.
fn key_spent(tw: &TestWorld, key: &mush_economy::IdempotencyKey) -> bool {
    snap(tw).idempotency(key).is_some()
}

// ---- C1 / B9 (owner decision F1(b)) — what a work order costs the owner ----

/// **An owner-operator's completed order is a recorded self-transfer and moves no
/// net worth** (C1).
///
/// `@facility/open` is staff-only and hardcodes `PrincipalId::Object(ctx.actor)` as
/// the owner, and `check_wo_accounts` requires the budget to belong to the requester,
/// so in the shipped tree the drafter, the payer and the payee's owner are one
/// principal. The capture is a closed loop — and it is now **recorded** as one rather
/// than presented as a money gate.
#[test]
fn vf_c1a_an_owner_operators_completed_order_is_a_recorded_self_transfer_and_moves_no_net_worth() {
    let (mut tw, families, budget, payee) = scene(100);
    let id = seed_order(&mut tw, &families, 0xC1, budget, 1);
    ok(&mut tw, GOD, &format!("@wo/submit {id}"));
    let net_before = bal(&tw, budget) + bal(&tw, payee);
    let total_before = ledger_total(&tw);
    set_state(
        &mut tw,
        &families,
        id,
        WorkOrderState::Completed {
            commit: CommitToken::from_bytes([0xC1; 16]),
        },
    );
    fire(&mut tw);

    assert!(
        matches!(
            budget_hold(&tw, id).expect("the hold survives").status,
            HoldStatus::Consumed { .. }
        ),
        "the reservation was SPENT, not refunded"
    );
    assert_eq!(
        ledger_total(&tw),
        total_before,
        "the ledger total is conserved"
    );
    assert_eq!(
        bal(&tw, budget) + bal(&tw, payee),
        net_before,
        "and so is the requester's net worth: payer and payee are one principal"
    );
    assert_eq!(held(&tw, budget), 0, "nothing stays reserved");
}

/// **A facility owner who holds ONE account drafts, submits and settles** (B9, C1's
/// mirror image; owner decision F1(b)).
///
/// Before the fold `payee_of`'s `candidate.id != account.id` clause refused a
/// one-account owner `NoPayee` — a refusal that never said "open a second account" —
/// so the only actor who could run a work order in the shipped tree could not draft
/// one at all.
#[test]
fn vf_c1b_a_one_account_owner_drafts_and_settles() {
    let (mut tw, families, budget, payee) = scene(100);
    ok(&mut tw, GOD, &format!("@account/close {payee}"));
    assert_eq!(
        snap(&tw)
            .accounts_of(&PrincipalId::Object(GOD))
            .filter(|a| matches!(a.status, mush_economy::AccountStatus::Active))
            .count(),
        1,
        "a one-account owner-operator"
    );
    ok(
        &mut tw,
        GOD,
        &format!("@wo/draft tacsim.core/alloy_smelt/1=7 1 {budget} {SCENE_SOURCE} {SCENE_DEST}"),
    );
    let _ = tw.engine_mut().take_authoritative_submit();

    let id = seed_order(&mut tw, &families, 0xC1B, budget, 1);
    ok(&mut tw, GOD, &format!("@wo/submit {id}"));
    let before = ledger_total(&tw);
    set_state(
        &mut tw,
        &families,
        id,
        WorkOrderState::Completed {
            commit: CommitToken::from_bytes([0xB1; 16]),
        },
    );
    fire(&mut tw);
    assert!(
        matches!(
            budget_hold(&tw, id).expect("the hold").status,
            HoldStatus::Consumed { .. }
        ),
        "the one-account owner's order SETTLED rather than being refused"
    );
    assert_eq!(
        bal(&tw, budget),
        100,
        "a zero-net posting: the balance is unmoved"
    );
    assert_eq!(ledger_total(&tw), before);
}

/// **A distinct requester really pays the facility's owner** — the path pass 2's
/// ownership split will rely on, pinned now (F1(c)).
#[test]
fn vf_c1c_a_distinct_requester_really_pays_the_facility_owner() {
    let (mut tw, families, budget, _boss, boss_account) = two_principals(100);
    let id = seed_order_at(&mut tw, &families, 0xC1C, budget, ObjId(8));
    ok(&mut tw, GOD, &format!("@wo/submit {id}"));
    let before = ledger_total(&tw);
    set_state(
        &mut tw,
        &families,
        id,
        WorkOrderState::Completed {
            commit: CommitToken::from_bytes([0xC3; 16]),
        },
    );
    fire(&mut tw);
    assert_eq!(
        bal(&tw, budget),
        100 - price(1),
        "the requester's budget really fell"
    );
    assert_eq!(
        bal(&tw, boss_account),
        price(1),
        "and the FACILITY OWNER — a different principal — was paid"
    );
    assert_eq!(ledger_total(&tw), before, "conserved across the payment");
}

/// **A draft is not an account census of a stranger** (B9's disclosure half): a
/// facility a stranger may not run answers `NO_SUCH_FACILITY` before any account is
/// resolved, byte-identically to one that does not exist.
#[test]
fn vf_c1d_the_draft_is_not_an_account_census_of_a_stranger() {
    let (mut tw, _families, _budget, _boss, _boss_account) = two_principals(100);
    let stranger = tw.spawn_player("Pat");
    let mine = open_account(&mut tw, stranger);
    let real = err(
        &mut tw,
        stranger,
        &format!("@wo/draft tacsim.core/alloy_smelt/1=8 1 {mine} {SCENE_SOURCE} {SCENE_DEST}"),
    );
    let phantom = err(
        &mut tw,
        stranger,
        &format!("@wo/draft tacsim.core/alloy_smelt/1=4242 1 {mine} {SCENE_SOURCE} {SCENE_DEST}"),
    );
    assert_eq!(
        real, phantom,
        "an unrunnable facility and a nonexistent one are one byte string"
    );
}

// ---- C2 / D2 / B1 — `@wo/restamp` is the exit it says it is ----

/// **The tree's first end-to-end pause and recovery**: submit, change the world-time
/// rate, deliver (pauses), restamp, deliver (**completes**).
///
/// Before the fold `stale_generation` read the order's own `ProductionSchedule`,
/// whose only writer was `wo_submit`, while `wo_restamp` rewrote the kernel's
/// `DurableSchedule` and then only `with_state(Running)` — which clones the record,
/// schedule included. The predicate was therefore still true on the very next attempt
/// and the order re-paused for ever, holding its budget, its materials and its
/// facility slot, with nothing paging (`Interrupted` holds resources and is not
/// terminal) and no exit but `@wo/cancel`. Both R5(b) pins were indirect, which is how
/// three lenses found this independently and no test did.
#[test]
fn vf_c2a_a_rate_change_pauses_an_order_and_the_restamp_lets_the_next_deliver_complete() {
    let (mut tw, families, budget, _payee) = scene(1_000);
    seed_metals(&mut tw, &families, 40);
    let id = seed_order(&mut tw, &families, 0xC2A, budget, 1);

    ok(&mut tw, GOD, &format!("@wo/submit {id}"));
    land(&mut tw, 0x21).expect("the submission resolves and installs");
    assert_eq!(order_state(&mut tw, &families, id), WorkOrderState::Running);
    run_production(&mut tw, &families, id);

    // The rate change: exactly the epoch `@tacsim/time rate=` installs.
    let epoch = tw
        .world()
        .next_world_timeline_epoch(2)
        .expect("a legal rate");
    tw.world_mut()
        .append_world_timeline_epoch(epoch)
        .expect("the generation opens");

    ok(&mut tw, GOD, &format!("@wo/deliver {id}"));
    land(&mut tw, 0x22).expect("the delivery resolves");
    assert_eq!(
        order_state(&mut tw, &families, id),
        WorkOrderState::Interrupted {
            cause: mush_industry::InterruptionCause::TimelineChanged
        },
        "the generation change PAUSED it rather than delivering"
    );

    ok(&mut tw, GOD, &format!("@wo/restamp {id}"));
    land(&mut tw, 0x23).expect("the restamp resolves");
    assert_eq!(order_state(&mut tw, &families, id), WorkOrderState::Running);

    ok(&mut tw, GOD, &format!("@wo/deliver {id}"));
    land(&mut tw, 0x24).expect("the second delivery resolves");
    assert!(
        matches!(
            order_state(&mut tw, &families, id),
            WorkOrderState::Completed { .. }
        ),
        "THE RESTAMP IS THE EXIT: the next deliver completes"
    );
}

/// **The restamp's post-image carries the LIVE generation on the order's own
/// production schedule** — the record-level pin under `vf_c2a`.
#[test]
fn vf_c2b_the_restamp_post_image_carries_the_live_generation() {
    let (mut tw, families, budget, _payee) = scene(1_000);
    seed_metals(&mut tw, &families, 40);
    let id = seed_order(&mut tw, &families, 0xC2B, budget, 1);
    ok(&mut tw, GOD, &format!("@wo/submit {id}"));
    land(&mut tw, 0x31).expect("submitted");
    let started = order_generation(&mut tw, &families, id);

    let epoch = tw
        .world()
        .next_world_timeline_epoch(3)
        .expect("a legal rate");
    tw.world_mut()
        .append_world_timeline_epoch(epoch)
        .expect("the generation opens");
    let live = tw.world().world_ic_now().clock;
    assert_ne!(started, live, "the stamp is stale by construction");

    ok(&mut tw, GOD, &format!("@wo/restamp {id}"));
    land(&mut tw, 0x32).expect("restamped");
    assert_eq!(
        order_generation(&mut tw, &families, id),
        live,
        "the restamp REWROTE the stamp `stale_generation` reads"
    );
}

/// **A paused order still holds its budget and its facility slot, and cancelling
/// releases both** (D2's retention half).
#[test]
fn vf_c2c_a_paused_order_still_holds_its_budget_and_its_slot_and_releases_them_on_cancel() {
    let paused = WorkOrderState::Interrupted {
        cause: mush_industry::InterruptionCause::TimelineChanged,
    };
    assert!(paused.holds_resources() && !paused.is_terminal());

    let (mut tw, families, budget, _payee) = scene(100);
    let id = seed_order(&mut tw, &families, 0xC2C, budget, 1);
    ok(&mut tw, GOD, &format!("@wo/submit {id}"));
    let _ = tw.engine_mut().take_authoritative_submit();
    set_state(&mut tw, &families, id, paused);

    fire(&mut tw);
    assert_eq!(
        held(&tw, budget),
        price(1),
        "a pause holds the money: the sweep's terminal and orphan arms both skip it"
    );
    let alerts = ok(&mut tw, GOD, "@econ/alerts");
    assert!(
        !alerts.contains(&format!("work order #{}", id.0)),
        "and nothing pages, because the hold is live: {alerts}"
    );

    set_state(&mut tw, &families, id, WorkOrderState::Cancelled);
    fire(&mut tw);
    assert_eq!(held(&tw, budget), 0, "cancelling gives it back");
    assert_eq!(bal(&tw, budget), 100);
}

// ---- C3 (owner decision F3(a)) — a completed order that cannot pay is REFUSED ----

/// **A completed order whose payee closed holds its budget and pages, rather than
/// refunding a delivered order** (C3).
///
/// `wo_deliver` materialises the output lot at the transition while the capture waits
/// for the Coarse tick, so the old `None => release(..)` arm handed the requester the
/// goods **and** every coin back — with the ledger total conserved throughout, which is
/// exactly why the conservation pins never saw it.
#[test]
fn vf_c3a_a_completed_order_whose_payee_closed_holds_its_budget_and_pages_rather_than_refunding() {
    let (mut tw, families, budget, boss, boss_account) = two_principals(100);
    let id = seed_order_at(&mut tw, &families, 0xC3A, budget, ObjId(8));
    ok(&mut tw, GOD, &format!("@wo/submit {id}"));
    set_state(
        &mut tw,
        &families,
        id,
        WorkOrderState::Completed {
            commit: CommitToken::from_bytes([0xC3; 16]),
        },
    );
    ok(&mut tw, GOD, &format!("@account/close {boss_account}"));

    fire(&mut tw);
    assert!(
        budget_hold(&tw, id).is_some_and(|h| h.status.is_held()),
        "the hold is still LIVE: a settlement that cannot pay is refused, not refunded"
    );
    assert_eq!(held(&tw, budget), price(1), "the money is still reserved");
    assert_eq!(bal(&tw, budget), 100, "nothing was captured");
    let alerts = ok(&mut tw, GOD, "@econ/alerts");
    assert!(
        alerts.contains(&format!("work order #{}", id.0))
            && alerts.contains("no account to be paid into"),
        "and staff are told what is wrong: {alerts}"
    );
    let recommended: Vec<String> = tw
        .world()
        .module_state::<EconomyStore>()
        .expect("economy")
        .escalations()
        .filter(|e| e.subject == mush_economy::EscalationSubject::WorkOrder(id.0))
        .flat_map(|e| e.recommended.clone())
        .collect();
    assert!(
        recommended
            .iter()
            .any(|line| line.starts_with(&format!("@account/open #{}=", boss.0))),
        "with the recommended command filled in: {recommended:?}"
    );
}

/// **Reopening a payout account lets the very next fire capture** — the refusal is
/// not a dead end.
#[test]
fn vf_c3b_reopening_a_payout_account_lets_the_next_fire_capture() {
    let (mut tw, families, budget, boss, boss_account) = two_principals(100);
    let id = seed_order_at(&mut tw, &families, 0xC3B, budget, ObjId(8));
    ok(&mut tw, GOD, &format!("@wo/submit {id}"));
    let before = ledger_total(&tw);
    set_state(
        &mut tw,
        &families,
        id,
        WorkOrderState::Completed {
            commit: CommitToken::from_bytes([0xC4; 16]),
        },
    );
    ok(&mut tw, GOD, &format!("@account/close {boss_account}"));
    fire(&mut tw);
    assert!(budget_hold(&tw, id).is_some_and(|h| h.status.is_held()));

    let reopened = open_account(&mut tw, boss);
    fire(&mut tw);
    assert!(
        matches!(
            budget_hold(&tw, id).expect("the hold").status,
            HoldStatus::Consumed { .. }
        ),
        "the next fire captured"
    );
    assert_eq!(bal(&tw, reopened), price(1), "into the reopened account");
    assert_eq!(ledger_total(&tw), before);
}

// ---- C4 / D1 / D8 (owner decision F2(a)) — the attempt term ----

/// **A released orphan is resubmittable, with a fresh live hold** (C4/D1).
///
/// `industry.wo.hold/{order}` carried no attempt term and doc 13 W-D4 keeps every key
/// for ever, so after the sweep's orphan release a retyped `@wo/submit` met
/// `AlreadyPosted` -> no live hold -> `NoHold`, and a **fully funded** player was told
/// *"That account cannot fund that work order"* until the end of the world.
#[test]
fn vf_c4a_a_released_orphan_is_resubmittable_with_a_new_live_hold() {
    let (mut tw, families, budget, _payee) = scene(100);
    let id = seed_order(&mut tw, &families, 0xD1, budget, 1);
    ok(&mut tw, GOD, &format!("@wo/submit {id}"));
    let _ = tw.engine_mut().take_authoritative_submit(); // the crash shape: never landed
    fire(&mut tw);
    fire(&mut tw);
    assert!(budget_hold(&tw, id).is_some_and(|h| matches!(h.status, HoldStatus::Released)));
    assert_eq!(held(&tw, budget), 0, "the money came back");

    let events = tw.run(GOD, &format!("@wo/submit {id}"));
    assert!(
        !has_error(&events),
        "a funded, drafted, live order must be resubmittable: {}",
        texts(&events)
    );
    assert_eq!(
        held(&tw, budget),
        price(1),
        "the SECOND attempt reserved the price again"
    );
    let holds = holds_of(&tw, id);
    assert_eq!(holds.len(), 2, "two attempts, two holds");
    assert!(holds[1].status.is_held(), "and the second one is live");
}

/// **The second placement's key differs from the first, and so does its settlement
/// key** — the two shapes move together or D5's refutation expires (F2(a)).
#[test]
fn vf_c4b_the_second_placements_key_differs_from_the_first() {
    let (mut tw, families, budget, _payee) = scene(100);
    let id = seed_order(&mut tw, &families, 0xD1B, budget, 1);
    ok(&mut tw, GOD, &format!("@wo/submit {id}"));
    let _ = tw.engine_mut().take_authoritative_submit();
    fire(&mut tw);
    fire(&mut tw);
    ok(&mut tw, GOD, &format!("@wo/submit {id}"));

    assert!(
        key_spent(&tw, &mush_industry::wo_hold_key(id, 0)),
        "attempt 0's placement key is spent"
    );
    assert!(
        key_spent(&tw, &mush_industry::wo_hold_key(id, 1)),
        "and attempt 1 posted under its OWN key rather than meeting it"
    );
    assert!(
        key_spent(&tw, &mush_industry::wo_settle_key(id, 0)),
        "attempt 0's settlement key is spent by its release"
    );
    assert!(
        !key_spent(&tw, &mush_industry::wo_settle_key(id, 1)),
        "attempt 1's settlement key is FRESH — this is what stops the capture \
         answering AlreadyPosted and shipping the goods free"
    );
}

/// **A live hold never coexists with a spent settle key** (D5's first refutation
/// probe, landed as a standing regression so F2's fix cannot re-open what the
/// refutation closed).
#[test]
fn vf_c4c_a_live_hold_never_coexists_with_a_spent_settle_key() {
    let (mut tw, families, budget, _payee) = scene(1_000);
    let id = seed_order(&mut tw, &families, 0xD5, budget, 1);
    ok(&mut tw, GOD, &format!("@wo/submit {id}"));
    let _ = tw.engine_mut().take_authoritative_submit();
    for round in 0..3u32 {
        for (n, hold) in holds_of(&tw, id).into_iter().enumerate() {
            let n = u32::try_from(n).expect("a small attempt");
            if hold.status.is_held() {
                assert!(
                    !key_spent(&tw, &mush_industry::wo_settle_key(id, n)),
                    "round {round}: a LIVE hold met a spent settle key"
                );
            }
        }
        fire(&mut tw);
        fire(&mut tw);
        ok(&mut tw, GOD, &format!("@wo/submit {id}"));
        let _ = tw.engine_mut().take_authoritative_submit();
    }
    set_state(
        &mut tw,
        &families,
        id,
        WorkOrderState::Completed {
            commit: CommitToken::from_bytes([0xD5; 16]),
        },
    );
    fire(&mut tw);
    assert!(
        holds_of(&tw, id)
            .iter()
            .any(|h| matches!(h.status, HoldStatus::Consumed { .. })),
        "the last attempt's capture LANDED rather than answering AlreadyPosted"
    );
}

/// **A payee that changes before the settle posts clean and pages nothing** (D5's
/// second refutation probe, landed as a standing regression).
#[test]
fn vf_c4d_a_payee_that_changes_before_the_settle_posts_clean_and_pages_nothing() {
    let (mut tw, families, budget, boss, boss_account) = two_principals(100);
    let id = seed_order_at(&mut tw, &families, 0xD5D, budget, ObjId(8));
    ok(&mut tw, GOD, &format!("@wo/submit {id}"));
    let before = ledger_total(&tw);
    set_state(
        &mut tw,
        &families,
        id,
        WorkOrderState::Completed {
            commit: CommitToken::from_bytes([0xD6; 16]),
        },
    );
    // The owner opens a second account and closes the first WHILE the hold is live:
    // the payee is re-resolved at settlement, so the capture lands on the live one.
    let second = open_account(&mut tw, boss);
    ok(&mut tw, GOD, &format!("@account/close {boss_account}"));
    fire(&mut tw);

    assert_eq!(bal(&tw, second), price(1), "paid into the live account");
    assert_eq!(ledger_total(&tw), before, "conserved");
    let alerts = ok(&mut tw, GOD, "@econ/alerts");
    assert!(
        !alerts.contains(&format!("work order #{}", id.0)),
        "and nothing was paged: {alerts}"
    );
}

// ---- C13 — the staff bypass on the budget's ownership ----

/// **Staff may not name a player's account as a work order's budget** (C13).
///
/// The hold carries `"industry.workorder"`, which is in `ENGINE_OWNED_RESERVATIONS`,
/// so `authorize_hold` refuses the account's own owner unconditionally: a staff draft
/// against a player's wallet reserved money the player could not release, with a staff
/// `@wo/cancel` as their only exit. A consent gap, not an escalation — and the
/// ownership clause loses its escape.
#[test]
fn vf_c13a_staff_may_not_name_a_players_account_as_a_work_orders_budget() {
    let (mut tw, _families, _budget, _payee) = scene(100);
    let pat = tw.spawn_player("Pat");
    let theirs = open_account(&mut tw, pat);
    let refusal = err(
        &mut tw,
        GOD,
        &format!("@wo/draft tacsim.core/alloy_smelt/1=7 1 {theirs} {SCENE_SOURCE} {SCENE_DEST}"),
    );
    assert_eq!(
        refusal,
        mush_industry::AccountRefusal::NotYours.public(),
        "even for staff, a budget is a reservation on the REQUESTER's own account"
    );
    assert!(
        tw.engine_mut().take_authoritative_submit().is_none(),
        "and nothing at all was drafted"
    );
}

// ---- D4 / B5 — the delivery ingress ----

/// **A swept vanished hold pauses the order and the OFFER ingress delivers nothing**
/// (D4; F3(a) adapted — the gate is `project_offer`'s server-authored fixed
/// parameter, because a *submitted* attestation would be forgeable: a command
/// submission and a hand-built offer submission take the same route).
#[test]
fn vf_d4a_a_swept_vanished_hold_pauses_the_order_and_the_offer_ingress_delivers_nothing() {
    let (mut tw, families, budget, _payee) = scene(1_000);
    seed_metals(&mut tw, &families, 40);
    let id = seed_order(&mut tw, &families, 0xD4, budget, 1);
    // A `Running` order with a complete schedule and no hold at all: the shape a lost
    // placement, a restore, or a forged direct submission leaves.
    set_state(&mut tw, &families, id, WorkOrderState::Running);
    assert!(budget_hold(&tw, id).is_none());

    fire(&mut tw);
    let alerts = ok(&mut tw, GOD, "@econ/alerts");
    assert!(
        alerts.contains(&format!("work order #{}", id.0)),
        "the sweep paged the subject: {alerts}"
    );
    // The command refuses…
    let refusal = err(&mut tw, GOD, &format!("@wo/deliver {id}"));
    assert!(refusal.contains("budget reservation is missing"));
    // …and so does the offer route, which is the ingress that used to be open. It is
    // driven directly here — no command involved — with exactly the one parameter the
    // projection declares.
    let action = mush_industry::action_ref(tw.registry(), mush_industry::IndustryVerb::WoDeliver)
        .expect("the deliver action is composed");
    let landed = land_parts(
        &mut tw,
        GOD,
        &action,
        &mush_core::PerceivedRef::SelfActor,
        serde_json::json!({ "order": id.to_string() }),
        0x42,
    );
    assert!(
        landed.is_err(),
        "an offer-route delivery of an unfunded order must not land: {landed:?}"
    );
    assert!(
        !matches!(
            order_state(&mut tw, &families, id),
            WorkOrderState::Completed { .. }
        ),
        "and nothing was produced"
    );
}

/// **A funded order still delivers through the offer route**, so the D4 gate refuses
/// the unfunded case and nothing else.
#[test]
fn vf_d4b_a_funded_order_still_delivers_through_the_offer_route() {
    let (mut tw, families, budget, _payee) = scene(1_000);
    seed_metals(&mut tw, &families, 40);
    let id = seed_order(&mut tw, &families, 0xD4B, budget, 1);
    ok(&mut tw, GOD, &format!("@wo/submit {id}"));
    land(&mut tw, 0x43).expect("submitted");
    run_production(&mut tw, &families, id);

    let action = mush_industry::action_ref(tw.registry(), mush_industry::IndustryVerb::WoDeliver)
        .expect("the deliver action is composed");
    land_parts(
        &mut tw,
        GOD,
        &action,
        &mush_core::PerceivedRef::SelfActor,
        serde_json::json!({ "order": id.to_string() }),
        0x44,
    )
    .expect("a funded order delivers");
    assert!(matches!(
        order_state(&mut tw, &families, id),
        WorkOrderState::Completed { .. }
    ));
}

/// **The vanished-hold refusal is byte-identical to a free id for a stranger** (B5).
#[test]
fn vf_b5a_the_vanished_hold_refusal_is_byte_identical_to_a_free_id_for_a_stranger() {
    let (mut tw, families, budget, _payee) = scene(100);
    let id = seed_order(&mut tw, &families, 0xB5, budget, 1);
    set_state(&mut tw, &families, id, WorkOrderState::Running);
    let stranger = tw.spawn_player("Pat");

    let real = texts(&tw.run(stranger, &format!("@wo/deliver {id}")));
    let _ = tw.engine_mut().take_authoritative_submit();
    let phantom = texts(&tw.run(stranger, "@wo/deliver #3735928559"));
    let _ = tw.engine_mut().take_authoritative_submit();
    assert_eq!(
        real, phantom,
        "a real unfunded order and a free id are one byte string for a non-viewer"
    );
    let mine = err(&mut tw, GOD, &format!("@wo/deliver {id}"));
    assert!(mine.contains("budget reservation is missing"));
}

// ---- D8 — a transient is a wait ----

/// **A transient placement is a WAIT, not a funding refusal** (D8, doc 13 W2).
///
/// The submit arm collapsed every `place` error into *"That account cannot fund that
/// work order"*, so doc 13 W-D1's `Unknown` verdict told a solvent player their
/// account was short. `NoEconomy` is `HoldError::is_transient`'s other arm and is the
/// reachable transient on the command path.
#[test]
fn vf_d8a_a_transient_placement_is_a_wait_not_a_funding_refusal() {
    let cell: InventoryHandleCell = Arc::default();
    let mut tw = TestWorld::with_modules(vec![
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
    let families = tw
        .world()
        .module_state::<IndustryHandles>()
        .expect("handles")
        .0
        .clone();
    ok(&mut tw, GOD, "@facility/open 7=2 smelt");
    let _ = tw.engine_mut().take_authoritative_submit();
    let id = seed_order(&mut tw, &families, 0xD8, AccountId(1), 1);
    let refusal = err(&mut tw, GOD, &format!("@wo/submit {id}"));
    assert_eq!(
        refusal,
        mush_industry::money::LEDGER_BUSY,
        "a transient is doc 13 W2's retry line, never a verdict on the wallet"
    );
    assert_eq!(order_state(&mut tw, &families, id), WorkOrderState::Draft);
}

/// **The orphan arm continues on a transient BEFORE it burns a settle slot or clears
/// the two-sighting note** (D8's second half, pinned at the source because the
/// economy exposes no way to inject a `RetryShortly` through a command).
///
/// A transient used to consume one of `MAX_WO_SETTLE_PER_FIRE` and reset the counter,
/// so each blip cost the orphan two more full rotations at `SWEEP_PERIOD_SECS`.
#[test]
fn vf_d8b_the_orphan_arm_continues_on_a_transient_before_the_slot_and_the_note() {
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/money.rs"),
    )
    .expect("the money source");
    let boundary = src.find("#[cfg(test)]").unwrap_or(src.len());
    let arm = src[..boundary]
        .find("} else if !order.state().holds_resources() {")
        .expect("the orphan arm");
    let body = &src[arm..];
    let end = body.find("\n        } else {").unwrap_or(body.len());
    let body = &body[..end];
    // Measured from the compensating release itself: the arm's earlier
    // `clear_settle_pending` belongs to the no-hold case and is not in question.
    let release = body
        .find("match release(ctx, id, &hold)")
        .expect("the compensating release");
    let body = &body[release..];
    let transient = body
        .find("Err(error) if error.is_transient() => continue")
        .expect("the transient continue");
    let slot = body.find("settled += 1").expect("the settle budget");
    let note = body
        .find("clear_settle_pending")
        .expect("the two-sighting note");
    assert!(
        transient < slot && transient < note,
        "a transient must cost neither a settle slot nor the pending note"
    );
}

// ---- D9 — the settle note's declared bound ----

/// **`settle_pending` never exceeds its declared cap**, and the cap is encoded
/// (D9's uncapped half; the coverage half stays a recorded deferral).
#[test]
fn vf_d9a_settle_pending_never_exceeds_its_declared_cap() {
    assert_eq!(mush_industry::money::MAX_SETTLE_PENDING, 256);
    let mut store = mush_industry::IndustryStore::default();
    for n in 0..(mush_industry::money::MAX_SETTLE_PENDING as u64 + 64) {
        let _ = store.note_settle_pending(WorkOrderId(n));
    }
    assert_eq!(
        store.settle_pending_len(),
        mush_industry::money::MAX_SETTLE_PENDING,
        "the note set is bounded"
    );
    assert_eq!(
        store.settle_pending_dropped(),
        64,
        "and the overflow is COUNTED"
    );
    // A note already held still promotes on its second sighting.
    assert!(store.note_settle_pending(WorkOrderId(0)));
}

// ---- D11 — the page that acknowledges itself ----

/// **Cancelling a paged order closes its alert on the next fire** (D11).
///
/// `money::page_order` recommends `@wo/cancel`; after it the order is terminal with no
/// live hold, and the sweep's terminal branch used to `continue` past it, leaving a
/// standing page about a finished order until a staffer ran `@econ/ack` by hand.
#[test]
fn vf_d11a_cancelling_a_paged_order_closes_its_alert_on_the_next_fire() {
    let (mut tw, families, budget, _payee) = scene(100);
    let id = seed_order(&mut tw, &families, 0xD11, budget, 1);
    set_state(&mut tw, &families, id, WorkOrderState::Running);
    fire(&mut tw);
    let alerts = ok(&mut tw, GOD, "@econ/alerts");
    assert!(
        alerts.contains(&format!("work order #{}", id.0)),
        "the page opened: {alerts}"
    );

    set_state(&mut tw, &families, id, WorkOrderState::Cancelled);
    fire(&mut tw);
    let alerts = ok(&mut tw, GOD, "@econ/alerts");
    assert!(
        !alerts.contains(&format!("work order #{}", id.0)),
        "the operator did what the page recommended, so the page acknowledged \
         itself: {alerts}"
    );
}

// ---- B15 / Z2 — the offer schema is the real gate ----

/// **Every `@wo` command submission resolves through the REAL offer route** (Z2, this
/// fold's own grounding — not on the verify's list).
///
/// `Ctx::submit_authoritative` is turned into an `OfferSubmission` against the actor's
/// own projection and run through `resolve_legal_action`, whose
/// `ProjectedParameterSchema::validate` refuses any key the projection does not
/// declare. `@wo/submit` staged `hold` against a projection that declared only
/// `order`, so on a real server every submit was refused `InvalidParameters` before it
/// reached the provider — R1's whole money leg was unreachable outside the tests,
/// which stage a submission and never resolve it. This is the missing rung.
#[test]
fn vf_z2a_every_wo_command_submission_resolves_through_the_real_offer_route() {
    let (mut tw, families, budget, _payee) = scene(1_000);
    seed_metals(&mut tw, &families, 40);
    ok(
        &mut tw,
        GOD,
        &format!("@wo/draft tacsim.core/alloy_smelt/1=7 1 {budget} {SCENE_SOURCE} {SCENE_DEST}"),
    );
    land(&mut tw, 0x51).expect("@wo/draft resolves");
    let id = seed_order(&mut tw, &families, 0xB15, budget, 1);
    ok(&mut tw, GOD, &format!("@wo/submit {id}"));
    land(&mut tw, 0x52).expect("@wo/submit resolves — this is the Z2 regression");
    ok(&mut tw, GOD, &format!("@wo/restamp {id}"));
    let restamp = land(&mut tw, 0x53);
    assert!(
        !format!("{restamp:?}").contains("InvalidParameters"),
        "the restamp's `policy` parameter is declared: {restamp:?}"
    );
    ok(&mut tw, GOD, &format!("@wo/cancel {id}"));
    land(&mut tw, 0x54).expect("@wo/cancel resolves");
}

/// **The real submit gate is the hold's PURPOSE, not the id in the parameter**
/// (B15, narrowed): a forged non-zero `hold` buys nothing, because `live_hold` finds
/// the reservation by purpose and the order reads `budget_missing` for ever after.
#[test]
fn vf_b15a_a_forged_hold_id_buys_no_production_because_the_authority_is_the_purpose() {
    let (mut tw, families, budget, _payee) = scene(100);
    let id = seed_order(&mut tw, &families, 0xB15F, budget, 1);
    set_state(&mut tw, &families, id, WorkOrderState::Running);
    assert!(budget_hold(&tw, id).is_none());
    let order = order_record(&mut tw, &families, id);
    assert!(
        mush_industry::money::budget_missing_in(tw.world(), &order),
        "the purpose finds no reservation, so the order is unfunded whatever id it \
         recorded"
    );
    assert!(
        err(&mut tw, GOD, &format!("@wo/deliver {id}")).contains("budget reservation is missing")
    );
}

// ---- Z4 — the draft corridor, one id from draft to delivered lot ----

/// One endpoint pool's **scalar** — `Pool::quantity` is the scalar's own `current`,
/// read through the very handle `wo_deliver` mints into, so the assertion below is
/// about the pool the provider wrote and not a projection of it.
fn pool_units(
    tw: &mut TestWorld,
    families: &IndustryFamilies,
    at: EndpointRef,
    commodity_id: &str,
) -> u64 {
    let what = commodity(commodity_id);
    let engine = tw.engine_mut();
    let mut txn = engine
        .action_transaction(&families.deposits)
        .expect("industry transaction");
    families
        .inventory
        .pool(&mut txn, at, &what)
        .expect("read the pool")
        .map_or(0, |pool| pool.quantity)
}

/// One minted lot, read straight out of `inventory.lot`.
fn lot_record(
    tw: &mut TestWorld,
    families: &IndustryFamilies,
    lot: mush_inventory::LotId,
) -> Option<mush_inventory::Lot> {
    let engine = tw.engine_mut();
    let mut txn = engine
        .action_transaction(&families.deposits)
        .expect("industry transaction");
    txn.get(&families.inventory.lots, &lot)
        .expect("read the lot family")
        .cloned()
}

/// **One drafted order chains draft → submit → advance → deliver, and the output
/// lands at the named endpoint in the recipe's own units** (Z4's missing rung).
///
/// Every other corridor in this file *seeds* its order with `seed_order`, which
/// hand-builds a `WorkOrder` and installs it: the provider's `wo_draft` never runs,
/// so nothing pins that the id, the quantity, the source and the destination a real
/// `@wo/draft` computes are the ones the rest of the lifecycle then reads. `vf_z2a`
/// drafts through the real offer route but then seeds a *different* id for the
/// submit, so the drafted order dies unread. This is the fixture the rest of the
/// workstream leans on: ONE id, produced by the provider from its own request,
/// carried the whole way, with the destination pool read before and after and
/// compared against `alloy_recipe`'s own output line rather than a number typed here.
#[test]
fn vf_z4a_one_drafted_order_runs_the_whole_corridor_and_mints_the_recipes_own_units() {
    let (mut tw, families, budget, _payee) = scene(1_000);
    seed_metals(&mut tw, &families, 40);
    let (_, pinned) = alloy_recipe();
    let quantity: u32 = 2;
    let metals_before = pool_units(&mut tw, &families, endpoint(SCENE_SOURCE), "metals");
    let alloys_before = pool_units(
        &mut tw,
        &families,
        endpoint(SCENE_DEST),
        "structural_alloys",
    );

    ok(
        &mut tw,
        GOD,
        &format!(
            "@wo/draft tacsim.core/alloy_smelt/1=7 {quantity} {budget} {SCENE_SOURCE} {SCENE_DEST}"
        ),
    );
    land(&mut tw, 0x61).expect("@wo/draft resolves through the real offer route");
    // `wo_draft` derives the id from the request it was invoked with, so the test
    // names the drafted order exactly the way the provider did — nothing was seeded.
    let id = mush_industry::derive_work_order_id(mush_core::RequestUid::from_bytes([0x61; 16]), 0)
        .expect("the drafted id");
    assert_eq!(
        order_state(&mut tw, &families, id),
        WorkOrderState::Draft,
        "the draft command really wrote a Draft order"
    );
    let drafted = order_record(&mut tw, &families, id);
    assert_eq!(drafted.quantity(), quantity, "the quantity it was told");
    assert_eq!(
        drafted.material_source(),
        Some(endpoint(SCENE_SOURCE)),
        "the source the command sent survives the round trip"
    );
    assert_eq!(
        drafted.destination().endpoint(),
        endpoint(SCENE_DEST),
        "and so does the destination — the Z4 type fix, observed"
    );

    ok(&mut tw, GOD, &format!("@wo/submit {id}"));
    land(&mut tw, 0x62).expect("@wo/submit resolves");
    assert_eq!(
        order_state(&mut tw, &families, id),
        WorkOrderState::Running,
        "the DRAFTED id is the one that submits"
    );
    run_production(&mut tw, &families, id);
    ok(&mut tw, GOD, &format!("@wo/deliver {id}"));
    land(&mut tw, 0x63).expect("@wo/deliver resolves");
    assert!(
        matches!(
            order_state(&mut tw, &families, id),
            WorkOrderState::Completed { .. }
        ),
        "and the one that completes"
    );

    let per_output = pinned
        .outputs()
        .first()
        .expect("the recipe has one output line")
        .units();
    let per_input = pinned
        .inputs()
        .first()
        .expect("the recipe has one input line")
        .units();
    // The output is a **lot**, not a pool credit: `wo_deliver` calls `mint_origin`,
    // which writes `inventory.lot` and moves the endpoint's lot census, so the
    // destination's pool scalar stays 0 and the parcel is where the units are. The
    // lot id is derived from the delivering request, exactly as the provider derived
    // it, so this reads the record the provider wrote and not a search for one.
    let lot = mush_inventory::derive_lot_id(mush_core::RequestUid::from_bytes([0x63; 16]), 0)
        .expect("the delivered lot id");
    let delivered = lot_record(&mut tw, &families, lot).expect("the delivery minted its lot");
    assert_eq!(
        delivered.endpoint(),
        endpoint(SCENE_DEST),
        "the output landed at the NAMED endpoint"
    );
    assert_eq!(delivered.commodity().as_str(), "structural_alloys");
    assert_eq!(
        delivered.quantity(),
        per_output * u64::from(quantity),
        "in the recipe's honest units, times the quantity"
    );
    assert_eq!(
        pool_units(
            &mut tw,
            &families,
            endpoint(SCENE_DEST),
            "structural_alloys"
        ),
        alloys_before,
        "and it is a parcel, so the destination's pool scalar did not move"
    );
    assert_eq!(
        pool_units(&mut tw, &families, endpoint(SCENE_SOURCE), "metals"),
        metals_before - per_input * u64::from(quantity),
        "while the source pool scalar fell by the recipe's inputs, times the quantity"
    );
}

/// **A draft that names no budget account refuses in the command and queues
/// nothing.**
///
/// `account` is positional, so "no account" reaches the command as `0`; `account 0`
/// is not an account, `money::check_wo_accounts` reads `NoBudget`, and the refusal
/// happens at the edit — before `submit` is ever called, so no authoritative
/// submission is staged and no `Draft` order exists to hold a facility slot. The
/// pair with `vf_z4a`: the corridor above only starts when the money end resolves.
#[test]
fn vf_z4b_a_draft_with_no_budget_account_refuses_and_stages_nothing() {
    let (mut tw, families, _budget, _payee) = scene(1_000);
    seed_metals(&mut tw, &families, 40);
    // An id no account answers to: `NoBudget`.
    let absent = err(
        &mut tw,
        GOD,
        &format!("@wo/draft tacsim.core/alloy_smelt/1=7 1 999999 {SCENE_SOURCE} {SCENE_DEST}"),
    );
    assert!(
        absent.contains("budget account that does not exist"),
        "the refusal names the budget, not the recipe: {absent}"
    );
    assert!(
        tw.engine_mut().take_authoritative_submit().is_none(),
        "and it staged NOTHING — no draft, no slot, no order id burnt"
    );
    // The positional default, `0`: in this fixture that is the system faucet, so the
    // requester does not own it and the refusal is `NotYours`. Either way the draft
    // dies at the edit.
    let zero = err(
        &mut tw,
        GOD,
        &format!("@wo/draft tacsim.core/alloy_smelt/1=7 1 0 {SCENE_SOURCE} {SCENE_DEST}"),
    );
    assert!(
        zero.contains("budget is a reservation on the requester's own account"),
        "account 0 is nobody's budget: {zero}"
    );
    assert!(
        tw.engine_mut().take_authoritative_submit().is_none(),
        "and that staged nothing either"
    );
    assert_eq!(
        pool_units(&mut tw, &families, endpoint(SCENE_SOURCE), "metals"),
        40,
        "nothing was reserved, so the source pool is untouched"
    );
}

// ---- Z1 — the wrapped player strings ----

/// **No shipped player-visible string carries a run of spaces mid-sentence.**
///
/// A Rust multi-line string literal without a `\` continuation keeps the source
/// indentation, so strings this pass introduced reached players with 10-18 spaces in
/// the middle of a sentence — the first thing a player reads after the two verbs the
/// pass added.
#[test]
fn vf_z1a_no_shipped_player_string_carries_a_double_space() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders: Vec<String> = Vec::new();
    for file in std::fs::read_dir(&dir).expect("src/").flatten() {
        let path = file.path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let boundary = text.find("#[cfg(test)]").unwrap_or(text.len());
        for (n, line) in text[..boundary].lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") || trimmed.starts_with('#') {
                continue;
            }
            // The un-continued-literal shape: a quoted run that both starts with real
            // words and contains two consecutive spaces, on one physical line.
            let Some(open) = line.find('"') else {
                continue;
            };
            let tail = &line[open + 1..];
            if tail.contains("  ")
                && tail.trim_end().ends_with('"')
                && tail
                    .split("  ")
                    .next()
                    .is_some_and(|head| head.chars().any(char::is_alphanumeric))
            {
                offenders.push(format!("{}:{}: {}", path.display(), n + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "player-visible literals with a run of spaces mid-sentence:\n{}",
        offenders.join("\n")
    );
}
