//! The single name-to-object resolver used by every command handler
//! (design doc §3.2 "Name matching for targets").
//!
//! [`match_object`] implements the classic precedence order:
//!
//! 1. `me` — the looker itself
//! 2. `here` — the looker's location
//! 3. `#123` — direct dbref (must exist)
//! 4. `*name` — player/NPC index lookup (global)
//! 5. exact name/alias in the looker's **inventory**
//! 6. exact name/alias in the looker's **location**
//! 7. exact **exit** name in the location
//! 8. unambiguous **prefix** over inventory + location contents + exits
//! 9. english ordinal — `2nd sword` picks the second name match
//!
//! Names are matched case-insensitively; `;`-segmented names (`"north;n"`)
//! match on every segment plus the alias list.

use crate::obj::{Obj, ObjId};
use crate::world::World;

/// Why a match failed.
#[derive(Copy, Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MatchErr {
    /// Nothing here answers to that name.
    #[error("I don't see that here.")]
    NoMatch,
    /// Several things answer to that name; use an ordinal (`2nd sword`).
    #[error("I don't know which one you mean!")]
    Ambiguous,
}

/// Resolve `raw` to an object from `looker`'s point of view. See the module
/// docs for the precedence order.
pub fn match_object(w: &World, looker: ObjId, raw: &str) -> Result<ObjId, MatchErr> {
    let q = raw.trim();
    if q.is_empty() {
        return Err(MatchErr::NoMatch);
    }
    if q.eq_ignore_ascii_case("me") {
        return if w.get(looker).is_some() {
            Ok(looker)
        } else {
            Err(MatchErr::NoMatch)
        };
    }
    if q.eq_ignore_ascii_case("here") {
        return w
            .get(looker)
            .and_then(|o| o.location)
            .ok_or(MatchErr::NoMatch);
    }
    if let Some(num) = q.strip_prefix('#') {
        return num
            .parse::<u32>()
            .ok()
            .map(ObjId)
            .filter(|&id| w.get(id).is_some())
            .ok_or(MatchErr::NoMatch);
    }
    if let Some(name) = q.strip_prefix('*') {
        return w.resolve_player(name.trim()).ok_or(MatchErr::NoMatch);
    }

    let loc = w.get(looker).and_then(|o| o.location);
    let inv = pool(w, w.contents_of(looker));
    // The looker is part of the location contents on purpose: matching
    // your own name is classic behavior ("@set Bob=..." typed by Bob).
    let here = loc.map_or_else(Vec::new, |l| pool(w, w.contents_of(l)));
    let exits = loc.map_or_else(Vec::new, |l| pool(w, w.exits_of(l)));

    // 5/6: exact in inventory, then location.
    if let Some(r) = pick_exact(w, &inv, q) {
        return r;
    }
    if let Some(r) = pick_exact(w, &here, q) {
        return r;
    }
    // 7: exits (first exact winner; parallel exits are legitimate).
    if let Some(&e) = exits
        .iter()
        .find(|&&e| w.get(e).is_some_and(|o| name_matches_exact(o, q)))
    {
        return Ok(e);
    }
    // 8: unambiguous prefix over everything visible.
    let all: Vec<ObjId> = inv
        .iter()
        .chain(here.iter())
        .chain(exits.iter())
        .copied()
        .collect();
    let prefix: Vec<ObjId> = all
        .iter()
        .copied()
        .filter(|&id| w.get(id).is_some_and(|o| name_matches_prefix(o, q)))
        .collect();
    match prefix.len() {
        1 => return Ok(prefix[0]),
        n if n > 1 => return Err(MatchErr::Ambiguous),
        _ => {}
    }
    // 9: english ordinal ("2nd sword").
    if let Some((n, rest)) = parse_ordinal(q) {
        let mut hits: Vec<ObjId> = all
            .iter()
            .copied()
            .filter(|&id| w.get(id).is_some_and(|o| name_matches_exact(o, rest)))
            .collect();
        for id in &all {
            if w.get(*id).is_some_and(|o| name_matches_prefix(o, rest)) && !hits.contains(id) {
                hits.push(*id);
            }
        }
        if let Some(&id) = hits.get(n - 1) {
            return Ok(id);
        }
    }
    Err(MatchErr::NoMatch)
}

/// Filter a candidate list: drop `GOING` husks.
fn pool(w: &World, ids: &[ObjId]) -> Vec<ObjId> {
    ids.iter()
        .copied()
        .filter(|&id| {
            w.get(id)
                .is_some_and(|o| !o.flags.contains(crate::flags::Flags::GOING))
        })
        .collect()
}

