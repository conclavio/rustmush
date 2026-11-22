//! **Doc 36a lane 1 — salvage claims and tow, the regressions** (doc 36a §3, §4, §10 "Lane 1";
//! doc 36 Q-36-20..Q-36-29, doc 32 §10), over the composed world (`driving`'s keyed rig) on
//! Urvara Reach, with GC.6's three parties:
//!
//! - **the title holder** (T), whose two disabled spare rigs lie on the ground and whose carrier
//!   stands on the pad;
//! - **the opportunist** (O), whose rig tows them into O's own carrier and files the claims;
//! - **the landholder** (L), who holds the Registry's claim on the block where the first lies.
//!
//! Every test reproduces the external review's steps as a player types them through the
//! production command paths (`+tv/<n> tow|load|release|pickup`, `+salvage/*`, `@contract/*`);
//! the stage's fixtures are GC.6's (the rigs' power destroyed and the rigs indexed by staff
//! hand, their title moved by `@asset/transfer`, L's block claim seeded as a typed record).
//!
//! | Review item | Test |
//! |---|---|
//! | P0-2 `@contract/amend` + `/deadline` | [`p0_2_the_salvor_cannot_amend_or_shorten_their_own_claim`] |
//! | P0-2 `@contract/dispute` + `/agree` | [`p0_2_the_salvor_cannot_dispute_and_agree_their_claim_away`] |
//! | P0-3 the Occator rig after the write-off | [`p0_3_a_leased_rig_written_off_in_custody_goes_home_and_is_never_awarded`] |
//! | P0-7 the award/contest second | [`p0_7_the_windows_last_second_belongs_to_the_award_not_a_contest`] |
//! | P0-7 the lost `Loaded` | [`p0_7_a_lost_loaded_finish_still_lands_after_the_carrier_has_gone`] |
//! | P1-3 `TowOrders` clobber | [`p1_3_the_salvors_inspect_spam_does_not_kill_a_queued_contest`] |
//! | P1-4 the attach race drops the pin | [`p1_4_a_second_tugs_attach_waits_and_the_first_tugs_pin_stands`] |
//! | P1-10 `Release` lacks `promoted` | [`p1_10_a_release_draining_after_a_promotion_waits`] |
//! | P2-2 the pickup never checks the hopper | [`p2_2_a_crate_the_hopper_cannot_take_is_refused_and_the_landholder_is_named`] |

use mush_core::ObjId;
use mush_economy::{ContractId, ContractState, EconomyStore};
use mush_org::PrincipalId;
use tacsim::surface::salvage::{ClaimSeam, SalvageCustody};
use tacsim::surface::site_record::SiteRecord;

use crate::driving_rig::{
    AHUNA, Rig, cid, command, hold_pool, launched, launched_at, on_pad, on_pad_as, on_pad_as_at,
    pilot, port_map, ramp_at, run, seed_pool, seed_typed, stocked_tuned, store, to_cell,
    to_cell_on,
};
use crate::field_gc4::{powerless, seconds_until};
use crate::field_gc5::skip_to_ic;
use crate::field_host_rig::{armed_on, seat, second, site_record, site_record_at, tune};
use crate::site_record_rig::{staff, staff_member};

/// Urvara Reach's port entry.
pub(crate) const URVARA: &str = "urvara_port";
/// The compressed contest window, real seconds.
pub(crate) const WINDOW: i64 = 600;

/// Short tow stages and a ten-minute contest window, on the field host's lab tempo.
pub(crate) fn quick(cfg: &mut tacsim::TacsimConfig) {
    tune(cfg);
    cfg.recovery.inspect_s = 30;
    cfg.recovery.attach_s = 30;
    cfg.recovery.load_s = 60;
    cfg.recovery.claim_window_ic_s = WINDOW;
}

/// **GC.6's stage**: T's carrier on Urvara's pad; O's carrier on the same pad with one rig at
/// the ramp; L the landholder of the block `near` lies on; T's two spare rigs disabled and
/// indexed, `near` beside the pad (in sight of T's carrier) on L's block, `far` out of sight on
/// open ground.
pub(crate) struct Stage {
    pub rig: Rig,
    pub t: ObjId,
    pub o: ObjId,
    pub o_carrier: ObjId,
    pub tug: ObjId,
    /// T's own rig, still in the Salt Moth's bay.
    pub t_tug: ObjId,
    pub l: ObjId,
    pub near: ObjId,
    pub far: ObjId,
    pub near_cell: (i64, i64),
    pub far_cell: (i64, i64),
    /// The site staff hand the stage was built with (one per world: its name is unique).
    pub staff: ObjId,
    pub ramp: (i64, i64),
}

pub(crate) fn record(rig: &mut Rig) -> SiteRecord {
    site_record_at(rig, URVARA).expect("Urvara Reach's record")
}

pub(crate) fn custody(rig: &mut Rig, hull: ObjId) -> Option<SalvageCustody> {
    record(rig).custody.into_iter().find(|row| row.hull == hull)
}

pub(crate) fn title(rig: &Rig, hull: ObjId) -> Option<PrincipalId> {
    rig.tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|e| e.snapshot().asset(hull).map(|a| a.title_owner.clone()))
}

pub(crate) fn contract(rig: &Rig, id: u64) -> mush_economy::EconomicContract {
    rig.tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|e| e.snapshot().contract(ContractId(id)).cloned())
        .expect("the claim's contract")
}

/// `line` as `who`, every submission it queued landed; the reply.
pub(crate) fn go(rig: &mut Rig, who: ObjId, line: &str) -> String {
    let (out, queued) = command(rig, who, line);
    assert_eq!(queued.len(), 1, "{line}: one submission: {out}");
    crate::driving_rig::land(rig, &queued[0]).unwrap_or_else(|e| panic!("{line}: {e}"));
    out
}

/// Move real time `secs` on, then a second of ticks.
pub(crate) fn jump(rig: &mut Rig, secs: i64) {
    let t = rig.tw.world().now().saturating_add(secs);
    rig.tw.set_now(t);
    let _ = second(rig);
}

/// The tug's tow on Urvara Reach.
pub(crate) fn towing(rig: &mut Rig, tug: ObjId) -> Option<ObjId> {
    record(rig).tows.towing(tug).map(|op| op.casualty())
}

