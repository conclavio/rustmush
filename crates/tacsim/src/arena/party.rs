//! **Parties and relations in the kernel** (doc 32 cut CF.0; Q-CF-3 to Q-CF-11).
//!
//! The kernel had a closed `enum Team { Red, Blue }` and read hostility as
//! `team != mine`. An IC field is several independent parties on one piece of
//! ground (doc 27 Q-H3), so this module replaces both:
//!
//! * [`PartyId`] — a party of a field, allocated per field. It **is** the
//!   party's letter (Q-CF-11): one byte of [`LETTERS`], the alphabet A–Z less
//!   the confusable `I` and `O` and the [`PROVISIONAL_PREFIX`] `X`. The IC host
//!   takes letters in admission order ([`PartyId::nth`]); the OOC host names its
//!   two parties red and blue and keeps `R` and `B`
//!   (`crate::match_`'s `PartyId::RED` and `PartyId::BLUE`).
//! * [`Relation`] — one party's view of another (Q-CF-5): own, friend, hostile,
//!   neutral or, for a contact not yet told apart, unknown.
//! * [`Relations`] — the books (Q-CF-6: hostility is a **belief, held per
//!   party**). Each party's book holds, in precedence order (Q-CF-7): its
//!   leader's explicit, private marks; the hostility fire produced; the
//!   standing friendships roster and organization give (the host supplies
//!   them); and Neutral below all three.
//! * [`FieldHost`] — what the host hands the kernel at check-out beside the
//!   units: the opening marks and friendships and the [`EndCondition`]
//!   (Q-CF-3: the end is the host's, not a hard-coded elimination of a team).
//!
//! The kernel never refuses a shot by relation (Q-CF-9); relations decide what
//! doctrine targets, what a frame colours, and what a mind reads.
//!
//! [`PROVISIONAL_PREFIX`]: crate::arena::state::PROVISIONAL_PREFIX

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::arena::state::Classification;

/// **The letters a party or a unit may be named by** (Q-CF-11), in admission
/// order: A–Z less `I` and `O` (confusable with `1` and `0`) and `X` (the
/// provisional designator's prefix). Twenty-three of them, so a field holds at
/// most twenty-three parties and a party at most twenty-three units.
pub const LETTERS: [u8; 23] = *b"ABCDEFGHJKLMNPQRSTUVWYZ";

/// **A party of a field** (doc 32 Q-CF-2, Q-CF-4): one detachment in IC, one
/// side of a match in OOC. The value is the party's letter, one byte of
/// [`LETTERS`]; [`PartyId::from_letter`] refuses any other.
///
/// It serializes as its wire word ([`PartyId::as_str`]): `red` and `blue` for
/// the OOC host's two letters `R` and `B`, so every OOC wire form a pilot, the
/// bot or a replay reads is unchanged, and the lowercase letter for every
/// other party.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PartyId(u8);

/// The wire word of every letter of [`LETTERS`], by index.
const WORDS: [&str; 23] = [
    "a", "b", "c", "d", "e", "f", "g", "h", "j", "k", "l", "m", "n", "p", "q", "r", "s", "t", "u",
    "v", "w", "y", "z",
];

/// The index of `letter` in [`LETTERS`].
const fn letter_index(letter: u8) -> Option<usize> {
    let mut at = 0;
    while at < LETTERS.len() {
        if LETTERS[at] == letter {
            return Some(at);
        }
        at += 1;
    }
    None
}

impl PartyId {
    /// The party named by `letter`, if it is one of [`LETTERS`] (upper case).
    #[must_use]
    pub const fn from_letter(letter: char) -> Option<PartyId> {
        if !letter.is_ascii_uppercase() {
            return None;
        }
        let byte = letter as u8;
        match letter_index(byte) {
            Some(_) => Some(PartyId(byte)),
            None => None,
        }
    }

    /// **The `n`-th party admitted to a field** (Q-CF-11: the party letter
    /// comes from the alphabet in admission order), from 0, or `None` past the
    /// twenty-third.
    #[must_use]
    pub const fn nth(n: usize) -> Option<PartyId> {
        if n < LETTERS.len() {
            Some(PartyId(LETTERS[n]))
        } else {
            None
        }
    }

    /// **A party a host names by a letter it knows is in [`LETTERS`]** — the
    /// OOC host's `R` and `B` (`crate::match_`). Crate-private: every other
    /// caller goes through [`PartyId::from_letter`] or [`PartyId::nth`].
    pub(crate) const fn known(letter: u8) -> PartyId {
        PartyId(letter)
    }

