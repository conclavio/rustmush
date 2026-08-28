//! The oWoD Health track: damage types and the marking / upgrade / rollover
//! rules from the crib.
//!
//! The track is 7 slots, each empty or one mark of a [`Damage`] type. The
//! invariant kept after every mutation: marks are sorted by **descending
//! severity** (aggravated, then lethal, then bashing) with empty slots last —
//! exactly how an oWoD sheet fills top-down. Consequences:
//!
//! - Applying damage drops a mark into the first empty slot, then re-sorts
//!   (bashing "fills from the top" among same-severity marks).
//! - When the track is full, further damage **upgrades** the least-severe
//!   existing mark by one severity step (bashing → lethal → aggravated) —
//!   so overflow bashing rolls over into lethal, per the crib.
//! - The wound penalty is the framework's deepest-filled-level lookup; genuine
//!   **incapacitation** is a fully-filled track.

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

/// A damage type, ordered by severity.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Damage {
    /// Bashing (fists, clubs): heals fastest, upgrades on overflow.
    Bashing,
    /// Lethal (blades, bullets).
    Lethal,
    /// Aggravated (fire, claws, supernatural).
    Aggravated,
}

impl Damage {
    /// The mark string stored in a track slot.
    #[must_use]
    pub fn mark(self) -> &'static str {
        match self {
            Damage::Bashing => "bashing",
            Damage::Lethal => "lethal",
            Damage::Aggravated => "aggravated",
        }
    }

    /// The one-letter code shown on a track (`B`/`L`/`A`).
    #[must_use]
    pub fn letter(self) -> char {
        match self {
            Damage::Bashing => 'B',
            Damage::Lethal => 'L',
            Damage::Aggravated => 'A',
        }
    }

    /// Severity rank (higher = worse), used for sorting.
    #[must_use]
    pub fn severity(self) -> u8 {
        match self {
            Damage::Bashing => 1,
            Damage::Lethal => 2,
            Damage::Aggravated => 3,
        }
    }

    /// The next-worse type on overflow (`Aggravated` stays put).
    #[must_use]
    pub fn upgraded(self) -> Damage {
        match self {
            Damage::Bashing => Damage::Lethal,
            Damage::Lethal | Damage::Aggravated => Damage::Aggravated,
        }
    }

    /// Parse a mark string / single-letter / full word (case-insensitive).
    #[must_use]
    pub fn parse(s: &str) -> Option<Damage> {
        match s.trim().to_ascii_lowercase().as_str() {
            "b" | "bashing" => Some(Damage::Bashing),
            "l" | "lethal" => Some(Damage::Lethal),
            "a" | "agg" | "aggravated" => Some(Damage::Aggravated),
            _ => None,
        }
    }
}

/// A parsed damage/heal amount, e.g. `3L`, `2 bashing`, `agg`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct DamageSpec {
    /// How many marks.
    pub count: u32,
    /// The damage type.
    pub kind: Damage,
}

/// Parse `<count><type>` / `<count> <type>` / `<type>` (count defaults to 1),
/// e.g. `"3L"`, `"2 lethal"`, `"aggravated"`.
#[must_use]
pub fn parse_damage_spec(raw: &str) -> Option<DamageSpec> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    // Split leading digits from a trailing type token.
    let digits: String = raw.chars().take_while(char::is_ascii_digit).collect();
    let rest = raw[digits.len()..].trim();
    let count = if digits.is_empty() {
        1
    } else {
        digits.parse::<u32>().ok()?
    };
    let kind = Damage::parse(rest)?;
    Some(DamageSpec { count, kind })
}

/// A 7-slot Health track, in-memory. Slot `None` is empty; `Some(_)` is a mark.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HealthTrack {
    slots: Vec<Option<Damage>>,
}

impl HealthTrack {
    /// Build from stored mark strings (`None` empty, unknown strings dropped).
    #[must_use]
    pub fn from_marks(marks: &[Option<SmolStr>]) -> HealthTrack {
        let slots = marks
            .iter()
            .map(|m| m.as_deref().and_then(Damage::parse))
            .collect();
        let mut t = HealthTrack { slots };
        t.sort();
        t
    }

    /// The number of slots.
    #[must_use]
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// Whether the track has no slots.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// The number of filled slots.
    #[must_use]
    pub fn filled(&self) -> usize {
        self.slots.iter().filter(|s| s.is_some()).count()
    }

    /// Whether every slot is filled (the incapacitation condition).
    #[must_use]
    pub fn is_incapacitated(&self) -> bool {
        !self.is_empty() && self.filled() == self.len()
    }

    /// Whether every slot holds aggravated damage (the death condition on
    /// further overflow).
    #[must_use]
    pub fn all_aggravated(&self) -> bool {
        !self.is_empty() && self.slots.iter().all(|s| *s == Some(Damage::Aggravated))
    }

    /// Marks as storage strings (`None` empty).
    #[must_use]
    pub fn to_marks(&self) -> Vec<Option<SmolStr>> {
        self.slots
            .iter()
            .map(|s| s.map(|d| SmolStr::new(d.mark())))
            .collect()
    }

    /// Keep marks sorted by descending severity, empties last.
    fn sort(&mut self) {
        self.slots
            .sort_by_key(|s| std::cmp::Reverse(severity_key(s)));
    }