impl Stage {
    /// The stage, built.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn build() -> Stage {
        let mut rig = stocked_tuned(quick);
        let (t, _, t_tvs) =
            on_pad_as_at(&mut rig, ("Pilot", 1, "Salt Moth"), ("urvara", URVARA), 1);
        let t_tug = t_tvs[0];
        let (o, o_row, o_tvs) =
            on_pad_as_at(&mut rig, ("Second", 2, "Red Moth"), ("urvara", URVARA), 1);
        let o_carrier = ObjId(o_row.hull);
        let tug = o_tvs[0];
        let l = pilot(&mut rig, "Lander", 3);
        let schema = mush_tactics::module::unit_of(rig.tw.world(), tug)
            .expect("the tug's unit")
            .schema()
            .clone();
        let spares: Vec<ObjId> = rig
            .tw
            .world()
            .module_state::<mush_tactics::UnitStore>()
            .expect("the units")
            .units
            .iter()
            .filter(|(id, unit)| **id != tug && unit.schema() == &schema)
            .map(|(id, _)| *id)
            .filter(|id| {
                crate::driving_rig::store(&rig.tw)
                    .tv_lease_on(*id)
                    .is_none()
            })
            .take(2)
            .collect();
        let [near, far] = spares.as_slice() else {
            panic!("two spare rigs in Occator's pool: {spares:?}");
        };
        let (near, far) = (*near, *far);
        for hull in [near, far] {
            let moved = run(
                &mut rig.tw,
                mush_core::GOD,
                &format!("@asset/transfer #{}=#{}", hull.0, t.0),
            );
            assert_eq!(title(&rig, hull), Some(PrincipalId::Object(t)), "{moved}");
        }
        let near_cell = (258_i64, 254_i64);
        let far_cell = (258_i64, 300_i64);
        to_cell_on(&mut rig, URVARA, near, near_cell);
        to_cell_on(&mut rig, URVARA, far, far_cell);
        let s = staff_member(&mut rig);
        for (hull, (x, y)) in [(near, near_cell), (far, far_cell)] {
            powerless(&mut rig, hull);
            staff(
                &mut rig,
                s,
                &format!("@tacsim/site {URVARA} leave #{} casualty {x},{y}", hull.0),
            );
        }
        // L's block: the Registry's claim on the block `near` lies in, seeded.
        let scheme = tacsim::surface::address::Scheme::of_site(512);
        let letter = tacsim::field_ops::block_letter_at_site_pose(
            mush_tactics::field_ops::FieldPose {
                site: ObjId(1),
                x_cm: near_cell.0 * 1_000 + 500,
                y_cm: near_cell.1 * 1_000 + 500,
            },
            scheme,
        )
        .expect("a block");
        let place = port_map(&rig.tw, URVARA)
            .site
            .and_then(|site| site.blocks.get(letter.to_string().as_str()).copied())
            .map(ObjId)
            .expect("the block's place");
        let now = rig.tw.world().world_ic_now().at;
        let mut tenure = tacsim::surface::claims::BlockTenure::new(cid(URVARA), letter, place);
        tenure.claim = Some(tacsim::surface::claims::BlockClaim {
            holder: PrincipalId::Object(l),
            account: rig
                .tw
                .world()
                .module_state::<mush_sim::AccountOracle>()
                .and_then(|oracle| oracle.account_of(l)),
            epoch: 1,
            opened_ic: None,
            registered_ic: now,
            term_start_ic: now,
            term_end_ic: now + 365 * 86_400,
            worked_at_term_start: 0,
            discovered_ic: None,
            declared: None,
            fee: None,
            refund: None,
            lapsed_ic: None,
        });
        let handles = rig
            .tw
            .world()
            .module_state::<tacsim::port::PortHandles>()
            .cloned()
            .expect("the port's handles");
        seed_typed(
            &mut rig,
            &handles.families.claims,
            &tacsim::surface::claims::BlockKey {
                site: cid(URVARA),
                letter,
            },
            tenure,
        );
        launched_at(&mut rig, URVARA, o, tug);
        let ramp = {
            let (x, y, _) = crate::driving_rig::ramp_at(&rig.tw, URVARA);
            (x.div_euclid(1_000), y.div_euclid(1_000))
        };
        Stage {
            rig,
            t,
            o,
            o_carrier,
            tug,
            t_tug,
            l,
            near,
            far,
            near_cell,
            far_cell,
            staff: s,
            ramp,
        }
    }

    /// O's rig, beside `casualty`, takes it in tow (`+tv/1 tow #<hull> go`, landed at its
    /// due instant).
    pub(crate) fn attach(&mut self, casualty: ObjId) {
        let cell = if casualty == self.near {
            self.near_cell
        } else {
            self.far_cell
        };
        to_cell_on(&mut self.rig, URVARA, self.tug, (cell.0 + 1, cell.1));
        let _ = second(&mut self.rig);
        let out = go(
            &mut self.rig,
            self.o,
            &format!("+tv/1 tow #{} go", casualty.0),
        );
        let tug = self.tug;
        assert!(
            seconds_until(&mut self.rig, 120, |rig| towing(rig, tug) == Some(casualty)),
            "the tow attaches: {out}"
        );
    }

    /// O drives the tow to O's own ramp and loads it into the Red Moth: in O's custody.
    pub(crate) fn load(&mut self, casualty: ObjId) -> SalvageCustody {
        to_cell_on(&mut self.rig, URVARA, self.tug, self.ramp);
        let _ = second(&mut self.rig);
        let out = go(&mut self.rig, self.o, "+tv/1 load go");
        let tug = self.tug;
        assert!(
            seconds_until(&mut self.rig, 120, |rig| custody(rig, casualty).is_some()
                && record(rig).tows.act_of(tug).is_none()),
            "the load lands in custody: {out}"
        );
        custody(&mut self.rig, casualty).expect("in O's custody")
    }

    /// O tows and loads `casualty`, then files the claim: the contract id and the filing's
    /// real instant.
    pub(crate) fn recover_and_claim(&mut self, casualty: ObjId) -> (u64, i64) {
        self.attach(casualty);
        let _ = self.load(casualty);
        let filed_at = self.rig.tw.world().now();
        let out = go(
            &mut self.rig,
            self.o,
            &format!("+salvage/claim #{} go", casualty.0),
        );
        assert!(out.contains("You file a salvage claim"), "{out}");
        let row = custody(&mut self.rig, casualty).expect("still in custody");
        let ClaimSeam::Filed { contract } = row.claim else {
            panic!("filed: {row:?}");
        };
        (contract, filed_at)
    }
}

