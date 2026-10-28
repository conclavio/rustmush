//! **`+hull/box`: a hull's black box, read by those who may** (doc 17h cut M8a; doc 23 §7,
//! §10; doc 17 §13, D-UM-42).
//!
//! The black box is a record ([`crate::surface::black_box`]), not a surface; this is the one
//! surface over it, and it prints the record in words — the open and close of each trip, oldest
//! first, the seal's class, governor and licence state, and the first eight characters of the
//! seal's digest. It is a pure read of the store's open tail
//! ([`crate::store::TacsimStore::black_box_lines`]): sealed chunks stay on disk, and the line that says how
//! many earlier lines are sealed in the box is the whole of what this surface says of them
//! (their continuation read is cut M8d's and M8g's).
//!
//! **Who reads it** (doc 17h §2.5): the hull's title holder (the economy's `title_owner`, a
//! character — a title an organization holds is refused here until cut M8c's org holder), the
//! operator whose lease the rig is under (the yard's lessee,
//! [`crate::port::yard::rig_lease`]), and staff. Everyone else is told the hull has no box they
//! can read, in the one sentence a hull with no box and a bad number get too (the observer
//! rule: a refusal names nothing the actor could not already see).
//!
//! The word "sortie" reaches no line here (doc 23 Q-23-5): a line is a trip, out and in.

use mush_core::{CmdError, Ctx, ObjId, World};
use mush_org::PrincipalId;
use mush_roll::commerce::ic_claims::IcCloseKind;

use crate::arena::core::CoreClass;
use crate::store::TacsimStore;
use crate::surface::black_box::{BlackBoxLine, EpochHost, Hangar, SealWord};

/// The verb.
pub const HULL_VERB: &str = "+hull";

/// The help text.
pub const HULL_HELP: &str = "+hull/box #<hull> [n]  the last n lines of a hull's black box, oldest first (8 by default, 32 at most): each trip's out and in — when it left and from where, the mind sealed for the trip, its class, whether it ran under the governor, the state of its licence and the first eight characters of its seal — as the box recorded them.
You can read the box of a hull whose title you hold or whose lease you are under; staff read any.";

/// The usage line, for an argument shape that is not `#<hull> [n]` at all.
const USAGE: &str = "Usage: +hull/box #<hull> [n]";

/// The one refusal (the observer rule): an unreadable hull, a hull with no box, a bad number.
pub const NO_BOX: &str = "That hull has no box you can read.";

/// A box the boot left dark (a refused restore; doc 23 cut S.1).
pub const DARK: &str = "The box is dark; nothing can be read until the next restart.";

/// Lines printed when no number is given.
const DEFAULT_LINES: usize = 8;

/// The most lines one read prints.
const MAX_LINES: usize = 32;

/// `+hull/box #<hull> [n]`.
///
/// # Errors
///
/// The usage line without the `/box` switch or with no `#<hull>` word; [`NO_BOX`] for a hull
/// the actor may not read, a hull with no box or a count outside `1..=32`; [`DARK`] when the
/// boot left the boxes dark.
pub(crate) fn hull_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    crate::commands::require_enabled(ctx)?;
    if !ctx.switches.has("box") {
        return Err(CmdError::msg(USAGE));
    }
    let mut words = ctx.args.raw().split_whitespace();
    let hull = words
        .next()
        .and_then(|word| word.strip_prefix('#'))
        .ok_or_else(|| CmdError::msg(USAGE))?;
    let hull = crate::port::families::strict_decimal_u32(hull)
        .map(ObjId)
        .ok_or_else(|| CmdError::msg(NO_BOX))?;
    let count = match words.next() {
        None => DEFAULT_LINES,
        Some(word) => crate::port::families::strict_decimal_u32(word)
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| (1..=MAX_LINES).contains(n))
            .ok_or_else(|| CmdError::msg(NO_BOX))?,
    };
    if words.next().is_some() {
        return Err(CmdError::msg(NO_BOX));
    }
    let actor = ctx.actor;
    let rows = box_lines(ctx.w, actor, hull, count)?;
    for row in rows {
        ctx.out.sys_to_actor(actor, "tacsim", row);
    }
    Ok(())
}

/// **Whether `who` may read `hull`'s box** (doc 17h §2.5): staff, the title holder, or the
/// lessee of the rig.
#[must_use]
pub(crate) fn may_read(w: &World, who: ObjId, hull: ObjId) -> bool {
    if crate::commands::is_staff(w, who) {
        return true;
    }
    let holds_title = w
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|economy| economy.snapshot().asset(hull))
        .is_some_and(|asset| asset.title_owner == PrincipalId::Object(who));
    holds_title || crate::port::yard::rig_lease(w, hull).is_some_and(|lease| lease.pilot == who)
}

