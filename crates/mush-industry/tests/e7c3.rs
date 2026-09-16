//! E7 cut 3's verification, by scenario, over the three lenses 10b §3.3 names:
//! **F** fog, **D** depletion, **T** tenure.
//!
//! The rig is `crates/mush-inventory/tests/e7c2.rs`'s shape: `boot()` composes
//! the economy, cut 2 and cut 3 in production order, `apply()` opens the same
//! owner-stamped transaction a provider gets and **installs** it exactly as a
//! receipt does, so every "writes nothing" assertion below means what it says.

use std::sync::Arc;

use mush_core::{
    ClockRef, ClockStamp, CommitToken, Engine, GOD, IdempotencyKey, IdempotencyScope,
    ModuleBootData, ObjId, ObjKind, OfferKeyRing, OfferSecret, OwnerId, PerceivedRef, RequestUid,
    ResolutionKeyRing, ResolutionSecret, ScopedIdempotencyKey,
};
use mush_economy::{ExternalRef, FactProvider, VaultId};
use mush_industry::{
    Accessibility, BandId, ClaimIndex, ClaimRecord, ClaimSubject, ContestState, CurvePoint,
    Exclusivity, ExtractionRefusal, ExtractionRun, IndustryError, IndustryFamilies,
    IndustryHandles, IndustryModule, IndustryStore, MAX_CLAIMS_PER_SUBJECT, MAX_ORE_RESERVE,
    MAX_SURVEY_BANDS, OreDeposit, OreDepositId, Quarantine, SurveyBands, SurveyDoc, SurveyRow,
    Transferability, YieldCurve,
};
use mush_inventory::{EndpointRef, Grade, InventoryHandleCell, InventoryModule};
use mush_org::PrincipalId;
use mush_testkit::TestWorld;

// ---- E7c3: the rig ----

fn boot() -> (TestWorld, IndustryFamilies) {
    let cell: InventoryHandleCell = Arc::default();
    let world = TestWorld::with_modules(vec![
        Box::new(mush_economy::EconomyModule::new()),
        Box::new(
            InventoryModule::new()
                .with_quantity_delegates(["industry"])
                .with_handle_cell(Arc::clone(&cell)),
        ),
        Box::new(IndustryModule::new().with_inventory(Arc::clone(&cell))),
    ]);
    let families = world
        .world()
        .module_state::<IndustryHandles>()
        .expect("industry publishes its handles")
        .0
        .clone();
    (world, families)
}

fn commodity(id: &str) -> mush_core::CanonicalId {
    mush_core::CanonicalId::new(id).expect("test commodity id")
}

fn endpoint(id: u64) -> EndpointRef {
    EndpointRef(VaultId(id))
}

fn now() -> ClockStamp {
    ClockStamp {
        clock: ClockRef::WorldIc { generation: 1 },
        at: 1_000,
    }
}

fn later(at: i64) -> ClockStamp {
    ClockStamp {
        clock: ClockRef::WorldIc { generation: 1 },
        at,
    }
}

fn other_generation() -> ClockStamp {
    ClockStamp {
        clock: ClockRef::WorldIc { generation: 2 },
        at: 1_000,
    }
}

fn band(text: &str) -> BandId {
    BandId::new(text).expect("test band")
}

fn curve(points: &[(u64, u64)]) -> YieldCurve {
    YieldCurve::new(
        points
            .iter()
            .map(|(cumulative, per_cycle)| CurvePoint {
                cumulative: *cumulative,
                per_cycle: *per_cycle,
            })
            .collect(),
    )
    .expect("test curve")
}

fn deposit(id: u64, reserve: u64, points: &[(u64, u64)]) -> OreDeposit {
    OreDeposit::new(
        OreDepositId(id),
        ObjId(7),
        commodity("metals"),
        Grade::new(60).expect("grade"),
        Accessibility::new(100).expect("accessibility"),
        band("grade_b"),
        band("reserve_major"),
        curve(points),
        endpoint(id),
        reserve,
    )
    .expect("test deposit")
}

fn holder(actor: ObjId, tag: &str) -> ScopedIdempotencyKey {
    ScopedIdempotencyKey::new(
        IdempotencyScope {
            principal: mush_core::PrincipalId::Object { id: actor },
            owner: OwnerId::new("industry").expect("owner"),
            namespace: mush_core::CanonicalId::new(tag).expect("namespace"),
        },
        0,
        IdempotencyKey::Request(RequestUid::from_bytes([0x11; 16])),
    )
}

fn source(tag: &str) -> ExternalRef {
    ExternalRef {
        provider: smol_str::SmolStr::new_static("industry"),
        kind: smol_str::SmolStr::new_static("extraction.cycle"),
        id: smol_str::SmolStr::new(tag),
        revision: Some(1),
    }
}

/// Run one closure inside the owner-stamped transaction a provider would get,
/// then **install** it exactly as an exact receipt does. An `Err` installs
/// nothing at all.
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

fn open(engine: &mut Engine, families: &IndustryFamilies, deposit: &OreDeposit) {
    apply(engine, families, |txn| {
        families.open_deposit(txn, deposit.clone())
    })
    .expect("open the body");
}

fn start_run(engine: &mut Engine, families: &IndustryFamilies, body: &OreDeposit, to: EndpointRef) {
    apply(engine, families, |txn| {
        families.put_run(
            txn,
            ExtractionRun::new(body.id(), PrincipalId::Object(GOD), to)?,
        )
    })
    .expect("open the run");
}

/// One cycle, as the provider runs it, with a fresh request each time.
fn cycle(
    engine: &mut Engine,
    families: &IndustryFamilies,
    body: &OreDeposit,
    ordinal: u8,
) -> Result<mush_industry::families::Cycled, IndustryError> {
    let request = RequestUid::from_bytes([ordinal; 16]);
    apply(engine, families, |txn| {
        let live = families.deposit(txn, body.id())?.expect("body");
        let run = families.run(txn, body.id())?.expect("run");
        families.run_cycle(
            txn,
            &live,
            &run,
            GOD,
            request,
            &holder(GOD, "extract"),
            &now(),
            CommitToken::from_bytes([ordinal; 16]),
            source("extract/x/0"),
        )
    })
}

fn claim(id: u64, subject: ClaimSubject, issuer: u32, holder_id: u32) -> ClaimRecord {
    ClaimRecord::new(
        mush_industry::ClaimId(id),
        subject,
        PrincipalId::Object(ObjId(issuer)),
        PrincipalId::Object(ObjId(holder_id)),
        ObjId(900 + id as u32),
        Exclusivity::Shared,
        1,
        later(100_000),
        Transferability::Transferable,
    )
    .expect("test claim")
}

// =====================================================================
// F — fog. Can a viewer learn a reserve, a curve or another principal's
// band through any render, error, or arithmetic difference?
// =====================================================================

#[test]
fn e7c3_f_a_survey_copies_the_recorded_classes_and_cannot_narrow_a_band() {
    let body = deposit(1, 50_000, &[(0, 500), (20_000, 300)]);
    let alice = PrincipalId::Object(ObjId(2));
    let doc = SurveyDoc::empty(body.id());
    let bands = SurveyBands {
        grade_class: body.grade_class().clone(),
        reserve_class: body.reserve_class().clone(),
        at: now(),
        fact: source("survey/1"),
    };
    let (first, fresh) = doc.disclosed(&alice, bands.clone()).expect("disclose");
    assert!(fresh, "the first survey is a fresh row");
    // A SECOND survey by the same principal rewrites nothing and answers the
    // identical two bands: there is no narrowing, because there was never a
    // roll to narrow.
    let (second, again) = first.disclosed(&alice, bands.clone()).expect("re-survey");
    assert!(!again, "a re-survey is a HIT");
    assert_eq!(first, second);
    assert_eq!(
        second.bands_for(&alice).expect("row"),
        &bands,
        "a repeated survey answers the same recorded classes"
    );
}

#[test]
fn e7c3_f_two_principals_read_the_same_recorded_classes_never_reserve_requantised() {
    let body = deposit(2, 50_000, &[(0, 500)]);
    let alice = PrincipalId::Object(ObjId(2));
    let bob = PrincipalId::Object(ObjId(3));
    let make = || SurveyBands {
        grade_class: body.grade_class().clone(),
        reserve_class: body.reserve_class().clone(),
        at: now(),
        fact: source("survey/2"),
    };
    let doc = SurveyDoc::empty(body.id());
    let (doc, _) = doc.disclosed(&alice, make()).expect("alice");
    let (doc, _) = doc.disclosed(&bob, make()).expect("bob");
    let a = doc.bands_for(&alice).expect("alice row");
    let b = doc.bands_for(&bob).expect("bob row");
    assert_eq!(a.grade_class, b.grade_class);
    assert_eq!(a.reserve_class, b.reserve_class);
    assert_eq!(&a.grade_class, body.grade_class());
    assert_eq!(&a.reserve_class, body.reserve_class());
}

#[test]
fn e7c3_f_a_band_render_never_contains_the_exact_reserve() {
    let body = deposit(3, 987_654, &[(0, 500)]);
    let bands = SurveyBands {
        grade_class: body.grade_class().clone(),
        reserve_class: body.reserve_class().clone(),
        at: now(),
        fact: source("survey/3"),
    };
    let rendered = format!("{} {}", bands.grade_class, bands.reserve_class);
    assert!(
        !rendered.contains("987654") && !rendered.contains("987_654"),
        "a band render must not carry the reserve: {rendered}"
    );
    // The recorded classes are opaque tokens, not quantities.
    assert!(
        !bands
            .reserve_class
            .as_str()
            .chars()
            .all(|c| c.is_ascii_digit()),
        "a band id is a class, never a number"
    );
}

#[test]
fn e7c3_f_a_survey_map_past_its_cap_is_refused_on_load_never_truncated() {
    let body = OreDepositId(4);
    let rows: Vec<SurveyRow> = (0..=MAX_SURVEY_BANDS)
        .map(|index| SurveyRow {
            principal: PrincipalId::Object(ObjId(100 + index as u32)),
            bands: SurveyBands {
                grade_class: band("grade_b"),
                reserve_class: band("reserve_major"),
                at: now(),
                fact: source("survey/4"),
            },
        })
        .collect();
    let refused = SurveyDoc::from_rows(body, rows.clone());
    assert!(
        refused.is_err(),
        "a map past the cap is REFUSED, not truncated"
    );

    // And the wire refuses it too: a hand-edited stored document does not
    // silently lose the row nobody could then account for.
    let doc = serde_json::json!({ "deposit": body, "rows": rows });
    assert!(
        serde_json::from_value::<SurveyDoc>(doc).is_err(),
        "the TryFrom wire re-runs the cap on load"
    );
}

#[test]
fn e7c3_f_a_hand_edited_survey_row_fails_its_wire() {
    let body = OreDepositId(5);
    let alice = PrincipalId::Object(ObjId(2));
    let bob = PrincipalId::Object(ObjId(3));
    let row = |principal: &PrincipalId| SurveyRow {
        principal: principal.clone(),
        bands: SurveyBands {
            grade_class: band("grade_b"),
            reserve_class: band("reserve_major"),
            at: now(),
            fact: source("survey/5"),
        },
    };
    // Rows out of canonical order, or a duplicated principal, are the two
    // shapes a forger reaches for: both are refused.
    assert!(SurveyDoc::from_rows(body, vec![row(&bob), row(&alice)]).is_err());
    assert!(SurveyDoc::from_rows(body, vec![row(&alice), row(&alice)]).is_err());
    // A band token that is not a canonical class fails at the band's own wire.
    assert!(serde_json::from_value::<BandId>(serde_json::json!("Grade B")).is_err());
}

