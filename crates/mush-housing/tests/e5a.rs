//! E5a acceptance — the pure housing library on hand-built records, in the
//! E2a/E3a/E4a rhythm: every arrow of the tenancy state machine and every forbidden
//! one, the story window's notice decay and escalation timing, the best-of support
//! assessment, the fail-closed access predicate, the `Obj.home` invariant helpers,
//! every §9 cap, a serde round-trip of every record, and a transition fuzz.
//!
//! Everything here is world-free. There is no `World`, no clock, and no ruleset:
//! `now` is a position on the tenancy's own cadence and every input is a value.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::ObjId;
use mush_economy::{Amount, CadenceId, ContractId, EscalationId, ObligationId};
use mush_housing::{
    Access, AccessTerms, Assessment, DELINQUENT_PERIODS, Feature, Funding, GRACE_PERIODS,
    HousingError, InspectionTerm, InspectionWhen, MAX_FEATURE_KEY_BYTES, MAX_FEATURES, MAX_GUESTS,
    MAX_PLAN_SOURCES, MAX_PROPERTY_NAME_BYTES, MAX_REASON_BYTES, MAX_ROOMS_PER_PROPERTY,
    MAX_TENANCY_HISTORY, MAX_TENANTS, MAX_WINDOW_NOTICES, Property, PropertyId, PropertyStatus,
    PropertyTier, Resolution, SceneRef, StoryWindow, SupportLevel, SupportPlan, SupportSource,
    Tenancy, TenancyEvent, TenancyId, TenancyStatus, TenancyStatusKind, UNKNOWN_GRACE,
    WINDOW_ESCALATE, WINDOW_ESCALATED_RENOTIFY, WINDOW_RENOTIFY, WindowAction, accept, advance,
    assess, attach_window, best_level, delinquent_elapsed, grace_elapsed, harden, homed_inside,
    homed_on_activate, may_enter, may_enter_with, may_take, period_missed, period_supported,
    periods_since, resolve, submit_plan, to_unhome_on_end, window_due, window_elapsed,
};
use mush_org::{OrgId, OrgScopeId, PrincipalId, SystemPrincipalId};

// ---- builders -------------------------------------------------------------

const ANCHOR: u32 = 100;
const ROOM_B: u32 = 101;
const TENANT: u32 = 10;
const CO_TENANT: u32 = 11;
const GUEST: u32 = 20;
const STRANGER: u32 = 30;
const INSPECTOR: u32 = 40;
const STAFFER: u32 = 50;

fn obj(n: u32) -> ObjId {
    ObjId(n)
}
fn housing_principal() -> PrincipalId {
    PrincipalId::System(SystemPrincipalId::new("wod.housing").expect("a valid system principal"))
}
fn rooms() -> BTreeSet<ObjId> {
    BTreeSet::from([obj(ANCHOR), obj(ROOM_B)])
}
fn property(tier: u8) -> Property {
    Property::new(
        PropertyId(1),
        "The Rookery",
        obj(ANCHOR),
        rooms(),
        PropertyTier::new(tier),
        housing_principal(),
    )
    .expect("a well-formed property")
}
fn tenancy(tenants: &[u32]) -> Tenancy {
    Tenancy::new(
        TenancyId(1),
        PropertyId(1),
        tenants.iter().copied().map(obj).collect(),
        CadenceId(7),
    )
    .expect("a well-formed tenancy")
}
fn plan(resident: u32, level: u8, at: i64) -> SupportPlan {
    SupportPlan::new(obj(resident), SupportLevel::new(level), Vec::new(), at)
        .expect("a well-formed plan")
}
/// A tenancy live at period 0 with one tenant and one guest.
fn live(tenants: &[u32]) -> Tenancy {
    let t = tenancy(tenants);
    let all: BTreeSet<ObjId> = tenants.iter().copied().map(obj).collect();
    let t = accept(&t, true, &all, 0).expect("both sides accept");
    t.add_guest(obj(GUEST)).expect("a guest fits")
}
/// Walk a live tenancy all the way to `EvictionPending` on unsupported periods.
/// Returns the tenancy and the period the window opened at.
fn to_window(t: &Tenancy) -> (Tenancy, i64) {
    let mut cur = t.clone();
    for now in 0..=3 {
        cur = advance(&cur, Assessment::Unsupported, now).expect("the ladder advances");
    }
    (cur, 3)
}

// ===========================================================================
// The state machine — the arrows
// ===========================================================================

#[test]
fn accept_needs_the_landlord_and_every_tenant_and_records_homed() {
    let t = tenancy(&[TENANT, CO_TENANT]);
    assert_eq!(t.status, TenancyStatus::Pending);
    let both = BTreeSet::from([obj(TENANT), obj(CO_TENANT)]);
    let one = BTreeSet::from([obj(TENANT)]);

    // No landlord, and a co-tenant who never agreed: both refused, and neither
    // leaves a half-live tenancy behind.
    assert_eq!(accept(&t, false, &both, 0), Err(HousingError::NotAccepted));
    assert_eq!(accept(&t, true, &one, 0), Err(HousingError::NotAccepted));

    let live = accept(&t, true, &both, 0).expect("both sides accept");
    assert_eq!(live.status, TenancyStatus::Active);
    assert_eq!(live.homed, BTreeSet::from([obj(TENANT), obj(CO_TENANT)]));
    assert!(live.history.contains(&TenancyEvent::Accepted { at: 0 }));
    assert!(live.revision > t.revision);
}

#[test]
fn accept_is_refused_on_every_status_but_pending() {
    let t = live(&[TENANT]);
    assert_eq!(
        accept(&t, true, &BTreeSet::from([obj(TENANT)]), 1),
        Err(HousingError::State(TenancyId(1)))
    );
    let ended = resolve(&t, Resolution::TenantLeft, 1).expect("a tenant may leave");
    assert_eq!(
        accept(&ended, true, &BTreeSet::from([obj(TENANT)]), 2),
        Err(HousingError::AlreadyEnded(TenancyId(1)))
    );
}

