//! **What a field tells a player who is not there** (doc 45 cut AP.1; Q-45-17, Q-45-21).
//!
//! A field's opening and settling were said only to whoever stood on the site, with a line that
//! an offline character never receives (`super::tell_site`). A player who was away learned of a
//! lost rig by finding it gone. This module tells **every player with a stake in the field**,
//! connected or not, through the two paths the game already has: the reconnect queue
//! ([`crate::port_tick::notify_budget`], trip level) and the site's IC mail
//! (`crate::surface::claim::letter`).
//!
//! - **A stake** is a unit of the field and the players behind it: whoever sat in it at
//!   admission, the operator of a rig out on an excursion, a carrier's commander
//!   ([`crate::port::coop::helm_holder`]), and its party's principals (its leader and, for a
//!   roster, its organizer). Only players: a raider's NPC leader is told nothing. A player with
//!   nothing on the site holds no stake and is told nothing.
//! - **The opening.** The field's first snapshot (the opening pulse's) reads the stakes, and each
//!   player is told [`super::FIELD_OPENED`] once: queued when they are not on watch, said when
//!   they are and the site's own line did not reach them (a commander who is elsewhere). A player
//!   the site's line reached hears it once, as before. A unit that joins the running field brings
//!   its players in the same way at the next snapshot.
//! - **The close.** Each stakeholder is told [`super::FIELD_SETTLED`] by the same rule, then what
//!   became of each of their units: one line a [`Fate`], naming the units.
//! - **Mail.** A player whose units include a loss, a casualty, a capture or a tow, or who is
//!   stranded on the site, is sent one letter listing every unit, so the record outlives a login.
//!   A character captured later, when their casualty is hauled into a captor's bay, is queued the
//!   capture's line and sent it by mail (`captured`).
//!
//! **The notice budget.** One field costs a player at most one opening line, one settling line,
//! one line a fate (seven) and one stranded line of the reconnect queue's
//! [`crate::port_tick::MAX_NOTICE_LINES`], however many units they have there: a fate's line
//! names at most [`MAX_NAMES_A_LINE`] units and counts the rest, and the letter carries the
//! whole list (to [`MAX_LETTER_UNITS`]).
//!
//! **What is RAM.** The stakes are module state (`Stakes`), since a unit that leaves at its
//! carrier's ramp drops off the live record before the close. A restart forgets them: the resumed
//! field's first snapshot reads them again from the units still in it, says no second opening,
//! and the close reports those units only.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{CanonicalId, Ctx, Flags, ObjId, ObjKind, World};

use super::close::Demoted;
use super::crew::CrewLocation;
use super::{FieldBook, FieldLive, FieldUnit};
use crate::arena::PartyId;
use crate::port_tick::{NoticeLevel, notify_budget, on_watch};
use crate::surface::act::Where;
use crate::surface::site_record::{LeftCondition, SiteRecord};

/// **The most units one fate's line names** (the notice budget): the rest are counted.
pub const MAX_NAMES_A_LINE: usize = 4;
/// The most bytes of a unit's name a line keeps.
const MAX_NAME_BYTES: usize = 28;
/// **The most units a letter lists**: the rest are counted.
pub const MAX_LETTER_UNITS: usize = 24;

/// **What became of a stakeholder's units** (`{n}` the ordinal, `{site}` its name, `{fate}`
/// [`Fate::word`], `{units}` their names). One line a fate.
pub const AFTER_FIELD: &str = "After field {n} on {site}, {fate}: {units}.";
/// The letter's subject.
pub const LETTER_SUBJECT: &str = "Field {n} on {site}: your units";
/// The letter's first line.
pub const LETTER_HEAD: &str = "Field {n} on {site} has settled. What became of your units there:";
/// The letter's last line.
pub const LETTER_TAIL: &str = "A casualty or a wreck left on a site may be towed, and whoever takes it into custody may claim it as salvage; +salvage lists the claims you stand in.";
/// A capture's letter's subject (`{site}`).
pub const CAPTURE_SUBJECT: &str = "Taken captive on {site}";

/// **What became of one unit when its field settled.** In the order a report lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Fate {
    /// A wreck, or remains: destroyed.
    Lost,
    /// Aboard a carrier that is not its party's or a Friend's.
    Captured,
    /// In tow behind a tug that is not its party's or a Friend's.
    InTow,
    /// A casualty lying on the site: immobile, unarmed or disabled.
    Down,
    /// Standing on the site's ground where the field left it.
    Holding,
    /// Aboard its own party's carrier or a Friend's; a carrier still on its pad.
    Home,
    /// No longer on the site, and aboard nothing.
    Away,
}