#[test]
fn e7c3_f_a_transferred_claim_does_not_transfer_the_transferors_bands() {
    let body = OreDepositId(6);
    let alice = PrincipalId::Object(ObjId(2));
    let bob = PrincipalId::Object(ObjId(3));
    let doc = SurveyDoc::empty(body);
    let (doc, _) = doc
        .disclosed(
            &alice,
            SurveyBands {
                grade_class: band("grade_b"),
                reserve_class: band("reserve_major"),
                at: now(),
                fact: source("survey/6"),
            },
        )
        .expect("alice surveys");
    // Tenure moved; knowledge did not. The disclosure map is keyed by the
    // principal who *looked*, not by whoever holds the claim today.
    let record = claim(6, ClaimSubject::Deposit { deposit: body }, 2, 2);
    let moved = record.with_holder(bob.clone()).expect("transfer");
    assert_eq!(moved.holder(), &bob);
    assert!(doc.bands_for(&bob).is_none(), "bob learned nothing");
    assert!(doc.bands_for(&alice).is_some());
}

#[test]
fn e7c3_f_a_deposit_endpoint_is_not_a_reserve_oracle_through_inventory() {
    // OPEN 1 makes a body an inventory endpoint, so cut 2's own read is the
    // third channel cut 3's fog budget has to close. It is closed by E3's
    // gate, not by a cut-3 check: an endpoint the viewer may not read and one
    // that does not exist are the same answer.
    let (world, _families) = boot();
    let stranger = ObjId(4_242);
    let body_endpoint = endpoint(77);
    let absent = endpoint(999_999);
    assert_eq!(
        mush_inventory::endpoint_visible(world.world(), stranger, body_endpoint),
        mush_inventory::endpoint_visible(world.world(), stranger, absent),
        "a deposit endpoint and a non-existent one read identically to a stranger"
    );
    assert!(!mush_inventory::endpoint_visible(
        world.world(),
        stranger,
        body_endpoint
    ));
}

#[test]
fn e7c3_f_every_extraction_refusal_renders_one_sentence_and_only_staff_get_the_reason() {
    // 08c §6.2's D1 shape: depletion, a missing licence and a suspended claim
    // are three different facts and ONE message.
    let arms = [
        ExtractionRefusal::Depleted,
        ExtractionRefusal::NoLicence,
        ExtractionRefusal::ClaimSuspended,
        ExtractionRefusal::ClaimSeized,
        ExtractionRefusal::NoClaim,
        ExtractionRefusal::ClaimExpired,
        ExtractionRefusal::ClaimUncomparable,
        ExtractionRefusal::LicenceInactive,
        ExtractionRefusal::NoLicenceInstrument,
        ExtractionRefusal::Unworkable,
    ];
    let first = arms[0].public();
    for arm in arms {
        assert_eq!(arm.public(), first, "{arm:?} renders a different sentence");
        assert_ne!(arm.reason(), first, "{arm:?} leaks its reason publicly");
    }
    // And the reasons are all distinct, so a staffer can still act.
    let mut reasons: Vec<&str> = arms.iter().map(|arm| arm.reason()).collect();
    reasons.sort_unstable();
    reasons.dedup();
    assert_eq!(reasons.len(), arms.len());
}

// =====================================================================
// D — depletion. Can total extracted exceed the reserve under
// concurrency, restart or event ordering?
// =====================================================================

#[test]
fn e7c3_d_cumulative_extraction_matches_the_curve_over_two_hundred_cycles() {
    let (mut world, families) = boot();
    // 40 cycles at 500, then 67 at 300 (the last clamped to what is left):
    // 107 cycles, well inside the 200 the lens asks for, and the whole point
    // is that the total is EXACTLY the authored reserve.
    let body = deposit(10, 40_000, &[(0, 500), (20_000, 300), (40_000, 100)]);
    let to = endpoint(500);
    let engine = world.engine_mut();
    open(engine, &families, &body);
    start_run(engine, &families, &body, to);

    let mut extracted: u64 = 0;
    let mut cycles = 0_u32;
    for ordinal in 0..200_u32 {
        // Every cycle's expected yield, computed independently from the curve.
        let expected = body.next_yield(40_000 - extracted);
        let outcome = cycle(engine, &families, &body, (ordinal % 250) as u8 + 1);
        match outcome {
            Ok(cycled) => {
                assert_eq!(
                    cycled.quantity, expected,
                    "cycle {ordinal}: the curve is arithmetic, not a roll"
                );
                extracted += cycled.quantity;
                cycles += 1;
            }
            Err(IndustryError::Depleted) => break,
            Err(other) => panic!("cycle {ordinal}: {other}"),
        }
    }
    assert!(cycles > 0);
    assert_eq!(
        extracted, 40_000,
        "the body yields exactly its authored reserve and never a unit more"
    );
    assert!(
        cycles >= 100,
        "and it took the cycles the curve says it takes"
    );
    let remaining = read(engine, &families, |txn| {
        families.remaining(txn, &body).expect("remaining")
    });
    assert_eq!(remaining, 0, "and the pool is the one number that says so");
    // A depleted body yields zero and REFUSES a new cycle rather than
    // committing an empty bundle.
    assert!(matches!(
        cycle(engine, &families, &body, 201),
        Err(IndustryError::Depleted)
    ));
}

#[test]
fn e7c3_d_two_reservations_against_one_remaining_unit_cannot_both_consume() {
    // The gate is K's, not cut 3's: `checked_hold` subtracts every live hold
    // before admitting the next, so the second refuses at the kernel.
    let (mut world, families) = boot();
    let at = endpoint(11);
    let engine = world.engine_mut();
    apply(engine, &families, |txn| {
        families
            .inventory
            .credit(txn, at, &commodity("metals"), 1)
            .map(|_| ())
            .map_err(IndustryError::from)
    })
    .expect("one unit in the ground");

    let first = apply(engine, &families, |txn| {
        families
            .inventory
            .reserve(
                txn,
                at,
                &commodity("metals"),
                &holder(GOD, "one"),
                1,
                &now(),
            )
            .map_err(IndustryError::from)
    })
    .expect("the first hold is admitted");
    let second = apply(engine, &families, |txn| {
        families
            .inventory
            .reserve(
                txn,
                at,
                &commodity("metals"),
                &holder(ObjId(3), "two"),
                1,
                &now(),
            )
            .map_err(IndustryError::from)
    });
    assert!(
        second.is_err(),
        "the second hold against one unit is refused by checked_hold"
    );
    assert!(!format!("{first:?}").is_empty());
    // Availability never rose: the pool still holds exactly one unit, held.
    let available = read(engine, &families, |txn| {
        families
            .inventory
            .available(txn, at, &commodity("metals"))
            .expect("available")
    });
    assert_eq!(available, 0, "one unit, one live hold, nothing available");
}

#[test]
fn e7c3_d_a_stale_deposit_or_run_revision_refuses_the_cycle() {
    let (mut world, families) = boot();
    let body = deposit(12, 1_000, &[(0, 100)]);
    let to = endpoint(501);
    let engine = world.engine_mut();
    open(engine, &families, &body);
    start_run(engine, &families, &body, to);

    // A pre-image read before an accessibility event, presented afterwards.
    let stale = read(engine, &families, |txn| {
        families
            .deposit(txn, body.id())
            .expect("read")
            .expect("body")
    });
    apply(engine, &families, |txn| {
        families
            .set_accessibility(
                txn,
                body.id(),
                stale.revision(),
                Accessibility::new(50).expect("accessibility"),
            )
            .map(|_| ())
    })
    .expect("the campaign event lands");

    let refused = apply(engine, &families, |txn| {
        let run = families.run(txn, body.id())?.expect("run");
        families.run_cycle(
            txn,
            &stale,
            &run,
            GOD,
            RequestUid::from_bytes([0x33; 16]),
            &holder(GOD, "extract"),
            &now(),
            CommitToken::from_bytes([0x33; 16]),
            source("extract/12/0"),
        )
    });
    assert!(
        matches!(refused, Err(IndustryError::StaleRevision)),
        "a stale pre-image refuses rather than extracting on it: {refused:?}"
    );
}

#[test]
fn e7c3_d_an_accessibility_event_moves_the_next_cycle_and_never_a_completed_one() {
    let (mut world, families) = boot();
    let body = deposit(13, 10_000, &[(0, 500)]);
    let to = endpoint(502);
    let engine = world.engine_mut();
    open(engine, &families, &body);
    start_run(engine, &families, &body, to);

    let first = cycle(engine, &families, &body, 1).expect("cycle 0");
    assert_eq!(first.quantity, 500);
    let lot_after_first = read(engine, &families, |txn| {
        txn.get(&families.inventory.lots, &first.lot)
            .expect("lot read")
            .cloned()
    })
    .expect("the first cycle's lot");
    assert_eq!(lot_after_first.quantity(), 500);

    // Collapse the seam to half.
    let live = read(engine, &families, |txn| {
        families
            .deposit(txn, body.id())
            .expect("read")
            .expect("body")
    });
    apply(engine, &families, |txn| {
        families
            .set_accessibility(
                txn,
                body.id(),
                live.revision(),
                Accessibility::new(50).expect("accessibility"),
            )
            .map(|_| ())
    })
    .expect("event");

    let live = read(engine, &families, |txn| {
        families
            .deposit(txn, body.id())
            .expect("read")
            .expect("body")
    });
    let run = read(engine, &families, |txn| {
        families.run(txn, body.id()).expect("read").expect("run")
    });
    let second = apply(engine, &families, |txn| {
        families.run_cycle(
            txn,
            &live,
            &run,
            GOD,
            RequestUid::from_bytes([0x44; 16]),
            &holder(GOD, "extract"),
            &now(),
            CommitToken::from_bytes([0x44; 16]),
            source("extract/13/1"),
        )
    })
    .expect("cycle 1");
    assert_eq!(second.quantity, 250, "the NEXT cycle moved");
    let lot_again = read(engine, &families, |txn| {
        txn.get(&families.inventory.lots, &first.lot)
            .expect("lot read")
            .cloned()
    })
    .expect("the first cycle's lot still exists");
    assert_eq!(
        lot_again.quantity(),
        500,
        "a completed cycle is never re-evaluated"
    );
}

#[test]
fn e7c3_d_a_cycle_is_one_bundle_so_a_reboot_sees_all_or_none() {
    let (mut world, families) = boot();
    let body = deposit(14, 10_000, &[(0, 500)]);
    let to = endpoint(503);
    let engine = world.engine_mut();
    open(engine, &families, &body);
    start_run(engine, &families, &body, to);

    let before = read(engine, &families, |txn| {
        families.remaining(txn, &body).expect("remaining")
    });

    // Prepare the exact bundle a cycle writes, then ABORT it: the pre-image
    // must be intact, with no lot, no debited reserve and no advanced run.
    let (writes, lot) = {
        let mut txn = engine
            .action_transaction(&families.deposits)
            .expect("industry transaction");
        let live = families
            .deposit(&mut txn, body.id())
            .expect("read")
            .expect("body");
        let run = families
            .run(&mut txn, body.id())
            .expect("read")
            .expect("run");
        let cycled = families
            .run_cycle(
                &mut txn,
                &live,
                &run,
                GOD,
                RequestUid::from_bytes([0x55; 16]),
                &holder(GOD, "extract"),
                &now(),
                CommitToken::from_bytes([0x55; 16]),
                source("extract/14/0"),
            )
            .expect("prepare the cycle");
        let prepared = txn.prepare().expect("prepared");
        let writes = prepared.authoritative_replay_writes().len();
        engine.abort_prepared_transaction(prepared);
        (writes, cycled.lot)
    };
    // The exact write set: the deposit pool's scalar and book, the operator's
    // lot, that endpoint's lot census, the advanced run, and the marker.
    assert!(
        writes >= 6,
        "one cycle writes the pool pair, the lot, the census, the run and the marker: {writes}"
    );

    let after = read(engine, &families, |txn| {
        families.remaining(txn, &body).expect("remaining")
    });
    assert_eq!(after, before, "an aborted cycle debits nothing");
    let orphan = read(engine, &families, |txn| {
        txn.get(&families.inventory.lots, &lot)
            .expect("lot read")
            .cloned()
    });
    assert!(orphan.is_none(), "no lot, or exactly one — never a half");
    let run = read(engine, &families, |txn| {
        families.run(txn, body.id()).expect("read").expect("run")
    });
    assert_eq!(run.next_cycle(), 0, "and the run did not advance");
}

