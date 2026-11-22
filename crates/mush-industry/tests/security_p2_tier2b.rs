//! Pass 2 Tier 2B defensive security regression tests for `mush-industry`:
//! - `P2-MOD-17`: `WoDraft`, `check_wo_accounts`, `@wo/submit`, `money::place`,
//!   and `eligible_submit_holds` require an active `InstrumentKind::Currency`
//!   account matching a valid payee for the targeted facility.
//! - `P2-MOD-46`: `money::attempt` / `attempt_of` / `place` survive 7-day
//!   economy hold GC (`HOLD_GC_AFTER_SECS`) without permanently locking live
//!   escrow holds or colliding on retry.
//! - `P2-MOD-47`: `ClaimContest` and `@claim/*` enforce the staff > governing
//!   jurisdiction > issuing org hierarchy so issuing orgs / non-staff cannot
//!   override or lift staff-imposed or governing-jurisdiction suspensions and
//!   seizures.
//! - `P2-MOD-48`: `@claim/transfer` normalizes bare-hex / uppercase IDs,
//!   handles >64 transferable claims without truncating the targeted transfer,
//!   and never splits E4 title from `ClaimRecord.holder`.
//! - `P2-MOD-49`: `@claim/*` and `ClaimFile` enforce caller authorization and
//!   uniform refusals before exposing claim/collateral/deposit/jurisdiction
//!   existence, and deduplicate/rate-limit level-2 staff pages.
//! - `P2-MOD-77`: `wo_restamp` re-anchors the facility slot reservation and
//!   `slot_expiry_schedule` across clock-generation changes while preserving
//!   the `P2-MOD-2` invariant for refit/integration slots.
//! - `P2-MOD-78`: Staff `ClaimContest` encumbrance scanning inspects only
//!   claim objects, stays bounded well inside `MAX_PROJECTED_LEGAL_ACTION_BYTES`,
//!   and fails closed if truncated.

#![allow(clippy::too_many_lines)]

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use mush_core::{
    CanonicalId, ClockStamp, CommitToken, DefinitionDigest, Engine, GOD, IdempotencyKey,
    IdempotencyScope, ObjId, ObjKind, OfferKeyRing, OfferSecret, RequestUid, ReservationId,
    ReservationState, ResolutionKeyRing, ResolutionSecret, ScheduleDriveCause,
    ScopedIdempotencyKey, World,
};
use mush_economy::{
    AccountId, EconomicVisibility, EconomyStore, HoldStatus, InstrumentDef, InstrumentId,
    InstrumentKind, InstrumentStatus, NegativePolicy, VaultId,
};
use mush_industry::facility::facility_capacity_key;
use mush_industry::provider::{
    ACTION_WO_RECLAIM, CLAIM_CONTEST_DISPUTED, CLAIM_CONTEST_DISPUTED_TRUNCATED,
    CLAIM_CONTEST_PLEDGED, CLAIM_CONTEST_PLEDGED_TRUNCATED, IndustryVerb, MAX_CONTEST_ENCUMBRANCES,
    MAX_DEPOSIT_CHOICES, action_ref,
};
use mush_industry::record::{
    Accessibility, BandId, ClaimId, ClaimRecord, ClaimSubject, ContestState, CurvePoint,
    Exclusivity, OreDeposit, OreDepositId, Transferability, YieldCurve,
};
use mush_industry::store::CLAIM_PAGE_COOLDOWN_SECS;
use mush_industry::workorder::{
    DeliveryTarget, PinnedRecipe, RecipeLine, RecipeRef, WorkOrder, WorkOrderId, WorkOrderState,
    wo_holder,
};
use mush_industry::{
    AccountRefusal, IndustryError, IndustryFamilies, IndustryHandles, IndustryModule,
    IndustryStore, money,
};
use mush_inventory::{EndpointRef, Grade, InventoryHandleCell, InventoryModule};
use mush_org::PrincipalId;
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, text_of};
use serde_json::json;
use smol_str::SmolStr;

// ---- Shared test rig -------------------------------------------------------

struct FixedRecipes;

impl FixedRecipes {
    fn digest() -> DefinitionDigest {
        DefinitionDigest::from_bytes([0x33; 32])
    }

