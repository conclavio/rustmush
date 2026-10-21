//! Doc 20 cut SM.6 — the industry seams, by direct call (the e7c3 style; lead
//! ruling 6): the system opener and its receipt-derived id, the external-draws
//! sweep invariant, `@extract` refusing an externally worked body, `ClaimRenew`
//! in place, the lapse / take-over / transfer block-event contract on the
//! per-place tenure record, the cooling term, the seize-transfers-tenure fix and
//! the contest offer's jurisdiction row.
//!
//! The rig is `e7c3.rs`'s: `boot*()` composes economy, inventory and industry
//! in production order, `apply()` opens the owner-stamped transaction a provider
//! gets and **installs** it exactly as a receipt does, so every "writes nothing"
//! assertion means what it says; `offer()` drives one submission the way the
//! world task does (regenerate the actor's own offer, resolve, prepare, install).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

use mush_core::{
    ClockRef, ClockStamp, CommitToken, Engine, GOD, IdempotencyKey, IdempotencyScope, ModuleCtx,
    ObjId, OfferKeyRing, OfferSecret, OwnerId, PerceivedRef, RequestUid, ResolutionKeyRing,
    ResolutionSecret, ScopedIdempotencyKey,
};
use mush_economy::{InstrumentId, VaultId};
use mush_industry::{
    Accessibility, BandId, ClaimId, ClaimRecord, ClaimSubject, ContestState, CurvePoint,
    Exclusivity, ExternalDraws, IndustryError, IndustryFamilies, IndustryHandles, IndustryModule,
    IndustryStore, IndustryVerb, JurisdictionAuthority, MAX_COOLING_ROWS, OreDeposit, OreDepositId,
    Quarantine, SystemOpener, SystemOpening, TenureMark, TenureRecord, Transferability, YieldCurve,
};
use mush_inventory::{EndpointRef, Grade, InventoryHandleCell, InventoryModule};
use mush_org::PrincipalId;
use mush_testkit::TestWorld;
use smol_str::SmolStr;

// ---- the rig ----

/// The composed seams one test world carries.
#[derive(Default)]
struct Seams {
    opener: Option<Arc<dyn SystemOpener>>,
    draws: Option<Arc<dyn ExternalDraws>>,
    jurisdiction: Option<Arc<dyn JurisdictionAuthority>>,
}

const LICENCE: &str = "test.licence";

fn licence_id() -> InstrumentId {
    InstrumentId::new(LICENCE).expect("licence id")
}