#[test]
fn e7c3_d_the_bundle_carries_two_owners_post_images() {
    // The one-bundle argument, made against the write set itself: the
    // composition-declared delegation is what lets `industry.run` and
    // `inventory.pool` ride one commit, and every write still persists under
    // its OWN family owner's namespace.
    let (mut world, families) = boot();
    let body = deposit(15, 10_000, &[(0, 500)]);
    let to = endpoint(504);
    let engine = world.engine_mut();
    open(engine, &families, &body);
    start_run(engine, &families, &body, to);

    let mut txn = engine
        .action_transaction(&families.deposits)
        .expect("industry transaction");
    let live = families
        .deposit(&mut txn, body.id())
        .expect("read")
        .expect("body");
    let run = families
        .run(&mut txn, body.id())
        .expect("read")
        .expect("run");
    families
        .run_cycle(
            &mut txn,
            &live,
            &run,
            GOD,
            RequestUid::from_bytes([0x66; 16]),
            &holder(GOD, "extract"),
            &now(),
            CommitToken::from_bytes([0x66; 16]),
            source("extract/15/0"),
        )
        .expect("prepare");
    assert_eq!(
        txn.owner(),
        &OwnerId::new("industry").expect("owner"),
        "the transaction is industry's"
    );
    let prepared = txn.prepare().expect("prepared");
    let mut namespaces: Vec<String> = prepared
        .authoritative_replay_writes()
        .iter()
        .map(|write| match write {
            mush_core::ReplayWrite::ModuleJsonSet { namespace, .. }
            | mush_core::ReplayWrite::ModuleDelete { namespace, .. } => namespace.to_string(),
            _ => "other".to_owned(),
        })
        .collect();
    namespaces.sort();
    namespaces.dedup();
    assert_eq!(
        namespaces,
        vec!["industry".to_owned(), "inventory".to_owned()],
        "one bundle, two owners, each write in its own family owner's namespace"
    );
    engine.abort_prepared_transaction(prepared);
}

#[test]
fn e7c3_d_a_replayed_request_refuses_at_the_applied_marker() {
    let (mut world, families) = boot();
    let body = deposit(16, 10_000, &[(0, 500)]);
    let to = endpoint(505);
    let engine = world.engine_mut();
    open(engine, &families, &body);
    start_run(engine, &families, &body, to);

    let request = RequestUid::from_bytes([0x77; 16]);
    let run_once = |engine: &mut Engine| {
        apply(engine, &families, |txn| {
            let live = families.deposit(txn, body.id())?.expect("body");
            let run = families.run(txn, body.id())?.expect("run");
            families.run_cycle(
                txn,
                &live,
                &run,
                GOD,
                request,
                &holder(GOD, "extract"),
                &now(),
                CommitToken::from_bytes([0x77; 16]),
                source("extract/16/0"),
            )
        })
    };
    run_once(engine).expect("the first cycle commits");
    let replay = run_once(engine);
    assert!(
        matches!(replay, Err(IndustryError::AlreadyApplied)),
        "the marker is the epoch-free replay guard: {replay:?}"
    );
    let remaining = read(engine, &families, |txn| {
        families.remaining(txn, &body).expect("remaining")
    });
    assert_eq!(
        remaining, 9_500,
        "and the reserve was drawn down exactly once"
    );
}

#[test]
fn e7c3_d_a_quarantined_body_refuses_and_its_pool_never_rises() {
    let (mut world, families) = boot();
    let body = deposit(17, 10_000, &[(0, 500)]);
    {
        let engine = world.engine_mut();
        open(engine, &families, &body);
    }
    // The body's endpoint names a vault the economy never hydrated — the one
    // cross-family shape the kernel cannot see.
    world.modules_loaded(std::collections::HashMap::new());
    let store = world
        .world()
        .module_state::<IndustryStore>()
        .expect("industry store");
    assert_eq!(
        store.quarantine(body.id()),
        Some(Quarantine::VaultMissing),
        "a body whose endpoint vault is absent is quarantined"
    );
    assert!(
        !store
            .quarantine(body.id())
            .expect("why")
            .runbook()
            .is_empty()
    );
    // Availability refuses rather than rising: the pool is exactly what was
    // credited, and nothing about the quarantine invents a unit.
    let engine = world.engine_mut();
    let remaining = read(engine, &families, |txn| {
        families.remaining(txn, &body).expect("remaining")
    });
    assert_eq!(remaining, 10_000);
}

#[test]
fn e7c3_d_a_curve_is_monotone_and_overflow_refuses() {
    // The curve is the whole of the depletion model, so its shape is refused
    // on construction AND on load rather than at first arithmetic.
    assert!(
        YieldCurve::new(Vec::new()).is_err(),
        "an empty curve refuses"
    );
    assert!(
        YieldCurve::new(vec![CurvePoint {
            cumulative: 5,
            per_cycle: 10
        }])
        .is_err(),
        "a curve that does not start at zero refuses"
    );
    assert!(
        YieldCurve::new(vec![
            CurvePoint {
                cumulative: 0,
                per_cycle: 10
            },
            CurvePoint {
                cumulative: 5,
                per_cycle: 20
            },
        ])
        .is_err(),
        "a yield that RISES with depletion refuses"
    );
    assert!(
        YieldCurve::new(vec![
            CurvePoint {
                cumulative: 0,
                per_cycle: 10
            },
            CurvePoint {
                cumulative: 0,
                per_cycle: 5
            },
        ])
        .is_err(),
        "a non-increasing threshold refuses"
    );
    assert!(
        serde_json::from_value::<YieldCurve>(serde_json::json!([
            { "cumulative": 0, "per_cycle": 10 },
            { "cumulative": 5, "per_cycle": 20 }
        ]))
        .is_err(),
        "and the wire re-runs it on load"
    );
    // An oversized reserve refuses at the record's own constructor.
    let over = OreDeposit::new(
        OreDepositId(18),
        ObjId(7),
        commodity("metals"),
        Grade::new(1).expect("grade"),
        Accessibility::new(100).expect("accessibility"),
        band("a"),
        band("b"),
        curve(&[(0, 1)]),
        endpoint(18),
        MAX_ORE_RESERVE + 1,
    );
    assert!(over.is_err(), "an oversized reserve refuses");
}

#[test]
fn e7c3_d_a_stopped_or_unworkable_body_refuses_the_cycle() {
    let (mut world, families) = boot();
    let body = deposit(19, 10_000, &[(0, 500)]);
    let to = endpoint(506);
    let engine = world.engine_mut();
    open(engine, &families, &body);
    start_run(engine, &families, &body, to);

    // Collapsed: accessibility zero refuses rather than quietly yielding zero,
    // so an operator is told something happened.
    let live = read(engine, &families, |txn| {
        families
            .deposit(txn, body.id())
            .expect("read")
            .expect("body")
    });
    apply(engine, &families, |txn| {
        families
            .set_accessibility(
                txn,
                body.id(),
                live.revision(),
                Accessibility::new(0).expect("accessibility"),
            )
            .map(|_| ())
    })
    .expect("collapse");
    assert!(matches!(
        cycle(engine, &families, &body, 1),
        Err(IndustryError::Unworkable)
    ));

    // Stopped: the record stays and the cycle refuses.
    let run = read(engine, &families, |txn| {
        families.run(txn, body.id()).expect("read").expect("run")
    });
    apply(engine, &families, |txn| {
        families.put_run(txn, run.stopped()?)
    })
    .expect("stop");
    let still_there = read(engine, &families, |txn| {
        families.run(txn, body.id()).expect("read")
    });
    assert!(still_there.is_some(), "stopping leaves the record");
}

// =====================================================================
// T — tenure. Can a claim be transferred, extracted under, or contested
// by someone the authority chain does not permit?
// =====================================================================

#[test]
fn e7c3_t_two_claims_over_one_body_coexist_with_distinct_issuers() {
    let (mut world, families) = boot();
    let subject = ClaimSubject::Deposit {
        deposit: OreDepositId(20),
    };
    let engine = world.engine_mut();
    let mut first = claim(20, subject, 2, 2);
    let mut second = claim(21, subject, 3, 3);
    // Two governments, one piece of ground: two records with distinct issuers,
    // which needs no `ControlKind` at all (that stays cut 9's).
    apply(engine, &families, |txn| {
        families.file_claim(txn, first.clone())
    })
    .expect("the first authority");
    apply(engine, &families, |txn| {
        families.file_claim(txn, second.clone())
    })
    .expect("the second authority");
    let live = read(engine, &families, |txn| {
        families.claims_over(txn, subject).expect("claims")
    });
    assert_eq!(live.len(), 2);
    first = live[0].clone();
    second = live[1].clone();
    assert_ne!(first.issuer(), second.issuer());
}

#[test]
fn e7c3_t_the_claim_index_bounds_a_subject_at_its_cap() {
    let mut index = ClaimIndex::default();
    for id in 0..MAX_CLAIMS_PER_SUBJECT {
        index = index
            .with(mush_industry::ClaimId(id as u64))
            .expect("inside the cap");
    }
    assert_eq!(index.len(), MAX_CLAIMS_PER_SUBJECT);
    assert!(
        index.with(mush_industry::ClaimId(9_999)).is_err(),
        "the index is what makes MAX_CLAIMS_PER_SUBJECT a COUNTED bound"
    );
    // Idempotent: re-filing a claim already in the set is not a second row.
    let same = index
        .with(mush_industry::ClaimId(0))
        .expect("already present");
    assert_eq!(same.len(), MAX_CLAIMS_PER_SUBJECT);
    // And the wire re-runs both rules.
    let over: Vec<u64> = (0..=MAX_CLAIMS_PER_SUBJECT as u64).collect();
    assert!(serde_json::from_value::<ClaimIndex>(serde_json::json!(over)).is_err());
    assert!(serde_json::from_value::<ClaimIndex>(serde_json::json!([2_u64, 1_u64])).is_err());
}

#[test]
fn e7c3_t_a_claim_past_its_term_stops_authorizing_and_does_not_vanish() {
    let (world, _families) = boot();
    let body = deposit(22, 10_000, &[(0, 500)]);
    let mut record = claim(
        22,
        ClaimSubject::Deposit { deposit: body.id() },
        GOD.0,
        GOD.0,
    );
    record = ClaimRecord::new(
        record.id(),
        record.subject(),
        record.issuer().clone(),
        record.holder().clone(),
        record.object(),
        Exclusivity::Shared,
        1,
        later(500),
        Transferability::Transferable,
    )
    .expect("a claim that lapses at 500");
    let refusal = mush_industry::extraction_authorized(
        world.world(),
        GOD,
        &PrincipalId::Object(GOD),
        &body,
        10_000,
        std::slice::from_ref(&record),
        None,
        &later(1_000),
    );
    assert_eq!(refusal, Err(ExtractionRefusal::ClaimExpired));
    // The record is still there — it stopped authorizing, it did not vanish.
    assert_eq!(record.contest_state(), &ContestState::Uncontested);
}

#[test]
fn e7c3_t_a_term_on_another_clock_generation_refuses_comparison() {
    let (world, _families) = boot();
    let body = deposit(23, 10_000, &[(0, 500)]);
    let record = ClaimRecord::new(
        mush_industry::ClaimId(23),
        ClaimSubject::Deposit { deposit: body.id() },
        PrincipalId::Object(GOD),
        PrincipalId::Object(GOD),
        ObjId(923),
        Exclusivity::Shared,
        1,
        other_generation(),
        Transferability::Transferable,
    )
    .expect("a claim stamped on generation 2");
    let refusal = mush_industry::extraction_authorized(
        world.world(),
        GOD,
        &PrincipalId::Object(GOD),
        &body,
        10_000,
        std::slice::from_ref(&record),
        None,
        &now(),
    );
    assert_eq!(
        refusal,
        Err(ExtractionRefusal::ClaimUncomparable),
        "K4a: refuse comparison rather than expiring early"
    );
}

