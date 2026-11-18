//! E7 residue pass 2's industry half: **P0a, the facility-ownership split**.
//!
//! The rig is `crates/mush-industry/tests/e7r1.rs`'s, narrowed to the money path:
//! `boot()` composes the economy, cut 2 and cut 4 in production order, `apply()`
//! opens the same owner-stamped transaction a provider gets, and every assertion is
//! an **observable** one — a rendered line, a posted balance, the ledger total.
//!
//! What P0a is for: pass 1's fold F1(c) found that drafter, payer and payee-owner
//! collapsed to one principal on any tree anyone had actually booted, because
//! `@facility/open` is staff-only and hardcodes the opener as owner. Pass 1 could
//! therefore only narrow its own headline. `@facility/owner` makes the owner
//! settable, so the two arms below are both real: a work order at **another's**
//! facility moves money between two distinct principals, and an **owner-operator's**
//! own order is still the recorded zero-net self-transfer pass 1's F1(b) kept.

use std::collections::BTreeSet;
use std::sync::Arc;

use mush_core::{CanonicalId, CommitToken, DefinitionDigest, Engine, GOD, ObjId, World};
use mush_economy::{AccountId, EconomicVisibility, EconomyStore, VaultId};
use mush_industry::facility::FacilityRecord;
use mush_industry::workorder::{
    DeliveryTarget, PinnedRecipe, RecipeLine, RecipeRef, WorkOrder, WorkOrderId, WorkOrderState,
    quote_price,
};
use mush_industry::{IndustryFamilies, IndustryHandles, IndustryModule, IndustryStore};
use mush_inventory::{EndpointRef, InventoryHandleCell, InventoryModule};
use mush_org::PrincipalId;
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, text_of};

