//! **Penalties beyond the fine, and the page that shows them** (doc 50 unit LW.4; Q-50-39 to
//! Q-50-42, Q-50-69, Q-50-70 as ruled, doc 50a B-1, C-2, C-7, A-10e).
//!
//! - **The standing loss** (`standing_loss`, Q-50-69, provisional): with the polity that proved
//!   the offense. −5 for a sale proven by a witness's report, −10 for running the picket, and
//!   nothing for an offense an authority's own inspection found and the offender paid at once (an
//!   audit's finding not paid at once costs −5). Standing never decays and paying a fine restores
//!   nothing (Q-50-42): it is earned back by lawful work.
//! - **The trading restriction** ([`restricted`], Q-50-39): while the actor's standing with a
//!   polity is unfriendly or lower (−10 or less) or a fine of its record stands unpaid, that
//!   polity's boards refuse the actor's trade in restricted goods, and its Registry issues no new
//!   license to the actor's gateway account. Nothing holds a character (Q-42-7).
//! - **The notice** (Q-50-40 as corrected): an official notice at once when a matter opens
//!   ([`crate::offense::record_offense`] tells the offender).
//! - **`+record`** ([`record_page`], Q-50-40, Q-50-70): one page for the actor: each proven
//!   offense, which polity proved it and by what kind of evidence (never who reported), its fine
//!   and grace in IC and in real time, the restrictions in force and what lifts each, the actor's
//!   own receipts of private sales, and a core under a live Registry impound on a hull the actor
//!   holds. `+debts` lists the Registry's mind fines too, and `+debts/pay` pays them.

use mush_core::{CmdError, Ctx, ObjId, World};
use mush_economy::{ObligationId, ObligationStatus};
use mush_faction::{FactionId, StandingEffect};

use crate::offense::{Evidence, Offender, OffenseKind, OffenseRecord, Proof, ProvenOffense};

/// The verb.
pub const RECORD_VERB: &str = "+record";
/// The help.
pub const RECORD_HELP: &str = "+record   your record: proven offenses, which polity proved each and \
     how, their fines and grace, the restrictions in force and what lifts each, your receipts of \
     private sales, your standing where an offense is proven, and any core the Registry holds \
     impounded on a hull of yours.";
/// **"Below neutral"** (Q-50-69): the unfriendly band or lower.
pub const RESTRICTED_AT_OR_BELOW: i16 = -10;
/// The standing a witness-proven sale costs (Q-50-69, provisional).
pub const SALE_STANDING: i16 = -5;
/// The standing running the picket costs (Q-50-69, provisional).
pub const RUN_STANDING: i16 = -10;
/// The standing an audit's finding not paid at once costs (provisional; Q-50-69 names only the
/// paid case).
pub const UNPAID_AUDIT_STANDING: i16 = -5;

/// **`+record`'s standing heading** (doc 50 cut OP.8): the standing lost with each polity that
/// proved an offense, which paying a fine restores none of (Q-50-42).
pub const STANDING_LOST: &str =
    "Your standing where an offense is proven (paying a fine restores none of it):";
/// The restriction's refusal on a board: `{polity}`.
pub const BOARD_RESTRICTED: &str = "{polity} does not let you trade restricted goods on its boards \
     while you are under its restriction (+record shows it, and what lifts it).";
/// The Registry's refusal of a license: `{polity}`.
pub const LICENSE_RESTRICTED: &str = "The Registry issues no new license to your account while \
     you are under {polity}'s restriction (+record shows it, and what lifts it).";

/// A polity's name.
#[must_use]
pub(crate) fn polity_name(w: &World, polity: FactionId) -> String {
    w.module_state::<mush_faction::FactionState>()
        .and_then(|s| s.factions.get(&polity).map(|f| f.name.to_string()))
        .unwrap_or_else(|| format!("faction #{}", polity.0))
}

/// `who`'s standing score with `polity` (0 when none is recorded).
fn score(w: &World, polity: FactionId, who: ObjId) -> i16 {
    mush_faction::public_standing(w, polity, who).map_or(0, |v| v.score)
}

/// Whether a row's fine stands unpaid. Doc 50b: an owed transit duty counts only once it is past
/// its grace (Q-50-54: within it the duty is owed and listed, and binds nothing).
fn unpaid(w: &World, row: &ProvenOffense) -> bool {
    if row.kind == OffenseKind::UnpaidDuty {
        return row.debt.is_some_and(|id| {
            w.module_state::<mush_economy::EconomyStore>()
                .and_then(|e| e.snapshot().obligation(ObligationId(id)).cloned())
                .is_some_and(|o| crate::surface::retrieval::past_due(&o))
        });
    }
    row.debt.is_some_and(|id| {
        w.module_state::<mush_economy::EconomyStore>()
            .and_then(|e| {
                e.snapshot()
                    .obligation(ObligationId(id))
                    .map(|o| o.status.clone())
            })
            .is_some_and(|s| {
                matches!(
                    s,
                    ObligationStatus::Active
                        | ObligationStatus::Grace { .. }
                        | ObligationStatus::Delinquent
                )
            })
    })
}

