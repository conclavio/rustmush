//! **A running field's durable half** (doc 32 cut CF.2; Q-CF-33, Q-CF-34): the live record and
//! the snapshot document, on the normal lane beside the arena's own (`tacsim:live:<n>`,
//! `tacsim:snapshot:<n>`), keyed by the site and never by a match number (D-23-10).
//!
//! The site record (`tacsim.field`, `crate::surface::site_record`) says a field is open; this
//! record says who is in it — every admitted unit's party, its pilot and its epoch, and each
//! party's letter, key and leader — and how far the last snapshot reached. A boot resumes from
//! the two together, after the outage countdown (Q-CF-33).

use mush_core::{CanonicalId, Ctx, ObjId};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::arena::PartyId;
use crate::arena::mind::SortieId;
use crate::detachment::sortie::OperationId;
use crate::store::push_op;
use crate::surface::site_record::FieldId;

/// [`FieldLive::format`].
pub const FIELD_LIVE_FORMAT: u16 = 1;
/// The live record's key prefix: `tacsim:field:live:<site>`.
pub const LIVE_PREFIX: &str = "tacsim:field:live:";
/// The snapshot's key prefix: `tacsim:field:snapshot:<site>`.
pub const SNAPSHOT_PREFIX: &str = "tacsim:field:snapshot:";

/// `tacsim:field:live:<site>`.
#[must_use]
pub fn live_key(site: &CanonicalId) -> SmolStr {
    SmolStr::new(format!("{LIVE_PREFIX}{site}"))
}

/// `tacsim:field:snapshot:<site>`.
#[must_use]
pub fn snapshot_key(site: &CanonicalId) -> SmolStr {
    SmolStr::new(format!("{SNAPSHOT_PREFIX}{site}"))
}

/// The site a live record's key names.
#[must_use]
pub fn live_site_of_key(key: &str) -> Option<CanonicalId> {
    CanonicalId::new(key.strip_prefix(LIVE_PREFIX)?).ok()
}

/// The site a snapshot's key names.
#[must_use]
pub fn snapshot_site_of_key(key: &str) -> Option<CanonicalId> {
    CanonicalId::new(key.strip_prefix(SNAPSHOT_PREFIX)?).ok()
}

/// **What a party is** (doc 32 Q-CF-4, F-5): one detachment activation, or — the degenerate
/// detachment — one unit with no roster.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum PartyKey {
    /// An accepted roster's activation.
    Operation {
        /// The operation.
        operation: OperationId,
    },
    /// A unit on the ground with no roster: a party of one.
    Unit {
        /// The unit.
        unit: ObjId,
    },
}

/// **One party in a field**: its letter in admission order, what it is, and who leads it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldParty {
    /// Its letter (`PartyId::nth` in admission order).
    pub party: PartyId,
    /// What it is.
    pub key: PartyKey,
    /// Its leader (doc 32 §4a: the roster's leader, the pilot of a one-unit party); `None` for
    /// a party nobody leads (a casualty left by an earlier field).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leader: Option<ObjId>,
}

/// **One unit in a field.**
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldUnit {
    /// The unit.
    pub unit: ObjId,
    /// Its party.
    pub party: PartyId,
    /// Who sat in it at admission, and on which station.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pilot: Option<(ObjId, SmolStr)>,
    /// The hull epoch it was in (Q-H9), kept for the site record at demotion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch: Option<SortieId>,
    /// **Everyone aboard it at admission**, when it is a carrier (doc 32 cut CF.3; Q-CF-38's
    /// "aboard the carrier", `super::crew::crew_location`): doc 32 cut CF.5's crew state reads
    /// it, and sets each soul's record from it at the carrier's check-in.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aboard: Vec<ObjId>,
}

/// **A running field's live record** (`tacsim:field:live:<site>`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldLive {
    /// [`FIELD_LIVE_FORMAT`].
    pub format: u16,
    /// The field.
    pub field: FieldId,
    /// The IC instant it opened.
    pub opened_ic: i64,
    /// **How many times its task has been opened** (1 at promotion, one more per resume): each
    /// opening writes its black box to a log of its own, `field.<site>.<ordinal>.<segment>`, so
    /// a resumed task never needs a restored ledger tail.
    pub segment: u32,
    /// The arena millisecond of the last snapshot the world holds (0 before the first).
    pub at_ms: i64,
    /// The parties, by letter.
    pub parties: Vec<FieldParty>,
    /// The units, by unit.
    pub units: Vec<FieldUnit>,
}

impl FieldLive {
    /// The party a unit is in.
    #[must_use]
    pub fn party_of(&self, unit: ObjId) -> Option<PartyId> {
        self.units
            .iter()
            .find(|row| row.unit == unit)
            .map(|row| row.party)
    }

    /// The party a leader leads.
    #[must_use]
    pub fn led_by(&self, leader: ObjId) -> Option<PartyId> {
        self.parties
            .iter()
            .find(|party| party.leader == Some(leader))
            .map(|party| party.party)
    }

