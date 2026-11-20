//! Tier 1B security hardening regression tests for `mush-industry`:
//! - CRIT-14: terminal `WorkOrder` pruning, fail-closed truncated scans, and
//!   point-lookup delivery/active checks (no 32-entry denylist bypass).
//! - CRIT-15: `WoSubmit` server-side binding to verified live `HoldId`, facility
//!   authz/quarantine/revision, and non-drifted recipe digest + pinned lines.
//! - HIGH-33: scheduled (`SlotExpiryScheduleAdapter`) and operator (`@wo/reclaim`)
//!   reclaim of elapsed facility slot holds via `expire_slot`.
//! - HIGH-34: `claim.issuer() == deposit.jurisdiction()`, `claims_over` beyond
//!   256 world claims, terminal claim pruning past `MAX_CLAIMS_PER_SUBJECT = 8`,
//!   and pre-mint validation in `@claim/file`.
//! - HIGH-35: `ExtractStop` gated on `acts_for(run.operator())` or staff.
//! - HIGH-36: `@wo/submit` checks `order.requester() == Object(actor)` and `Draft`
//!   state before calling `money::place`.
//! - HIGH-37: `ClaimMove` + `@claim/transfer` pre-checks (`Transferable`,
//!   unencumbered, live E4 title match), `claim.revision()` in transfer
//!   idempotency key, and `Bound` claim title-owner authorization check.

#![allow(clippy::too_many_lines)]

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use mush_core::{
    CanonicalId, ClockStamp, CommitToken, DefinitionDigest, Engine, GOD, IdempotencyKey,
    IdempotencyScope, ObjId, ObjKind, OfferKeyRing, OfferSecret, RequestUid, ResolutionKeyRing,
    ResolutionSecret, ScheduleDriveCause, ScopedIdempotencyKey, World,
};
use mush_economy::{AccountId, EconomicVisibility, EconomyStore, VaultId};
use mush_industry::facility::facility_capacity_key;
use mush_industry::provider::{IndustryVerb, action_ref};
use mush_industry::record::{
    Accessibility, BandId, ClaimId, ClaimRecord, ClaimSubject, ContestState, CurvePoint,
    Exclusivity, MAX_CLAIMS_PER_SUBJECT, OreDeposit, OreDepositId, RunState, Transferability,
    YieldCurve,
};
use mush_industry::workorder::{
    DeliveryTarget, MAX_RETAINED_TERMINAL_DRAFT_CANCELS, PinnedRecipe, RecipeLine, RecipeRef,
    WorkOrder, WorkOrderId, WorkOrderState,
};
use mush_industry::{
    IndustryError, IndustryFamilies, IndustryHandles, IndustryModule, IndustryStore, money,
};
use mush_inventory::{EndpointRef, Grade, InventoryHandleCell, InventoryModule};
use mush_org::PrincipalId;
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, text_of};
use serde_json::json;

// ---- Shared test rig -------------------------------------------------------

struct ToggleRecipes {
    drifted: Arc<AtomicBool>,
}

impl ToggleRecipes {
    fn digest(&self) -> DefinitionDigest {
        if self.drifted.load(Ordering::Relaxed) {
            DefinitionDigest::from_bytes([0x22; 32])
        } else {
            DefinitionDigest::from_bytes([0x11; 32])
        }
    }

    fn alloy(&self) -> (RecipeRef, PinnedRecipe) {
        let recipe = RecipeRef::new(cid("tacsim.core"), 1, cid("alloy_smelt"), self.digest())
            .expect("recipe ref");
        let pinned = PinnedRecipe::new(
            vec![RecipeLine::new(cid("metals"), 4).expect("input")],
            vec![RecipeLine::new(cid("structural_alloys"), 1).expect("output")],
            caps(&["smelt"]),
            3,
        )
        .expect("pinned");
        (recipe, pinned)
    }
}

impl mush_industry::RecipeAuthority for ToggleRecipes {
    fn digest_of(&self, _world: &World, recipe: &RecipeRef) -> Option<DefinitionDigest> {
        (recipe.pack().as_str() == "tacsim.core"
            && recipe.version() == 1
            && recipe.id().as_str() == "alloy_smelt")
            .then(|| self.digest())
    }
    fn resolve(
        &self,
        _world: &World,
        pack: &CanonicalId,
        version: u32,
        id: &CanonicalId,
    ) -> Option<(RecipeRef, PinnedRecipe)> {
        (pack.as_str() == "tacsim.core" && version == 1 && id.as_str() == "alloy_smelt")
            .then(|| self.alloy())
    }
    fn catalog_revision(&self, _world: &World) -> u64 {
        if self.drifted.load(Ordering::Relaxed) {
            2
        } else {
            1
        }
    }
    fn catalog(&self, _world: &World) -> Vec<(RecipeRef, PinnedRecipe)> {
        vec![self.alloy()]
    }
}

fn cid(text: &str) -> CanonicalId {
    CanonicalId::new(text).expect("canonical id")
}

fn caps(ids: &[&str]) -> BTreeSet<CanonicalId> {
    ids.iter().map(|s| cid(s)).collect()
}

fn boot_full() -> (TestWorld, IndustryFamilies, Arc<AtomicBool>) {
    let drifted = Arc::new(AtomicBool::new(false));
    let cell: InventoryHandleCell = Arc::default();
    let mut world = TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(
            mush_economy::EconomyModule::new()
                .with_title_issuers(vec![mush_industry::industry_title_issuer()]),
        ),
        Box::new(
            InventoryModule::new()
                .with_quantity_delegates(["industry"])
                .with_handle_cell(Arc::clone(&cell)),
        ),
        Box::new(
            IndustryModule::new()
                .with_inventory(Arc::clone(&cell))
                .with_recipes(Arc::new(ToggleRecipes {
                    drifted: Arc::clone(&drifted),
                }))
                .with_extraction_licence(
                    mush_economy::InstrumentId::new("licence").expect("licence instrument"),
                ),
        ),
    ]);
    world.modules_loaded(HashMap::new());
    world.engine_mut().set_offer_keys(
        OfferKeyRing::new(OfferSecret::new(1, [0x41; 32]).expect("offer secret"), [])
            .expect("offer ring"),
    );
    world.engine_mut().set_resolution_keys(
        ResolutionKeyRing::new(
            ResolutionSecret::new(1, [0x52; 32]).expect("res secret"),
            [],
        )
        .expect("res ring"),
    );
    let families = world
        .world()
        .module_state::<IndustryHandles>()
        .expect("industry handles")
        .0
        .clone();
    (world, families, drifted)
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
    assert!(!has_error(&ev), "`{line}` refused: {}", texts(&ev));
    texts(&ev)
}

fn err(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    let ev = tw.run(actor, line);
    assert!(
        has_error(&ev),
        "`{line}` should have refused, got: {}",
        texts(&ev)
    );
    texts(&ev)
}

fn snap(tw: &TestWorld) -> &mush_economy::EconomySnapshot {
    tw.world()
        .module_state::<EconomyStore>()
        .expect("economy")
        .snapshot()
}