    /// Apply `count` points of `kind`. Empty slots fill first; a full track
    /// upgrades the least-severe mark one step per overflow point (bashing
    /// rolls over into lethal). No-op on a zero-length track.
    pub fn apply(&mut self, kind: Damage, count: u32) {
        if self.is_empty() {
            return;
        }
        // Every point beyond 3× the track length is a guaranteed no-op: at
        // most `len` points fill the track, then two upgrade passes
        // (bashing→lethal→aggravated) exhaust it to all-aggravated, after
        // which `upgrade_least_severe` changes nothing. Clamp so a bogus huge
        // `count` (e.g. `+harm Bob=4000000000A`) can't spin billions of
        // no-op iterations.
        let count = count.min(self.slots.len() as u32 * 3);
        for _ in 0..count {
            if let Some(idx) = self.slots.iter().position(Option::is_none) {
                self.slots[idx] = Some(kind);
            } else {
                self.upgrade_least_severe();
            }
            self.sort();
        }
    }

    /// Upgrade the least-severe mark by one severity step; if all are
    /// aggravated, nothing changes (overkill).
    fn upgrade_least_severe(&mut self) {
        let least = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.map(|d| (i, d)))
            .min_by_key(|(_, d)| d.severity());
        if let Some((i, d)) = least
            && d != Damage::Aggravated
        {
            self.slots[i] = Some(d.upgraded());
        }
    }

    /// Heal up to `count` marks of `kind`, least-severe positions first.
    /// Returns how many were actually removed.
    pub fn heal(&mut self, kind: Damage, count: u32) -> u32 {
        let mut removed = 0;
        for _ in 0..count {
            // Rightmost matching mark (least severe / most recently filled).
            let pos = self.slots.iter().rposition(|s| *s == Some(kind));
            match pos {
                Some(i) => {
                    self.slots[i] = None;
                    removed += 1;
                }
                None => break,
            }
        }
        self.sort();
        removed
    }
}

/// Sort key: `(severity, filled)` so filled marks come before empties and more
/// severe marks come first.
fn severity_key(slot: &Option<Damage>) -> (u8, u8) {
    match slot {
        Some(d) => (d.severity(), 1),
        None => (0, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty() -> HealthTrack {
        HealthTrack::from_marks(&vec![None; 7])
    }

    #[test]
    fn parse_specs() {
        assert_eq!(
            parse_damage_spec("3L"),
            Some(DamageSpec {
                count: 3,
                kind: Damage::Lethal
            })
        );
        assert_eq!(
            parse_damage_spec("2 bashing"),
            Some(DamageSpec {
                count: 2,
                kind: Damage::Bashing
            })
        );
        assert_eq!(
            parse_damage_spec("aggravated"),
            Some(DamageSpec {
                count: 1,
                kind: Damage::Aggravated
            })
        );
        assert_eq!(parse_damage_spec("nonsense"), None);
    }

    #[test]
    fn bashing_fills_then_rolls_over_to_lethal() {
        let mut t = empty();
        t.apply(Damage::Bashing, 7);
        assert!(t.is_incapacitated());
        assert!(t.to_marks().iter().all(|m| m.as_deref() == Some("bashing")));
        // One more bashing: track full → least-severe bashing upgrades to lethal.
        t.apply(Damage::Bashing, 1);
        let marks = t.to_marks();
        // Sorted desc: one lethal leads, six bashing follow.
        assert_eq!(marks[0].as_deref(), Some("lethal"));
        assert_eq!(
            marks
                .iter()
                .filter(|m| m.as_deref() == Some("lethal"))
                .count(),
            1
        );
        assert_eq!(
            marks
                .iter()
                .filter(|m| m.as_deref() == Some("bashing"))
                .count(),
            6
        );
    }

    #[test]
    fn lethal_sorts_above_bashing() {
        let mut t = empty();
        t.apply(Damage::Bashing, 2);
        t.apply(Damage::Lethal, 1);
        let marks = t.to_marks();
        assert_eq!(marks[0].as_deref(), Some("lethal"));
        assert_eq!(marks[1].as_deref(), Some("bashing"));
        assert_eq!(marks[2].as_deref(), Some("bashing"));
        assert_eq!(marks[3], None);
    }

    #[test]
    fn heal_removes_least_severe_first() {
        let mut t = empty();
        t.apply(Damage::Lethal, 2);
        t.apply(Damage::Bashing, 2);
        assert_eq!(t.filled(), 4);
        // Heal 2 bashing → only lethal remains.
        assert_eq!(t.heal(Damage::Bashing, 2), 2);
        let marks = t.to_marks();
        assert_eq!(marks.iter().filter(|m| m.is_some()).count(), 2);
        assert!(
            marks
                .iter()
                .filter(|m| m.is_some())
                .all(|m| m.as_deref() == Some("lethal"))
        );
        // Healing bashing when none present removes nothing.
        assert_eq!(t.heal(Damage::Bashing, 1), 0);
    }

    #[test]
    fn apply_clamps_absurd_count() {
        // A bogus huge count must return promptly (the loop is bounded to
        // 3× the track length) and land on the correct saturated state.
        let mut t = empty();
        t.apply(Damage::Aggravated, u32::MAX);
        assert!(t.all_aggravated());
        // Huge bashing overflows through lethal all the way to aggravated.
        let mut t2 = empty();
        t2.apply(Damage::Bashing, u32::MAX);
        assert!(t2.all_aggravated());
    }

    #[test]
    fn overflow_upgrades_toward_aggravated() {
        let mut t = empty();
        t.apply(Damage::Lethal, 7);
        assert!(t.is_incapacitated());
        // Full lethal + one lethal overflow → least-severe lethal upgrades to agg.
        t.apply(Damage::Lethal, 1);
        assert_eq!(t.to_marks()[0].as_deref(), Some("aggravated"));
        // Fill entirely with aggravated: further overflow is a no-op.
        t.apply(Damage::Aggravated, 7);
        assert!(t.all_aggravated());
        t.apply(Damage::Aggravated, 1);
        assert!(t.all_aggravated());
    }
}
