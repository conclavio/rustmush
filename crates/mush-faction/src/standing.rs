//! The faction-standing predicate provider (E1c) — the concrete
//! [`OrgPredicateProvider`] that lets an org [`mush_org::AccessPolicy`] gate on **live**
//! faction standing, without `mush-org` depending on the faction registry.
//!
//! An org policy rule whose subject is
//! `PolicySubject::RegisteredPredicate { provider: "faction", predicate, arguments }`
//! is resolved here against the live [`FactionState`]. This is the seam that keeps
//! the faction authorization model (R-D1) entirely faction-side — the level ladder,
//! `officer_level`, `intel_level`, and the covert-membership disclosure rules never
//! leave `mush-faction`; the engine only ever learns a boolean verdict.
//!
//! ## Predicates
//! - `member`  — the actor is a member of the named faction (any visibility; truth).
//! - `officer` — the actor's level ≥ the faction's `officer_level`.
//! - `intel`   — the actor's level ≥ the faction's `intel_level` (may read the pool).
//! - `standing_at_least` — public-grant score meets `minimum` (-100..100).
//!   This does not change membership or either clearance threshold above.
//!
//! The argument names the faction: a bare id number, or `{"faction": id}` (the
//! [`Principal`](crate::Principal) wire form). Anything unresolvable — a bad
//! argument, an unknown faction, an unknown predicate, or a missing registry —
//! returns `None`, which the engine treats **fail-closed** (suppresses an Allow,
//! satisfies a Deny). The provider returns only the boolean, never the underlying
//! level or a hidden membership.

use mush_core::{ObjId, World};
use mush_org::OrgPredicateProvider;

use crate::data::{FactionId, FactionState};

/// Resolves org policy predicates under the `"faction"` provider token against the
/// live faction registry. Frozen into the `OrgModule` at wiring (E1c).
#[derive(Debug, Default, Clone, Copy)]
pub struct FactionStandingProvider;

impl OrgPredicateProvider for FactionStandingProvider {
    fn provider(&self) -> &str {
        "faction"
    }

    fn eval(
        &self,
        w: &World,
        predicate: &str,
        arguments: &serde_json::Value,
        actor: ObjId,
    ) -> Option<bool> {
        let fid = parse_faction(arguments)?;
        let st = w.module_state::<FactionState>()?;
        // An unknown faction is unresolvable (fail-closed), never "definitely not a
        // member" — a Deny gated on it must still fire.
        if !st.factions.contains_key(&fid) {
            return None;
        }
        match predicate {
            "member" => Some(st.is_member(actor, fid)),
            "officer" => Some(st.is_officer(actor, fid)),
            "intel" => Some(st.can_read_intel(actor, fid)),
            "standing_at_least" => {
                let minimum = arguments.get("minimum")?.as_i64()?;
                if !(-100..=100).contains(&minimum) {
                    return None;
                }
                Some(i64::from(crate::reputation::policy_score(w, fid, actor)?) >= minimum)
            }
            // Unknown predicate: fail closed rather than guess.
            _ => None,
        }
    }
}

/// Parse the faction id from a predicate argument: a bare number (`3`) or the
/// [`Principal`](crate::Principal) faction wire form (`{"faction": 3}`).
fn parse_faction(arguments: &serde_json::Value) -> Option<FactionId> {
    let n = arguments
        .as_u64()
        .or_else(|| arguments.get("faction").and_then(serde_json::Value::as_u64))?;
    Some(FactionId(n))
}
