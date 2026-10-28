//! 17g C2c (C2 plan §2.2): `+minds/holdings`, the actor's own licences.
//!
//! - Bare: `HOLD_HEAD`, up to 20 owner rows (`HOLD_MORE owner <cursor>` when
//!   cut), up to 20 paid rows (`HOLD_MORE paid <cursor>`), then the purchases
//!   waiting; `HOLD_EMPTY` instead of the head and both sections when there
//!   is nothing in either section (no row and no cursor).
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
//! **The paid section and the waiting purchases (17g C2g, the C2e plan §4.1
//! item 2),** over 17g C2f's owner indexes, written inside the money
//! transactions:
//! - paid rows come from Roll's own-only `OwnHoldings { section: Paid }`
//!   read, one bounded scan of the actor's own key prefix of the per-holder
//!   index (`roll_mind_paid_holdings`) a read, each row exactly Roll's
//!   `OwnPaid` view (`paid_line`); the cursor is the last row's licence;
//! - after the sections, when the market read's own-only `OwnOpen` finds a
//!   funded purchase (the per-buyer entitlement index), `HOLD_WAIT_HEAD` and
//!   one `HOLD_WAIT` per funded purchase, whatever its listing's currency (a
//!   withdrawn listing's escrow is still the actor's, and `+minds/buy/
//!   complete` and `/refund` resolve it through the same index).
//!
//! Every `+minds` form sits behind the market read (`market::handles`,
//! reviewer T-F4), so all three forms are live on the format-2 Active launch
//! and refuse `NOT_OPEN` elsewhere.

use mush_core::read_service::OwnerReadHandle;
use mush_core::{CmdError, Ctx};
use mush_market::mind_commands::{MarketCommandReader, MarketCommandRequest};
use mush_roll::commerce::command_reader::{
    HoldingSection, OwnerHoldingView, OwnerPhase, PaidHoldingView, RenewalMark,
    RollCommandReadRequest, RollCommandReadResult,
};
use mush_roll::commerce::holding_reader::{
    OriginalOwnerReader, OriginalReadRequest, OriginalReadResult,
};
use mush_roll::commerce::paid::PaidForm;

use super::market::{HOLDING_NAME, handles};
use super::words::{self, Refusal, cut, fill};

/// Rows one section shows at most.
const SECTION_ROWS: usize = 20;
/// Reads one section makes at most (Roll answers up to three rows a read).
const MAX_READS: usize = 12;

fn not_open() -> Refusal {
    Refusal::not_open()
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
    let (market, roll) = handles(ctx)?;
    let lines = match which {
        Holdings::Owner(cursor) => {
            let (rows, more) = owner_section(ctx, &roll, Some(cursor))?;
            owner_lines(&rows, more)
        }
        Holdings::Paid(cursor) => {
            let (rows, more) = paid_section(ctx, &roll, Some(cursor))?;
            section_lines(rows.iter().map(paid_line).collect(), "paid", more)
        }
        Holdings::All => {
            let (owner, owner_more) = owner_section(ctx, &roll, None)?;
            let (paid, paid_more) = paid_section(ctx, &roll, None)?;
            let waiting = waiting(ctx, &market)?;
            all_lines(
                (owner.iter().map(owner_line).collect(), owner_more),
                (paid.iter().map(paid_line).collect(), paid_more),
                &waiting,
            )
        }
    };
    let actor = ctx.actor;
    for line in lines {
        ctx.out.info(actor, line);
    }
    Ok(())
}

/// One paged owner section's lines (`section_lines` over `HOLD_OWNER` rows):
/// `HOLD_HEAD`, a row each and `HOLD_MORE owner <cursor>` while the section
/// goes on. Never `HOLD_EMPTY`: a page can be empty because the cursor is
/// past the last charter or the reads met only other titles, and neither
/// means the actor holds nothing (the cursor is printed in the second case).
pub(crate) fn owner_lines(rows: &[OwnerHoldingView], more: Option<u64>) -> Vec<String> {
    section_lines(rows.iter().map(owner_line).collect(), "owner", more)
}

/// 17g C2g: one paged section's lines: `HOLD_HEAD`, its rendered `rows` and
/// `HOLD_MORE <section> <cursor>` while it goes on; never `HOLD_EMPTY`.
pub(crate) fn section_lines(rows: Vec<String>, section: &str, more: Option<u64>) -> Vec<String> {
    let mut lines = vec![words::HOLD_HEAD.to_owned()];
    lines.extend(rows);
    lines.extend(more_line(section, more));
    lines
}