// ---- E7r2: the rig (e7r1's, narrowed) --------------------------------------

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
    body: impl FnOnce(&mut mush_core::ActionTxn<'_>) -> Result<T, mush_industry::IndustryError>,
) -> Result<T, mush_industry::IndustryError> {
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

fn accounts_of(tw: &TestWorld, owner: ObjId) -> Vec<u64> {
    snap(tw)
        .accounts()
        .filter(|a| a.owner == PrincipalId::Object(owner))
        .map(|a| a.id.0)
        .collect()
}

/// One fresh gold account for `owner`.
fn open_account(tw: &mut TestWorld, owner: ObjId) -> AccountId {
    let before = accounts_of(tw, owner);
    ok(tw, GOD, &format!("@account/open #{}=gold", owner.0));
    let fresh = accounts_of(tw, owner)
        .into_iter()
        .find(|id| !before.contains(id))
        .expect("a fresh account");
    AccountId(fresh)
}

/// The whole fixture: an instrument, a faucet, GOD's budget funded with `funds`,
/// and a two-slot `smelt` facility on object #7 that GOD opened (and therefore
/// owns).
fn scene(funds: i64) -> (TestWorld, IndustryFamilies, AccountId) {
    let (mut tw, families) = boot();
    ok(&mut tw, GOD, "@instrument/register gold=0");
    ok(&mut tw, GOD, "@account/system faucet=gold");
    let faucet = snap(&tw)
        .accounts()
        .find(|a| matches!(a.owner, PrincipalId::System(_)))
        .expect("faucet")
        .id;
    let budget = open_account(&mut tw, GOD);
    if funds > 0 {
        ok(
            &mut tw,
            GOD,
            &format!("@econ/mint {faucet}={budget} {funds}"),
        );
    }
    ok(&mut tw, GOD, "@facility/open 7=2 smelt");
    let _ = tw.engine_mut().take_authoritative_submit();
    (tw, families, budget)
}

/// Stage one `Draft` order at facility #7, drafted and paid by `GOD`.
fn seed_order(
    tw: &mut TestWorld,
    families: &IndustryFamilies,
    id: u64,
    budget: AccountId,
    policy_revision: u64,
) -> WorkOrderId {
    let (recipe, pinned) = alloy_recipe();
    let order = WorkOrder::draft(
        WorkOrderId(id),
        PrincipalId::Object(GOD),
        PrincipalId::Object(GOD),
        ObjId(7),
        2,
        1,
        policy_revision,
        recipe,
        pinned,
        1,
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

/// One `industry.sweep` fire.
fn fire(tw: &mut TestWorld) {
    let period = i64::try_from(mush_industry::SWEEP_PERIOD_SECS).unwrap_or(60);
    let t = tw.world().now() + period + 1;
    tw.set_now(t);
    let _ = tw.fire_due(t.saturating_mul(1_000));
}

/// The price one order's budget reserves — the tree's own pure quote.
fn price(quantity: u32) -> i64 {
    let (_, pinned) = alloy_recipe();
    i64::try_from(quote_price(&pinned, quantity)).expect("a small price")
}

fn facility(tw: &TestWorld, object: ObjId) -> FacilityRecord {
    tw.world()
        .module_state::<IndustryStore>()
        .and_then(|store| store.facility(object).cloned())
        .expect("the facility record")
}

// ---- E7r2: P0a — the facility-ownership split ------------------------------

/// **The headline, restored unqualified: a work order at ANOTHER'S facility pays a
/// DISTINCT principal, and the ledger total does not move.**
///
/// This is the arm pass 1 could not build. `@facility/owner 7=#pat` hands GOD's
/// facility to an ordinary, non-staff player; GOD then drafts and runs an order
/// there out of GOD's own budget, and the capture credits **Pat's** account. Two
/// principals, two accounts, one zero-sum posting — and the conservation probe is
/// the assertion that says no faucet opened along the way.
#[test]
fn e7r2_p0a_an_order_at_anothers_facility_pays_a_distinct_principal_and_conserves_the_ledger() {
    let (mut tw, families, budget) = scene(100);
    let pat = tw.spawn_player("Pat");
    let pats = open_account(&mut tw, pat);
    ok(&mut tw, GOD, &format!("@facility/owner 7=#{}", pat.0));
    assert_eq!(
        facility(&tw, ObjId(7)).owner(),
        &PrincipalId::Object(pat),
        "the record carries the new owner"
    );

    // The observable outcome first: what a would-be requester READS before drafting.
    let shown = ok(&mut tw, GOD, "@facility/show 7");
    assert!(
        shown.contains(&format!("owner obj:{}", pat.0)),
        "`@facility/show` names who gets paid: {shown}"
    );

    let id = seed_order(&mut tw, &families, 0xB0, budget, 2);
    ok(&mut tw, GOD, &format!("@wo/submit {id}"));
    let before = ledger_total(&tw);
    set_state(
        &mut tw,
        &families,
        id,
        WorkOrderState::Completed {
            commit: CommitToken::from_bytes([0xB0; 16]),
        },
    );
    fire(&mut tw);

    assert_eq!(bal(&tw, budget), 100 - price(1), "the requester paid");
    assert_eq!(
        bal(&tw, pats),
        price(1),
        "and the facility's NEW owner — not the drafter — was paid"
    );
    assert_eq!(
        ledger_total(&tw),
        before,
        "two distinct principals, one zero-sum posting: nothing was minted"
    );
    // Fire two: a no-op past the applied point.
    fire(&mut tw);
    assert_eq!(bal(&tw, pats), price(1), "the second fire settled nothing");
    assert_eq!(ledger_total(&tw), before);
}

/// **The owner-operator arm is KEPT, and is still the recorded zero-net
/// self-transfer.** (Pass 1 fold F1(b), which P0a is required not to break.)
///
/// GOD owns facility #7 and holds one gold account, so `payee_of` answers the budget
/// account itself: the capture writes both lines onto one account, the posted balance
/// is unmoved and only the reservation is spent. Self-production stays legitimate.
#[test]
fn e7r2_p0a_the_owner_operator_arm_is_kept_as_a_zero_net_self_transfer() {
    let (mut tw, families, budget) = scene(100);
    assert_eq!(
        facility(&tw, ObjId(7)).owner(),
        &PrincipalId::Object(GOD),
        "the opener owns it until somebody re-assigns it"
    );
    let id = seed_order(&mut tw, &families, 0xB1, budget, 1);
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

    assert_eq!(
        bal(&tw, budget),
        100,
        "the owner-operator's balance is exactly where it started: the debit and the \
         credit landed on one account"
    );
    assert_eq!(
        ledger_total(&tw),
        before,
        "and the ledger total is unmoved on this arm too"
    );
}

/// **Who may re-assign: staff, or a principal `acts_for` the CURRENT owner — and a
/// stranger reads `NO_SUCH_FACILITY`, byte-identically to a facility that does not
/// exist.**
///
/// Every arm of the guard, named: the stranger, the ex-owner (whose authority went
/// with the facility), the new owner (who has it), and the object that is no
/// facility at all. The two refusals a viewer may not distinguish are byte-identical.
#[test]
fn e7r2_p0a_only_staff_or_the_current_owner_may_re_assign_and_a_stranger_is_existence_hidden() {
    let (mut tw, _families, _budget) = scene(0);
    let pat = tw.spawn_player("Pat");
    let sam = tw.spawn_player("Sam");

    // A stranger, and an object that is no facility: the same line.
    let stranger = err(&mut tw, pat, &format!("@facility/owner 7=#{}", pat.0));
    let nothing = err(&mut tw, pat, &format!("@facility/owner 4242=#{}", pat.0));
    assert_eq!(
        stranger, nothing,
        "a facility a stranger may not re-assign reads exactly what one that does not \
         exist reads"
    );
    assert_eq!(
        facility(&tw, ObjId(7)).owner(),
        &PrincipalId::Object(GOD),
        "and the refusal wrote nothing"
    );

    // Staff may. The owner then changes, and with it the authority.
    ok(&mut tw, GOD, &format!("@facility/owner 7=#{}", pat.0));
    // The NEW owner may hand it on — the record's own predicate, not a second one.
    ok(&mut tw, pat, &format!("@facility/owner 7=#{}", sam.0));
    assert_eq!(facility(&tw, ObjId(7)).owner(), &PrincipalId::Object(sam));
    // …and the previous owner is now a stranger, reading the stranger's line.
    let after = err(&mut tw, pat, &format!("@facility/owner 7=#{}", pat.0));
    assert_eq!(
        after, stranger,
        "authority went with the facility, and the refusal is the same one"
    );
    assert_eq!(facility(&tw, ObjId(7)).owner(), &PrincipalId::Object(sam));
}

/// **The policy revision bumps on every re-assignment, and the capacity revision does
/// not** — so an order drafted against the old owner cannot submit against the new
/// one, while a running order is not interrupted for a paperwork change.
///
/// `money::payee_account` re-resolves the payee from the **live** record at
/// settlement. Without the bump, an order drafted while GOD owned the facility would
/// submit unchanged and pay Pat. With it, the submit refuses and the requester
/// re-drafts against the owner they will actually pay.
#[test]
fn e7r2_p0a_the_policy_revision_bump_invalidates_a_draft_against_the_old_owner() {
    let (mut tw, families, budget) = scene(100);
    let pat = tw.spawn_player("Pat");
    let _ = open_account(&mut tw, pat);
    let before = facility(&tw, ObjId(7));
    assert_eq!(before.policy_revision(), 1);
    assert_eq!(before.capacity_revision(), 1);

    // A draft pinned against the OLD owner's policy revision.
    let stale = seed_order(&mut tw, &families, 0xB2, budget, 1);

    ok(&mut tw, GOD, &format!("@facility/owner 7=#{}", pat.0));
    let after = facility(&tw, ObjId(7));
    assert_eq!(
        after.policy_revision(),
        2,
        "the ownership change is a POLICY change"
    );
    assert_eq!(
        after.capacity_revision(),
        1,
        "and not a capacity one: nothing about the slots moved"
    );

    let refusal = err(&mut tw, GOD, &format!("@wo/submit {stale}"));
    assert!(
        !refusal.is_empty(),
        "the stale draft is refused rather than silently re-pointed at the new owner"
    );
    // A fresh draft against the new revision submits.
    let fresh = seed_order(&mut tw, &families, 0xB3, budget, 2);
    ok(&mut tw, GOD, &format!("@wo/submit {fresh}"));
}

/// **A no-op re-assignment is refused rather than burning a revision**, and an
/// unresolvable principal writes nothing.
///
/// Bumping the policy revision invalidates every in-flight draft, so a re-assignment
/// that changes nothing must not do it: a retyped command is a refusal, never a
/// second bump.
#[test]
fn e7r2_p0a_a_no_op_re_assignment_and_an_unknown_principal_both_write_nothing() {
    let (mut tw, _families, _budget) = scene(0);
    let same = err(&mut tw, GOD, &format!("@facility/owner 7=#{}", GOD.0));
    assert!(
        same.contains("already has that owner"),
        "a no-op re-assignment says so: {same}"
    );
    assert_eq!(
        facility(&tw, ObjId(7)).policy_revision(),
        1,
        "and burns no revision, because a bump invalidates every drafted order"
    );
    err(&mut tw, GOD, "@facility/owner 7=org:404");
    assert_eq!(
        facility(&tw, ObjId(7)).policy_revision(),
        1,
        "an unresolvable principal writes nothing either"
    );
    assert_eq!(facility(&tw, ObjId(7)).owner(), &PrincipalId::Object(GOD));
}

/// **The record re-enters through its own wire after a re-assignment.**
///
/// `FacilityRecord::with_owner` goes through `from_parts`, which is the same
/// constructor `Deserialize` runs, so a re-assigned record re-loads byte-identically
/// and a revision that overflowed would refuse on load rather than truncate.
#[test]
fn e7r2_p0a_a_re_assigned_facility_re_enters_through_its_own_wire() {
    let (mut tw, _families, _budget) = scene(0);
    let pat = tw.spawn_player("Pat");
    ok(&mut tw, GOD, &format!("@facility/owner 7=#{}", pat.0));
    let record = facility(&tw, ObjId(7));
    let json = serde_json::to_value(&record).expect("serialize");
    let back: FacilityRecord = serde_json::from_value(json).expect("re-enter through the wire");
    assert_eq!(back, record, "the wire is lossless");
    assert_eq!(back.owner(), &PrincipalId::Object(pat));
    assert_eq!(back.policy_revision(), 2);
}

// ---- E7r2: the adversarial verify's own folds --------------------------------

/// **vf_c1 — a facility's owner is never a SYSTEM principal.**
///
/// `mush_economy::parse_principal` accepts `sys:<name>`, so without the refusal an
/// ordinary player owning a facility could name a faucet, sink, escrow or clearing
/// principal as its owner — and every completed order there would capture its budget
/// into an account **no principal may withdraw from**. The ledger total still holds
/// (the posting is zero-sum), so this is a **sink**, not a mint; it is refused because
/// D-E6-4 keeps a system principal out of a contract for exactly the same reason, and
/// because a facility's owner has to be somebody who can be paid.
#[test]
fn vf_c1_a_facility_owner_is_never_a_system_principal() {
    let (mut tw, _families, _budget) = scene(0);
    ok(&mut tw, GOD, "@account/system sink=gold");
    let refusal = err(&mut tw, GOD, "@facility/owner 7=sys:sink");
    assert!(
        refusal.contains("system principal"),
        "the refusal says why: {refusal}"
    );
    assert_eq!(
        facility(&tw, ObjId(7)).owner(),
        &PrincipalId::Object(GOD),
        "and it wrote nothing"
    );
    assert_eq!(
        facility(&tw, ObjId(7)).policy_revision(),
        1,
        "…including no revision bump"
    );
    // The legitimate arms are unaffected: a player and an organization both take.
    let pat = tw.spawn_player("Pat");
    ok(&mut tw, GOD, &format!("@facility/owner 7=#{}", pat.0));
    assert_eq!(facility(&tw, ObjId(7)).owner(), &PrincipalId::Object(pat));
}

// ---- Doc 39 cut IN.4: the cost model prices the order ----------------------

/// **`order_price` from the facility's policy half** (doc 39 §4.4, Q-39-9, Q-39-32): facility
/// #7 re-priced with §4.4's works row (3 M over 25 years, 2 % upkeep, eight at 150, 25 %
/// margin; 1,693.16 cr a day over two slots) quotes and holds the customer's fittings order —
/// six work units, here the test recipe's three at quantity two — at **265 cr** (264.56 rounded
/// up to the whole credit), not one minor an input unit. The re-price bumps the policy
/// revision, so a draft against the unpriced record is refused at submit.
///
/// `+works/quote fittings` is IN.3's verb and does not exist yet: this asserts the same figure
/// through `@wo/quote` and the submitted hold, which is `order_price`.
#[test]
fn in4_order_price_is_the_fee_from_the_facilitys_cost_model() {
    let (mut tw, families, budget) = scene(100_000);
    let works = mush_industry::FacilityCostDef {
        capital_major: 3_000_000,
        lifetime_years: 25,
        upkeep_permille: 20,
        staff: 8,
        wage_major: 150,
        margin_permille: 250,
    };
    let stale = seed_order(&mut tw, &families, 0xC0, budget, 1);
    let priced = facility(&tw, ObjId(7))
        .with_cost(Some(works))
        .expect("re-price");
    assert_eq!(
        priced.policy_revision(),
        2,
        "the re-price bumps the policy revision"
    );
    tw.world_mut()
        .module_state_mut::<IndustryStore>()
        .expect("industry")
        .put_facility(priced);
    let refusal = err(&mut tw, GOD, &format!("@wo/submit {stale}"));
    assert!(
        !refusal.is_empty(),
        "a draft against the unpriced record is refused"
    );

    let (recipe, pinned) = alloy_recipe();
    let order = WorkOrder::draft(
        WorkOrderId(0xC1),
        PrincipalId::Object(GOD),
        PrincipalId::Object(GOD),
        ObjId(7),
        2,
        1,
        2,
        recipe,
        pinned,
        2,
        budget,
        Some(endpoint(500)),
        DeliveryTarget::Endpoint {
            endpoint: endpoint(600),
        },
        EconomicVisibility::Parties,
    )
    .expect("draft a work order");
    let id = order.id();
    apply(tw.engine_mut(), &families, |txn| {
        families.open_work_order(txn, order)
    })
    .expect("seed the order");
    let quoted = ok(&mut tw, GOD, &format!("@wo/quote {id}"));
    assert!(
        quoted.contains("fee 265 cr, duration 6 work units"),
        "{quoted}"
    );
    ok(&mut tw, GOD, &format!("@wo/submit {id}"));
    let held = snap(&tw)
        .holds()
        .find(|h| h.purpose == mush_industry::money::purpose(id) && h.status.is_held())
        .map(|h| h.amount)
        .expect("the order's hold");
    assert_eq!(
        held,
        mush_economy::Amount::from_minor(26_500),
        "the hold is fee(order)"
    );
}

// ---- doc 48 unit PL.9: facilities as titled assets -------------------------

/// Run `f` as staff with a `Ctx` over the world, its writes folded back.
fn staff_hand<T>(tw: &mut TestWorld, f: impl FnOnce(&mut mush_core::Ctx<'_>) -> T) -> T {
    let mut out = mush_core::Outbox::new();
    let engine = tw.engine_mut();
    let mut ctx = mush_core::Ctx {
        w: &mut engine.world,
        actor: GOD,
        switches: mush_core::SwitchSet::default(),
        args: mush_core::ParsedArgs::default(),
        out: &mut out,
        reg: &engine.registry,
        module: Some(smol_str::SmolStr::new("industry")),
        typed_state: mush_core::ModuleActionState::unavailable(),
    };
    f(&mut ctx)
}

fn cost(margin: u32) -> mush_industry::cost::FacilityCostDef {
    mush_industry::cost::FacilityCostDef {
        capital_major: 100_000,
        lifetime_years: 10,
        upkeep_permille: 50,
        staff: 2,
        wage_major: 100,
        margin_permille: margin,
    }
}

/// **Doc 48 unit PL.9 (Q-48-37): a facility's owner sets its customers and margin, and only staff
/// set its cost model.** Staff price facility #7 and hand it to Pat. A stranger reads
/// `No such facility.`. Pat closes its doors and raises its margin to 400 per mille: the other
/// cost fields are untouched and the policy revision moves. Pat naming the whole cost model is
/// refused with its sentence, and so is a margin past its bound; neither writes. A margin on a
/// facility nothing prices is refused too. Staff keep the full form.
#[test]
fn pl9_the_owner_sets_customers_and_margin_but_not_the_cost_model() {
    let (mut tw, _families, _budget) = scene(0);
    let pat = tw.spawn_player("Pat");
    let sam = tw.spawn_player("Sam");
    ok(
        &mut tw,
        GOD,
        "@facility/terms 7=public 100000/10/50/2/100/250",
    );
    let stranger = err(&mut tw, sam, "@facility/terms 7=owner margin=10");
    assert!(stranger.contains("No such facility."), "{stranger}");
    ok(&mut tw, GOD, &format!("@facility/owner 7=#{}", pat.0));
    let before = facility(&tw, ObjId(7));
    let set = ok(&mut tw, pat, "@facility/terms 7=owner margin=400");
    assert!(set.contains("margin 400 per mille"), "{set}");
    let after = facility(&tw, ObjId(7));
    assert_eq!(after.customers(), mush_industry::Customers::OwnerOnly);
    assert_eq!(after.cost().copied(), Some(cost(400)));
    assert!(after.policy_revision() > before.policy_revision());
    let refused = err(&mut tw, pat, "@facility/terms 7=public 1/1/0/0/0/0");
    assert!(
        refused.contains(mush_industry::commands::TERMS_COST_STAFF),
        "{refused}"
    );
    let _ = err(&mut tw, pat, "@facility/terms 7=public margin=99999");
    assert_eq!(facility(&tw, ObjId(7)), after, "the refusals wrote nothing");
    ok(&mut tw, GOD, "@facility/open 8=1 smelt");
    let unpriced = err(&mut tw, GOD, "@facility/terms 8=public margin=10");
    assert!(
        unpriced.contains(mush_industry::commands::TERMS_NO_PRICE),
        "{unpriced}"
    );
    ok(
        &mut tw,
        GOD,
        "@facility/terms 7=public 100000/10/60/2/100/250",
    );
    assert_eq!(
        facility(&tw, ObjId(7)).cost().map(|c| c.upkeep_permille),
        Some(60)
    );
}

/// **Doc 48 unit PL.9 (Q-48-45, Q-48-53): a sold facility is owner-held, so its authored re-term
/// leaves it.**
/// - An authored facility is re-termed by its content, as before.
/// - Sold to Pat, it is Pat's, owner-held, its policy revision moved. Pat raises the margin and
///   closes the doors, and the content's re-term (at another margin and upkeep) leaves both
///   alone. The content's identity check still refuses another object's capabilities or slots.
/// - A facility kept public by its sale (a yard) cannot be closed by its owner, nor by staff.
/// - On the wire, an authored record writes no `holding`, and a held one round-trips.
#[test]
fn pl9_a_sold_facility_is_owner_held_and_its_authored_re_term_leaves_it() {
    let (mut tw, _families, _budget) = scene(0);
    let pat = tw.spawn_player("Pat");
    let room = tw
        .world_mut()
        .create(mush_core::ObjKind::Thing, "Works", GOD, None)
        .expect("the works");
    let authored = |margin: u32, upkeep: u32| {
        let mut c = cost(margin);
        c.upkeep_permille = upkeep;
        FacilityRecord::new(room, PrincipalId::Object(GOD), capabilities(&["smelt"]), 2)
            .and_then(|r| r.with_authored_cost(c))
            .map(|r| r.with_authored_customers(mush_industry::Customers::Public))
            .expect("an authored record")
    };
    staff_hand(&mut tw, |ctx| {
        mush_industry::store::install_authored_facility(ctx, authored(250, 50))
    })
    .expect("opened");
    staff_hand(&mut tw, |ctx| {
        mush_industry::store::install_authored_facility(ctx, authored(300, 50))
    })
    .expect("re-termed");
    assert_eq!(
        facility(&tw, room).cost().map(|c| c.margin_permille),
        Some(300)
    );
    let unsold = facility(&tw, room);
    staff_hand(&mut tw, |ctx| {
        mush_industry::store::sell_facility(ctx, room, PrincipalId::Object(pat), false)
    })
    .expect("sold");
    let sold = facility(&tw, room);
    assert_eq!(sold.owner(), &PrincipalId::Object(pat));
    assert_eq!(sold.holding(), mush_industry::Holding::Owner);
    assert!(sold.policy_revision() > unsold.policy_revision());
    ok(
        &mut tw,
        pat,
        &format!("@facility/terms {}=owner margin=500", room.0),
    );
    let owned = facility(&tw, room);
    staff_hand(&mut tw, |ctx| {
        mush_industry::store::install_authored_facility(ctx, authored(250, 70))
    })
    .expect("the lane applies a held facility");
    assert_eq!(facility(&tw, room), owned, "the owner's terms stand");
    let drifted = FacilityRecord::new(room, PrincipalId::Object(GOD), capabilities(&["smelt"]), 3)
        .expect("a record");
    assert!(
        staff_hand(&mut tw, |ctx| {
            mush_industry::store::install_authored_facility(ctx, drifted)
        })
        .is_err(),
        "the slots are still the content's"
    );
    // A yard: kept public.
    staff_hand(&mut tw, |ctx| {
        mush_industry::store::sell_facility(ctx, room, PrincipalId::Object(GOD), true)
    })
    .expect("sold again");
    let yard = facility(&tw, room);
    assert_eq!(yard.holding(), mush_industry::Holding::OwnerPublic);
    assert_eq!(yard.customers(), mush_industry::Customers::Public);
    let closed = err(&mut tw, GOD, &format!("@facility/terms {}=owner", room.0));
    assert!(
        closed.contains(mush_industry::commands::TERMS_KEPT_PUBLIC),
        "{closed}"
    );
    assert_eq!(facility(&tw, room), yard);
    // The wire.
    let plain = serde_json::to_value(&unsold).expect("encodes");
    assert!(plain.get("holding").is_none());
    let held = serde_json::to_value(&yard).expect("encodes");
    assert_eq!(held["holding"], serde_json::json!("owner_public"));
    let back: FacilityRecord = serde_json::from_value(held.clone()).expect("decodes");
    assert_eq!(back, yard);
    let mut closed = held;
    closed["customers"] = serde_json::json!("owner_only");
    assert!(serde_json::from_value::<FacilityRecord>(closed).is_err());
}

/// **Doc 48 unit PL.6 (Q-48-25): a lane bump that only grows an authored facility's
/// capabilities widens it in place.**
/// - A yard opened with `yard` takes `yard` and `yard.heavy` with a new margin: the capacity
///   revision stands (no order is interrupted), and the policy revision moves once, for the
///   re-term alone.
/// - Dropping a capability, swapping one, or growing the set while the slots or the owner change,
///   is still drift, refused with nothing written.
/// - A sold (owner-held) facility grows too: its capabilities are the content's; its owner, its
///   terms and its revisions stay its buyer's.
#[test]
fn pl6_a_lane_bump_grows_a_facility_capabilities_in_place() {
    let (mut tw, _families, _budget) = scene(0);
    let pat = tw.spawn_player("Pat");
    let room = tw
        .world_mut()
        .create(mush_core::ObjKind::Thing, "Yard", GOD, None)
        .expect("the yard");
    let authored = |words: &[&str], margin: u32, slots: u32| {
        FacilityRecord::new(room, PrincipalId::Object(GOD), capabilities(words), slots)
            .and_then(|r| r.with_authored_cost(cost(margin)))
            .map(|r| r.with_authored_customers(mush_industry::Customers::Public))
            .expect("an authored record")
    };
    let install = |tw: &mut TestWorld, record: FacilityRecord| {
        staff_hand(tw, |ctx| {
            mush_industry::store::install_authored_facility(ctx, record)
        })
    };
    install(&mut tw, authored(&["yard"], 250, 2)).expect("opened");
    let opened = facility(&tw, room);
    install(&mut tw, authored(&["yard", "yard.heavy"], 300, 2)).expect("grown");
    let grown = facility(&tw, room);
    assert_eq!(grown.capabilities(), &capabilities(&["yard", "yard.heavy"]));
    assert_eq!(
        grown.capacity_revision(),
        opened.capacity_revision(),
        "no order is interrupted"
    );
    assert_eq!(
        grown.cost().map(|c| c.margin_permille),
        Some(300),
        "re-termed in the same bump"
    );
    assert_eq!(
        grown.policy_revision(),
        opened.policy_revision() + 1,
        "the re-term's bump alone"
    );
    // Drift, each refused before a write: a dropped capability; a grown set with another slot
    // count; more capabilities that are not a superset (one swapped); a grown set under another
    // owner.
    let elsewhere = FacilityRecord::new(
        room,
        PrincipalId::Object(pat),
        capabilities(&["yard", "yard.heavy", "yard.ship"]),
        2,
    )
    .and_then(|r| r.with_authored_cost(cost(300)))
    .map(|r| r.with_authored_customers(mush_industry::Customers::Public))
    .expect("an authored record");
    for drifted in [
        authored(&["yard"], 300, 2),
        authored(&["yard", "yard.heavy", "yard.ship"], 300, 3),
        authored(&["yard.heavy", "yard.ship", "yard.slip"], 300, 2),
        elsewhere,
    ] {
        assert!(install(&mut tw, drifted).is_err(), "drift is refused");
        assert_eq!(facility(&tw, room), grown, "the refusal wrote nothing");
    }
    // Sold, then grown: the content's capabilities, the buyer's owner and terms.
    staff_hand(&mut tw, |ctx| {
        mush_industry::store::sell_facility(ctx, room, PrincipalId::Object(pat), false)
    })
    .expect("sold");
    let sold = facility(&tw, room);
    install(
        &mut tw,
        authored(&["yard", "yard.heavy", "yard.ship"], 250, 2),
    )
    .expect("the lane grows a held facility");
    let held = facility(&tw, room);
    assert_eq!(
        held.capabilities(),
        &capabilities(&["yard", "yard.heavy", "yard.ship"])
    );
    assert_eq!(
        (
            held.owner(),
            held.holding(),
            held.cost(),
            held.policy_revision(),
            held.capacity_revision()
        ),
        (
            sold.owner(),
            sold.holding(),
            sold.cost(),
            sold.policy_revision(),
            sold.capacity_revision()
        ),
        "the buyer's owner, terms and revisions stand"
    );
}