fn boot(seams: Seams) -> (TestWorld, IndustryFamilies) {
    let cell: InventoryHandleCell = Arc::default();
    let mut industry = IndustryModule::new()
        .with_inventory(Arc::clone(&cell))
        .with_extraction_licence(licence_id());
    if let Some(opener) = seams.opener {
        industry = industry.with_system_opener(opener);
    }
    if let Some(draws) = seams.draws {
        industry = industry.with_external_draws(draws);
    }
    if let Some(jurisdiction) = seams.jurisdiction {
        industry = industry.with_jurisdiction_authority(jurisdiction);
    }
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

fn stamp(at: i64) -> ClockStamp {
    ClockStamp {
        clock: ClockRef::WorldIc { generation: 1 },
        at,
    }
}

fn commodity(id: &str) -> mush_core::CanonicalId {
    mush_core::CanonicalId::new(id).expect("test commodity id")
}

fn one_point() -> YieldCurve {
    YieldCurve::new(vec![CurvePoint {
        cumulative: 0,
        per_cycle: 1,
    }])
    .expect("curve")
}

/// A surface-shaped body: accessibility 100, the one-point curve, both bands
/// `surface`, worked by `tacsim.surface`.
fn body(id: u64, place: ObjId, good: &str, vault: u64, reserve: u64) -> OreDeposit {
    OreDeposit::new(
        OreDepositId(id),
        place,
        commodity(good),
        Grade::new(0).expect("grade"),
        Accessibility::new(100).expect("accessibility"),
        BandId::new("surface").expect("band"),
        BandId::new("surface").expect("band"),
        one_point(),
        EndpointRef(VaultId(vault)),
        reserve,
    )
    .expect("test body")
}

fn external(body: OreDeposit) -> OreDeposit {
    body.with_external("tacsim.surface").expect("external")
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

fn read<T>(
    engine: &mut Engine,
    families: &IndustryFamilies,
    body: impl FnOnce(&mut mush_core::ActionTxn<'_>) -> T,
) -> T {
    let mut txn = engine
        .action_transaction(&families.deposits)
        .expect("industry transaction");
    body(&mut txn)
}

fn open(world: &mut TestWorld, families: &IndustryFamilies, deposit: &OreDeposit) {
    apply(world.engine_mut(), families, |txn| {
        families.open_deposit(txn, deposit.clone())
    })
    .expect("open the body");
}

fn deposit_of(
    world: &mut TestWorld,
    families: &IndustryFamilies,
    id: OreDepositId,
) -> Option<OreDeposit> {
    read(world.engine_mut(), families, |txn| {
        families.deposit(txn, id).expect("deposit reads")
    })
}

fn claim_of(world: &mut TestWorld, families: &IndustryFamilies, id: ClaimId) -> ClaimRecord {
    read(world.engine_mut(), families, |txn| {
        families
            .claim(txn, id)
            .expect("claim reads")
            .expect("claim")
    })
}

fn tenure_of(world: &mut TestWorld, families: &IndustryFamilies, place: ObjId) -> TenureRecord {
    read(world.engine_mut(), families, |txn| {
        families.tenure(txn, place).expect("tenure reads")
    })
}

fn tenure_stored(world: &mut TestWorld, families: &IndustryFamilies, place: ObjId) -> bool {
    read(world.engine_mut(), families, |txn| {
        txn.get(&families.tenure, &place)
            .expect("tenure family")
            .is_some()
    })
}

fn index_len(world: &mut TestWorld, families: &IndustryFamilies, subject: ClaimSubject) -> usize {
    read(world.engine_mut(), families, |txn| {
        txn.get(
            &families.claim_index,
            &mush_industry::claim_subject_token(subject),
        )
        .expect("index reads")
        .map_or(0, mush_industry::ClaimIndex::len)
    })
}

fn pool(world: &mut TestWorld, families: &IndustryFamilies, deposit: &OreDeposit) -> u64 {
    read(world.engine_mut(), families, |txn| {
        families.remaining(txn, deposit).expect("pool reads")
    })
}

/// A claim over `subject` issued by `issuer`, held by `holder`, exclusive,
/// termed at `term`.
fn claim(
    id: u64,
    subject: ClaimSubject,
    issuer: PrincipalId,
    holder: PrincipalId,
    term: i64,
) -> ClaimRecord {
    ClaimRecord::new(
        ClaimId(id),
        subject,
        issuer,
        holder,
        ObjId(9_000 + u32::try_from(id % 1_000).expect("small id")),
        Exclusivity::Exclusive,
        1,
        stamp(term),
        Transferability::Transferable,
    )
    .expect("test claim")
}

fn file(world: &mut TestWorld, families: &IndustryFamilies, claim: &ClaimRecord) {
    apply(world.engine_mut(), families, |txn| {
        families.file_claim(txn, claim.clone())
    })
    .expect("file the claim");
}

fn obj(id: ObjId) -> PrincipalId {
    PrincipalId::Object(id)
}

fn org(id: u64) -> PrincipalId {
    PrincipalId::Organization(mush_org::OrgId(id))
}

/// Register the entitlement instrument the rig's industry names and open one
/// Active licence account for `who`.
fn license(world: &mut TestWorld, who: ObjId) {
    let known = world
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .is_some_and(|store| store.snapshot().instrument(&licence_id()).is_some());
    if !known {
        let mut out = mush_core::Outbox::new();
        let engine = world.engine_mut();
        let mut ctx = mush_core::Ctx {
            w: &mut engine.world,
            actor: GOD,
            switches: mush_core::SwitchSet::default(),
            args: mush_core::ParsedArgs::default(),
            out: &mut out,
            reg: &engine.registry,
            module: Some(SmolStr::new("economy")),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };
        mush_economy::register_instrument(
            &mut ctx,
            mush_economy::InstrumentDef {
                id: licence_id(),
                ruleset: SmolStr::new_static("test"),
                code: SmolStr::new_static("TLIC"),
                kind: mush_economy::InstrumentKind::Entitlement,
                scale: 0,
                transferable: false,
                negative_policy: mush_economy::NegativePolicy::Forbid,
                status: mush_economy::InstrumentStatus::Active,
                display: None,
                revision: 1,
            },
        )
        .expect("the licence instrument registers");
    }
    world.run(GOD, &format!("@account/open #{}={LICENCE}", who.0));
    assert!(
        matches!(
            mush_industry::licence_accounts(world.world(), &licence_id(), &obj(who)),
            Some(mush_economy::AccountStatus::Active)
        ),
        "the licence account is open and active"
    );
}

/// Every vault in the world, by raw id.
fn vault_ids(world: &TestWorld) -> Vec<u64> {
    world
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .expect("economy store")
        .snapshot()
        .vaults()
        .map(|vault| vault.id.0)
        .collect()
}

/// A real container-backed vault, so the sweep's vault check passes.
fn make_vault(world: &mut TestWorld, name: &str) -> u64 {
    let object = world
        .world_mut()
        .create(mush_core::ObjKind::Thing, name, GOD, Some(mush_core::LIMBO))
        .expect("a backing object");
    let before = vault_ids(world);
    world.run(
        GOD,
        &format!(
            "@vault/create {name}=container:#{} owner=#{}",
            object.0, GOD.0
        ),
    );
    vault_ids(world)
        .into_iter()
        .find(|id| !before.contains(id))
        .expect("a fresh vault")
}

fn action(world: &TestWorld, verb: IndustryVerb) -> mush_core::DefinitionRef {
    mush_industry::action_ref(world.registry(), verb).expect("the action is registered")
}

/// Whether `project_offer`, regenerated through the engine as
/// `resolve_legal_action` does, publishes an offer with `verb` to `actor`.
fn projects_verb(world: &TestWorld, actor: ObjId, verb: &str) -> bool {
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
        .any(|offer| offer.presentation().verb == verb)
}

/// The parameters a resolved submission carries — the submitted ones merged
/// with the offer's own fixed parameters — which is how a test reads what the
/// projection server-authored.
fn resolved_parameters(
    world: &TestWorld,
    actor: ObjId,
    verb: IndustryVerb,
    parameters: serde_json::Value,
) -> serde_json::Value {
    resolve(world, actor, verb, parameters, 0xEE)
        .expect("the submission resolves")
        .1
        .invocation()
        .parameters()
        .clone()
}

/// Resolve one submission of `verb` by `actor` against the actor's own
/// regenerated offer.
fn resolve(
    world: &TestWorld,
    actor: ObjId,
    verb: IndustryVerb,
    parameters: serde_json::Value,
    token: u8,
) -> Result<
    (
        mush_core::DefinitionRef,
        mush_core::ResolvedActionInvocation,
    ),
    String,
> {
    let wanted = action(world, verb);
    {
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
            parameters,
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
        Ok((action, resolved))
    }
}

/// Drive one submission of `verb` by `actor` with `parameters` the way the
/// world task does: regenerate the actor's own offer, resolve, prepare, install.
fn offer(
    world: &mut TestWorld,
    actor: ObjId,
    verb: IndustryVerb,
    parameters: serde_json::Value,
    token: u8,
) -> Result<(), String> {
    let (action, resolved) = resolve(world, actor, verb, parameters, token)?;
    let (prepared, _) = world
        .engine_mut()
        .prepare_registered_resolved_authoritative_action(
            &action,
            &resolved,
            CommitToken::from_bytes([token; 16]),
        )
        .map_err(|error| format!("{error:?}"))?;
    world
        .engine_mut()
        .install_prepared_action(prepared.into_parts().1)
        .map_err(|error| format!("{error:?}"))?;
    Ok(())
}

/// One invocation, hand-built: how a submission that never met the offer is put
/// in front of `prepare_authoritative`.
fn invocation(
    action: &mush_core::DefinitionRef,
    actor: ObjId,
    request: u8,
    parameters: serde_json::Value,
) -> mush_core::ActionInvocation {
    mush_core::ActionInvocation::new(mush_core::ActionInvocationDraft {
        request: RequestUid::from_bytes([request; 16]),
        principal: mush_core::PrincipalId::Object { id: actor },
        actor,
        offered_action: mush_core::ActionOfferId(mush_core::CanonicalHash::from_bytes(
            [request; 32],
        )),
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
            IdempotencyKey::Request(RequestUid::from_bytes([request; 16])),
        ),
    })
    .expect("an invocation")
}

/// Prepare a hand-built invocation; `Ok` installs it.
fn hand(
    world: &mut TestWorld,
    verb: IndustryVerb,
    actor: ObjId,
    request: u8,
    parameters: serde_json::Value,
) -> Result<(), mush_core::ActionErrorCode> {
    let action = action(world, verb);
    match world.engine_mut().prepare_registered_authoritative_action(
        &action,
        &invocation(&action, actor, request, parameters),
        CommitToken::from_bytes([request; 16]),
    ) {
        Ok((prepared, _)) => {
            world
                .engine_mut()
                .install_prepared_action(prepared.into_parts().1)
                .expect("the fixture installs");
            Ok(())
        }
        Err(mush_core::ActionProviderPrepareError::Provider { source, .. }) => Err(source.code()),
        Err(other) => panic!("expected a provider refusal, got {other}"),
    }
}

fn hand_message(
    world: &mut TestWorld,
    verb: IndustryVerb,
    actor: ObjId,
    request: u8,
    parameters: serde_json::Value,
) -> String {
    let action = action(world, verb);
    match world.engine_mut().prepare_registered_authoritative_action(
        &action,
        &invocation(&action, actor, request, parameters),
        CommitToken::from_bytes([request; 16]),
    ) {
        Ok(_) => "accepted".to_owned(),
        Err(mush_core::ActionProviderPrepareError::Provider { source, .. }) => {
            source.message().to_owned()
        }
        Err(other) => format!("{other}"),
    }
}

/// One `industry.sweep` fire (e7r1's `fire`): the tick is
/// `Coarse(SWEEP_PERIOD_SECS)`, so the world clock moves a whole period first.
fn sweep(tw: &mut TestWorld) {
    let period = i64::try_from(mush_industry::SWEEP_PERIOD_SECS).unwrap_or(60);
    let t = tw.world().now() + period + 1;
    tw.set_now(t);
    let _ = tw.fire_due(t.saturating_mul(1_000));
}

// ---- the test seams ----

/// One opening, offered to one actor.
struct OneOpening {
    actor: Arc<OnceLock<ObjId>>,
    opening: SystemOpening,
}

impl SystemOpener for OneOpening {
    fn next_opening(&self, _engine: &Engine, actor: ObjId) -> Option<SystemOpening> {
        (self.actor.get() == Some(&actor)).then(|| self.opening.clone())
    }
}

/// Draws answered from a table the test edits.
#[derive(Default)]
struct TableDraws(Mutex<BTreeMap<OreDepositId, Option<u64>>>);

impl ExternalDraws for TableDraws {
    fn drawn(&self, _ctx: &ModuleCtx<'_>, deposit: &OreDeposit) -> Option<u64> {
        self.0
            .lock()
            .ok()
            .and_then(|table| table.get(&deposit.id()).copied().flatten())
    }
}

/// Vouches for one claim to one actor.
struct Vouch {
    actor: Arc<OnceLock<ObjId>>,
    claim: ClaimId,
}

impl JurisdictionAuthority for Vouch {
    fn claims(&self, _engine: &Engine, _families: &IndustryFamilies, actor: ObjId) -> Vec<ClaimId> {
        if self.actor.get() == Some(&actor) {
            vec![self.claim]
        } else {
            Vec::new()
        }
    }
}

fn opening(receipt: &str, place: ObjId, vault: u64, reserve: u64) -> SystemOpening {
    SystemOpening {
        receipt: SmolStr::new(receipt),
        place,
        commodity: commodity("metal_ore"),
        grade: 0,
        accessibility: 100,
        grade_class: "surface".to_owned(),
        reserve_class: "surface".to_owned(),
        curve: vec![CurvePoint {
            cumulative: 0,
            per_cycle: 1,
        }],
        endpoint: EndpointRef(VaultId(vault)),
        reserve,
        external: SmolStr::new("tacsim.surface"),
        jurisdiction: Some(org(7)),
    }
}

// ---- the system opener (§3.4, D-20-5) ----

#[test]
fn sm6_a_system_opening_is_offered_only_to_its_opener_and_opens_once() {
    let who = Arc::new(OnceLock::new());
    let row = opening("ahuna_flank/c/metals", ObjId(77), 501, 1_234);
    let (mut world, families) = boot(Seams {
        opener: Some(Arc::new(OneOpening {
            actor: Arc::clone(&who),
            opening: row.clone(),
        })),
        ..Seams::default()
    });
    let clerk = world.spawn_player("clerk");
    let stranger = world.spawn_player("stranger");
    who.set(clerk).expect("the opener's actor");

    assert!(
        projects_verb(&world, clerk, "deposit-open"),
        "the opener is offered its one opening"
    );
    assert!(
        !projects_verb(&world, stranger, "deposit-open"),
        "nobody else is"
    );
    let fixed = resolved_parameters(
        &world,
        clerk,
        IndustryVerb::DepositOpen,
        serde_json::json!({ "receipt": "ahuna_flank/c/metals" }),
    );
    let rows = fixed
        .get(mush_industry::SYSTEM_OPENINGS)
        .and_then(serde_json::Value::as_object)
        .expect("the openings object");
    assert_eq!(rows.len(), 1, "one row an offer");
    assert_eq!(
        fixed[mush_industry::SYSTEM_OPENER],
        serde_json::json!(clerk.0)
    );

    offer(
        &mut world,
        clerk,
        IndustryVerb::DepositOpen,
        serde_json::json!({ "receipt": "ahuna_flank/c/metals" }),
        0x61,
    )
    .expect("the opener's submission lands");
    let id = mush_industry::derive_system_deposit_id("ahuna_flank/c/metals").expect("id");
    let opened = deposit_of(&mut world, &families, id).expect("the body is open at the derived id");
    assert_eq!(opened.place(), ObjId(77));
    assert_eq!(
        opened.external().map(SmolStr::as_str),
        Some("tacsim.surface")
    );
    assert_eq!(opened.jurisdiction(), Some(&org(7)));
    assert_eq!(opened.accessibility().get(), 100);
    assert_eq!(
        opened.curve().points(),
        &[CurvePoint {
            cumulative: 0,
            per_cycle: 1
        }]
    );
    assert_eq!(opened.initial_reserve(), 1_234);
    assert_eq!(pool(&mut world, &families, &opened), 1_234);

    // A second submission of the receipt refuses at the prepare, writing nothing.
    let again = offer(
        &mut world,
        clerk,
        IndustryVerb::DepositOpen,
        serde_json::json!({ "receipt": "ahuna_flank/c/metals" }),
        0x62,
    )
    .expect_err("a second opening of one receipt refuses");
    assert!(again.contains("that opening is already made"), "{again}");
    assert_eq!(
        pool(&mut world, &families, &opened),
        1_234,
        "nothing credited twice"
    );
    let census = read(world.engine_mut(), &families, |txn| {
        txn.get(&families.place_census, &ObjId(77))
            .expect("census")
            .copied()
            .map(mush_industry::PlaceCensus::get)
    });
    assert_eq!(census, Some(1));

    // A forged opener: the stranger's invocation carrying the clerk's rows.
    let mut rows_map = serde_json::Map::new();
    rows_map.insert(
        "ahuna_flank/d/metals".to_owned(),
        serde_json::to_value(opening("ahuna_flank/d/metals", ObjId(78), 502, 99)).expect("row"),
    );
    let forged = serde_json::json!({
        "now": stamp(1_000),
        "receipt": "ahuna_flank/d/metals",
        mush_industry::SYSTEM_OPENER: clerk.0,
        mush_industry::SYSTEM_OPENINGS: rows_map,
    });
    assert_eq!(
        hand(
            &mut world,
            IndustryVerb::DepositOpen,
            stranger,
            0x63,
            forged
        ),
        Err(mush_core::ActionErrorCode::Unauthorized),
        "another actor cannot carry the opener's rows"
    );
    let forged_id = mush_industry::derive_system_deposit_id("ahuna_flank/d/metals").expect("id");
    assert!(
        deposit_of(&mut world, &families, forged_id).is_none(),
        "nothing opened"
    );
}

#[test]
fn sm6_the_receipt_derives_the_id_whoever_asks() {
    let a = mush_industry::derive_system_deposit_id("ceres_port/a/metals").expect("id");
    let b = mush_industry::derive_system_deposit_id("ceres_port/a/metals").expect("id");
    let c = mush_industry::derive_system_deposit_id("ceres_port/a/ices").expect("id");
    assert_eq!(a, b, "one receipt, one id");
    assert_ne!(a, c, "another receipt, another id");
    for bad in ["", "Ceres/A/metals", "a b", &"x".repeat(97)] {
        assert_eq!(
            mush_industry::derive_system_deposit_id(bad),
            Err(IndustryError::InvalidReceipt),
            "{bad:?} is not a receipt"
        );
    }
    assert!(mush_industry::derive_system_deposit_id(&"x".repeat(96)).is_ok());
}

#[test]
fn sm6_one_system_opening_fits_the_offer() {
    let mut row = opening(&"r".repeat(96), ObjId(u32::MAX), u64::MAX, u64::MAX);
    row.external = SmolStr::new("e".repeat(64));
    row.jurisdiction = Some(PrincipalId::System(
        mush_org::SystemPrincipalId::new("s".repeat(40)).expect("system principal"),
    ));
    let mut rows = serde_json::Map::new();
    rows.insert(
        row.receipt.to_string(),
        serde_json::to_value(&row).expect("row"),
    );
    let bytes = serde_json::to_vec(&serde_json::json!({
        "now": stamp(i64::MAX),
        mush_industry::SYSTEM_OPENER: u32::MAX,
        mush_industry::SYSTEM_OPENINGS: rows,
    }))
    .expect("encodes")
    .len();
    assert!(
        bytes < mush_core::MAX_ACTION_PARAMETER_BYTES / 16,
        "the largest opening is {bytes} bytes, far under the parameter cap"
    );
}

// ---- the external refusal (§3.4; SM.5's F12 (ii)) ----

#[test]
fn sm6_an_external_deposit_is_never_offered_to_extract_and_its_prepare_refuses() {
    let (mut world, families) = boot(Seams::default());
    let miner = world.spawn_player("miner");
    license(&mut world, miner);
    let now = world.world().world_ic_now();
    let surface = external(body(0x5101, ObjId(70), "metal_ore", 701, 500));
    let twin = body(0x5102, ObjId(71), "metal_ore", 702, 500);
    open(&mut world, &families, &surface);
    open(&mut world, &families, &twin);
    for (n, deposit) in [(1_u64, &surface), (2, &twin)] {
        let mut record = claim(
            0x5100 + n,
            ClaimSubject::Deposit {
                deposit: deposit.id(),
            },
            org(7),
            obj(miner),
            0,
        );
        record = record
            .with_term(ClockStamp {
                clock: now.clock.clone(),
                at: now.at + 100_000,
            })
            .expect("term");
        file(&mut world, &families, &record);
    }
    let offered =
        mush_industry::provider::authorized_deposits(world.engine(), &families, miner, &obj(miner));
    assert_eq!(
        offered,
        vec![twin.id()],
        "the twin is offered, the surface body never"
    );

    // A hand-built start or cycle meets the structural refusal.
    let start = hand_message(
        &mut world,
        IndustryVerb::ExtractStart,
        miner,
        0x71,
        serde_json::json!({ "now": now, "deposit": surface.id().to_string(), "endpoint": "999" }),
    );
    assert_eq!(start, "You cannot work that here.");
    apply(world.engine_mut(), &families, |txn| {
        families.put_run(
            txn,
            mush_industry::ExtractionRun::new(surface.id(), obj(miner), EndpointRef(VaultId(999)))?,
        )
    })
    .expect("a run seeded by hand");
    let cycle = hand_message(
        &mut world,
        IndustryVerb::ExtractCycle,
        miner,
        0x72,
        serde_json::json!({ "now": now, "deposit": surface.id().to_string() }),
    );
    assert_eq!(cycle, "You cannot work that here.");
    assert_eq!(
        pool(&mut world, &families, &surface),
        500,
        "no unit left the ground"
    );
}

// ---- the sweep invariant (§3.4, D11) ----

#[test]
fn sm6_the_sweep_holds_a_balanced_external_body_and_quarantines_a_forged_debit() {
    let draws = Arc::new(TableDraws::default());
    let (mut world, families) = boot(Seams {
        draws: Some(Arc::clone(&draws) as Arc<dyn ExternalDraws>),
        ..Seams::default()
    });
    let v1 = make_vault(&mut world, "block_a");
    let v2 = make_vault(&mut world, "block_b");
    let a = external(body(0x6101, ObjId(80), "metal_ore", v1, 100));
    let sibling = external(body(0x6102, ObjId(80), "ice_ore", v1, 50));
    let other = external(body(0x6103, ObjId(81), "metal_ore", v2, 40));
    for deposit in [&a, &sibling, &other] {
        open(&mut world, &families, deposit);
    }
    // The provider drew 3 from `a`: the pool is short by exactly that.
    apply(world.engine_mut(), &families, |txn| {
        families
            .inventory
            .debit(txn, a.endpoint(), a.commodity(), 3)
            .map_err(IndustryError::Inventory)
    })
    .expect("the provider's draw");
    {
        let mut table = draws.0.lock().expect("table");
        table.insert(a.id(), Some(3));
        table.insert(sibling.id(), Some(0));
        table.insert(other.id(), Some(0));
    }
    sweep(&mut world);
    let quarantined = |world: &TestWorld| {
        world
            .world()
            .module_state::<IndustryStore>()
            .expect("store")
            .quarantined()
    };
    assert!(
        quarantined(&world).is_empty(),
        "a balanced body is held: {:?}",
        quarantined(&world)
    );

    // A forged debit of 1 the cursor does not account for.
    apply(world.engine_mut(), &families, |txn| {
        families
            .inventory
            .debit(txn, a.endpoint(), a.commodity(), 1)
            .map_err(IndustryError::Inventory)
    })
    .expect("the forged debit");
    sweep(&mut world);
    let store = world
        .world()
        .module_state::<IndustryStore>()
        .expect("store");
    assert_eq!(store.quarantine(a.id()), Some(Quarantine::DrawsMismatch));
    assert_eq!(
        store.quarantine(sibling.id()),
        Some(Quarantine::DrawsMismatch),
        "the whole block is closed"
    );
    assert_eq!(store.quarantine(other.id()), None, "another block is not");
    assert!(!Quarantine::DrawsMismatch.runbook().is_empty());

    // An answer of `None` is unanswered.
    draws.0.lock().expect("table").insert(other.id(), None);
    sweep(&mut world);
    let store = world
        .world()
        .module_state::<IndustryStore>()
        .expect("store");
    assert_eq!(
        store.quarantine(other.id()),
        Some(Quarantine::DrawsUnanswered)
    );

    // No composed seam at all: unwired.
    let (mut bare, bare_families) = boot(Seams::default());
    let v3 = make_vault(&mut bare, "block_c");
    let lone = external(body(0x6104, ObjId(82), "metal_ore", v3, 10));
    open(&mut bare, &bare_families, &lone);
    sweep(&mut bare);
    assert_eq!(
        bare.world()
            .module_state::<IndustryStore>()
            .expect("store")
            .quarantine(lone.id()),
        Some(Quarantine::ExternalUnwired)
    );
}

// ---- ClaimRenew in place (§9.2) ----

#[test]
fn sm6_renewing_in_place_never_spends_the_subject_index() {
    let (mut world, families) = boot(Seams::default());
    let deposit = body(0x7101, ObjId(90), "metal_ore", 901, 100);
    open(&mut world, &families, &deposit);
    let subject = ClaimSubject::Deposit {
        deposit: deposit.id(),
    };
    let record = claim(0x7101, subject, org(7), obj(ObjId(5)), 1_000);
    file(&mut world, &families, &record);
    let mut term = 1_000;
    for round in 1..=100_u64 {
        term += 10;
        let live = claim_of(&mut world, &families, record.id());
        apply(world.engine_mut(), &families, |txn| {
            families.renew_claim(txn, record.id(), live.revision(), stamp(term))
        })
        .expect("a renewal in place");
        let live = claim_of(&mut world, &families, record.id());
        assert_eq!(live.revision(), round + 1);
        assert_eq!(live.term(), &stamp(term));
        if round == 9 {
            assert_eq!(
                index_len(&mut world, &families, subject),
                1,
                "nine renewals, one slot"
            );
            assert_eq!(live.revision(), 10);
        }
    }
    assert_eq!(
        index_len(&mut world, &families, subject),
        1,
        "a hundred renewals, one slot"
    );
    assert_eq!(claim_of(&mut world, &families, record.id()).revision(), 101);
    // Another issuer's filing is still accepted.
    file(
        &mut world,
        &families,
        &claim(0x7102, subject, org(8), obj(ObjId(6)), 5_000),
    );
    assert_eq!(index_len(&mut world, &families, subject), 2);

    // A term not later refuses.
    let live = claim_of(&mut world, &families, record.id());
    assert_eq!(
        apply(world.engine_mut(), &families, |txn| {
            families.renew_claim(txn, record.id(), live.revision(), stamp(term))
        }),
        Err(IndustryError::TermNotLater)
    );
    // Another generation refuses too (fail closed).
    assert_eq!(
        apply(world.engine_mut(), &families, |txn| {
            families.renew_claim(
                txn,
                record.id(),
                live.revision(),
                ClockStamp {
                    clock: ClockRef::WorldIc { generation: 2 },
                    at: term + 1,
                },
            )
        }),
        Err(IndustryError::TermNotLater)
    );
    // Suspended and seized refuse.
    for (n, state) in [
        (0, ContestState::Suspended { by: org(7) }),
        (
            1,
            ContestState::Seized {
                by: org(7),
                at: stamp(10),
            },
        ),
    ] {
        let other = claim(
            0x7110 + n,
            ClaimSubject::Place {
                place: ObjId(91 + u32::try_from(n).expect("n")),
            },
            org(7),
            obj(ObjId(5)),
            1_000,
        );
        file(&mut world, &families, &other);
        let moved = other.with_contest_state(state).expect("state");
        apply(world.engine_mut(), &families, |txn| {
            families.put_claim(txn, 1, moved.clone())
        })
        .expect("moved");
        assert_eq!(
            apply(world.engine_mut(), &families, |txn| {
                families.renew_claim(txn, other.id(), 2, stamp(5_000))
            }),
            Err(IndustryError::ClaimNotLive)
        );
    }
    // A lapsed claim refuses: it is taken over, never renewed.
    let live = claim_of(&mut world, &families, record.id());
    apply(world.engine_mut(), &families, |txn| {
        families.lapse_claim(
            txn,
            record.id(),
            live.revision(),
            &stamp(2_000),
            &stamp(3_000),
        )
    })
    .expect("the lapse");
    let live = claim_of(&mut world, &families, record.id());
    assert_eq!(
        apply(world.engine_mut(), &families, |txn| {
            families.renew_claim(txn, record.id(), live.revision(), stamp(9_000))
        }),
        Err(IndustryError::ClaimLapsed)
    );
}

#[test]
fn sm6_a_filing_over_a_lapsed_claim_refuses_and_spends_nothing() {
    let (mut world, families) = boot(Seams::default());
    let deposit = body(0x7201, ObjId(92), "metal_ore", 921, 100);
    open(&mut world, &families, &deposit);
    let subject = ClaimSubject::Deposit {
        deposit: deposit.id(),
    };
    let issuer = org(4242);
    let record = claim(0x7201, subject, issuer.clone(), obj(ObjId(5)), 1_000);
    file(&mut world, &families, &record);
    apply(world.engine_mut(), &families, |txn| {
        families.lapse_claim(txn, record.id(), 1, &stamp(2_000), &stamp(3_000))
    })
    .expect("the lapse");
    // A same-issuer exclusive filing through the provider (staff, so the issuer
    // gate passes) refuses with the take-over line.
    let message = hand_message(
        &mut world,
        IndustryVerb::ClaimFile,
        GOD,
        0x73,
        serde_json::json!({
            "now": stamp(2_500),
            "subject": format!("deposit:{}", deposit.id()),
            "issuer": "org:4242",
            "holder": "obj:6",
            "object": 9_200,
            "exclusivity": "exclusive",
            "priority": 1,
            "term": 9_000,
            "transferability": "transferable",
            "claim_filed_by": GOD.0,
            "claim_file_staff": true,
            "claim_issuers": [],
        }),
    );
    assert_eq!(message, "that claim has lapsed: take it over");
    assert_eq!(
        index_len(&mut world, &families, subject),
        1,
        "nothing spent"
    );
    // The remedy: the take-over (by a stranger, so no cooling binds).
    apply(world.engine_mut(), &families, |txn| {
        families.take_over_claim(
            txn,
            record.id(),
            2,
            obj(ObjId(6)),
            stamp(9_000),
            None,
            &stamp(2_600),
        )
    })
    .expect("the take-over");
    let live = claim_of(&mut world, &families, record.id());
    assert_eq!(live.holder(), &obj(ObjId(6)));
    assert!(!live.is_lapsed());
    assert_eq!(index_len(&mut world, &families, subject), 1);
}

/// Four deposits at one place, opened, and their four claims.
fn four_claim_block(
    world: &mut TestWorld,
    families: &IndustryFamilies,
    place: ObjId,
    holder: PrincipalId,
    base: u64,
) -> Vec<ClaimRecord> {
    let goods = ["metal_ore", "ice_ore", "carbon_ore", "strategic_ore"];
    let deposits: Vec<OreDeposit> = goods
        .iter()
        .enumerate()
        .map(|(i, good)| body(base + i as u64, place, good, 950, 100))
        .collect();
    for deposit in &deposits {
        open(world, families, deposit);
    }
    let claims: Vec<ClaimRecord> = deposits
        .iter()
        .enumerate()
        .map(|(i, deposit)| {
            claim(
                base + i as u64,
                ClaimSubject::Deposit {
                    deposit: deposit.id(),
                },
                org(7),
                holder.clone(),
                1_000,
            )
        })
        .collect();
    // Staked in ONE bundle.
    apply(world.engine_mut(), families, |txn| {
        for record in &claims {
            families.file_claim(txn, record.clone())?;
        }
        Ok(())
    })
    .expect("a four-claim stake in one bundle");
    claims
}

#[test]
fn sm6_a_four_deposit_block_staked_in_one_bundle_files_four_claims() {
    let (mut world, families) = boot(Seams::default());
    let place = ObjId(100);
    let claims = four_claim_block(&mut world, &families, place, obj(ObjId(5)), 0x8100);
    for record in &claims {
        assert_eq!(claim_of(&mut world, &families, record.id()).epoch(), 0);
        assert_eq!(index_len(&mut world, &families, record.subject()), 1);
    }
    assert!(
        !tenure_stored(&mut world, &families, place),
        "a filing writes no tenure"
    );
    let tenure = tenure_of(&mut world, &families, place);
    assert_eq!(tenure.epoch(), 0, "epoch 0: since the block opened");
    assert_eq!(tenure.mark(), None);
}

/// One block event over all four claims in ONE bundle: lapse.
fn lapse_all(
    world: &mut TestWorld,
    families: &IndustryFamilies,
    claims: &[ClaimRecord],
    at: i64,
    cooling: i64,
) {
    let revisions: Vec<u64> = claims
        .iter()
        .map(|record| claim_of(world, families, record.id()).revision())
        .collect();
    apply(world.engine_mut(), families, |txn| {
        for (record, revision) in claims.iter().zip(&revisions) {
            families.lapse_claim(txn, record.id(), *revision, &stamp(at), &stamp(cooling))?;
        }
        Ok(())
    })
    .expect("one lapse bundle over the block");
}

/// One block event over all four claims in ONE bundle: take-over.
fn take_all(
    world: &mut TestWorld,
    families: &IndustryFamilies,
    claims: &[ClaimRecord],
    holder: &PrincipalId,
    at: i64,
) -> Result<(), IndustryError> {
    let revisions: Vec<u64> = claims
        .iter()
        .map(|record| claim_of(world, families, record.id()).revision())
        .collect();
    apply(world.engine_mut(), families, |txn| {
        for (record, revision) in claims.iter().zip(&revisions) {
            families.take_over_claim(
                txn,
                record.id(),
                *revision,
                holder.clone(),
                stamp(at + 100_000),
                Some(stamp(at)),
                &stamp(at),
            )?;
        }
        Ok(())
    })
}

#[test]
fn sm6_ten_block_lapses_and_take_overs_advance_the_epoch_once_each() {
    let (mut world, families) = boot(Seams::default());
    let place = ObjId(101);
    let claims = four_claim_block(&mut world, &families, place, obj(ObjId(5)), 0x8200);
    let cooling = 1_000;
    let mut at = 10_000;
    let mut epoch = 0;
    for cycle in 0..10_u32 {
        lapse_all(&mut world, &families, &claims, at, at + cooling);
        epoch += 1;
        let tenure = tenure_of(&mut world, &families, place);
        assert_eq!(tenure.epoch(), epoch, "a lapse is one block event");
        assert_eq!(tenure.mark(), Some(&TenureMark::Lapsed));
        let holder = claim_of(&mut world, &families, claims[0].id())
            .holder()
            .clone();
        assert_eq!(
            tenure
                .cooling()
                .iter()
                .filter(|row| row.holder == holder)
                .count(),
            1,
            "one cooling row for the holder"
        );
        at += cooling;
        // A stranger, after the cooling term.
        let stranger = obj(ObjId(200 + cycle));
        take_all(&mut world, &families, &claims, &stranger, at).expect("the take-over");
        epoch += 1;
        let tenure = tenure_of(&mut world, &families, place);
        assert_eq!(tenure.epoch(), epoch, "a take-over is one block event");
        for record in &claims {
            let live = claim_of(&mut world, &families, record.id());
            assert_eq!(live.epoch(), epoch, "the claim carries the new epoch");
            assert_eq!(live.holder(), &stranger);
            assert_eq!(index_len(&mut world, &families, record.subject()), 1);
        }
        at += 10;
    }
    let tenure = tenure_of(&mut world, &families, place);
    assert_eq!(tenure.epoch(), 20);
    assert_eq!(
        tenure.since(),
        Some(&stamp(at - 10)),
        "since the last take-over"
    );
    // An eleventh is accepted.
    lapse_all(&mut world, &families, &claims, at, at + cooling);
    at += cooling;
    take_all(&mut world, &families, &claims, &obj(ObjId(999)), at).expect("the eleventh take-over");
    assert_eq!(tenure_of(&mut world, &families, place).epoch(), 22);

    // A lapse split over four bundles advances it once.
    let before = tenure_of(&mut world, &families, place).epoch();
    for record in &claims {
        let live = claim_of(&mut world, &families, record.id());
        apply(world.engine_mut(), &families, |txn| {
            families.lapse_claim(
                txn,
                record.id(),
                live.revision(),
                &stamp(at + 5),
                &stamp(at + 5 + cooling),
            )
        })
        .expect("a split lapse");
    }
    assert_eq!(tenure_of(&mut world, &families, place).epoch(), before + 1);
}

#[test]
fn sm6_the_lapsed_holder_cools_for_a_term_and_a_stranger_does_not() {
    let (mut world, families) = boot(Seams::default());
    let place = ObjId(102);
    let holder = obj(ObjId(5));
    let claims = four_claim_block(&mut world, &families, place, holder.clone(), 0x8300);
    lapse_all(&mut world, &families, &claims, 10_000, 11_000);
    let tenure = tenure_of(&mut world, &families, place);
    assert_eq!(
        tenure.cooling().len(),
        1,
        "a four-claim lapse leaves ONE cooling row"
    );
    assert_eq!(
        read(world.engine_mut(), &families, |txn| families
            .cooling_until(txn, place, &holder, &stamp(10_500))
            .expect("reads")),
        Some(stamp(11_000))
    );
    // The lapsed holder at until − 1 refuses; at until it is accepted.
    assert_eq!(
        take_all(&mut world, &families, &claims, &holder, 10_999),
        Err(IndustryError::Cooling {
            until: stamp(11_000)
        })
    );
    assert!(
        claim_of(&mut world, &families, claims[0].id()).is_lapsed(),
        "the refused bundle wrote nothing"
    );
    take_all(&mut world, &families, &claims, &holder, 11_000).expect("at the end of the term");

    // A stranger is accepted at once.
    let other_place = ObjId(103);
    let others = four_claim_block(&mut world, &families, other_place, holder.clone(), 0x8400);
    lapse_all(&mut world, &families, &others, 20_000, 21_000);
    take_all(&mut world, &families, &others, &obj(ObjId(6)), 20_001).expect("a stranger at once");

    // Five distinct holders lapsing within one term keep four rows: the
    // soonest-ending is dropped and the four latest kept.
    let mut tenure = TenureRecord::fresh(ObjId(104));
    for (n, until) in [
        (1_u32, 5_000_i64),
        (2, 5_400),
        (3, 5_100),
        (4, 5_300),
        (5, 5_200),
    ] {
        let next = tenure
            .on_lapse(
                &stamp(1_000 + i64::from(n)),
                &obj(ObjId(n)),
                &stamp(until),
                &stamp(1_000 + i64::from(n)),
            )
            .expect("lapse");
        if let Some(next) = next {
            tenure = next;
        }
    }
    assert_eq!(tenure.cooling().len(), MAX_COOLING_ROWS);
    assert!(
        tenure.cooling_for(&obj(ObjId(1)), &stamp(1_010)).is_none(),
        "the soonest-ending row (holder 1, 5000) was dropped"
    );
    for n in 2..=5 {
        assert!(
            tenure.cooling_for(&obj(ObjId(n)), &stamp(1_010)).is_some(),
            "holder {n} kept"
        );
    }
}

// ---- the seize-transfers-tenure fix (§9.4; D10) ----

#[test]
fn sm6_a_contest_seizure_with_a_holder_transfers_the_tenure_once() {
    let (mut world, families) = boot(Seams::default());
    let challenger = world.spawn_player("challenger");
    license(&mut world, challenger);
    let later = world.world().now() + 100_000;
    world.set_now(later);
    let now = world.world().world_ic_now();
    assert!(now.at > 10, "the world clock has run: {now:?}");
    let at = |offset: i64| ClockStamp {
        clock: now.clock.clone(),
        at: now.at + offset,
    };
    let place = ObjId(110);
    let goods = ["metal_ore", "ice_ore", "carbon_ore", "strategic_ore"];
    let mut claims = Vec::new();
    for (i, good) in goods.iter().enumerate() {
        let deposit = body(0x9100 + i as u64, place, good, 960, 100);
        open(&mut world, &families, &deposit);
        let record = claim(
            0x9100 + i as u64,
            ClaimSubject::Deposit {
                deposit: deposit.id(),
            },
            org(7),
            obj(ObjId(5)),
            0,
        )
        .with_term(at(100_000))
        .expect("term");
        file(&mut world, &families, &record);
        claims.push((deposit, record));
    }
    let seize = |claim: ClaimId, revision: u64, extra: serde_json::Value| {
        let mut parameters = serde_json::json!({
            "claim": claim.to_string(),
            "state": "seized",
            "revision": revision,
        });
        if let (Some(map), Some(extra)) = (parameters.as_object_mut(), extra.as_object()) {
            for (key, value) in extra {
                map.insert(key.clone(), value.clone());
            }
        }
        parameters
    };
    let discovered = now.at.saturating_sub(10);
    let holder_token = format!("obj:{}", challenger.0);
    let (first_deposit, first) = &claims[0];
    offer(
        &mut world,
        GOD,
        IndustryVerb::ClaimContest,
        seize(
            first.id(),
            2, // `with_term` bumped the filed revision
            serde_json::json!({ "holder": holder_token, "discovered": discovered }),
        ),
        0x81,
    )
    .expect("a seizure that names a holder lands");
    let live = claim_of(&mut world, &families, first.id());
    assert_eq!(live.contest_state(), &ContestState::Uncontested);
    assert_eq!(live.holder(), &obj(challenger));
    assert_eq!(
        live.discovered_at(),
        Some(&ClockStamp {
            clock: now.clock.clone(),
            at: discovered
        }),
        "the stamp passed, not now"
    );
    assert_eq!(live.epoch(), 1);
    assert_eq!(tenure_of(&mut world, &families, place).epoch(), 1);
    let remaining = pool(&mut world, &families, first_deposit);
    assert!(
        mush_industry::extraction_authorized(
            world.world(),
            challenger,
            &obj(challenger),
            first_deposit,
            remaining,
            std::slice::from_ref(&live),
            Some(&licence_id()),
            &world.world().world_ic_now(),
        )
        .is_ok(),
        "the new holder is authorized"
    );
    // A replay at the old revision refuses.
    assert!(
        offer(
            &mut world,
            GOD,
            IndustryVerb::ClaimContest,
            seize(first.id(), 2, serde_json::json!({ "holder": holder_token })),
            0x82,
        )
        .is_err(),
        "a replay refuses at the checked revision"
    );
    // The three siblings to the same holder, each in its own invocation: the
    // epoch stays where the first left it. The second carries no discovery:
    // its stamp is `now`.
    for (n, (_, record)) in claims.iter().enumerate().skip(1) {
        let extra = if n == 1 {
            serde_json::json!({ "holder": holder_token })
        } else {
            serde_json::json!({ "holder": holder_token, "discovered": discovered })
        };
        offer(
            &mut world,
            GOD,
            IndustryVerb::ClaimContest,
            seize(record.id(), 2, extra),
            0x83 + u8::try_from(n).expect("n"),
        )
        .expect("a sibling seizure");
        assert_eq!(tenure_of(&mut world, &families, place).epoch(), 1);
        let sibling = claim_of(&mut world, &families, record.id());
        assert_eq!(sibling.epoch(), 1, "a sibling carries the block's epoch");
        assert_eq!(sibling.holder(), &obj(challenger));
    }
    // The clock has not moved since `now` was read, so the seizure's own `now`
    // is exactly that stamp (review T9: the whole stamp, not only its clock).
    assert_eq!(world.world().world_ic_now(), now, "the clock is still");
    let second = claim_of(&mut world, &families, claims[1].1.id());
    assert_eq!(
        second.discovered_at(),
        Some(&now),
        "without `discovered` the stamp is the seizure's own now"
    );
    // `discovered` after now, and `discovered` without `holder`, refuse.
    let place2 = ObjId(111);
    let deposit = body(0x9200, place2, "metal_ore", 961, 100);
    open(&mut world, &families, &deposit);
    let record = claim(
        0x9200,
        ClaimSubject::Deposit {
            deposit: deposit.id(),
        },
        org(7),
        obj(ObjId(5)),
        0,
    )
    .with_term(at(100_000))
    .expect("term");
    file(&mut world, &families, &record);
    let late = offer(
        &mut world,
        GOD,
        IndustryVerb::ClaimContest,
        seize(
            record.id(),
            2,
            serde_json::json!({ "holder": holder_token, "discovered": now.at + 1_000_000 }),
        ),
        0x90,
    )
    .expect_err("a discovery after the seizure");
    assert!(
        late.contains("a discovery cannot postdate the seizure"),
        "{late}"
    );
    let lone = offer(
        &mut world,
        GOD,
        IndustryVerb::ClaimContest,
        seize(
            record.id(),
            2,
            serde_json::json!({ "discovered": discovered }),
        ),
        0x91,
    )
    .expect_err("a discovery without a holder");
    assert!(
        lone.contains("a discovery stamp goes with a holder"),
        "{lone}"
    );
    // Without a holder the seizure is exactly as before, and the epoch is still.
    offer(
        &mut world,
        GOD,
        IndustryVerb::ClaimContest,
        seize(record.id(), 2, serde_json::json!({})),
        0x92,
    )
    .expect("a plain seizure");
    let live = claim_of(&mut world, &families, record.id());
    assert!(matches!(live.contest_state(), ContestState::Seized { .. }));
    assert_eq!(live.holder(), &obj(ObjId(5)));
    assert_eq!(tenure_of(&mut world, &families, place2).epoch(), 0);
    assert!(!tenure_stored(&mut world, &families, place2));
}

/// D-20-12 (§9.2): a LAPSED claim authorizes nobody, whatever its term says —
/// `extraction_authorized` answers `ClaimExpired` for a claim whose term is still
/// in the future once `lapse_claim` has marked it, and answered `Ok` for the same
/// holder, licence and body the moment before (review T3).
#[test]
fn sm6_a_lapsed_claim_with_a_live_term_authorizes_nobody() {
    let (mut world, families) = boot(Seams::default());
    let holder = world.spawn_player("holder");
    license(&mut world, holder);
    let later = world.world().now() + 100_000;
    world.set_now(later);
    let now = world.world().world_ic_now();
    let deposit = body(0x9300, ObjId(112), "metal_ore", 962, 100);
    open(&mut world, &families, &deposit);
    let term = ClockStamp {
        clock: now.clock.clone(),
        at: now.at + 100_000,
    };
    let record = claim(
        0x9300,
        ClaimSubject::Deposit {
            deposit: deposit.id(),
        },
        org(7),
        obj(holder),
        0,
    )
    .with_term(term.clone())
    .expect("term");
    file(&mut world, &families, &record);
    let ask = |world: &mut TestWorld, live: &ClaimRecord| {
        let remaining = pool(world, &families, &deposit);
        mush_industry::extraction_authorized(
            world.world(),
            holder,
            &obj(holder),
            &deposit,
            remaining,
            std::slice::from_ref(live),
            Some(&licence_id()),
            &world.world().world_ic_now(),
        )
    };
    let live = claim_of(&mut world, &families, record.id());
    assert!(ask(&mut world, &live).is_ok(), "the live claim authorizes");
    apply(world.engine_mut(), &families, |txn| {
        families.lapse_claim(
            txn,
            record.id(),
            live.revision(),
            &now,
            &ClockStamp {
                clock: now.clock.clone(),
                at: now.at + 1_000,
            },
        )
    })
    .expect("the lapse");
    let lapsed = claim_of(&mut world, &families, record.id());
    assert!(lapsed.is_lapsed());
    assert_eq!(lapsed.term(), &term, "the term is still in the future");
    assert_eq!(
        ask(&mut world, &lapsed).map(|_| ()),
        Err(mush_industry::ExtractionRefusal::ClaimExpired),
        "a lapsed claim is taken over, never worked"
    );
}

// ---- the jurisdiction row (§9.6, D8) ----

/// An org `name`, with `who` granted the spending capability in it.
fn vouch(world: &mut TestWorld, name: &str, who: ObjId) -> mush_org::OrgId {
    let _ = world.run(GOD, &format!("@org/create {name}"));
    let id = world
        .world()
        .module_state::<mush_org::OrgStore>()
        .expect("org store")
        .state()
        .orgs()
        .find(|o| o.name.as_str() == name)
        .expect("the org")
        .id;
    let _ = world.run(
        GOD,
        &format!("@org/grant #{}={name} economy.account.post", who.0),
    );
    id
}

#[test]
fn sm6_the_jurisdiction_row_reaches_its_own_deposits_and_no_others() {
    let (mut world, families) = boot(Seams::default());
    let registrar = world.spawn_player("registrar");
    let j = vouch(&mut world, "Registry", registrar);
    let ours = body(0xA101, ObjId(120), "metal_ore", 970, 100)
        .with_jurisdiction(PrincipalId::Organization(j));
    let theirs = body(0xA102, ObjId(121), "metal_ore", 971, 100).with_jurisdiction(org(4_444));
    let nobodys = body(0xA103, ObjId(122), "metal_ore", 972, 100);
    for deposit in [&ours, &theirs, &nobodys] {
        open(&mut world, &families, deposit);
    }
    let paper = |id: u64, deposit: &OreDeposit| {
        claim(
            id,
            ClaimSubject::Deposit {
                deposit: deposit.id(),
            },
            org(5_555), // the Syndicate's paper
            obj(ObjId(5)),
            100_000_000,
        )
    };
    let at_ours = paper(0xA101, &ours);
    let at_theirs = paper(0xA102, &theirs);
    let at_nobodys = paper(0xA103, &nobodys);
    for record in [&at_ours, &at_theirs, &at_nobodys] {
        file(&mut world, &families, record);
    }
    assert_eq!(
        mush_industry::jurisdiction_claims(world.engine(), &families, registrar),
        vec![at_ours.id()]
    );
    assert!(
        projects_verb(&world, registrar, "claim-contest"),
        "the registrar is offered"
    );
    let fixed = resolved_parameters(
        &world,
        registrar,
        IndustryVerb::ClaimContest,
        serde_json::json!({ "claim": at_ours.id().to_string(), "state": "suspended", "revision": 1 }),
    );
    let listed: Vec<String> = fixed[mush_industry::CLAIM_ADMINISTRABLE]
        .as_array()
        .expect("the list")
        .iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    assert_eq!(listed, vec![at_ours.id().to_string()]);
    offer(
        &mut world,
        registrar,
        IndustryVerb::ClaimContest,
        serde_json::json!({ "claim": at_ours.id().to_string(), "state": "suspended", "revision": 1 }),
        0xA1,
    )
    .expect("the issuer suspends a foreign claim at its own site");
    assert_eq!(
        claim_of(&mut world, &families, at_ours.id()).contest_state(),
        &ContestState::Suspended { by: obj(registrar) }
    );
    // The seize-transfers-tenure parameters are staff-only (review F1): the
    // registrar's own offer does not declare `holder`, and a hand-built seizure
    // of its own listed claim that names one — or carries `discovered` — refuses
    // with nothing written: no holder rewritten, no tenure epoch moved.
    let declared = resolve(
        &world,
        registrar,
        IndustryVerb::ClaimContest,
        serde_json::json!({
            "claim": at_ours.id().to_string(),
            "state": "seized",
            "revision": 2,
            "holder": format!("obj:{}", registrar.0),
        }),
        0xA4,
    );
    assert!(declared.is_err(), "the non-staff offer declares no holder");
    for (n, extra) in [
        serde_json::json!({ "holder": format!("obj:{}", registrar.0) }),
        serde_json::json!({ "discovered": 0 }),
        serde_json::json!({ "holder": format!("obj:{}", registrar.0), "discovered": 0 }),
    ]
    .into_iter()
    .enumerate()
    {
        let mut parameters = serde_json::json!({
            "now": stamp(1),
            "claim": at_ours.id().to_string(),
            "state": "seized",
            "revision": 2,
            mush_industry::CLAIM_CONTEST_BY: registrar.0,
            mush_industry::CLAIM_CONTEST_STAFF: false,
            mush_industry::CLAIM_ADMINISTRABLE: listed,
        });
        if let (Some(map), Some(extra)) = (parameters.as_object_mut(), extra.as_object()) {
            for (key, value) in extra {
                map.insert(key.clone(), value.clone());
            }
        }
        assert_eq!(
            hand(
                &mut world,
                IndustryVerb::ClaimContest,
                registrar,
                0xA5 + u8::try_from(n).expect("n"),
                parameters,
            ),
            Err(mush_core::ActionErrorCode::Unauthorized),
            "{extra}"
        );
        let live = claim_of(&mut world, &families, at_ours.id());
        assert_eq!(live.holder(), &obj(ObjId(5)));
        assert_eq!(live.revision(), 2);
        assert_eq!(
            live.contest_state(),
            &ContestState::Suspended { by: obj(registrar) }
        );
        assert!(!tenure_stored(&mut world, &families, ObjId(120)));
    }
    // Not offered elsewhere, and a hand-built contest refuses.
    for record in [&at_theirs, &at_nobodys] {
        assert_eq!(
            hand(
                &mut world,
                IndustryVerb::ClaimContest,
                registrar,
                0xA2,
                serde_json::json!({
                    "now": stamp(1),
                    "claim": record.id().to_string(),
                    "state": "suspended",
                    "revision": 1,
                    mush_industry::CLAIM_CONTEST_BY: registrar.0,
                    mush_industry::CLAIM_CONTEST_STAFF: false,
                    mush_industry::CLAIM_ADMINISTRABLE: listed,
                }),
            ),
            Err(mush_core::ActionErrorCode::Unauthorized)
        );
        assert_eq!(
            claim_of(&mut world, &families, record.id()).contest_state(),
            &ContestState::Uncontested
        );
    }
    // An invocation with no list refuses outright.
    assert_eq!(
        hand(
            &mut world,
            IndustryVerb::ClaimContest,
            registrar,
            0xA3,
            serde_json::json!({
                "now": stamp(1),
                "claim": at_theirs.id().to_string(),
                "state": "suspended",
                "revision": 1,
            }),
        ),
        Err(mush_core::ActionErrorCode::Unauthorized)
    );
}

#[test]
fn sm6_a_composed_jurisdiction_authority_replaces_the_default() {
    let who = Arc::new(OnceLock::new());
    let (mut world, families) = boot(Seams {
        jurisdiction: Some(Arc::new(Vouch {
            actor: Arc::clone(&who),
            claim: ClaimId(0xB101),
        })),
        ..Seams::default()
    });
    let renewer = world.spawn_player("renewer");
    who.set(renewer).expect("actor");
    let registrar = world.spawn_player("registrar");
    let j = vouch(&mut world, "Registry", registrar);
    let one = body(0xB101, ObjId(130), "metal_ore", 980, 100)
        .with_jurisdiction(PrincipalId::Organization(j));
    let two = body(0xB102, ObjId(131), "metal_ore", 981, 100)
        .with_jurisdiction(PrincipalId::Organization(j));
    open(&mut world, &families, &one);
    open(&mut world, &families, &two);
    for (id, deposit) in [(0xB101_u64, &one), (0xB102, &two)] {
        file(
            &mut world,
            &families,
            &claim(
                id,
                ClaimSubject::Deposit {
                    deposit: deposit.id(),
                },
                org(5_555),
                obj(ObjId(5)),
                100_000_000,
            ),
        );
    }
    let fixed = resolved_parameters(
        &world,
        renewer,
        IndustryVerb::ClaimContest,
        serde_json::json!({ "claim": ClaimId(0xB101).to_string(), "state": "suspended", "revision": 1 }),
    );
    assert_eq!(
        fixed[mush_industry::CLAIM_ADMINISTRABLE],
        serde_json::json!([ClaimId(0xB101).to_string()])
    );
    offer(
        &mut world,
        renewer,
        IndustryVerb::ClaimContest,
        serde_json::json!({ "claim": ClaimId(0xB101).to_string(), "state": "suspended", "revision": 1 }),
        0xB1,
    )
    .expect("the vouched claim moves");
    // The default row no longer applies: the registrar, who acts for the
    // jurisdiction, is offered nothing.
    assert!(!projects_verb(&world, registrar, "claim-contest"));
}

// ---- the records ----

#[test]
fn sm6_the_new_fields_round_trip_and_an_old_record_decodes() {
    let deposit = body(0xC101, ObjId(140), "metal_ore", 990, 100);
    let encoded = serde_json::to_value(&deposit).expect("encodes");
    let keys: Vec<&str> = encoded
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        vec![
            "accessibility",
            "commodity",
            "curve",
            "endpoint",
            "grade",
            "grade_class",
            "id",
            "initial_reserve",
            "place",
            "reserve_class",
            "revision"
        ],
        "a staff-opened body encodes exactly as before"
    );
    assert_eq!(
        serde_json::from_value::<OreDeposit>(encoded).expect("an old record decodes"),
        deposit
    );
    let marked = external(deposit.clone()).with_jurisdiction(org(7));
    let back: OreDeposit =
        serde_json::from_value(serde_json::to_value(&marked).expect("encodes")).expect("decodes");
    assert_eq!(back, marked);
    assert!(deposit.clone().with_external("Bad Name").is_err());
    let mut forged = serde_json::to_value(&marked).expect("encodes");
    forged["external"] = serde_json::json!("x".repeat(65));
    assert!(
        serde_json::from_value::<OreDeposit>(forged).is_err(),
        "the wire re-runs the check"
    );

    let record = claim(
        0xC101,
        ClaimSubject::Place { place: ObjId(140) },
        org(7),
        obj(ObjId(5)),
        1_000,
    );
    let encoded = serde_json::to_value(&record).expect("encodes");
    for key in ["epoch", "discovered_at", "lapsed_at"] {
        assert!(
            encoded.get(key).is_none(),
            "{key} is skipped at its default"
        );
    }
    assert_eq!(
        serde_json::from_value::<ClaimRecord>(encoded).expect("decodes"),
        record
    );
    let moved = record
        .taken_over(obj(ObjId(6)), stamp(5_000), Some(stamp(10)), 3)
        .expect("taken over")
        .lapsed(stamp(20))
        .expect("lapsed");
    let back: ClaimRecord =
        serde_json::from_value(serde_json::to_value(&moved).expect("encodes")).expect("decodes");
    assert_eq!(back, moved);
    assert_eq!(back.epoch(), 3);

    let row = |n: u32| serde_json::json!({ "holder": obj(ObjId(n)), "until": stamp(10) });
    let five = serde_json::json!({
        "place": 1, "epoch": 1, "since": null,
        "cooling": [row(1), row(2), row(3), row(4), row(5)],
    });
    assert!(
        serde_json::from_value::<TenureRecord>(five).is_err(),
        "past the cap"
    );
    let twice = serde_json::json!({
        "place": 1, "epoch": 1, "since": null, "cooling": [row(1), row(1)],
    });
    assert!(
        serde_json::from_value::<TenureRecord>(twice).is_err(),
        "two rows for one holder"
    );
    let fine = serde_json::json!({
        "place": 1, "epoch": 1, "since": stamp(5), "mark": { "kind": "lapsed" },
        "cooling": [row(1)],
    });
    let tenure: TenureRecord = serde_json::from_value(fine).expect("decodes");
    assert_eq!(tenure.mark(), Some(&TenureMark::Lapsed));
}