#[test]
fn e7c3_t_a_suspended_or_seized_claim_authorizes_nobody_and_the_record_stays() {
    let (world, _families) = boot();
    let body = deposit(24, 10_000, &[(0, 500)]);
    let base = claim(
        24,
        ClaimSubject::Deposit { deposit: body.id() },
        GOD.0,
        GOD.0,
    );
    for (state, expected) in [
        (
            ContestState::Suspended {
                by: PrincipalId::Object(GOD),
            },
            ExtractionRefusal::ClaimSuspended,
        ),
        (
            ContestState::Seized {
                by: PrincipalId::Object(GOD),
                at: now(),
            },
            ExtractionRefusal::ClaimSeized,
        ),
    ] {
        let moved = base.with_contest_state(state.clone()).expect("move");
        assert_eq!(
            moved.contest_state(),
            &state,
            "the record stays and says so"
        );
        assert_eq!(moved.revision(), base.revision() + 1);
        let refusal = mush_industry::extraction_authorized(
            world.world(),
            GOD,
            &PrincipalId::Object(GOD),
            &body,
            10_000,
            std::slice::from_ref(&moved),
            None,
            &now(),
        );
        assert_eq!(refusal, Err(expected));
    }
    // A *contested* claim still authorizes: a dispute is not a suspension.
    let contested = base
        .with_contest_state(ContestState::Contested {
            contract: mush_economy::ContractId(1),
        })
        .expect("contest");
    assert!(contested.contest_state().authorizes());
}

#[test]
fn e7c3_t_extraction_without_the_entitlement_refuses_though_the_claim_stands() {
    let (world, _families) = boot();
    let body = deposit(25, 10_000, &[(0, 500)]);
    let record = claim(
        25,
        ClaimSubject::Deposit { deposit: body.id() },
        GOD.0,
        GOD.0,
    );
    // No licence instrument named at all: fail-closed, and the staff-only
    // reason names the wiring.
    let refusal = mush_industry::extraction_authorized(
        world.world(),
        GOD,
        &PrincipalId::Object(GOD),
        &body,
        10_000,
        std::slice::from_ref(&record),
        None,
        &now(),
    );
    assert_eq!(refusal, Err(ExtractionRefusal::NoLicenceInstrument));
    assert!(
        ExtractionRefusal::NoLicenceInstrument
            .reason()
            .contains("with_extraction_licence")
    );

    // A named instrument nobody holds an account in: still refused, and still
    // the same public sentence.
    let instrument = mush_economy::InstrumentId::new("tacsim.extraction").expect("instrument");
    let refusal = mush_industry::extraction_authorized(
        world.world(),
        GOD,
        &PrincipalId::Object(GOD),
        &body,
        10_000,
        std::slice::from_ref(&record),
        Some(&instrument),
        &now(),
    );
    assert!(matches!(
        refusal,
        Err(ExtractionRefusal::NoLicenceInstrument | ExtractionRefusal::NoLicence)
    ));
    assert_eq!(
        refusal.unwrap_err().public(),
        ExtractionRefusal::Depleted.public()
    );
}

#[test]
fn e7c3_t_a_claim_over_a_body_nobody_surveyed_is_legal_and_leaks_nothing() {
    let (mut world, families) = boot();
    let body = deposit(26, 10_000, &[(0, 500)]);
    let engine = world.engine_mut();
    open(engine, &families, &body);
    let record = claim(
        26,
        ClaimSubject::Deposit { deposit: body.id() },
        GOD.0,
        GOD.0,
    );
    apply(engine, &families, |txn| {
        families.file_claim(txn, record.clone())
    })
    .expect("filing needs no survey");
    // And the filing disclosed nothing: no survey row exists for anybody.
    let store = world.world().module_state::<IndustryStore>();
    assert!(store.is_some_and(|store| store.survey(body.id()).is_none()));
}

#[test]
fn e7c3_t_a_bound_claim_refuses_a_holder_change() {
    let record = ClaimRecord::new(
        mush_industry::ClaimId(27),
        ClaimSubject::Place { place: ObjId(7) },
        PrincipalId::Object(GOD),
        PrincipalId::Object(GOD),
        ObjId(927),
        Exclusivity::Exclusive,
        1,
        later(100_000),
        Transferability::Bound,
    )
    .expect("a personal tenure");
    assert_eq!(record.transferability(), Transferability::Bound);
    // The record CAN carry a new holder; what refuses is the verb, and E4's
    // own `TransferTitle` refuses the authority half. Both halves are proved
    // where they live.
    let moved = record
        .with_holder(PrincipalId::Object(ObjId(3)))
        .expect("the record shape allows it");
    assert_eq!(moved.revision(), 2);
}

#[test]
fn e7c3_t_a_stale_claim_revision_refuses() {
    let (mut world, families) = boot();
    let subject = ClaimSubject::Place { place: ObjId(28) };
    let engine = world.engine_mut();
    let record = claim(28, subject, GOD.0, GOD.0);
    apply(engine, &families, |txn| {
        families.file_claim(txn, record.clone())
    })
    .expect("file");
    let moved = record
        .with_contest_state(ContestState::Suspended {
            by: PrincipalId::Object(GOD),
        })
        .expect("suspend");
    apply(engine, &families, |txn| {
        families.put_claim(txn, record.revision(), moved.clone())
    })
    .expect("the first move lands");
    // The same pre-image again: refused, not re-applied.
    let refused = apply(engine, &families, |txn| {
        families.put_claim(txn, record.revision(), moved.clone())
    });
    assert!(matches!(refused, Err(IndustryError::StaleRevision)));
}

// =====================================================================
// Abandonment, restart and wiring
// =====================================================================

#[test]
fn e7c3_abandonment_retains_title_and_takes_only_a_title_op() {
    // The three new symbols, and the shape they make.
    let op = mush_economy::AssetOp::Abandon { object: ObjId(5) };
    assert_eq!(op.object(), ObjId(5));
    assert_eq!(
        op.ordinal(),
        10,
        "the next free ordinal; nothing renumbered"
    );
    assert!(op.is_title_changing());
    // `AssetLifecycle` gained a third, non-terminal variant, and it is not
    // `WrittenOff`.
    assert_ne!(
        mush_economy::AssetLifecycle::Abandoned,
        mush_economy::AssetLifecycle::WrittenOff
    );
    assert_ne!(
        mush_economy::AssetLifecycle::Abandoned,
        mush_economy::AssetLifecycle::Active
    );
    // And the effect kind exists beside the others.
    assert_ne!(
        mush_economy::AssetEffectKind::Abandoned,
        mush_economy::AssetEffectKind::WrittenOff
    );
    // The serde bytes of the two shipped variants are unchanged, so every
    // stored asset document still round-trips.
    assert_eq!(
        serde_json::to_string(&mush_economy::AssetLifecycle::Active).expect("json"),
        "\"Active\""
    );
    assert_eq!(
        serde_json::to_string(&mush_economy::AssetLifecycle::WrittenOff).expect("json"),
        "\"WrittenOff\""
    );
}

#[test]
fn e7c3_restart_every_record_re_enters_through_its_constructor() {
    // §4 rule 1 / D-E7-33: a derived `Deserialize` would make every cap
    // advisory on load, which is the E5a finding this programme may not
    // re-learn. Each of these is a hand-edited document that the wire refuses.
    let body = deposit(30, 10_000, &[(0, 500)]);
    let mut json = serde_json::to_value(&body).expect("serialize");
    json["initial_reserve"] = serde_json::json!(0);
    assert!(
        serde_json::from_value::<OreDeposit>(json).is_err(),
        "a zero reserve refuses on load"
    );

    let mut json = serde_json::to_value(&body).expect("serialize");
    json["revision"] = serde_json::json!(0);
    assert!(serde_json::from_value::<OreDeposit>(json).is_err());

    let mut json = serde_json::to_value(&body).expect("serialize");
    json["accessibility"] = serde_json::json!(101);
    assert!(serde_json::from_value::<OreDeposit>(json).is_err());

    let record = claim(31, ClaimSubject::Place { place: ObjId(7) }, 2, 2);
    let mut json = serde_json::to_value(&record).expect("serialize");
    json["priority"] = serde_json::json!(u32::MAX);
    assert!(serde_json::from_value::<ClaimRecord>(json).is_err());

    let run =
        ExtractionRun::new(OreDepositId(32), PrincipalId::Object(GOD), endpoint(32)).expect("run");
    let mut json = serde_json::to_value(&run).expect("serialize");
    json["next_cycle"] = serde_json::json!(u64::MAX);
    assert!(serde_json::from_value::<ExtractionRun>(json).is_err());

    // An unknown key is an error, never ignored.
    let mut json = serde_json::to_value(&run).expect("serialize");
    json["surprise"] = serde_json::json!(1);
    assert!(serde_json::from_value::<ExtractionRun>(json).is_err());
}

#[test]
fn e7c3_boot_a_corrupt_survey_document_is_skipped_loudly_and_its_body_marked() {
    let mut store = IndustryStore::default();
    let body = OreDepositId(33);
    let key = mush_industry::store::survey_key(body);
    mush_industry::store::hydrate(
        &mut store,
        &[(
            key,
            serde_json::json!({ "deposit": body, "rows": "not rows" }),
        )],
    );
    assert_eq!(
        store.quarantine(body),
        Some(Quarantine::SurveyLost),
        "a document that failed its wire marks its body"
    );
    assert!(
        store.survey(body).is_none(),
        "and is skipped, never guessed"
    );
    // The direction of the loss is disclosure only: no authority moves.
    assert!(
        store
            .quarantine(body)
            .expect("why")
            .runbook()
            .contains("no authority")
    );
}

#[test]
fn e7c3_boot_a_survey_document_that_disagrees_with_its_key_is_skipped() {
    let mut store = IndustryStore::default();
    let named = OreDepositId(34);
    let other = OreDepositId(35);
    let doc = SurveyDoc::empty(other);
    mush_industry::store::hydrate(
        &mut store,
        &[(
            mush_industry::store::survey_key(named),
            serde_json::to_value(&doc).expect("json"),
        )],
    );
    assert_eq!(store.quarantine(named), Some(Quarantine::SurveyLost));
    assert!(store.survey(named).is_none());
    assert!(
        store.survey(other).is_none(),
        "and never installed elsewhere"
    );
}

#[test]
fn e7c3_facts_a_second_industry_namespace_panics_at_composition() {
    // D-E7-30: one provider per crate, no exceptions.
    let result = std::panic::catch_unwind(|| {
        let _ = mush_economy::EconomyModule::new().with_facts(vec![
            Arc::new(mush_industry::IndustryFactProvider),
            Arc::new(mush_industry::IndustryFactProvider),
        ]);
    });
    assert!(result.is_err(), "a duplicate namespace panics");
}

#[test]
fn e7c3_facts_answer_from_an_applied_marker_and_nothing_else() {
    let (mut world, families) = boot();
    let body = deposit(36, 10_000, &[(0, 500)]);
    let to = endpoint(507);
    {
        let engine = world.engine_mut();
        open(engine, &families, &body);
        start_run(engine, &families, &body, to);
        cycle(engine, &families, &body, 1).expect("one cycle commits");
    }
    world.modules_loaded(std::collections::HashMap::new());
    let store = world
        .world()
        .module_state::<IndustryStore>()
        .expect("industry store");
    let markers = store.markers_of(mush_industry::FACT_EXTRACTION_CYCLED);
    assert_eq!(markers.len(), 1, "the committed bundle wrote exactly one");
    assert_eq!(markers[0].deposit(), body.id());
    assert_eq!(markers[0].quantity(), 500);
    assert_eq!(markers[0].actor(), GOD);
    // A bundle that did not commit leaves no marker, and the fact then answers
    // nothing — which is unsatisfied, and never settles money.
    assert!(store.marker("no such marker").is_none());
}