/// The claim's window as the economy and the record run it, real seconds.
fn window_end(rig: &Rig, contract: u64) -> i64 {
    mush_economy::salvage_claim_window(rig.tw.world(), ContractId(contract))
        .expect("a salvage claim's window")
        .0
}

/// **P0-2, path A** (doc 36a §3): the salvor, the contract's only party, may not amend their
/// own claim back to a draft and set its deadline to one second. Both switches are refused,
/// the contract stays `Active` with its window unmoved, the custody row stays `Filed`, and the
/// next ticks award nothing: the title is still T's.
#[test]
fn p0_2_the_salvor_cannot_amend_or_shorten_their_own_claim() {
    let mut stage = Stage::build();
    let far = stage.far;
    let (c, filed_at) = stage.recover_and_claim(far);
    let end = window_end(&stage.rig, c);
    assert_eq!(end, filed_at + WINDOW, "the ruled window");
    let o = stage.o;
    let amend = run(&mut stage.rig.tw, o, &format!("@contract/amend #{c}"));
    assert!(
        amend.contains("is a salvage claim"),
        "the amend is refused: {amend}"
    );
    let shorten = run(&mut stage.rig.tw, o, &format!("@contract/deadline #{c}=1"));
    assert!(
        shorten.contains("is a salvage claim"),
        "the deadline is refused: {shorten}"
    );
    let after = contract(&stage.rig, c);
    assert_eq!(after.state, ContractState::Active, "{after:?}");
    assert_eq!(after.amendments, 0);
    assert_eq!(window_end(&stage.rig, c), end, "the window is unchanged");
    // Nothing settles early: a minute of ticks awards nothing, and the claim still stands.
    jump(&mut stage.rig, 60);
    for _ in 0..3 {
        let _ = second(&mut stage.rig);
    }
    let row = custody(&mut stage.rig, far).expect("still in custody");
    assert_eq!(row.claim, ClaimSeam::Filed { contract: c });
    assert_eq!(title(&stage.rig, far), Some(PrincipalId::Object(stage.t)));
    // The title holder's own answer is still open to them.
    let bare = run(
        &mut stage.rig.tw,
        stage.t,
        &format!("+salvage/contest #{}", far.0),
    );
    assert!(bare.contains("Bare title never defeats"), "{bare}");
}

/// **P0-2, path B** (doc 36a §3): the salvor may not dispute their own claim and, alone and
/// so unanimous, agree it to a refund — which would withdraw it and drop a contest on bare
/// title. Both switches are refused; the contract is never `Disputed`.
#[test]
fn p0_2_the_salvor_cannot_dispute_and_agree_their_claim_away() {
    let mut stage = Stage::build();
    let far = stage.far;
    let (c, _) = stage.recover_and_claim(far);
    let o = stage.o;
    let dispute = run(&mut stage.rig.tw, o, &format!("@contract/dispute #{c}"));
    assert!(
        dispute.contains("is a salvage claim"),
        "the dispute is refused: {dispute}"
    );
    assert_eq!(contract(&stage.rig, c).state, ContractState::Active);
    let agree = run(
        &mut stage.rig.tw,
        o,
        &format!("@contract/agree #{c}=refund"),
    );
    assert!(
        agree.contains("is a salvage claim"),
        "the agreement is refused: {agree}"
    );
    let cancel = run(&mut stage.rig.tw, o, &format!("@contract/cancel #{c}"));
    assert!(
        cancel.contains("is a salvage claim"),
        "the cancel is refused: {cancel}"
    );
    let after = contract(&stage.rig, c);
    assert_eq!(after.state, ContractState::Active, "{after:?}");
    assert!(after.agreements.is_empty());
    let _ = second(&mut stage.rig);
    let row = custody(&mut stage.rig, far).expect("still in custody");
    assert_eq!(row.claim, ClaimSeam::Filed { contract: c });
    assert!(row.file.is_some_and(|file| file.state.is_pending()));
}