/// Exact matches within one pool: one winner passes, several are ambiguous.
fn pick_exact(w: &World, ids: &[ObjId], q: &str) -> Option<Result<ObjId, MatchErr>> {
    let hits: Vec<ObjId> = ids
        .iter()
        .copied()
        .filter(|&id| w.get(id).is_some_and(|o| name_matches_exact(o, q)))
        .collect();
    match hits.len() {
        0 => None,
        1 => Some(Ok(hits[0])),
        _ => Some(Err(MatchErr::Ambiguous)),
    }
}

/// Whether `q` exactly matches any `;`-segment of the name or any alias
/// (case-insensitive).
fn name_matches_exact(o: &Obj, q: &str) -> bool {
    o.name
        .split(';')
        .map(str::trim)
        .any(|seg| seg.eq_ignore_ascii_case(q))
        || o.aliases.iter().any(|a| a.eq_ignore_ascii_case(q))
}

/// Whether `q` is a (case-insensitive, non-empty) prefix of any name
/// segment or alias.
fn name_matches_prefix(o: &Obj, q: &str) -> bool {
    if q.is_empty() {
        return false;
    }
    let ql = q.to_lowercase();
    o.name
        .split(';')
        .map(str::trim)
        .any(|seg| seg.to_lowercase().starts_with(&ql))
        || o.aliases.iter().any(|a| a.to_lowercase().starts_with(&ql))
}

