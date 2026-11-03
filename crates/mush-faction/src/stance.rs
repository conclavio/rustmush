//! **The stance table** (doc 40 cut FR.1; Q-38-23, Q-40-11..14): the relationship matrix
//! [`crate::data`]'s head reserved, between organizations — a faction or a mush-org — one row a
//! direction.
//!
//! | key                    | document                                     |
//! |------------------------|----------------------------------------------|
//! | `stance:{from}:{to}`   | one [`StanceRow`] (`{side}` is `f<id>` or `o<id>`) |
//!
//! - **Allied, Neutral, Hostile** (Q-38-23). Neutral is the absence of a row: declaring it deletes
//!   the row. A row is written whole, write-through like every faction mutation, with no receipt.
//! - **Hostile is directional** (Q-40-12): the declaring side's parties open a field with a
//!   Hostile mark on the other's. **Allied counts only when both declare it**: a mutual pair is
//!   a friendship; a one-sided Allied has no field effect and `+faction` shows it
//!   "unreciprocated".
//! - **Declared by** (Q-40-11): for a faction, one of its principals or a member at or above
//!   `officer_level`; for a mush-org, a member holding [`mush_org::STANCE_DECLARE_CAPABILITY`] on
//!   its root scope; staff always. A declaration is a notice to the other side's officers.
//! - **Read at admission and at a join, never mid-field** (Q-38-23): the field host reads
//!   [`FactionState::stance`] as a field is built; a change while a field runs changes nothing in
//!   it. A stance never opens a field (Q-40-13).

use mush_core::ObjId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::data::FactionId;

/// **One side of a stance row**: an organization a field's party can stand in.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StanceSide {
    /// A mush-org organization (a co-op, a house).
    Org(mush_org::OrgId),
    /// A faction.
    Faction(FactionId),
}

impl StanceSide {
    /// Its word in a stance key (`f<id>` or `o<id>`).
    #[must_use]
    pub fn key_word(self) -> String {
        match self {
            StanceSide::Org(org) => format!("o{}", org.0),
            StanceSide::Faction(faction) => format!("f{}", faction.0),
        }
    }

    /// The side a key word names.
    #[must_use]
    pub fn from_key_word(word: &str) -> Option<StanceSide> {
        let (kind, id) = word.split_at_checked(1)?;
        let id = id.parse::<u64>().ok()?;
        match kind {
            "o" => Some(StanceSide::Org(mush_org::OrgId(id))),
            "f" => Some(StanceSide::Faction(FactionId(id))),
            _ => None,
        }
    }
}

/// **A declared stance** (Q-38-23).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stance {
    /// Allied: a friendship in a field when both sides declare it.
    Allied,
    /// Neutral: no row.
    Neutral,
    /// Hostile: the declaring side's parties open a field with a Hostile mark on the other's.
    Hostile,
}

impl Stance {
    /// The stance a player's word names.
    #[must_use]
    pub fn parse(word: &str) -> Option<Stance> {
        match word.trim().to_ascii_lowercase().as_str() {
            "allied" | "ally" | "allies" => Some(Stance::Allied),
            "neutral" | "none" | "clear" => Some(Stance::Neutral),
            "hostile" | "enemy" => Some(Stance::Hostile),
            _ => None,
        }
    }

    /// Its word in a line.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Stance::Allied => "Allied",
            Stance::Neutral => "Neutral",
            Stance::Hostile => "Hostile",
        }
    }
}

/// **One stance row**, persisted whole under [`stance_key`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StanceRow {
    /// The declaring side.
    pub from: StanceSide,
    /// The side it names.
    pub to: StanceSide,
    /// Allied or Hostile (a Neutral is no row).
    pub stance: Stance,
    /// Who declared it.
    pub declared_by: ObjId,
    /// When (unix seconds).
    pub at: i64,
}

/// The kv key of one stance row.
#[must_use]
pub fn stance_key(from: StanceSide, to: StanceSide) -> SmolStr {
    SmolStr::new(format!("stance:{}:{}", from.key_word(), to.key_word()))
}

/// The `(from, to)` a stance key names.
#[must_use]
pub fn parse_stance_key(key: &str) -> Option<(StanceSide, StanceSide)> {
    let rest = key.strip_prefix("stance:")?;
    let (from, to) = rest.split_once(':')?;
    Some((
        StanceSide::from_key_word(from)?,
        StanceSide::from_key_word(to)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stance_key_round_trips_both_kinds_of_side() {
        let from = StanceSide::Org(mush_org::OrgId(7));
        let to = StanceSide::Faction(FactionId(3));
        let key = stance_key(from, to);
        assert_eq!(key.as_str(), "stance:o7:f3");
        assert_eq!(parse_stance_key(&key), Some((from, to)));
        assert_eq!(parse_stance_key("stance:x7:f3"), None);
        assert_eq!(parse_stance_key("stance:o7"), None);
    }

    #[test]
    fn a_row_reads_back_and_refuses_an_unknown_field() {
        let row = StanceRow {
            from: StanceSide::Org(mush_org::OrgId(1)),
            to: StanceSide::Org(mush_org::OrgId(2)),
            stance: Stance::Hostile,
            declared_by: ObjId(9),
            at: 5,
        };
        let value = serde_json::to_value(&row).expect("plain data");
        assert_eq!(
            value,
            serde_json::json!({
                "from": { "org": 1 }, "to": { "org": 2 }, "stance": "hostile",
                "declared_by": 9, "at": 5,
            })
        );
        assert_eq!(
            serde_json::from_value::<StanceRow>(value.clone()).expect("reads back"),
            row
        );
        let mut extra = value;
        extra["note"] = serde_json::json!("x");
        assert!(serde_json::from_value::<StanceRow>(extra).is_err());
    }
}