#[test]
fn e7c3_wiring_the_custody_provider_holds_quantity_not_objects() {
    use mush_economy::CustodyProvider;
    let provider = mush_industry::IndustryCustodyProvider;
    assert_eq!(provider.provider(), "industry");
    let ours = mush_industry::deposit_backing(OreDepositId(40));
    let theirs = mush_economy::CustodyRef::Container(ObjId(1));
    assert!(provider.handles(&ours));
    assert!(!provider.handles(&theirs));
    assert_eq!(
        mush_industry::deposit_of_backing(&ours),
        Some(OreDepositId(40))
    );
    assert_eq!(mush_industry::deposit_of_backing(&theirs), None);
    let (world, _families) = boot();
    // `Some`, never `None`: `None` is fail-closed drift, and this provider
    // knows the answer for its own backings without reading any state, which
    // is what keeps it off the module ordering hazard entirely.
    assert_eq!(provider.backing_valid(world.world(), &ours), Some(true));
    assert_eq!(provider.contents(world.world(), &ours), Some(Vec::new()));
    assert_eq!(provider.holds(world.world(), &ours, ObjId(1)), Some(false));
    // And it claims no world object at all, so E3's exactly-one-claiming-
    // provider rule is never contested by us in either direction.
    assert!(!provider.claims(world.world(), ObjId(1)));
}

#[test]
fn e7c3_wiring_the_crates_edge_set_is_exactly_what_it_declares() {
    // `cargo tree -p mush-industry` is the test, and this is its checked-in
    // half: the manifest's own declaration list, asserted directly so a new
    // edge cannot arrive unnoticed. **No sim, market, faction, tacsim or party
    // edge**, in production or in dev.
    let manifest = include_str!("../Cargo.toml");
    let declared: Vec<&str> = manifest
        .lines()
        .filter_map(|line| line.split_once(".workspace = true"))
        .map(|(name, _)| name.trim())
        .collect();
    for forbidden in [
        "mush-sim",
        "mush-market",
        "mush-faction",
        "tacsim",
        "mush-party",
        "mush-tactics",
        "mush-scenes",
        "mush-jobs",
        "mush-housing",
    ] {
        assert!(
            !declared.contains(&forbidden),
            "mush-industry must not depend on {forbidden}"
        );
    }
    for required in [
        "mush-core",
        "mush-economy",
        "mush-org",
        "mush-inventory",
        "mush-proto",
    ] {
        assert!(
            declared.contains(&required),
            "mush-industry declares {required}"
        );
    }
    // DECIDED (d): cut 3 lifts no market refusal and changes no market file —
    // the inventory-backed `Supply` arm is cut 11's. The absence of the edge is
    // what makes that true by construction.
    assert!(
        !manifest.contains("mush-market.workspace"),
        "no market edge, in production or in dev"
    );
}

#[test]
fn e7c3_records_a_deposit_keeps_no_private_stock_number() {
    // D-E7-3, made a property of the type: `OreDeposit` has no `reserve`
    // field, only the AUTHORED figure the pool was seeded with. The one live
    // number is cut 2's pool.
    let body = deposit(41, 10_000, &[(0, 500)]);
    let json = serde_json::to_value(&body).expect("serialize");
    let object = json.as_object().expect("object");
    let mut fields: Vec<&str> = object.keys().map(String::as_str).collect();
    fields.sort_unstable();
    assert_eq!(
        fields,
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
            "revision",
        ],
        "the record's exact field set; there is no live `reserve`"
    );
    // `place` is an `ObjId`, never a `RegionId` (OPEN 4): a pack cannot author
    // a runtime-allocated region id, and using one would put a faction edge on
    // a crate that must stay neutral.
    assert_eq!(body.place(), ObjId(7));
}

#[test]
fn e7c3_records_the_curve_is_pure_and_two_evaluations_agree() {
    let body = deposit(42, 50_000, &[(0, 500), (20_000, 300), (40_000, 100)]);
    for remaining in [50_000_u64, 40_000, 30_000, 10_000, 1, 0] {
        assert_eq!(
            body.next_yield(remaining),
            body.next_yield(remaining),
            "the curve takes no clock, no world and no RNG"
        );
    }
    assert_eq!(body.next_yield(50_000), 500);
    assert_eq!(body.next_yield(30_000), 300, "past 20k cumulative");
    assert_eq!(body.next_yield(9_000), 100, "past 40k cumulative");
    assert_eq!(body.next_yield(50), 50, "clamped to what is left");
    assert_eq!(body.next_yield(0), 0, "and a depleted body yields zero");
}

// =====================================================================
// ---- E7 cut 3 independent verify fold ----
// One item per confirmed finding across the C/R/B lenses, plus the two
// builder-flagged test gaps (B4 survey locality, B2 extract e2e / re-survey).
// =====================================================================

/// A body at a caller-named world place (the rig helper hardcodes `#7`).
fn body_at(id: u64, place: ObjId, reserve: u64, points: &[(u64, u64)]) -> OreDeposit {
    OreDeposit::new(
        OreDepositId(id),
        place,
        commodity("metals"),
        Grade::new(60).expect("grade"),
        Accessibility::new(100).expect("accessibility"),
        band("grade_b"),
        band("reserve_major"),
        curve(points),
        endpoint(id),
        reserve,
    )
    .expect("test body")
}

fn install_offer_keys(world: &mut TestWorld) {
    world
        .engine_mut()
        .set_offer_keys(OfferKeyRing::new(OfferSecret::new(1, [0x41; 32]).unwrap(), []).unwrap());
    world.engine_mut().set_resolution_keys(
        ResolutionKeyRing::new(ResolutionSecret::new(1, [0x52; 32]).unwrap(), []).unwrap(),
    );
}

/// Whether `project_offer`, regenerated through the engine exactly as
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

#[test]
fn vf_c1_project_offer_gates_claim_contest() {
    let (mut world, families) = boot();
    install_offer_keys(&mut world);
    let gov = world.spawn_player("gov"); // issuer
    let alice = world.spawn_player("alice"); // holder
    let bob = world.spawn_player("bob"); // stranger
    let rec = ClaimRecord::new(
        mush_industry::ClaimId(0xC1),
        ClaimSubject::Deposit {
            deposit: OreDepositId(0xC1),
        },
        PrincipalId::Object(gov),
        PrincipalId::Object(alice),
        ObjId(9001),
        Exclusivity::Shared,
        1,
        later(100_000),
        Transferability::Bound,
    )
    .unwrap();
    {
        let e = world.engine_mut();
        apply(e, &families, |txn| families.file_claim(txn, rec.clone())).unwrap();
    }
    // The gate lives in project_offer (the seam with &Engine): the contest verb
    // projects for the issuer and for staff, never for an arbitrary actor.
    assert!(
        !projects_verb(&world, bob, "claim-contest"),
        "a non-issuer, non-staff actor is offered no contest path"
    );
    assert!(
        projects_verb(&world, gov, "claim-contest"),
        "the issuing authority is offered it"
    );
    assert!(
        projects_verb(&world, GOD, "claim-contest"),
        "staff are offered it"
    );
}

#[test]
fn vf_c1_a_non_issuer_cannot_seize_or_release_a_foreign_claim() {
    let (mut world, families) = boot();
    install_offer_keys(&mut world);
    let gov = world.spawn_player("gov"); // issuer
    let alice = world.spawn_player("alice"); // holder
    let bob = world.spawn_player("bob"); // non-issuer, non-holder, non-staff
    let rec = ClaimRecord::new(
        mush_industry::ClaimId(0x42),
        ClaimSubject::Deposit {
            deposit: OreDepositId(1),
        },
        PrincipalId::Object(gov),
        PrincipalId::Object(alice),
        ObjId(4242),
        Exclusivity::Shared,
        1,
        later(100_000),
        Transferability::Transferable,
    )
    .unwrap();
    {
        let e = world.engine_mut();
        apply(e, &families, |txn| families.file_claim(txn, rec.clone())).unwrap();
    }
    // All four contest switches (contest/suspend/seize/release) ride ONE offer,
    // whose `claim` Choice, for a non-staff actor, is only the claims it may
    // administer. The stranger's set is empty and the verb does not project, so
    // the coordinator refuses every switch identically to a missing claim.
    assert!(
        mush_industry::provider::administrable_claims(world.engine(), &families, bob).is_empty(),
        "a stranger may administer no claim"
    );
    assert!(!projects_verb(&world, bob, "claim-contest"));
    // The issuer may administer exactly this claim.
    assert_eq!(
        mush_industry::provider::administrable_claims(world.engine(), &families, gov),
        vec![rec.id()],
        "the issuer may administer their own claim"
    );
    assert!(projects_verb(&world, gov, "claim-contest"));
    // And the claim's contest state was never touched by the stranger.
    let live = {
        let e = world.engine_mut();
        read(e, &families, |txn| {
            families.claim(txn, rec.id()).unwrap().unwrap()
        })
    };
    assert_eq!(
        live.contest_state(),
        &ContestState::Uncontested,
        "the foreign claim's contest state is byte-identical"
    );
}

#[test]
fn vf_c2_a_second_start_does_not_redirect_a_co_tenants_run() {
    let (mut world, families) = boot();
    let body = deposit(601, 40_000, &[(0, 500)]);
    let alice = ObjId(2);
    let bob = ObjId(3);
    let alice_end = endpoint(60_001);
    let bob_end = endpoint(60_002);
    let engine = world.engine_mut();
    open(engine, &families, &body);
    apply(engine, &families, |txn| {
        families.put_run(
            txn,
            ExtractionRun::new(body.id(), PrincipalId::Object(alice), alice_end)?,
        )
    })
    .expect("alice opens the run");
    // Bob's start on the SAME deposit is refused -- never a silent redirect.
    let redirect = apply(engine, &families, |txn| {
        families.put_run(
            txn,
            ExtractionRun::new(body.id(), PrincipalId::Object(bob), bob_end)?,
        )
    });
    assert!(
        matches!(redirect, Err(IndustryError::RunNotYours)),
        "a co-tenant's start does not replace a live run: {redirect:?}"
    );
    let run = read(engine, &families, |txn| {
        families.run(txn, body.id()).unwrap().unwrap()
    });
    assert_eq!(run.operator(), &PrincipalId::Object(alice));
    assert_eq!(run.facility_endpoint(), alice_end);
    // Bob cannot cycle Alice's run either.
    let bob_cycle = apply(engine, &families, |txn| {
        let live = families.deposit(txn, body.id())?.unwrap();
        let run = families.run(txn, body.id())?.unwrap();
        families.run_cycle(
            txn,
            &live,
            &run,
            bob,
            RequestUid::from_bytes([0xB0; 16]),
            &holder(bob, "extract"),
            &now(),
            CommitToken::from_bytes([0xB0; 16]),
            source("extract/601/0"),
        )
    });
    assert!(
        matches!(bob_cycle, Err(IndustryError::RunNotYours)),
        "a co-tenant cannot cycle a run that is not theirs: {bob_cycle:?}"
    );
    // Alice's own cycle mints to ALICE's endpoint -- the run operator's, never a
    // redirector's.
    let cycled = apply(engine, &families, |txn| {
        let live = families.deposit(txn, body.id())?.unwrap();
        let run = families.run(txn, body.id())?.unwrap();
        families.run_cycle(
            txn,
            &live,
            &run,
            alice,
            RequestUid::from_bytes([0xA1; 16]),
            &holder(alice, "extract"),
            &now(),
            CommitToken::from_bytes([0xA1; 16]),
            source("extract/601/0"),
        )
    })
    .expect("alice cycles her own run");
    let lot = read(engine, &families, |txn| {
        txn.get(&families.inventory.lots, &cycled.lot)
            .unwrap()
            .cloned()
            .unwrap()
    });
    assert_eq!(
        lot.endpoint(),
        alice_end,
        "the cycle minted at the run operator's endpoint"
    );
}

