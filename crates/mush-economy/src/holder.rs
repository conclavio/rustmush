//! Doc 17h §3 M8c (the lead's ruling B1; doc 17 §16, D-UM-39): the typed
//! holder of a mind licence.
//!
//! Every commerce family that records who holds a paid licence, who is buying
//! one, or who a term was accepted by, used to carry a bare character `ObjId`.
//! An organisation can now hold a licence too (House owner attestation, M8c;
//! NPC procurement, M8f), so the holder is one of two arms:
//!
//! - `Character(c)`: the character `c` holds the licence for itself;
//! - `Organization { org, acting }`: the organisation `org` holds it, and the
//!   character `acting` acted for it when the record was written (bought it,
//!   attested it, accepted the term). The acting character is attribution; the
//!   authority is the live `roll.holder.org` grant `org_holder::org_actor`
//!   checks at every act.
//!
//! **Bytes.** The character arm serialises as the bare number it always was:
//! a stored row with `"holder": 7` decodes and re-encodes as `Character(7)`,
//! so every existing record's bytes and every pinned definition digest are
//! unchanged (B1, B8: no migration). The organisation arm is the object
//! `{ "org": n, "acting": m }`, refusing unknown fields.
//!
//! The type lives in the economy, the lowest crate that names an organisation
//! principal and that Roll and the market both depend on; Roll re-exports it
//! at `mush_roll::commerce::Holder`.
use mush_core::ObjId;
use mush_org::{OrgId, PrincipalId};
use serde::{Deserialize, Serialize};

/// Who holds a mind licence: a character, or an organisation through the
/// character that acted for it. See the module note for the bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "HolderWire", into = "HolderWire")]
pub enum Holder {
    /// The character holds the licence for itself.
    Character(ObjId),
    /// The organisation holds it; `acting` is the character that acted for it
    /// when this record was written.
    Organization { org: OrgId, acting: ObjId },
}