    /// The party's letter: the first of its units' two-letter designators.
    #[must_use]
    pub const fn letter(&self) -> char {
        self.0 as char
    }

    /// The byte the state hash folds: the letter.
    #[must_use]
    pub const fn code(&self) -> u8 {
        self.0
    }

    /// **The wire word** — `red` for `R` and `blue` for `B` (the OOC host's
    /// names, kept so its wire does not move), else the lowercase letter.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self.0 {
            b'R' => "red",
            b'B' => "blue",
            letter => match letter_index(letter) {
                Some(at) => WORDS[at],
                None => "?",
            },
        }
    }

    /// The exact inverse of [`PartyId::as_str`].
    #[must_use]
    pub fn parse(word: &str) -> Option<PartyId> {
        match word {
            "red" => return Some(PartyId(b'R')),
            "blue" => return Some(PartyId(b'B')),
            "r" | "b" => return None,
            _ => {}
        }
        let at = WORDS.iter().position(|w| *w == word)?;
        LETTERS.get(at).map(|letter| PartyId(*letter))
    }
}

impl std::fmt::Display for PartyId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for PartyId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for PartyId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<PartyId, D::Error> {
        let word = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        PartyId::parse(&word)
            .ok_or_else(|| serde::de::Error::custom(format!("no party is called {word:?}")))
    }
}

/// **One party's view of another** (Q-CF-5), or of a contact it has not told
/// apart yet ([`Relation::Unknown`], Q-CF-10).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    /// The viewer's own party.
    Own,
    /// A friend: by a Friendly mark, or by roster or organization.
    Friend,
    /// A hostile: by a Hostile mark, or because it fired on the viewer or on a
    /// friend of the viewer's in this field.
    Hostile,
    /// A party told apart that is none of the above.
    Neutral,
    /// A contact not yet classified: the viewer does not know whose it is.
    Unknown,
}

impl Relation {
    /// The wire word: `own`, `friend`, `hostile`, `neutral`, `unknown`.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Relation::Own => "own",
            Relation::Friend => "friend",
            Relation::Hostile => "hostile",
            Relation::Neutral => "neutral",
            Relation::Unknown => "unknown",
        }
    }

    /// Own or friend: what a frame, a notice and a mind call *friendly*.
    #[must_use]
    pub const fn is_friendly(&self) -> bool {
        matches!(self, Relation::Own | Relation::Friend)
    }
}

/// **A leader's explicit mark** (Q-CF-7): the top of a party's book, private
/// to it — the marked party is never told.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mark {
    /// Friendly: also how a leader cancels the hostility friendly fire produced.
    Friendly,
    /// Hostile.
    Hostile,
}

/// **One party's book** (Q-CF-6, Q-CF-7): what it believes of every other.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Book {
    /// The leader's marks, by marked party.
    pub marks: BTreeMap<PartyId, Mark>,
    /// The parties this one holds hostile because it saw them fire on it or on
    /// a friend of its own.
    pub fired: BTreeSet<PartyId>,
    /// The parties the host says are friends by roster or organization.
    pub friends: BTreeSet<PartyId>,
}

/// **Every party's book in one field** (Q-CF-5 to Q-CF-8), carried by the
/// snapshot and folded into the state hash.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Relations {
    books: BTreeMap<PartyId, Book>,
}

impl Relations {
    /// No party at all.
    #[must_use]
    pub fn new() -> Relations {
        Relations::default()
    }

    /// Admit `party` with an empty book, if it has none.
    pub fn admit(&mut self, party: PartyId) {
        self.books.entry(party).or_default();
    }