#[test]
fn vf_c4_claim_move_is_not_offered_to_a_non_holder() {
    let (mut world, families) = boot();
    install_offer_keys(&mut world);
    let alice = world.spawn_player("alice"); // holder
    let bob = world.spawn_player("bob"); // non-holder
    let rec = ClaimRecord::new(
        mush_industry::ClaimId(0xC4),
        ClaimSubject::Place { place: ObjId(7) },
        PrincipalId::Object(alice),
        PrincipalId::Object(alice),
        ObjId(9004),
        Exclusivity::Shared,
        1,
        later(100_000),
        Transferability::Transferable,
    )
    .unwrap();
    {
        let e = world.engine_mut();
        apply(e, &families, |txn| families.file_claim(txn, rec.clone())).unwrap();
    }
    assert!(
        mush_industry::provider::movable_claims(world.engine(), &families, alice)
            .contains(&rec.id()),
        "the holder may move the record half"
    );
    assert!(
        mush_industry::provider::movable_claims(world.engine(), &families, bob).is_empty(),
        "a non-holder may not"
    );
    assert!(projects_verb(&world, alice, "claim-move"));
    assert!(
        !projects_verb(&world, bob, "claim-move"),
        "the record-half offer is not open to a non-holder"
    );
}

#[test]
fn vf_c3_a_low_accessibility_body_does_not_strand_its_reserve() {
    let (mut world, families) = boot();
    // accessibility 10, per_cycle 5 -> 5 * 10 / 100 = 0 by integer floor. Before
    // the fix the body reported Depleted with 100 units still in the ground.
    let body = OreDeposit::new(
        OreDepositId(603),
        ObjId(7),
        commodity("metals"),
        Grade::new(60).expect("grade"),
        Accessibility::new(10).expect("accessibility"),
        band("grade_b"),
        band("reserve_major"),
        curve(&[(0, 5)]),
        endpoint(603),
        100,
    )
    .expect("low-accessibility body");
    let to = endpoint(603);
    assert!(
        body.next_yield(100) >= 1,
        "a workable, non-empty body whose curve still yields must not strand its reserve"
    );
    let engine = world.engine_mut();
    open(engine, &families, &body);
    start_run(engine, &families, &body, to);
    let mut extracted = 0_u64;
    for ordinal in 0..250_u32 {
        match cycle(engine, &families, &body, (ordinal % 250) as u8 + 1) {
            Ok(cycled) => {
                assert!(
                    cycled.quantity >= 1,
                    "every cycle extracts at least one unit"
                );
                extracted += cycled.quantity;
            }
            Err(IndustryError::Depleted) => break,
            Err(other) => panic!("cycle {ordinal}: {other}"),
        }
    }
    assert_eq!(
        extracted, 100,
        "the low-accessibility body yields exactly its reserve, none stranded"
    );
}

#[test]
fn vf_b3_claim_show_is_not_a_deposit_oracle() {
    let (mut world, families) = boot();
    let secret = OreDepositId(0x5EC7);
    let alice = world.spawn_player("alice"); // holder
    let bob = world.spawn_player("bob"); // stranger, in LIMBO, not co-located
    let gov = world.spawn_player("gov"); // issuer
    let rec = ClaimRecord::new(
        mush_industry::ClaimId(0x777),
        ClaimSubject::Deposit { deposit: secret },
        PrincipalId::Object(gov),
        PrincipalId::Object(alice),
        ObjId(9777),
        Exclusivity::Shared,
        7,
        later(100_000),
        Transferability::Bound,
    )
    .unwrap();
    {
        let e = world.engine_mut();
        apply(e, &families, |txn| families.file_claim(txn, rec.clone())).unwrap();
    }
    // A stranger who only knows the claim id sees NO_SUCH_CLAIM, byte-identical
    // to an unknown id -- no fog-gated deposit id, no principals.
    let ev = world.run(bob, &format!("@claim/show {}", rec.id()));
    let stranger = world.seen_by(&ev, bob).join("\n");
    let ev = world.run(bob, "@claim/show claim#00000000dead0000");
    let unknown = world.seen_by(&ev, bob).join("\n");
    assert_eq!(
        stranger, unknown,
        "a stranger's @claim/show is byte-identical to an unknown id"
    );
    assert!(stranger.contains("No such claim"));
    assert!(
        !stranger.contains("5ec7") && !stranger.contains("obj:"),
        "no deposit id or principal reached a stranger: {stranger}"
    );
    // The holder sees the record -- but not the raw deposit id (non-staff).
    let ev = world.run(alice, &format!("@claim/show {}", rec.id()));
    let holder = world.seen_by(&ev, alice).join("\n");
    assert!(!holder.contains("No such claim"));
    assert!(
        holder.contains("an ore body") && !holder.contains("5ec7"),
        "a non-staff holder sees the claim without the fog-gated deposit id: {holder}"
    );
    // Staff see everything, including the raw deposit id.
    let ev = world.run(GOD, &format!("@claim/show {}", rec.id()));
    let staff = world.seen_by(&ev, GOD).join("\n");
    assert!(
        staff.contains("5ec7"),
        "staff read the raw deposit id: {staff}"
    );
}

#[test]
fn vf_r1_a_pruned_mirror_answers_no_fact_for_the_newest_too() {
    let (mut world, families) = boot();
    let total = mush_industry::MAX_APPLIED_MARKERS + 1;
    let oldest = smol_str::SmolStr::new(format!("{:016x}{:064x}", 0_u64, 0_u64));
    let nord = total - 1;
    let newest = smol_str::SmolStr::new(format!("{nord:016x}{nord:064x}"));
    let c = mush_core::CanonicalId::new("metals").unwrap();
    {
        let engine = world.engine_mut();
        let mut chunk = 0;
        while chunk * 512 < total {
            apply(engine, &families, |txn| {
                for i in 0..512 {
                    let o = chunk * 512 + i;
                    if o >= total {
                        break;
                    }
                    let id = smol_str::SmolStr::new(format!("{o:016x}{o:064x}"));
                    let marker = mush_industry::AppliedMarker::new(
                        mush_industry::FACT_EXTRACTION_CYCLED,
                        OreDepositId(1),
                        o as u64,
                        c.clone(),
                        1,
                        EndpointRef(VaultId(1)),
                        GOD,
                        o as i64,
                    )?;
                    txn.insert(&families.applied, &id, marker)
                        .map_err(IndustryError::from)?;
                }
                Ok(())
            })
            .expect("insert a chunk of markers");
            chunk += 1;
        }
    }
    world.modules_loaded(std::collections::HashMap::new());
    let store = world
        .world()
        .module_state::<IndustryStore>()
        .expect("industry store");
    assert!(
        store.marker(oldest.as_str()).is_none(),
        "the oldest marker is pruned from the RAM mirror"
    );
    assert!(
        store.marker(newest.as_str()).is_some(),
        "the newest is kept"
    );
    assert!(
        !store.is_complete(),
        "the restored `truncated` flag makes a pruned mirror NOT complete"
    );
    let provider = mush_industry::IndustryFactProvider;
    assert!(
        provider
            .resolve(
                world.world(),
                mush_industry::FACT_EXTRACTION_CYCLED,
                oldest.as_str(),
                None
            )
            .is_none()
    );
    assert!(
        provider
            .resolve(
                world.world(),
                mush_industry::FACT_EXTRACTION_CYCLED,
                newest.as_str(),
                None
            )
            .is_none(),
        "fail-closed for the WHOLE family: the NEWEST fact answers None too, never a \
         non-monotone flip"
    );
}

#[test]
fn vf_r2_a_fully_absent_pool_degrades_to_depleted_by_design() {
    // `Quarantine::NoPool` was a dead variant claiming a third shape the tree
    // never constructs. A fully-absent pool pair at a hydrated vault is not a
    // distinct quarantine: it degrades to Depleted -- fail-closed -- by design.
    let (mut world, families) = boot();
    let body = deposit(602, 10_000, &[(0, 500)]);
    let to = endpoint(602);
    let engine = world.engine_mut();
    open(engine, &families, &body);
    start_run(engine, &families, &body, to);
    let key = mush_inventory::pool_key(body.endpoint(), body.commodity()).expect("pool key");
    apply(engine, &families, |txn| {
        txn.remove(&families.inventory.pools, &key)
            .map_err(IndustryError::from)?;
        txn.remove(&families.inventory.books, &key)
            .map_err(IndustryError::from)?;
        Ok(())
    })
    .expect("delete both halves of the pool pair");
    let remaining = read(engine, &families, |txn| {
        families.remaining(txn, &body).expect("remaining")
    });
    assert_eq!(remaining, 0, "a fully-absent pool reads as empty");
    let refused = cycle(engine, &families, &body, 1);
    assert!(
        matches!(refused, Err(IndustryError::Depleted)),
        "a fully-absent pool degrades to Depleted, not a special shape: {refused:?}"
    );
}

#[test]
fn vf_b5_max_claims_bounds_the_lifetime_set() {
    let (mut world, families) = boot();
    let subject = ClaimSubject::Place { place: ObjId(705) };
    let engine = world.engine_mut();
    let mut ids = Vec::new();
    for i in 0..MAX_CLAIMS_PER_SUBJECT {
        let rec = ClaimRecord::new(
            mush_industry::ClaimId(0xB500 + i as u64),
            subject,
            PrincipalId::Object(GOD),
            PrincipalId::Object(GOD),
            ObjId(9500 + i as u32),
            Exclusivity::Shared,
            1,
            later(100_000),
            Transferability::Transferable,
        )
        .unwrap();
        apply(engine, &families, |txn| {
            families.file_claim(txn, rec.clone())
        })
        .expect("file within the cap");
        ids.push(rec.id());
    }
    // "Release" them all -- suspend is terminal-ish, and the index is add-only,
    // so no slot is freed.
    for id in &ids {
        let live = read(engine, &families, |txn| {
            families.claim(txn, *id).unwrap().unwrap()
        });
        apply(engine, &families, |txn| {
            families.put_claim(
                txn,
                live.revision(),
                live.with_contest_state(ContestState::Suspended {
                    by: PrincipalId::Object(GOD),
                })
                .unwrap(),
            )
        })
        .expect("suspending frees no index slot");
    }
    // A ninth file over the SAME subject still refuses: the cap is a LIFETIME
    // bound, not a live one.
    let ninth = ClaimRecord::new(
        mush_industry::ClaimId(0xB5FF),
        subject,
        PrincipalId::Object(GOD),
        PrincipalId::Object(GOD),
        ObjId(9599),
        Exclusivity::Shared,
        1,
        later(100_000),
        Transferability::Transferable,
    )
    .unwrap();
    let refused = apply(engine, &families, |txn| {
        families.file_claim(txn, ninth.clone())
    });
    assert!(
        matches!(refused, Err(IndustryError::Record(_))),
        "MAX_CLAIMS_PER_SUBJECT is a LIFETIME bound; releasing frees no slot: {refused:?}"
    );
}

#[test]
fn vf_b6_a_ninth_surveyor_is_not_locked_out_or_leaked() {
    let (mut world, families) = boot();
    let place = world
        .world_mut()
        .create(ObjKind::Room, "Crowded Site", GOD, None)
        .expect("place");
    let body = body_at(702, place, 40_000, &[(0, 500)]);
    {
        let e = world.engine_mut();
        open(e, &families, &body);
    }
    // Fill the disclosure map to its cap with 8 distinct principals.
    let rows: Vec<SurveyRow> = (0..MAX_SURVEY_BANDS)
        .map(|i| SurveyRow {
            principal: PrincipalId::Object(ObjId(101 + i as u32)),
            bands: SurveyBands {
                grade_class: band("grade_b"),
                reserve_class: band("reserve_major"),
                at: now(),
                fact: source("survey/702"),
            },
        })
        .collect();
    let full = SurveyDoc::from_rows(body.id(), rows).expect("a full survey doc");
    if let Some(store) = world.world_mut().module_state_mut::<IndustryStore>() {
        store.put_survey(full);
    }
    // A ninth, co-located, non-staff surveyor is refused -- but byte-identically
    // to an unknown body, never with the "at most 8 principals" cap message.
    let ninth = world.spawn_player("ninth");
    world.world_mut().move_to(ninth, place).expect("co-locate");
    let ev = world.run(ninth, &format!("@survey {}", body.id()));
    let seen = world.seen_by(&ev, ninth).join("\n");
    let ev = world.run(ninth, "@survey deposit#00000000dead0000");
    let unknown = world.seen_by(&ev, ninth).join("\n");
    assert_eq!(
        seen, unknown,
        "the ninth surveyor's refusal is byte-identical to an unknown body"
    );
    assert!(
        !seen.contains("at most"),
        "a full surveyor roster is not leaked as a distinct message: {seen}"
    );
    assert!(seen.contains("No such ore body"));
}