/// **The last `count` lines of `hull`'s box, in words, oldest first**, and the line that counts
/// the earlier ones when the box holds more than is printed. The reader check comes first, the
/// dark check second (a dark box is told only to a reader), then the box's own emptiness.
///
/// # Errors
///
/// [`NO_BOX`] or [`DARK`], as the command's.
pub(crate) fn box_lines(
    w: &World,
    who: ObjId,
    hull: ObjId,
    count: usize,
) -> Result<Vec<String>, CmdError> {
    if !may_read(w, who, hull) {
        return Err(CmdError::msg(NO_BOX));
    }
    let store = w
        .module_state::<TacsimStore>()
        .ok_or_else(|| CmdError::msg(NO_BOX))?;
    if store.black_box_dark() {
        return Err(CmdError::msg(DARK));
    }
    let ever = store.black_box_len(hull);
    if ever == 0 {
        return Err(CmdError::msg(NO_BOX));
    }
    let (tail, _skipped) = store.black_box_lines(hull);
    let start = tail.len().saturating_sub(count);
    let mut rows: Vec<String> = tail
        .iter()
        .skip(start)
        .map(|(_, line)| line_words(w, line))
        .collect();
    // The lines sealed in chunks on disk, not the open tail's: a count asked below the tail
    // is the reader's choice, not a seal.
    let open = u64::try_from(tail.len()).unwrap_or(u64::MAX);
    let sealed = ever.saturating_sub(open);
    if sealed == 1 {
        rows.push("… and 1 earlier line is sealed in the box.".to_owned());
    } else if sealed > 1 {
        rows.push(format!(
            "… and {sealed} earlier lines are sealed in the box."
        ));
    }
    Ok(rows)
}

/// **One line in words** (doc 17h §2.5's shapes). The open line: `<IC date> out · <host> from
/// <hangar> · <mind> · <class> · <governed|ungoverned> · licence <state> · seal <8>`; with no
/// seal recorded, `<IC date> out · <host> from <hangar> · no seal`. The close line: `<IC date>
/// in · <close> [· recovered by <name>] [· abandoned since <IC date>]`.
fn line_words(w: &World, line: &BlackBoxLine) -> String {
    match line {
        BlackBoxLine::MindSealed {
            at_ic,
            host,
            hangar,
            seal,
            mind,
            class,
            governed,
            licence,
            licence_state,
            seal_digest,
            ..
        } => {
            let mut out = format!(
                "{} out · {} from {}",
                crate::port::licence::ic_stamp(*at_ic),
                host_word(*host),
                hangar_words(w, hangar)
            );
            if *seal == SealWord::Absent {
                out.push_str(" · no seal");
                return out;
            }
            let mind = mind.as_deref().unwrap_or("Standard Doctrine");
            let class = class_word(class.unwrap_or(CoreClass::Zero));
            let governed = if governed.unwrap_or(true) {
                "governed"
            } else {
                "ungoverned"
            };
            // A line written before M8a carries no state: the licence label stands in.
            let state = licence_state.map_or_else(
                || licence.as_deref().unwrap_or("doctrine").to_owned(),
                |state| state.as_str().to_owned(),
            );
            let digest = seal_digest.as_deref().unwrap_or_default();
            let digest = digest.get(..8).unwrap_or(digest);
            out.push_str(&format!(
                " · {mind} · {class} · {governed} · licence {state} · seal {digest}"
            ));
            out
        }
        BlackBoxLine::SortieClosed {
            at_ic,
            close,
            abandoned,
            abandoned_ic,
            recovered_by,
            ..
        } => {
            let mut out = format!(
                "{} in · {}",
                crate::port::licence::ic_stamp(*at_ic),
                close_word(close)
            );
            if let Some(by) = recovered_by {
                out.push_str(&format!(
                    " · recovered by {}",
                    crate::port::coop::name_of(w, *by)
                ));
            }
            if *abandoned && let Some(since) = abandoned_ic {
                out.push_str(&format!(
                    " · abandoned since {}",
                    crate::port::licence::ic_stamp(*since)
                ));
            }
            out
        }
    }
}

/// `surface` or `port`.
const fn host_word(host: EpochHost) -> &'static str {
    match host {
        EpochHost::Surface => "surface",
        EpochHost::Port => "port",
    }
}

/// `the bay of <carrier>` or the port's name.
fn hangar_words(w: &World, hangar: &Hangar) -> String {
    match hangar {
        Hangar::Bay { hull, .. } => format!("the bay of {}", crate::port::coop::name_of(w, *hull)),
        Hangar::Port { entry, base } => crate::port::voyage::lane_ports(w)
            .into_iter()
            .find_map(|(def, map)| {
                map.filter(|map| map.entry == *entry)
                    .map(|_| crate::port::coop::port_name(def))
            })
            .unwrap_or_else(|| crate::port::coop::name_of(w, *base)),
    }
}

/// `the ROM`, `Class I`, `Class II`, `Class III`.
pub(crate) const fn class_word(class: CoreClass) -> &'static str {
    match class {
        CoreClass::Zero => "the ROM",
        CoreClass::I => "Class I",
        CoreClass::II => "Class II",
        CoreClass::III => "Class III",
    }
}

/// `returned`, `powered down`, `destroyed`, `recovered`.
const fn close_word(close: &IcCloseKind) -> &'static str {
    match close {
        IcCloseKind::ReturnedToBase { .. } => "returned",
        IcCloseKind::PoweredDownInBase { .. } => "powered down",
        IcCloseKind::Destroyed => "destroyed",
        IcCloseKind::RecoveredToBase { .. } => "recovered",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The words a line prints: the class, the close kind and the host.
    #[test]
    fn m8a_the_words() {
        assert_eq!(class_word(CoreClass::Zero), "the ROM");
        assert_eq!(class_word(CoreClass::III), "Class III");
        assert_eq!(
            close_word(&IcCloseKind::RecoveredToBase { base: ObjId(1) }),
            "recovered"
        );
        assert_eq!(close_word(&IcCloseKind::Destroyed), "destroyed");
        assert_eq!(host_word(EpochHost::Port), "port");
        assert!(
            !HULL_HELP.contains("sortie") && !NO_BOX.contains("sortie") && !DARK.contains("sortie")
        );
        assert!(HULL_HELP.lines().all(|line| line.len() <= 400));
    }
}
