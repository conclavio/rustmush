//! **The Isidis fronts: the Concession Review and Hold the Concession** (doc 50 cut OP.2;
//! Q-50-20, Q-50-61; doc 11 §6).
//!
//! - **Two fronts at world start**, seeded by the lane's regions applier beside Customs Assertion
//!   when the lane has the `isidis` row (`seed_isidis_fronts`):
//!   - **The Concession Review** on the Republic's board: six segments, at two of six. Its fill
//!     is the Republic's decision to assume the Concession's administration, which the declaring
//!     arm acts on (OP.5); the flip of control is the operation's, so the clock names none.
//!   - **Hold the Concession** on the Freeport's board: six segments at two, no flip. The
//!     stockpile at its threshold moves it once an IC month (doc 50 cut OP.4,
//!     `crate::stockpile`); a loss on the run lands with OP.5.
//! - **The scheduled tick, a typed cause** ([`ASSEMBLY_TICK`]): the Assembly votes once an IC month
//!   ([`IC_MONTH_S`]), **counted from the instant the Review was seeded** (the Isidis bring-up's
//!   fix): vote `n` falls `n` IC months after it, wherever in the calendar the world first booted.
//!   Each vote moves the Review one segment through `mush_faction::advance_clock_by_cause` with
//!   the cause id `assembly.vote/<n>`. The seed's own vote is the anchor: the Review opens at one,
//!   and the seed's vote, cause id `assembly.vote/0@<IC seconds>` ([`seed_cause`]), takes it to
//!   two and records the seeding instant in the clock's persisted cause list. Every later fire
//!   moves it once for each whole IC month since that instant that it has not counted
//!   ([`votes_due`]). A restart, a missed fire, downtime or a paused world moves it no more and no
//!   less; a cause id already counted writes nothing.
//! - **A seized Freeport-flagged hauler** ([`record_flagged_seizure`]): a seizure ruling at the
//!   Kirkwood picket on a hull whose flag ([`hull_flag`]) is the Freeport's moves the Review one
//!   segment, the picket incident's own `source` its cause id (Q-50-20 as corrected).
//! - **A hull's flag** ([`hull_flag`], Q-50-61): the polity of the organization that holds its
//!   title, from the lane's table. A Co-op Dray is titled to a Freeport organization, so it is
//!   Freeport-flagged; a personally titled or untitled hull flies none.

use std::sync::Arc;
use std::time::Duration;

use mush_core::module::{ModError, Registrar, TickCadence};
use mush_core::{Ctx, ObjId, SceneId, World};
use mush_faction::{ClockAdvance, ClockId, FactionId, SimState, Visibility};

use crate::polity::{RegionsDef, polity, polity_of_org};

/// The Concession Review's title (doc 11 §6).
pub const CONCESSION_REVIEW_TITLE: &str = "The Concession Review";
/// Hold the Concession's title (doc 11 §6).
pub const HOLD_THE_CONCESSION_TITLE: &str = "Hold the Concession";
/// Both clocks' length (doc 11 §6).
pub const CONCESSION_SEGMENTS: u8 = 6;
/// The count both clocks stand at on a fresh world (doc 11 §6, Q-50-20).
pub const CONCESSION_STARTS_AT: u8 = 2;
/// The Review's consequence line.
pub const CONCESSION_REVIEW_CONSEQUENCE: &str = "The Assembly ends its review of the Isidis \
     Concession: the Republic will move to assume the Concession's administration.";
/// Hold the Concession's consequence line.
pub const HOLD_THE_CONCESSION_CONSEQUENCE: &str = "The Freeport holds the Isidis Concession: its \
     defenses stand ready, and the Concord is now involved.";
/// The regions row the fronts belong to.
pub const ISIDIS_ROW: &str = "isidis";
/// One IC month, for the Assembly's vote (Q-50-20: one an IC month).
pub const IC_MONTH_S: i64 = 30 * 86_400;
/// The vote's cause-id prefix; the vote's number since the seeding follows it.
pub const ASSEMBLY_CAUSE: &str = "assembly.vote/";
/// The seed vote's marker after [`ASSEMBLY_CAUSE`]: vote zero, at the IC instant that follows.
const SEED_MARK: &str = "0@";
/// The tick's tag.
pub const ASSEMBLY_TICK: &str = "tacsim.assembly";
/// How often it fires (Coarse). A month is 30 real hours at 24:1, so a minute is plenty.
pub const ASSEMBLY_TICK_PERIOD: Duration = Duration::from_secs(60);