/// **Why `who` is under `polity`'s restriction**, if they are (Q-50-39): standing at or below
/// [`RESTRICTED_AT_OR_BELOW`], or an unpaid fine on their record there.
#[must_use]
pub fn restriction(w: &World, who: ObjId, polity: FactionId) -> Option<Restriction> {
    let standing = score(w, polity, who);
    let unpaid_fines: Vec<u64> = crate::offense::record_of(
        w,
        &crate::offense::OffenseKey {
            offender: Offender::Character(who),
            polity,
        },
    )
    .map(|r| {
        r.rows
            .iter()
            .filter(|row| unpaid(w, row))
            .filter_map(|row| row.debt)
            .collect()
    })
    .unwrap_or_default();
    (standing <= RESTRICTED_AT_OR_BELOW || !unpaid_fines.is_empty()).then_some(Restriction {
        polity,
        standing,
        unpaid_fines,
    })
}

/// **A restriction in force**: whose, and what keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Restriction {
    /// The polity.
    pub polity: FactionId,
    /// The actor's standing with it.
    pub standing: i16,
    /// The unpaid fines, `ObligationId.0`.
    pub unpaid_fines: Vec<u64>,
}

/// **Whether `who` is restricted by `polity`** ([`restriction`]).
#[must_use]
pub fn restricted(w: &World, who: ObjId, polity: FactionId) -> bool {
    restriction(w, who, polity).is_some()
}

/// **Whether `who`'s gateway account is barred from a new license by `polity`** (Q-50-68: the
/// bar binds the account): any character of the same gateway account under its restriction.
#[must_use]
pub fn account_barred(w: &World, who: ObjId, polity: FactionId) -> bool {
    if restricted(w, who, polity) {
        return true;
    }
    let Some(account) = crate::port::licence::gateway_account(w, who) else {
        return false;
    };
    w.iter()
        .filter(|o| o.kind == mush_core::ObjKind::Player && o.id != who)
        .filter(|o| crate::port::licence::gateway_account(w, o.id) == Some(account))
        .any(|o| restricted(w, o.id, polity))
}

/// The board's refusal for a restricted actor.
#[must_use]
pub fn board_refusal(w: &World, polity: FactionId) -> String {
    BOARD_RESTRICTED.replace("{polity}", &polity_name(w, polity))
}

/// The license desk's refusal for a barred account.
#[must_use]
pub fn license_refusal(w: &World, polity: FactionId) -> String {
    LICENSE_RESTRICTED.replace("{polity}", &polity_name(w, polity))
}

/// **Record a standing loss** for an offense (Q-50-69): `delta` with `polity`, under a cause
/// stable for the offense, so a retry records it once.
pub(crate) fn standing_loss(
    ctx: &mut Ctx<'_>,
    who: ObjId,
    polity: FactionId,
    delta: i16,
    cause: &str,
    reason: &str,
) {
    if delta == 0 {
        return;
    }
    if let Err(error) = mush_faction::record_standing(
        ctx,
        cause,
        who,
        &[StandingEffect {
            faction: polity,
            delta,
        }],
        reason,
    ) {
        tracing::warn!(who = who.0, %error, %cause, "tacsim: a standing loss was not recorded");
    }
}

/// **The standing losses of `key`'s rows that `pick` names** (Q-50-69): each row's
/// [`standing_for`], its fine paid at once or not, under [`offense_cause`]. Called right after
/// a prover records and fines; an organization or account offender has no standing.
pub(crate) fn lose_standing(
    ctx: &mut Ctx<'_>,
    key: &crate::offense::OffenseKey,
    pick: impl Fn(&ProvenOffense) -> bool,
) {
    let Offender::Character(who) = key.offender else {
        return;
    };
    let Some(record) = crate::offense::record_of(ctx.w, key) else {
        return;
    };
    for row in record.rows.iter().filter(|row| pick(row)) {
        let paid = row.debt.is_some_and(|id| {
            ctx.w
                .module_state::<mush_economy::EconomyStore>()
                .and_then(|e| {
                    e.snapshot()
                        .obligation(ObligationId(id))
                        .map(|o| o.status.clone())
                })
                .is_some_and(|s| matches!(s, ObligationStatus::Satisfied))
        });
        let delta = standing_for(row.kind, paid);
        let cause = offense_cause(key.polity, who, row);
        standing_loss(ctx, who, key.polity, delta, &cause, row.kind.what());
    }
}