/// **P0-3** (doc 36a §3; Q-36-29, Q-36-22): a leased rig in a stranger's custody with a claim
/// filed on it is written off after 28 IC days out of its bay. The award is never posted — the
/// title is Occator's — the write-off hands the rig back to the shed, the claim's contract is
/// closed and the custody row goes: the rig is back in the pool, Occator's alone.
#[test]
#[allow(clippy::too_many_lines)]
fn p0_3_a_leased_rig_written_off_in_custody_goes_home_and_is_never_awarded() {
    let mut rig = stocked_tuned(quick);
    // A's leased rig lies disabled beside Ahuna's ramp, indexed by an earlier visit's field.
    let (a, _, tvs) = on_pad(&mut rig, 1);
    let escort = tvs[0];
    let ramp = {
        let (x, y, _) = ramp_at(&rig.tw, AHUNA);
        (x.div_euclid(1_000), y.div_euclid(1_000))
    };
    launched(&mut rig, a, escort);
    let lying = (ramp.0 + 3, ramp.1);
    to_cell(&mut rig, escort, lying);
    powerless(&mut rig, escort);
    let s = staff_member(&mut rig);
    staff(
        &mut rig,
        s,
        &format!(
            "@tacsim/site {AHUNA} leave #{} casualty {},{}",
            escort.0, lying.0, lying.1
        ),
    );
    let occator = title(&rig, escort).expect("the pool rig's title");
    assert!(
        matches!(occator, PrincipalId::Organization(_)),
        "a pool rig is Occator's: {occator:?}"
    );
    let left = site_record(&mut rig)
        .and_then(|record| record.units.first().map(|row| row.left_ic))
        .expect("indexed");
    // The stranger tows it into the Red Moth and files a claim on it.
    let (b, b_row, b_tvs) = on_pad_as(&mut rig, "Second", 2, "Red Moth", 1);
    let b_carrier = ObjId(b_row.hull);
    let tug = b_tvs[0];
    launched(&mut rig, b, tug);
    to_cell(&mut rig, tug, (lying.0 + 1, lying.1));
    let _ = second(&mut rig);
    let out = go(&mut rig, b, &format!("+tv/1 tow #{} go", escort.0));
    assert!(
        seconds_until(&mut rig, 120, |rig| site_record(rig).is_some_and(
            |record| { record.tows.towing(tug).map(|op| op.casualty()) == Some(escort) }
        )),
        "the tow attaches: {out}"
    );
    to_cell(&mut rig, tug, ramp);
    let _ = second(&mut rig);
    let out = go(&mut rig, b, "+tv/1 load go");
    assert!(
        seconds_until(&mut rig, 120, |rig| site_record(rig).is_some_and(
            |record| {
                record.custody.iter().any(|row| row.hull == escort)
                    && record.tows.act_of(tug).is_none()
            }
        )),
        "the load lands in custody: {out}"
    );
    assert!(
        tacsim::surface::tow::in_bay_of(rig.tw.world(), escort, b_carrier),
        "in the stranger's bay"
    );
    let out = go(&mut rig, b, &format!("+salvage/claim #{} go", escort.0));
    assert!(out.contains("You file a salvage claim"), "{out}");
    let row = site_record(&mut rig)
        .and_then(|record| record.custody.into_iter().find(|row| row.hull == escort))
        .expect("in custody");
    let ClaimSeam::Filed { contract: c } = row.claim else {
        panic!("filed: {row:?}");
    };
    // The window runs out long before the write-off: nothing is awarded, the title stays.
    jump(&mut rig, WINDOW + 60);
    for _ in 0..3 {
        let _ = second(&mut rig);
    }
    assert_eq!(title(&rig, escort), Some(occator.clone()), "never awarded");
    assert!(
        site_record(&mut rig).is_some_and(|record| record.custody.iter().any(|r| r.hull == escort)),
        "the claim stands pending"
    );
    // Past the 28 days: written off on the tick, handed back, and the claim closed.
    let bar = tacsim::port::tv::WRITE_OFF_LEFT_IC_DAYS * 86_400;
    skip_to_ic(&mut rig, left + bar + 60);
    assert!(
        seconds_until(&mut rig, 120, |rig| tacsim::port::tv::written_off(
            rig.tw.world(),
            escort
        )),
        "written off"
    );
    assert!(
        seconds_until(&mut rig, 30, |rig| site_record(rig)
            .is_some_and(|record| record.custody.iter().all(|r| r.hull != escort))),
        "the custody row goes"
    );
    // The sweep lapsed the contract at its deadline (`Expired`, terminal); a claim still live
    // would have been cancelled. Either way nothing of it is awarded.
    let closed = contract(&rig, c).state;
    assert!(closed.is_terminal(), "the claim is closed: {closed:?}");
    assert_eq!(title(&rig, escort), Some(occator), "Occator's alone");
    assert!(
        tacsim::port::tv::in_shed(rig.tw.world(), escort),
        "back in Occator's shed"
    );
    assert!(
        !tacsim::surface::tow::in_bay_of(rig.tw.world(), escort, b_carrier),
        "not in the stranger's bay"
    );
    let ended = store(&rig.tw)
        .tv_leases_on(escort)
        .max_by_key(|record| record.lease)
        .cloned()
        .expect("the lease record");
    assert_eq!(ended.status, tacsim::port::tv::TvLeaseStatus::Returned);
    assert!(ended.written_off);
    let listed = run(&mut rig.tw, b, "+salvage");
    assert!(listed.contains("no salvage claim"), "{listed}");
}

/// **P0-7, the boundary** (doc 36a §4): a contest filed in the window's last second used to
/// land on a claim whose title the same tick had posted, so the next tick vacated a void claim
/// with the title gone. Now the window's last second is the award's alone: at `end - 1` the
/// title holder may still answer, at `end` the window has closed, and the tick then awards with
/// the custody row dropped in the same bundle.
#[test]
fn p0_7_the_windows_last_second_belongs_to_the_award_not_a_contest() {
    let mut stage = Stage::build();
    let far = stage.far;
    let (c, filed_at) = stage.recover_and_claim(far);
    let end = window_end(&stage.rig, c);
    assert_eq!(end, filed_at + WINDOW);
    let t = stage.t;
    stage.rig.tw.set_now(end - 1);
    let open = run(
        &mut stage.rig.tw,
        t,
        &format!("+salvage/contest #{}", far.0),
    );
    assert!(
        open.contains("Bare title never defeats"),
        "still open: {open}"
    );
    stage.rig.tw.set_now(end);
    let closed = run(
        &mut stage.rig.tw,
        t,
        &format!("+salvage/contest #{} go", far.0),
    );
    assert!(closed.contains("has closed"), "closed at the end: {closed}");
    assert!(
        seconds_until(&mut stage.rig, 30, |rig| custody(rig, far).is_none()),
        "the uncontested claim settles"
    );
    assert_eq!(title(&stage.rig, far), Some(PrincipalId::Object(stage.o)));
    let again = run(
        &mut stage.rig.tw,
        t,
        &format!("+salvage/contest #{} go", far.0),
    );
    assert!(
        again.contains("No salvage custody names"),
        "nothing left to vacate: {again}"
    );
    assert!(
        record(&mut stage.rig)
            .unlawful
            .iter()
            .all(|act| act.subject != far.0),
        "no vacate was logged against the award"
    );
}