    /// Every admitted party, in letter order.
    pub fn parties(&self) -> impl Iterator<Item = PartyId> + '_ {
        self.books.keys().copied()
    }

    /// Whether `party` is admitted.
    #[must_use]
    pub fn contains(&self, party: PartyId) -> bool {
        self.books.contains_key(&party)
    }

    /// `party`'s book, if it is admitted.
    #[must_use]
    pub fn book(&self, party: PartyId) -> Option<&Book> {
        self.books.get(&party)
    }

    /// **What `viewer` believes `other` is** (Q-CF-7's precedence): its own
    /// party; else its leader's mark; else hostility fire produced; else a
    /// standing friend; else neutral.
    #[must_use]
    pub fn relation(&self, viewer: PartyId, other: PartyId) -> Relation {
        if viewer == other {
            return Relation::Own;
        }
        let Some(book) = self.books.get(&viewer) else {
            return Relation::Neutral;
        };
        match book.marks.get(&other) {
            Some(Mark::Friendly) => Relation::Friend,
            Some(Mark::Hostile) => Relation::Hostile,
            None if book.fired.contains(&other) => Relation::Hostile,
            None if book.friends.contains(&other) => Relation::Friend,
            None => Relation::Neutral,
        }
    }

    /// **What `viewer` makes of a contact held at `class`**: unknown until it
    /// is classified (Q-CF-10), then the party relation.
    #[must_use]
    pub fn of_class(&self, viewer: PartyId, class: Classification) -> Relation {
        match class.party() {
            Some(party) => self.relation(viewer, party),
            None => Relation::Unknown,
        }
    }

    /// Whether `viewer` holds a contact at `class` hostile.
    #[must_use]
    pub fn hostile(&self, viewer: PartyId, class: Classification) -> bool {
        self.of_class(viewer, class) == Relation::Hostile
    }

    /// **A leader's mark** (Q-CF-7): `by` marks `of` Friendly or Hostile, or
    /// clears its mark with `None`. Private: only `by`'s book changes. A mark
    /// of one's own party is ignored.
    pub fn mark(&mut self, by: PartyId, of: PartyId, mark: Option<Mark>) {
        if by == of {
            return;
        }
        let book = self.books.entry(by).or_default();
        match mark {
            Some(mark) => {
                book.marks.insert(of, mark);
            }
            None => {
                book.marks.remove(&of);
            }
        }
    }

    /// `by`'s mark of `of`, if its leader has made one.
    #[must_use]
    pub fn mark_of(&self, by: PartyId, of: PartyId) -> Option<Mark> {
        self.books.get(&by)?.marks.get(&of).copied()
    }

    /// **A standing friendship by roster or organization** (Q-CF-5), both
    /// ways: the host's to give.
    pub fn befriend(&mut self, a: PartyId, b: PartyId) {
        if a == b {
            return;
        }
        self.books.entry(a).or_default().friends.insert(b);
        self.books.entry(b).or_default().friends.insert(a);
    }

    /// **`shooter` fired on `victim`** (Q-CF-5, Q-CF-9): the victim's book and
    /// the book of every party that holds the victim a friend now hold the
    /// shooter hostile — under any mark, which still decides. The shooter's
    /// own friends are not moved. Fire within a party moves nothing.
    ///
    /// The kernel reads "observed" as the victim's knowing it was fired on and
    /// its friends' sharing that, which is the first version's reading.
    /// Answers whether any book changed.
    pub fn note_fire(&mut self, shooter: PartyId, victim: PartyId) -> bool {
        if shooter == victim {
            return false;
        }
        let told: Vec<PartyId> = self
            .books
            .keys()
            .copied()
            .filter(|party| {
                *party != shooter
                    && (*party == victim || self.relation(*party, victim) == Relation::Friend)
            })
            .collect();
        let mut changed = false;
        for party in told {
            changed |= self.books.entry(party).or_default().fired.insert(shooter);
        }
        changed
    }
}

/// **How a field ends**, as its host supplies it (Q-CF-3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndCondition {
    /// The OOC host's rule (doc 16 §3.8): the field ends when at most one
    /// party stands, and as `spent` when nobody can fire. Every match.
    #[default]
    Elimination,
    /// The host ends the field itself (the IC host's demotion, Q-CF-20): the
    /// kernel never ends it on a standing count.
    Host,
}

impl EndCondition {
    /// The byte the state hash folds.
    #[must_use]
    pub const fn code(&self) -> u8 {
        match self {
            EndCondition::Elimination => 0,
            EndCondition::Host => 1,
        }
    }
}

/// **What the host hands the kernel at check-out beside the units** (doc 32
/// cut CF.0): how the parties stand toward each other at the opening, and how
/// the field ends. The parties themselves are the units' own.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FieldHost {
    /// Every party opens marking every other Hostile — a practice, where each
    /// side's lead has declared on all the others.
    pub all_hostile: bool,
    /// Opening marks, `(by, of, mark)`, applied after `all_hostile`.
    pub marks: Vec<(PartyId, PartyId, Mark)>,
    /// Standing friendships by roster or organization.
    pub friends: Vec<(PartyId, PartyId)>,
    /// How the field ends.
    pub end: EndCondition,
}

impl FieldHost {
    /// **Every party hostile to every other, ending by elimination**: the
    /// OOC range's host, two parties or three (Q-CF-46).
    #[must_use]
    pub fn all_hostile() -> FieldHost {
        FieldHost {
            all_hostile: true,
            marks: Vec::new(),
            friends: Vec::new(),
            end: EndCondition::Elimination,
        }
    }