/// The IC month `ic` falls in.
#[must_use]
pub fn ic_month(ic: i64) -> i64 {
    ic.div_euclid(IC_MONTH_S)
}

/// **The seed vote's cause id**: `assembly.vote/0@<IC seconds>`, the instant the Review was
/// seeded, which every later vote counts from.
#[must_use]
pub fn seed_cause(seeded_ic: i64) -> String {
    format!("{ASSEMBLY_CAUSE}{SEED_MARK}{seeded_ic}")
}

/// **The instant the Review was seeded**, read from its seed vote's cause ([`seed_cause`]).
#[must_use]
pub fn seeded_at(clock: &mush_faction::Clock) -> Option<i64> {
    clock.causes.iter().find_map(|cause| {
        cause
            .strip_prefix(ASSEMBLY_CAUSE)?
            .strip_prefix(SEED_MARK)?
            .parse::<i64>()
            .ok()
    })
}

/// **The Assembly's votes due by `now`** for a Review seeded at `seeded_ic`: one for each whole
/// IC month since the seeding, whatever the calendar's phase. Vote `n` is due at
/// `seeded_ic + n * IC_MONTH_S`.
#[must_use]
pub fn votes_due(seeded_ic: i64, now: i64) -> i64 {
    now.saturating_sub(seeded_ic).div_euclid(IC_MONTH_S).max(0)
}

/// **A hull's flag** (Q-50-61): the polity of the organization that holds its title, or none.
#[must_use]
pub fn hull_flag(w: &World, hull: ObjId) -> Option<FactionId> {
    match crate::port::house_sweep::title_of(w, hull)? {
        mush_org::PrincipalId::Organization(org) => polity_of_org(w, org),
        _ => None,
    }
}

/// The clock of the front titled `title` on `board`, if seeded.
fn front_clock(w: &World, board: FactionId, title: &str) -> Option<mush_faction::Clock> {
    let sim = w.module_state::<SimState>()?;
    let front = sim
        .fronts
        .values()
        .find(|f| f.faction == board && f.title == title)?;
    sim.clocks.values().find(|c| c.front == front.id).cloned()
}

/// The Concession Review's clock, if seeded.
#[must_use]
pub fn review_clock(w: &World) -> Option<mush_faction::Clock> {
    front_clock(w, polity(w, "republic")?, CONCESSION_REVIEW_TITLE)
}

/// Hold the Concession's clock, if seeded.
#[must_use]
pub fn hold_clock(w: &World) -> Option<mush_faction::Clock> {
    front_clock(w, polity(w, "freeport")?, HOLD_THE_CONCESSION_TITLE)
}

pub(crate) fn draft(
    ctx: &Ctx<'_>,
    kind: &str,
    summary: String,
    source: String,
) -> mush_canon::CanonDraft {
    mush_canon::CanonDraft {
        kind: kind.into(),
        summary,
        entities: Vec::new(),
        scope: mush_canon::CanonScope::Public,
        ic_time: ctx.w.world_ic_now().at,
        source,
        detail: serde_json::json!({}),
    }
}

/// The Assembly's vote on the Review, under the cause id `cause`.
fn vote(ctx: &mut Ctx<'_>, clock: ClockId, cause: String) -> Result<ClockAdvance, String> {
    let line = draft(
        ctx,
        "op.concession_review",
        format!(
            "The Assembly at Coprates votes on the Concession Review; {CONCESSION_REVIEW_TITLE} advances."
        ),
        cause,
    );
    mush_faction::advance_clock_by_cause(ctx, clock, 1, line)
}

