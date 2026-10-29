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
//! **Who reads it** (doc 17h §2.5; D-UM-42): the hull's title holder (the economy's
//! `title_owner`, a character — a title an organization holds is refused here until cut M8c's
//! org holder) and staff read every line; the operator whose lease the rig is under (the
//! yard's lessee, [`crate::port::yard::rig_lease`]) reads only the lines at or after their
//! lease's start (the M8a fix round's F7), and is told nothing of the sealed or unreadable
//! lines, which may be another lessee's. Everyone else is told the hull has no box they can
//! read, in the one sentence a hull with no box and a bad number get too (the observer rule:
//! a refusal names nothing the actor could not already see).
//!
//! **A trip's open is two lines** (the fix round's F1, the 120-column floor): when and where
//! from, then — indented two spaces — the mind, its class, the governor, the licence state
//! and the seal. A line that fails the black box's own check
//! ([`crate::surface::black_box::BlackBoxLine::check`]) is never printed with invented facts:
//! it is counted with the undecodable ones in one closing line.
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
pub const HULL_HELP: &str = "+hull/box #<hull> [n]  the last n lines of a hull's black box, oldest first (8 by default, 32 at most).
Each trip's out and in: when it left and from where, the mind sealed for the trip, its class,
whether it ran under the governor, the state of its licence and the first eight characters of
its seal, as the box recorded them. You can read the box of a hull whose title you hold, or the
lines since your lease began on a hull you lease; staff read any.";

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

/// **How much of a box a reader reads** (doc 17h §2.5; D-UM-42, the M8a fix round's F7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reach {
    /// Staff and the title holder: every line, and the sealed and unreadable counts.
    All,
    /// The rig's lessee: the lines at or after this IC instant (their lease's start).
    Since(i64),
}

/// **How much of `hull`'s box `who` may read**: staff and the title holder read all of it,
/// the rig's lessee the lines since their lease began; `None` for anyone else.
#[must_use]
pub(crate) fn reach(w: &World, who: ObjId, hull: ObjId) -> Option<Reach> {
    if crate::commands::is_staff(w, who) {
        return Some(Reach::All);
    }
    let holds_title = w
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|economy| economy.snapshot().asset(hull))
        .is_some_and(|asset| asset.title_owner == PrincipalId::Object(who));
    if holds_title {
        return Some(Reach::All);
    }
    crate::port::yard::rig_lease(w, hull)
        .filter(|lease| lease.pilot == who)
        .map(|lease| Reach::Since(lease.leased_at_ic))
}

/// **The last `count` lines of `hull`'s box, in words, oldest first** (a trip's open prints as
/// two rows), and — to a reader who reads it all — the lines that count the sealed and the
/// unreadable ones. The reader check comes first, the dark check second (a dark box is told
/// only to a reader), then the box's own emptiness; a lessee with no line since their lease
/// began is told the hull has no box they can read.
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
    let reach = reach(w, who, hull).ok_or_else(|| CmdError::msg(NO_BOX))?;
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
    let (tail, skipped) = store.black_box_lines(hull);
    let mut unreadable = skipped;
    let mut readable: Vec<Vec<String>> = Vec::new();
    for (_, line) in &tail {
        if let Reach::Since(start) = reach
            && line_at(line) < start
        {
            continue;
        }
        match line_words(w, hull, line) {
            Some(rows) => readable.push(rows),
            None => unreadable = unreadable.saturating_add(1),
        }
    }
    if readable.is_empty() && reach != Reach::All {
        return Err(CmdError::msg(NO_BOX));
    }
    let first = readable.len().saturating_sub(count);
    let mut rows: Vec<String> = readable.into_iter().skip(first).flatten().collect();
    if reach == Reach::All {
        // The lines sealed in chunks on disk: every line ever written, less the open tail's
        // decoded and undecodable ones. A count asked below the tail is the reader's choice,
        // not a seal.
        let open = u64::try_from(tail.len().saturating_add(skipped)).unwrap_or(u64::MAX);
        rows.extend(sealed_row(ever.saturating_sub(open)));
        rows.extend(unreadable_row(unreadable));
    }
    if rows.is_empty() {
        return Err(CmdError::msg(NO_BOX));
    }
    Ok(rows)
}

/// The closing line that counts the sealed lines, when there are any.
pub(crate) fn sealed_row(sealed: u64) -> Option<String> {
    match sealed {
        0 => None,
        1 => Some("… and 1 earlier line is sealed in the box.".to_owned()),
        n => Some(format!("… and {n} earlier lines are sealed in the box.")),
    }
}

