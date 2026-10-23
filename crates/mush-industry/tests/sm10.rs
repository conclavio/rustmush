//! Doc 20 cut SM.10 — the composed tenure authority and its one verb
//! (`industry_claim_tenure`), by the offer route with a stub authority (§9,
//! D-20-12): the server-authored row and its structural checks, the
//! block-wide stake / take-over / renewal / lapse / contest / seed bundles, the
//! block-level tenure seams (`tenure_lapse`, `tenure_hold`) that move a block
//! with no claim to carry its epoch (reviewer F4), and the staging seam's
//! all-or-nothing contract.
//!
//! The rig is `sm6.rs`'s (itself `e7c3.rs`'s): `boot()` composes economy,
//! inventory and industry in production order, `apply()`/`read()` open the
//! owner-stamped transaction a provider gets, and `submit()` drives one
//! submission the way the world task does (regenerate the actor's own offer,
//! resolve, prepare, install).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use mush_core::{
    ClockStamp, CommitToken, GOD, IdempotencyKey, IdempotencyScope, ObjId, OfferKeyRing,
    OfferSecret, OwnerId, PerceivedRef, RequestUid, ResolutionKeyRing, ResolutionSecret,
    ScopedIdempotencyKey,
};
use mush_economy::{ContractId, VaultId};
use mush_industry::{
    Accessibility, BandId, ClaimId, ClaimRecord, ClaimRev, ClaimSubject, ContestState, CurvePoint,
    IndustryError, IndustryFamilies, IndustryHandles, IndustryModule, IndustryVerb, OreDeposit,
    OreDepositId, TenureAct, TenureAuthority, TenureKind, TenureMark, TenureOutcome, TenureRecord,
    YieldCurve,
};
use mush_inventory::{EndpointRef, Grade, InventoryHandleCell, InventoryModule};
use mush_org::PrincipalId;
use mush_testkit::TestWorld;
use smol_str::SmolStr;

// ---- the stub authority ----

/// Vouches for whatever row the test set for an actor — or, with `any` set, hands that one row
/// to EVERY actor asked about (a leaking authority, so the provider's own actor check is what
/// decides); records every outcome `stage` was handed; refuses at `stage` while `fail` holds a
/// line.
#[derive(Default)]
struct Stub {
    rows: Mutex<BTreeMap<ObjId, TenureAct>>,
    any: Mutex<Option<TenureAct>>,
    fail: Mutex<Option<String>>,
    seen: Mutex<Vec<TenureOutcome>>,
}

impl TenureAuthority for Stub {
    fn next_tenure(&self, _engine: &mush_core::Engine, actor: ObjId) -> Option<TenureAct> {
        if let Some(row) = self.any.lock().ok()?.clone() {
            return Some(row);
        }
        self.rows.lock().ok()?.get(&actor).cloned()
    }

    fn stage(
        &self,
        _txn: &mut mush_core::ActionTxn<'_>,
        _act: &TenureAct,
        outcome: &TenureOutcome,
        _now: &ClockStamp,
    ) -> Result<(), String> {
        if let Some(line) = self.fail.lock().map_err(|_| "poisoned".to_owned())?.clone() {
            return Err(line);
        }
        self.seen
            .lock()
            .map_err(|_| "poisoned".to_owned())?
            .push(*outcome);
        Ok(())
    }
}

impl Stub {
    fn vouch(&self, act: TenureAct) {
        self.rows.lock().expect("rows").insert(act.actor, act);
    }
    fn clear(&self) {
        self.rows.lock().expect("rows").clear();
        *self.any.lock().expect("any") = None;
    }
    /// Hand `act` to every actor asked about, whoever it names.
    fn leak(&self, act: TenureAct) {
        *self.any.lock().expect("any") = Some(act);
    }
    fn last(&self) -> TenureOutcome {
        *self.seen.lock().expect("seen").last().expect("an outcome")
    }
}

// ---- the rig ----

fn boot(stub: &Arc<Stub>) -> (TestWorld, IndustryFamilies) {
    let cell: InventoryHandleCell = Arc::default();
    let authority: Arc<dyn TenureAuthority> = Arc::clone(stub) as Arc<dyn TenureAuthority>;
    let industry = IndustryModule::new()
        .with_inventory(Arc::clone(&cell))
        .with_tenure_authority(authority);
    let mut world = TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(mush_economy::EconomyModule::new()),
        Box::new(
            InventoryModule::new()
                .with_quantity_delegates(["industry"])
                .with_handle_cell(Arc::clone(&cell)),
        ),
        Box::new(industry),
    ]);
    world
        .engine_mut()
        .set_offer_keys(OfferKeyRing::new(OfferSecret::new(1, [0x41; 32]).unwrap(), []).unwrap());
    world.engine_mut().set_resolution_keys(
        ResolutionKeyRing::new(ResolutionSecret::new(1, [0x52; 32]).unwrap(), []).unwrap(),
    );
    let families = world
        .world()
        .module_state::<IndustryHandles>()
        .expect("industry publishes its handles")
        .0
        .clone();
    (world, families)
}

/// The world's IC instant, the offer's `now`.
fn now(world: &TestWorld) -> ClockStamp {
    world.world().world_ic_now()
}

fn at(world: &TestWorld, secs: i64) -> ClockStamp {
    let now = now(world);
    ClockStamp {
        clock: now.clock,
        at: now.at + secs,
    }
}

fn obj(id: ObjId) -> PrincipalId {
    PrincipalId::Object(id)
}

