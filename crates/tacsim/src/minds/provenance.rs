//! **Where a core came from** (doc 17h cut M8a, A6; §2.3 "Provenance"; doc 17 §13; the fix
//! round's F6).
//!
//! The read behind the inspection's `provenance` word, over two owners' records and nothing
//! this crate writes: the economy's managed title birth of the core
//! ([`mush_economy::EconomyStore::core_title_birth`]: the completed integration whose output
//! the core is, with the die lot and the die certificate its funding named) and the industry's
//! retained die certificate under that name ([`mush_industry::core_production::DieCertificates`],
//! the read-only grant the hardware composition gives this crate). The certificate is the
//! proof: it must exist, be readable, and name the birth's die lot and class; then the core is
//! `certified` by the certificate's importer at the certificate's class. A missing or
//! unreadable certificate, or one that disagrees with the birth, is `unknown` — never
//! agreement. M8a's read returns the two words and no other (`heirloom` and `grey` arrive in
//! cut M8b).
//!
//! A read: it grants nothing, consumes nothing and vouches for nothing beyond the records.

use mush_core::{ObjId, World};
use mush_economy::EconomyStore;
use mush_economy::core_integration::{CoreClass, CoreTitleBirth};
use mush_industry::core_production::{DieCertificate, DieCertificates, DieClass};
pub use mush_roll::commerce::registry_inspection::Provenance;

use crate::minds::registry_inspection::facts::Reads;

/// The economy's class word for the industry's die class.
const fn economy_class(class: DieClass) -> CoreClass {
    match class {
        DieClass::I => CoreClass::I,
        DieClass::II => CoreClass::II,
        DieClass::III => CoreClass::III,
    }
}

/// **What a birth and its certificate say together**: `Certified` by the certificate's
/// importer at its class when the certificate is the birth's (its operation, its lot, its
/// class); `Unknown` when there is no certificate or it disagrees.
#[must_use]
pub fn from_certificate(
    birth: &CoreTitleBirth,
    certificate: Option<&DieCertificate>,
) -> Provenance {
    let Some(certificate) = certificate else {
        return Provenance::Unknown;
    };
    let grade = economy_class(certificate.class());
    if certificate.operation() != birth.die_certificate
        || certificate.lot().0 != birth.die_lot
        || grade != birth.class
    {
        return Provenance::Unknown;
    }
    Provenance::Certified {
        importer: certificate.importer(),
        grade,
    }
}

/// **The provenance of `core`** (A6, F6): `Certified { importer, grade }` from the retained
/// die certificate its title's birth names, when that certificate agrees with the birth;
/// `Unknown` otherwise — no title, an unmanaged or charter title, a frozen economy, no
/// certificate read granted, a missing or unreadable certificate, or one that disagrees.
pub fn provenance_of<R: Reads>(w: &World, reader: &mut R, core: ObjId) -> Provenance {
    let owned_by = |type_id, owner: &str| {
        w.module_state_owner(type_id)
            .is_some_and(|o| o.as_str() == owner)
    };
    if !owned_by(std::any::TypeId::of::<EconomyStore>(), "economy")
        || !owned_by(std::any::TypeId::of::<DieCertificates>(), mush_industry::NS)
    {
        return Provenance::Unknown;
    }
    let Some(birth) = w
        .module_state::<EconomyStore>()
        .and_then(|economy| economy.core_title_birth(core).ok().flatten())
    else {
        return Provenance::Unknown;
    };
    let Some(certificates) = w.module_state::<DieCertificates>() else {
        return Provenance::Unknown;
    };
    let certificate = reader.typed(certificates.handle(), &birth.die_certificate.to_hex());
    from_certificate(&birth, certificate.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire words of every provenance kind (`kind`-tagged, snake_case) and the die class
    /// mapping.
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
        assert_eq!(economy_class(DieClass::II), CoreClass::II);
    }
}