/// The cause of an offense's standing loss: its key and evidence.
#[must_use]
pub fn offense_cause(polity: FactionId, who: ObjId, row: &ProvenOffense) -> String {
    let evidence = match &row.evidence {
        Evidence::Site { site, act } => format!("site/{site}/{}/{}", act.subject, act.at_ic),
        Evidence::Audit {
            hull,
            entry,
            good,
            at_ic,
            ..
        } => {
            format!("audit/{}/{entry}/{good}/{at_ic}", hull.0)
        }
        Evidence::Receipt { quote, .. } => format!("receipt/{quote}"),
        Evidence::Duty {
            hull,
            cursor,
            at_ic,
        } => format!("duty/{}/{cursor}/{at_ic}", hull.0),
    };
    format!("tacsim.law/{}/{}/{evidence}", polity.0, who.0)
}

// ---- the page --------------------------------------------------------------------------------

/// What kind of evidence proved a row, never who reported it.
fn proven_by(row: &ProvenOffense) -> &'static str {
    match row.proof {
        Proof::Vacated => "a vacated claim, by the registry's finding",
        Proof::Reported { .. } => match row.evidence {
            Evidence::Receipt { .. } => "a party's report of the sale's receipt",
            _ => "a witness's report to the registry",
        },
        Proof::Audit => match row.evidence {
            Evidence::Duty { .. } => "the customs picket's assessment",
            _ => "a port audit",
        },
    }
}

/// The fine's line: paid, or due with its grace in IC and real time, or past due.
fn fine_line(ctx: &Ctx<'_>, row: &ProvenOffense) -> String {
    let Some(id) = row.debt else {
        return "no fine yet; it is levied at your next docking at its ports".to_owned();
    };
    let Some(o) = ctx
        .w
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|e| e.snapshot().obligation(ObligationId(id)).cloned())
    else {
        return format!("fine #{id}");
    };
    let amount = crate::port::coop::money_minor(ctx, o.amount.minor());
    match o.status {
        ObligationStatus::Satisfied => format!("fine #{id} of {amount}, paid"),
        ObligationStatus::Delinquent => format!(
            "fine #{id} of {amount}, past its grace: it holds your hull at its ports until paid \
             (+debts/pay {id})"
        ),
        _ => {
            let now = ctx.w.world_ic_now().at;
            let due = match o.status {
                ObligationStatus::Grace { deadline, .. } => deadline.at,
                _ => o.schedule.next_due,
            };
            let rate = i64::from(ctx.w.world_timeline().current().rate.max(1));
            let real_h = due.saturating_sub(now).max(0) / rate / 3_600;
            format!(
                "fine #{id} of {amount}, due; its grace runs to {} IC (about {real_h} real hours) (+debts/pay {id})",
                crate::port::licence::ic_stamp(due)
            )
        }
    }
}