/// **P0-7, the lost `Loaded`** (doc 36a §4): the load's finish is lost after the tick put the
/// casualty in the bay, and the carrier is gone from the pad before the next tick (the stand-in:
/// the carrier unplaced by staff hand, as a lift unplaces it). The next tick's finish still
/// completes the load off the bay's own facts: the casualty is off the site's index, the tow is
/// spent, and the custody row stands.
#[test]
fn p0_7_a_lost_loaded_finish_still_lands_after_the_carrier_has_gone() {
    let mut stage = Stage::build();
    let (far, tug, o, o_carrier) = (stage.far, stage.tug, stage.o, stage.o_carrier);
    stage.attach(far);
    to_cell_on(&mut stage.rig, URVARA, tug, stage.ramp);
    let _ = second(&mut stage.rig);
    let out = go(&mut stage.rig, o, "+tv/1 load go");
    assert!(out.contains("hauls the"), "{out}");
    // The finish lands at its due instant; its submission is drained and lost.
    let mut lost = false;
    for _ in 0..40 {
        let (_, queued) = crate::driving_rig::fire(&mut stage.rig);
        if queued
            .iter()
            .any(|submit| submit.action.key().id.as_str() == tacsim::surface::tow::TOW_ACTION)
        {
            lost = true;
            break;
        }
    }
    assert!(lost, "the load's finish was submitted");
    assert!(
        tacsim::surface::tow::in_bay_of(stage.rig.tw.world(), far, o_carrier),
        "the tick put the casualty in the bay"
    );
    let residue = record(&mut stage.rig);
    assert!(
        residue.units.iter().any(|left| left.unit == far),
        "still indexed"
    );
    assert_eq!(residue.tows.towing(tug).map(|op| op.casualty()), Some(far));
    assert!(residue.tows.act_of(tug).is_some(), "the act is still due");
    // The carrier is gone from the pad before the next tick.
    crate::driving_rig::staff_hand(&mut stage.rig, |ctx| {
        mush_sim::unplace(ctx, o_carrier, "lifted").expect("the carrier lifts");
    });
    assert!(
        mush_sim::pose_of(stage.rig.tw.world(), o_carrier).is_none(),
        "no longer berthed here"
    );
    assert!(
        seconds_until(&mut stage.rig, 30, |rig| {
            let record = record(rig);
            record.units.iter().all(|left| left.unit != far)
                && record.tows.towing(tug).is_none()
                && record.tows.act_of(tug).is_none()
        }),
        "the next tick's finish completes the load"
    );
    let held = custody(&mut stage.rig, far).expect("in O's custody");
    assert_eq!((held.holder, held.carrier), (o, o_carrier));
}

/// **P1-3** (doc 36a §3): the tow orders were one row a hull, so a salvor aboard the carrier
/// spamming `+salvage/inspect #H` near the deadline replaced the challenger's queued contest,
/// which then died unauthorized after CONTEST_FILED had printed. Keyed by actor and hull, the
/// contest lands whatever the salvor types, and it vacates the unlawful claim.
#[test]
fn p1_3_the_salvors_inspect_spam_does_not_kill_a_queued_contest() {
    let mut stage = Stage::build();
    let (near, t, o) = (stage.near, stage.t, stage.o);
    let (c, _) = stage.recover_and_claim(near);
    // T's contest is queued, not yet landed; O then spams the inspection three times.
    let (out, contest) = command(
        &mut stage.rig,
        t,
        &format!("+salvage/contest #{} go", near.0),
    );
    assert!(out.contains("You file your contest"), "{out}");
    assert_eq!(contest.len(), 1, "one contest queued: {out}");
    let mut inspects = Vec::new();
    for _ in 0..3 {
        let (out, queued) = command(&mut stage.rig, o, &format!("+salvage/inspect #{}", near.0));
        assert!(out.contains("black box"), "{out}");
        inspects.extend(queued);
    }
    assert_eq!(inspects.len(), 3);
    crate::driving_rig::land(&mut stage.rig, &contest[0])
        .expect("the contest lands after the spam");
    for submit in &inspects {
        // O's own later inspections replaced O's earlier ones: the first two have no order
        // left to land on, the last lands.
        let _ = crate::driving_rig::land(&mut stage.rig, submit);
    }
    let row = custody(&mut stage.rig, near).expect("still in custody until the tick");
    assert_eq!(row.claim, ClaimSeam::Filed { contract: c });
    let file = row.file.as_deref().expect("the claim");
    assert_eq!(file.contests.len(), 1, "{file:?}");
    assert!(file.has_vacating_contest(), "{file:?}");
    assert!(
        row.evidence.iter().any(|e| e.holder == o),
        "O's inspection was recorded too: {:?}",
        row.evidence
    );
    assert!(
        seconds_until(&mut stage.rig, 10, |rig| custody(rig, near).is_none()),
        "the contest vacates the claim"
    );
    assert_eq!(contract(&stage.rig, c).state, ContractState::Cancelled);
    assert_eq!(title(&stage.rig, near), Some(PrincipalId::Object(t)));
}

/// **P1-4** (doc 36a §3): tug B's attach during tug A's 900 IC-second act replaced A's tenure
/// pin; A landed first and loaded with no pin, so no `Recovery` entry was logged, no witnesses
/// were kept and `+salvage/claim` answered NO_TITLE for good. Now the second attach start waits
/// while the first is in flight: O's attach lands, the load carries O's pin into custody with
/// the unlawful recovery logged, and O's claim is open to file.
#[test]
fn p1_4_a_second_tugs_attach_waits_and_the_first_tugs_pin_stands() {
    let mut stage = Stage::build();
    let (near, near_cell, t, o, tug, t_tug, l) = (
        stage.near,
        stage.near_cell,
        stage.t,
        stage.o,
        stage.tug,
        stage.t_tug,
        stage.l,
    );
    // Both tugs stand beside the casualty; O's attach starts (in flight for a minute of IC
    // time), and T's rig tries to take it in the same second: refused.
    launched_at(&mut stage.rig, URVARA, t, t_tug);
    to_cell_on(
        &mut stage.rig,
        URVARA,
        t_tug,
        (near_cell.0, near_cell.1 + 1),
    );
    to_cell_on(&mut stage.rig, URVARA, tug, (near_cell.0 + 1, near_cell.1));
    let _ = second(&mut stage.rig);
    let out = go(&mut stage.rig, o, &format!("+tv/1 tow #{} go", near.0));
    assert!(out.contains("makes fast"), "{out}");
    assert!(record(&mut stage.rig).tows.act_of(tug).is_some());
    let refused = run(&mut stage.rig.tw, t, &format!("+tv/1 tow #{} go", near.0));
    assert!(
        refused.contains("Another tug is making fast"),
        "the second attach waits: {refused}"
    );
    assert!(
        record(&mut stage.rig).tows.act_of(t_tug).is_none(),
        "nothing of T's is in flight"
    );
    let pin = record(&mut stage.rig)
        .tows
        .pins
        .into_iter()
        .find(|pin| pin.casualty == near)
        .expect("O's pin stands");
    assert_eq!(pin.tug, tug, "{pin:?}");
    // O's attach lands; O loads into the Red Moth: the pin rides into custody.
    assert!(
        seconds_until(&mut stage.rig, 120, |rig| towing(rig, tug) == Some(near)),
        "O's attach lands"
    );
    let held = stage.load(near);
    assert_eq!(held.pin.as_ref(), Some(&pin), "the pin rides into custody");
    assert_eq!(pin.landholder(), Some(l));
    assert_eq!(pin.answering, Some(t));
    assert!(pin.witnesses.iter().any(|w| w.holder == t), "{pin:?}");
    assert!(
        record(&mut stage.rig).unlawful.iter().any(|act| act.kind
            == tacsim::surface::claim::UnlawfulKind::Recovery
            && act.actor == o
            && act.subject == near.0
            && act.landholder == Some(l)),
        "the unlawful recovery is evidence on the site's log"
    );
    let preview = run(&mut stage.rig.tw, o, &format!("+salvage/claim #{}", near.0));
    assert!(
        preview.contains("You would file a salvage claim") && !preview.contains("carries no title"),
        "{preview}"
    );
}

