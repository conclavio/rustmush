//! **No timer that waits on an absent player's response runs under 28 IC days** (owner rule,
//! 2026-11-04).
//!
//! Players are not online all the time, so a timer that takes something from a player who
//! has not answered allows at least 28 IC days (about 28 real hours at IC rate 24). A timer
//! that resolves in the absent player's favour, or an offer whose lapse takes nothing, may stay
//! short. This guard pins the seven the audit at `bf0b5f24` found and the owner raised, and
//! each code cap that would otherwise hold a value under the floor, so none drifts back.
//! Doc 41's salvage claim window, debt grace and RS.7 hold, raised by the RS window, are pinned
//! here with RS.10's consent wait (8).

use serde_json::Value;

/// The owner's floor, in IC seconds.
const FLOOR: i64 = 28 * 86_400;

fn at_least(name: &str, seconds: i64) {
    assert!(
        seconds >= FLOOR,
        "{name} is {seconds} IC s, under the 28-IC-day player-response floor ({FLOOR})"
    );
}

/// 1. A mind licence's Grace after its term, and every bound on it.
#[test]
fn mind_licence_grace_is_at_least_28_ic_days() {
    at_least(
        "tacsim::launch::GRACE_SECONDS",
        tacsim::launch::GRACE_SECONDS,
    );
    at_least(
        "mush_roll::commerce::MAX_GRACE_SECONDS",
        mush_roll::commerce::MAX_GRACE_SECONDS,
    );
    at_least(
        "mush_core::player_wait::MAX_LICENCE_GRACE_SECONDS",
        mush_core::player_wait::MAX_LICENCE_GRACE_SECONDS,
    );
}

/// 2. A House or personal renewal offer's lifetime, the deployment default and both caps.
#[test]
fn renewal_offers_are_at_least_28_ic_days() {
    at_least(
        "tacsim::launch::OFFER_SECONDS",
        tacsim::launch::OFFER_SECONDS,
    );
    at_least(
        "MarketTerms::PROVISIONAL.renewal_offer_ic_seconds",
        tacsim::launch::MarketTerms::PROVISIONAL.renewal_offer_ic_seconds,
    );
    for (name, seconds) in [
        (
            "managed_renewal::DEFAULT_RENEWAL_OFFER_IC_SECONDS",
            mush_economy::managed_renewal::DEFAULT_RENEWAL_OFFER_IC_SECONDS,
        ),
        (
            "managed_renewal::MAX_RENEWAL_OFFER_IC_SECONDS",
            mush_economy::managed_renewal::MAX_RENEWAL_OFFER_IC_SECONDS,
        ),
        (
            "managed_house_renewal::DEFAULT_RENEWAL_OFFER_IC_SECONDS",
            mush_economy::managed_house_renewal::DEFAULT_RENEWAL_OFFER_IC_SECONDS,
        ),
        (
            "managed_house_renewal::MAX_RENEWAL_OFFER_IC_SECONDS",
            mush_economy::managed_house_renewal::MAX_RENEWAL_OFFER_IC_SECONDS,
        ),
    ] {
        at_least(name, seconds);
    }
    let example: Value = serde_json::from_str(include_str!(
        "../../../../deploy/instance/launch.json.example"
    ))
    .expect("launch.json.example parses");
    let found = find(&example, "renewal_offer_ic_seconds")
        .and_then(Value::as_i64)
        .expect("launch.json.example names renewal_offer_ic_seconds");
    at_least("launch.json.example renewal_offer_ic_seconds", found);
}

/// 3. The prospecting licence's grace, as the shipped lane authors it.
#[test]
fn prospecting_licence_grace_is_at_least_28_ic_days() {
    let lane: Value =
        serde_json::from_str(tacsim::definitions::content::LANE_JSON).expect("the lane parses");
    let days = find(&lane, "licence")
        .and_then(|licence| licence.get("grace_ic_days"))
        .and_then(Value::as_i64)
        .expect("the lane authors a licence grace");
    at_least("lane licence.grace_ic_days", days * 86_400);
}