impl Fate {
    /// The word a line prints.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Fate::Lost => "lost",
            Fate::Captured => "captured, aboard another's carrier",
            Fate::InTow => "taken in tow",
            Fate::Down => "down on the site, a casualty",
            Fate::Holding => "holding on the site",
            Fate::Home => "home",
            Fate::Away => "gone from the site",
        }
    }

    /// Whether a unit that met this fate is worth a letter.
    #[must_use]
    pub const fn mailed(self) -> bool {
        matches!(self, Fate::Lost | Fate::Captured | Fate::InTow | Fate::Down)
    }
}

/// One unit's stake: its party as the field lettered it, and the players behind it.
#[derive(Clone, Debug)]
struct Stake {
    party: PartyId,
    players: BTreeSet<ObjId>,
}

/// **Who has what in one field** (RAM; this module's head): every unit the field has held, and
/// the players already told it opened. It is one field's, by its ordinal: stakes a field left
/// behind without a close are not the next field's.
#[derive(Clone, Debug, Default)]
pub(crate) struct Stakes {
    ordinal: u32,
    units: BTreeMap<ObjId, Stake>,
    told: BTreeSet<ObjId>,
}

/// A live player, the only kind of character a notice is for.
fn player(w: &World, who: ObjId) -> bool {
    w.get(who).is_some_and(|object| {
        object.kind == ObjKind::Player && !object.flags.contains(Flags::GOING)
    })
}

/// **The players behind `row`** (this module's head, "a stake").
fn holders(ctx: &Ctx<'_>, live: &FieldLive, row: &FieldUnit) -> BTreeSet<ObjId> {
    let mut out = BTreeSet::new();
    if let Some((pilot, _)) = &row.pilot {
        out.insert(*pilot);
    }
    if let Some(operator) = super::parties::rig_record(ctx, row.unit)
        .and_then(|record| record.excursion.as_ref().map(|open| open.operator))
    {
        out.insert(operator);
    }
    if super::carrier::is_carrier(ctx.w, row.unit)
        && let Some(commander) = crate::port::coop::helm_holder(ctx.w, row.unit)
    {
        out.insert(commander);
    }
    if let Some(party) = live.parties.iter().find(|party| party.party == row.party) {
        out.extend(super::parties::principals(party));
    }
    out.retain(|who| player(ctx.w, *who));
    out
}

/// A field's opening line.
fn opened_line(ctx: &Ctx<'_>, site: &CanonicalId, ordinal: u32) -> String {
    super::FIELD_OPENED
        .replace("{n}", &ordinal.to_string())
        .replace("{site}", &super::admit::site_name(ctx, site))
}

/// **Read the stakes of `site`'s field** (the host's snapshot branch, and the close): every unit
/// on the live record the stakes do not hold yet, with the players behind it. `opening` is the
/// field's first snapshot. A player a new unit brings is told the field is open, once, unless
/// the stakes are being read for the first time on a field that is not opening (a resume: its
/// players were told before the restart).
pub(crate) fn fold(ctx: &mut Ctx<'_>, site: &CanonicalId, opening: bool) {
    let Some((live, seeded)) = ctx.w.module_state::<FieldBook>().and_then(|book| {
        let hosted = book.hosted(site)?;
        let known = book
            .stakes
            .get(site)
            .filter(|stakes| stakes.ordinal == hosted.live.field.ordinal);
        hosted
            .live
            .units
            .iter()
            .any(|row| known.is_none_or(|stakes| !stakes.units.contains_key(&row.unit)))
            .then(|| (hosted.live.clone(), known.is_some()))
    }) else {
        return;
    };
    let fresh: Vec<(ObjId, Stake)> = {
        let known = ctx
            .w
            .module_state::<FieldBook>()
            .and_then(|book| book.stakes.get(site))
            .filter(|stakes| stakes.ordinal == live.field.ordinal);
        live.units
            .iter()
            .filter(|row| known.is_none_or(|stakes| !stakes.units.contains_key(&row.unit)))
            .map(|row| {
                (
                    row.unit,
                    Stake {
                        party: row.party,
                        players: holders(ctx, &live, row),
                    },
                )
            })
            .collect()
    };
    let mut brought: Vec<ObjId> = Vec::new();
    if let Some(book) = ctx.w.module_state_mut::<FieldBook>() {
        let stakes = book.stakes.entry(site.clone()).or_default();
        if stakes.ordinal != live.field.ordinal {
            *stakes = Stakes {
                ordinal: live.field.ordinal,
                ..Stakes::default()
            };
        }
        for (unit, stake) in fresh {
            for who in &stake.players {
                if stakes.told.insert(*who) {
                    brought.push(*who);
                }
            }
            stakes.units.insert(unit, stake);
        }
    }
    if !(opening || seeded) || brought.is_empty() {
        return;
    }
    let line = opened_line(ctx, site, live.field.ordinal);
    let audience = super::site_audience(ctx, site);
    for who in brought {
        // The site's own line reached them at admission.
        if audience.contains(&who) && on_watch(ctx.w, who) {
            continue;
        }
        notify_budget(ctx, who, &line, NoticeLevel::Trip);
    }
}