/// **Seed the two Isidis fronts** (this module's head): when `def` has the [`ISIDIS_ROW`] and
/// the Republic and the Freeport stand. Idempotent: a seeded front is left as it is.
///
/// # Errors
///
/// The faction engine's refusal.
pub(crate) fn seed_isidis_fronts(ctx: &mut Ctx<'_>, def: &RegionsDef) -> Result<(), String> {
    let Some(row) = def.rows.iter().find(|row| row.id.as_str() == ISIDIS_ROW) else {
        return Ok(());
    };
    let (Some(republic), Some(freeport)) = (polity(ctx.w, "republic"), polity(ctx.w, "freeport"))
    else {
        return Ok(());
    };
    if review_clock(ctx.w).is_none() {
        let opening = draft(
            ctx,
            "op.concession_review",
            format!(
                "{CONCESSION_REVIEW_TITLE}: the Republic questions the treaty that keeps the {}.",
                row.name
            ),
            "tacsim.concession.review".into(),
        );
        mush_faction::open_faction_front(
            ctx,
            republic,
            &mush_faction::FrontSpec {
                title: CONCESSION_REVIEW_TITLE,
                clock_title: "Assembly votes & seized Freeport haulers",
                segments: CONCESSION_SEGMENTS,
                initial: CONCESSION_STARTS_AT - 1,
                consequence: CONCESSION_REVIEW_CONSEQUENCE,
                control: None,
                visibility: Visibility::Public,
            },
            opening,
        )?;
        // The seed's vote: two of six, and the instant the tick counts from.
        let clock = review_clock(ctx.w).ok_or("the Concession Review's clock is missing")?;
        let seeded = ctx.w.world_ic_now().at;
        vote(ctx, clock.id, seed_cause(seeded))?;
    }
    if hold_clock(ctx.w).is_none() {
        let opening = draft(
            ctx,
            "op.hold_the_concession",
            format!(
                "{HOLD_THE_CONCESSION_TITLE}: the Port Council answers the review and stockpiles at the {}.",
                row.name
            ),
            "tacsim.concession.hold".into(),
        );
        mush_faction::open_faction_front(
            ctx,
            freeport,
            &mush_faction::FrontSpec {
                title: HOLD_THE_CONCESSION_TITLE,
                clock_title: "Reinforcements, stockpiles & haulers through the pickets",
                segments: CONCESSION_SEGMENTS,
                initial: CONCESSION_STARTS_AT,
                consequence: HOLD_THE_CONCESSION_CONSEQUENCE,
                control: None,
                visibility: Visibility::Public,
            },
            opening,
        )?;
    }
    Ok(())
}

/// **The Assembly's votes due now** (this module's head, "The scheduled tick"): one for each
/// whole IC month since the Review was seeded ([`votes_due`]) that the clock has not counted.
/// Answers how many moved it.
pub fn assembly_votes(ctx: &mut Ctx<'_>) -> usize {
    let Some(clock) = review_clock(ctx.w) else {
        return 0;
    };
    if clock.completed {
        return 0;
    }
    let Some(seeded) = seeded_at(&clock) else {
        return 0;
    };
    let due = votes_due(seeded, ctx.w.world_ic_now().at);
    let mut moved = 0;
    for n in 1..=due {
        match vote(ctx, clock.id, format!("{ASSEMBLY_CAUSE}{n}")) {
            Ok(ClockAdvance::Advanced { completed, .. }) => {
                moved += 1;
                if completed {
                    break;
                }
            }
            Ok(_) => {}
            Err(why) => {
                tracing::warn!(target: "tacsim", vote = n, why, "tacsim: the Assembly's vote did not land");
                break;
            }
        }
    }
    if moved > 0 {
        ctx.out.pin_deferred();
    }
    moved
}

/// The tick (this module's head).
fn assembly_tick(ctx: &mut Ctx<'_>, _tag: &str, _scene: Option<SceneId>) {
    if !ctx
        .w
        .module_state::<crate::TacsimStore>()
        .is_some_and(crate::TacsimStore::enabled)
    {
        return;
    }
    let _ = assembly_votes(ctx);
    // Doc 50 cut OP.5: the declaring arm, on the Review's fill, and the outcome once decided.
    crate::isidis_op::tick(ctx);
}