/// Parse a leading english ordinal: `"2nd sword"` → `(2, "sword")`.
/// Only numeric ordinals (`1st`, `2nd`, `3rd`, `4th`, …) are recognized.
fn parse_ordinal(q: &str) -> Option<(usize, &str)> {
    let (first, rest) = q.split_once(char::is_whitespace)?;
    let digits_end = first.find(|c: char| !c.is_ascii_digit())?;
    if digits_end == 0 {
        return None;
    }
    let (digits, suffix) = first.split_at(digits_end);
    if !matches!(
        suffix.to_ascii_lowercase().as_str(),
        "st" | "nd" | "rd" | "th"
    ) {
        return None;
    }
    let n: usize = digits.parse().ok()?;
    if n == 0 {
        return None;
    }
    let rest = rest.trim_start();
    if rest.is_empty() {
        return None;
    }
    Some((n, rest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flags::Flags;
    use crate::obj::{GOD, ObjKind};

    /// Nick in the Hall carrying a sword; the hall holds Bob, two more
    /// swords and a lantern, with exits `north;n` and `south;s`.
    #[allow(clippy::type_complexity)]
    fn build() -> (
        World,
        ObjId, // nick
        ObjId, // hall
        ObjId, // bob
        ObjId, // my_sword (carried)
        ObjId, // sword1 (room)
        ObjId, // sword2 (room)
        ObjId, // lantern (room)
        ObjId, // north exit
    ) {
        let mut w = World::new_minimal();
        let hall = w.create(ObjKind::Room, "Hall", GOD, None).unwrap();
        let nick = w.create(ObjKind::Player, "Nick", GOD, Some(hall)).unwrap();
        let bob = w.create(ObjKind::Player, "Bob", GOD, Some(hall)).unwrap();
        let my_sword = w.create(ObjKind::Thing, "sword", GOD, Some(nick)).unwrap();
        let sword1 = w.create(ObjKind::Thing, "sword", GOD, Some(hall)).unwrap();
        let sword2 = w.create(ObjKind::Thing, "sword", GOD, Some(hall)).unwrap();
        let lantern = w
            .create(ObjKind::Thing, "lantern", GOD, Some(hall))
            .unwrap();
        let north = w.create(ObjKind::Exit, "north;n", GOD, Some(hall)).unwrap();
        let _south = w.create(ObjKind::Exit, "south;s", GOD, Some(hall)).unwrap();
        (w, nick, hall, bob, my_sword, sword1, sword2, lantern, north)
    }

    #[test]
    fn keywords_and_refs() {
        let (w, nick, hall, bob, ..) = build();
        assert_eq!(match_object(&w, nick, "me"), Ok(nick));
        assert_eq!(match_object(&w, nick, "ME"), Ok(nick));
        assert_eq!(match_object(&w, nick, "here"), Ok(hall));
        assert_eq!(match_object(&w, nick, &hall.to_string()), Ok(hall));
        assert_eq!(match_object(&w, nick, "#9999"), Err(MatchErr::NoMatch));
        assert_eq!(match_object(&w, nick, "#12x"), Err(MatchErr::NoMatch));
        // *player is global: works even from another room.
        assert_eq!(match_object(&w, GOD, "*bob"), Ok(bob));
        assert_eq!(match_object(&w, nick, "*nobody"), Err(MatchErr::NoMatch));
        assert_eq!(match_object(&w, nick, ""), Err(MatchErr::NoMatch));
        assert_eq!(match_object(&w, nick, "   "), Err(MatchErr::NoMatch));
    }

    #[test]
    fn inventory_beats_location() {
        let (w, nick, _hall, _bob, my_sword, ..) = build();
        // Exact "sword": one in inventory wins over two in the room.
        assert_eq!(match_object(&w, nick, "sword"), Ok(my_sword));
        assert_eq!(match_object(&w, nick, "SWORD"), Ok(my_sword));
    }

    #[test]
    fn location_exact_and_players_by_name() {
        let (w, nick, _hall, bob, _my_sword, _s1, _s2, lantern, _n) = build();
        assert_eq!(match_object(&w, nick, "lantern"), Ok(lantern));
        // Bob stands in the room: bare-name exact match works.
        assert_eq!(match_object(&w, nick, "bob"), Ok(bob));
        // Matching your own name works too (classic).
        assert_eq!(match_object(&w, nick, "nick"), Ok(nick));
        // Two swords in the room, none carried by Bob: ambiguous for Bob.
        assert_eq!(match_object(&w, bob, "sword"), Err(MatchErr::Ambiguous));
    }

    #[test]
    fn exits_match_segments_and_aliases() {
        let (w, nick, _hall, _bob, _ms, _s1, _s2, _l, north) = build();
        assert_eq!(match_object(&w, nick, "north"), Ok(north));
        assert_eq!(match_object(&w, nick, "n"), Ok(north));
        assert_eq!(match_object(&w, nick, "N"), Ok(north));
    }

    #[test]
    fn prefix_matching() {
        let (w, nick, _hall, _bob, _ms, _s1, _s2, lantern, north) = build();
        assert_eq!(match_object(&w, nick, "lan"), Ok(lantern));
        assert_eq!(match_object(&w, nick, "nor"), Ok(north));
        // "s" is an exact alias segment of south;s — exact beats prefix.
        let south = w.exits_of(_hall)[1];
        assert_eq!(match_object(&w, nick, "s"), Ok(south));
        // "sw" prefixes all three swords: ambiguous.
        assert_eq!(match_object(&w, nick, "sw"), Err(MatchErr::Ambiguous));
        assert_eq!(match_object(&w, nick, "xyzzy"), Err(MatchErr::NoMatch));
    }

    #[test]
    fn english_ordinals() {
        let (w, nick, _hall, bob, my_sword, sword1, sword2, ..) = build();
        // From Nick: pool order is inventory first, then room.
        assert_eq!(match_object(&w, nick, "1st sword"), Ok(my_sword));
        assert_eq!(match_object(&w, nick, "2nd sword"), Ok(sword1));
        assert_eq!(match_object(&w, nick, "3rd sword"), Ok(sword2));
        assert_eq!(match_object(&w, nick, "4th sword"), Err(MatchErr::NoMatch));
        // From Bob (nothing carried, Nick's sword invisible): room order.
        assert_eq!(match_object(&w, bob, "1st sword"), Ok(sword1));
        assert_eq!(match_object(&w, bob, "2nd sword"), Ok(sword2));
        // 0th is not an ordinal.
        assert_eq!(match_object(&w, nick, "0th sword"), Err(MatchErr::NoMatch));
    }

    #[test]
    fn going_objects_never_match() {
        let (mut w, nick, _hall, _bob, my_sword, ..) = build();
        w.destroy(my_sword).unwrap();
        // The carried sword is GOING; the room swords are still ambiguous.
        assert_eq!(match_object(&w, nick, "sword"), Err(MatchErr::Ambiguous));
        // GOING flag also hides from prefix pool.
        let (mut w2, nick2, _h, _b, _ms, s1, s2, lantern, _n) = build();
        w2.destroy(lantern).unwrap();
        assert_eq!(match_object(&w2, nick2, "lan"), Err(MatchErr::NoMatch));
        let _ = (s1, s2);
    }

    #[test]
    fn going_flag_does_not_hide_dbref() {
        // #dbref still resolves a GOING husk (needed by @destroy tooling).
        let (mut w, nick, _hall, _bob, my_sword, ..) = build();
        w.destroy(my_sword).unwrap();
        assert_eq!(match_object(&w, nick, &my_sword.to_string()), Ok(my_sword));
        let _ = Flags::GOING;
    }

    #[test]
    fn ordinal_parser() {
        assert_eq!(parse_ordinal("2nd sword"), Some((2, "sword")));
        assert_eq!(parse_ordinal("11th red box"), Some((11, "red box")));
        assert_eq!(parse_ordinal("1ST sword"), Some((1, "sword")));
        assert_eq!(parse_ordinal("2xd sword"), None);
        assert_eq!(parse_ordinal("nd sword"), None);
        assert_eq!(parse_ordinal("2nd"), None);
        assert_eq!(parse_ordinal("0th sword"), None);
        assert_eq!(parse_ordinal("sword"), None);
    }
}