    /// The opening books for `parties`.
    #[must_use]
    pub fn open(&self, parties: &BTreeSet<PartyId>) -> Relations {
        let mut relations = Relations::new();
        for party in parties {
            relations.admit(*party);
        }
        if self.all_hostile {
            for by in parties {
                for of in parties {
                    relations.mark(*by, *of, Some(Mark::Hostile));
                }
            }
        }
        for (a, b) in &self.friends {
            relations.befriend(*a, *b);
        }
        for (by, of, mark) in &self.marks {
            relations.mark(*by, *of, Some(*mark));
        }
        relations
    }
}

impl crate::arena::state::Arena {
    /// **A detachment leader's mark** (Q-CF-7): `by`'s book marks `of`
    /// Friendly or Hostile, or `None` clears the mark. Private kernel state: no
    /// ledger line, no notice, nothing in `of`'s book or frame; a Friendly mark
    /// is how a leader cancels the hostility friendly fire produced. The IC
    /// verbs that set it are the host's (doc 32 cut CF.2); the kernel holds it,
    /// folds it into the state hash and honours it from the next frame and the
    /// next think.
    pub fn mark_party(&mut self, by: PartyId, of: PartyId, mark: Option<Mark>) {
        self.relations.mark(by, of, mark);
    }

    /// **What `viewer`'s party believes `other` is** (Q-CF-5 to Q-CF-7).
    #[must_use]
    pub fn relation(&self, viewer: PartyId, other: PartyId) -> Relation {
        self.relations.relation(viewer, other)
    }
}

/// **What a unit is called within its party** (Q-CF-11): one letter of
/// [`LETTERS`], or — past a party's twenty-third unit, which no field inside
/// doc 32's budget reaches — a pair, `first * 23 + second`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitName {
    /// One letter: every unit of a party of at most twenty-three.
    Letter(u8),
    /// A pair, index `0..529`.
    Pair(u16),
}

/// The most units one party may field: twenty-three letters, then the 529
/// pairs.
pub const MAX_PARTY_UNITS: usize = 23 + 23 * 23;