#[test]
fn the_ladder_walks_active_to_the_window_in_three_periods() {
    let t = live(&[TENANT]);
    let g = advance(&t, Assessment::Unsupported, 0).expect("miss");
    assert_eq!(g.status, TenancyStatus::Grace { since: 0 });

    // A second miss inside the same period changes nothing (the run is not restamped).
    let same = advance(&g, Assessment::Unsupported, 0).expect("still in grace");
    assert_eq!(same.status, TenancyStatus::Grace { since: 0 });

    let d = advance(&g, Assessment::Unsupported, GRACE_PERIODS).expect("grace elapsed");
    assert_eq!(d.status, TenancyStatus::Delinquent { since: 1 });

    // One period into delinquency is not yet two.
    let waiting = advance(&d, Assessment::Unsupported, 2).expect("not due");
    assert_eq!(waiting.status, TenancyStatus::Delinquent { since: 1 });

    let w = advance(&waiting, Assessment::Unsupported, 1 + DELINQUENT_PERIODS).expect("window");
    assert_eq!(w.status.kind(), TenancyStatusKind::EvictionPending);
    assert_eq!(w.status.since(), Some(3), "three periods from the first miss");
}

#[test]
fn a_miss_inside_the_ladder_never_restamps_the_run() {
    let t = live(&[TENANT]);
    let g = period_missed(&t, 0).expect("miss");
    // Ten more unsupported periods, each of which would reset `since` if the arrow
    // restamped — and the tenancy would then never reach `Delinquent`.
    let mut cur = g;
    for now in 1..=10 {
        cur = period_missed(&cur, now).expect("still missing");
    }
    assert_eq!(cur.status, TenancyStatus::Grace { since: 0 });
}

#[test]
fn the_elapse_arrows_refuse_off_their_state_and_before_their_period() {
    let t = live(&[TENANT]);
    // Off-state is `State`; too early is `NotDue`. The two answers are different on
    // purpose: one is a retry, the other is a bug.
    assert_eq!(grace_elapsed(&t, 5), Err(HousingError::State(TenancyId(1))));
    assert_eq!(delinquent_elapsed(&t, 5), Err(HousingError::State(TenancyId(1))));

    let g = period_missed(&t, 0).expect("miss");
    assert_eq!(grace_elapsed(&g, 0), Err(HousingError::NotDue));
    assert_eq!(delinquent_elapsed(&g, 9), Err(HousingError::State(TenancyId(1))));

    let d = grace_elapsed(&g, 1).expect("grace elapsed");
    assert_eq!(delinquent_elapsed(&d, 2), Err(HousingError::NotDue));
    assert_eq!(grace_elapsed(&d, 9), Err(HousingError::State(TenancyId(1))));
    assert!(delinquent_elapsed(&d, 3).is_ok());
}

#[test]
fn recovery_returns_to_active_silently_from_every_failure_state() {
    let t = live(&[TENANT]);
    let g = period_missed(&t, 0).expect("miss");
    let d = grace_elapsed(&g, 1).expect("grace elapsed");
    let (w, opened) = to_window(&t);

    for (label, from) in [("grace", g), ("delinquent", d), ("window", w)] {
        let back = period_supported(&from, opened + 5).expect("recovery");
        assert_eq!(back.status, TenancyStatus::Active, "recovered from {label}");
        assert!(back.status.since().is_none());
    }
    // Recovery from the window closes it, so the notice machinery has nothing left.
    let (w, _) = to_window(&t);
    let back = period_supported(&w, 9).expect("recovery");
    assert_eq!(window_due(&back, 99), WindowAction::Silent);
    // Already-active is unchanged and does not even bump the revision — a routine
    // supported period says and does nothing.
    let quiet = period_supported(&back, 10).expect("nothing to do");
    assert_eq!(quiet, back);
}

#[test]
fn a_pending_tenancy_is_not_in_the_ladder() {
    let t = tenancy(&[TENANT]);
    assert_eq!(period_missed(&t, 0), Err(HousingError::State(TenancyId(1))));
    assert_eq!(period_supported(&t, 0), Err(HousingError::State(TenancyId(1))));
    assert_eq!(
        advance(&t, Assessment::Unsupported, 0),
        Err(HousingError::State(TenancyId(1)))
    );
    assert_eq!(window_elapsed(&t, 0, None), Err(HousingError::NoWindow(TenancyId(1))));
}

#[test]
fn nothing_moves_an_ended_tenancy() {
    let t = live(&[TENANT]);
    let e = resolve(&t, Resolution::TenantLeft, 1).expect("a tenant may leave");
    let ended = Err(HousingError::AlreadyEnded(TenancyId(1)));
    assert_eq!(period_missed(&e, 2), ended);
    assert_eq!(period_supported(&e, 2), ended);
    assert_eq!(grace_elapsed(&e, 2), ended);
    assert_eq!(delinquent_elapsed(&e, 2), ended);
    assert_eq!(window_elapsed(&e, 2, None), ended);
    assert_eq!(attach_window(&e, Some(1), None), ended);
    assert_eq!(advance(&e, Assessment::Supported, 2), ended);
    assert_eq!(resolve(&e, Resolution::Settled, 2), ended);
    assert_eq!(e.add_guest(obj(STRANGER)), ended);
    assert_eq!(submit_plan(&e, plan(TENANT, 5, 2)), ended);
}

#[test]
fn only_an_explicit_resolution_reaches_ended_and_it_always_carries_one() {
    let t = live(&[TENANT]);
    // The whole automatic half, driven to exhaustion, never ends anything.
    let mut cur = t.clone();
    for now in 0..40 {
        cur = advance(&cur, Assessment::Unsupported, now).expect("the ladder advances");
        if window_due(&cur, now) != WindowAction::Silent {
            cur = window_elapsed(&cur, now, Some(EscalationId(1))).expect("a notice");
        }
        assert!(!cur.status.is_ended(), "automatic processing ended a tenancy at period {now}");
    }
    assert_eq!(cur.status.kind(), TenancyStatusKind::EvictionPending);

    for how in [
        Resolution::TenantLeft,
        Resolution::staff_ended(obj(STAFFER), "after the scene").expect("a bounded reason"),
        Resolution::Waived { by: obj(STAFFER) },
        Resolution::Settled,
    ] {
        let e = resolve(&cur, how.clone(), 99).expect("an explicit act ends it");
        assert_eq!(e.status, TenancyStatus::Ended { how: how.clone() });
        assert!(e.history.contains(&TenancyEvent::Ended { how, at: 99 }));
        assert_eq!(
            resolve(&e, Resolution::Settled, 100),
            Err(HousingError::AlreadyEnded(TenancyId(1))),
            "a second resolution cannot overwrite the record of what happened"
        );
    }
}