fn apply<T>(
    engine: &mut Engine,
    families: &IndustryFamilies,
    body: impl FnOnce(&mut mush_core::ActionTxn<'_>) -> Result<T, IndustryError>,
) -> Result<T, IndustryError> {
    let (result, prepared) = {
        let mut txn = engine
            .action_transaction(&families.deposits)
            .expect("industry txn");
        let result = body(&mut txn);
        match result {
            Ok(val) => (Ok(val), txn.prepare().ok()),
            Err(err) => (Err(err), None),
        }
    };
    if let Some(prepared) = prepared {
        let (_, installation) = prepared.into_parts();
        engine
            .install_prepared_action(installation)
            .expect("install");
    }
    result
}

fn read<T>(
    engine: &mut Engine,
    families: &IndustryFamilies,
    body: impl FnOnce(&mut mush_core::ActionTxn<'_>) -> T,
) -> T {
    let mut txn = engine
        .action_transaction(&families.deposits)
        .expect("industry txn");
    body(&mut txn)
}

fn land(tw: &mut TestWorld, token: u64) -> Result<serde_json::Value, String> {
    let submit = tw
        .engine_mut()
        .take_authoritative_submit()
        .ok_or_else(|| "nothing staged".to_owned())?;
    submit_direct(tw, submit.actor, submit.action, submit.parameters, token)
}

fn submit_direct(
    tw: &mut TestWorld,
    actor: ObjId,
    action: mush_core::DefinitionRef,
    parameters: serde_json::Value,
    token: u64,
) -> Result<serde_json::Value, String> {
    let mut bytes = [0_u8; 16];
    bytes[..8].copy_from_slice(&token.to_be_bytes());
    bytes[8..16].copy_from_slice(&token.wrapping_mul(0x9E37_79B9_7F4A_7C15).to_be_bytes());
    let req = RequestUid::from_bytes(bytes);
    let commit = CommitToken::from_bytes(bytes);
    let (resolved_action, resolved) = {
        let engine = tw.engine();
        let query = engine
            .legal_action_query(
                mush_core::PrincipalId::Object { id: actor },
                actor,
                mush_core::PerceivedRef::SelfActor,
            )
            .map_err(|e| format!("{e:?}"))?;
        let offers = engine
            .project_legal_actions(&query)
            .map_err(|e| format!("{e:?}"))?;
        let projected = offers
            .projected_for_action(&action)
            .ok_or_else(|| "action not offered".to_owned())?;
        let submission = mush_core::OfferSubmission {
            offer: projected.offer(),
            subject: projected.subject().clone(),
            targets: Vec::new(),
            parameters,
        };
        let execution = mush_core::OfferExecutionContext {
            request: req,
            principal: mush_core::PrincipalId::Object { id: actor },
            actor,
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal: mush_core::PrincipalId::Object { id: actor },
                    owner: action.key().owner.clone(),
                    namespace: cid(action.key().id.as_str()),
                },
                1,
                IdempotencyKey::Request(req),
            ),
            viewer_scope: query.viewer_scope(),
            now: None,
        };
        let resolved = engine
            .resolve_legal_action(&offers, execution, submission)
            .map_err(|e| format!("{e:?}"))?;
        let (a, r, _g) = resolved.into_authoritative_parts();
        (a, r)
    };
    let (prepared, outcome) = tw
        .engine_mut()
        .prepare_registered_resolved_authoritative_action(&resolved_action, &resolved, commit)
        .map_err(|e| format!("{e:?}"))?;
    let (_, installation) = prepared.into_parts();
    tw.engine_mut()
        .install_prepared_action(installation)
        .map_err(|e| format!("{e:?}"))?;
    Ok(outcome.outcome().data().clone())
}

fn verb_offered(tw: &TestWorld, actor: ObjId, verb: IndustryVerb) -> bool {
    let engine = tw.engine();
    let Some(action) = action_ref(&engine.registry, verb) else {
        return false;
    };
    let Ok(query) = engine.legal_action_query(
        mush_core::PrincipalId::Object { id: actor },
        actor,
        mush_core::PerceivedRef::SelfActor,
    ) else {
        return false;
    };
    let Ok(offers) = engine.project_legal_actions(&query) else {
        return false;
    };
    offers.projected_for_action(&action).is_some()
}

fn open_account_with_funds(
    tw: &mut TestWorld,
    owner: ObjId,
    faucet: AccountId,
    funds: i64,
) -> AccountId {
    let before: BTreeSet<u64> = snap(tw).accounts().map(|a| a.id.0).collect();
    ok(tw, GOD, &format!("@account/open #{}=gold", owner.0));
    let id = snap(tw)
        .accounts()
        .map(|a| a.id)
        .find(|id| !before.contains(&id.0))
        .expect("fresh account");
    if funds > 0 {
        ok(tw, GOD, &format!("@econ/mint {faucet}={id} {funds}"));
    }
    id
}

fn register_licence_instrument(tw: &mut TestWorld) {
    let mut out = mush_core::Outbox::new();
    let engine = tw.engine_mut();
    let mut ctx = mush_core::Ctx {
        w: &mut engine.world,
        actor: GOD,
        args: mush_core::ParsedArgs::parse(mush_core::ArgShape::Raw, ""),
        switches: mush_core::SwitchSet::default(),
        out: &mut out,
        reg: &engine.registry,
        module: Some(smol_str::SmolStr::new("economy")),
        typed_state: mush_core::ModuleActionState::unavailable(),
    };
    mush_economy::register_instrument(
        &mut ctx,
        mush_economy::InstrumentDef {
            id: mush_economy::InstrumentId::new("licence").expect("id"),
            ruleset: smol_str::SmolStr::new_static("test"),
            code: smol_str::SmolStr::new_static("TLIC"),
            kind: mush_economy::InstrumentKind::Entitlement,
            scale: 0,
            transferable: false,
            negative_policy: mush_economy::NegativePolicy::Forbid,
            status: mush_economy::InstrumentStatus::Active,
            display: None,
            revision: 1,
        },
    )
    .expect("register entitlement instrument");
}

fn make_endpoint(tw: &mut TestWorld, name: &str, owner: ObjId) -> EndpointRef {
    let obj = tw
        .world_mut()
        .create(ObjKind::Thing, name, owner, Some(mush_core::LIMBO))
        .expect("vault obj");
    let before: BTreeSet<u64> = snap(tw).vaults().map(|v| v.id.0).collect();
    ok(
        tw,
        GOD,
        &format!(
            "@vault/create {name}=container:#{} owner=#{}",
            obj.0, owner.0
        ),
    );
    let fresh = snap(tw)
        .vaults()
        .map(|v| v.id)
        .find(|id| !before.contains(&id.0))
        .expect("fresh vault");
    EndpointRef(fresh)
}

fn setup_wo_scene(
    slots: u32,
    funds: i64,
) -> (
    TestWorld,
    IndustryFamilies,
    Arc<AtomicBool>,
    AccountId,
    AccountId,
    EndpointRef,
    EndpointRef,
) {
    let (mut tw, families, drifted) = boot_full();
    ok(&mut tw, GOD, "@instrument/register gold=0");
    ok(&mut tw, GOD, "@account/system faucet=gold");
    let faucet = snap(&tw)
        .accounts()
        .find(|a| matches!(a.owner, PrincipalId::System(_)))
        .expect("faucet")
        .id;
    let budget = open_account_with_funds(&mut tw, GOD, faucet, funds);
    let _payee = open_account_with_funds(&mut tw, GOD, faucet, 0);
    // Pad vault 0 so real source/dest have non-zero ids.
    let _pad = make_endpoint(&mut tw, "pad", GOD);
    let source = make_endpoint(&mut tw, "source", GOD);
    let dest = make_endpoint(&mut tw, "dest", GOD);
    ok(&mut tw, GOD, &format!("@facility/open 7={slots} smelt"));
    let _ = tw.engine_mut().take_authoritative_submit();
    // Stock source pool with metals.
    let engine = tw.engine_mut();
    apply(engine, &families, |txn| {
        families
            .inventory
            .credit(txn, source, &cid("metals"), 10_000)
            .map(|_| ())
            .map_err(IndustryError::Inventory)
    })
    .expect("stock metals");
    (tw, families, drifted, faucet, budget, source, dest)
}