    /// The black box's log for this segment: `field.<site>.<ordinal>.<segment>`.
    #[must_use]
    pub fn log_id(&self) -> String {
        format!(
            "field.{}.{}.{}",
            self.field.site, self.field.ordinal, self.segment
        )
    }

    /// The record's own invariants.
    ///
    /// # Errors
    ///
    /// The first broken, as a sentence.
    pub fn check(&self) -> Result<(), String> {
        if self.format != FIELD_LIVE_FORMAT || self.field.ordinal == 0 || self.segment == 0 {
            return Err("a field's live record is malformed".to_owned());
        }
        if self.units.windows(2).any(|pair| match pair {
            [a, b] => a.unit >= b.unit,
            _ => false,
        }) || self.parties.windows(2).any(|pair| match pair {
            [a, b] => a.party >= b.party,
            _ => false,
        }) {
            return Err("a field's live record lists its units and parties in order".to_owned());
        }
        if self
            .units
            .iter()
            .any(|unit| !self.parties.iter().any(|party| party.party == unit.party))
        {
            return Err("every unit in a field is in one of its parties".to_owned());
        }
        Ok(())
    }

    /// The record as its document.
    ///
    /// # Errors
    ///
    /// The encode's refusal.
    pub fn document(&self) -> Result<serde_json::Value, String> {
        serde_json::to_value(self).map_err(|error| error.to_string())
    }

    /// A document back as a record, checked.
    ///
    /// # Errors
    ///
    /// The decode's or the check's refusal.
    pub fn from_document(value: &serde_json::Value) -> Result<FieldLive, String> {
        let live: FieldLive =
            serde_json::from_value(value.clone()).map_err(|error| error.to_string())?;
        live.check()?;
        Ok(live)
    }
}

/// **Write the live record** on the module's own lane.
pub(crate) fn write_live(ctx: &mut Ctx<'_>, live: &FieldLive) {
    match live.document() {
        Ok(value) => push_op(
            ctx.out,
            mush_core::ModuleWrite::JsonSet {
                key: live_key(&live.field.site),
                value,
            },
        ),
        Err(error) => tracing::error!(%error, "tacsim: a field's live record did not encode"),
    }
}

/// **Write a snapshot document.**
pub(crate) fn write_snapshot(ctx: &mut Ctx<'_>, site: &CanonicalId, document: serde_json::Value) {
    push_op(
        ctx.out,
        mush_core::ModuleWrite::JsonSet {
            key: snapshot_key(site),
            value: document,
        },
    );
}

/// **Delete both documents** (a field demoted or voided).
pub(crate) fn delete(ctx: &mut Ctx<'_>, site: &CanonicalId) {
    for key in [live_key(site), snapshot_key(site)] {
        push_op(ctx.out, mush_core::ModuleWrite::Del { key });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live() -> FieldLive {
        FieldLive {
            format: FIELD_LIVE_FORMAT,
            field: FieldId {
                site: CanonicalId::new("ahuna_port").expect("canonical"),
                ordinal: 2,
            },
            opened_ic: 100,
            segment: 1,
            at_ms: 0,
            parties: vec![
                FieldParty {
                    party: PartyId::nth(0).expect("A"),
                    key: PartyKey::Unit { unit: ObjId(7) },
                    leader: Some(ObjId(3)),
                },
                FieldParty {
                    party: PartyId::nth(1).expect("B"),
                    key: PartyKey::Unit { unit: ObjId(9) },
                    leader: None,
                },
            ],
            units: vec![
                FieldUnit {
                    unit: ObjId(7),
                    party: PartyId::nth(0).expect("A"),
                    pilot: Some((ObjId(3), SmolStr::new("cockpit"))),
                    epoch: None,
                    aboard: Vec::new(),
                },
                FieldUnit {
                    unit: ObjId(9),
                    party: PartyId::nth(1).expect("B"),
                    pilot: None,
                    epoch: None,
                    aboard: vec![ObjId(3)],
                },
            ],
        }
    }

    #[test]
    fn a_live_record_round_trips_and_names_its_log() {
        let live = live();
        let back = FieldLive::from_document(&live.document().expect("encodes")).expect("decodes");
        assert_eq!(back, live);
        assert_eq!(back.log_id(), "field.ahuna_port.2.1");
        assert_eq!(back.party_of(ObjId(9)), PartyId::nth(1));
        assert_eq!(back.led_by(ObjId(3)), PartyId::nth(0));
        assert_eq!(
            live_site_of_key(&live_key(&live.field.site)),
            Some(live.field.site.clone())
        );
    }

    #[test]
    fn a_unit_outside_every_party_is_refused() {
        let mut live = live();
        live.parties.pop();
        assert!(FieldLive::from_document(&live.document().expect("encodes")).is_err());
    }
}