fn org(id: u64) -> PrincipalId {
    PrincipalId::Organization(mush_org::OrgId(id))
}

/// A live block place object.
fn place(world: &mut TestWorld, name: &str) -> ObjId {
    world
        .world_mut()
        .create(mush_core::ObjKind::Thing, name, GOD, Some(mush_core::LIMBO))
        .expect("a block place")
}

fn apply<T>(
    world: &mut TestWorld,
    families: &IndustryFamilies,
    body: impl FnOnce(&mut mush_core::ActionTxn<'_>) -> Result<T, IndustryError>,
) -> Result<T, IndustryError> {
    let engine = world.engine_mut();
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

fn read<T>(
    world: &mut TestWorld,
    families: &IndustryFamilies,
    body: impl FnOnce(&mut mush_core::ActionTxn<'_>) -> T,
) -> T {
    let mut txn = world
        .engine_mut()
        .action_transaction(&families.deposits)
        .expect("industry transaction");
    body(&mut txn)
}

/// One surface-shaped deposit at `place` under the issuer `org(7)`.
fn deposit(
    world: &mut TestWorld,
    families: &IndustryFamilies,
    id: u64,
    place: ObjId,
    good: &str,
    jurisdiction: PrincipalId,
) -> OreDepositId {
    let body = OreDeposit::new(
        OreDepositId(id),
        place,
        mush_core::CanonicalId::new(good).expect("good"),
        Grade::new(0).expect("grade"),
        Accessibility::new(100).expect("accessibility"),
        BandId::new("surface").expect("band"),
        BandId::new("surface").expect("band"),
        YieldCurve::new(vec![CurvePoint {
            cumulative: 0,
            per_cycle: 1,
        }])
        .expect("curve"),
        EndpointRef(VaultId(950)),
        100,
    )
    .expect("body")
    .with_external("tacsim.surface")
    .expect("external")
    .with_jurisdiction(jurisdiction);
    apply(world, families, |txn| families.open_deposit(txn, body)).expect("the deposit opens");
    OreDepositId(id)
}

/// A block of `n` deposits (one per family) at a fresh place.
fn block(
    world: &mut TestWorld,
    families: &IndustryFamilies,
    name: &str,
    base: u64,
    n: usize,
) -> (ObjId, Vec<OreDepositId>) {
    let place = place(world, name);
    let goods = ["metal_ore", "ice_ore", "carbon_ore", "strategic_ore"];
    let ids = goods
        .iter()
        .take(n)
        .enumerate()
        .map(|(i, good)| deposit(world, families, base + i as u64, place, good, org(7)))
        .collect();
    (place, ids)
}

fn claims_over(
    world: &mut TestWorld,
    families: &IndustryFamilies,
    deposit: OreDepositId,
) -> Vec<ClaimRecord> {
    read(world, families, |txn| {
        families
            .claims_over(txn, ClaimSubject::Deposit { deposit })
            .expect("claims read")
    })
}

fn claim_of(world: &mut TestWorld, families: &IndustryFamilies, id: ClaimId) -> ClaimRecord {
    read(world, families, |txn| {
        families
            .claim(txn, id)
            .expect("claim reads")
            .expect("claim")
    })
}

fn tenure_of(world: &mut TestWorld, families: &IndustryFamilies, place: ObjId) -> TenureRecord {
    read(world, families, |txn| {
        families.tenure(txn, place).expect("tenure reads")
    })
}

fn tenure_stored(world: &mut TestWorld, families: &IndustryFamilies, place: ObjId) -> bool {
    read(world, families, |txn| {
        txn.get(&families.tenure, &place)
            .expect("tenure family")
            .is_some()
    })
}

fn index_len(world: &mut TestWorld, families: &IndustryFamilies, deposit: OreDepositId) -> usize {
    read(world, families, |txn| {
        txn.get(
            &families.claim_index,
            &mush_industry::claim_subject_token(ClaimSubject::Deposit { deposit }),
        )
        .expect("index reads")
        .map_or(0, mush_industry::ClaimIndex::len)
    })
}

fn revs(
    world: &mut TestWorld,
    families: &IndustryFamilies,
    deposits: &[OreDepositId],
) -> Vec<ClaimRev> {
    deposits
        .iter()
        .flat_map(|deposit| claims_over(world, families, *deposit))
        .map(|claim| ClaimRev {
            claim: claim.id(),
            revision: claim.revision(),
        })
        .collect()
}

fn action(world: &TestWorld) -> mush_core::DefinitionRef {
    mush_industry::action_ref(world.registry(), IndustryVerb::ClaimTenure)
        .expect("the action is registered")
}

/// Whether the regenerated projection offers `claim-tenure` to `actor`.
fn offered(world: &TestWorld, actor: ObjId) -> bool {
    let query = world
        .engine()
        .legal_action_query(
            mush_core::PrincipalId::Object { id: actor },
            actor,
            PerceivedRef::SelfActor,
        )
        .expect("self-actor query");
    world
        .engine()
        .project_legal_actions(&query)
        .expect("projection")
        .projected()
        .into_iter()
        .any(|offer| offer.presentation().verb == "claim-tenure")
}

/// Drive one submission by `actor` with `receipt` through the offer route;
/// `Ok` installs it and returns the outcome's data.
fn submit(
    world: &mut TestWorld,
    actor: ObjId,
    receipt: &str,
    token: u8,
) -> Result<serde_json::Value, String> {
    let wanted = action(world);
    let (action, resolved) = {
        let engine = world.engine();
        let query = engine
            .legal_action_query(
                mush_core::PrincipalId::Object { id: actor },
                actor,
                PerceivedRef::SelfActor,
            )
            .map_err(|error| error.to_string())?;
        let offers = engine
            .project_legal_actions(&query)
            .map_err(|error| error.to_string())?;
        let projected = offers
            .projected_for_action(&wanted)
            .ok_or_else(|| "that action is not currently offered".to_owned())?;
        let submission = mush_core::OfferSubmission {
            offer: projected.offer(),
            subject: projected.subject().clone(),
            targets: Vec::new(),
            parameters: serde_json::json!({ "receipt": receipt }),
        };
        let execution = mush_core::OfferExecutionContext {
            request: RequestUid::from_bytes([token; 16]),
            principal: mush_core::PrincipalId::Object { id: actor },
            actor,
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal: mush_core::PrincipalId::Object { id: actor },
                    owner: wanted.key().owner.clone(),
                    namespace: mush_core::CanonicalId::new(wanted.key().id.as_str())
                        .map_err(|_| "namespace".to_owned())?,
                },
                1,
                IdempotencyKey::Request(RequestUid::from_bytes([token; 16])),
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
    let (prepared, output) = world
        .engine_mut()
        .prepare_registered_resolved_authoritative_action(
            &action,
            &resolved,
            CommitToken::from_bytes([token; 16]),
        )
        .map_err(|error| format!("{error}"))?;
    let data = output.outcome().data().clone();
    world
        .engine_mut()
        .install_prepared_action(prepared.into_parts().1)
        .map_err(|error| format!("{error:?}"))?;
    Ok(data)
}

/// A hand-built invocation that never met the offer.
fn hand(world: &mut TestWorld, actor: ObjId, parameters: serde_json::Value) -> String {
    let action = action(world);
    let invocation = mush_core::ActionInvocation::new(mush_core::ActionInvocationDraft {
        request: RequestUid::from_bytes([0x99; 16]),
        principal: mush_core::PrincipalId::Object { id: actor },
        actor,
        offered_action: mush_core::ActionOfferId(mush_core::CanonicalHash::from_bytes([0x99; 32])),
        subject: PerceivedRef::SelfActor,
        targets: Vec::new(),
        parameters,
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal: mush_core::PrincipalId::Object { id: actor },
                owner: OwnerId::new("industry").expect("owner"),
                namespace: mush_core::CanonicalId::new(action.key().id.as_str())
                    .expect("namespace"),
            },
            1,
            IdempotencyKey::Request(RequestUid::from_bytes([0x99; 16])),
        ),
    })
    .expect("an invocation");
    match world.engine_mut().prepare_registered_authoritative_action(
        &action,
        &invocation,
        CommitToken::from_bytes([0x99; 16]),
    ) {
        Ok(_) => "accepted".to_owned(),
        Err(mush_core::ActionProviderPrepareError::Provider { source, .. }) => {
            format!("{:?}: {}", source.code(), source.message())
        }
        Err(other) => format!("{other}"),
    }
}

fn act(receipt: &str, actor: ObjId, place: ObjId, kind: TenureKind) -> TenureAct {
    TenureAct {
        receipt: SmolStr::new(receipt),
        actor,
        place,
        kind,
        summary: "Tenure recorded.".to_owned(),
        game: serde_json::json!({ "site": "test" }),
    }
}

fn stake(
    world: &TestWorld,
    holder: ObjId,
    reopen: bool,
    file: Vec<OreDepositId>,
    take_over: Vec<ClaimRev>,
) -> TenureKind {
    TenureKind::Stake {
        issuer: org(7),
        holder: obj(holder),
        term_at: at(world, 10_000).at,
        discovered_at: at(world, -5).at,
        reopen,
        file,
        take_over,
    }
}

fn lapse(world: &TestWorld, holder: ObjId, claims: Vec<ClaimRev>) -> TenureKind {
    TenureKind::Lapse {
        holder: obj(holder),
        claims,
        at: now(world).at,
        cooling_until: at(world, 1_000).at,
    }
}

// ---- the offer ----

#[test]
fn sm10_the_tenure_verb_offers_only_what_the_authority_vouches_for() {
    let stub = Arc::new(Stub::default());
    let (mut world, families) = boot(&stub);
    let pilot = world.spawn_player("pilot");
    let (place, deposits) = block(&mut world, &families, "block", 0xA100, 2);

    // No row: no offer, staff included.
    assert!(!offered(&world, pilot), "no row, no offer");
    assert!(!offered(&world, GOD), "staff are offered nothing either");

    // A row for another actor is not this actor's offer.
    let kind = stake(&world, pilot, false, deposits.clone(), Vec::new());
    stub.vouch(act("t/a/stake", GOD, place, kind.clone()));
    assert!(!offered(&world, pilot));
    stub.clear();
    // Even an authority that hands that row to the pilot (a leak): the provider's own
    // `act.actor == actor` check refuses it, and the same row naming the pilot is offered.
    stub.leak(act("t/a/stake", GOD, place, kind.clone()));
    assert!(
        !offered(&world, pilot),
        "a row naming another actor is never this actor's offer"
    );
    stub.leak(act("t/a/stake", pilot, place, kind.clone()));
    assert!(
        offered(&world, pilot),
        "the leaked path does reach the offer"
    );
    stub.clear();

    // A row naming a place that is no live object offers nothing.
    stub.vouch(act("t/a/stake", pilot, ObjId(4_000_000), kind.clone()));
    assert!(!offered(&world, pilot), "a dead place is never offered");

    stub.vouch(act("t/a/stake", pilot, place, kind));
    assert!(offered(&world, pilot), "the vouched row is offered");

    // A forged receipt is refused by the offer's own `Choice`.
    let forged = submit(&mut world, pilot, "t/b/stake", 1);
    assert!(forged.is_err(), "a receipt not offered refuses: {forged:?}");
    assert!(claims_over(&mut world, &families, deposits[0]).is_empty());

    // A hand-built invocation without the fixed row refuses structurally.
    let stamp_now = now(&world);
    let hand_built = hand(
        &mut world,
        pilot,
        serde_json::json!({ "receipt": "t/a/stake", "now": stamp_now }),
    );
    assert_eq!(hand_built, "Unauthorized: You cannot work that here.");
    assert!(claims_over(&mut world, &families, deposits[0]).is_empty());
}

#[test]
fn sm10_a_stake_files_every_deposit_of_the_block_in_one_bundle() {
    let stub = Arc::new(Stub::default());
    let (mut world, families) = boot(&stub);
    let pilot = world.spawn_player("pilot");
    let (place, deposits) = block(&mut world, &families, "block", 0xA200, 4);
    let kind = stake(&world, pilot, false, deposits.clone(), Vec::new());
    stub.vouch(act("t/stake", pilot, place, kind));
    submit(&mut world, pilot, "t/stake", 2).expect("the stake lands");

    let mut ids = Vec::new();
    for deposit in &deposits {
        let claims = claims_over(&mut world, &families, *deposit);
        assert_eq!(claims.len(), 1, "one claim a deposit");
        let claim = &claims[0];
        assert_eq!(claim.holder(), &obj(pilot));
        assert_eq!(claim.issuer(), &org(7));
        assert_eq!(
            claim.object(),
            place,
            "the claim's object is the block place"
        );
        assert_eq!(claim.term(), &at(&world, 10_000), "one term");
        assert_eq!(
            claim.discovered_at(),
            Some(&at(&world, -5)),
            "the discovery stamp"
        );
        assert_eq!(claim.epoch(), 0, "epoch 0: since the block opened");
        assert_eq!(claim.contest_state(), &ContestState::Uncontested);
        assert_eq!(
            claim.transferability(),
            mush_industry::Transferability::Bound
        );
        assert_eq!(index_len(&mut world, &families, *deposit), 1);
        ids.push(claim.id());
    }
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 4, "four distinct claim ids");
    assert!(
        !tenure_stored(&mut world, &families, place),
        "a first filing writes no tenure"
    );
    assert_eq!(
        stub.last(),
        TenureOutcome {
            epoch_before: 0,
            since_before: None,
            epoch_after: 0
        }
    );

    // A second stake on the held block refuses with the public line.
    let other = world.spawn_player("other");
    let kind = stake(&world, other, false, deposits.clone(), Vec::new());
    stub.vouch(act("t/stake2", other, place, kind));
    let refused = submit(&mut world, other, "t/stake2", 3).expect_err("the block is held");
    assert!(refused.contains("That block is under claim."), "{refused}");
}

#[test]
fn sm10_a_stake_over_a_lapsed_block_takes_it_over() {
    let stub = Arc::new(Stub::default());
    let (mut world, families) = boot(&stub);
    let a = world.spawn_player("a");
    let b = world.spawn_player("b");
    let (place, deposits) = block(&mut world, &families, "block", 0xA300, 3);
    stub.vouch(act(
        "t/stake",
        a,
        place,
        stake(&world, a, false, deposits.clone(), Vec::new()),
    ));
    submit(&mut world, a, "t/stake", 4).expect("a stakes");
    let before: Vec<ClaimId> = revs(&mut world, &families, &deposits)
        .iter()
        .map(|rev| rev.claim)
        .collect();
    let held = revs(&mut world, &families, &deposits);
    stub.vouch(act("t/lapse", a, place, lapse(&world, a, held)));
    submit(&mut world, a, "t/lapse", 5).expect("the lapse lands");
    assert_eq!(tenure_of(&mut world, &families, place).epoch(), 1);

    // The lapsed holder is refused by `tenure_hold` while cooling.
    let lapsed = revs(&mut world, &families, &deposits);
    stub.vouch(act(
        "t/again",
        a,
        place,
        stake(&world, a, true, Vec::new(), lapsed.clone()),
    ));
    let cooling = submit(&mut world, a, "t/again", 6).expect_err("a cools");
    assert!(cooling.contains("cooling term"), "{cooling}");
    assert_eq!(tenure_of(&mut world, &families, place).epoch(), 1);

    // A stranger takes the records over: no new id, the epoch once.
    stub.vouch(act(
        "t/take",
        b,
        place,
        stake(&world, b, true, Vec::new(), lapsed),
    ));
    submit(&mut world, b, "t/take", 7).expect("b takes the block over");
    let after: Vec<ClaimId> = revs(&mut world, &families, &deposits)
        .iter()
        .map(|rev| rev.claim)
        .collect();
    assert_eq!(before, after, "the records are taken over, never re-filed");
    for deposit in &deposits {
        assert_eq!(index_len(&mut world, &families, *deposit), 1);
        let claim = &claims_over(&mut world, &families, *deposit)[0];
        assert_eq!(claim.holder(), &obj(b));
        assert_eq!(claim.epoch(), 2);
        assert!(!claim.is_lapsed());
    }
    let tenure = tenure_of(&mut world, &families, place);
    assert_eq!(tenure.epoch(), 2, "the take-over advanced the epoch once");
    assert_eq!(tenure.mark(), Some(&TenureMark::Held { holder: obj(b) }));
    assert_eq!(stub.last().epoch_before, 1);
    assert_eq!(stub.last().since_before, Some(now(&world).at));
    assert_eq!(stub.last().epoch_after, 2);
}

#[test]
fn sm10_an_empty_stake_and_lapse_still_move_the_place() {
    let stub = Arc::new(Stub::default());
    let (mut world, families) = boot(&stub);
    let a = world.spawn_player("a");
    let b = world.spawn_player("b");
    let place = place(&mut world, "barren block");

    // A first stake with no deposit writes no industry record: with this stub
    // (which stages nothing of its own) the bundle is empty and the engine
    // refuses to persist it; in production the authority's own block record
    // is the bundle's write (tacsim's `tests/claims.rs` C23).
    stub.vouch(act(
        "t/s1",
        a,
        place,
        stake(&world, a, false, Vec::new(), Vec::new()),
    ));
    let empty = submit(&mut world, a, "t/s1", 8).expect_err("an empty bundle");
    assert!(empty.contains("empty logical action"), "{empty}");
    assert!(!tenure_stored(&mut world, &families, place));

    // Its lapse moves the place: the epoch and a's cooling row.
    stub.vouch(act("t/l1", a, place, lapse(&world, a, Vec::new())));
    submit(&mut world, a, "t/l1", 9).expect("the empty lapse lands");
    let tenure = tenure_of(&mut world, &families, place);
    assert_eq!(tenure.epoch(), 1, "a block with no claim still lapses");
    assert_eq!(tenure.mark(), Some(&TenureMark::Lapsed));
    assert_eq!(tenure.cooling().len(), 1);
    assert_eq!(tenure.cooling()[0].holder, obj(a));
    assert_eq!(tenure.cooling()[0].until, at(&world, 1_000));

    // A cooling holder's empty re-stake refuses; a stranger's holds the place.
    stub.vouch(act(
        "t/s2",
        a,
        place,
        stake(&world, a, true, Vec::new(), Vec::new()),
    ));
    assert!(submit(&mut world, a, "t/s2", 10).is_err(), "a cools");
    stub.vouch(act(
        "t/s3",
        b,
        place,
        stake(&world, b, true, Vec::new(), Vec::new()),
    ));
    submit(&mut world, b, "t/s3", 11).expect("b re-opens the empty block");
    let tenure = tenure_of(&mut world, &families, place);
    assert_eq!(tenure.epoch(), 2);
    assert_eq!(tenure.mark(), Some(&TenureMark::Held { holder: obj(b) }));
}

#[test]
fn sm10_a_block_lapse_then_its_claims_advance_the_epoch_once() {
    let stub = Arc::new(Stub::default());
    let (mut world, families) = boot(&stub);
    let a = world.spawn_player("a");
    let (place, deposits) = block(&mut world, &families, "block", 0xA400, 4);
    stub.vouch(act(
        "t/stake",
        a,
        place,
        stake(&world, a, false, deposits.clone(), Vec::new()),
    ));
    submit(&mut world, a, "t/stake", 12).expect("a stakes");
    let held = revs(&mut world, &families, &deposits);
    // Direct: the block-level seam first, then each claim's own lapse, in ONE
    // bundle — the claims write no tenure (a second mutation would refuse).
    let (when, until) = (now(&world), at(&world, 1_000));
    apply(&mut world, &families, |txn| {
        families.tenure_lapse(txn, place, &obj(a), &when, &until)?;
        for rev in &held {
            families.lapse_claim(txn, rev.claim, rev.revision, &when, &until)?;
        }
        Ok(())
    })
    .expect("one lapse bundle");
    let tenure = tenure_of(&mut world, &families, place);
    assert_eq!(tenure.epoch(), 1, "advanced once");
    assert_eq!(tenure.cooling().len(), 1);
    for rev in &held {
        assert!(claim_of(&mut world, &families, rev.claim).is_lapsed());
    }
}

#[test]
fn sm10_a_block_hold_refuses_a_cooling_holder() {
    let stub = Arc::new(Stub::default());
    let (mut world, families) = boot(&stub);
    let place = place(&mut world, "block");
    let (when, until) = (now(&world), at(&world, 1_000));
    apply(&mut world, &families, |txn| {
        families
            .tenure_lapse(txn, place, &obj(ObjId(5)), &when, &until)
            .map(|_| ())
    })
    .expect("the lapse");
    let refused = apply(&mut world, &families, |txn| {
        families
            .tenure_hold(txn, place, &obj(ObjId(5)), &when)
            .map(|_| ())
    });
    assert!(matches!(refused, Err(IndustryError::Cooling { .. })));
    // At the cooling term's end the hold is accepted.
    apply(&mut world, &families, |txn| {
        families
            .tenure_hold(txn, place, &obj(ObjId(5)), &until)
            .map(|_| ())
    })
    .expect("the hold after cooling");
    assert_eq!(tenure_of(&mut world, &families, place).epoch(), 2);
}

#[test]
fn sm10_a_block_with_no_claims_still_lapses_and_cools() {
    let stub = Arc::new(Stub::default());
    let (mut world, families) = boot(&stub);
    let place = place(&mut world, "block");
    let (when, until) = (now(&world), at(&world, 1_000));
    let after = apply(&mut world, &families, |txn| {
        families.tenure_lapse(txn, place, &obj(ObjId(6)), &when, &until)
    })
    .expect("the lapse");
    assert_eq!(after.epoch(), 1);
    assert!(tenure_stored(&mut world, &families, place));
    let cooling = read(&mut world, &families, |txn| {
        families
            .cooling_until(txn, place, &obj(ObjId(6)), &when)
            .expect("reads")
    });
    assert_eq!(cooling, Some(until.clone()));
    // A second lapse of the same holder in the same epoch changes nothing.
    let unchanged = apply(&mut world, &families, |txn| {
        families.tenure_lapse(txn, place, &obj(ObjId(6)), &when, &until)
    });
    assert!(unchanged.is_ok());
    assert_eq!(tenure_of(&mut world, &families, place).epoch(), 1);
}

#[test]
fn sm10_a_row_naming_another_places_deposit_refuses() {
    let stub = Arc::new(Stub::default());
    let (mut world, families) = boot(&stub);
    let a = world.spawn_player("a");
    let (place, _) = block(&mut world, &families, "here", 0xA500, 1);
    let (_, elsewhere) = block(&mut world, &families, "there", 0xA600, 1);
    stub.vouch(act(
        "t/stake",
        a,
        place,
        stake(&world, a, false, elsewhere.clone(), Vec::new()),
    ));
    let refused = submit(&mut world, a, "t/stake", 13).expect_err("another place's deposit");
    assert!(refused.contains("You cannot work that here."), "{refused}");
    assert!(claims_over(&mut world, &families, elsewhere[0]).is_empty());
}

#[test]
fn sm10_renew_suspends_paper_only_under_its_jurisdiction() {
    let stub = Arc::new(Stub::default());
    let (mut world, families) = boot(&stub);
    let a = world.spawn_player("a");
    let (place, deposits) = block(&mut world, &families, "papered", 0xA700, 2);
    // The Syndicate's paper (issuer org 9), seeded Contested.
    let clerk = world.spawn_player("clerk");
    stub.vouch(act(
        "t/seed",
        clerk,
        place,
        TenureKind::Seed {
            issuer: org(9),
            holder: org(9),
            term_at: at(&world, 1_000_000).at,
            deposits: deposits.clone(),
            contest: Some(ContractId(42)),
        },
    ));
    submit(&mut world, clerk, "t/seed", 14).expect("the paper is seeded");
    let paper = revs(&mut world, &families, &deposits);
    for rev in &paper {
        assert_eq!(
            claim_of(&mut world, &families, rev.claim).contest_state(),
            &ContestState::Contested {
                contract: ContractId(42)
            }
        );
    }
    // The pilot's own claim under the site's issuer (org 7).
    stub.vouch(act(
        "t/stake",
        a,
        place,
        stake(&world, a, false, deposits.clone(), Vec::new()),
    ));
    submit(&mut world, a, "t/stake", 15).expect("a stakes the papered block");
    let mine: Vec<ClaimRev> = revs(&mut world, &families, &deposits)
        .into_iter()
        .filter(|rev| !paper.iter().any(|p| p.claim == rev.claim))
        .collect();
    assert_eq!(mine.len(), 2);

    // A suspension under a jurisdiction that is not the deposits' refuses whole.
    stub.vouch(act(
        "t/renew9",
        a,
        place,
        TenureKind::Renew {
            claims: mine.clone(),
            term_at: at(&world, 20_000).at,
            suspend: paper.clone(),
            by: Some(org(9)),
        },
    ));
    assert!(submit(&mut world, a, "t/renew9", 16).is_err());
    for rev in &mine {
        assert_eq!(
            claim_of(&mut world, &families, rev.claim).term(),
            &at(&world, 10_000),
            "nothing renewed"
        );
    }

    stub.vouch(act(
        "t/renew",
        a,
        place,
        TenureKind::Renew {
            claims: mine.clone(),
            term_at: at(&world, 20_000).at,
            suspend: paper.clone(),
            by: Some(org(7)),
        },
    ));
    submit(&mut world, a, "t/renew", 17).expect("the renewal lands");
    for rev in &mine {
        assert_eq!(
            claim_of(&mut world, &families, rev.claim).term(),
            &at(&world, 20_000)
        );
    }
    for rev in &paper {
        assert_eq!(
            claim_of(&mut world, &families, rev.claim).contest_state(),
            &ContestState::Suspended { by: org(7) }
        );
    }
    assert!(
        !tenure_stored(&mut world, &families, place),
        "a renewal moves no tenure"
    );
}

#[test]
fn sm10_a_lapse_row_advances_the_epoch_once_and_cools_the_holder() {
    let stub = Arc::new(Stub::default());
    let (mut world, families) = boot(&stub);
    let a = world.spawn_player("a");
    let b = world.spawn_player("b");
    let (place, deposits) = block(&mut world, &families, "block", 0xA800, 4);
    stub.vouch(act(
        "t/stake",
        a,
        place,
        stake(&world, a, false, deposits.clone(), Vec::new()),
    ));
    submit(&mut world, a, "t/stake", 18).expect("a stakes");
    let held = revs(&mut world, &families, &deposits);

    // A lapse naming the wrong holder refuses.
    stub.vouch(act("t/wrong", b, place, lapse(&world, b, held.clone())));
    assert!(submit(&mut world, b, "t/wrong", 19).is_err());
    assert!(!tenure_stored(&mut world, &families, place));

    stub.vouch(act("t/lapse", a, place, lapse(&world, a, held.clone())));
    submit(&mut world, a, "t/lapse", 20).expect("the lapse lands");
    let tenure = tenure_of(&mut world, &families, place);
    assert_eq!(tenure.epoch(), 1);
    assert_eq!(tenure.mark(), Some(&TenureMark::Lapsed));
    assert_eq!(tenure.since(), Some(&now(&world)));
    assert_eq!(tenure.cooling().len(), 1);
    for rev in &held {
        let claim = claim_of(&mut world, &families, rev.claim);
        assert_eq!(claim.lapsed_at(), Some(&now(&world)));
    }
}

#[test]
fn sm10_a_contest_row_transfers_the_block_once() {
    let stub = Arc::new(Stub::default());
    let (mut world, families) = boot(&stub);
    let a = world.spawn_player("a");
    let b = world.spawn_player("b");
    let (place, deposits) = block(&mut world, &families, "block", 0xA900, 4);
    stub.vouch(act(
        "t/stake",
        a,
        place,
        stake(&world, a, false, deposits.clone(), Vec::new()),
    ));
    submit(&mut world, a, "t/stake", 21).expect("a stakes");
    let held = revs(&mut world, &families, &deposits);

    // A discovery after now refuses.
    stub.vouch(act(
        "t/late",
        b,
        place,
        TenureKind::Contest {
            claims: held.clone(),
            to: obj(b),
            discovered_at: at(&world, 60).at,
            term_at: at(&world, 50_000).at,
        },
    ));
    let late = submit(&mut world, b, "t/late", 22).expect_err("a later stamp");
    assert!(
        late.contains("a discovery cannot postdate the seizure"),
        "{late}"
    );

    stub.vouch(act(
        "t/contest",
        b,
        place,
        TenureKind::Contest {
            claims: held.clone(),
            to: obj(b),
            discovered_at: at(&world, -60).at,
            term_at: at(&world, 50_000).at,
        },
    ));
    submit(&mut world, b, "t/contest", 23).expect("the contest lands");
    let tenure = tenure_of(&mut world, &families, place);
    assert_eq!(tenure.epoch(), 1, "one block event");
    assert_eq!(tenure.mark(), Some(&TenureMark::Held { holder: obj(b) }));
    for rev in &held {
        let claim = claim_of(&mut world, &families, rev.claim);
        assert_eq!(claim.holder(), &obj(b));
        assert_eq!(claim.discovered_at(), Some(&at(&world, -60)));
        assert_eq!(claim.epoch(), 1);
        assert_eq!(
            claim.term(),
            &at(&world, 50_000),
            "the challenger's paid term"
        );
        assert_eq!(
            claim.revision(),
            rev.revision + 1,
            "a contest moves the revision by one (conformance F7)"
        );
    }
}

/// **A transfer with its term is one post-image, one revision** (the judge's conformance F7 on
/// SM.10): `transfer_tenure_termed` moves a claim's revision by exactly one, as
/// `transfer_tenure`, `renew_claim` and `lapse_claim` each do — never two (a transfer then a
/// second `with_term` bump) — and still carries the new holder, stamp, epoch and term.
#[test]
fn sm10_a_termed_transfer_moves_the_revision_once() {
    let stub = Arc::new(Stub::default());
    let (mut world, families) = boot(&stub);
    let a = world.spawn_player("a");
    let b = world.spawn_player("b");
    let c = world.spawn_player("c");
    let (place, deposits) = block(&mut world, &families, "block", 0xAA00, 2);
    stub.vouch(act(
        "t/stake",
        a,
        place,
        stake(&world, a, false, deposits.clone(), Vec::new()),
    ));
    submit(&mut world, a, "t/stake", 31).expect("a stakes");
    let held = revs(&mut world, &families, &deposits);
    let (Some(first), Some(second)) = (held.first().copied(), held.get(1).copied()) else {
        panic!("two claims over the block: {held:?}");
    };
    let (when, found, term) = (now(&world), at(&world, -60), at(&world, 90_000));
    // The termed transfer: one bump.
    let moved = apply(&mut world, &families, |txn| {
        families.transfer_tenure_termed(
            txn,
            first.claim,
            first.revision,
            obj(b),
            Some(found.clone()),
            term.clone(),
            &when,
        )
    })
    .expect("the termed transfer");
    assert_eq!(moved.revision(), first.revision + 1, "one revision");
    let stored = claim_of(&mut world, &families, first.claim);
    assert_eq!(stored.revision(), first.revision + 1);
    assert_eq!(stored.holder(), &obj(b));
    assert_eq!(stored.discovered_at(), Some(&found));
    assert_eq!(stored.term(), &term);
    assert_eq!(stored.epoch(), 1);
    // The untermed transfer, beside it: one bump too, the term kept.
    let kept = claim_of(&mut world, &families, second.claim).term().clone();
    let plain = apply(&mut world, &families, |txn| {
        families.transfer_tenure(txn, second.claim, second.revision, obj(c), None, &when)
    })
    .expect("the plain transfer");
    assert_eq!(plain.revision(), second.revision + 1, "one revision");
    assert_eq!(plain.term(), &kept, "the term kept");
    // A replay at the old revision refuses.
    let stale = apply(&mut world, &families, |txn| {
        families.transfer_tenure_termed(
            txn,
            first.claim,
            first.revision,
            obj(c),
            None,
            term.clone(),
            &when,
        )
    });
    assert!(
        matches!(stale, Err(IndustryError::StaleRevision)),
        "{stale:?}"
    );
}

/// **A lapse cooling to its own instant cools nobody** (the lead's ruling on SM.10's R1/OC16: a
/// claim whose fee expired unaccepted lapses with no cooling term): `cooling_until == at` writes
/// **no** cooling row — never one that has run out, which another clock generation would read as
/// live — so the holder holds the block again at once, while the epoch still advances for the
/// block event; a holder's earlier live row is kept, never shortened.
#[test]
fn sm10_a_lapse_cooling_to_its_own_instant_cools_nobody() {
    let stub = Arc::new(Stub::default());
    let (mut world, families) = boot(&stub);
    let place = place(&mut world, "block");
    let when = now(&world);
    let after = apply(&mut world, &families, |txn| {
        families.tenure_lapse(txn, place, &obj(ObjId(8)), &when, &when)
    })
    .expect("the lapse");
    assert_eq!(after.epoch(), 1, "the block event still opens an epoch");
    assert!(after.cooling().is_empty(), "no row: {after:?}");
    let cooling = read(&mut world, &families, |txn| {
        families
            .cooling_until(txn, place, &obj(ObjId(8)), &when)
            .expect("reads")
    });
    assert_eq!(cooling, None, "no live cooling term");
    apply(&mut world, &families, |txn| {
        families
            .tenure_hold(txn, place, &obj(ObjId(8)), &when)
            .map(|_| ())
    })
    .expect("the holder holds it again at once");
    assert_eq!(tenure_of(&mut world, &families, place).epoch(), 2);
    // A live row of the holder's own stands through a later no-cooling lapse.
    let until = at(&world, 1_000);
    apply(&mut world, &families, |txn| {
        families.tenure_lapse(txn, place, &obj(ObjId(8)), &when, &until)
    })
    .expect("a cooling lapse");
    apply(&mut world, &families, |txn| {
        families.tenure_hold(txn, place, &obj(ObjId(9)), &when)
    })
    .expect("another holds it");
    let kept = apply(&mut world, &families, |txn| {
        families.tenure_lapse(txn, place, &obj(ObjId(8)), &when, &when)
    })
    .expect("a no-cooling lapse");
    assert_eq!(kept.cooling().len(), 1);
    assert_eq!(
        kept.cooling_for(&obj(ObjId(8)), &when),
        Some(&until),
        "kept, never shortened"
    );
}

#[test]
fn sm10_seeding_twice_refuses() {
    let stub = Arc::new(Stub::default());
    let (mut world, families) = boot(&stub);
    let clerk = world.spawn_player("clerk");
    let (place, deposits) = block(&mut world, &families, "held", 0xAA00, 2);
    let seed = TenureKind::Seed {
        issuer: org(7),
        holder: org(11),
        term_at: at(&world, 1_000_000).at,
        deposits: deposits.clone(),
        contest: None,
    };
    stub.vouch(act("t/seed1", clerk, place, seed.clone()));
    submit(&mut world, clerk, "t/seed1", 24).expect("the seed lands");
    for deposit in &deposits {
        let claims = claims_over(&mut world, &families, *deposit);
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].holder(), &org(11));
        assert_eq!(claims[0].contest_state(), &ContestState::Uncontested);
    }
    stub.vouch(act("t/seed2", clerk, place, seed));
    let twice = submit(&mut world, clerk, "t/seed2", 25).expect_err("seeded already");
    assert!(twice.contains("that block is already seeded"), "{twice}");
}