#[test]
fn a_pending_offer_may_be_declined_and_the_last_tenant_leaves_only_by_resolution() {
    let t = tenancy(&[TENANT, CO_TENANT]);
    let declined = resolve(&t, Resolution::TenantLeft, 0).expect("a declined offer ends");
    assert_eq!(declined.status, TenancyStatus::Ended { how: Resolution::TenantLeft });

    let l = live(&[TENANT, CO_TENANT]);
    let one = l.remove_tenant(obj(CO_TENANT), 1).expect("a co-tenant may leave");
    assert_eq!(one.tenants, BTreeSet::from([obj(TENANT)]));
    assert!(!one.homed.contains(&obj(CO_TENANT)));
    assert!(one.history.contains(&TenancyEvent::TenantRemoved { who: obj(CO_TENANT), at: 1 }));
    assert_eq!(
        one.remove_tenant(obj(TENANT), 2),
        Err(HousingError::Invalid("the last tenant leaves through an explicit resolution")),
        "removing the last tenant would end a tenancy nobody resolved"
    );
    // Removing somebody who is not a tenant is a no-op, not an error.
    assert_eq!(one.remove_tenant(obj(STRANGER), 3), Ok(one.clone()));
}

#[test]
fn a_clock_that_moved_backwards_elapses_nothing() {
    let t = live(&[TENANT]);
    let g = period_missed(&t, 100).expect("miss");
    // `now` behind `since` — an epoch change, a re-pointed cadence, a restored
    // backup. Zero periods elapsed, so nothing is due, and nothing wraps.
    assert_eq!(periods_since(100, 5), Ok(0));
    assert_eq!(grace_elapsed(&g, 5), Err(HousingError::NotDue));
    assert_eq!(advance(&g, Assessment::Unsupported, 5).map(|t| t.status), Ok(g.status.clone()));
}

#[test]
fn period_arithmetic_is_checked_at_the_i64_extremes() {
    assert_eq!(periods_since(i64::MIN, i64::MAX), Err(HousingError::Overflow));
    assert_eq!(periods_since(0, i64::MAX), Ok(i64::MAX));
    assert_eq!(periods_since(i64::MAX, i64::MAX), Ok(0));
    // The overflow propagates as a refusal, never as a fired arrow.
    let mut g = live(&[TENANT]);
    g.status = TenancyStatus::Grace { since: i64::MIN };
    assert_eq!(grace_elapsed(&g, i64::MAX), Err(HousingError::Overflow));
    assert_eq!(advance(&g, Assessment::Unsupported, i64::MAX), Err(HousingError::Overflow));
}

// ===========================================================================
// The story window
// ===========================================================================

#[test]
fn the_window_re_notifies_three_times_and_then_goes_quiet() {
    let (w, opened) = to_window(&live(&[TENANT]));
    assert_eq!(window_due(&w, opened), WindowAction::Silent, "not on the opening period");

    let mut cur = w;
    for n in 1..=i64::from(MAX_WINDOW_NOTICES) {
        let at = opened + n * WINDOW_RENOTIFY;
        assert_eq!(window_due(&cur, at), WindowAction::Notify);
        cur = window_elapsed(&cur, at, None).expect("a re-notice");
        let TenancyStatus::EvictionPending { window } = &cur.status else {
            panic!("the window must not change state");
        };
        assert_eq!(window.notified, n as u8);
        assert_eq!(window.last_notice, at);
        assert!(window.escalation.is_none());
    }
    // Spent. One period later is silence, not a fourth notice.
    let quiet = opened + i64::from(MAX_WINDOW_NOTICES) + 1;
    assert_eq!(window_due(&cur, quiet), WindowAction::Silent);
    assert_eq!(window_elapsed(&cur, quiet, None).expect("silence"), cur);
}

#[test]
fn the_escalation_opens_after_the_quiet_gap_and_refuses_to_be_skipped() {
    let (w, opened) = to_window(&live(&[TENANT]));
    let mut cur = w;
    for n in 1..=i64::from(MAX_WINDOW_NOTICES) {
        cur = window_elapsed(&cur, opened + n, None).expect("a re-notice");
    }
    let last_notice = opened + i64::from(MAX_WINDOW_NOTICES);
    let due = last_notice + WINDOW_ESCALATE;
    assert_eq!(window_due(&cur, due - 1), WindowAction::Silent);
    assert_eq!(window_due(&cur, due), WindowAction::Escalate);
    assert_eq!(
        window_elapsed(&cur, due, None),
        Err(HousingError::NeedsEscalation),
        "a page must not be silently dropped"
    );

    let e = window_elapsed(&cur, due, Some(EscalationId(42))).expect("the escalation opens");
    let TenancyStatus::EvictionPending { window } = &e.status else {
        panic!("still eviction-pending");
    };
    assert_eq!(window.escalation, Some(EscalationId(42)));
    assert_eq!(window.notified, MAX_WINDOW_NOTICES);
    assert!(e.history.contains(&TenancyEvent::Escalated { id: EscalationId(42), at: due }));
}

#[test]
fn after_the_escalation_notices_slow_to_the_long_interval_and_the_count_stays_capped() {
    let (w, opened) = to_window(&live(&[TENANT]));
    let mut cur = w;
    for n in 1..=i64::from(MAX_WINDOW_NOTICES) {
        cur = window_elapsed(&cur, opened + n, None).expect("a re-notice");
    }
    let esc_at = opened + i64::from(MAX_WINDOW_NOTICES) + WINDOW_ESCALATE;
    cur = window_elapsed(&cur, esc_at, Some(EscalationId(1))).expect("escalate");

    // Six quiet periods, then one notice; and the notice count never grows past the
    // cap however long this runs.
    for gap in 1..WINDOW_ESCALATED_RENOTIFY {
        assert_eq!(window_due(&cur, esc_at + gap), WindowAction::Silent, "gap {gap}");
    }
    let mut at = esc_at;
    for _ in 0..5 {
        at += WINDOW_ESCALATED_RENOTIFY;
        assert_eq!(window_due(&cur, at), WindowAction::NotifyEscalated);
        cur = window_elapsed(&cur, at, None).expect("a slow notice");
        let TenancyStatus::EvictionPending { window } = &cur.status else {
            panic!("still eviction-pending");
        };
        assert_eq!(window.notified, MAX_WINDOW_NOTICES);
        assert_eq!(window.escalation, Some(EscalationId(1)));
    }
    assert_eq!(cur.status.kind(), TenancyStatusKind::EvictionPending);
}