/// **Draw every unit's name** (Q-CF-11): per party, in unit-id order, each
/// unit takes a letter of [`LETTERS`] its party has not given, picked by a
/// keyed mix of the seed, the party and the unit id — random to a viewer,
/// stable for the field, and a function of nothing a viewer could read a slot
/// from. A party's twenty-fourth unit on draws from the pairs the same way.
/// `units` is `(party, unit id)` in unit-id order; answers each unit's name in
/// the same order.
///
/// # Errors
///
/// The first party that fields more than [`MAX_PARTY_UNITS`].
pub fn draw_unit_letters(seed: u64, units: &[(PartyId, u32)]) -> Result<Vec<UnitName>, PartyId> {
    let mut letters: BTreeMap<PartyId, Vec<u8>> = BTreeMap::new();
    let mut pairs: BTreeMap<PartyId, Vec<u16>> = BTreeMap::new();
    let mut out = Vec::with_capacity(units.len());
    for (party, id) in units {
        let key =
            crate::arena::rng::splitmix64(seed ^ (u64::from(party.code()) << 40) ^ u64::from(*id));
        let pick = |len: usize| usize::try_from(key % len.max(1) as u64).unwrap_or(0);
        let left = letters.entry(*party).or_insert_with(|| LETTERS.to_vec());
        if !left.is_empty() {
            let at = pick(left.len()).min(left.len() - 1);
            out.push(UnitName::Letter(left.remove(at)));
            continue;
        }
        let wide = pairs
            .entry(*party)
            .or_insert_with(|| (0..23 * 23).collect());
        if wide.is_empty() {
            return Err(*party);
        }
        let at = pick(wide.len()).min(wide.len() - 1);
        out.push(UnitName::Pair(wide.remove(at)));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_alphabet_has_no_confusable_letter_and_no_provisional_prefix() {
        assert_eq!(LETTERS.len(), 23);
        for banned in [b'I', b'O', crate::arena::state::PROVISIONAL_PREFIX as u8] {
            assert!(!LETTERS.contains(&banned), "{}", banned as char);
        }
        let mut sorted = LETTERS.to_vec();
        sorted.dedup();
        assert_eq!(sorted.len(), 23, "distinct");
    }

    #[test]
    fn a_party_word_round_trips_and_the_ooc_letters_keep_their_names() {
        for n in 0..23 {
            let party = PartyId::nth(n).unwrap();
            assert_eq!(PartyId::parse(party.as_str()), Some(party));
            let text = serde_json::to_string(&party).unwrap();
            assert_eq!(serde_json::from_str::<PartyId>(&text).unwrap(), party);
        }
        assert_eq!(PartyId::from_letter('R').unwrap().as_str(), "red");
        assert_eq!(PartyId::from_letter('B').unwrap().as_str(), "blue");
        assert_eq!(PartyId::from_letter('X'), None);
        assert_eq!(PartyId::from_letter('I'), None);
        assert_eq!(PartyId::nth(23), None);
        assert_eq!(PartyId::parse("r"), None);
    }

    #[test]
    fn precedence_is_mark_then_fire_then_standing_then_neutral() {
        let [a, b, c] = [0, 1, 2].map(|n| PartyId::nth(n).unwrap());
        let mut books = FieldHost::default().open(&BTreeSet::from([a, b, c]));
        assert_eq!(books.relation(a, a), Relation::Own);
        assert_eq!(books.relation(a, b), Relation::Neutral);
        books.befriend(a, b);
        assert_eq!(books.relation(a, b), Relation::Friend);
        assert_eq!(books.relation(b, a), Relation::Friend);
        // Friendly fire makes the shot party hostile to the shooter, in its own
        // book only.
        assert!(books.note_fire(a, b));
        assert_eq!(books.relation(b, a), Relation::Hostile);
        assert_eq!(books.relation(a, b), Relation::Friend);
        // The leader's Friendly mark cancels it.
        books.mark(b, a, Some(Mark::Friendly));
        assert_eq!(books.relation(b, a), Relation::Friend);
        // And a Hostile mark is private to the marker.
        books.mark(c, a, Some(Mark::Hostile));
        assert_eq!(books.relation(c, a), Relation::Hostile);
        assert_eq!(books.relation(a, c), Relation::Neutral);
        books.mark(c, a, None);
        assert_eq!(books.relation(c, a), Relation::Neutral);
    }

    #[test]
    fn a_neutral_fired_on_holds_only_the_shooter_hostile() {
        let [miner, escort, raider, bystander] = [0, 1, 2, 3].map(|n| PartyId::nth(n).unwrap());
        let mut books =
            FieldHost::default().open(&BTreeSet::from([miner, escort, raider, bystander]));
        books.befriend(miner, escort);
        books.mark(raider, escort, Some(Mark::Hostile));
        books.note_fire(raider, bystander);
        assert_eq!(books.relation(bystander, raider), Relation::Hostile);
        assert_eq!(
            books.relation(escort, raider),
            Relation::Neutral,
            "not the bystander's friend"
        );
        // Fire on the miner reaches the miner's friend the escort.
        books.note_fire(raider, miner);
        assert_eq!(books.relation(miner, raider), Relation::Hostile);
        assert_eq!(books.relation(escort, raider), Relation::Hostile);
        assert_eq!(books.relation(bystander, miner), Relation::Neutral);
    }

    #[test]
    fn unit_letters_are_distinct_per_party_stable_and_bounded() {
        let red = PartyId::from_letter('R').unwrap();
        let blue = PartyId::from_letter('B').unwrap();
        let units: Vec<(PartyId, u32)> = (1..=23)
            .map(|id| (red, id))
            .chain((24..=30).map(|id| (blue, id)))
            .collect();
        let names = draw_unit_letters(7, &units).unwrap();
        assert_eq!(names, draw_unit_letters(7, &units).unwrap(), "stable");
        let mut reds: Vec<u8> = names[..23]
            .iter()
            .map(|name| match name {
                UnitName::Letter(letter) => *letter,
                UnitName::Pair(_) => 0,
            })
            .collect();
        reds.sort_unstable();
        assert_eq!(reds, LETTERS.to_vec(), "every letter once");
        assert_ne!(
            names,
            draw_unit_letters(8, &units).unwrap(),
            "keyed on the seed"
        );
        // A party's twenty-fourth unit draws a pair; past the pairs, a refusal.
        let mut wide = units.clone();
        wide.push((red, 99));
        assert!(matches!(
            draw_unit_letters(7, &wide).unwrap()[30],
            UnitName::Pair(_)
        ));
        let full: Vec<(PartyId, u32)> = (0..u32::try_from(MAX_PARTY_UNITS).unwrap() + 1)
            .map(|id| (red, id))
            .collect();
        assert_eq!(draw_unit_letters(7, &full), Err(red));
    }
}