/// One stakeholder's report at the close.
#[derive(Clone, Debug)]
struct Report {
    who: ObjId,
    fates: BTreeMap<Fate, Vec<String>>,
    stranded: Option<String>,
}

/// **What a field's close owes its stakeholders** ([`closing`]'s answer, [`tell_closed`]'s input).
#[derive(Clone, Debug, Default)]
pub(crate) struct Closing {
    ordinal: u32,
    reports: Vec<Report>,
}

/// What [`fate`] reads beside the unit itself.
struct Ground<'a> {
    domain: Option<mush_sim::DomainId>,
    demoted: &'a Demoted,
    carried: BTreeMap<ObjId, (ObjId, u32)>,
    friends: BTreeSet<(PartyId, PartyId)>,
    parties: BTreeMap<ObjId, PartyId>,
}

/// **What became of `unit`** (this module's head): from what the demotion left on the site and
/// where the unit stands now.
fn fate(w: &World, unit: ObjId, party: PartyId, ground: &Ground<'_>) -> Fate {
    let left = |condition: LeftCondition| {
        ground
            .demoted
            .left
            .iter()
            .any(|row| row.unit == unit && row.condition == condition)
    };
    if ground.demoted.remains.iter().any(|row| row.unit == unit)
        || ground.demoted.wrecked.contains(&unit)
        || left(LeftCondition::Wreck)
        || super::close::is_irrecoverable(w, unit)
    {
        return Fate::Lost;
    }
    // Another party's, and no Friend's: a hull with no party in this field is nobody's captor.
    let stranger = |other: ObjId| {
        ground.parties.get(&other).is_some_and(|theirs| {
            *theirs != party
                && !ground
                    .friends
                    .contains(&((*theirs).min(party), (*theirs).max(party)))
        })
    };
    match crate::surface::act::where_is(w, unit) {
        Where::Bay { hull } if stranger(hull) => Fate::Captured,
        Where::Bay { .. } => Fate::Home,
        Where::Ground { domain, .. } if Some(domain) == ground.domain => {
            if ground
                .carried
                .get(&unit)
                .is_some_and(|(tug, _)| stranger(*tug))
            {
                Fate::InTow
            } else if left(LeftCondition::Casualty) {
                Fate::Down
            } else if super::carrier::is_carrier(w, unit) {
                Fate::Home
            } else {
                Fate::Holding
            }
        }
        _ => Fate::Away,
    }
}

/// A unit's name as a line prints it, cut to [`MAX_NAME_BYTES`] on a char boundary.
fn unit_name(w: &World, unit: ObjId) -> String {
    let mut name = crate::port::coop::name_of(w, unit);
    if name.is_empty() {
        name = format!("#{}", unit.0);
    }
    if name.len() > MAX_NAME_BYTES {
        let mut cut = MAX_NAME_BYTES;
        while cut > 0 && !name.is_char_boundary(cut) {
            cut = cut.saturating_sub(1);
        }
        name.truncate(cut);
    }
    name
}