#[test]
fn a_window_elapsing_changes_nothing_but_the_notice_bookkeeping() {
    let (w, opened) = to_window(&live(&[TENANT]));
    let after = window_elapsed(&w, opened + 1, None).expect("a re-notice");
    assert_eq!(after.tenants, w.tenants);
    assert_eq!(after.guests, w.guests);
    assert_eq!(after.homed, w.homed);
    assert_eq!(after.funding, w.funding);
    assert_eq!(after.status.kind(), w.status.kind());
    assert_eq!(after.status.since(), w.status.since(), "the window did not reopen");
    // And the tenant still gets in.
    assert_eq!(may_enter(&after, &after.terms, obj(TENANT), false, opened + 1), Access::Tenant);
}

#[test]
fn the_window_records_the_job_and_the_scene_the_module_opened() {
    let (w, _) = to_window(&live(&[TENANT]));
    let with_job = attach_window(&w, Some(77), None).expect("a job id");
    let with_both = attach_window(&with_job, None, Some(SceneRef(9))).expect("a scene later");
    let TenancyStatus::EvictionPending { window } = &with_both.status else {
        panic!("still eviction-pending");
    };
    assert_eq!(window.job, Some(77), "a later call with None keeps the job");
    assert_eq!(window.scene, Some(SceneRef(9)));

    let live = live(&[TENANT]);
    assert_eq!(attach_window(&live, Some(1), None), Err(HousingError::NoWindow(TenancyId(1))));
    assert_eq!(window_elapsed(&live, 5, None), Err(HousingError::NoWindow(TenancyId(1))));
    assert_eq!(window_due(&live, 5), WindowAction::Silent);
}

// ===========================================================================
// Assessment
// ===========================================================================

#[test]
fn assess_maps_the_bands_to_supported_strained_and_unsupported() {
    let cases = [
        (2u8, 5u8, Assessment::Supported),
        (2, 2, Assessment::Supported),
        (2, 1, Assessment::Strained),
        (2, 0, Assessment::Unsupported),
        (5, 3, Assessment::Unsupported),
        (5, 4, Assessment::Strained),
        (0, 0, Assessment::Supported),
    ];
    for (tier, level, want) in cases {
        let plans = BTreeMap::from([(obj(TENANT), plan(TENANT, level, 0))]);
        assert_eq!(assess(PropertyTier::new(tier), &plans), want, "tier {tier} level {level}");
    }
    assert!(Assessment::Supported.covers() && Assessment::Strained.covers());
    assert!(!Assessment::Unsupported.covers() && !Assessment::Unknown.covers());
}

#[test]
fn assess_is_best_of_across_residents_so_one_payer_carries_the_lease() {
    let plans = BTreeMap::from([
        (obj(TENANT), plan(TENANT, 0, 0)),
        (obj(CO_TENANT), plan(CO_TENANT, 4, 0)),
    ]);
    assert_eq!(assess(PropertyTier::new(3), &plans), Assessment::Supported);
    assert_eq!(best_level(&plans), Some(SupportLevel::new(4)));

    // The solvent co-tenant's fortunes collapse; now nobody carries it.
    let plans = BTreeMap::from([
        (obj(TENANT), plan(TENANT, 0, 1)),
        (obj(CO_TENANT), plan(CO_TENANT, 0, 1)),
    ]);
    assert_eq!(assess(PropertyTier::new(3), &plans), Assessment::Unsupported);

    // A resident whose plan is missing entirely does not drag the other down.
    let plans = BTreeMap::from([(obj(CO_TENANT), plan(CO_TENANT, 3, 2))]);
    assert_eq!(assess(PropertyTier::new(3), &plans), Assessment::Supported);
}

#[test]
fn no_plan_is_unknown_and_only_hardens_into_a_miss_after_the_grace() {
    let empty: BTreeMap<ObjId, SupportPlan> = BTreeMap::new();
    assert_eq!(assess(PropertyTier::new(2), &empty), Assessment::Unknown);
    assert_eq!(best_level(&empty), None);

    for periods in 0..=UNKNOWN_GRACE {
        assert_eq!(harden(Assessment::Unknown, periods), Assessment::Unknown, "{periods}");
    }
    assert_eq!(harden(Assessment::Unknown, UNKNOWN_GRACE + 1), Assessment::Unsupported);
    // Nothing else is touched.
    for a in [Assessment::Supported, Assessment::Strained, Assessment::Unsupported] {
        assert_eq!(harden(a, 99), a);
    }

    // And `Unknown` reaching the ladder moves nothing at all.
    let t = live(&[TENANT]);
    assert_eq!(advance(&t, Assessment::Unknown, 5).expect("unchanged"), t);
    let g = period_missed(&t, 0).expect("miss");
    assert_eq!(advance(&g, Assessment::Unknown, 50).expect("unchanged"), g);
}

#[test]
fn a_plan_must_name_a_current_tenant_and_never_overwrites_another_resident() {
    let t = live(&[TENANT, CO_TENANT]);
    assert_eq!(
        submit_plan(&t, plan(STRANGER, 5, 0)),
        Err(HousingError::NotATenant(obj(STRANGER)))
    );

    let a = submit_plan(&t, plan(TENANT, 1, 0)).expect("one resident's plan");
    let b = submit_plan(&a, plan(CO_TENANT, 4, 0)).expect("the other's");
    let plans = b.funding.plans().expect("abstract").clone();
    assert_eq!(plans.len(), 2);
    assert_eq!(plans.get(&obj(TENANT)).map(|p| p.level), Some(SupportLevel::new(1)));
    assert_eq!(plans.get(&obj(CO_TENANT)).map(|p| p.level), Some(SupportLevel::new(4)));

    // Re-submitting replaces only that resident's own plan.
    let c = submit_plan(&b, plan(TENANT, 5, 1)).expect("a fresh assertion");
    let plans = c.funding.plans().expect("abstract");
    assert_eq!(plans.get(&obj(TENANT)).map(|p| p.level), Some(SupportLevel::new(5)));
    assert_eq!(plans.get(&obj(CO_TENANT)).map(|p| p.level), Some(SupportLevel::new(4)));
    assert!(c.history.iter().any(|e| matches!(
        e,
        TenancyEvent::PlanSubmitted { resident, .. } if *resident == obj(TENANT)
    )));

    // A tenant who leaves takes their plan with them.
    let gone = c.remove_tenant(obj(TENANT), 2).expect("a co-tenant may leave");
    assert!(!gone.funding.plans().expect("abstract").contains_key(&obj(TENANT)));
}

