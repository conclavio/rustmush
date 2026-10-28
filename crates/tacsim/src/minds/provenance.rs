//! **Where a core came from** (doc 17h cut M8a, A6; §2.3 "Provenance"; doc 17 §13).
//!
//! The read behind the inspection's `provenance` word, over two owners' records and nothing
//! this crate writes: the economy's managed title birth of the core
//! ([`mush_economy::EconomyStore::core_title_birth`]: the completed integration whose
//! output the core is, with the die lot and certificate its funding named: the industry's
//! acknowledged certificate, so the birth itself is the proof) and, while the inventory still
//! holds the die lot ([`mush_inventory::record::Lot`], read through the quantity delegation
//! tacsim holds; the integration consumes the lot, `mush_inventory`'s pool removes a spent lot,
//! so after delivery it is normally gone), that lot's origin, which must be the industry's
//! certificate for that very operation (`industry` / `certified_core_die.v1` / the
//! certificate's commit, `mush_industry`'s own `source`) or the read is `unknown`. Then the
//! core is `certified` by the supplier organization that stocked the die — the importer, in
//! the one shipped composition (`crate::hardware::composition`) — at the class ordered.
//! Anything short of that is `unknown`: M8a's read returns the two words and no other
//! (`heirloom` and `grey` wait on the cuts that can tell them apart).
//!
//! A read: it grants nothing, consumes nothing and vouches for nothing beyond the records.

use mush_core::{ObjId, World};
use mush_economy::EconomyStore;
use mush_inventory::record::{Lineage, LotId};
pub use mush_roll::commerce::registry_inspection::Provenance;

use crate::minds::registry_inspection::facts::Reads;
use crate::port::families::PortHandles;

/// The industry's provider word on a certified die lot's origin.
const INDUSTRY: &str = "industry";
/// The industry's certificate kind on a certified die lot's origin (`mush_industry`'s
/// `SOURCE_KIND`, named here because that constant is the industry's own).
const CERTIFIED_DIE: &str = "certified_core_die.v1";

/// **The provenance of `core`** (A6): `Certified { importer, grade }` when its title was born
/// of a completed integration (the funding's die certificate) and the die lot, if the
/// inventory still holds it, carries that certificate as its origin; `Unknown` otherwise — no
/// title, an unmanaged or charter title, a frozen economy, a lot re-lineaged or under another
/// certificate.
pub fn provenance_of<R: Reads>(w: &World, reader: &mut R, core: ObjId) -> Provenance {
    if w.module_state_owner(std::any::TypeId::of::<EconomyStore>())
        .is_none_or(|owner| owner.as_str() != "economy")
    {
        return Provenance::Unknown;
    }
    let Some(birth) = w
        .module_state::<EconomyStore>()
        .and_then(|economy| economy.core_title_birth(core).ok().flatten())
    else {
        return Provenance::Unknown;
    };
    let held = w
        .module_state::<PortHandles>()
        .and_then(|handles| handles.inventory.as_ref())
        .and_then(|inventory| reader.typed(&inventory.lots, &LotId(birth.die_lot)));
    let agrees = held.is_none_or(|lot| {
        matches!(lot.lineage(), Lineage::Origin { source }
            if source.provider.as_str() == INDUSTRY
                && source.kind.as_str() == CERTIFIED_DIE
                && source.id.as_str() == birth.die_certificate.to_hex())
    });
    if agrees {
        Provenance::Certified {
            importer: birth.supplier,
            grade: birth.class,
        }
    } else {
        Provenance::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two origin words the read matches are the industry's own, and the wire words of
    /// every provenance kind (`kind`-tagged, snake_case).
    #[test]
    fn m8a_provenance_words() {
        assert_eq!(
            serde_json::to_value(Provenance::Unknown).expect("serializes"),
            serde_json::json!({"kind": "unknown"})
        );
        assert_eq!(
            serde_json::to_value(Provenance::Heirloom).expect("serializes"),
            serde_json::json!({"kind": "heirloom"})
        );
        assert_eq!(
            serde_json::to_value(Provenance::Grey).expect("serializes"),
            serde_json::json!({"kind": "grey"})
        );
        assert_eq!(CERTIFIED_DIE, "certified_core_die.v1");
        assert_eq!(INDUSTRY, mush_industry::NS);
    }
}
