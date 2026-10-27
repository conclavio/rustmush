//! 17g C2c (C2 plan §2.2): `+minds/holdings`, the actor's own licences.
//!
//! - Bare: `HOLD_HEAD`, up to 20 owner rows (`HOLD_MORE owner <cursor>` when
//!   cut), up to 20 paid rows (`HOLD_MORE paid <cursor>`), then the purchases
//!   waiting; `HOLD_EMPTY` when there is nothing at all.
//! - `owner <cursor>` and `paid <cursor>` page one section: `HOLD_HEAD`, the
//!   section's rows after the cursor (possibly none) and `HOLD_MORE` while
//!   the section goes on. A paged section past its end, or a stretch of own
//!   titles with no filed mind in it, prints no `HOLD_EMPTY`: that sentence
//!   is the bare form's, for an actor with nothing in either section (the C2c
//!   fix round, reviewers F3 and C2C-4).
//!
//! Owner rows come from Roll's own-only `OwnHoldings { section: Owner }`
//! read: the actor's own E4 charter titles, never another holder's (C2 plan
//! §2.4). Roll answers at most `OWNER_ROWS_PER_READ` rows a read, inside the
//! folded read's declared row ceiling, so this command makes up to
//! `MAX_READS` reads to fill a section's 20 rows.
//!
//! **Not built (the C2c builder's report):** the paid section reads the
//! per-holder paid index and the waiting purchases the per-buyer entitlement
//! index (C2 plan §2.5 items 1 and 2), which C2b did not land. Roll answers
//! the paid section `Unavailable`, so bare `+minds/holdings` and
//! `+minds/holdings paid <cursor>` refuse `NOT_OPEN` rather than print a
//! list that leaves the paid licences out (`HOLD_EMPTY` would then be
//! false; it waits in `words::pending`). `+minds/holdings owner <cursor>` is
//! built, but like every `+minds` form it needs the market read
//! (`market::handles`), which no composition grants yet (`market`'s module
//! doc), so today it too refuses `NOT_OPEN` (reviewer T-F4).

use mush_core::{CmdError, Ctx};
use mush_roll::commerce::command_reader::{
    HoldingSection, OwnerHoldingView, OwnerPhase, RollCommandReadRequest, RollCommandReadResult,
};
use mush_roll::commerce::holding_reader::{OriginalOwnerReader, OriginalReadRequest};

use super::market::{HOLDING_NAME, handles};
use super::words::{self, cut, fill};

/// Rows one section shows at most.
const SECTION_ROWS: usize = 20;
/// Reads one section makes at most (Roll answers up to three rows a read).
const MAX_READS: usize = 12;

fn not_open() -> CmdError {
    CmdError::msg(words::NOT_OPEN)
}

/// Which part `+minds/holdings`' argument asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Holdings {
    All,
    Owner(u64),
    Paid(u64),
}

/// `""`, `owner <cursor>` or `paid <cursor>`, strictly.
pub(crate) fn holdings_arguments(text: &str) -> Option<Holdings> {
    let mut words = text.split_whitespace();
    let parsed = match (words.next(), words.next()) {
        (None, _) => Holdings::All,
        (Some("owner"), Some(cursor)) => Holdings::Owner(super::author::number(cursor)?),
        (Some("paid"), Some(cursor)) => Holdings::Paid(super::author::number(cursor)?),
        _ => return None,
    };
    words.next().is_none().then_some(parsed)
}

/// `+minds/holdings [owner|paid <cursor>]`.
pub(crate) fn holdings(ctx: &mut Ctx<'_>, text: &str) -> Result<(), CmdError> {
    let which = holdings_arguments(text).ok_or_else(|| CmdError::msg(super::market::HELP))?;
    let (_, roll) = handles(ctx)?;
    let (rows, more) = match which {
        // The paid section and the waiting purchases need the indexes C2b
        // did not land (module doc): Roll answers the paid section
        // `Unavailable`, so these refuse rather than list a partial set.
        Holdings::All | Holdings::Paid(_) => return Err(not_open()),
        Holdings::Owner(cursor) => owner_section(ctx, &roll, Some(cursor))?,
    };
    let actor = ctx.actor;
    for line in owner_lines(&rows, more) {
        ctx.out.info(actor, line);
    }
    Ok(())
}