#[test]
fn an_exact_tenancy_takes_no_abstract_plans() {
    let mut t = live(&[TENANT]);
    t.funding = Funding::Exact { contract: ContractId(3), obligation: ObligationId(4) };
    assert_eq!(submit_plan(&t, plan(TENANT, 5, 0)), Err(HousingError::WrongFunding));
    assert!(t.funding.plans().is_none());
}

// ===========================================================================
// Access — fail closed
// ===========================================================================

fn statuses_live(t: &Tenancy) -> Vec<(TenancyStatusKind, Tenancy)> {
    let g = period_missed(t, 0).expect("miss");
    let d = grace_elapsed(&g, 1).expect("grace elapsed");
    let (w, _) = to_window(t);
    vec![
        (TenancyStatusKind::Active, t.clone()),
        (TenancyStatusKind::Grace, g),
        (TenancyStatusKind::Delinquent, d),
        (TenancyStatusKind::EvictionPending, w),
    ]
}

#[test]
fn tenants_and_guests_enter_at_every_live_status_including_delinquency() {
    let t = live(&[TENANT]);
    for (kind, cur) in statuses_live(&t) {
        assert_eq!(cur.status.kind(), kind);
        assert_eq!(may_enter(&cur, &cur.terms, obj(TENANT), false, 9), Access::Tenant, "{kind:?}");
        assert_eq!(may_enter(&cur, &cur.terms, obj(GUEST), false, 9), Access::Guest, "{kind:?}");
        assert_eq!(
            may_enter(&cur, &cur.terms, obj(STRANGER), false, 9),
            Access::Refused,
            "{kind:?}"
        );
    }
}

#[test]
fn inspection_needs_an_explicit_term_and_while_delinquent_means_while_delinquent() {
    let base = live(&[TENANT]);
    // No term at all: the inspector is a stranger.
    for (_, cur) in statuses_live(&base) {
        assert_eq!(may_enter(&cur, &cur.terms, obj(INSPECTOR), false, 9), Access::Refused);
    }

    let mut always = base.clone();
    always.terms = AccessTerms {
        inspection: Some(InspectionTerm {
            who: PrincipalId::Object(obj(INSPECTOR)),
            when: InspectionWhen::Always,
        }),
    };
    for (kind, cur) in statuses_live(&always) {
        assert_eq!(
            may_enter(&cur, &always.terms, obj(INSPECTOR), false, 9),
            Access::Inspection,
            "{kind:?}"
        );
    }

    let mut behind = base.clone();
    behind.terms = AccessTerms {
        inspection: Some(InspectionTerm {
            who: PrincipalId::Object(obj(INSPECTOR)),
            when: InspectionWhen::WhileDelinquent,
        }),
    };
    for (kind, cur) in statuses_live(&behind) {
        let want = if cur.status.is_behind() { Access::Inspection } else { Access::Refused };
        assert_eq!(may_enter(&cur, &behind.terms, obj(INSPECTOR), false, 9), want, "{kind:?}");
    }

    // A `since` stamp in the future (a rewound cadence) does not open the door.
    let mut future = behind.clone();
    future.status = TenancyStatus::Delinquent { since: 100 };
    assert_eq!(may_enter(&future, &future.terms, obj(INSPECTOR), false, 9), Access::Refused);
    assert_eq!(may_enter(&future, &future.terms, obj(INSPECTOR), false, 100), Access::Inspection);
}

#[test]
fn an_org_inspection_term_fails_closed_until_the_module_resolves_it() {
    let mut t = live(&[TENANT]);
    t.terms = AccessTerms {
        inspection: Some(InspectionTerm {
            who: PrincipalId::Organization(OrgId(5)),
            when: InspectionWhen::Always,
        }),
    };
    // The pure library cannot resolve org membership from an ObjId, so it refuses.
    assert_eq!(may_enter(&t, &t.terms, obj(INSPECTOR), false, 0), Access::Refused);
    // E5b answers the membership question and calls the `_with` form.
    assert_eq!(
        may_enter_with(&t, &t.terms, obj(INSPECTOR), true, false, 0),
        Access::Inspection
    );
    assert_eq!(
        may_enter_with(&t, &t.terms, obj(INSPECTOR), false, false, 0),
        Access::Refused
    );
}

#[test]
fn staff_bypass_applies_and_a_pending_or_ended_tenancy_admits_nobody_else() {
    let t = live(&[TENANT]);
    assert_eq!(may_enter(&t, &t.terms, obj(STAFFER), true, 0), Access::Staff);
    // A staffer who is also the tenant reads as the tenant (the more specific fact).
    assert_eq!(may_enter(&t, &t.terms, obj(TENANT), true, 0), Access::Tenant);

    let pending = tenancy(&[TENANT]);
    let ended = resolve(&t, Resolution::TenantLeft, 1).expect("a tenant may leave");
    for cur in [pending, ended] {
        for who in [TENANT, GUEST, INSPECTOR, STRANGER] {
            assert_eq!(
                may_enter(&cur, &cur.terms, obj(who), false, 5),
                Access::Refused,
                "{:?} admits #{who}",
                cur.status.kind()
            );
        }
        assert_eq!(may_enter(&cur, &cur.terms, obj(STAFFER), true, 5), Access::Staff);
    }
    assert!(!Access::Refused.allowed());
    assert!(Access::Tenant.allowed() && Access::Guest.allowed() && Access::Inspection.allowed());
    assert_eq!(Access::default(), Access::Refused, "the default answer is no");
}