#[test]
fn sm10_a_stage_error_writes_nothing() {
    let stub = Arc::new(Stub::default());
    let (mut world, families) = boot(&stub);
    let a = world.spawn_player("a");
    let (place, deposits) = block(&mut world, &families, "block", 0xAB00, 2);
    *stub.fail.lock().expect("fail") = Some("the most one account may".to_owned());
    stub.vouch(act(
        "t/stake",
        a,
        place,
        stake(&world, a, false, deposits.clone(), Vec::new()),
    ));
    let refused = submit(&mut world, a, "t/stake", 26).expect_err("the stage refuses");
    assert!(refused.contains("the most one account may"), "{refused}");
    for deposit in &deposits {
        assert!(claims_over(&mut world, &families, *deposit).is_empty());
        assert_eq!(index_len(&mut world, &families, *deposit), 0);
    }
    // A lapse refused at its stage leaves the tenure unwritten too.
    stub.vouch(act("t/lapse", a, place, lapse(&world, a, Vec::new())));
    assert!(submit(&mut world, a, "t/lapse", 27).is_err());
    assert!(!tenure_stored(&mut world, &families, place));
}

#[test]
fn sm10_the_outcome_names_no_count() {
    let stub = Arc::new(Stub::default());
    let (mut world, families) = boot(&stub);
    let a = world.spawn_player("a");
    let mut outcomes = Vec::new();
    for (i, n) in [1_usize, 2, 4].into_iter().enumerate() {
        let (place, deposits) = block(&mut world, &families, "block", 0xAC00 + 0x10 * i as u64, n);
        stub.vouch(act(
            "t/stake",
            a,
            place,
            stake(&world, a, false, deposits, Vec::new()),
        ));
        let token = 28 + u8::try_from(i).expect("small");
        let mut data = submit(&mut world, a, "t/stake", token).expect("the stake lands");
        let map = data.as_object_mut().expect("an object");
        let keys: Vec<&str> = map.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            vec!["actor", "place", "receipt", "summary", "verb"],
            "the outcome never carries a count, a claim list, a stamp or a reserve"
        );
        map.remove("place");
        outcomes.push(data);
    }
    assert_eq!(outcomes[0], outcomes[1]);
    assert_eq!(
        outcomes[1], outcomes[2],
        "the same outcome whatever the block holds"
    );
}