/// **What `site`'s closing field owes its stakeholders** (`close::on_ended`, while the host still
/// holds the field and after its units and crews are checked in): each player's units by fate,
/// and whether the close left the player stranded there. The site's stakes are forgotten.
pub(crate) fn closing(ctx: &mut Ctx<'_>, site: &CanonicalId, demoted: &Demoted) -> Closing {
    // A unit that joined since the last snapshot.
    fold(ctx, site, false);
    let Some((live, stakes)) = ctx.w.module_state_mut::<FieldBook>().and_then(|book| {
        let stakes = book.stakes.remove(site)?;
        Some((book.hosted(site)?.live.clone(), stakes))
    }) else {
        return Closing::default();
    };
    let record: Option<SiteRecord> = super::close::site_record(ctx, site);
    let mut parties: BTreeMap<ObjId, PartyId> = stakes
        .units
        .iter()
        .map(|(unit, stake)| (*unit, stake.party))
        .collect();
    parties.extend(live.units.iter().map(|row| (row.unit, row.party)));
    let ground = Ground {
        domain: super::parties::site_domain(ctx.w, site),
        demoted,
        carried: record
            .as_ref()
            .map(crate::surface::tow::carried)
            .unwrap_or_default(),
        friends: super::parties::friends_of(ctx.w, &live.parties)
            .into_iter()
            .collect(),
        parties,
    };
    let mut by_player: BTreeMap<ObjId, BTreeMap<Fate, Vec<String>>> = BTreeMap::new();
    for (unit, stake) in &stakes.units {
        if stake.players.is_empty() {
            continue;
        }
        let met = fate(ctx.w, *unit, stake.party, &ground);
        let name = unit_name(ctx.w, *unit);
        for who in &stake.players {
            by_player
                .entry(*who)
                .or_default()
                .entry(met)
                .or_default()
                .push(name.clone());
        }
    }
    let site_name = super::admit::site_name(ctx, site);
    let reports = by_player
        .into_iter()
        .filter(|(who, _)| player(ctx.w, *who))
        .map(|(who, fates)| Report {
            who,
            fates,
            stranded: super::crew::crew_of(ctx.w, who).and_then(|record| match &record.location {
                CrewLocation::Stranded { site: on, unit } if on == site => Some(
                    super::crew::STRANDED
                        .replace("{unit}", &unit.0.to_string())
                        .replace("{site}", &site_name),
                ),
                _ => None,
            }),
        })
        .collect();
    Closing {
        ordinal: live.field.ordinal,
        reports,
    }
}

/// The units of one fate as a line names them: at most [`MAX_NAMES_A_LINE`], the rest counted.
fn named(names: &[String]) -> String {
    let shown: Vec<&str> = names
        .iter()
        .take(MAX_NAMES_A_LINE)
        .map(String::as_str)
        .collect();
    let rest = names.len().saturating_sub(shown.len());
    if rest == 0 {
        shown.join(", ")
    } else {
        format!("{} and {rest} more", shown.join(", "))
    }
}

/// One fate's line of a report.
fn fate_line(ordinal: u32, site_name: &str, fate: Fate, names: &[String]) -> String {
    AFTER_FIELD
        .replace("{n}", &ordinal.to_string())
        .replace("{site}", site_name)
        .replace("{fate}", fate.word())
        .replace("{units}", &named(names))
}

/// A report's letter: its subject and its body, every unit a line to [`MAX_LETTER_UNITS`].
fn letter_of(ordinal: u32, site_name: &str, report: &Report) -> (String, String) {
    let fill = |text: &str| {
        text.replace("{n}", &ordinal.to_string())
            .replace("{site}", site_name)
    };
    let mut body = vec![fill(LETTER_HEAD)];
    let rows: Vec<String> = report
        .fates
        .iter()
        .flat_map(|(fate, names)| {
            names
                .iter()
                .map(move |name| format!("{name}: {}.", fate.word()))
        })
        .collect();
    let rest = rows.len().saturating_sub(MAX_LETTER_UNITS);
    body.extend(rows.into_iter().take(MAX_LETTER_UNITS));
    if rest > 0 {
        body.push(format!("And {rest} more."));
    }
    if let Some(stranded) = &report.stranded {
        body.push(stranded.clone());
    }
    body.push(LETTER_TAIL.to_owned());
    (fill(LETTER_SUBJECT), body.join("\n"))
}