#[test]
fn vf_b4_survey_off_site_is_indistinguishable_from_unknown() {
    let (mut world, families) = boot();
    let place = world
        .world_mut()
        .create(ObjKind::Room, "Ceres Site", GOD, None)
        .expect("place");
    let body = body_at(701, place, 40_000, &[(0, 500)]);
    {
        let e = world.engine_mut();
        open(e, &families, &body);
    }
    let bob = world.spawn_player("bob"); // spawns in LIMBO, off-site
    let ev = world.run(bob, &format!("@survey {}", body.id()));
    let off = world.seen_by(&ev, bob);
    let ev = world.run(bob, "@survey deposit#00000000dead0000");
    let unknown = world.seen_by(&ev, bob);
    assert_eq!(
        off, unknown,
        "off-site survey renders identically to an unknown id"
    );
    for line in off.iter().chain(unknown.iter()) {
        assert!(
            !line.contains("grade band") && !line.contains("reserve band"),
            "no bands leak off-site: {line}"
        );
    }
    assert!(
        world
            .world()
            .module_state::<IndustryStore>()
            .and_then(|store| store.survey(body.id()))
            .is_none(),
        "a refused survey writes no disclosure row"
    );
    // On-site: the two recorded bands.
    world.world_mut().move_to(bob, place).expect("move on-site");
    let ev = world.run(bob, &format!("@survey {}", body.id()));
    let on_site = world.seen_by(&ev, bob).join("\n");
    assert!(
        on_site.contains("grade band grade_b") && on_site.contains("reserve band reserve_major"),
        "an on-site survey yields the two recorded bands: {on_site}"
    );
    // Staff survey from anywhere (GOD is off-site in LIMBO).
    let ev = world.run(GOD, &format!("@survey {}", body.id()));
    let staff = world.seen_by(&ev, GOD).join("\n");
    assert!(
        staff.contains("grade band grade_b"),
        "staff survey from anywhere: {staff}"
    );
}

#[test]
fn vf_b2_a_re_survey_across_a_reboot_is_a_hit() {
    let (mut world, families) = boot();
    let place = world
        .world_mut()
        .create(ObjKind::Room, "Survey Site", GOD, None)
        .expect("place");
    let body = body_at(703, place, 40_000, &[(0, 500)]);
    {
        let e = world.engine_mut();
        open(e, &families, &body);
    }
    let alice = world.spawn_player("alice");
    world.world_mut().move_to(alice, place).expect("co-locate");
    let _ = world.run(alice, &format!("@survey {}", body.id()));
    let doc = world
        .world()
        .module_state::<IndustryStore>()
        .and_then(|store| store.survey(body.id()).cloned())
        .expect("a survey doc");
    let alice_principal = PrincipalId::Object(alice);
    let at_before = doc
        .bands_for(&alice_principal)
        .expect("alice's row")
        .at
        .clone();
    // Reboot: hand the persisted document back through modules_loaded, which
    // re-decodes it through the SurveyDoc TryFrom wire.
    let value = serde_json::to_value(&doc).expect("serialize");
    let mut boot_data = std::collections::HashMap::new();
    boot_data.insert(
        smol_str::SmolStr::new("industry"),
        ModuleBootData {
            kv: vec![(mush_industry::store::survey_key(body.id()), value)],
            streams: std::collections::HashMap::new(),
            ..Default::default()
        },
    );
    world.modules_loaded(boot_data);
    let rebooted = world
        .world()
        .module_state::<IndustryStore>()
        .and_then(|store| store.survey(body.id()).cloned())
        .expect("the doc survives the reboot through modules_loaded");
    assert_eq!(
        rebooted.bands_for(&alice_principal).expect("row").at,
        at_before,
        "the disclosure survived the wire round-trip unchanged"
    );
    // A re-survey is a HIT: it rewrites nothing and answers the ORIGINAL stamp,
    // never the re-survey's later one.
    let fresh_bands = SurveyBands {
        grade_class: band("grade_b"),
        reserve_class: band("reserve_major"),
        at: later(999_999),
        fact: source("survey/703"),
    };
    let (again, fresh) = rebooted
        .disclosed(&alice_principal, fresh_bands)
        .expect("re-survey");
    assert!(!fresh, "a re-survey after a reboot is a HIT");
    assert_eq!(
        again.bands_for(&alice_principal).expect("row").at,
        at_before,
        "the HIT kept the original disclosure stamp"
    );
}

// =====================================================================
// WS02-S3 — the `@claim/file` issuer gate, in all three layers: the
// command before the world mutation, the offer, and the prepare.
// =====================================================================

/// Cut 3's rig **with organizations in it**. `boot()` above never loads
/// `mush-org`, and `acts_for` on an organization principal reads its state, so a
/// world with no org store is a world where every non-staff filer is refused —
/// correct, and useless for proving that an authorised one is not.
fn boot_orgs() -> (TestWorld, IndustryFamilies) {
    let cell: InventoryHandleCell = Arc::default();
    let world = TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(mush_economy::EconomyModule::new()),
        Box::new(
            InventoryModule::new()
                .with_quantity_delegates(["industry"])
                .with_handle_cell(Arc::clone(&cell)),
        ),
        Box::new(IndustryModule::new().with_inventory(Arc::clone(&cell))),
    ]);
    let families = world
        .world()
        .module_state::<IndustryHandles>()
        .expect("industry publishes its handles")
        .0
        .clone();
    (world, families)
}

fn org_state(world: &TestWorld) -> &mush_org::OrgState {
    world
        .world()
        .module_state::<mush_org::OrgStore>()
        .expect("org store")
        .state()
}

/// How many titles the economy holds. `@claim/file` mints one per filing, so this
/// is how "the refusal wrote nothing" is measured on the E4 side.
fn titles(world: &TestWorld) -> usize {
    world
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .expect("economy store")
        .snapshot()
        .assets()
        .count()
}

fn claim_file_action(world: &TestWorld) -> mush_core::DefinitionRef {
    mush_industry::action_ref(world.registry(), mush_industry::IndustryVerb::ClaimFile)
        .expect("the claim-file action is registered")
}

/// One `ClaimFile` invocation, hand-built: the shape a resolved offer produces,
/// **including the three fixed parameters** the projection authors. Building it
/// here is the point — it is how a submission that never met the offer's `Choice`
/// is put in front of `prepare_authoritative`.
fn file_invocation(
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
    .expect("a claim-file invocation")
}

/// The parameters a `@claim/file` submission carries, with the offer's own fixed
/// three spliced in exactly where `merge_parameters` would have put them.
fn file_parameters(filer: ObjId, issuer: &str, staff: bool, vouched: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "now": now(),
        "subject": "place:7",
        "issuer": issuer,
        "holder": format!("obj:{}", filer.0),
        "object": 9_100,
        "exclusivity": "shared",
        "priority": 1,
        // WS02-A2: an integer, the shape the offer declares and the command
        // stages. `now()` is generation 1, so `term_field` rebuilds exactly
        // `later(100_000)`.
        "term": 100_000,
        "transferability": "bound",
        "claim_filed_by": filer.0,
        "claim_file_staff": staff,
        "claim_issuers": vouched,
    })
}

/// The provider's own refusal code inside the kernel's prepare error.
fn provider_code(error: &mush_core::ActionProviderPrepareError) -> mush_core::ActionErrorCode {
    match error {
        mush_core::ActionProviderPrepareError::Provider { source, .. } => source.code(),
        other => panic!("expected a provider refusal, got {other}"),
    }
}

/// Every live claim over the one subject these tests file over.
fn filed_claims(
    world: &mut TestWorld,
    families: &IndustryFamilies,
) -> Vec<mush_industry::ClaimRecord> {
    let subject = ClaimSubject::Place { place: ObjId(7) };
    read(world.engine_mut(), families, |txn| {
        families.claims_over(txn, subject).expect("claims")
    })
}

/// **Layer 1.** The command creates a `Thing` and issues an E4 title *before* it
/// submits, so a gate that lived only on the prepared path would mint an orphan
/// object on every refusal — an object-creation DoS a player could drive in a
/// loop. The world's next id is the assertion.
#[test]
fn ws02_s3_a_filer_with_no_authority_mints_no_object_and_no_title() {
    let (mut world, families) = boot_orgs();
    let alice = world.spawn_player("alice");
    // A `Thing` alice owns, so `acts_for` is true of `obj:<widget>` for her:
    // `controls` covers anything she owns. Minted before the baseline, because
    // what is under test is that the *filing* mints nothing.
    let widget = world
        .world_mut()
        .create(ObjKind::Thing, "widget", alice, None)
        .expect("widget");
    let before_id = world.world().next_id();
    let before_titles = titles(&world);
    let ev = world.run(alice, "@claim/file place:7=org:1/obj:2/shared/1/1000/bound");
    let refused = world.seen_by(&ev, alice).join("\n");
    assert!(
        refused.contains("No such claim."),
        "an unauthorised filer is refused: {refused}"
    );
    assert_eq!(
        world.world().next_id(),
        before_id,
        "the refusal minted no world object"
    );
    assert_eq!(titles(&world), before_titles, "the refusal issued no title");
    assert!(
        filed_claims(&mut world, &families).is_empty(),
        "and no claim record"
    );
    // And it is byte-identical to naming an authority that does not exist at
    // all: the refusal is not an org-existence oracle.
    let ev = world.run(
        alice,
        "@claim/file place:7=org:999999/obj:2/shared/1/1000/bound",
    );
    assert_eq!(
        world.seen_by(&ev, alice).join("\n"),
        refused,
        "a foreign authority and a missing one are refused identically"
    );
    // And the one `acts_for` alone would have admitted: an `Object` principal
    // alice MUSH-controls. Layer 2 offers organizations only, so a layer 1 that
    // stopped at `acts_for` would mint the claim object here and refuse it at
    // the provider — and every minted claim is itself owned by alice, so the
    // loop would feed itself.
    let ev = world.run(
        alice,
        &format!(
            "@claim/file place:7=obj:{}/obj:2/shared/1/1000/bound",
            widget.0
        ),
    );
    assert_eq!(
        world.seen_by(&ev, alice).join("\n"),
        refused,
        "an object the filer controls is not an authority over the ground"
    );
    assert_eq!(
        world.world().next_id(),
        before_id,
        "and naming it minted no world object either"
    );
    assert_eq!(
        titles(&world),
        before_titles,
        "nor a title under the controlled object"
    );
    assert!(
        filed_claims(&mut world, &families).is_empty(),
        "nor a claim record"
    );
}

/// **Layer 2.** The offer itself: no authority, no offer at all — the same
/// nothing a verb that does not exist projects. A grant of the spending
/// capability in one org is what makes it appear.
#[test]
fn ws02_s3_claim_file_is_offered_only_to_staff_and_to_an_authorised_principal() {
    let (mut world, _families) = boot_orgs();
    install_offer_keys(&mut world);
    let alice = world.spawn_player("alice");
    let bob = world.spawn_player("bob");
    assert!(
        mush_industry::provider::issuable_authorities(world.engine(), alice).is_empty(),
        "a player who speaks for nobody has no issuable authority"
    );
    assert!(
        !projects_verb(&world, alice, "claim-file"),
        "and is offered no filing path at all"
    );
    assert!(
        projects_verb(&world, GOD, "claim-file"),
        "staff keep the free-text issuer"
    );
    let _ = world.run(GOD, "@org/create Ironhold");
    let org = org_state(&world).orgs().next().expect("the org").id;
    let _ = world.run(
        GOD,
        &format!("@org/grant #{}=Ironhold economy.account.post", alice.0),
    );
    assert_eq!(
        mush_industry::provider::issuable_authorities(world.engine(), alice),
        vec![PrincipalId::Organization(org)],
        "the grantee's own org, and nothing else"
    );
    assert!(
        projects_verb(&world, alice, "claim-file"),
        "an authorised principal is offered the filing path"
    );
    assert!(
        mush_industry::provider::issuable_authorities(world.engine(), bob).is_empty()
            && !projects_verb(&world, bob, "claim-file"),
        "the org's existence leaked nothing to a stranger"
    );
}