/// **P1-10** (doc 36a §3): every other player arm of the tow action refused under `promoted`;
/// `Release` did not, and the field gate had run at command time. A release queued before a
/// hostile mark promotes the site, and draining after it, retired the tow with the casualty
/// neither fielded nor carried. Now it waits (FIELD_OPEN), and the tow stands.
#[test]
fn p1_10_a_release_draining_after_a_promotion_waits() {
    let mut stage = Stage::build();
    let (far, far_cell, t, o, tug) = (stage.far, stage.far_cell, stage.t, stage.o, stage.tug);
    stage.attach(far);
    // The release is queued on the open ground...
    let (out, release) = command(&mut stage.rig, o, "+tv/1 release go");
    assert!(out.contains("lets go of"), "{out}");
    assert_eq!(release.len(), 1, "one release queued: {out}");
    // ...and a stranger's hostile mark promotes the site before it drains.
    let hull = armed_on(
        &mut stage.rig,
        URVARA,
        "Red Hound",
        (far_cell.0 - 8, far_cell.1),
        90_000,
    );
    seat(&mut stage.rig, hull, t);
    crate::driving_rig::warm(&mut stage.rig);
    let (out, mark) = command(&mut stage.rig, t, "+field/mark E=hostile");
    assert_eq!(mark.len(), 1, "one promotion: {out}");
    crate::driving_rig::land(&mut stage.rig, &mark[0]).expect("the promotion lands");
    assert!(
        record(&mut stage.rig).open_field().is_some(),
        "the site is promoted"
    );
    let refused = crate::driving_rig::land(&mut stage.rig, &release[0])
        .expect_err("the release waits for the field");
    assert!(
        refused.contains("a field is being fought"),
        "FIELD_OPEN: {refused}"
    );
    let after = record(&mut stage.rig);
    assert_eq!(
        after.tows.towing(tug).map(|op| op.casualty()),
        Some(far),
        "the tow stands"
    );
    assert!(
        after.units.iter().any(|left| left.unit == far),
        "the casualty is still indexed"
    );
}

/// **P2-2** (doc 36a §3): the pickup moved a crate's goods into the hopper with no capacity
/// check anywhere, told the player the rig "takes" the crate when it had only queued the act,
/// and logged an unlawful pickup with no landholder. Now the command refuses a crate the hopper
/// cannot take (the mining plan's merged-compartment rule), says "queued", and the prepare
/// re-sums the hopper's pools against the facts' limits; the evidence row names the landholder.
/// The last step's hopper fill between the queue and the landing is a staff fixture (a run's
/// ore would do the same); the landing itself goes through the production path.
#[test]
fn p2_2_a_crate_the_hopper_cannot_take_is_refused_and_the_landholder_is_named() {
    let mut stage = Stage::build();
    let (near_cell, o, tug, l, s) = (stage.near_cell, stage.o, stage.tug, stage.l, stage.staff);
    to_cell_on(&mut stage.rig, URVARA, tug, (near_cell.0 + 1, near_cell.1));
    let _ = second(&mut stage.rig);
    // Three crates beside the rig on L's block, their goods on Urvara's spoil: one too heavy
    // for a 12 t hopper, one that fits, one that fits only while the hopper stays as it is.
    for (qty, dx) in [(13_u64, 1_i64), (2, 2), (10, 0)] {
        staff(
            &mut stage.rig,
            s,
            &format!(
                "@tacsim/site {URVARA} drop crate {qty} metal_ore {},{}",
                near_cell.0 + dx,
                near_cell.1 + 1
            ),
        );
    }
    let spoil = mush_economy::VaultId(
        port_map(&stage.rig.tw, URVARA)
            .site
            .as_ref()
            .and_then(|site| site.spoil_vault)
            .expect("Urvara's spoil vault"),
    );
    seed_pool(
        &mut stage.rig,
        spoil,
        "metal_ore",
        25,
        mush_inventory::LotId(9_601),
    );
    let crate_of = |rig: &mut Rig, qty: u64| -> u32 {
        record(rig)
            .objects
            .iter()
            .find(|object| {
                matches!(
                    &object.kind,
                    tacsim::surface::site_record::GroundKind::Cargo { manifest, .. }
                        if manifest.get(&cid("metal_ore")) == Some(&qty)
                )
            })
            .map(|object| object.id)
            .expect("the crate is on the record")
    };
    let (heavy, light, filling) = (
        crate_of(&mut stage.rig, 13),
        crate_of(&mut stage.rig, 2),
        crate_of(&mut stage.rig, 10),
    );
    let hopper = tacsim::port::vehicle::hopper_of(stage.rig.tw.world(), tug).expect("a hopper");

    // The heavy crate is refused at the command: nothing is queued, nothing moves.
    let (out, queued) = command(&mut stage.rig, o, &format!("+tv/1 pickup {heavy} go"));
    assert!(
        out.contains("hopper has no room for a crate of 13 metal_ore")
            && out.contains("12000 kg free")
            && out.contains("13000 kg"),
        "{out}"
    );
    assert!(queued.is_empty(), "nothing queued: {out}");
    assert_eq!(hold_pool(&mut stage.rig.tw, hopper, "metal_ore"), 0);

    // The light crate is queued (and said so), lands, and is logged against L's ground.
    let (out, queued) = command(&mut stage.rig, o, &format!("+tv/1 pickup {light} go"));
    assert!(
        out.contains("will take a crate of 2 metal_ore aboard into its hopper: queued"),
        "{out}"
    );
    assert!(!out.contains("takes "), "no claim of a done deed: {out}");
    assert_eq!(queued.len(), 1, "one pickup: {out}");
    crate::driving_rig::land(&mut stage.rig, &queued[0]).expect("the pickup lands");
    assert_eq!(hold_pool(&mut stage.rig.tw, hopper, "metal_ore"), 2);
    let logged = record(&mut stage.rig);
    let row = logged
        .unlawful
        .iter()
        .find(|act| act.kind == tacsim::surface::claim::UnlawfulKind::Pickup)
        .expect("the pickup off L's ground is evidence");
    assert_eq!(
        (row.actor, row.subject, row.landholder),
        (o, light, Some(l))
    );

    // The filling crate fits now (2 t + 10 t); queued, then the hopper takes 1 t more before
    // the landing, and the prepare's re-check refuses it.
    let (out, queued) = command(&mut stage.rig, o, &format!("+tv/1 pickup {filling} go"));
    assert_eq!(queued.len(), 1, "queued while it fit: {out}");
    seed_pool(
        &mut stage.rig,
        hopper,
        "metal_ore",
        1,
        mush_inventory::LotId(9_602),
    );
    let refused =
        crate::driving_rig::land(&mut stage.rig, &queued[0]).expect_err("the hopper is now short");
    assert!(refused.contains("pickup_no_room"), "{refused}");
    assert_eq!(
        hold_pool(&mut stage.rig.tw, hopper, "metal_ore"),
        3,
        "nothing moved"
    );
    assert!(
        record(&mut stage.rig)
            .objects
            .iter()
            .any(|object| object.id == filling),
        "the crate is still on the ground"
    );
}