#[test]
fn carry_mirrors_enter_exactly() {
    let mut t = live(&[TENANT]);
    t.terms = AccessTerms {
        inspection: Some(InspectionTerm {
            who: PrincipalId::Object(obj(INSPECTOR)),
            when: InspectionWhen::Always,
        }),
    };
    let ended = resolve(&t, Resolution::TenantLeft, 1).expect("a tenant may leave");
    for cur in [t.clone(), ended] {
        for (who, staff) in
            [(TENANT, false), (GUEST, false), (INSPECTOR, false), (STRANGER, false), (STAFFER, true)]
        {
            assert_eq!(
                may_take(&cur, &cur.terms, obj(who), staff, 3),
                may_enter(&cur, &cur.terms, obj(who), staff, 3),
                "#{who} carry and enter disagree"
            );
        }
    }
}

#[test]
fn a_removed_guest_is_refused_again() {
    let t = live(&[TENANT]);
    assert_eq!(may_enter(&t, &t.terms, obj(GUEST), false, 0), Access::Guest);
    let t = t.remove_guest(obj(GUEST));
    assert_eq!(may_enter(&t, &t.terms, obj(GUEST), false, 0), Access::Refused);
    // Removing somebody who was never a guest changes nothing.
    assert_eq!(t.remove_guest(obj(STRANGER)), t);
}

// ===========================================================================
// The `Obj.home` invariant
// ===========================================================================

#[test]
fn homed_on_activate_is_the_tenants_and_never_the_guests() {
    let t = live(&[TENANT, CO_TENANT]);
    assert_eq!(homed_on_activate(&t), BTreeSet::from([obj(TENANT), obj(CO_TENANT)]));
    assert!(!homed_on_activate(&t).contains(&obj(GUEST)));
    assert_eq!(t.homed, BTreeSet::from([obj(TENANT), obj(CO_TENANT)]));
}

#[test]
fn ending_clears_every_home_that_points_inside_and_leaves_one_that_moved_away() {
    let t = live(&[TENANT, CO_TENANT]);
    let rooms = rooms();
    // TENANT is still homed here; CO_TENANT has taken a home elsewhere; the GUEST
    // homed themselves on the spare room; a stranger is homed elsewhere entirely.
    let resolve_home = |o: ObjId| match o.0 {
        TENANT => Some(obj(ANCHOR)),
        CO_TENANT => Some(obj(900)),
        GUEST => Some(obj(ROOM_B)),
        _ => None,
    };
    let out = to_unhome_on_end(&t, resolve_home, &rooms);
    assert!(out.contains(&obj(TENANT)), "the homed tenant is cleared");
    assert!(out.contains(&obj(GUEST)), "a guest homed inside is cleared too");
    assert!(
        !out.contains(&obj(CO_TENANT)),
        "a home that has moved to another property belongs to that tenancy now"
    );

    // An object housing homed whose home has since gone missing is still cleared
    // (harmlessly), so the set never fails open.
    let none = to_unhome_on_end(&t, |_| None, &rooms);
    assert_eq!(none, BTreeSet::from([obj(TENANT), obj(CO_TENANT)]));
}

#[test]
fn an_item_homed_inside_the_property_is_found_by_the_content_sweep() {
    let rooms = rooms();
    let resolve_home = |o: ObjId| match o.0 {
        700 => Some(obj(ROOM_B)),
        701 => Some(obj(ANCHOR)),
        702 => Some(obj(901)),
        _ => None,
    };
    let found = homed_inside([obj(700), obj(701), obj(702), obj(703)], resolve_home, &rooms);
    assert_eq!(found, BTreeSet::from([obj(700), obj(701)]));
    assert!(homed_inside([], resolve_home, &rooms).is_empty());
}

// ===========================================================================
// Caps (§9)
// ===========================================================================

#[test]
fn every_property_cap_refuses_rather_than_panics() {
    // Rooms.
    let many: BTreeSet<ObjId> = (0..MAX_ROOMS_PER_PROPERTY as u32 + 1).map(obj).collect();
    assert_eq!(
        Property::new(PropertyId(1), "x", obj(0), many, PropertyTier::new(1), housing_principal()),
        Err(HousingError::TooLarge("rooms per property"))
    );
    let mut p = property(2);
    for n in 0..MAX_ROOMS_PER_PROPERTY as u32 {
        p = match p.add_room(obj(200 + n)) {
            Ok(next) => next,
            Err(e) => {
                assert_eq!(e, HousingError::TooLarge("rooms per property"));
                assert_eq!(p.rooms.len(), MAX_ROOMS_PER_PROPERTY);
                break;
            }
        };
    }
    assert_eq!(p.rooms.len(), MAX_ROOMS_PER_PROPERTY);
    // Re-adding a room already held costs nothing.
    assert_eq!(p.add_room(obj(ANCHOR)).expect("a no-op"), p);
    // The anchor may never be removed; another room may.
    assert_eq!(
        p.remove_room(obj(ANCHOR)),
        Err(HousingError::Invalid("the anchor room cannot be removed"))
    );
    assert!(p.remove_room(obj(ROOM_B)).expect("a spare room").rooms.len() < p.rooms.len());

    // Name.
    let long = "n".repeat(MAX_PROPERTY_NAME_BYTES + 1);
    assert_eq!(
        Property::new(
            PropertyId(1),
            &long,
            obj(ANCHOR),
            rooms(),
            PropertyTier::new(1),
            housing_principal()
        ),
        Err(HousingError::TooLarge("property name"))
    );
    assert_eq!(
        Property::new(
            PropertyId(1),
            "   ",
            obj(ANCHOR),
            rooms(),
            PropertyTier::new(1),
            housing_principal()
        ),
        Err(HousingError::Invalid("a property needs a name"))
    );

    // Features.
    let mut p = property(2);
    for n in 0..MAX_FEATURES {
        p = p.add_feature(format!("f{n}"), obj(STAFFER)).expect("a feature fits");
    }
    assert_eq!(p.features.len(), MAX_FEATURES);
    assert_eq!(
        p.add_feature("one-too-many", obj(STAFFER)),
        Err(HousingError::TooLarge("features per property"))
    );
    assert_eq!(p.add_feature("f0", obj(STAFFER)).expect("a duplicate is a no-op"), p);
    assert_eq!(
        p.add_feature("k".repeat(MAX_FEATURE_KEY_BYTES + 1), obj(STAFFER)),
        Err(HousingError::TooLarge("feature key"))
    );
    assert_eq!(
        p.add_feature("  ", obj(STAFFER)),
        Err(HousingError::Invalid("a feature needs a key"))
    );
}