fn more_line(section: &str, more: Option<u64>) -> Option<String> {
    more.map(|cursor| {
        fill(
            words::HOLD_MORE,
            &[("section", section), ("cursor", &cursor.to_string())],
        )
    })
}

/// 17g C2g: the bare form's lines from the two rendered sections (rows and
/// cursor each) and the waiting purchases' listing codes: `HOLD_HEAD`, the
/// owner rows and their `HOLD_MORE owner`, the paid rows and their
/// `HOLD_MORE paid`; or `HOLD_EMPTY` alone in their place when neither
/// section has a row or a cursor (the C2c fix round's rule: never after a
/// section that goes on). Then `HOLD_WAIT_HEAD` and a `HOLD_WAIT` per code,
/// when there are any.
pub(crate) fn all_lines(
    owner: (Vec<String>, Option<u64>),
    paid: (Vec<String>, Option<u64>),
    waiting: &[String],
) -> Vec<String> {
    let (owner_rows, owner_more) = owner;
    let (paid_rows, paid_more) = paid;
    let mut lines = Vec::new();
    if owner_rows.is_empty() && owner_more.is_none() && paid_rows.is_empty() && paid_more.is_none()
    {
        lines.push(words::HOLD_EMPTY.to_owned());
    } else {
        lines.push(words::HOLD_HEAD.to_owned());
        lines.extend(owner_rows);
        lines.extend(more_line("owner", owner_more));
        lines.extend(paid_rows);
        lines.extend(more_line("paid", paid_more));
    }
    if !waiting.is_empty() {
        lines.push(words::HOLD_WAIT_HEAD.to_owned());
        lines.extend(
            waiting
                .iter()
                .map(|code| fill(words::HOLD_WAIT, &[("code", code)])),
        );
    }
    lines
}

/// One Roll command read of the folded original-holdings read, or
/// `NOT_OPEN` when it cannot run or answers another kind.
fn command(
    reads: &impl super::Reads,
    roll: &OwnerReadHandle<OriginalOwnerReader>,
    request: RollCommandReadRequest,
) -> Result<RollCommandReadResult, Refusal> {
    reads
        .owner_read(roll, OriginalReadRequest::command(request))
        .ok()
        .and_then(OriginalReadResult::into_command)
        .ok_or_else(not_open)
}

