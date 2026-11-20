//! **A front's lifecycle, and what moves a polity's clock** (doc 50 cut KC.0a, Q-50-51).
//!
//! - [`reopen_front`] opens a resolved front again, and [`set_clock`] sets a clock's count
//!   back, for a compiled owner (Q-50-31's repeat, Q-50-16's later push-back). Staff reach the
//!   same two through `@front/reopen` and `@clock/set <id>=filled:<n>`.
//! - [`advance_clock_by_cause`] moves a clock by a typed owner call that carries a cause id,
//!   the canon draft's `source`, as the customs path does. It never moves a clock that watches
//!   a canon kind, so a polity's clock has one cause path and the canon observer
//!   (`on_canon_recorded`) keeps serving the clocks that use it today.
//!
//! Completion goes through the engine's one path (`advance_clock`): the consequence fires, any
//! `on_complete_control` flips, and the front is marked resolved when its last clock fills.

use mush_core::Ctx;
use smol_str::SmolStr;

use crate::sim::{self, Clock, ClockId, Front, FrontId, MAX_CLOCK_CAUSES, SimState};

/// The longest cause id a typed advance takes.
pub const MAX_CAUSE_LEN: usize = 96;

/// What [`advance_clock_by_cause`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClockAdvance {
    /// The clock moved. `completed` says this call filled it (its consequence fired).
    Advanced {
        /// The filled count after the call.
        filled: u8,
        /// Whether this call completed the clock.
        completed: bool,
    },
    /// The cause id had already moved this clock: nothing was written.
    AlreadyCounted,
}

/// **Reopen a resolved front** (Q-50-51), so its clocks advance again. A front already open
/// is left as it is. Its clocks keep their counts; [`set_clock`] sets one back.
///
/// # Errors
///
/// The faction module is not loaded, or there is no such front.
pub fn reopen_front(ctx: &mut Ctx<'_>, front: FrontId) -> Result<Front, String> {
    let f = {
        let st = ctx
            .w
            .module_state_mut::<SimState>()
            .ok_or("Fronts unavailable.")?;
        let f = st.fronts.get_mut(&front).ok_or("No such front.")?;
        if !f.resolved {
            return Ok(f.clone());
        }
        f.resolved = false;
        f.clone()
    };
    sim::persist_front(ctx.out, &f);
    ctx.out.pin_deferred();
    Ok(f)
}

/// **Set a clock's filled count** (Q-50-51): `filled` below the clock's segments, and the
/// clock is no longer completed, so it can fill and fire again. To fill a clock, advance it:
/// completion has one path. The front's resolved flag is untouched; a clock on a resolved
/// front stays frozen until [`reopen_front`].
///
/// # Errors
///
/// The faction module is not loaded, there is no such clock, or `filled` is not below its
/// segments.
pub fn set_clock(ctx: &mut Ctx<'_>, clock: ClockId, filled: u8) -> Result<Clock, String> {
    let c = {
        let st = ctx
            .w
            .module_state_mut::<SimState>()
            .ok_or("Fronts unavailable.")?;
        let c = st.clocks.get_mut(&clock).ok_or("No such clock.")?;
        if filled >= c.segments {
            return Err(format!(
                "A clock of {} is set below {}; advance it to fill it.",
                c.segments, c.segments
            ));
        }
        c.filled = filled;
        c.completed = false;
        c.clone()
    };
    sim::persist_clock(ctx.out, &c);
    ctx.out.pin_deferred();
    Ok(c)
}

/// **Move a clock by a typed owner call** (Q-50-51): commit `draft`, whose `source` is the
/// cause id, then advance the clock `by` segments through the engine's one completion path. A
/// cause id the clock has already counted writes nothing and answers
/// [`ClockAdvance::AlreadyCounted`], so a retry is harmless.
///
/// # Errors
///
/// Nothing is written when: the modules are not loaded; there is no such clock; the clock is
/// completed, watches a canon kind (it moves by the canon observer, not by this call), or
/// stands on a resolved front; `by` is zero; the cause id is empty or longer than
/// [`MAX_CAUSE_LEN`]; the draft's kind is the reserved consequence kind; or the canon append
/// fails.
pub fn advance_clock_by_cause(
    ctx: &mut Ctx<'_>,
    clock: ClockId,
    by: u8,
    draft: mush_canon::CanonDraft,
) -> Result<ClockAdvance, String> {
    let cause = SmolStr::new(&draft.source);
    if cause.trim().is_empty() || cause.len() > MAX_CAUSE_LEN {
        return Err("A clock's cause id must be 1 to 96 characters.".into());
    }
    if draft.kind == crate::CONSEQUENCE_KIND {
        return Err("A cause cannot be a clock consequence.".into());
    }
    if by == 0 {
        return Err("A clock advances by one segment or more.".into());
    }
    {
        let st = ctx
            .w
            .module_state::<SimState>()
            .ok_or("Fronts unavailable.")?;
        let c = st.clocks.get(&clock).ok_or("No such clock.")?;
        if c.causes.contains(&cause) {
            return Ok(ClockAdvance::AlreadyCounted);
        }
        if c.completed {
            return Err("That clock has completed.".into());
        }
        if c.advance_on.is_some() {
            return Err("That clock moves by its canon kind, not by an owner call.".into());
        }
        if st.fronts.get(&c.front).is_none_or(|f| f.resolved) {
            return Err("That clock's front is resolved or missing.".into());
        }
    }
    if mush_canon::commit(ctx, draft).is_none() {
        return Err("Canon unavailable.".into());
    }
    // No fallible step follows the canon append.
    if let Some(c) = ctx
        .w
        .module_state_mut::<SimState>()
        .and_then(|st| st.clocks.get_mut(&clock))
    {
        if c.causes.len() >= MAX_CLOCK_CAUSES {
            c.causes.remove(0);
        }
        c.causes.push(cause);
    }
    let completed = crate::advance_clock(ctx, clock, by);
    let filled = ctx
        .w
        .module_state::<SimState>()
        .and_then(|st| st.clocks.get(&clock))
        .map_or(0, |c| c.filled);
    ctx.out.pin_deferred();
    Ok(ClockAdvance::Advanced { filled, completed })
}