#[test]
fn every_tenancy_cap_refuses_rather_than_panics() {
    // Tenants.
    let many: BTreeSet<ObjId> = (0..MAX_TENANTS as u32 + 1).map(obj).collect();
    assert_eq!(
        Tenancy::new(TenancyId(1), PropertyId(1), many, CadenceId(1)),
        Err(HousingError::TooLarge("tenants per tenancy"))
    );
    assert_eq!(
        Tenancy::new(TenancyId(1), PropertyId(1), BTreeSet::new(), CadenceId(1)),
        Err(HousingError::Invalid("a tenancy needs at least one tenant"))
    );

    // Guests.
    let mut t = live(&[TENANT]);
    for n in 0..MAX_GUESTS as u32 {
        t = match t.add_guest(obj(300 + n)) {
            Ok(next) => next,
            Err(e) => {
                assert_eq!(e, HousingError::TooLarge("guests per tenancy"));
                break;
            }
        };
    }
    assert_eq!(t.guests.len(), MAX_GUESTS);
    assert_eq!(t.add_guest(obj(999)), Err(HousingError::TooLarge("guests per tenancy")));
    assert_eq!(t.add_guest(obj(GUEST)).expect("a duplicate is a no-op"), t);
    assert_eq!(
        t.add_guest(obj(TENANT)),
        Err(HousingError::Invalid("a tenant is not their own guest"))
    );

    // Plan sources, and the resolution reason.
    let sources = vec![SupportSource::Inheritance; MAX_PLAN_SOURCES + 1];
    assert_eq!(
        SupportPlan::new(obj(TENANT), SupportLevel::new(1), sources, 0),
        Err(HousingError::TooLarge("plan sources"))
    );
    assert_eq!(
        Resolution::staff_ended(obj(STAFFER), "r".repeat(MAX_REASON_BYTES + 1)),
        Err(HousingError::TooLarge("resolution reason"))
    );
    // And a hand-built oversized reason is refused by the transition too, not just by
    // the constructor — a corrupt document must fail the arrow, never the server.
    let oversized = Resolution::StaffEnded {
        by: obj(STAFFER),
        reason: "r".repeat(MAX_REASON_BYTES + 1).into(),
    };
    assert_eq!(
        resolve(&live(&[TENANT]), oversized, 1),
        Err(HousingError::TooLarge("resolution reason"))
    );

    // The opaque scalars clamp rather than overflow their band.
    assert_eq!(SupportLevel::new(200).get(), 5);
    assert_eq!(PropertyTier::new(200).get(), 5);
}

#[test]
fn the_history_tail_is_bounded_at_thirty_two() {
    let mut t = live(&[TENANT]);
    for n in 0..200i64 {
        t = submit_plan(&t, plan(TENANT, 3, n)).expect("a plan each period");
        assert!(t.history.len() <= MAX_TENANCY_HISTORY, "history grew past the cap at {n}");
    }
    assert_eq!(t.history.len(), MAX_TENANCY_HISTORY);
    // The tail is a FIFO: the newest event survives, the oldest is gone.
    assert!(matches!(
        t.history.last(),
        Some(TenancyEvent::PlanSubmitted { at, .. }) if *at == 199
    ));
    assert!(!t.history.contains(&TenancyEvent::Accepted { at: 0 }));
}

// ===========================================================================
// Serde
// ===========================================================================

fn round_trip<T>(v: &T, what: &str)
where
    T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let j = serde_json::to_string(v).expect("serializes");
    let back: T = serde_json::from_str(&j).unwrap_or_else(|e| panic!("{what} failed to load: {e}"));
    assert_eq!(&back, v, "{what} did not round-trip");
}

#[test]
fn every_record_round_trips_through_serde() {
    let mut p = property(3);
    p.landlord_scope = Some(OrgScopeId(4));
    p.upkeep = Some(Amount::from_minor(1234));
    p.features = vec![Feature { key: "workshop".into(), approved_by: obj(STAFFER) }];
    p.status = PropertyStatus::Let;
    round_trip(&p, "Property");
    round_trip(&PropertyStatus::Withdrawn, "PropertyStatus");
    round_trip(&PropertyTier::new(4), "PropertyTier");
    round_trip(&SupportLevel::new(2), "SupportLevel");
    round_trip(&PropertyId(9), "PropertyId");
    round_trip(&TenancyId(9), "TenancyId");
    round_trip(&SceneRef(9), "SceneRef");

    for s in [
        SupportSource::Career { kind: "clerk".into() },
        SupportSource::Patron { who: PrincipalId::Object(obj(STRANGER)) },
        SupportSource::Inheritance,
        SupportSource::Criminal,
        SupportSource::Domain { org: OrgId(2) },
        SupportSource::Dependent,
    ] {
        round_trip(&s, "SupportSource");
    }
    let sp = SupportPlan::new(
        obj(TENANT),
        SupportLevel::new(3),
        vec![SupportSource::Criminal, SupportSource::Domain { org: OrgId(1) }],
        11,
    )
    .expect("a plan");
    round_trip(&sp, "SupportPlan");

    round_trip(&AccessTerms::default(), "AccessTerms (empty)");
    round_trip(
        &AccessTerms {
            inspection: Some(InspectionTerm {
                who: PrincipalId::Object(obj(INSPECTOR)),
                when: InspectionWhen::WhileDelinquent,
            }),
        },
        "AccessTerms (term)",
    );
    round_trip(&InspectionWhen::Always, "InspectionWhen");
    round_trip(
        &Funding::Exact { contract: ContractId(1), obligation: ObligationId(2) },
        "Funding::Exact",
    );
    round_trip(&Funding::default(), "Funding::Abstract");

    let mut w = StoryWindow::opened_at(5);
    w.job = Some(3);
    w.scene = Some(SceneRef(8));
    w.notified = 2;
    w.escalation = Some(EscalationId(6));
    round_trip(&w, "StoryWindow");
    round_trip(&TenancyStatus::EvictionPending { window: w }, "TenancyStatus::EvictionPending");
    round_trip(&TenancyStatus::Grace { since: 1 }, "TenancyStatus::Grace");
    round_trip(&TenancyStatus::Delinquent { since: 1 }, "TenancyStatus::Delinquent");
    round_trip(&TenancyStatusKind::Delinquent, "TenancyStatusKind");
    for how in [
        Resolution::TenantLeft,
        Resolution::StaffEnded { by: obj(STAFFER), reason: "after the scene".into() },
        Resolution::Waived { by: obj(STAFFER) },
        Resolution::Settled,
    ] {
        round_trip(&TenancyStatus::Ended { how: how.clone() }, "TenancyStatus::Ended");
        round_trip(&how, "Resolution");
    }
    for e in [
        TenancyEvent::Accepted { at: 1 },
        TenancyEvent::StatusChanged {
            from: TenancyStatusKind::Active,
            to: TenancyStatusKind::Grace,
            at: 2,
        },
        TenancyEvent::PlanSubmitted { resident: obj(TENANT), level: SupportLevel::new(2), at: 3 },
        TenancyEvent::Notified { count: 2, at: 4 },
        TenancyEvent::Escalated { id: EscalationId(1), at: 5 },
        TenancyEvent::TenantRemoved { who: obj(CO_TENANT), at: 6 },
        TenancyEvent::Ended { how: Resolution::Settled, at: 7 },
    ] {
        round_trip(&e, "TenancyEvent");
    }

    // A whole tenancy, driven through the ladder so the round-trip carries a window,
    // a plan map, guests, homed objects and a populated history.
    let (w, _) = to_window(&live(&[TENANT, CO_TENANT]));
    let w = submit_plan(&w, plan(TENANT, 2, 3)).expect("a plan");
    let w = attach_window(&w, Some(4), Some(SceneRef(5))).expect("a job");
    round_trip(&w, "Tenancy (eviction-pending)");
    round_trip(&resolve(&w, Resolution::Settled, 9).expect("settled"), "Tenancy (ended)");
    round_trip(&tenancy(&[TENANT]), "Tenancy (pending)");
}