/// **A seizure ruling on a Freeport-flagged hull** (Q-50-20 as corrected): the Review moves one
/// segment, `source` (the picket incident's) its cause id. Any other hull moves nothing.
/// Answers whether it moved.
pub fn record_flagged_seizure(ctx: &mut Ctx<'_>, hull: ObjId, source: &str) -> bool {
    let (Some(flag), Some(freeport)) = (hull_flag(ctx.w, hull), polity(ctx.w, "freeport")) else {
        return false;
    };
    if flag != freeport {
        return false;
    }
    let Some(clock) = review_clock(ctx.w) else {
        return false;
    };
    if clock.completed {
        return false;
    }
    let mut line = draft(
        ctx,
        "op.concession_review",
        format!(
            "Republic Customs seized a Freeport-flagged hauler at the Kirkwood picket; {CONCESSION_REVIEW_TITLE} advances."
        ),
        source.to_owned(),
    );
    line.entities = vec![hull];
    matches!(
        mush_faction::advance_clock_by_cause(ctx, clock.id, 1, line),
        Ok(ClockAdvance::Advanced { .. })
    )
}

/// Register the Assembly's tick.
///
/// # Errors
///
/// Any registration the registrar refuses.
pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.tick(
        ASSEMBLY_TICK,
        TickCadence::Coarse(ASSEMBLY_TICK_PERIOD),
        Arc::new(assembly_tick),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400;

    /// A Review clock that counted `causes`.
    fn review_with(causes: &[String]) -> mush_faction::Clock {
        mush_faction::Clock {
            id: ClockId(1),
            front: mush_faction::FrontId(1),
            faction: FactionId(1),
            title: "Assembly votes & seized Freeport haulers".to_owned(),
            segments: CONCESSION_SEGMENTS,
            filled: CONCESSION_STARTS_AT,
            visibility: Visibility::Public,
            advance_on: None,
            consequence: String::new(),
            on_complete_control: None,
            completed: false,
            causes: causes.iter().map(|c| c.as_str().into()).collect(),
        }
    }

    /// **The Isidis bring-up's fix**: a world seeded 15 or 25 IC days before a calendar month's
    /// boundary has its first vote 30 IC days after the seeding, not at the boundary.
    #[test]
    fn the_first_vote_falls_a_month_after_the_seeding_not_at_the_calendars_boundary() {
        let boundary = 28_636 * IC_MONTH_S;
        for before in [15, 25] {
            let seeded = boundary - before * DAY;
            assert_eq!(
                ic_month(seeded) + 1,
                ic_month(boundary),
                "part-way into a month"
            );
            assert_eq!(
                votes_due(seeded, boundary),
                0,
                "{before} days: none at the boundary"
            );
            assert_eq!(
                votes_due(seeded, seeded + IC_MONTH_S - 1),
                0,
                "{before} days"
            );
            assert_eq!(votes_due(seeded, seeded + IC_MONTH_S), 1, "{before} days");
            assert_eq!(
                votes_due(seeded, seeded + 4 * IC_MONTH_S),
                4,
                "{before} days"
            );
        }
    }

    /// **Downtime**: the months missed are counted from the seeding, and only whole ones.
    #[test]
    fn missed_months_are_caught_up_from_the_seeding() {
        let seeded = 28_636 * IC_MONTH_S - 15 * DAY;
        assert_eq!(votes_due(seeded, seeded + 95 * DAY), 3);
        assert_eq!(votes_due(seeded, seeded + 5 * IC_MONTH_S), 5);
        assert_eq!(
            votes_due(seeded, seeded - DAY),
            0,
            "never before the seeding"
        );
    }

    /// The seeding instant is read back from the seed vote's cause, which persists with the
    /// clock, and neither a later vote nor a seizure is taken for it.
    #[test]
    fn the_seeding_instant_is_read_back_from_the_seed_votes_cause() {
        let seeded = 28_636 * IC_MONTH_S - 25 * DAY;
        let clock = review_with(&[
            seed_cause(seeded),
            format!("{ASSEMBLY_CAUSE}1"),
            "picket.incident/7".to_owned(),
        ]);
        assert_eq!(seed_cause(seeded), format!("assembly.vote/0@{seeded}"));
        assert_eq!(seeded_at(&clock), Some(seeded));
        assert_eq!(
            seeded_at(&review_with(&[format!("{ASSEMBLY_CAUSE}1")])),
            None
        );
    }
}