/// **P2-TSW-1**:
/// 1. Dropping an uncommitted `ClaimOrder::File` must NOT send IC mail and must clean up the
///    orphaned unattached economy contract on the next surface tick.
/// 2. A stale/expired unattached economy salvage contract must be closed and replaced with a fresh
///    full-window contract instead of copying an expired `window_end` into a zero-second claim.
/// 3. Once `ClaimOrder::File` commits, `claim::tick` delivers the deferred IC mail exactly once.
/// 4. If the actor already has an authoritative submission pending (`authoritative_pending`),
///    `+salvage/claim <hull> go` rejects upfront without creating an economy contract.
#[test]
fn p2_tsw_1_stale_unattached_contract_replaced_and_claim_mail_deferred() {
    use std::sync::{Arc, Mutex};

    let mut stage = Stage::build();
    let (o, t, far) = (stage.o, stage.t, stage.far);
    stage.attach(far);
    stage.load(far);

    let posted: Arc<Mutex<Vec<Vec<ObjId>>>> = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&posted);
    assert!(tacsim::surface::claim::install_claim_mail(
        stage.rig.tw.world_mut(),
        Arc::new(move |_ctx, _from, to, _subject, _body| {
            log.lock().expect("mail log").push(to.to_vec());
            Ok(())
        }),
    ));

    // (1) Uncommitted ClaimOrder::File: mail is NOT delivered before commit, and if the submission
    //     never commits, the next surface tick discards the notice and closes the orphaned
    //     economy contract.
    let (out, queued) = command(&mut stage.rig, o, &format!("+salvage/claim #{} go", far.0));
    assert!(out.contains("You file a salvage claim"), "{out}");
    assert_eq!(queued.len(), 1, "one ClaimOrder::File queued");
    assert!(
        posted.lock().expect("mail log").is_empty(),
        "IC mail must be deferred until ClaimOrder::File commits"
    );
    assert!(
        mush_economy::live_salvage_claim(stage.rig.tw.world(), o, far).is_some(),
        "economy contract was opened for the filing"
    );
    // Drop `queued` without landing it; next surface tick sees ClaimSeam::Owed with no pending
    // submission, drops the mail, and closes the orphaned unattached economy contract.
    let _ = second(&mut stage.rig);
    assert!(
        posted.lock().expect("mail log").is_empty(),
        "uncommitted filing must never spam IC mail"
    );
    assert!(
        mush_economy::live_salvage_claim(stage.rig.tw.world(), o, far).is_none(),
        "orphaned unattached economy contract must be closed after dropped filing"
    );

    // (2) Stale/expired unattached economy contract: even if an unattached contract exists and
    //     time advances past its deadline, `+salvage/claim <hull> go` closes & recreates it with
    //     a fresh full contest window instead of copying the expired `window_end`.
    let stale_id = crate::driving_rig::staff_hand(&mut stage.rig, |ctx| {
        let remainder = tacsim::port::credit::own_account(ctx.w, o)
            .expect("O's account")
            .id;
        mush_economy::file_salvage_claim(
            ctx,
            mush_economy::SalvageClaimSpec {
                name: smol_str::SmolStr::new("Stale salvage claim"),
                claimant: o,
                hull: far,
                audience: vec![PrincipalId::Object(t)],
                window_secs: WINDOW,
                remainder_to: remainder,
            },
        )
        .expect("stale unattached contract")
    });
    let expired_at = stage.rig.tw.world().now() + WINDOW + 120;
    stage.rig.tw.set_now(expired_at);

    // (4) While `o` already has an authoritative submission pending in the engine queue,
    //     `+salvage/claim` rejects upfront without touching the stale contract or opening a new
    //     one.
    let inspected = run(
        &mut stage.rig.tw,
        o,
        &format!("+salvage/inspect #{}", far.0),
    );
    assert!(
        inspected.contains("black box"),
        "inspect queued an authoritative submission: {inspected}"
    );
    let blocked = run(
        &mut stage.rig.tw,
        o,
        &format!("+salvage/claim #{} go", far.0),
    );
    assert!(
        !blocked.contains("You file a salvage claim"),
        "pending authoritative submission must block claim_cmd upfront: {blocked}"
    );
    // Drain the inspect submission without landing it so `o` is idle again.
    let drained = crate::driving_rig::drain(&mut stage.rig.tw);
    assert_eq!(
        drained.len(),
        1,
        "only the pending inspect submission was queued; blocked claim queued nothing"
    );

    // Now file cleanly: stale contract is replaced with a fresh full-window contract.
    let filed_now = stage.rig.tw.world().now();
    let out = go(&mut stage.rig, o, &format!("+salvage/claim #{} go", far.0));
    assert!(out.contains("You file a salvage claim"), "{out}");
    let row = custody(&mut stage.rig, far).expect("custody row");
    let ClaimSeam::Filed { contract: fresh_id } = row.claim else {
        panic!("expected Filed claim: {row:?}");
    };
    assert_ne!(
        fresh_id, stale_id.0,
        "stale unattached economy contract must be closed and replaced"
    );
    let file = row.file.as_deref().expect("claim file");
    let remaining = file.deadline_real_s.saturating_sub(filed_now);
    assert!(
        remaining >= WINDOW - 60,
        "attached claim must carry a full contest window, got remaining={remaining}s"
    );

    // (3) Next surface tick flushes the deferred IC mail once.
    let _ = second(&mut stage.rig);
    assert_eq!(
        *posted.lock().expect("mail log"),
        vec![vec![t]],
        "committed filing delivers IC mail exactly once"
    );
}