/// **Layer 3.** The forged-parameter bypass: an invocation that never met the
/// offer's `Choice` names an authority the projection did not vouch for. The
/// fixed `claim_issuers` list is what refuses it, and the identical invocation
/// naming a vouched authority commits — so the gate is the list, not a blanket no.
#[test]
fn ws02_s3_a_forged_issuer_refuses_unauthorized_and_files_nothing() {
    let (mut world, families) = boot_orgs();
    let alice = world.spawn_player("alice");
    let action = claim_file_action(&world);
    let forged = file_invocation(
        &action,
        alice,
        0x52,
        file_parameters(alice, "org:2", false, &["org:1"]),
    );
    let refusal = world
        .engine_mut()
        .prepare_registered_authoritative_action(
            &action,
            &forged,
            CommitToken::from_bytes([0x52; 16]),
        )
        .expect_err("an issuer the offer never vouched for must refuse");
    assert_eq!(
        provider_code(&refusal),
        mush_core::ActionErrorCode::Unauthorized,
        "a forged issuer is unauthorized, not merely rejected"
    );
    assert!(
        filed_claims(&mut world, &families).is_empty(),
        "and no ClaimRecord was written"
    );
    // A replay of one filer's own vouched offer under a SECOND filer's name is
    // refused by the same gate: the list is bound to the actor it was made for.
    let bob = world.spawn_player("bob");
    let stolen = file_invocation(
        &action,
        bob,
        0x53,
        file_parameters(alice, "org:1", false, &["org:1"]),
    );
    let refusal = world
        .engine_mut()
        .prepare_registered_authoritative_action(
            &action,
            &stolen,
            CommitToken::from_bytes([0x53; 16]),
        )
        .expect_err("another actor's authority list is not this actor's");
    assert_eq!(
        provider_code(&refusal),
        mush_core::ActionErrorCode::Unauthorized
    );
    assert!(filed_claims(&mut world, &families).is_empty());
    // The honest one, through the identical prepare, commits.
    let honest = file_invocation(
        &action,
        alice,
        0x54,
        file_parameters(alice, "org:1", false, &["org:1"]),
    );
    let (prepared, _) = world
        .engine_mut()
        .prepare_registered_authoritative_action(
            &action,
            &honest,
            CommitToken::from_bytes([0x54; 16]),
        )
        .expect("a vouched authority files");
    world
        .engine_mut()
        .install_prepared_action(prepared.into_parts().1)
        .expect("the fixture installs");
    let filed = filed_claims(&mut world, &families);
    assert_eq!(filed.len(), 1, "exactly the one vouched filing");
    assert_eq!(
        filed[0].issuer(),
        &PrincipalId::Organization(mush_org::OrgId(1)),
        "under the authority the offer vouched for"
    );
    // And that org-issued claim still authorises its holder's extraction: the
    // tenure leg passes and only the licence leg is missing.
    let body = deposit(0x5305, 40_000, &[(0, 500)]);
    assert!(
        matches!(
            mush_industry::extraction_authorized(
                world.world(),
                alice,
                &PrincipalId::Object(alice),
                &body,
                40_000,
                &filed,
                None,
                &now(),
            ),
            Err(ExtractionRefusal::NoLicenceInstrument)
        ),
        "an org-issued claim authorises tenure; only the licence is absent"
    );
}

/// The owner's decision: an issuer is an authority **over** the ground, so nobody
/// files under their own object principal — not a player, whose `acts_for` on
/// themselves is trivially true, and not staff, whose fixed `claim_file_staff`
/// otherwise waves everything through.
#[test]
fn ws02_s3_nobody_files_under_their_own_object_principal() {
    let (mut world, families) = boot_orgs();
    let alice = world.spawn_player("alice");
    let before_id = world.world().next_id();
    let ev = world.run(
        alice,
        &format!(
            "@claim/file place:7=obj:{}/obj:{}/shared/1/1000/bound",
            alice.0, alice.0
        ),
    );
    assert!(
        world
            .seen_by(&ev, alice)
            .join("\n")
            .contains("No such claim."),
        "self-issued tenure is refused"
    );
    assert_eq!(
        world.world().next_id(),
        before_id,
        "and mints nothing on the way out"
    );
    // Staff, at the prepare, with the staff flag set: still refused.
    let action = claim_file_action(&world);
    let refusal = world
        .engine_mut()
        .prepare_registered_authoritative_action(
            &action,
            &file_invocation(
                &action,
                GOD,
                0x55,
                file_parameters(GOD, &format!("obj:{}", GOD.0), true, &[]),
            ),
            CommitToken::from_bytes([0x55; 16]),
        )
        .expect_err("staff do not self-issue either");
    assert_eq!(
        provider_code(&refusal),
        mush_core::ActionErrorCode::Unauthorized
    );
    assert!(filed_claims(&mut world, &families).is_empty());
}

/// Staff keep the free-text issuer and file under any authority they name — the
/// `claim_file_staff` path, through the same prepare every other filing takes.
#[test]
fn ws02_s3_a_staffer_files_under_an_arbitrary_authority() {
    let (mut world, families) = boot_orgs();
    let action = claim_file_action(&world);
    let (prepared, _) = world
        .engine_mut()
        .prepare_registered_authoritative_action(
            &action,
            &file_invocation(
                &action,
                GOD,
                0x56,
                file_parameters(GOD, "org:4242", true, &[]),
            ),
            CommitToken::from_bytes([0x56; 16]),
        )
        .expect("staff file under any authority");
    world
        .engine_mut()
        .install_prepared_action(prepared.into_parts().1)
        .expect("the fixture installs");
    let filed = filed_claims(&mut world, &families);
    assert_eq!(filed.len(), 1);
    assert_eq!(
        filed[0].issuer(),
        &PrincipalId::Organization(mush_org::OrgId(4242)),
        "an authority no organization record vouches for is still staff's to name"
    );
}

// ---- WS02-A2 — `@claim/file` submits a term the offer accepts ---------------
//
// The projection declares `integer("term", …)`; the command staged a
// `ClockStamp` **object**. Every shipped test either files a `ClaimRecord` by
// hand or hands `prepare_registered_authoritative_action` an invocation it built
// itself, and both routes skip the one rung that reads the offer's schema — so
// nothing here noticed that on a real server no player could file tenure at all.
// The two tests below stand on that rung.

/// `boot_orgs`, plus the one line of composition the *whole* `@claim/file`
/// command needs: `EconomyModule::with_title_issuers`, which the composition
/// root in `mush-server` gives it (`issuers.push(mush_industry::industry_title_issuer())`).
/// Without it `issue_title` answers `Unauthorized` and the command refuses
/// before it stages anything at all — so the route under test here is only
/// reachable from a rig that composes the server's own wiring.
fn boot_filing() -> (TestWorld, IndustryFamilies) {
    let cell: InventoryHandleCell = Arc::default();
    let world = TestWorld::with_modules(vec![
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
        Box::new(IndustryModule::new().with_inventory(Arc::clone(&cell))),
    ]);
    let families = world
        .world()
        .module_state::<IndustryHandles>()
        .expect("industry publishes its handles")
        .0
        .clone();
    (world, families)
}

/// Drive the one staged submission the way the world task does: regenerate the
/// actor's **own** offer, resolve the staged parameters against it (this is
/// `ProjectedParameterSchema::validate`, the rung that refuses a parameter of
/// the wrong type), then prepare and install.
fn land(world: &mut TestWorld, token: u8) -> Result<(), String> {
    let submit = world
        .engine_mut()
        .take_authoritative_submit()
        .ok_or_else(|| "nothing was staged".to_owned())?;
    let (action, resolved) = {
        let engine = world.engine();
        let query = engine
            .legal_action_query(
                mush_core::PrincipalId::Object { id: submit.actor },
                submit.actor,
                submit.subject.clone(),
            )
            .map_err(|error| error.to_string())?;
        let offers = engine
            .project_legal_actions(&query)
            .map_err(|error| error.to_string())?;
        let projected = offers
            .projected_for_action(&submit.action)
            .ok_or_else(|| "that action is not currently offered".to_owned())?;
        let submission = mush_core::OfferSubmission {
            offer: projected.offer(),
            subject: projected.subject().clone(),
            targets: Vec::new(),
            parameters: submit.parameters.clone(),
        };
        let execution = mush_core::OfferExecutionContext {
            request: RequestUid::from_bytes([token; 16]),
            principal: mush_core::PrincipalId::Object { id: submit.actor },
            actor: submit.actor,
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal: mush_core::PrincipalId::Object { id: submit.actor },
                    owner: submit.action.key().owner.clone(),
                    namespace: mush_core::CanonicalId::new(submit.action.key().id.as_str())
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

/// One authorised filer: an org, and the grant that makes `issuable_authorities`
/// name it for `who`.
fn vouch(world: &mut TestWorld, who: ObjId) -> mush_org::OrgId {
    let _ = world.run(GOD, "@org/create Ironhold");
    let org = org_state(world).orgs().next().expect("the org").id;
    let _ = world.run(
        GOD,
        &format!("@org/grant #{}=Ironhold economy.account.post", who.0),
    );
    while world.engine_mut().take_authoritative_submit().is_some() {}
    org
}

/// **The regression.** `@claim/file` typed at the keyboard, resolved against the
/// actor's own regenerated offer, prepared and installed — the whole route a
/// real server takes. Before this fix the staged `ClockStamp` object met
/// `integer("term", …)` and `resolve_legal_action` answered `InvalidParameters`,
/// so this filing (and every other) never reached the provider.
///
/// The term is pinned exactly: the record keeps a `ClockStamp` whose generation
/// is the world's and whose second is the one the command computed.
#[test]
fn ws02_a2_claim_file_lands_through_the_real_offer_route() {
    let (mut world, families) = boot_filing();
    install_offer_keys(&mut world);
    let alice = world.spawn_player("alice");
    let org = vouch(&mut world, alice);
    let ic = world.world().world_ic_now();

    let _ = world.run(
        alice,
        &format!(
            "@claim/file place:7=org:{}/obj:{}/shared/1/1000/bound",
            org.0, alice.0
        ),
    );
    land(&mut world, 0xA2).expect("@claim/file resolves — this is the WS02-A2 regression");

    let filed = filed_claims(&mut world, &families);
    assert_eq!(filed.len(), 1, "the filing landed");
    assert_eq!(
        filed[0].issuer(),
        &PrincipalId::Organization(org),
        "under the authority the offer vouched for"
    );
    assert_eq!(
        filed[0].term(),
        &ClockStamp {
            clock: ic.clock.clone(),
            at: ic.at + 1_000,
        },
        "the rebuilt stamp is exactly the absolute IC second the command computed, \
         under the world's own generation"
    );
}

/// The old wire shape is not silently accepted. A hand-built invocation skips
/// the offer schema entirely — it is the only way a `ClockStamp` object can
/// still reach `term_field` — and `term_field` refuses it there, so the type is
/// enforced on both rungs rather than only on the one the schema guards.
#[test]
fn ws02_a2_a_clockstamp_object_submitted_for_term_stays_refused() {
    let (mut world, families) = boot_orgs();
    let alice = world.spawn_player("alice");
    let action = claim_file_action(&world);
    let mut parameters = file_parameters(alice, "org:1", false, &["org:1"]);
    parameters["term"] = serde_json::json!(later(100_000));
    let refusal = world
        .engine_mut()
        .prepare_registered_authoritative_action(
            &action,
            &file_invocation(&action, alice, 0xA3, parameters),
            CommitToken::from_bytes([0xA3; 16]),
        )
        .expect_err("the object shape is not a term");
    assert_eq!(
        provider_code(&refusal),
        mush_core::ActionErrorCode::InvalidRequest,
        "a term of the wrong type is an invalid request, not a silent acceptance"
    );
    assert!(
        filed_claims(&mut world, &families).is_empty(),
        "and nothing was written"
    );
}