/// Up to `SECTION_ROWS` owner rows after `after`, and the cursor to show in
/// `HOLD_MORE` when the section goes on: the last shown charter's asset id
/// (a charter is its own E4 asset), or Roll's own cursor when the reads ran
/// out first (then the rows may be empty and the cursor still `Some`).
pub(super) fn owner_section(
    reads: &impl super::Reads,
    roll: &OwnerReadHandle<OriginalOwnerReader>,
    after: Option<u64>,
) -> Result<(Vec<OwnerHoldingView>, Option<u64>), Refusal> {
    let mut rows: Vec<OwnerHoldingView> = Vec::new();
    let mut cursor = after;
    for _ in 0..MAX_READS {
        let request = RollCommandReadRequest::own_holdings(HoldingSection::Owner, cursor);
        let page = command(reads, roll, request)?
            .into_owner_holdings()
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

/// 17g C2g: up to `SECTION_ROWS` paid rows after the licence `after`, and
/// the cursor to show in `HOLD_MORE paid` when the section goes on: the last
/// shown licence, or Roll's own cursor when the reads ran out first.
pub(super) fn paid_section(
    reads: &impl super::Reads,
    roll: &OwnerReadHandle<OriginalOwnerReader>,
    after: Option<u64>,
) -> Result<(Vec<PaidHoldingView>, Option<u64>), Refusal> {
    let mut rows: Vec<PaidHoldingView> = Vec::new();
    let mut cursor = after;
    for _ in 0..MAX_READS {
        let request = RollCommandReadRequest::own_holdings(HoldingSection::Paid, cursor);
        let page = command(reads, roll, request)?
            .into_paid_holdings()
            .ok_or_else(not_open)?;
        for row in page.rows() {
            if rows.len() == SECTION_ROWS {
                let last = rows.last().map(|r| r.licence().get());
                return Ok((rows, last));
            }
            rows.push(row.clone());
        }
        match page.next() {
            None => return Ok((rows, None)),
            Some(next) => cursor = Some(next.get()),
        }
    }
    Ok((rows, cursor))
}

/// 17g C2g: the listing codes of the actor's open (funded) purchases, from
/// the market read's own-only `OwnOpen`.
pub(super) fn waiting(
    reads: &impl super::Reads,
    market: &OwnerReadHandle<MarketCommandReader>,
) -> Result<Vec<String>, Refusal> {
    let open = reads
        .owner_read(market, MarketCommandRequest::own_open())
        .ok()
        .and_then(mush_market::mind_commands::MarketCommandResult::into_own_open)
        .ok_or_else(not_open)?;
    Ok(open.iter().map(|view| view.code().to_owned()).collect())
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

// ---- the paid rows (17g C2d; live since 17g C2g) ----

/// A paid row's flags, each prefixed `; `, in this order: `no resale` (the
/// term is not transferable), `revoked` (the Registry revoked the current
/// term), `selling` (a funded resale holds the buyer's payment), then `renew
/// offer` or `renew paid` (the licence's renewal head: a drafted offer, or
/// one the holder has paid). 17g C2g (the lead's ruling R4) shortened the
/// two renewal words from C2d's `renewal offered` and `renewal paid` so the
/// widest legal row fits the 120-column floor.
///
/// **The one legality rule** (the width test, C2d test 8, enumerates paid
/// rows under it and `paid_flags_follow_the_one_rule` is its truth table;
/// C2c test 4 no longer enumerates paid rows): a paid row carries
/// at most one renewal mark; `selling` only on a transferable, unrevoked
/// licence; `selling` never with a renewal mark (the resale provider refuses
/// an Accept across an open renewal, "resale cannot cross open renewal", and
/// the renewal reader refuses a Draft while a resale is pending,
/// `require_not_pending`); a renewal mark never with an Active phase (the
/// renewal reader refuses a Draft before `predecessor_term_end`); `revoked`
/// with a renewal mark is legal (a revocation admits a matching pending
/// renewal). The widest legal row is a Grace phase with `; no resale;
/// revoked; renew offer`: 118 columns at the two-digit day maxima.
///
/// The rule is a claim about what Roll's providers let a row reach, not a
/// check here: `paid_flags` prints whatever flags the read returns. Its
/// reachability through real rows is the C2h story's (parts 2 and 3).
pub(crate) fn paid_flags(
    transferable: bool,
    revoked: bool,
    selling: bool,
    renewal: Option<RenewalMark>,
) -> String {
    let mut flags = String::new();
    for (on, flag) in [
        (!transferable, "; no resale"),
        (revoked, "; revoked"),
        (selling, "; selling"),
        (renewal == Some(RenewalMark::Offered), "; renew offer"),
        (renewal == Some(RenewalMark::Paid), "; renew paid"),
    ] {
        if on {
            flags.push_str(flag);
        }
    }
    flags
}

/// `HOLD_PAID` from its parts (trailing spaces trimmed): `{kind}` is `House
/// Pin` or `paid`, `{form}` `sealed`, `open` or `open-resale`.
pub(crate) fn paid_row(
    licence: u64,
    house: bool,
    name: &str,
    form: &str,
    phase: &str,
    flags: &str,
) -> String {
    let licence = licence.to_string();
    let name = cut(name, HOLDING_NAME);
    let kind = if house { "House Pin" } else { "paid" };
    fill(
        words::HOLD_PAID,
        &[
            ("licence", &licence),
            ("kind", kind),
            ("name", &name),
            ("form", form),
            ("phase", phase),
            ("flags", flags),
        ],
    )
    .trim_end()
    .to_owned()
}

/// One paid row from Roll's own `OwnPaid` view (the paid section's rows are
/// exactly that view).
pub(crate) fn paid_line(view: &PaidHoldingView) -> String {
    let form = match view.form() {
        PaidForm::Sealed {} => "sealed",
        PaidForm::Open {} => "open",
        PaidForm::OpenResale {} => "open-resale",
    };
    paid_row(
        view.licence().get(),
        view.house(),
        view.name(),
        form,
        &phase_words(view.phase()),
        &paid_flags(
            view.transferable(),
            view.revoked(),
            view.selling(),
            view.renewal(),
        ),
    )
}