/// **P2-TSW-13**: filling `documented_repairs_at_risk` to `MAX_AT_RISK` (16) while a 17th
/// completed bay repair exists on the hull must NOT wedge `claim::tick` in an infinite
/// `ClaimOrder::AtRisk` loop ahead of `Settlement::Vacate`.
#[test]
fn p2_tsw_13_saturated_at_risk_tick_vacates_without_deadlock() {
    use tacsim::port::bay_work::{BayHullState, OrderStatus};
    use tacsim::surface::claim::MAX_AT_RISK;
    use tacsim::surface::site_record::SiteKey;

    let mut stage = Stage::build();
    let (o, t, near, o_carrier) = (stage.o, stage.t, stage.near, stage.o_carrier);
    stage.attach(near);
    let _ = stage.load(near);

    // Jam a component on `near`, stand O in O's carrier bay, and complete a real salvor bay
    // unjam order BEFORE filing the claim (so its IC duration does not expire the short 10-minute
    // test contest window), giving `at_risk_of` a fresh completed repair entry on every tick.
    let comp_id = mush_tactics::module::unit_of(stage.rig.tw.world(), near)
        .expect("near unit")
        .components()
        .next()
        .map(|(id, _)| id.clone())
        .expect("at least one component");
    crate::driving_rig::staff_hand(&mut stage.rig, |ctx| {
        let plan = mush_tactics::UnitMutationPlan::new(
            vec![mush_tactics::UnitMutationOp::SetComponentCondition {
                component: comp_id,
                condition: mush_tactics::ConditionId::new("jammed").expect("jammed"),
            }],
            None,
            Some(smol_str::SmolStr::new_static("test:p2-tsw-13-jam")),
        )
        .expect("jam plan");
        mush_tactics::module::apply_plan(ctx, near, &plan).expect("jam applied");
    });
    for step in ["inner", "hold"] {
        let _ = run(&mut stage.rig.tw, o, step);
    }
    let entered = run(&mut stage.rig.tw, o, "+tv/bay in");
    assert!(entered.contains("entered vehicle bay"), "{entered}");
    let _ = o_carrier;
    let ordered = run(
        &mut stage.rig.tw,
        o,
        &format!("+bay/order #{}=unjam", near.0),
    );
    assert!(ordered.contains("taken"), "bay order taken: {ordered}");
    let due_ic = match stage
        .rig
        .tw
        .world()
        .module_state::<tacsim::TacsimStore>()
        .and_then(|store| store.bay_hull(near))
    {
        Some(BayHullState::Present(bay)) => bay.order.as_ref().expect("bay order").due_ic,
        other => panic!("expected bay order: {other:?}"),
    };
    let _ = crate::driving_rig::to_ic(&mut stage.rig, due_ic + 60);
    let done = matches!(
        stage
            .rig
            .tw
            .world()
            .module_state::<tacsim::TacsimStore>()
            .and_then(|store| store.bay_hull(near)),
        Some(BayHullState::Present(bay))
            if bay.order.as_ref().is_some_and(|ord| ord.status == OrderStatus::Done)
    );
    assert!(done, "bay unjam order completed");

    // File the salvage claim now, then replace its `documented_repairs_at_risk` with MAX_AT_RISK
    // (16) distinct earlier entries so the completed bay unjam order above is a 17th entry past
    // `MAX_AT_RISK`.
    let out = go(&mut stage.rig, o, &format!("+salvage/claim #{} go", near.0));
    assert!(out.contains("You file a salvage claim"), "{out}");
    let row = custody(&mut stage.rig, near).expect("still in custody");
    let ClaimSeam::Filed { contract: claim_id } = row.claim else {
        panic!("filed: {row:?}");
    };
    let mut rec = record(&mut stage.rig);
    let site_id = cid(URVARA);
    {
        let file = rec
            .custody
            .iter_mut()
            .find(|row| row.hull == near)
            .expect("custody")
            .file
            .as_deref_mut()
            .expect("filed claim");
        file.documented_repairs_at_risk.clear();
        for i in 0..MAX_AT_RISK {
            file.documented_repairs_at_risk
                .push(tacsim::field_ops::AtRiskRepairEntry {
                    description: smol_str::SmolStr::new(format!("prior patch #{i}")),
                    cost_incurred: 1_000,
                    performed_at_ic: 1_000 + i64::try_from(i).expect("small"),
                });
        }
    }
    let handles = stage
        .rig
        .tw
        .world()
        .module_state::<tacsim::port::PortHandles>()
        .cloned()
        .expect("port handles");
    crate::driving_rig::reseed_typed(
        &mut stage.rig,
        &handles.families.sites,
        &SiteKey(site_id),
        rec,
    );

    // Title owner T (who holds sensor witness evidence of the unlawful recovery on L's block)
    // contests the claim.
    let contested = go(
        &mut stage.rig,
        t,
        &format!("+salvage/contest #{} go", near.0),
    );
    assert!(contested.contains("You file your contest"), "{contested}");

    // Drive surface ticks: claim::tick must settle Settlement::Vacate cleanly rather than
    // looping on a doomed 17th ClaimOrder::AtRisk.
    assert!(
        seconds_until(&mut stage.rig, 10, |rig| custody(rig, near).is_none()),
        "saturated MAX_AT_RISK must not deadlock Settlement::Vacate; contract={:?}",
        contract(&stage.rig, claim_id).state
    );
    assert_eq!(
        contract(&stage.rig, claim_id).state,
        mush_economy::ContractState::Cancelled,
        "vacated salvage contract must settle to Cancelled"
    );
}