/// One paged owner section's lines: `HOLD_HEAD`, a `HOLD_OWNER` row each and
/// `HOLD_MORE owner <cursor>` while the section goes on. Never `HOLD_EMPTY`:
/// a page can be empty because the cursor is past the last charter or the
/// reads met only other titles, and neither means the actor holds nothing
/// (the cursor is printed in the second case).
pub(crate) fn owner_lines(rows: &[OwnerHoldingView], more: Option<u64>) -> Vec<String> {
    let mut lines = vec![words::HOLD_HEAD.to_owned()];
    lines.extend(rows.iter().map(owner_line));
    if let Some(cursor) = more {
        let cursor = cursor.to_string();
        lines.push(fill(
            words::HOLD_MORE,
            &[("section", "owner"), ("cursor", &cursor)],
        ));
    }
    lines
}

/// Up to `SECTION_ROWS` owner rows after `after`, and the cursor to show in
/// `HOLD_MORE` when the section goes on: the last shown charter's asset id
/// (a charter is its own E4 asset), or Roll's own cursor when the reads ran
/// out first (then the rows may be empty and the cursor still `Some`).
fn owner_section(
    ctx: &Ctx<'_>,
    roll: &mush_core::read_service::OwnerReadHandle<OriginalOwnerReader>,
    after: Option<u64>,
) -> Result<(Vec<OwnerHoldingView>, Option<u64>), CmdError> {
    let mut rows: Vec<OwnerHoldingView> = Vec::new();
    let mut cursor = after;
    for _ in 0..MAX_READS {
        let request = OriginalReadRequest::command(RollCommandReadRequest::own_holdings(
            HoldingSection::Owner,
            cursor,
        ));
        let page = ctx
            .read_owner(roll, request)
            .ok()
            .and_then(mush_roll::commerce::holding_reader::OriginalReadResult::into_command)
            .and_then(RollCommandReadResult::into_owner_holdings)
            .ok_or_else(not_open)?;
        for row in page.rows() {
            if rows.len() == SECTION_ROWS {
                let last = rows.last().map(|r| u64::from(r.charter().get()));
                return Ok((rows, last));
            }
            rows.push(row.clone());
        }
        match page.next() {
            None => return Ok((rows, None)),
            Some(next) => cursor = Some(next),
        }
    }
    Ok((rows, cursor))
}

/// `Active, {n} days left`, `Grace, {n} days left` or `Lapsed`.
pub(crate) fn phase_words(phase: OwnerPhase) -> String {
    match phase {
        OwnerPhase::Active { days_left } => format!("Active, {days_left} days left"),
        OwnerPhase::Grace { days_left } => format!("Grace, {days_left} days left"),
        OwnerPhase::Lapsed => "Lapsed".to_owned(),
    }
}

/// One `HOLD_OWNER` row.
pub(crate) fn owner_line(row: &OwnerHoldingView) -> String {
    owner_row(
        row.filed().get(),
        row.dependent(),
        row.name(),
        &phase_words(row.owner_phase()),
        row.selling(),
    )
}

/// `HOLD_OWNER` from its parts (trailing spaces trimmed).
pub(crate) fn owner_row(
    filed: u64,
    dependent: bool,
    name: &str,
    phase: &str,
    selling: bool,
) -> String {
    let filed = filed.to_string();
    let name = cut(name, HOLDING_NAME);
    let kind = if dependent { "dependent" } else { "owner" };
    let flags = if selling { "; selling" } else { "" };
    fill(
        words::HOLD_OWNER,
        &[
            ("filed", &filed),
            ("kind", kind),
            ("name", &name),
            ("phase", phase),
            ("flags", flags),
        ],
    )
    .trim_end()
    .to_owned()
}