/// **`+record`'s page for `who`** (Q-50-40, Q-50-70).
#[must_use]
pub fn record_page(ctx: &Ctx<'_>, who: ObjId) -> String {
    let w: &World = ctx.w;
    let records: Vec<OffenseRecord> = crate::offense::records_against(w, Offender::Character(who));
    let mut out = vec!["Your record:".to_owned()];
    let rows: usize = records.iter().map(|r| r.rows.len()).sum();
    if rows == 0 {
        out.push("  No offense is proven against you.".to_owned());
    } else {
        out.push("Proven offenses:".to_owned());
        for r in &records {
            let polity = polity_name(w, r.key.polity);
            for row in &r.rows {
                out.push(format!(
                    "  {polity}: {}, proven by {}, {} IC; {}.",
                    row.kind.what(),
                    proven_by(row),
                    crate::port::licence::ic_stamp(row.proven_ic),
                    fine_line(ctx, row)
                ));
            }
        }
    }
    let mut proving: Vec<FactionId> = records.iter().map(|r| r.key.polity).collect();
    proving.sort();
    proving.dedup();
    let mut polities = proving.clone();
    if let Some(st) = w.module_state::<mush_faction::FactionState>() {
        polities.extend(st.factions.keys().copied());
    }
    polities.sort();
    polities.dedup();
    let restrictions: Vec<Restriction> = polities
        .into_iter()
        .filter_map(|p| restriction(w, who, p))
        .collect();
    if restrictions.is_empty() {
        out.push("No restriction is in force.".to_owned());
    } else {
        out.push("Restrictions in force:".to_owned());
        for r in &restrictions {
            let mut lifts = Vec::new();
            if !r.unpaid_fines.is_empty() {
                let ids: Vec<String> = r.unpaid_fines.iter().map(|id| format!("#{id}")).collect();
                lifts.push(format!("paying fine {}", ids.join(", ")));
            }
            if r.standing <= RESTRICTED_AT_OR_BELOW {
                lifts.push(format!(
                    "your standing rising above {RESTRICTED_AT_OR_BELOW} (it is {}) through lawful work for it",
                    r.standing
                ));
            }
            out.push(format!(
                "  {}: its boards refuse your trade in restricted goods, and its Registry issues \
                 your account no new license. It lifts on {}.",
                polity_name(w, r.polity),
                lifts.join(" and ")
            ));
        }
    }
    // Doc 50 cut OP.8 (Q-50-47's third story, Q-50-42): the standing an offense cost stays, a
    // fine paid or not, so the page shows it with each polity that proved one.
    let lost: Vec<String> = proving
        .into_iter()
        .filter_map(|p| {
            let standing = score(w, p, who);
            (standing < 0).then(|| format!("  {}: {standing}.", polity_name(w, p)))
        })
        .collect();
    if !lost.is_empty() {
        out.push(STANDING_LOST.to_owned());
        out.extend(lost);
    }
    if let Some(book) =
        crate::port::law_report::receipts_of(w, who).filter(|b| !b.receipts.is_empty())
    {
        out.push("Your receipts of private sales of restricted goods:".to_owned());
        for r in &book.receipts {
            let side = if r.buyer == who { "bought" } else { "sold" };
            out.push(format!(
                "  sale {}: {side} {} {} at {}, {} IC{}",
                r.quote,
                r.units,
                r.good,
                r.venue,
                crate::port::licence::ic_stamp(r.at_ic),
                if r.reported { " (reported)" } else { "" }
            ));
        }
    }
    let impounded: Vec<ObjId> = w
        .module_state::<mush_economy::EconomyStore>()
        .map(|e| {
            e.snapshot()
                .assets_of(&mush_org::PrincipalId::Object(who))
                .map(|a| a.object)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
        .into_iter()
        .filter(|hull| {
            mush_tactics::module::unit_of(w, *hull)
                .is_some_and(|unit| crate::match_::seal::core_impounded(w, unit))
        })
        .collect();
    for hull in impounded {
        out.push(format!(
            "The Registry holds the core fitted on the {} impounded.",
            crate::port::coop::name_of(w, hull)
        ));
    }
    out.join("\n")
}

/// **`+record`'s spec**.
#[must_use]
pub(crate) fn record_spec() -> mush_core::CommandSpec {
    mush_core::CommandSpec {
        name: RECORD_VERB.into(),
        shape: mush_core::ArgShape::Raw,
        help: RECORD_HELP.into(),
        ..mush_core::CommandSpec::default()
    }
}

/// **`+record`** (doc 50 unit LW.4): a pure read of the actor's own record.
///
/// # Errors
///
/// None in practice; the verb is a read.
pub(crate) fn record_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    crate::commands::require_enabled(ctx)?;
    let page = record_page(ctx, ctx.actor);
    ctx.out.sys_to_actor(ctx.actor, "tacsim.law", page);
    Ok(())
}

/// The standing a newly proven row costs (Q-50-69), given whether its fine was paid at once.
#[must_use]
pub fn standing_for(kind: OffenseKind, paid_at_once: bool) -> i16 {
    match kind {
        OffenseKind::RestrictedSale | OffenseKind::RestrictedPurchase => SALE_STANDING,
        OffenseKind::UnlicensedPossession if !paid_at_once => UNPAID_AUDIT_STANDING,
        OffenseKind::UnlicensedPossession
        | OffenseKind::UnpaidDuty
        | OffenseKind::SalvagePickup
        | OffenseKind::SalvageScrap
        | OffenseKind::SalvageRecovery => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Doc 50 unit LW.4: the sizes** (Q-50-69): two proven sales, or one run, restrict.
    #[test]
    fn lw4_the_sizes_reach_the_band() {
        assert_eq!(standing_for(OffenseKind::RestrictedSale, true), -5);
        assert_eq!(standing_for(OffenseKind::UnlicensedPossession, true), 0);
        assert_eq!(standing_for(OffenseKind::UnlicensedPossession, false), -5);
        assert_eq!(standing_for(OffenseKind::SalvageRecovery, false), 0);
        const {
            assert!(2 * SALE_STANDING <= RESTRICTED_AT_OR_BELOW);
            assert!(SALE_STANDING > RESTRICTED_AT_OR_BELOW, "one sale does not");
            assert!(RUN_STANDING <= RESTRICTED_AT_OR_BELOW);
        }
    }
}