/// The wire twin: untagged, so the character arm is a bare number.
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum HolderWire {
    Character(ObjId),
    Organization(OrgHolderWire),
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OrgHolderWire {
    org: OrgId,
    acting: ObjId,
}
impl TryFrom<HolderWire> for Holder {
    type Error = &'static str;
    fn try_from(w: HolderWire) -> Result<Self, Self::Error> {
        match w {
            HolderWire::Character(c) => Ok(Self::Character(c)),
            HolderWire::Organization(OrgHolderWire { org, acting }) => {
                // M8c F8: organisation 0 is an organisation (mush-org mints
                // from 0); only the acting character must be someone.
                if acting.0 == 0 {
                    return Err("organisation holder names no acting character");
                }
                Ok(Self::Organization { org, acting })
            }
        }
    }
}
impl From<Holder> for HolderWire {
    fn from(h: Holder) -> Self {
        match h {
            Holder::Character(c) => Self::Character(c),
            Holder::Organization { org, acting } => {
                Self::Organization(OrgHolderWire { org, acting })
            }
        }
    }
}
impl From<ObjId> for Holder {
    fn from(c: ObjId) -> Self {
        Self::Character(c)
    }
}
impl Holder {
    /// The character that acts for this holder: the character itself, or the
    /// organisation's acting character. Readers that need a character (an
    /// alt refusal, a notice drain, an admission) take this and say so.
    #[must_use]
    pub const fn acting_character(self) -> ObjId {
        match self {
            Self::Character(c) | Self::Organization { acting: c, .. } => c,
        }
    }
    /// The principal the licence is held *for*: the Registry's reads (M8a's
    /// licence word, M8d's revocation per principal) key on this.
    #[must_use]
    pub const fn principal(self) -> PrincipalId {
        match self {
            Self::Character(c) => PrincipalId::Object(c),
            Self::Organization { org, .. } => PrincipalId::Organization(org),
        }
    }
    /// The organisation, for the organisation arm.
    #[must_use]
    pub const fn organization(self) -> Option<OrgId> {
        match self {
            Self::Character(_) => None,
            Self::Organization { org, .. } => Some(org),
        }
    }
    /// Whether this is the character arm (the only arm before M8c).
    #[must_use]
    pub const fn is_character(self) -> bool {
        matches!(self, Self::Character(_))
    }
    /// The character arm's character, if this is one.
    #[must_use]
    pub const fn character(self) -> Option<ObjId> {
        match self {
            Self::Character(c) => Some(c),
            Self::Organization { .. } => None,
        }
    }
    /// Whether the holder names no one (`Character(ObjId(0))`, the records'
    /// own "absent" value); the organisation arm never does.
    #[must_use]
    pub const fn is_nobody(self) -> bool {
        matches!(self, Self::Character(ObjId(0)))
    }
    /// The same principal, whoever acted: `Organization { org, .. }` matches
    /// itself under any acting character; a character matches only itself.
    #[must_use]
    pub fn same_principal(self, other: Self) -> bool {
        self.principal() == other.principal()
    }
    /// The key prefix an index keyed by holder uses, so one holder's rows
    /// are contiguous: `{character:020}` for a character and
    /// `org/{org:020}` for an organisation (the two never collide: a
    /// character's prefix is all digits). Mirrored byte for byte by
    /// mush-core's owner-index kernel (`terminal_capacity::owner_index`).
    #[must_use]
    pub fn key_prefix(self) -> String {
        match self {
            Self::Character(c) => format!("{:020}", c.0),
            Self::Organization { org, .. } => format!("org/{:020}", org.0),
        }
    }
}

/// M8c B3: whether an account on `scope` may pay for `buyer`: a character pays
/// from an unscoped (personal) account; an organisation pays from an account
/// on one of its own Active scopes (its treasury). The scope's organisation
/// is read from the org store; without one no scoped account fits.
#[must_use]
pub fn payer_scope_fits(
    world: &mush_core::World,
    buyer: Holder,
    scope: Option<mush_org::OrgScopeId>,
) -> bool {
    match (buyer, scope) {
        (Holder::Character(_), None) => true,
        (Holder::Character(_), Some(_)) | (Holder::Organization { .. }, None) => false,
        (Holder::Organization { org, .. }, Some(scope)) => world
            .module_state::<mush_org::OrgStore>()
            .and_then(|s| s.state().scope(scope).map(|node| (node.org, node.status)))
            .is_some_and(|(owner, status)| owner == org && status == mush_org::ScopeStatus::Active),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn character_arm_is_the_bare_number_both_ways() {
        let h = Holder::Character(ObjId(7));
        assert_eq!(serde_json::to_string(&h).unwrap(), "7");
        assert_eq!(serde_json::from_str::<Holder>("7").unwrap(), h);
        // The bytes a stored row carried before M8c are the bytes an `ObjId`
        // gives, exactly.
        assert_eq!(
            serde_json::to_string(&h).unwrap(),
            serde_json::to_string(&ObjId(7)).unwrap()
        );
        assert_eq!(
            serde_json::from_str::<Holder>("0").unwrap(),
            Holder::Character(ObjId(0))
        );
    }

    #[test]
    fn organization_arm_round_trips_as_an_object() {
        let h = Holder::Organization {
            org: OrgId(3),
            acting: ObjId(9),
        };
        let text = serde_json::to_string(&h).unwrap();
        assert_eq!(text, r#"{"org":3,"acting":9}"#);
        assert_eq!(serde_json::from_str::<Holder>(&text).unwrap(), h);
        assert_eq!(h.acting_character(), ObjId(9));
        assert_eq!(h.principal(), PrincipalId::Organization(OrgId(3)));
        assert_eq!(h.key_prefix(), "org/00000000000000000003");
        assert_eq!(
            Holder::Character(ObjId(9)).key_prefix(),
            "00000000000000000009"
        );
        // M8c F8: organisation 0 holds.
        let zero = Holder::Organization {
            org: OrgId(0),
            acting: ObjId(9),
        };
        assert_eq!(
            serde_json::from_str::<Holder>(r#"{"org":0,"acting":9}"#).unwrap(),
            zero
        );
        assert_eq!(zero.key_prefix(), "org/00000000000000000000");
        assert_eq!(zero.principal(), PrincipalId::Organization(OrgId(0)));
    }

    #[test]
    fn organization_arm_refuses_unknown_fields_and_nobody() {
        for text in [
            r#"{"org":3,"acting":9,"x":1}"#,
            r#"{"org":3,"acting":0}"#,
            r#"{"org":3}"#,
            r#""7""#,
            "-1",
        ] {
            assert!(serde_json::from_str::<Holder>(text).is_err(), "{text}");
        }
    }

    #[test]
    fn principal_and_same_principal() {
        let a = Holder::Organization {
            org: OrgId(3),
            acting: ObjId(9),
        };
        let b = Holder::Organization {
            org: OrgId(3),
            acting: ObjId(10),
        };
        assert_ne!(a, b);
        assert!(a.same_principal(b));
        assert!(!a.same_principal(Holder::Character(ObjId(9))));
        assert_eq!(
            Holder::Character(ObjId(9)).principal(),
            PrincipalId::Object(ObjId(9))
        );
        assert!(Holder::Character(ObjId(0)).is_nobody());
        assert!(!a.is_nobody());
    }
}