/// 4. A granted recovery lease's acceptance window.
#[test]
fn recovery_acceptance_is_at_least_28_ic_days() {
    at_least(
        "mush_campaign::RECOVERY_ACCEPT_TTL_SECS",
        mush_campaign::RECOVERY_ACCEPT_TTL_SECS,
    );
}

/// 5. An operation's notice to its defenders (a stopgap until async attack and defence).
#[test]
fn operation_notice_is_at_least_28_ic_days() {
    at_least(
        "mush_campaign::OPERATION_NOTICE_MIN",
        mush_campaign::OPERATION_NOTICE_MIN,
    );
    at_least(
        "mush_campaign::OPERATION_NOTICE_MAX",
        mush_campaign::OPERATION_NOTICE_MAX,
    );
}

/// 6. A refit's reservation, in the economy and in the kernel's audit.
#[test]
fn refit_reservations_are_at_least_28_ic_days() {
    for (name, seconds) in [
        (
            "bay_refit::RESERVATION_SECONDS",
            mush_economy::bay_refit::RESERVATION_SECONDS,
        ),
        (
            "core_refit::RESERVATION_SECONDS",
            mush_economy::core_refit::RESERVATION_SECONDS,
        ),
        (
            "mush_core::player_wait::REFIT_RESERVATION_SECONDS",
            mush_core::player_wait::REFIT_RESERVATION_SECONDS,
        ),
    ] {
        at_least(name, seconds);
    }
}

/// 7. A survey contract's term.
#[test]
fn survey_terms_are_at_least_28_ic_days() {
    at_least(
        "tacsim::port::survey::SURVEY_TERM_IC_S",
        tacsim::port::survey::SURVEY_TERM_IC_S,
    );
}

/// 8. Doc 41's waits (the RS window, cut RS.10): a salvage claim's contest window and the
///    shortest a deployment may set, a debt's grace (RS.5's charge and RS.7's fine, which holds a
///    carrier only past it), and a landholder's answer to a consent ask (silence or refusal).
#[test]
fn salvage_waits_are_at_least_28_ic_days() {
    for (name, days) in [
        (
            "tacsim::config::CLAIM_WINDOW_IC_DAYS",
            tacsim::config::CLAIM_WINDOW_IC_DAYS,
        ),
        (
            "tacsim::config::MIN_CLAIM_WINDOW_IC_DAYS",
            tacsim::config::MIN_CLAIM_WINDOW_IC_DAYS,
        ),
        (
            "tacsim::surface::retrieval::DEBT_GRACE_IC_DAYS",
            tacsim::surface::retrieval::DEBT_GRACE_IC_DAYS,
        ),
        (
            "tacsim::surface::claim::CONSENT_IC_DAYS",
            tacsim::surface::claim::CONSENT_IC_DAYS,
        ),
    ] {
        at_least(name, days * 86_400);
    }
    at_least(
        "tacsim::surface::claim::CONSENT_IC_SECS",
        tacsim::surface::claim::CONSENT_IC_SECS,
    );
}

/// 10. **Doc 38a review F-14**: a captive's release fee (doc 42 Q-42-7) falls due no sooner than
///     the debt floor, so an absent ex-captive is never refused at a desk the next sweep.
#[test]
fn a_release_fee_falls_due_after_28_ic_days() {
    let w = mush_core::World::new_minimal();
    let now = w.world_ic_now().at;
    at_least(
        "tacsim::field::crew::release_fee_due(w) - now",
        tacsim::field::crew::release_fee_due(&w) - now,
    );
}

/// The first value under `key`, depth first.
fn find<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    match value {
        Value::Object(map) => map
            .get(key)
            .or_else(|| map.values().find_map(|child| find(child, key))),
        Value::Array(items) => items.iter().find_map(|child| find(child, key)),
        _ => None,
    }
}