fn count_work_orders(tw: &TestWorld, families: &IndustryFamilies) -> usize {
    let state = tw.engine().action_state();
    let mut cursor: Option<String> = None;
    let mut count = 0;
    loop {
        let page = state
            .scan_family(&families.work_orders, cursor.as_deref(), 64)
            .expect("scan work_orders");
        count += page.entries().len();
        match page.next_after().map(ToString::to_string) {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    count
}

fn slot_available(tw: &mut TestWorld, families: &IndustryFamilies, facility: ObjId) -> u64 {
    let key = facility_capacity_key(facility).expect("cap key");
    let state = tw.engine().action_state();
    let Some(scalar) = state.get(&families.facility_slots, &key).expect("scalar") else {
        return 0;
    };
    let Some(book) = state.get(&families.facility_books, &key).expect("book") else {
        return 0;
    };
    book.checked_available(*scalar).expect("available")
}

fn seed_running_order(
    tw: &mut TestWorld,
    families: &IndustryFamilies,
    raw_id: u64,
    requester: ObjId,
    budget: AccountId,
    source: EndpointRef,
    dest: EndpointRef,
) -> WorkOrderId {
    let (recipe, pinned) = ToggleRecipes {
        drifted: Arc::new(AtomicBool::new(false)),
    }
    .alloy();
    let id = WorkOrderId(raw_id);
    let draft = WorkOrder::draft(
        id,
        PrincipalId::Object(requester),
        PrincipalId::Object(requester),
        ObjId(7),
        2,
        1,
        1,
        recipe,
        pinned,
        1,
        budget,
        Some(source),
        DeliveryTarget::Endpoint { endpoint: dest },
        EconomicVisibility::Parties,
    )
    .expect("draft");
    let running = draft.with_state(WorkOrderState::Running).expect("running");
    apply(tw.engine_mut(), families, |txn| {
        families.open_work_order(txn, running)
    })
    .expect("insert running");
    id
}

// ---- CRIT-14 tests ---------------------------------------------------------

#[test]
fn crit14_draft_cancel_loop_prunes_terminal_orders_and_stays_bounded() {
    let (mut tw, families, _drifted, _faucet, budget, source, dest) = setup_wo_scene(2, 10_000);
    let mut tok = 100u64;
    for _ in 0..80 {
        ok(
            &mut tw,
            GOD,
            &format!(
                "@wo/draft tacsim.core/alloy_smelt/1=7 1 {} {} {}",
                budget.0, source.0.0, dest.0.0
            ),
        );
        tok += 1;
        let drafted = land(&mut tw, tok).expect("draft lands");
        let order_id = drafted
            .get("order")
            .and_then(|v| v.as_str())
            .expect("order id")
            .to_owned();
        ok(&mut tw, GOD, &format!("@wo/cancel {order_id}"));
        tok += 1;
        land(&mut tw, tok).expect("cancel lands");
    }
    let retained = count_work_orders(&tw, &families);
    assert!(
        retained <= MAX_RETAINED_TERMINAL_DRAFT_CANCELS,
        "80 draft+cancel loops must prune terminal draft cancels to <= {MAX_RETAINED_TERMINAL_DRAFT_CANCELS}, got {retained}"
    );
    // Boot reconcile stays complete (not flipped to complete=false).
    tw.modules_loaded(HashMap::new());
    let store = tw.world().module_state::<IndustryStore>().expect("store");
    assert!(store.is_complete(), "store must remain complete");
    assert!(!store.orders_truncated(), "work orders must not truncate");
}

#[test]
fn crit14_over_32_unfunded_orders_never_bypass_deliver_and_truncated_scan_fails_closed() {
    let (mut tw, families, _drifted, _faucet, budget, source, dest) = setup_wo_scene(2, 10_000);
    let mut unfunded_ids = Vec::new();
    for i in 0..35u64 {
        let id = seed_running_order(&mut tw, &families, 0xA000 + i, GOD, budget, source, dest);
        unfunded_ids.push(id);
    }
    // None of the 35 unfunded orders may be deliverable (old 32-entry denylist
    // would have let orders #33..#35 bypass!).
    assert!(
        !verb_offered(&tw, GOD, IndustryVerb::WoDeliver),
        "35 unfunded running orders must yield no WoDeliver offer"
    );
    let deliver_action = action_ref(&tw.engine().registry, IndustryVerb::WoDeliver).expect("act");
    let last = unfunded_ids[34];
    let attempt = submit_direct(
        &mut tw,
        GOD,
        deliver_action.clone(),
        json!({ "order": last.to_string() }),
        500,
    );
    assert!(
        attempt.is_err(),
        "35th unfunded order must NEVER bypass delivery check: {attempt:?}"
    );

    // Now seed >4,096 terminal work orders so boot reconcile truncates the scan,
    // and assert `@wo/draft`, `WoDraft`, and `WoDeliver` ALL fail closed!
    let (recipe, pinned) = ToggleRecipes {
        drifted: Arc::new(AtomicBool::new(false)),
    }
    .alloy();
    {
        let engine = tw.engine_mut();
        for batch in 0..130u64 {
            apply(engine, &families, |txn| {
                for offset in 0..32u64 {
                    let i = batch * 32 + offset;
                    let id = WorkOrderId(0xD0000 + i);
                    let order = WorkOrder::draft(
                        id,
                        PrincipalId::Object(GOD),
                        PrincipalId::Object(GOD),
                        ObjId(7),
                        2,
                        1,
                        1,
                        recipe.clone(),
                        pinned.clone(),
                        1,
                        budget,
                        Some(source),
                        DeliveryTarget::Endpoint { endpoint: dest },
                        EconomicVisibility::Parties,
                    )
                    .expect("draft")
                    .with_state(WorkOrderState::Cancelled)
                    .expect("cancelled");
                    txn.insert(&families.work_orders, &id, order)?;
                }
                Ok(())
            })
            .expect("seed terminal batch");
        }
    }
    // Open a clean facility #8 (not quarantined by the seeded running orders on #7)
    // so `@wo/draft` reaches `active_work_orders` and fails closed on `orders_truncated`.
    tw.modules_loaded(HashMap::new());
    ok(&mut tw, GOD, "@instrument/register gold=0");
    ok(&mut tw, GOD, "@account/system faucet=gold");
    let faucet2 = snap(&tw)
        .accounts()
        .find(|a| matches!(a.owner, PrincipalId::System(_)))
        .expect("faucet2")
        .id;
    let budget2 = open_account_with_funds(&mut tw, GOD, faucet2, 10_000);
    let _payee2 = open_account_with_funds(&mut tw, GOD, faucet2, 0);
    let _pad2 = make_endpoint(&mut tw, "pad2", GOD);
    let source2 = make_endpoint(&mut tw, "source2", GOD);
    let dest2 = make_endpoint(&mut tw, "dest2", GOD);
    ok(&mut tw, GOD, "@facility/open 8=2 smelt");
    let _ = tw.engine_mut().take_authoritative_submit();
    let store = tw.world().module_state::<IndustryStore>().expect("store");
    assert!(
        store.orders_truncated(),
        "reconcile must record orders_truncated when work_orders exceeds 4,096"
    );
    assert!(
        !verb_offered(&tw, GOD, IndustryVerb::WoDraft),
        "WoDraft offer must fail closed when orders_truncated"
    );
    assert!(
        !verb_offered(&tw, GOD, IndustryVerb::WoDeliver),
        "WoDeliver offer must fail closed when orders_truncated"
    );
    let draft_err = err(
        &mut tw,
        GOD,
        &format!(
            "@wo/draft tacsim.core/alloy_smelt/1=8 1 {} {} {}",
            budget2.0, source2.0.0, dest2.0.0
        ),
    );
    assert!(
        draft_err.contains("maximum number of open work orders"),
        "@wo/draft must fail closed on truncated work-order scan: {draft_err}"
    );
}

// ---- CRIT-15 test ----------------------------------------------------------

#[test]
fn crit15_direct_wo_submit_forged_hold_quarantine_revision_and_drift_refuse() {
    let (mut tw, families, drifted, _faucet, budget, source, dest) = setup_wo_scene(1, 10_000);
    // Draft one work order via the real command + offer path.
    ok(
        &mut tw,
        GOD,
        &format!(
            "@wo/draft tacsim.core/alloy_smelt/1=7 1 {} {} {}",
            budget.0, source.0.0, dest.0.0
        ),
    );
    let drafted = land(&mut tw, 1001).expect("draft lands");
    let order_str = drafted["order"].as_str().expect("order id").to_owned();
    let order_id = WorkOrderId::parse(&order_str).expect("parse wo id");
    let submit_action = action_ref(&tw.engine().registry, IndustryVerb::WoSubmit).expect("act");

    // 1. No economy hold exists: direct WoSubmit with a forged non-zero hold MUST refuse!
    assert!(
        !verb_offered(&tw, GOD, IndustryVerb::WoSubmit),
        "WoSubmit must NOT be offered when no live economy hold exists"
    );
    let forged = submit_direct(
        &mut tw,
        GOD,
        submit_action.clone(),
        json!({ "order": order_str, "hold": 999_999 }),
        1002,
    );
    assert!(
        forged.is_err(),
        "forged non-zero hold must be refused: {forged:?}"
    );
    let state = read(tw.engine_mut(), &families, |txn| {
        *families.work_order(txn, order_id).unwrap().unwrap().state()
    });
    assert_eq!(state, WorkOrderState::Draft, "order must remain Draft");

    // 2. Place a REAL hold via `@wo/submit`, inspect the real hold id, discard the
    // staged submit, and try submitting a MISMATCHED hold id directly.
    ok(&mut tw, GOD, &format!("@wo/submit {order_str}"));
    let staged = tw
        .engine_mut()
        .take_authoritative_submit()
        .expect("staged submit");
    let real_hold = staged.parameters["hold"].as_u64().expect("real hold id");
    assert_ne!(real_hold, 0);
    assert!(
        verb_offered(&tw, GOD, IndustryVerb::WoSubmit),
        "WoSubmit IS offered once a real hold stands"
    );
    let wrong_hold = submit_direct(
        &mut tw,
        GOD,
        submit_action.clone(),
        json!({ "order": order_str, "hold": real_hold + 1 }),
        1003,
    );
    assert!(
        wrong_hold.is_err(),
        "mismatched hold id must be refused: {wrong_hold:?}"
    );

    // 3. Recipe drift drops WoSubmit from legal offers even with a live hold!
    drifted.store(true, Ordering::Relaxed);
    assert!(
        !verb_offered(&tw, GOD, IndustryVerb::WoSubmit),
        "drifted recipe must drop WoSubmit offer"
    );
    drifted.store(false, Ordering::Relaxed);
    assert!(verb_offered(&tw, GOD, IndustryVerb::WoSubmit));

    // 4. Facility policy/capacity revision change drops WoSubmit from legal offers!
    ok(&mut tw, GOD, "@facility/terms 7=public");
    assert!(
        !verb_offered(&tw, GOD, IndustryVerb::WoSubmit),
        "facility policy revision bump must drop stale draft from WoSubmit offer"
    );

    // 5. Draft a fresh order matching current facility revision, place real hold,
    // and submit with the matching real hold -> succeeds!
    ok(
        &mut tw,
        GOD,
        &format!(
            "@wo/draft tacsim.core/alloy_smelt/1=7 1 {} {} {}",
            budget.0, source.0.0, dest.0.0
        ),
    );
    let drafted2 = land(&mut tw, 1004).expect("fresh draft lands");
    let order2_str = drafted2["order"].as_str().expect("order2").to_owned();
    let order2_id = WorkOrderId::parse(&order2_str).expect("parse order2");
    ok(&mut tw, GOD, &format!("@wo/submit {order2_str}"));
    land(&mut tw, 1005).expect("honest submit with verified live hold succeeds");
    let state2 = read(tw.engine_mut(), &families, |txn| {
        *families
            .work_order(txn, order2_id)
            .unwrap()
            .unwrap()
            .state()
    });
    assert_eq!(state2, WorkOrderState::Running);
}

// ---- HIGH-33 test ----------------------------------------------------------

#[test]
fn high33_elapsed_slot_reclaimed_by_schedule_and_operator_reclaim() {
    let (mut tw, families, _drifted, faucet, _god_budget, _god_src, _god_dst) =
        setup_wo_scene(1, 10_000);
    let alice = tw.spawn_player("Alice");
    let bob = tw.spawn_player("Bob");
    let alice_budget = open_account_with_funds(&mut tw, alice, faucet, 10_000);
    let _alice_payee = open_account_with_funds(&mut tw, alice, faucet, 0);
    let alice_src = make_endpoint(&mut tw, "alice_src", alice);
    let alice_dst = make_endpoint(&mut tw, "alice_dst", alice);
    ok(&mut tw, GOD, &format!("@facility/owner 7=#{}", alice.0));
    apply(tw.engine_mut(), &families, |txn| {
        families
            .inventory
            .credit(txn, alice_src, &cid("metals"), 100)
            .map(|_| ())
            .map_err(IndustryError::Inventory)
    })
    .expect("stock alice metals");

    // Part A: Scheduled expiry via SlotExpiryScheduleAdapter.
    ok(
        &mut tw,
        alice,
        &format!(
            "@wo/draft tacsim.core/alloy_smelt/1=7 1 {} {} {}",
            alice_budget.0, alice_src.0.0, alice_dst.0.0
        ),
    );
    let d1 = land(&mut tw, 2001).expect("draft1");
    let order1_str = d1["order"].as_str().expect("order1").to_owned();
    let order1_id = WorkOrderId::parse(&order1_str).expect("parse order1");
    ok(&mut tw, alice, &format!("@wo/submit {order1_str}"));
    land(&mut tw, 2002).expect("submit1");
    assert_eq!(
        slot_available(&mut tw, &families, ObjId(7)),
        0,
        "sole slot held"
    );
    let metals_held = read(tw.engine_mut(), &families, |txn| {
        families
            .inventory
            .available(txn, alice_src, &cid("metals"))
            .expect("avail")
    });
    assert_eq!(metals_held, 96, "4 metals reserved");

    // Advance IC clock past the slot reservation's deadline (work lead + 7-day TTL).
    let slot_deadline = {
        let key = facility_capacity_key(ObjId(7)).expect("key");
        let book = tw
            .engine()
            .action_state()
            .get(&families.facility_books, &key)
            .expect("book")
            .expect("some book");
        book.records()[0].expires().at
    };
    tw.set_now(slot_deadline + 60);

    // Discover due scheduled actions and drive `slot_expiry` (`industry_wo_reclaim`).
    let candidates = tw
        .engine()
        .discover_scheduled_actions(ScheduleDriveCause::Live, None, 32)
        .expect("discover")
        .candidates()
        .to_vec();
    let expiry_cand = candidates
        .into_iter()
        .find(|c| c.action().key().id.as_str() == mush_industry::provider::ACTION_WO_RECLAIM)
        .expect("slot_expiry candidate must be discovered once deadline elapses");
    let ingress =
        mush_core::scheduled_action_ingress(expiry_cand.action(), expiry_cand.advance(), 1)
            .expect("ingress");
    let plan = tw
        .engine()
        .plan_scheduled_action(&expiry_cand, &ingress)
        .expect("plan slot expiry");
    let (prepared, _outcome) = tw
        .engine_mut()
        .prepare_registered_scheduled_authoritative_action(
            &expiry_cand,
            plan.invocation(),
            CommitToken::from_bytes([0x33; 16]),
        )
        .expect("prepare scheduled WoReclaim");
    let (_, installation) = prepared.into_parts();
    tw.engine_mut()
        .install_prepared_action(installation)
        .expect("install scheduled WoReclaim");

    assert_eq!(
        slot_available(&mut tw, &families, ObjId(7)),
        1,
        "scheduled WoReclaim must expire and free the slot"
    );
    let metals_after = read(tw.engine_mut(), &families, |txn| {
        families
            .inventory
            .available(txn, alice_src, &cid("metals"))
            .expect("avail")
    });
    assert_eq!(metals_after, 100, "material reservation must be released");
    let state1 = read(tw.engine_mut(), &families, |txn| {
        *families
            .work_order(txn, order1_id)
            .unwrap()
            .unwrap()
            .state()
    });
    assert_eq!(
        state1,
        WorkOrderState::Failed,
        "abandoned order must transition to Failed"
    );
    // Fire money sweep so budget hold releases too.
    let now = tw.world().now() + 60;
    tw.set_now(now);
    let _ = tw.fire_due(now * 1000);
    assert!(
        money::live_hold_in(tw.world(), order1_id).is_none(),
        "money sweep must release budget hold of Failed reclaimed order"
    );

    // Part B: Manual operator `@wo/reclaim <facility>` (and stranger refusal).
    ok(
        &mut tw,
        alice,
        &format!(
            "@wo/draft tacsim.core/alloy_smelt/1=7 1 {} {} {}",
            alice_budget.0, alice_src.0.0, alice_dst.0.0
        ),
    );
    let d2 = land(&mut tw, 2003).expect("draft2");
    let order2_str = d2["order"].as_str().expect("order2").to_owned();
    let order2_id = WorkOrderId::parse(&order2_str).expect("parse order2");
    ok(&mut tw, alice, &format!("@wo/submit {order2_str}"));
    land(&mut tw, 2004).expect("submit2");
    assert_eq!(slot_available(&mut tw, &families, ObjId(7)), 0);

    let deadline2 = {
        let key = facility_capacity_key(ObjId(7)).expect("key");
        let book = tw
            .engine()
            .action_state()
            .get(&families.facility_books, &key)
            .expect("book")
            .expect("some book");
        book.records()
            .iter()
            .find(|r| r.state() == mush_core::ReservationState::Held)
            .expect("held slot")
            .expires()
            .at
    };
    tw.set_now(deadline2 + 60);

    // Stranger Bob is refused existence-hidingly.
    let stranger_err = err(&mut tw, bob, "@wo/reclaim 7");
    assert_eq!(stranger_err, mush_industry::commands::NO_SUCH_FACILITY);
    assert!(tw.engine_mut().take_authoritative_submit().is_none());

    // Facility owner Alice reclaims manually via `@wo/reclaim 7`.
    ok(&mut tw, alice, "@wo/reclaim 7");
    land(&mut tw, 2005).expect("manual @wo/reclaim lands");
    assert_eq!(
        slot_available(&mut tw, &families, ObjId(7)),
        1,
        "manual @wo/reclaim must free the elapsed slot"
    );
    let state2 = read(tw.engine_mut(), &families, |txn| {
        *families
            .work_order(txn, order2_id)
            .unwrap()
            .unwrap()
            .state()
    });
    assert_eq!(state2, WorkOrderState::Failed);
}

// ---- HIGH-34 test ----------------------------------------------------------

#[test]
fn high34_jurisdiction_enforcement_over_256_claims_pruning_and_no_orphan_mints() {
    let (mut tw, families, _drifted) = boot_full();
    let alice = tw.spawn_player("Alice");
    let place = tw
        .world_mut()
        .create(ObjKind::Room, "Belt Alpha", GOD, None)
        .expect("place");

    // Create two orgs: Registry (jurisdiction) and Alice's player org.
    ok(&mut tw, GOD, "@org/create Registry");
    let reg_org = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .expect("orgs")
        .state()
        .orgs()
        .find(|o| o.name == "Registry")
        .expect("registry org")
        .id;
    ok(&mut tw, GOD, "@org/create Freebooters");
    ok(
        &mut tw,
        GOD,
        &format!("@org/grant #{}=Freebooters economy.account.post", alice.0),
    );
    let player_org = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .expect("orgs")
        .state()
        .orgs()
        .find(|o| o.name == "Freebooters")
        .expect("player org")
        .id;

    // Open a deposit governed by `Registry` jurisdiction.
    let dep_id = OreDepositId(0x3401);
    let dep = OreDeposit::new(
        dep_id,
        place,
        cid("metals"),
        Grade::new(1).expect("grade"),
        Accessibility::new(100).expect("acc"),
        BandId::new("high").expect("gband"),
        BandId::new("rich").expect("rband"),
        YieldCurve::new(vec![CurvePoint {
            cumulative: 0,
            per_cycle: 100,
        }])
        .expect("curve"),
        EndpointRef(VaultId(501)),
        10_000,
    )
    .expect("deposit")
    .with_jurisdiction(PrincipalId::Organization(reg_org));
    apply(tw.engine_mut(), &families, |txn| {
        families.open_deposit(txn, dep.clone())
    })
    .expect("open deposit");

    // 1. Non-staff Alice trying to file with her player org over a Registry-governed
    // deposit is refused BEFORE minting any world object or E4 title!
    let objs_before = tw.world().next_id();
    let titles_before = snap(&tw).assets().count();
    err(
        &mut tw,
        alice,
        &format!(
            "@claim/file {dep_id}=org:{}/obj:{}/shared/1/10000/transferable",
            player_org.0, alice.0
        ),
    );
    assert_eq!(
        tw.world().next_id(),
        objs_before,
        "foreign-jurisdiction @claim/file must mint ZERO world objects"
    );
    assert_eq!(
        snap(&tw).assets().count(),
        titles_before,
        "foreign-jurisdiction @claim/file must mint ZERO E4 titles"
    );

    // 2. Invalid `@claim/file` parameters (nonexistent deposit, bad priority, bad
    // exclusivity) also mint ZERO world objects and ZERO E4 titles!
    for bad_cmd in [
        format!(
            "@claim/file dep#0000000000009999=org:{}/obj:{}/shared/1/10000/transferable",
            reg_org.0, alice.0
        ),
        format!(
            "@claim/file {dep_id}=org:{}/obj:{}/shared/99999/10000/transferable",
            reg_org.0, alice.0
        ),
        format!(
            "@claim/file {dep_id}=org:{}/obj:{}/bogus/1/10000/transferable",
            reg_org.0, alice.0
        ),
    ] {
        err(&mut tw, GOD, &bad_cmd);
    }
    assert_eq!(tw.world().next_id(), objs_before);
    assert_eq!(snap(&tw).assets().count(), titles_before);

    // 3. Even if a foreign-issuer claim exists on the deposit (e.g. staff-filed),
    // `extraction_authorized` MUST reject it when `claim.issuer() != deposit.jurisdiction()`.
    let now = tw.world().world_ic_now();
    let future = ClockStamp {
        clock: now.clock.clone(),
        at: now.at + 100_000,
    };
    let foreign_claim = ClaimRecord::new(
        ClaimId(0x340A),
        ClaimSubject::Deposit { deposit: dep_id },
        PrincipalId::Organization(player_org),
        PrincipalId::Object(alice),
        ObjId(9300),
        Exclusivity::Shared,
        1,
        future.clone(),
        Transferability::Transferable,
    )
    .expect("foreign claim");
    assert!(
        mush_industry::licence::extraction_authorized(
            tw.world(),
            alice,
            &PrincipalId::Object(alice),
            &dep,
            dep.initial_reserve(),
            &[foreign_claim],
            None,
            &now,
        )
        .is_err(),
        "foreign-issuer claim must NOT authorize extraction on a jurisdiction-governed deposit"
    );

    // 4. Seed 270 unrelated claims (exceeding the old 256-claim scan cutoff!) and
    // file 1 valid jurisdiction-matching claim on `dep_id` -> `authorized_deposits`
    // MUST still find it via `claims_over`!
    apply(tw.engine_mut(), &families, |txn| {
        for i in 0..270u64 {
            let dummy = ClaimRecord::new(
                ClaimId(0x1000 + i),
                ClaimSubject::Place {
                    place: ObjId(20_000 + u32::try_from(i).expect("u32")),
                },
                PrincipalId::Organization(reg_org),
                PrincipalId::Object(GOD),
                ObjId(30_000 + u32::try_from(i).expect("u32")),
                Exclusivity::Shared,
                1,
                future.clone(),
                Transferability::Transferable,
            )
            .expect("dummy claim");
            families.file_claim(txn, dummy)?;
        }
        Ok(())
    })
    .expect("seed 270 unrelated claims");

    // Grant Alice an extraction license entitlement and a live endpoint vault.
    register_licence_instrument(&mut tw);
    ok(&mut tw, GOD, &format!("@account/open #{}=licence", alice.0));
    let _alice_end = make_endpoint(&mut tw, "alice_ore", alice);
    // File the real jurisdiction-matching claim for Alice on `dep_id`.
    ok(
        &mut tw,
        GOD,
        &format!(
            "@claim/file {dep_id}=org:{}/obj:{}/shared/1/100000/transferable",
            reg_org.0, alice.0
        ),
    );
    land(&mut tw, 3401).expect("jurisdiction claim lands");
    assert!(
        verb_offered(&tw, alice, IndustryVerb::ExtractStart),
        "ExtractStart must still be offered for Alice past 270 unrelated claims (claims_over)"
    );

    // 5. Fill a fresh deposit to `MAX_CLAIMS_PER_SUBJECT` (8) with 1 `Suspended`
    // claim and 7 expired/seized terminal claims, then file a 9th claim ->
    // succeeds by pruning a terminal claim while preserving the `Suspended` claim!
    let dep2_id = OreDepositId(0x3402);
    let dep2 = OreDeposit::new(
        dep2_id,
        place,
        cid("metals"),
        Grade::new(1).expect("grade"),
        Accessibility::new(100).expect("acc"),
        BandId::new("high").expect("gband"),
        BandId::new("rich").expect("rband"),
        YieldCurve::new(vec![CurvePoint {
            cumulative: 0,
            per_cycle: 100,
        }])
        .expect("curve"),
        EndpointRef(VaultId(502)),
        10_000,
    )
    .expect("deposit2")
    .with_jurisdiction(PrincipalId::Organization(reg_org));
    let past = ClockStamp {
        clock: now.clock.clone(),
        at: now.at - 10,
    };
    let suspended_id = ClaimId(0x34F0);
    apply(tw.engine_mut(), &families, |txn| {
        families.open_deposit(txn, dep2)?;
        let suspended = ClaimRecord::new(
            suspended_id,
            ClaimSubject::Deposit { deposit: dep2_id },
            PrincipalId::Organization(reg_org),
            PrincipalId::Object(alice),
            ObjId(9400),
            Exclusivity::Shared,
            1,
            future.clone(),
            Transferability::Transferable,
        )
        .expect("suspended")
        .with_contest_state(ContestState::Suspended {
            by: PrincipalId::Organization(reg_org),
        })
        .expect("suspend");
        families.file_claim(txn, suspended)?;
        Ok(())
    })
    .expect("open dep2 + suspended claim");
    for i in 1..MAX_CLAIMS_PER_SUBJECT {
        let c = ClaimRecord::new(
            ClaimId(0x34F0 + i as u64),
            ClaimSubject::Deposit { deposit: dep2_id },
            PrincipalId::Organization(reg_org),
            PrincipalId::Object(alice),
            ObjId(9400 + i as u32),
            Exclusivity::Shared,
            1,
            past.clone(),
            Transferability::Transferable,
        )
        .expect("expired");
        apply(tw.engine_mut(), &families, |txn| {
            families.file_claim(txn, c)
        })
        .expect("fill expired claim");
    }

    // Filing a 9th claim via `@claim/file` + `land` succeeds by pruning an expired claim.
    ok(
        &mut tw,
        GOD,
        &format!(
            "@claim/file {dep2_id}=org:{}/obj:{}/shared/1/100000/transferable",
            reg_org.0, alice.0
        ),
    );
    land(&mut tw, 3402).expect("9th claim succeeds by pruning terminal expired claim");
    let after_claims = read(tw.engine_mut(), &families, |txn| {
        families
            .claims_over(txn, ClaimSubject::Deposit { deposit: dep2_id })
            .expect("claims_over")
    });
    assert_eq!(
        after_claims.len(),
        2,
        "all 7 expired terminal claims pruned, leaving the 1 Suspended claim + 1 new live claim"
    );
    assert!(
        after_claims.iter().any(|c| c.id() == suspended_id),
        "Suspended claim must NEVER be pruned"
    );
}

// ---- HIGH-35 test ----------------------------------------------------------

#[test]
fn high35_extract_stop_requires_run_operator_or_staff() {
    let (mut tw, families, _drifted) = boot_full();
    let alice = tw.spawn_player("Alice");
    let bob = tw.spawn_player("Bob");
    let carol = tw.spawn_player("Carol"); // co-tenant with valid claim + license
    let place = tw
        .world_mut()
        .create(ObjKind::Room, "Mine", GOD, None)
        .expect("place");

    ok(&mut tw, GOD, "@org/create Authority");
    let auth_org = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .expect("orgs")
        .state()
        .orgs()
        .find(|o| o.name == "Authority")
        .expect("org")
        .id;

    let dep_id = OreDepositId(0x3501);
    let dep = OreDeposit::new(
        dep_id,
        place,
        cid("metals"),
        Grade::new(1).expect("grade"),
        Accessibility::new(100).expect("acc"),
        BandId::new("high").expect("gband"),
        BandId::new("rich").expect("rband"),
        YieldCurve::new(vec![CurvePoint {
            cumulative: 0,
            per_cycle: 50,
        }])
        .expect("curve"),
        EndpointRef(VaultId(601)),
        5_000,
    )
    .expect("deposit")
    .with_jurisdiction(PrincipalId::Organization(auth_org));
    apply(tw.engine_mut(), &families, |txn| {
        families.open_deposit(txn, dep)
    })
    .expect("open deposit");

    // License + shared claims for Alice and Carol; endpoint for Alice.
    register_licence_instrument(&mut tw);
    ok(&mut tw, GOD, &format!("@account/open #{}=licence", alice.0));
    ok(&mut tw, GOD, &format!("@account/open #{}=licence", carol.0));
    let alice_end = make_endpoint(&mut tw, "alice_ore", alice);
    ok(
        &mut tw,
        GOD,
        &format!(
            "@claim/file {dep_id}=org:{}/obj:{}/shared/1/100000/transferable",
            auth_org.0, alice.0
        ),
    );
    land(&mut tw, 3501).expect("alice claim");
    ok(
        &mut tw,
        GOD,
        &format!(
            "@claim/file {dep_id}=org:{}/obj:{}/shared/1/100000/transferable",
            auth_org.0, carol.0
        ),
    );
    land(&mut tw, 3502).expect("carol claim");

    // Alice starts an extraction run.
    ok(
        &mut tw,
        alice,
        &format!("@extract/start {dep_id}={}", alice_end.0.0),
    );
    land(&mut tw, 3503).expect("alice starts run");

    // Neither stranger Bob nor co-tenant Carol may stop Alice's run!
    assert!(!verb_offered(&tw, bob, IndustryVerb::ExtractStop));
    assert!(!verb_offered(&tw, carol, IndustryVerb::ExtractStop));
    ok(&mut tw, bob, &format!("@extract/stop {dep_id}"));
    assert!(
        land(&mut tw, 3504).is_err(),
        "stranger Bob must NOT stop Alice's extraction run"
    );
    ok(&mut tw, carol, &format!("@extract/stop {dep_id}"));
    assert!(
        land(&mut tw, 3505).is_err(),
        "co-tenant Carol must NOT stop Alice's extraction run"
    );
    let still_running = read(tw.engine_mut(), &families, |txn| {
        families.run(txn, dep_id).unwrap().unwrap().state()
    });
    assert_eq!(still_running, RunState::Running);

    // Operator Alice CAN stop her own run.
    assert!(verb_offered(&tw, alice, IndustryVerb::ExtractStop));
    ok(&mut tw, alice, &format!("@extract/stop {dep_id}"));
    land(&mut tw, 3506).expect("operator Alice stops her own run");
    let stopped = read(tw.engine_mut(), &families, |txn| {
        families.run(txn, dep_id).unwrap().unwrap().state()
    });
    assert_eq!(stopped, RunState::Stopped);

    // Alice restarts; staff (GOD) CAN stop it too.
    ok(
        &mut tw,
        alice,
        &format!("@extract/start {dep_id}={}", alice_end.0.0),
    );
    land(&mut tw, 3507).expect("alice restarts");
    assert!(verb_offered(&tw, GOD, IndustryVerb::ExtractStop));
    ok(&mut tw, GOD, &format!("@extract/stop {dep_id}"));
    land(&mut tw, 3508).expect("staff stops run");
    let stopped_by_staff = read(tw.engine_mut(), &families, |txn| {
        families.run(txn, dep_id).unwrap().unwrap().state()
    });
    assert_eq!(stopped_by_staff, RunState::Stopped);
}

// ---- HIGH-36 test ----------------------------------------------------------

#[test]
fn high36_wo_submit_checks_requester_and_draft_before_placing_hold() {
    let (mut tw, _families, _drifted, faucet, _god_budget, _god_src, _god_dst) =
        setup_wo_scene(2, 10_000);
    // Open facility to public so Bob can pass `may_order` if requester weren't checked.
    ok(&mut tw, GOD, "@facility/terms 7=public");
    let alice = tw.spawn_player("Alice");
    let bob = tw.spawn_player("Bob");
    let alice_acct = open_account_with_funds(&mut tw, alice, faucet, 10_000);
    let _bob_acct = open_account_with_funds(&mut tw, bob, faucet, 10_000);
    let alice_src = make_endpoint(&mut tw, "alice_src", alice);
    let alice_dst = make_endpoint(&mut tw, "alice_dst", alice);

    ok(
        &mut tw,
        alice,
        &format!(
            "@wo/draft tacsim.core/alloy_smelt/1=7 1 {} {} {}",
            alice_acct.0, alice_src.0.0, alice_dst.0.0
        ),
    );
    let drafted = land(&mut tw, 3601).expect("alice drafts");
    let order_str = drafted["order"].as_str().expect("order").to_owned();
    let order_id = WorkOrderId::parse(&order_str).expect("parse order");

    // Stranger Bob calls `@wo/submit <alice_order>`: MUST refuse BEFORE `money::place`!
    let bob_refusal = err(&mut tw, bob, &format!("@wo/submit {order_str}"));
    assert_eq!(bob_refusal, "You cannot do that with that work order.");
    assert!(
        tw.engine_mut().take_authoritative_submit().is_none(),
        "Bob's refused @wo/submit must stage nothing"
    );
    assert!(
        money::live_hold_in(tw.world(), order_id).is_none(),
        "Bob must NOT place a budget hold on Alice's account"
    );
    assert_eq!(
        snap(&tw)
            .account(alice_acct)
            .expect("acct")
            .held_amount
            .minor(),
        0,
        "Alice's available funds must remain completely untouched"
    );

    // Now Alice stocks metals and submits her own Draft order -> succeeds.
    let handles = tw
        .world()
        .module_state::<IndustryHandles>()
        .expect("handles")
        .0
        .clone();
    apply(tw.engine_mut(), &handles, |txn| {
        handles
            .inventory
            .credit(txn, alice_src, &cid("metals"), 20)
            .map(|_| ())
            .map_err(IndustryError::Inventory)
    })
    .expect("stock metals");
    ok(&mut tw, alice, &format!("@wo/submit {order_str}"));
    land(&mut tw, 3602).expect("alice submits");
    let holds_count = snap(&tw).holds().count();

    // Calling `@wo/submit` again while `Running` refuses up front with no new hold.
    let repeat_refusal = err(&mut tw, alice, &format!("@wo/submit {order_str}"));
    assert_eq!(repeat_refusal, "You cannot do that with that work order.");
    assert!(tw.engine_mut().take_authoritative_submit().is_none());
    assert_eq!(snap(&tw).holds().count(), holds_count);
}

// ---- HIGH-37 test ----------------------------------------------------------

#[test]
fn high37_claim_transfer_bound_encumbered_revision_key_and_atomic_coordination() {
    let (mut tw, families, _drifted) = boot_full();
    let alice = tw.spawn_player("Alice");
    let bob = tw.spawn_player("Bob");
    let place = tw
        .world_mut()
        .create(ObjKind::Room, "Claim Site", GOD, None)
        .expect("place");

    ok(&mut tw, GOD, "@org/create Crown");
    let crown_org = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .expect("orgs")
        .state()
        .orgs()
        .find(|o| o.name == "Crown")
        .expect("crown org")
        .id;

    // 1. Bound claim: `@claim/transfer` refuses BEFORE `TransferTitle` (title owner
    // stays Alice!), and `movable_claims` excludes it.
    ok(
        &mut tw,
        GOD,
        &format!(
            "@claim/file place:{}=org:{}/obj:{}/shared/1/100000/bound",
            place.0, crown_org.0, alice.0
        ),
    );
    let bound_out = land(&mut tw, 3701).expect("bound claim files");
    let bound_id = ClaimId::parse(bound_out["claim"].as_str().expect("id")).expect("claim id");
    let bound_obj = ObjId(u32::try_from(bound_out["object"].as_u64().expect("obj")).expect("u32"));
    err(
        &mut tw,
        alice,
        &format!("@claim/transfer {bound_id}=obj:{}", bob.0),
    );
    assert!(tw.engine_mut().take_authoritative_submit().is_none());
    assert_eq!(
        snap(&tw).asset(bound_obj).expect("bound asset").title_owner,
        PrincipalId::Object(alice),
        "refused @claim/transfer on Bound claim must NOT transfer E4 title"
    );
    assert!(
        !mush_industry::provider::movable_claims(tw.engine(), &families, alice).contains(&bound_id),
        "Bound claim must NOT be in movable_claims"
    );

    // 2. Transferable claim: pledged -> refused BEFORE `TransferTitle` and excluded
    // from `movable_claims`.
    ok(
        &mut tw,
        GOD,
        &format!(
            "@claim/file place:{}=org:{}/obj:{}/shared/1/100000/transferable",
            place.0, crown_org.0, alice.0
        ),
    );
    let pledged_out = land(&mut tw, 3702).expect("pledged claim files");
    let pledged_id = ClaimId::parse(pledged_out["claim"].as_str().expect("id")).expect("claim id");
    let pledged_obj =
        ObjId(u32::try_from(pledged_out["object"].as_u64().expect("obj")).expect("u32"));
    ok(&mut tw, GOD, &format!("@claim/pledge {pledged_id}=42"));
    err(
        &mut tw,
        alice,
        &format!("@claim/transfer {pledged_id}=obj:{}", bob.0),
    );
    assert!(tw.engine_mut().take_authoritative_submit().is_none());
    assert_eq!(
        snap(&tw)
            .asset(pledged_obj)
            .expect("pledged asset")
            .title_owner,
        PrincipalId::Object(alice),
        "pledged claim title must stay with Alice"
    );
    assert!(
        !mush_industry::provider::movable_claims(tw.engine(), &families, alice)
            .contains(&pledged_id),
        "encumbered claim must NOT be in movable_claims"
    );

    // 3. Clean Transferable claim: direct `ClaimMove` WITHOUT prior `TransferTitle`
    // cannot split `claim.holder()` from `asset.title_owner`!
    ok(
        &mut tw,
        GOD,
        &format!(
            "@claim/file place:{}=org:{}/obj:{}/shared/1/100000/transferable",
            place.0, crown_org.0, alice.0
        ),
    );
    let xfer_out = land(&mut tw, 3703).expect("xfer claim files");
    let xfer_id = ClaimId::parse(xfer_out["claim"].as_str().expect("id")).expect("claim id");
    let xfer_obj = ObjId(u32::try_from(xfer_out["object"].as_u64().expect("obj")).expect("u32"));
    let move_action = action_ref(&tw.engine().registry, IndustryVerb::ClaimMove).expect("act");
    let split_attempt = submit_direct(
        &mut tw,
        alice,
        move_action,
        json!({
            "claim": xfer_id.to_string(),
            "holder": format!("obj:{}", bob.0),
            "revision": 1,
        }),
        3704,
    );
    assert!(
        split_attempt.is_err(),
        "direct ClaimMove without E4 TransferTitle must refuse: {split_attempt:?}"
    );
    let holder_after_split = read(tw.engine_mut(), &families, |txn| {
        families
            .claim(txn, xfer_id)
            .unwrap()
            .unwrap()
            .holder()
            .clone()
    });
    assert_eq!(holder_after_split, PrincipalId::Object(alice));

    // 4. Multi-hop `@claim/transfer` (Alice -> Bob -> Alice) succeeds because the
    // idempotency key includes `claim.revision()`!
    ok(
        &mut tw,
        GOD,
        &format!("@claim/transfer {xfer_id}=obj:{}", bob.0),
    );
    land(&mut tw, 3705).expect("hop 1 (Alice -> Bob) lands");
    assert_eq!(
        snap(&tw).asset(xfer_obj).expect("asset").title_owner,
        PrincipalId::Object(bob)
    );
    assert_eq!(
        read(tw.engine_mut(), &families, |txn| {
            families
                .claim(txn, xfer_id)
                .unwrap()
                .unwrap()
                .holder()
                .clone()
        }),
        PrincipalId::Object(bob)
    );
    ok(
        &mut tw,
        GOD,
        &format!("@claim/transfer {xfer_id}=obj:{}", alice.0),
    );
    land(&mut tw, 3706).expect("hop 2 (Bob -> Alice) lands without idempotency collision");
    assert_eq!(
        snap(&tw).asset(xfer_obj).expect("asset").title_owner,
        PrincipalId::Object(alice)
    );
    assert_eq!(
        read(tw.engine_mut(), &families, |txn| {
            families
                .claim(txn, xfer_id)
                .unwrap()
                .unwrap()
                .holder()
                .clone()
        }),
        PrincipalId::Object(alice)
    );

    // 5. Bound claim whose E4 title owner diverges from `claim.holder()` does NOT
    // authorize extraction.
    let dep = OreDeposit::new(
        OreDepositId(0x3799),
        place,
        cid("metals"),
        Grade::new(1).expect("grade"),
        Accessibility::new(100).expect("acc"),
        BandId::new("high").expect("gband"),
        BandId::new("rich").expect("rband"),
        YieldCurve::new(vec![CurvePoint {
            cumulative: 0,
            per_cycle: 10,
        }])
        .expect("curve"),
        EndpointRef(VaultId(799)),
        1_000,
    )
    .expect("deposit")
    .with_jurisdiction(PrincipalId::Organization(crown_org));
    let now = tw.world().world_ic_now();
    let mismatched_bound = ClaimRecord::new(
        ClaimId(0x379A),
        ClaimSubject::Deposit { deposit: dep.id() },
        PrincipalId::Organization(crown_org),
        PrincipalId::Object(bob), // claim says Bob, but E4 title on `bound_obj` is Alice!
        bound_obj,
        Exclusivity::Shared,
        1,
        ClockStamp {
            clock: now.clock.clone(),
            at: now.at + 100_000,
        },
        Transferability::Bound,
    )
    .expect("bound claim");
    assert!(
        mush_industry::licence::extraction_authorized(
            tw.world(),
            bob,
            &PrincipalId::Object(bob),
            &dep,
            dep.initial_reserve(),
            &[mismatched_bound],
            None,
            &now,
        )
        .is_err(),
        "Bound claim whose E4 asset.title_owner != claim.holder() must NOT authorize extraction"
    );
}

#[test]
fn t2a_direct_claim_contest_cannot_seize_pledged_or_contest_disputed_claims() {
    let (mut tw, families, _) = boot_full();
    let place = tw
        .world_mut()
        .create(ObjKind::Room, "ContestedBelt", GOD, None)
        .expect("place");
    let alice = tw.spawn_player("Alice");
    let registrar = tw.spawn_player("Registrar");
    let challenger = tw.spawn_player("Challenger");

    ok(&mut tw, GOD, "@org/create Crown");
    let crown_org = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .expect("org")
        .state()
        .orgs()
        .find(|o| o.name == "Crown")
        .expect("Crown")
        .id;
    let crown_root = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .expect("org")
        .state()
        .org(crown_org)
        .expect("Crown")
        .root_scope;
    ok(
        &mut tw,
        GOD,
        &format!(
            "@org/grant #{}=Crown economy.account.post scope={}",
            registrar.0, crown_root.0
        ),
    );

    // 1. Pledged claim: direct `ClaimContest` with state="seized" (both by non-staff
    // issuer registrar AND by staff GOD, with or without `holder`) must be refused.
    ok(
        &mut tw,
        GOD,
        &format!(
            "@claim/file place:{}=org:{}/obj:{}/shared/1/100000/transferable",
            place.0, crown_org.0, alice.0
        ),
    );
    let pledged_out = land(&mut tw, 3801).expect("pledged claim files");
    let pledged_id = ClaimId::parse(pledged_out["claim"].as_str().expect("id")).expect("claim id");
    ok(&mut tw, GOD, &format!("@claim/pledge {pledged_id}=99"));

    let contest_action =
        action_ref(&tw.engine().registry, IndustryVerb::ClaimContest).expect("claim contest");
    let non_staff_seize = submit_direct(
        &mut tw,
        registrar,
        contest_action.clone(),
        json!({
            "claim": pledged_id.to_string(),
            "state": "seized",
            "revision": 1,
        }),
        3802,
    );
    assert!(
        non_staff_seize.is_err(),
        "direct ClaimContest seizure on pledged claim by issuer must refuse: {non_staff_seize:?}"
    );
    let staff_seize = submit_direct(
        &mut tw,
        GOD,
        contest_action.clone(),
        json!({
            "claim": pledged_id.to_string(),
            "state": "seized",
            "revision": 1,
            "holder": format!("obj:{}", challenger.0),
        }),
        3803,
    );
    assert!(
        staff_seize.is_err(),
        "direct ClaimContest staff seizure with holder on pledged claim must refuse: {staff_seize:?}"
    );
    let pledged_live = read(tw.engine_mut(), &families, |txn| {
        families.claim(txn, pledged_id).unwrap().unwrap()
    });
    assert_eq!(pledged_live.contest_state(), &ContestState::Uncontested);
    assert_eq!(pledged_live.holder(), &PrincipalId::Object(alice));

    // Suspending a pledged claim is still permitted (it preserves collateral).
    submit_direct(
        &mut tw,
        registrar,
        contest_action.clone(),
        json!({
            "claim": pledged_id.to_string(),
            "state": "suspended",
            "revision": 1,
        }),
        3804,
    )
    .expect("suspending a pledged claim remains permitted");

    // 2. Disputed claim: direct `ClaimContest` with state="contested" must refuse;
    // state="suspended" remains allowed.
    ok(
        &mut tw,
        GOD,
        &format!(
            "@claim/file place:{}=org:{}/obj:{}/shared/1/100000/transferable",
            place.0, crown_org.0, alice.0
        ),
    );
    let disputed_out = land(&mut tw, 3805).expect("disputed claim files");
    let disputed_id =
        ClaimId::parse(disputed_out["claim"].as_str().expect("id")).expect("claim id");
    let disputed_obj =
        ObjId(u32::try_from(disputed_out["object"].as_u64().expect("obj")).expect("u32"));
    ok(
        &mut tw,
        GOD,
        &format!("@asset/stolen #{}=public disputed title", disputed_obj.0),
    );

    let direct_contest = submit_direct(
        &mut tw,
        registrar,
        contest_action.clone(),
        json!({
            "claim": disputed_id.to_string(),
            "state": "contested",
            "revision": 1,
            "contract": 0,
        }),
        3806,
    );
    assert!(
        direct_contest.is_err(),
        "direct ClaimContest state=contested on disputed claim must refuse: {direct_contest:?}"
    );
    let disputed_live = read(tw.engine_mut(), &families, |txn| {
        families.claim(txn, disputed_id).unwrap().unwrap()
    });
    assert_eq!(disputed_live.contest_state(), &ContestState::Uncontested);

    submit_direct(
        &mut tw,
        registrar,
        contest_action,
        json!({
            "claim": disputed_id.to_string(),
            "state": "suspended",
            "revision": 1,
        }),
        3807,
    )
    .expect("suspending a disputed claim remains permitted");
}