#[test]
fn an_omitted_optional_field_loads_as_its_default() {
    // The `#[serde(default)]` fields a later cut extends: a document written before
    // they existed must still load, and must load closed (no guests, no terms, no
    // homed set, no plans).
    let json = r#"{"id":1,"property":1,"tenants":[10],"cadence":7,"revision":1}"#;
    let t: Tenancy = serde_json::from_str(json).expect("an older tenancy document loads");
    assert_eq!(t.status, TenancyStatus::Pending);
    assert!(t.guests.is_empty() && t.homed.is_empty() && t.history.is_empty());
    assert_eq!(t.terms, AccessTerms::default());
    assert_eq!(t.funding.plans().map(BTreeMap::len), Some(0));
    assert_eq!(may_enter(&t, &t.terms, obj(TENANT), false, 0), Access::Refused);

    let json = r#"{"id":1,"name":"x","anchor":100,"rooms":[100],"tier":2,
                   "landlord":{"System":"wod.housing"},"revision":1}"#;
    let p: Property = serde_json::from_str(json).expect("an older property document loads");
    assert_eq!(p.status, PropertyStatus::Vacant);
    assert!(p.features.is_empty() && p.upkeep.is_none() && p.landlord_scope.is_none());
}

// ===========================================================================
// Fuzz
// ===========================================================================

#[test]
fn fuzz_random_transition_sequences_never_panic_or_end_without_a_resolution() {
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut reached = [0usize; 6];
    let mut ended_with_resolution = 0usize;

    for run in 0..300u64 {
        let mut t = tenancy(&[TENANT, CO_TENANT]);
        // A cadence that jitters forwards and backwards, so a rewound clock is part
        // of the run rather than a special case.
        let mut now: i64 = 0;
        for _ in 0..60 {
            now = now.saturating_add((next() % 5) as i64 - 1);
            let before = t.clone();
            let out = match next() % 11 {
                0 => accept(&t, next() % 4 != 0, &BTreeSet::from([obj(TENANT), obj(CO_TENANT)]), now),
                1 => period_missed(&t, now),
                2 => period_supported(&t, now),
                3 => grace_elapsed(&t, now),
                4 => delinquent_elapsed(&t, now),
                5 => window_elapsed(&t, now, Some(EscalationId(run))),
                6 => attach_window(&t, Some(run), Some(SceneRef(run))),
                7 => advance(
                    &t,
                    match next() % 4 {
                        0 => Assessment::Supported,
                        1 => Assessment::Strained,
                        2 => Assessment::Unsupported,
                        _ => Assessment::Unknown,
                    },
                    now,
                ),
                8 => submit_plan(&t, plan(TENANT, (next() % 6) as u8, now)),
                9 => t.add_guest(obj(400 + (next() % 40) as u32)),
                _ => {
                    // Only this arm can end anything, and it cannot be spelled
                    // without naming a resolution.
                    if next() % 8 == 0 {
                        resolve(&t, Resolution::Waived { by: obj(STAFFER) }, now)
                    } else {
                        Ok(t.remove_guest(obj(400 + (next() % 40) as u32)))
                    }
                }
            };
            t = out.unwrap_or(before);

            // The invariants, checked after every single step.
            assert!(t.history.len() <= MAX_TENANCY_HISTORY);
            assert!(t.guests.len() <= MAX_GUESTS);
            assert!(t.tenants.len() <= MAX_TENANTS);
            assert!(!t.tenants.is_empty());
            if let TenancyStatus::EvictionPending { window } = &t.status {
                assert!(window.notified <= MAX_WINDOW_NOTICES);
            }
            if let TenancyStatus::Ended { how } = &t.status {
                // Reachable only through `resolve`, and only with one of the four
                // deliberate acts.
                assert!(matches!(
                    how,
                    Resolution::TenantLeft
                        | Resolution::StaffEnded { .. }
                        | Resolution::Waived { .. }
                        | Resolution::Settled
                ));
                ended_with_resolution += 1;
            }
            reached[match t.status.kind() {
                TenancyStatusKind::Pending => 0,
                TenancyStatusKind::Active => 1,
                TenancyStatusKind::Grace => 2,
                TenancyStatusKind::Delinquent => 3,
                TenancyStatusKind::EvictionPending => 4,
                TenancyStatusKind::Ended => 5,
            }] += 1;
        }
    }
    assert!(ended_with_resolution > 0, "the fuzz never ended a tenancy");
    for (n, count) in reached.iter().enumerate() {
        assert!(*count > 0, "the fuzz never reached status {n}");
    }
}