/// The closing line that counts the lines the box holds but cannot print, when there are any.
pub(crate) fn unreadable_row(unreadable: usize) -> Option<String> {
    match unreadable {
        0 => None,
        1 => Some("1 line in the box could not be read.".to_owned()),
        n => Some(format!("{n} lines in the box could not be read.")),
    }
}

/// A line's IC instant.
const fn line_at(line: &BlackBoxLine) -> i64 {
    match line {
        BlackBoxLine::MindSealed { at_ic, .. } | BlackBoxLine::SortieClosed { at_ic, .. } => *at_ic,
    }
}

/// **One line in words** (doc 17h §2.5's shapes; the M8a fix round's F1). A trip's open is
/// two rows: `<IC date> out · <host> from <hangar>`, then `  <mind> · <class> ·
/// <governed|ungoverned> · licence <state> · seal <8>`; with no seal recorded, one row,
/// `<IC date> out · <host> from <hangar> · no seal`. The close: `<IC date> in · <close> [·
/// recovered by <name>] [· abandoned since <IC date>]`. `None` for a line that fails the
/// black box's own check: nothing is invented for it.
pub(crate) fn line_words(w: &World, hull: ObjId, line: &BlackBoxLine) -> Option<Vec<String>> {
    line.check(hull).ok()?;
    let (hangar, recovered) = match line {
        BlackBoxLine::MindSealed { hangar, .. } => (hangar_words(w, hangar), None),
        BlackBoxLine::SortieClosed { recovered_by, .. } => (
            String::new(),
            recovered_by.map(|by| crate::port::coop::name_of(w, by)),
        ),
    };
    render(line, &hangar, recovered.as_deref())
}