    fn alloy() -> (RecipeRef, PinnedRecipe) {
        let recipe = RecipeRef::new(cid("tacsim.core"), 1, cid("alloy_smelt"), Self::digest())
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

impl mush_industry::RecipeAuthority for FixedRecipes {
    fn digest_of(&self, _world: &World, recipe: &RecipeRef) -> Option<DefinitionDigest> {
        (recipe.pack().as_str() == "tacsim.core"
            && recipe.version() == 1
            && recipe.id().as_str() == "alloy_smelt")
            .then(Self::digest)
    }
    fn resolve(
        &self,
        _world: &World,
        pack: &CanonicalId,
        version: u32,
        id: &CanonicalId,
    ) -> Option<(RecipeRef, PinnedRecipe)> {
        (pack.as_str() == "tacsim.core" && version == 1 && id.as_str() == "alloy_smelt")
            .then(Self::alloy)
    }
    fn catalog_revision(&self, _world: &World) -> u64 {
        1
    }
    fn catalog(&self, _world: &World) -> Vec<(RecipeRef, PinnedRecipe)> {
        vec![Self::alloy()]
    }
}

fn cid(text: &str) -> CanonicalId {
    CanonicalId::new(text).expect("canonical id")
}

fn caps(ids: &[&str]) -> BTreeSet<CanonicalId> {
    ids.iter().map(|s| cid(s)).collect()
}

fn boot_full() -> (TestWorld, IndustryFamilies) {
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
                .with_recipes(Arc::new(FixedRecipes))
                .with_extraction_licence(InstrumentId::new("licence").expect("licence")),
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
    (world, families)
}

fn with_ctx<R>(
    tw: &mut TestWorld,
    actor: ObjId,
    body: impl FnOnce(&mut mush_core::Ctx<'_>) -> R,
) -> R {
    let mut out = mush_core::Outbox::new();
    let engine = tw.engine_mut();
    let mut ctx = mush_core::Ctx {
        w: &mut engine.world,
        actor,
        args: mush_core::ParsedArgs::parse(mush_core::ArgShape::Raw, ""),
        switches: mush_core::SwitchSet::default(),
        out: &mut out,
        reg: &engine.registry,
        module: Some(SmolStr::new_static("industry")),
        typed_state: mush_core::ModuleActionState::unavailable(),
    };
    body(&mut ctx)
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

fn register_instrument_kind(tw: &mut TestWorld, code: &str, kind: InstrumentKind) {
    with_ctx(tw, GOD, |ctx| {
        mush_economy::register_instrument(
            ctx,
            InstrumentDef {
                id: InstrumentId::new(code).expect("inst id"),
                ruleset: SmolStr::new_static("test"),
                code: SmolStr::new(code),
                kind,
                scale: 0,
                transferable: kind != InstrumentKind::Entitlement,
                negative_policy: NegativePolicy::SystemOnly,
                status: InstrumentStatus::Active,
                display: None,
                revision: 1,
            },
        )
        .expect("register instrument");
    });
}

fn open_account(tw: &mut TestWorld, owner: ObjId, inst: &str) -> AccountId {
    let before: BTreeSet<u64> = snap(tw).accounts().map(|a| a.id.0).collect();
    ok(tw, GOD, &format!("@account/open #{}={inst}", owner.0));
    let id = snap(tw)
        .accounts()
        .map(|a| a.id.0)
        .find(|id| !before.contains(id))
        .expect("new account");
    AccountId(id)
}

fn open_sys_faucet(tw: &mut TestWorld, name: &str, inst: &str) -> AccountId {
    let before: BTreeSet<u64> = snap(tw).accounts().map(|a| a.id.0).collect();
    ok(tw, GOD, &format!("@account/system {name}={inst}"));
    let id = snap(tw)
        .accounts()
        .map(|a| a.id.0)
        .find(|id| !before.contains(id))
        .expect("new sys account");
    AccountId(id)
}

fn make_endpoint(tw: &mut TestWorld, owner: ObjId, name: &str) -> EndpointRef {
    let mut create_once = |label: &str| {
        let obj = tw
            .world_mut()
            .create(ObjKind::Thing, label, owner, Some(mush_core::LIMBO))
            .expect("vault obj");
        let before: BTreeSet<u64> = snap(tw).vaults().map(|v| v.id.0).collect();
        ok(
            tw,
            GOD,
            &format!(
                "@vault/create {label}=container:#{} owner=#{}",
                obj.0, owner.0
            ),
        );
        snap(tw)
            .vaults()
            .map(|v| v.id.0)
            .find(|id| !before.contains(id))
            .expect("vault id")
    };
    let mut vid = create_once(name);
    if vid == 0 {
        // `optional_endpoint` treats 0 as absent, so ensure real test endpoints are non-zero.
        vid = create_once(&format!("{name}1"));
    }
    EndpointRef(VaultId(vid))
}

fn fire_sweeps(tw: &mut TestWorld, advance_secs: i64) {
    let t = tw.world().now() + advance_secs;
    tw.set_now(t);
    let _ = tw.fire_due(t.saturating_mul(1_000));
}

fn create_org_with_officer(tw: &mut TestWorld, name: &str, officer: ObjId) -> mush_org::OrgId {
    ok(tw, GOD, &format!("@org/create {name}"));
    let org_id = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .expect("orgs")
        .state()
        .orgs()
        .find(|o| o.name == name)
        .expect("org")
        .id;
    ok(
        tw,
        GOD,
        &format!("@org/grant #{}={name} economy.account.post", officer.0),
    );
    org_id
}

fn seed_deposit(
    tw: &mut TestWorld,
    families: &IndustryFamilies,
    id: OreDepositId,
    place: ObjId,
    vault_num: u64,
    jurisdiction: Option<PrincipalId>,
) -> OreDeposit {
    let mut dep = OreDeposit::new(
        id,
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
        EndpointRef(VaultId(vault_num)),
        10_000,
    )
    .expect("deposit");
    if let Some(jur) = jurisdiction {
        dep = dep.with_jurisdiction(jur);
    }
    apply(tw.engine_mut(), families, |txn| {
        families.open_deposit(txn, dep.clone())
    })
    .expect("open deposit");
    dep
}

// ---- P2-MOD-17: Currency & payee enforcement on WoDraft / @wo/submit / WoSubmit ----

#[test]
fn p2_mod_17_wo_draft_and_submit_require_active_currency_and_facility_payee() {
    let (mut tw, families) = boot_full();
    let boss = tw.spawn_player("Boss");
    let alice = tw.spawn_player("Alice");

    register_instrument_kind(&mut tw, "gold", InstrumentKind::Currency);
    register_instrument_kind(&mut tw, "silver", InstrumentKind::Currency);
    register_instrument_kind(&mut tw, "fuel", InstrumentKind::Commodity);
    register_instrument_kind(&mut tw, "licence", InstrumentKind::Entitlement);

    let gold_faucet = open_sys_faucet(&mut tw, "goldfaucet", "gold");
    let silver_faucet = open_sys_faucet(&mut tw, "silverfaucet", "silver");
    let fuel_faucet = open_sys_faucet(&mut tw, "fuelfaucet", "fuel");

    // Boss (facility owner) holds gold (Currency), fuel (Commodity), and licence (Entitlement),
    // but does NOT hold silver (Currency).
    let boss_gold = open_account(&mut tw, boss, "gold");
    let _boss_fuel = open_account(&mut tw, boss, "fuel");
    let _boss_lic = open_account(&mut tw, boss, "licence");

    // Alice holds funded accounts in gold, silver, fuel, and licence.
    let alice_gold = open_account(&mut tw, alice, "gold");
    let alice_silver = open_account(&mut tw, alice, "silver");
    let alice_fuel = open_account(&mut tw, alice, "fuel");
    let alice_lic = open_account(&mut tw, alice, "licence");
    ok(
        &mut tw,
        GOD,
        &format!("@econ/mint {gold_faucet}={alice_gold} 10000"),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@econ/mint {silver_faucet}={alice_silver} 10000"),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@econ/mint {fuel_faucet}={alice_fuel} 10000"),
    );

    // Public facility owned by Boss, plus a second facility owned by Alice (where Alice's
    // silver account IS valid as an owner-operator currency account).
    let boss_fac = tw
        .world_mut()
        .create(ObjKind::Thing, "BossWorks", boss, None)
        .expect("boss fac");
    let alice_fac = tw
        .world_mut()
        .create(ObjKind::Thing, "AliceWorks", alice, None)
        .expect("alice fac");
    ok(
        &mut tw,
        GOD,
        &format!("@facility/open {}=2 smelt", boss_fac.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@facility/owner {}=#{}", boss_fac.0, boss.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@facility/terms {}=public", boss_fac.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@facility/open {}=2 smelt", alice_fac.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@facility/owner {}=#{}", alice_fac.0, alice.0),
    );

    let source = make_endpoint(&mut tw, alice, "AliceSource");
    let dest = make_endpoint(&mut tw, alice, "AliceDest");
    apply(tw.engine_mut(), &families, |txn| {
        families
            .inventory
            .credit(txn, source, &cid("metals"), 1_000)
            .map(|_| ())
            .map_err(IndustryError::Inventory)
    })
    .expect("seed metals");

    // 1. `@wo/draft` refuses Commodity, Entitlement, and currency with no facility payee.
    let fuel_err = err(
        &mut tw,
        alice,
        &format!(
            "@wo/draft tacsim.core/alloy_smelt/1={} 1 {} {} {}",
            boss_fac.0,
            alice_fuel.0,
            source.vault().0,
            dest.vault().0
        ),
    );
    assert_eq!(fuel_err, AccountRefusal::NotCurrency.public());
    let lic_err = err(
        &mut tw,
        alice,
        &format!(
            "@wo/draft tacsim.core/alloy_smelt/1={} 1 {} {} {}",
            boss_fac.0,
            alice_lic.0,
            source.vault().0,
            dest.vault().0
        ),
    );
    assert_eq!(lic_err, AccountRefusal::NotCurrency.public());
    let silver_err = err(
        &mut tw,
        alice,
        &format!(
            "@wo/draft tacsim.core/alloy_smelt/1={} 1 {} {} {}",
            boss_fac.0,
            alice_silver.0,
            source.vault().0,
            dest.vault().0
        ),
    );
    assert_eq!(silver_err, AccountRefusal::NoPayee.public());

    // 2. `wo_offer_books` excludes Commodity & Entitlement globally, and excludes `silver`
    // from Boss's facility even though `silver` IS valid at Alice's own facility.
    let facilities = mush_industry::provider::draftable_facilities(tw.engine(), alice, false);
    let catalog = mush_industry::provider::catalog_entries(&FixedRecipes, tw.world());
    let books = mush_industry::provider::wo_offer_books(tw.engine(), alice, &facilities, &catalog);
    assert!(books.accounts.contains(&alice_gold.0));
    assert!(books.accounts.contains(&alice_silver.0));
    assert!(!books.accounts.contains(&alice_fuel.0));
    assert!(!books.accounts.contains(&alice_lic.0));
    let boss_row_accounts: Vec<u64> = books.facilities[&boss_fac.0.to_string()]["accounts"]
        .as_array()
        .expect("boss accounts")
        .iter()
        .filter_map(serde_json::Value::as_u64)
        .collect();
    assert_eq!(
        boss_row_accounts,
        vec![alice_gold.0],
        "Boss's facility only accepts Alice's gold currency account"
    );

    // 3. Direct `WoDraft` at Boss's facility refuses fuel, licence, AND silver (cross-facility mismatch).
    let draft_action = action_ref(&tw.engine().registry, IndustryVerb::WoDraft).expect("draft");
    let (_, pinned) = FixedRecipes::alloy();
    let pinned_json = serde_json::to_string(&pinned).expect("pinned json");
    for (idx, bad_acct) in [alice_fuel, alice_lic, alice_silver]
        .into_iter()
        .enumerate()
    {
        let res = submit_direct(
            &mut tw,
            alice,
            draft_action.clone(),
            json!({
                "facility": boss_fac.0,
                "pack": "tacsim.core",
                "version": 1,
                "recipe": "alloy_smelt",
                "quantity": 1,
                "account": bad_acct.0,
                "source": source.vault().0,
                "destination": dest.vault().0,
                "beneficiary": format!("obj:{}", alice.0),
                "pinned": pinned_json,
            }),
            1700 + idx as u64,
        );
        assert!(
            res.is_err(),
            "direct WoDraft at Boss's facility with incompatible account {bad_acct} must refuse: {res:?}"
        );
    }

    // 4. Even if a stale/seeded Draft order carries a Commodity or no-payee currency account,
    // `@wo/submit`, `money::place`, AND direct `WoSubmit` (`eligible_submit_holds`) ALL refuse.
    let boss_rec = tw
        .world()
        .module_state::<IndustryStore>()
        .expect("store")
        .facility(boss_fac)
        .expect("boss fac")
        .clone();
    let (recipe_ref, pinned) = FixedRecipes::alloy();
    for (seq, bad_acct) in [(0x17A1_u64, alice_fuel), (0x17A2_u64, alice_silver)] {
        let bad_order = WorkOrder::draft(
            WorkOrderId(seq),
            PrincipalId::Object(alice),
            PrincipalId::Object(alice),
            boss_fac,
            boss_rec.maximum_slots(),
            boss_rec.capacity_revision(),
            boss_rec.policy_revision(),
            recipe_ref.clone(),
            pinned.clone(),
            1,
            bad_acct,
            Some(source),
            DeliveryTarget::Endpoint { endpoint: dest },
            EconomicVisibility::Parties,
        )
        .expect("seed bad draft");
        apply(tw.engine_mut(), &families, |txn| {
            families.open_work_order(txn, bad_order.clone())
        })
        .expect("insert bad draft");
        let _ = err(&mut tw, alice, &format!("@wo/submit {}", bad_order.id()));
        assert!(
            money::live_hold_in(tw.world(), bad_order.id()).is_none(),
            "refused @wo/submit must place NO budget hold"
        );
        let place_res = with_ctx(&mut tw, alice, |ctx| money::place(ctx, &bad_order));
        assert!(
            place_res.is_err(),
            "money::place must refuse incompatible account {bad_acct}: {place_res:?}"
        );
    }

    // 5. Valid gold currency account drafts and submits cleanly; if Boss closes their gold payee
    // before submit, BOTH `@wo/submit` and direct `WoSubmit` refuse cleanly.
    ok(
        &mut tw,
        alice,
        &format!(
            "@wo/draft tacsim.core/alloy_smelt/1={} 1 {} {} {}",
            boss_fac.0,
            alice_gold.0,
            source.vault().0,
            dest.vault().0
        ),
    );
    let drafted = land(&mut tw, 1710).expect("valid gold draft lands");
    let order_id = WorkOrderId::parse(drafted["order"].as_str().expect("id")).expect("wo id");

    // Close Boss's gold payee -> `@wo/submit` refuses with NoPayee without placing a hold.
    ok(&mut tw, GOD, &format!("@account/close {boss_gold}"));
    let no_payee_err = err(&mut tw, alice, &format!("@wo/submit {order_id}"));
    assert_eq!(no_payee_err, AccountRefusal::NoPayee.public());
    assert!(money::live_hold_in(tw.world(), order_id).is_none());

    // Reopen Boss's gold payee, place a real hold via `@wo/submit` (dropping the submit),
    // then close Boss's gold payee again -> `eligible_submit_holds` / direct `WoSubmit`
    // MUST STILL refuse even with a live hold present!
    let boss_gold_2 = open_account(&mut tw, boss, "gold");
    ok(&mut tw, alice, &format!("@wo/submit {order_id}"));
    let _dropped = tw.engine_mut().take_authoritative_submit();
    assert!(
        money::live_hold_in(tw.world(), order_id).is_some(),
        "hold was placed while payee was open"
    );
    ok(&mut tw, GOD, &format!("@account/close {boss_gold_2}"));
    let submit_action = action_ref(&tw.engine().registry, IndustryVerb::WoSubmit).expect("submit");
    let direct_submit = submit_direct(
        &mut tw,
        alice,
        submit_action,
        json!({ "order": order_id.to_string(), "revision": 1 }),
        1710,
    );
    assert!(
        direct_submit.is_err(),
        "direct WoSubmit (`eligible_submit_holds`) must refuse when facility payee closed: {direct_submit:?}"
    );
    let no_payee_err_2 = err(&mut tw, alice, &format!("@wo/submit {order_id}"));
    assert_eq!(no_payee_err_2, AccountRefusal::NoPayee.public());

    // Reopen Boss's gold payee -> `@wo/submit` + `land` succeeds reusing the live hold!
    let _boss_gold_3 = open_account(&mut tw, boss, "gold");
    ok(&mut tw, alice, &format!("@wo/submit {order_id}"));
    land(&mut tw, 1711).expect("submit succeeds once currency payee is active");
}

// ---- P2-MOD-46: money::attempt / attempt_of survive 7-day HOLD_GC_AFTER_SECS ----

#[test]
fn p2_mod_46_money_attempt_and_attempt_of_survive_seven_day_hold_gc() {
    let (mut tw, families) = boot_full();
    let boss = tw.spawn_player("Boss");
    let alice = tw.spawn_player("Alice");

    register_instrument_kind(&mut tw, "gold", InstrumentKind::Currency);
    let faucet = open_sys_faucet(&mut tw, "faucet", "gold");
    let boss_payee = open_account(&mut tw, boss, "gold");
    let alice_budget = open_account(&mut tw, alice, "gold");
    ok(
        &mut tw,
        GOD,
        &format!("@econ/mint {faucet}={alice_budget} 50000"),
    );

    let fac = tw
        .world_mut()
        .create(ObjKind::Thing, "Mill", boss, None)
        .expect("fac");
    ok(&mut tw, GOD, &format!("@facility/open {}=2 smelt", fac.0));
    ok(
        &mut tw,
        GOD,
        &format!("@facility/owner {}=#{}", fac.0, boss.0),
    );
    ok(&mut tw, GOD, &format!("@facility/terms {}=public", fac.0));
    let source = make_endpoint(&mut tw, alice, "Src");
    let dest = make_endpoint(&mut tw, alice, "Dst");

    let fac_rec = tw
        .world()
        .module_state::<IndustryStore>()
        .expect("store")
        .facility(fac)
        .expect("fac")
        .clone();
    let (recipe_ref, pinned) = FixedRecipes::alloy();

    // ---- Scenario A: live attempt-1 hold settles cleanly AFTER attempt-0 hold is GC'd ----
    let wo1 = WorkOrder::draft(
        WorkOrderId(0x4601),
        PrincipalId::Object(alice),
        PrincipalId::Object(alice),
        fac,
        fac_rec.maximum_slots(),
        fac_rec.capacity_revision(),
        fac_rec.policy_revision(),
        recipe_ref.clone(),
        pinned.clone(),
        1,
        alice_budget,
        Some(source),
        DeliveryTarget::Endpoint { endpoint: dest },
        EconomicVisibility::Parties,
    )
    .expect("wo1");
    apply(tw.engine_mut(), &families, |txn| {
        families.open_work_order(txn, wo1.clone())
    })
    .expect("open wo1");

    // Attempt 0: submit places hold H0, drop the authoritative submit, release H0 on 2 sweeps.
    ok(&mut tw, alice, &format!("@wo/submit {}", wo1.id()));
    let _ = tw.engine_mut().take_authoritative_submit();
    fire_sweeps(&mut tw, 120);
    fire_sweeps(&mut tw, 120);
    assert!(
        snap(&tw)
            .holds()
            .any(|h| h.purpose == money::purpose(wo1.id())
                && matches!(h.status, HoldStatus::Released)),
        "attempt-0 hold H0 was released"
    );

    // Attempt 1: resubmit places hold H1 (`holder = industry.wo.hold/{wo1}/1`), transition Running.
    ok(&mut tw, alice, &format!("@wo/submit {}", wo1.id()));
    let _ = tw.engine_mut().take_authoritative_submit();
    let h1 = money::live_hold_in(tw.world(), wo1.id()).expect("live attempt-1 hold H1");
    assert_eq!(h1.holder, money::wo_hold_key(wo1.id(), 1));
    apply(tw.engine_mut(), &families, |txn| {
        let live = families.work_order(txn, wo1.id())?.expect("wo1");
        let next = live
            .with_state(WorkOrderState::Running)
            .map_err(|_| IndustryError::State)?;
        families.put_work_order(txn, live.revision(), next)
    })
    .expect("mark wo1 running");

    // Advance past 7-day HOLD_GC_AFTER_SECS and run sweeps so economy GCs terminal H0!
    fire_sweeps(&mut tw, mush_economy::HOLD_GC_AFTER_SECS + 120);
    let surviving_wo1: Vec<_> = snap(&tw)
        .holds()
        .filter(|h| h.purpose == money::purpose(wo1.id()))
        .collect();
    assert_eq!(
        surviving_wo1.len(),
        1,
        "7-day GC must have removed terminal H0 leaving ONLY live H1"
    );
    assert_eq!(surviving_wo1[0].id, h1.id);
    // `attempt_of` MUST still answer 1 (not 0!) even after H0 was GC'd.
    let resolved_attempt = with_ctx(&mut tw, alice, |ctx| {
        money::attempt_of(ctx, wo1.id(), h1.id)
    });
    assert_eq!(
        resolved_attempt, 1,
        "attempt_of must read attempt 1 from Hold::holder after H0 is GC'd"
    );

    // Complete wo1 and fire sweep -> H1 MUST capture cleanly under wo_settle_key(wo1, 1)!
    apply(tw.engine_mut(), &families, |txn| {
        let live = families.work_order(txn, wo1.id())?.expect("wo1");
        let next = live
            .with_state(WorkOrderState::Completed {
                commit: CommitToken::from_bytes([0x46; 16]),
            })
            .map_err(|_| IndustryError::State)?;
        families.put_work_order(txn, live.revision(), next)
    })
    .expect("complete wo1");
    let payee_before = snap(&tw)
        .account(boss_payee)
        .expect("payee")
        .posted_balance
        .minor();
    fire_sweeps(&mut tw, 120);
    assert!(
        money::live_hold_in(tw.world(), wo1.id()).is_none(),
        "H1 must NOT be permanently locked after 7-day GC of H0"
    );
    let payee_after = snap(&tw)
        .account(boss_payee)
        .expect("payee")
        .posted_balance
        .minor();
    assert_eq!(
        payee_after - payee_before,
        h1.amount.minor(),
        "facility payee must receive the captured hold amount"
    );

    // ---- Scenario B: retry placement after ALL prior holds GC'd advances cleanly to attempt 1 ----
    let wo2 = WorkOrder::draft(
        WorkOrderId(0x4602),
        PrincipalId::Object(alice),
        PrincipalId::Object(alice),
        fac,
        fac_rec.maximum_slots(),
        fac_rec.capacity_revision(),
        fac_rec.policy_revision(),
        recipe_ref,
        pinned,
        1,
        alice_budget,
        Some(source),
        DeliveryTarget::Endpoint { endpoint: dest },
        EconomicVisibility::Parties,
    )
    .expect("wo2");
    apply(tw.engine_mut(), &families, |txn| {
        families.open_work_order(txn, wo2.clone())
    })
    .expect("open wo2");

    // Attempt 0: place -> drop -> 2 sweeps release H0 -> 7-day GC removes H0 completely!
    ok(&mut tw, alice, &format!("@wo/submit {}", wo2.id()));
    let _ = tw.engine_mut().take_authoritative_submit();
    fire_sweeps(&mut tw, 120);
    fire_sweeps(&mut tw, 120);
    fire_sweeps(&mut tw, mush_economy::HOLD_GC_AFTER_SECS + 120);
    assert_eq!(
        snap(&tw)
            .holds()
            .filter(|h| h.purpose == money::purpose(wo2.id()))
            .count(),
        0,
        "all prior holds for wo2 must be GC'd"
    );

    // Resubmitting wo2 after full GC MUST NOT collide on spent attempt 0!
    ok(&mut tw, alice, &format!("@wo/submit {}", wo2.id()));
    let _ = tw.engine_mut().take_authoritative_submit();
    let wo2_h1 = money::live_hold_in(tw.world(), wo2.id())
        .expect("retry after 7-day GC must place fresh live hold under attempt 1");
    assert_eq!(wo2_h1.holder, money::wo_hold_key(wo2.id(), 1));
}

// ---- P2-MOD-47: ClaimContest authority hierarchy (staff > gov > issuer) ----

#[test]
fn p2_mod_47_claim_contest_enforces_staff_and_jurisdiction_authority_hierarchy() {
    let (mut tw, families) = boot_full();
    let registrar = tw.spawn_player("Registrar");
    let syn_officer = tw.spawn_player("SyndicateOfficer");
    let holder = tw.spawn_player("Miner");
    let place = tw
        .world_mut()
        .create(ObjKind::Room, "Belt", GOD, None)
        .expect("place");

    let gov_org = create_org_with_officer(&mut tw, "GovRegistry", registrar);
    let syn_org = create_org_with_officer(&mut tw, "Syndicate", syn_officer);
    let dep = seed_deposit(
        &mut tw,
        &families,
        OreDepositId(0x4701),
        place,
        4701,
        Some(PrincipalId::Organization(gov_org)),
    );

    // File two claims on `dep`: one issued by `GovRegistry`, one foreign paper by `Syndicate`.
    ok(
        &mut tw,
        GOD,
        &format!(
            "@claim/file {}=org:{}/obj:{}/shared/1/100000/transferable",
            dep.id(),
            gov_org.0,
            holder.0
        ),
    );
    let own_out = land(&mut tw, 4701).expect("file own claim");
    let own_id = ClaimId::parse(own_out["claim"].as_str().expect("id")).expect("claim id");

    ok(
        &mut tw,
        GOD,
        &format!(
            "@claim/file {}=org:{}/obj:{}/shared/1/100000/transferable",
            dep.id(),
            syn_org.0,
            holder.0
        ),
    );
    let syn_out = land(&mut tw, 4702).expect("file syndicate claim");
    let syn_id = ClaimId::parse(syn_out["claim"].as_str().expect("id")).expect("claim id");
    let contest_action =
        action_ref(&tw.engine().registry, IndustryVerb::ClaimContest).expect("contest");

    // 1. Staff suspends `own_id` -> NEITHER issuer NOR governing-jurisdiction officer can touch it.
    ok(&mut tw, GOD, &format!("@claim/suspend {own_id}"));
    land(&mut tw, 4703).expect("staff suspend");
    for cmd in [
        format!("@claim/release {own_id}"),
        format!("@claim/contest {own_id}=0"),
        format!("@claim/suspend {own_id}"),
        format!("@claim/seize {own_id}"),
    ] {
        let msg = err(&mut tw, registrar, &cmd);
        assert_eq!(msg, mush_industry::commands::NO_SUCH_CLAIM);
    }
    for state in ["uncontested", "contested", "suspended", "seized"] {
        let direct = submit_direct(
            &mut tw,
            registrar,
            contest_action.clone(),
            json!({ "claim": own_id.to_string(), "state": state, "revision": 2, "contract": 0 }),
            4710,
        );
        assert!(
            direct.is_err(),
            "non-staff must not override staff suspension via direct ClaimContest ({state}): {direct:?}"
        );
    }
    // Staff upgrades to Seized -> non-staff still locked out; staff CAN release.
    ok(&mut tw, GOD, &format!("@claim/seize {own_id}"));
    land(&mut tw, 4711).expect("staff seize");
    assert_eq!(
        err(&mut tw, registrar, &format!("@claim/release {own_id}")),
        mush_industry::commands::NO_SUCH_CLAIM
    );
    ok(&mut tw, GOD, &format!("@claim/release {own_id}"));
    land(&mut tw, 4712).expect("staff can lift own seizure");

    // 2. Governing jurisdiction (`registrar`) suspends Syndicate's claim `syn_id` ->
    // Syndicate officer (`syn_officer`) CANNOT release, contest, overwrite, or seize it.
    ok(&mut tw, registrar, &format!("@claim/suspend {syn_id}"));
    land(&mut tw, 4720).expect("gov suspend");
    for cmd in [
        format!("@claim/release {syn_id}"),
        format!("@claim/contest {syn_id}=0"),
        format!("@claim/suspend {syn_id}"),
        format!("@claim/seize {syn_id}"),
    ] {
        assert_eq!(
            err(&mut tw, syn_officer, &cmd),
            mush_industry::commands::NO_SUCH_CLAIM
        );
    }
    assert!(
        submit_direct(
            &mut tw,
            syn_officer,
            contest_action.clone(),
            json!({ "claim": syn_id.to_string(), "state": "uncontested", "revision": 2 }),
            4721,
        )
        .is_err(),
        "issuing org must not lift governing-jurisdiction suspension via direct ClaimContest"
    );
    // Governing jurisdiction CAN release its own suspension.
    ok(&mut tw, registrar, &format!("@claim/release {syn_id}"));
    land(&mut tw, 4722).expect("gov lifts own suspension");

    // Also verify org-attributed suspension (`TenureKind::Renew`: `by: Organization(gov_org)`).
    apply(tw.engine_mut(), &families, |txn| {
        let live = families.claim(txn, syn_id)?.expect("syn claim");
        let next = live
            .with_contest_state(ContestState::Suspended {
                by: PrincipalId::Organization(gov_org),
            })
            .map_err(|_| IndustryError::State)?;
        families.put_claim(txn, live.revision(), next)
    })
    .expect("set org-level gov suspension");
    assert_eq!(
        err(&mut tw, syn_officer, &format!("@claim/release {syn_id}")),
        mush_industry::commands::NO_SUCH_CLAIM
    );
    ok(&mut tw, registrar, &format!("@claim/release {syn_id}"));
    land(&mut tw, 4723).expect("gov lifts org-level suspension");

    // 3. Issuing org's own non-staff suspension: Syndicate CAN lift its own suspension,
    // while distinct governing jurisdiction (`registrar`) CANNOT reopen it to uncontested/contested.
    ok(&mut tw, syn_officer, &format!("@claim/suspend {syn_id}"));
    land(&mut tw, 4730).expect("syndicate suspends own claim");
    assert_eq!(
        err(&mut tw, registrar, &format!("@claim/release {syn_id}")),
        mush_industry::commands::NO_SUCH_CLAIM
    );
    let cur_rev = read(tw.engine_mut(), &families, |txn| {
        families.claim(txn, syn_id).unwrap().unwrap().revision()
    });
    assert!(
        submit_direct(
            &mut tw,
            registrar,
            contest_action,
            json!({ "claim": syn_id.to_string(), "state": "uncontested", "revision": cur_rev }),
            4731,
        )
        .is_err(),
        "governing jurisdiction must not reopen foreign issuer's own suspension"
    );
    ok(&mut tw, syn_officer, &format!("@claim/release {syn_id}"));
    land(&mut tw, 4732).expect("syndicate lifts its own non-staff suspension");
    let final_state = read(tw.engine_mut(), &families, |txn| {
        families
            .claim(txn, syn_id)
            .unwrap()
            .unwrap()
            .contest_state()
            .clone()
    });
    assert_eq!(final_state, ContestState::Uncontested);
}

// ---- P2-MOD-48: @claim/transfer bare-hex ID & >64 transferable claims ----

#[test]
fn p2_mod_48_claim_transfer_bare_hex_and_over_64_claims_never_split_title() {
    let (mut tw, families) = boot_full();
    let alice = tw.spawn_player("Alice");
    let bob = tw.spawn_player("Bob");
    let issuer = create_org_with_officer(&mut tw, "LandOffice", GOD);
    let guild = create_org_with_officer(&mut tw, "MinerGuild", alice);
    ok(
        &mut tw,
        GOD,
        &format!("@org/grant #{}=MinerGuild asset.title.transfer", alice.0),
    );
    let guild_scope = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .expect("orgs")
        .state()
        .org(guild)
        .expect("guild")
        .root_scope;

    // 1. Bare-hex ID (lowercase and uppercase) never splits E4 title from ClaimRecord.holder.
    let place0 = tw
        .world_mut()
        .create(ObjKind::Room, "Sector0", GOD, None)
        .expect("place0");
    ok(
        &mut tw,
        GOD,
        &format!(
            "@claim/file place:{}=org:{}/obj:{}/shared/1/100000/transferable",
            place0.0, issuer.0, alice.0
        ),
    );
    let c0_out = land(&mut tw, 4800).expect("file c0");
    let c0_id = ClaimId::parse(c0_out["claim"].as_str().expect("id")).expect("id");
    let c0_obj = ObjId(u32::try_from(c0_out["object"].as_u64().expect("obj")).expect("u32"));

    let bare_lower = format!("{:016x}", c0_id.0);
    ok(
        &mut tw,
        GOD,
        &format!("@claim/transfer {bare_lower}=obj:{}", bob.0),
    );
    land(&mut tw, 4801).expect("bare-hex transfer lands cleanly");
    assert_eq!(
        snap(&tw).asset(c0_obj).expect("asset").title_owner,
        PrincipalId::Object(bob)
    );
    assert_eq!(
        read(tw.engine_mut(), &families, |txn| {
            families
                .claim(txn, c0_id)
                .unwrap()
                .unwrap()
                .holder()
                .clone()
        }),
        PrincipalId::Object(bob),
        "ClaimRecord.holder must match E4 title after bare-hex transfer"
    );

    let bare_upper = format!("{:016X}", c0_id.0);
    ok(
        &mut tw,
        GOD,
        &format!("@claim/transfer {bare_upper}=obj:{}", alice.0),
    );
    land(&mut tw, 4802).expect("uppercase bare-hex transfer lands cleanly");
    assert_eq!(
        snap(&tw).asset(c0_obj).expect("asset").title_owner,
        PrincipalId::Object(alice)
    );
    assert_eq!(
        read(tw.engine_mut(), &families, |txn| {
            families
                .claim(txn, c0_id)
                .unwrap()
                .unwrap()
                .holder()
                .clone()
        }),
        PrincipalId::Object(alice)
    );

    // 2. Seed 70 (> MAX_DEPOSIT_CHOICES = 64) transferable claims held by `MinerGuild`
    // (scoped to `guild_scope`, where non-staff officer Alice holds `asset.title.transfer`
    // and can legitimately receive title onto herself `obj:alice`).
    let total_claims = MAX_DEPOSIT_CHOICES + 6;
    let mut ids: Vec<(ClaimId, ObjId)> = Vec::with_capacity(total_claims);
    let now = tw.world().world_ic_now();
    let term = ClockStamp {
        clock: now.clock.clone(),
        at: now.at + 100_000,
    };
    for i in 0..total_claims {
        let place = tw
            .world_mut()
            .create(ObjKind::Room, &format!("Plot{i}"), GOD, None)
            .expect("plot");
        let obj = tw
            .world_mut()
            .create(ObjKind::Thing, &format!("Deed{i}"), GOD, None)
            .expect("deed");
        with_ctx(&mut tw, GOD, |ctx| {
            mush_economy::issue_title(
                ctx,
                obj,
                PrincipalId::Organization(guild),
                Some(guild_scope),
                mush_industry::industry_title_issuer(),
                None,
            )
            .expect("issue title");
        });
        let cid = ClaimId(0x48_0000 + i as u64);
        let rec = ClaimRecord::new(
            cid,
            ClaimSubject::Place { place },
            PrincipalId::Organization(issuer),
            PrincipalId::Organization(guild),
            obj,
            Exclusivity::Shared,
            1,
            term.clone(),
            Transferability::Transferable,
        )
        .expect("claim rec");
        apply(tw.engine_mut(), &families, |txn| {
            families.file_claim(txn, rec)
        })
        .expect("file seeded claim");
        ids.push((cid, obj));
    }

    // Transfer the LAST claim (#70, past the 64-entry truncation boundary!) as non-staff Alice
    // using a bare-hex ID!
    let (last_id, last_obj) = *ids.last().expect("last claim");
    let last_bare = format!("{:016x}", last_id.0);
    ok(
        &mut tw,
        alice,
        &format!("@claim/transfer {last_bare}=obj:{}", alice.0),
    );
    land(&mut tw, 4850).expect("70th transferable claim must resolve without truncating");
    assert_eq!(
        snap(&tw).asset(last_obj).expect("asset").title_owner,
        PrincipalId::Object(alice)
    );
    assert_eq!(
        read(tw.engine_mut(), &families, |txn| {
            families
                .claim(txn, last_id)
                .unwrap()
                .unwrap()
                .holder()
                .clone()
        }),
        PrincipalId::Object(alice),
        "70th claim must never split E4 title from ClaimRecord.holder"
    );

    // 3. Staff transfer of claim #69 among >64 transferable claims also succeeds cleanly.
    let (staff_id, staff_obj) = ids[ids.len() - 2];
    ok(
        &mut tw,
        GOD,
        &format!("@claim/transfer {staff_id}=obj:{}", bob.0),
    );
    land(&mut tw, 4851).expect("staff transfer of 69th claim resolves cleanly");
    assert_eq!(
        snap(&tw).asset(staff_obj).expect("asset").title_owner,
        PrincipalId::Object(bob)
    );
    assert_eq!(
        read(tw.engine_mut(), &families, |txn| {
            families
                .claim(txn, staff_id)
                .unwrap()
                .unwrap()
                .holder()
                .clone()
        }),
        PrincipalId::Object(bob)
    );

    // 4. Retry-sync heals if `TransferTitle` landed while `ClaimMove` was dropped.
    let (retry_id, retry_obj) = ids[ids.len() - 3];
    ok(
        &mut tw,
        alice,
        &format!("@claim/transfer {retry_id}=obj:{}", alice.0),
    );
    let _dropped = tw.engine_mut().take_authoritative_submit();
    assert_eq!(
        snap(&tw).asset(retry_obj).expect("asset").title_owner,
        PrincipalId::Object(alice)
    );
    ok(
        &mut tw,
        alice,
        &format!("@claim/transfer {retry_id}=obj:{}", alice.0),
    );
    land(&mut tw, 4852).expect("retry-sync completes ClaimMove without re-posting TransferTitle");
    assert_eq!(
        read(tw.engine_mut(), &families, |txn| {
            families
                .claim(txn, retry_id)
                .unwrap()
                .unwrap()
                .holder()
                .clone()
        }),
        PrincipalId::Object(alice)
    );
}

// ---- P2-MOD-49: Claim/collateral/deposit oracles + staff-page deduplication ----

#[test]
fn p2_mod_49_claim_oracles_and_staff_page_spam_and_claim_file_oracle() {
    let (mut tw, families) = boot_full();
    let registrar = tw.spawn_player("Registrar");
    let alice = tw.spawn_player("Alice");
    let eve = tw.spawn_player("Eve");
    let place = tw
        .world_mut()
        .create(ObjKind::Room, "Belt", GOD, None)
        .expect("place");

    let crown_org = create_org_with_officer(&mut tw, "Crown", registrar);
    let eve_org = create_org_with_officer(&mut tw, "EveCo", eve);

    let foreign_dep = seed_deposit(
        &mut tw,
        &families,
        OreDepositId(0x4901),
        place,
        4901,
        Some(PrincipalId::Organization(crown_org)),
    );
    let unj_dep = seed_deposit(&mut tw, &families, OreDepositId(0x4902), place, 4902, None);
    let eve_dep = seed_deposit(
        &mut tw,
        &families,
        OreDepositId(0x4903),
        place,
        4903,
        Some(PrincipalId::Organization(eve_org)),
    );
    let fake_dep = OreDepositId(0x4999);

    // File one pledged claim and one disputed claim under Crown.
    ok(
        &mut tw,
        GOD,
        &format!(
            "@claim/file {}=org:{}/obj:{}/shared/1/100000/transferable",
            foreign_dep.id(),
            crown_org.0,
            alice.0
        ),
    );
    let pledged_out = land(&mut tw, 4901).expect("pledged claim");
    let pledged_id = ClaimId::parse(pledged_out["claim"].as_str().expect("id")).expect("id");
    ok(&mut tw, GOD, &format!("@claim/pledge {pledged_id}=55"));

    ok(
        &mut tw,
        GOD,
        &format!(
            "@claim/file {}=org:{}/obj:{}/shared/1/100000/transferable",
            foreign_dep.id(),
            crown_org.0,
            alice.0
        ),
    );
    let disputed_out = land(&mut tw, 4902).expect("disputed claim");
    let disputed_id = ClaimId::parse(disputed_out["claim"].as_str().expect("id")).expect("id");
    let disputed_obj =
        ObjId(u32::try_from(disputed_out["object"].as_u64().expect("obj")).expect("u32"));
    ok(
        &mut tw,
        GOD,
        &format!("@asset/stolen #{}=public disputed title", disputed_obj.0),
    );
    let fake_claim = ClaimId(0x0000_0000_dead_beef);

    // 1. Unauthenticated Eve gets byte-identical `NO_SUCH_CLAIM` across real pledged/disputed
    // claims and fake IDs, with ZERO staged submissions and ZERO staff pages!
    for switch in ["seize", "contest", "suspend", "release"] {
        let on_pledged = err(&mut tw, eve, &format!("@claim/{switch} {pledged_id}"));
        let on_disputed = err(&mut tw, eve, &format!("@claim/{switch} {disputed_id}"));
        let on_fake = err(&mut tw, eve, &format!("@claim/{switch} {fake_claim}"));
        assert_eq!(on_pledged, mush_industry::commands::NO_SUCH_CLAIM);
        assert_eq!(on_disputed, mush_industry::commands::NO_SUCH_CLAIM);
        assert_eq!(on_fake, mush_industry::commands::NO_SUCH_CLAIM);
        assert!(tw.engine_mut().take_authoritative_submit().is_none());
    }
    assert_eq!(
        tw.world()
            .module_state::<IndustryStore>()
            .expect("store")
            .claim_pages_emitted(),
        0,
        "unauthenticated @claim/* calls must emit ZERO level-2 staff pages"
    );

    // 2. Unauthenticated `@claim/pledge` cannot distinguish real from nonexistent claims
    // via syntax-error-vs-No-such-claim oracle.
    let bad_real = err(
        &mut tw,
        eve,
        &format!("@claim/pledge {pledged_id}=notanumber"),
    );
    let bad_fake = err(
        &mut tw,
        eve,
        &format!("@claim/pledge {fake_claim}=notanumber"),
    );
    assert_eq!(
        bad_real, bad_fake,
        "@claim/pledge with bad contract syntax must be byte-identical for real and fake claims"
    );
    let auth_real = err(&mut tw, eve, &format!("@claim/pledge {pledged_id}=77"));
    let auth_fake = err(&mut tw, eve, &format!("@claim/pledge {fake_claim}=77"));
    assert_eq!(auth_real, mush_industry::commands::NO_SUCH_CLAIM);
    assert_eq!(auth_fake, mush_industry::commands::NO_SUCH_CLAIM);

    // 3. Authorized level-2 staff pages are deduplicated within `CLAIM_PAGE_COOLDOWN_SECS`.
    for _ in 0..10 {
        ok(&mut tw, registrar, &format!("@claim/seize {pledged_id}"));
    }
    assert_eq!(
        tw.world()
            .module_state::<IndustryStore>()
            .expect("store")
            .claim_pages_emitted(),
        1,
        "10 rapid authorized @claim/seize calls on a pledged claim must emit only 1 staff page"
    );
    for _ in 0..10 {
        ok(
            &mut tw,
            registrar,
            &format!("@claim/contest {disputed_id}=0"),
        );
        let _ = tw.engine_mut().take_authoritative_submit();
    }
    assert_eq!(
        tw.world()
            .module_state::<IndustryStore>()
            .expect("store")
            .claim_pages_emitted(),
        2,
        "10 rapid authorized @claim/contest calls on a disputed claim must emit only 1 staff page"
    );
    // Past cooldown, a new page may be emitted once again.
    let later = tw.world().now() + CLAIM_PAGE_COOLDOWN_SECS + 10;
    tw.set_now(later);
    ok(&mut tw, registrar, &format!("@claim/seize {pledged_id}"));
    assert_eq!(
        tw.world()
            .module_state::<IndustryStore>()
            .expect("store")
            .claim_pages_emitted(),
        3
    );

    // 4. Direct `ClaimFile` deposit/jurisdiction oracle is closed:
    // Mint a valid unclaimed claim title for Eve so `CLAIM_FILE_OBJECTS` vouches for it,
    // then probe nonexistent, foreign-jurisdiction, un-jurisdicted, and `Place` subjects.
    let fresh_obj = tw
        .world_mut()
        .create(ObjKind::Thing, "EveClaimObj", eve, None)
        .expect("fresh obj");
    with_ctx(&mut tw, eve, |ctx| {
        mush_economy::issue_title(
            ctx,
            fresh_obj,
            PrincipalId::Object(eve),
            None,
            mush_industry::industry_title_issuer(),
            Some(mush_economy::CatalogRef {
                ruleset: SmolStr::new_static(mush_industry::CLAIM_CATALOG_RULESET),
                code: SmolStr::new_static(mush_industry::CLAIM_CATALOG_CODE),
            }),
        )
        .expect("issue claim title");
    });
    let file_action = action_ref(&tw.engine().registry, IndustryVerb::ClaimFile).expect("file");
    let now_at = tw.world().world_ic_now().at;

    // 4a. With invalid `exclusivity` (before deposit lookup): ALL deposits (including Eve's own!)
    // return the exact same InvalidRequest error — no deposit/jurisdiction leak.
    let invalid_results: Vec<String> = [fake_dep, foreign_dep.id(), unj_dep.id(), eve_dep.id()]
        .into_iter()
        .enumerate()
        .map(|(i, dep_id)| {
            submit_direct(
                &mut tw,
                eve,
                file_action.clone(),
                json!({
                    "subject": dep_id.to_string(),
                    "issuer": format!("org:{}", eve_org.0),
                    "holder": format!("obj:{}", eve.0),
                    "object": fresh_obj.0,
                    "exclusivity": "bogus",
                    "priority": 1,
                    "term": now_at + 10_000,
                    "transferability": "transferable",
                }),
                4920 + i as u64,
            )
            .expect_err("invalid exclusivity must refuse")
        })
        .collect();
    assert!(
        invalid_results.windows(2).all(|w| w[0] == w[1]),
        "ClaimFile parameter validation must not leak deposit existence or jurisdiction: {invalid_results:?}"
    );

    // 4b. With valid parameters and a vouched object: nonexistent, foreign, un-jurisdicted, and
    // Place subjects ALL return the byte-identical Unauthorized + REFUSED error!
    let unauthorized_results: Vec<String> = [
        fake_dep.to_string(),
        foreign_dep.id().to_string(),
        unj_dep.id().to_string(),
        format!("place:{}", place.0),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, subject)| {
        submit_direct(
            &mut tw,
            eve,
            file_action.clone(),
            json!({
                "subject": subject,
                "issuer": format!("org:{}", eve_org.0),
                "holder": format!("obj:{}", eve.0),
                "object": fresh_obj.0,
                "exclusivity": "shared",
                "priority": 1,
                "term": now_at + 10_000,
                "transferability": "transferable",
            }),
            4930 + i as u64,
        )
        .expect_err("unauthorized subject must refuse")
    })
    .collect();
    assert!(
        unauthorized_results.windows(2).all(|w| w[0] == w[1]),
        "missing, foreign, un-jurisdicted, and Place ClaimFile refusals must be byte-identical: {unauthorized_results:?}"
    );
}

// ---- P2-MOD-77: wo_restamp re-anchors slot reservation & slot_expiry_schedule ----

#[test]
fn p2_mod_77_wo_restamp_reanchors_slot_reservation_and_slot_expiry_schedule() {
    let (mut tw, families) = boot_full();
    register_instrument_kind(&mut tw, "gold", InstrumentKind::Currency);
    let faucet = open_sys_faucet(&mut tw, "faucet", "gold");
    let budget = open_account(&mut tw, GOD, "gold");
    let _payee = open_account(&mut tw, GOD, "gold");
    ok(
        &mut tw,
        GOD,
        &format!("@econ/mint {faucet}={budget} 100000"),
    );

    let fac = tw
        .world_mut()
        .create(ObjKind::Thing, "Yard", GOD, None)
        .expect("fac");
    ok(&mut tw, GOD, &format!("@facility/open {}=2 smelt", fac.0));
    let source = make_endpoint(&mut tw, GOD, "Src");
    let dest = make_endpoint(&mut tw, GOD, "Dst");
    apply(tw.engine_mut(), &families, |txn| {
        families
            .inventory
            .credit(txn, source, &cid("metals"), 1_000)
            .map(|_| ())
            .map_err(IndustryError::Inventory)
    })
    .expect("seed metals");

    // Draft and submit a standard work order in WorldIc generation 1.
    ok(
        &mut tw,
        GOD,
        &format!(
            "@wo/draft tacsim.core/alloy_smelt/1={} 1 {} {} {}",
            fac.0,
            budget.0,
            source.vault().0,
            dest.vault().0
        ),
    );
    let drafted = land(&mut tw, 7701).expect("draft");
    let order_id = WorkOrderId::parse(drafted["order"].as_str().expect("id")).expect("id");
    ok(&mut tw, GOD, &format!("@wo/submit {order_id}"));
    land(&mut tw, 7702).expect("submit");

    // Also reserve a non-WO (refit-shaped) slot on the same facility to assert P2-MOD-2 preservation.
    let now_gen1 = tw.world().world_ic_now();
    let refit_holder = ScopedIdempotencyKey::new(
        IdempotencyScope {
            principal: mush_core::PrincipalId::Object { id: GOD },
            owner: mush_industry::owner(),
            namespace: cid("bay_refit"),
        },
        1,
        IdempotencyKey::Request(RequestUid::from_bytes([0x77; 16])),
    );
    let refit_res_id = apply(tw.engine_mut(), &families, |txn| {
        families.reserve_slot(txn, fac, 2, &refit_holder, &now_gen1)
    })
    .expect("reserve refit slot");

    let cap_resource = mush_industry::facility_resource(fac).expect("res");
    let wo_cap_id = ReservationId::derive(&wo_holder(order_id).expect("holder"), &cap_resource)
        .expect("wo cap id");
    let slot_sid = {
        let hex: String = wo_cap_id.0.to_hex().chars().take(24).collect();
        let local = CanonicalId::new(format!("slot.{}.{hex}", fac.0)).expect("local");
        mush_core::ScheduleId::derive(&mush_industry::owner(), &local).expect("sid")
    };

    // Confirm both slot reservation and slot_expiry_schedule start on generation 1.
    let (slot_gen1, sched_gen1) = read(tw.engine_mut(), &families, |txn| {
        let slot = families
            .slot_reservation(txn, fac, wo_cap_id)
            .expect("slot")
            .expect("held");
        let sched = txn
            .get(&families.slot_expiry_schedules, &slot_sid)
            .expect("get")
            .expect("sched")
            .clone();
        (slot.expires().clock.clone(), sched.clock().clone())
    });
    assert_eq!(slot_gen1, now_gen1.clock);
    assert_eq!(sched_gen1, now_gen1.clock);

    // Bump WorldIc to generation 2!
    let epoch = tw.world().next_world_timeline_epoch(2).expect("next epoch");
    tw.world_mut()
        .append_world_timeline_epoch(epoch)
        .expect("append epoch");
    let now_gen2 = tw.world().world_ic_now();
    assert_ne!(now_gen2.clock, now_gen1.clock);

    // Restamp the order -> must re-anchor BOTH facility slot reservation AND slot_expiry_schedule!
    ok(&mut tw, GOD, &format!("@wo/restamp {order_id}"));
    land(&mut tw, 7703).expect("restamp");

    let (slot_after, sched_after, refit_after) = read(tw.engine_mut(), &families, |txn| {
        let slot = families
            .slot_reservation(txn, fac, wo_cap_id)
            .expect("slot")
            .expect("held");
        let sched = txn
            .get(&families.slot_expiry_schedules, &slot_sid)
            .expect("get")
            .expect("sched")
            .clone();
        let refit = families
            .slot_reservation(txn, fac, refit_res_id)
            .expect("refit")
            .expect("refit held");
        (slot, sched, refit)
    });
    assert_eq!(slot_after.state(), ReservationState::Held);
    assert_eq!(
        slot_after.expires().clock,
        now_gen2.clock,
        "facility slot reservation must be re-anchored to generation 2"
    );
    assert_eq!(
        sched_after.clock(),
        &now_gen2.clock,
        "slot_expiry_schedule must be re-anchored to generation 2"
    );
    assert_eq!(
        sched_after.next_due(),
        slot_after.expires(),
        "slot_expiry_schedule next_due must match re-anchored slot expiry"
    );
    // P2-MOD-2 invariant preserved: refit slot is completely untouched (Held, rev 1),
    // and `renew_slot_for` itself refuses a non-standard-WO (refit) reservation.
    assert_eq!(refit_after.state(), ReservationState::Held);
    assert_eq!(refit_after.revision(), 1);
    assert_eq!(refit_after.expires().clock, now_gen1.clock);
    let refit_renew = apply(tw.engine_mut(), &families, |txn| {
        families.renew_slot_for(
            txn,
            fac,
            refit_res_id,
            refit_after.revision(),
            &now_gen2,
            60,
        )
    });
    assert!(
        matches!(refit_renew, Err(IndustryError::NoWorkOrder)),
        "renew_slot_for must refuse refit/integration slot reservations (P2-MOD-2): {refit_renew:?}"
    );

    // Advance past the re-anchored generation-2 expiry -> slot_expiry schedule is discovered
    // and drives cleanly in generation 2, while leaving the refit slot untouched!
    let past_expiry_wall =
        tw.world().now() + ((slot_after.expires().at - now_gen2.at) / 2) + 10_000;
    tw.set_now(past_expiry_wall);
    let past_ic = tw.world().world_ic_now();
    assert!(
        matches!(
            slot_after.expires().checked_cmp(&past_ic),
            Ok(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
        ),
        "past_ic must be at or past the re-anchored generation-2 deadline"
    );
    let elapsed = read(tw.engine_mut(), &families, |txn| {
        families
            .first_elapsed_slot_reservation(txn, fac, &past_ic)
            .expect("elapsed lookup")
    });
    assert_eq!(
        elapsed.map(|r| r.id()),
        Some(wo_cap_id),
        "first_elapsed_slot_reservation must find the re-anchored standard WO slot on gen 2"
    );

    // Drive the durable slot_expiry schedule source to reclaim the restamped abandoned slot!
    let candidates = tw
        .engine()
        .discover_scheduled_actions(ScheduleDriveCause::Live, None, 32)
        .expect("discover")
        .candidates()
        .to_vec();
    let expiry_cand = candidates
        .into_iter()
        .find(|c| c.action().key().id.as_str() == ACTION_WO_RECLAIM)
        .expect("re-anchored slot_expiry candidate must be discovered in generation 2");
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
            CommitToken::from_bytes([0x7A; 16]),
        )
        .expect("prepare scheduled WoReclaim");
    let (_, installation) = prepared.into_parts();
    tw.engine_mut()
        .install_prepared_action(installation)
        .expect("install scheduled WoReclaim");

    let (wo_after, slot_final, refit_final) = read(tw.engine_mut(), &families, |txn| {
        let wo = families.work_order(txn, order_id).unwrap().unwrap();
        let slot = families
            .slot_reservation(txn, fac, wo_cap_id)
            .unwrap()
            .unwrap();
        let refit = families
            .slot_reservation(txn, fac, refit_res_id)
            .unwrap()
            .unwrap();
        (wo, slot, refit)
    });
    assert_eq!(*wo_after.state(), WorkOrderState::Failed);
    assert_eq!(slot_final.state(), ReservationState::Expired);
    assert_eq!(
        refit_final.state(),
        ReservationState::Held,
        "P2-MOD-2: refit slot must NEVER be expired by work-order expiry"
    );
    assert_eq!(refit_final.revision(), 1);
    let _ = facility_capacity_key(fac).expect("cap key");
}

// ---- P2-MOD-78: Staff ClaimContest encumbrance scan bounded & fail-closed ----

#[test]
fn p2_mod_78_staff_claim_contest_bounded_within_projection_limit() {
    let (mut tw, families) = boot_full();
    let alice = tw.spawn_player("Alice");
    let issuer = create_org_with_officer(&mut tw, "Crown", GOD);
    let place = tw
        .world_mut()
        .create(ObjKind::Room, "Belt", GOD, None)
        .expect("place");

    // 1. Create 400 encumbered NON-claim E4 assets (Collateral + Disputed) through real E4 posts
    // to prove staff `ClaimContest` ignores non-claim assets instead of blowing up projection bytes.
    let mut non_claim_objs = Vec::with_capacity(400);
    for i in 0..400u32 {
        let obj = tw
            .world_mut()
            .create(ObjKind::Thing, &format!("Ship{i}"), alice, None)
            .expect("ship");
        non_claim_objs.push(obj);
    }
    with_ctx(&mut tw, GOD, |ctx| {
        for (i, &obj) in non_claim_objs.iter().enumerate() {
            mush_economy::issue_title(
                ctx,
                obj,
                PrincipalId::Object(alice),
                None,
                mush_industry::industry_title_issuer(),
                None,
            )
            .expect("issue title");
            let plan = mush_economy::EconomyPlan {
                asset_ops: vec![
                    mush_economy::AssetOp::Encumber {
                        object: obj,
                        encumbrance: mush_economy::Encumbrance::Collateral {
                            contract: mush_economy::ContractId(i as u64 + 1),
                        },
                    },
                    mush_economy::AssetOp::Encumber {
                        object: obj,
                        encumbrance: mush_economy::Encumbrance::Disputed {
                            claimant: PrincipalId::Object(GOD),
                            reason: "test dispute".to_string(),
                        },
                    },
                ],
                ..mush_economy::EconomyPlan::new(
                    mush_economy::IdempotencyKey::new("test.encumber", format!("nc/{i}")),
                    PrincipalId::Object(GOD),
                    mush_economy::EconomicCause::Staff {
                        reason: "seed non-claim encumbrances".to_string(),
                    },
                    Vec::new(),
                )
            };
            mush_economy::post(ctx, &plan).expect("encumber non-claim asset");
        }
    });

    // File 1 clean claim and 1 pledged claim.
    ok(
        &mut tw,
        GOD,
        &format!(
            "@claim/file place:{}=org:{}/obj:{}/shared/1/100000/transferable",
            place.0, issuer.0, alice.0
        ),
    );
    let clean_out = land(&mut tw, 7801).expect("clean claim");
    let clean_id = ClaimId::parse(clean_out["claim"].as_str().expect("id")).expect("id");

    ok(
        &mut tw,
        GOD,
        &format!(
            "@claim/file place:{}=org:{}/obj:{}/shared/1/100000/transferable",
            place.0, issuer.0, alice.0
        ),
    );
    let pledged_out = land(&mut tw, 7802).expect("pledged claim");
    let pledged_id = ClaimId::parse(pledged_out["claim"].as_str().expect("id")).expect("id");
    let pledged_obj =
        ObjId(u32::try_from(pledged_out["object"].as_u64().expect("obj")).expect("u32"));
    ok(&mut tw, GOD, &format!("@claim/pledge {pledged_id}=888"));

    // Staff offer projection MUST ignore all 400 non-claim assets and list ONLY `pledged_obj`.
    let contest_action =
        action_ref(&tw.engine().registry, IndustryVerb::ClaimContest).expect("contest");
    {
        let engine = tw.engine();
        let query = engine
            .legal_action_query(
                mush_core::PrincipalId::Object { id: GOD },
                GOD,
                mush_core::PerceivedRef::SelfActor,
            )
            .expect("query");
        let offers = engine
            .project_legal_actions(&query)
            .expect("staff projection must succeed");
        let projected = offers
            .projected_for_action(&contest_action)
            .expect("staff ClaimContest offered");
        let req = RequestUid::from_bytes([0x78; 16]);
        let resolved = engine
            .resolve_legal_action(
                &offers,
                mush_core::OfferExecutionContext {
                    request: req,
                    principal: mush_core::PrincipalId::Object { id: GOD },
                    actor: GOD,
                    idempotency: ScopedIdempotencyKey::new(
                        IdempotencyScope {
                            principal: mush_core::PrincipalId::Object { id: GOD },
                            owner: contest_action.key().owner.clone(),
                            namespace: cid(contest_action.key().id.as_str()),
                        },
                        1,
                        IdempotencyKey::Request(req),
                    ),
                    viewer_scope: query.viewer_scope(),
                    now: None,
                },
                mush_core::OfferSubmission {
                    offer: projected.offer(),
                    subject: projected.subject().clone(),
                    targets: Vec::new(),
                    parameters: json!({
                        "claim": clean_id.to_string(),
                        "state": "suspended",
                        "revision": 1,
                    }),
                },
            )
            .expect("resolve staff ClaimContest");
        let (_, inv, _) = resolved.into_authoritative_parts();
        let params = inv.invocation().parameters().as_object().expect("params");
        assert_eq!(params[CLAIM_CONTEST_PLEDGED], json!([pledged_obj.0]));
        assert_eq!(params[CLAIM_CONTEST_DISPUTED], json!([]));
        assert_eq!(params[CLAIM_CONTEST_PLEDGED_TRUNCATED], json!(false));
        assert_eq!(params[CLAIM_CONTEST_DISPUTED_TRUNCATED], json!(false));
    }

    // Pledged claim seizure is still refused.
    assert!(
        submit_direct(
            &mut tw,
            GOD,
            contest_action.clone(),
            json!({ "claim": pledged_id.to_string(), "state": "seized", "revision": 1 }),
            7803,
        )
        .is_err(),
        "pledged claim seizure still refuses"
    );

    // 2. Bind > MAX_CONTEST_ENCUMBRANCES (260) of those encumbered assets to `ClaimRecord`s ->
    // projection MUST stay bounded inside MAX_PROJECTED_LEGAL_ACTION_BYTES AND fail closed!
    let now = tw.world().world_ic_now();
    let term = ClockStamp {
        clock: now.clock.clone(),
        at: now.at + 100_000,
    };
    for (i, &obj) in non_claim_objs
        .iter()
        .take(MAX_CONTEST_ENCUMBRANCES + 4)
        .enumerate()
    {
        let p = ObjId(70_000 + u32::try_from(i).expect("u32"));
        let rec = ClaimRecord::new(
            ClaimId(0x78_0000 + i as u64),
            ClaimSubject::Place { place: p },
            PrincipalId::Organization(issuer),
            PrincipalId::Object(alice),
            obj,
            Exclusivity::Shared,
            1,
            term.clone(),
            Transferability::Transferable,
        )
        .expect("claim rec");
        apply(tw.engine_mut(), &families, |txn| {
            families.file_claim(txn, rec)
        })
        .expect("file pledged claim");
    }
    {
        let engine = tw.engine();
        let query = engine
            .legal_action_query(
                mush_core::PrincipalId::Object { id: GOD },
                GOD,
                mush_core::PerceivedRef::SelfActor,
            )
            .expect("query");
        let offers = engine
            .project_legal_actions(&query)
            .expect("staff projection must stay inside MAX_PROJECTED_LEGAL_ACTION_BYTES past 256 encumbered claims");
        let projected = offers
            .projected_for_action(&contest_action)
            .expect("staff ClaimContest still offered");
        let req = RequestUid::from_bytes([0x79; 16]);
        let resolved = engine
            .resolve_legal_action(
                &offers,
                mush_core::OfferExecutionContext {
                    request: req,
                    principal: mush_core::PrincipalId::Object { id: GOD },
                    actor: GOD,
                    idempotency: ScopedIdempotencyKey::new(
                        IdempotencyScope {
                            principal: mush_core::PrincipalId::Object { id: GOD },
                            owner: contest_action.key().owner.clone(),
                            namespace: cid(contest_action.key().id.as_str()),
                        },
                        1,
                        IdempotencyKey::Request(req),
                    ),
                    viewer_scope: query.viewer_scope(),
                    now: None,
                },
                mush_core::OfferSubmission {
                    offer: projected.offer(),
                    subject: projected.subject().clone(),
                    targets: Vec::new(),
                    parameters: json!({
                        "claim": clean_id.to_string(),
                        "state": "suspended",
                        "revision": 1,
                    }),
                },
            )
            .expect("resolve bounded staff ClaimContest");
        let (_, inv, _) = resolved.into_authoritative_parts();
        let params = inv.invocation().parameters().as_object().expect("params");
        assert_eq!(
            params[CLAIM_CONTEST_PLEDGED].as_array().expect("arr").len(),
            MAX_CONTEST_ENCUMBRANCES
        );
        assert_eq!(
            params[CLAIM_CONTEST_DISPUTED]
                .as_array()
                .expect("arr")
                .len(),
            MAX_CONTEST_ENCUMBRANCES
        );
        assert_eq!(params[CLAIM_CONTEST_PLEDGED_TRUNCATED], json!(true));
        assert_eq!(params[CLAIM_CONTEST_DISPUTED_TRUNCATED], json!(true));
    }

    // Fail-closed: seizing & contesting refuse when truncated, while suspending still works.
    assert!(
        submit_direct(
            &mut tw,
            GOD,
            contest_action.clone(),
            json!({ "claim": clean_id.to_string(), "state": "seized", "revision": 1 }),
            7810,
        )
        .is_err(),
        "staff seizure must fail closed when pledged encumbrances truncated"
    );
    assert!(
        submit_direct(
            &mut tw,
            GOD,
            contest_action.clone(),
            json!({ "claim": clean_id.to_string(), "state": "contested", "revision": 1, "contract": 0 }),
            7811,
        )
        .is_err(),
        "staff contest must fail closed when disputed encumbrances truncated"
    );
    submit_direct(
        &mut tw,
        GOD,
        contest_action,
        json!({ "claim": clean_id.to_string(), "state": "suspended", "revision": 1 }),
        7812,
    )
    .expect("staff suspension remains permitted even when encumbrances truncated");
}