/// **Tell a settled field's stakeholders** (`close::on_ended`, after the site's own line):
/// `settled` to each the site's line did not reach, what became of their units a line a fate,
/// the stranded line to one who was not there to hear it, and a letter for a loss.
pub(crate) fn tell_closed(ctx: &mut Ctx<'_>, site: &CanonicalId, settled: &str, closing: Closing) {
    if closing.reports.is_empty() {
        return;
    }
    let audience = super::site_audience(ctx, site);
    let site_name = super::admit::site_name(ctx, site);
    for report in closing.reports {
        let watching = on_watch(ctx.w, report.who);
        if !(watching && audience.contains(&report.who)) {
            notify_budget(ctx, report.who, settled, NoticeLevel::Trip);
        }
        for (fate, names) in &report.fates {
            let line = fate_line(closing.ordinal, &site_name, *fate, names);
            notify_budget(ctx, report.who, &line, NoticeLevel::Trip);
        }
        if let Some(stranded) = report.stranded.as_ref().filter(|_| !watching) {
            notify_budget(ctx, report.who, stranded, NoticeLevel::Trip);
        }
        if report.stranded.is_some() || report.fates.keys().any(|fate| fate.mailed()) {
            let (subject, body) = letter_of(closing.ordinal, &site_name, &report);
            let _ = crate::surface::claim::letter(ctx, site, report.who, &subject, &body);
        }
    }
}

/// **A character taken captive is told** (doc 45 cut AP.1; Q-45-21; `crew::captured_loaded`):
/// `line` said as before, queued for a player who is not on watch, and sent to a player by IC
/// mail under the site's name.
pub(crate) fn captured(ctx: &mut Ctx<'_>, site: &CanonicalId, who: ObjId, line: &str) {
    ctx.out.sys_to_actor(who, "tacsim.field", line.to_owned());
    if !player(ctx.w, who) {
        return;
    }
    if !on_watch(ctx.w, who) {
        crate::port_tick::queue_notice(ctx, who, line);
    }
    let subject = CAPTURE_SUBJECT.replace("{site}", &super::admit::site_name(ctx, site));
    let _ = crate::surface::claim::letter(ctx, site, who, &subject, line);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(fates: &[(Fate, &[&str])]) -> Report {
        Report {
            who: ObjId(7),
            fates: fates
                .iter()
                .map(|(fate, names)| (*fate, names.iter().map(|name| (*name).to_owned()).collect()))
                .collect(),
            stranded: None,
        }
    }

    /// **The notice budget**: a fate's line names four units and counts the rest, and stays one
    /// row of the reconnect queue however many units a player has in the field.
    #[test]
    fn ap1_a_fates_line_names_four_units_and_counts_the_rest() {
        let names: Vec<String> = (0..40)
            .map(|n| format!("A Rig With A Long Name {n:02} Xx"))
            .collect();
        let line = fate_line(12, "Urvara Reach", Fate::Captured, &names);
        assert!(
            line.len() <= crate::port_tick::MAX_NOTICE_BYTES,
            "{} bytes: {line}",
            line.len()
        );
        assert!(line.ends_with(" and 36 more."), "{line}");
        assert_eq!(
            fate_line(1, "Ahuna Flank", Fate::Lost, &["Dust Mule".to_owned()]),
            "After field 1 on Ahuna Flank, lost: Dust Mule."
        );
    }

    /// A letter lists every unit under its fate, the losses first, and is sent only for a fate
    /// worth one.
    #[test]
    fn ap1_a_letter_lists_every_unit_and_is_owed_only_for_a_loss() {
        let lost = report(&[(Fate::Home, &["Salt Moth"]), (Fate::Lost, &["Dust Mule"])]);
        let (subject, body) = letter_of(3, "Ahuna Flank", &lost);
        assert_eq!(subject, "Field 3 on Ahuna Flank: your units");
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines[1], "Dust Mule: lost.");
        assert_eq!(lines[2], "Salt Moth: home.");
        assert_eq!(lines.last().copied(), Some(LETTER_TAIL));
        assert!(lost.fates.keys().any(|fate| fate.mailed()));
        let fine = report(&[
            (Fate::Home, &["Salt Moth"]),
            (Fate::Holding, &["Dust Mule"]),
        ]);
        assert!(!fine.fates.keys().any(|fate| fate.mailed()));
        // The list is bounded, and says what it left out.
        let many: Vec<String> = (0..30).map(|n| format!("Rig {n}")).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        let (_, body) = letter_of(3, "Ahuna Flank", &report(&[(Fate::Lost, &many)]));
        assert!(body.contains("And 6 more."), "{body}");
    }
}