/// [`line_words`]' words over the names it looked up: `hangar` for an open line, `recovered`
/// for a close a staff actor recovered. Pure, so the width test can feed worst-case names.
fn render(line: &BlackBoxLine, hangar: &str, recovered: Option<&str>) -> Option<Vec<String>> {
    match line {
        BlackBoxLine::MindSealed {
            at_ic,
            host,
            seal,
            mind,
            class,
            governed,
            licence,
            licence_state,
            seal_digest,
            ..
        } => {
            let out = format!(
                "{} out · {} from {hangar}",
                crate::port::licence::ic_stamp(*at_ic),
                host_word(*host)
            );
            if *seal == SealWord::Absent {
                return Some(vec![format!("{out} · no seal")]);
            }
            // `check` holds a sealed line to every fact; a missing one is not invented.
            let (Some(mind), Some(class), Some(governed), Some(licence), Some(digest)) =
                (mind, class, governed, licence, seal_digest)
            else {
                return None;
            };
            // A line written before M8a carries no state: the licence label stands in.
            let state: String =
                licence_state.map_or_else(|| licence.to_string(), |s| s.as_str().to_owned());
            Some(vec![
                out,
                format!(
                    "  {mind} · {} · {} · licence {state} · seal {}",
                    class_word(*class),
                    if *governed { "governed" } else { "ungoverned" },
                    digest.get(..8).unwrap_or(digest)
                ),
            ])
        }
        BlackBoxLine::SortieClosed {
            at_ic,
            close,
            abandoned,
            abandoned_ic,
            ..
        } => {
            let mut out = format!(
                "{} in · {}",
                crate::port::licence::ic_stamp(*at_ic),
                close_word(close)
            );
            if let Some(by) = recovered {
                out.push_str(&format!(" · recovered by {by}"));
            }
            if *abandoned && let Some(since) = abandoned_ic {
                out.push_str(&format!(
                    " · abandoned since {}",
                    crate::port::licence::ic_stamp(*since)
                ));
            }
            Some(vec![out])
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
        assert!(
            HULL_HELP.lines().all(|line| line.chars().count() <= 120),
            "{HULL_HELP}"
        );
    }

    /// A 32-character name: the worst case the 120-column floor is held against.
    const LONG: &str = "Abcdefghijklmnopqrstuvwxyzabcdef";

    /// An open line in the S.1 wire shape, sealed at the worst-case words (a 32-character
    /// mind, Class III, ungoverned, recalled), or with `mind` left out.
    fn open_line(with_mind: bool) -> BlackBoxLine {
        let mut value = serde_json::json!({
            "kind": "mind_sealed", "unit": 30, "sortie": {"ic": {"hull": 30, "at_ic": 500}},
            "at_ic": 800, "host": "surface", "hangar": {"hull": 20, "slot": 1, "sortie": null},
            "site": "ahuna", "seal": "sealed", "mind": LONG, "source": "0123456789abcdef",
            "card_digest": "c".repeat(64), "class": "III", "governed": false,
            "licence": "4294967295", "licence_state": "recalled", "seal_digest": "d".repeat(64),
            "admitted": {"host": "surface", "count": 1},
        });
        if !with_mind && let Some(map) = value.as_object_mut() {
            map.remove("mind");
        }
        serde_json::from_value(value).expect("an open line")
    }

    /// **F1: every row fits 120 columns** at the worst-case words: a trip's open is two rows
    /// (the second indented two spaces) over a 32-character carrier, the no-seal open, and a
    /// close a 32-character staff name recovered and abandoned.
    #[test]
    fn m8a_every_box_row_fits_one_hundred_and_twenty_columns() {
        let carrier = format!("the bay of {LONG}");
        let open = render(&open_line(true), &carrier, None).expect("a sealed open");
        assert_eq!(open.len(), 2, "{open:?}");
        assert!(
            open[1].starts_with("  ") && open[1].contains(" · licence recalled · seal dddddddd")
        );
        assert!(open[1].contains("Class III · ungoverned"), "{open:?}");
        let mut absent: serde_json::Value = serde_json::to_value(open_line(true)).expect("encodes");
        if let Some(map) = absent.as_object_mut() {
            for key in [
                "mind",
                "source",
                "card_digest",
                "class",
                "governed",
                "licence",
                "licence_state",
                "seal_digest",
            ] {
                map.remove(key);
            }
            map.insert("seal".into(), "absent".into());
        }
        let absent: BlackBoxLine = serde_json::from_value(absent).expect("an absent open");
        let no_seal = render(&absent, &carrier, None).expect("an absent open");
        assert_eq!(no_seal.len(), 1);
        assert!(no_seal[0].ends_with(" · no seal"), "{no_seal:?}");
        let close: BlackBoxLine = serde_json::from_value(serde_json::json!({
            "kind": "sortie_closed", "unit": 30, "sortie": {"ic": {"hull": 30, "at_ic": 500}},
            "at_ic": 900, "close": {"kind": "recovered_to_base", "base": 5}, "abandoned": true,
            "abandoned_ic": 850, "recovered_by": 7,
        }))
        .expect("a close line");
        let close = render(&close, "", Some(LONG)).expect("a close");
        for row in open.iter().chain(&no_seal).chain(&close) {
            assert!(
                row.chars().count() <= 120,
                "{} columns: {row}",
                row.chars().count()
            );
            assert!(!row.contains("sortie"), "{row}");
        }
    }

    /// **F11: nothing is invented.** A sealed open missing a fact renders nothing (the reader
    /// counts it unreadable), never "Standard Doctrine" or "governed"; and the two closing
    /// lines' words, singular and plural, and none at zero.
    #[test]
    fn m8a_a_line_short_of_its_facts_is_counted_not_invented() {
        assert_eq!(render(&open_line(false), "the bay of X", None), None);
        assert_eq!(sealed_row(0), None);
        assert_eq!(
            sealed_row(1).as_deref(),
            Some("… and 1 earlier line is sealed in the box.")
        );
        assert_eq!(
            sealed_row(3).as_deref(),
            Some("… and 3 earlier lines are sealed in the box.")
        );
        assert_eq!(unreadable_row(0), None);
        assert_eq!(
            unreadable_row(1).as_deref(),
            Some("1 line in the box could not be read.")
        );
        assert_eq!(
            unreadable_row(2).as_deref(),
            Some("2 lines in the box could not be read.")
        );
    }

    /// A booted world holding a store whose box for hull #30 is `ledger`, a staff reader (GOD)
    /// and a player who reads nothing; `dark` sets the boot's refused-restore flag.
    fn box_world(ledger: mush_core::Ledger, dark: bool) -> (World, ObjId) {
        let (mut w, _reg) = mush_testkit::boot_world();
        let stranger = w
            .create(
                mush_core::ObjKind::Player,
                "Stranger",
                mush_core::GOD,
                Some(mush_core::LIMBO),
            )
            .expect("a player");
        let mut store = crate::store::load(&[], crate::config::TacsimConfig::default());
        store.black_boxes = ledger;
        store.black_box_dark = dark;
        w.module_state_insert(store);
        (w, stranger)
    }

    /// The one sentence a refused read says, if it was refused.
    fn refusal(read: Result<Vec<String>, CmdError>) -> Option<&'static str> {
        match read {
            Err(CmdError::Msg(text)) if text == NO_BOX => Some(NO_BOX),
            Err(CmdError::Msg(text)) if text == DARK => Some(DARK),
            _ => None,
        }
    }

    /// The close line of the epoch `open_line` opened.
    fn close_line() -> BlackBoxLine {
        serde_json::from_value(serde_json::json!({
            "kind": "sortie_closed", "unit": 30, "sortie": {"ic": {"hull": 30, "at_ic": 500}},
            "at_ic": 900, "close": {"kind": "returned_to_base", "base": 20}, "abandoned": false,
        }))
        .expect("a close line")
    }

    fn append(ledger: &mut mush_core::Ledger, value: serde_json::Value) {
        let prepared = ledger
            .prepare_append(crate::surface::black_box::log_id(ObjId(30)).as_str(), value)
            .expect("the line prepares");
        let _ = ledger.apply_prepared(prepared).expect("the line appends");
    }

    /// **F10/F11 over a store**: a box with a good open, an entry that does not decode, a
    /// sealed open short of its facts (it fails the line's own check) and a good close reads,
    /// to staff, the three rows of the two good lines and one line counting the two it cannot
    /// print — and nothing at all to a player who is neither holder nor lessee.
    #[test]
    fn m8a_staff_read_the_lines_and_the_count_of_the_unreadable() {
        let mut ledger = mush_core::Ledger::new();
        append(
            &mut ledger,
            serde_json::to_value(open_line(true)).expect("encodes"),
        );
        append(&mut ledger, serde_json::json!({"kind": "not_a_line"}));
        append(
            &mut ledger,
            serde_json::to_value(open_line(false)).expect("encodes"),
        );
        append(
            &mut ledger,
            serde_json::to_value(close_line()).expect("encodes"),
        );
        let (w, stranger) = box_world(ledger, false);
        let rows = box_lines(&w, mush_core::GOD, ObjId(30), 8).expect("staff read it");
        assert_eq!(rows.len(), 4, "{rows:?}");
        assert!(rows[0].contains(" out · surface from "), "{rows:?}");
        assert!(rows[1].starts_with("  "), "{rows:?}");
        assert!(rows[2].contains(" in · returned"), "{rows:?}");
        assert_eq!(rows[3], "2 lines in the box could not be read.");
        assert!(
            !rows.iter().any(|r| r.contains("Standard Doctrine")),
            "{rows:?}"
        );
        assert_eq!(refusal(box_lines(&w, stranger, ObjId(30), 8)), Some(NO_BOX));
    }

    /// **F10: the sealed line.** A ledger that seals every entry into a chunk leaves the open
    /// tail empty: staff read the one line counting what is sealed in the box, never zero
    /// lines; the stranger reads the one sentence.
    #[test]
    fn m8a_staff_read_how_many_lines_are_sealed() {
        let mut ledger = mush_core::Ledger::with_chunk_target(1);
        append(
            &mut ledger,
            serde_json::to_value(open_line(true)).expect("encodes"),
        );
        append(
            &mut ledger,
            serde_json::to_value(close_line()).expect("encodes"),
        );
        let (w, stranger) = box_world(ledger, false);
        let rows = box_lines(&w, mush_core::GOD, ObjId(30), 8).expect("staff read it");
        assert_eq!(
            rows.last().map(String::as_str),
            Some("… and 2 earlier lines are sealed in the box."),
            "{rows:?}"
        );
        assert_eq!(refusal(box_lines(&w, stranger, ObjId(30), 8)), Some(NO_BOX));
    }

    /// **F10: the dark box.** A boot that refused the restore leaves every box dark: a reader
    /// is told so in `DARK`'s one sentence; anyone else still reads only that the hull has no
    /// box they can read (the reader check comes first).
    #[test]
    fn m8a_a_dark_box_is_told_to_a_reader_alone() {
        let mut ledger = mush_core::Ledger::new();
        append(
            &mut ledger,
            serde_json::to_value(open_line(true)).expect("encodes"),
        );
        let (w, stranger) = box_world(ledger, true);
        assert_eq!(
            refusal(box_lines(&w, mush_core::GOD, ObjId(30), 8)),
            Some(DARK)
        );
        assert_eq!(refusal(box_lines(&w, stranger, ObjId(30), 8)), Some(NO_BOX));
    }
}
