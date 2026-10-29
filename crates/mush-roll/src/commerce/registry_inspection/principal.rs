//! **A principal revoked for a term** (doc 17h cut M8d D4; doc 17 §10): after a Registry
//! inspection finds an ungoverned core, the Registry bars a principal — a character, or an
//! organisation (M8c) — from every new licence until `until_ic`. One [`RevokedPrincipal`] per
//! principal in `roll_registry_revoked_principals`, keyed as the holder indexes key
//! (`{character:020}`, `org/{org:020}`, `Holder::key_prefix`); a later revocation replaces it.
//!
//! **What it bars** (the lead's ruling on option (c): equivalent in play to revoking every held
//! term while the window runs, and nothing destroyed, so the held terms resume at its end):
//! - **writing a new licence**: every path that writes one asks `refuse` in its own transaction
//!   — the filing (K3), the original, entitlement and House purchases (at the Fund, K8, and again
//!   at the grant), and the paid-resale acceptance. Renewing an existing held term and attesting
//!   an existing owner licence pass (K5, the lead's ruling): the same licence continues, its use
//!   still barred;
//! - **use**: both holding projections (`owner_holdings::project_owner_for`,
//!   `personal_paid_holdings::project_paid_for`) ask `barred_read`, so the dispatch holding,
//!   the admission, the own read that flies or seals and the source reads answer "not held";
//! - **moving**: a barred principal cannot sell, resell, offer or accept a charter or a paid
//!   licence, on either side; a pending sale or resale of theirs refuses its next step
//!   (`refuse_parties`), with no automatic refund — the operator's `@registry/revoke <licence>`
//!   still revokes a term and refunds as today.
//!
//! The record is written from tacsim's transaction through the inspection writer's
//! delegation, as the inspection record is.

use super::super::records::{PublicationError, Record};
use super::*;
use mush_core::{ActionTxn, CommitToken, PrepareCtx};
use mush_org::PrincipalId;

/// The record family.
pub const PRINCIPAL_FAMILY: &str = "roll_registry_revoked_principals";

/// **One revoked principal**: barred from new licences from `from_ic` until `until_ic` (World
/// IC seconds, the end exclusive), for the inspection `inspection` of `hull`'s ring at `seq`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevokedPrincipal {
    pub format: u16,
    /// A character or an organisation; never a system principal.
    pub principal: PrincipalId,
    pub from_ic: i64,
    pub until_ic: i64,
    pub inspection: CommitToken,
    pub hull: ObjId,
    pub seq: u64,
    /// The revocation's commit and the operator who made it.
    pub operation: CommitToken,
    pub operator: ObjId,
}

impl RevokedPrincipal {
    /// Whether the principal is barred at World IC second `now`.
    #[must_use]
    pub const fn bars(&self, now: i64) -> bool {
        self.from_ic <= now && now < self.until_ic
    }
}

/// The record key of `principal` (the holder indexes' prefix), or `None` for a system
/// principal or object 0.
#[must_use]
pub fn principal_key(principal: &PrincipalId) -> Option<String> {
    match principal {
        PrincipalId::Object(c) if c.0 != 0 => {
            Some(mush_economy::Holder::Character(*c).key_prefix())
        }
        PrincipalId::Organization(org) => Some(
            mush_economy::Holder::Organization {
                org: *org,
                acting: ObjId(1),
            }
            .key_prefix(),
        ),
        PrincipalId::Object(_) | PrincipalId::System(_) => None,
    }
}

impl Record for RevokedPrincipal {
    const FAMILY: &'static str = PRINCIPAL_FAMILY;
    const MAX_BYTES: usize = 2048;
    fn key(&self) -> String {
        principal_key(&self.principal).unwrap_or_default()
    }
    fn validate(&self) -> Result<(), PublicationError> {
        if self.format != 1
            || principal_key(&self.principal).is_none()
            || self.until_ic <= self.from_ic
            || self.hull.0 == 0
            || self.seq == 0
            || self.operator.0 == 0
            || self.inspection == self.operation
        {
            return Err(PublicationError::Invalid);
        }
        Ok(())
    }
}

/// **The refusal every licence-writing path asks** (D4): `Err(Unavailable)` while `holder`'s
/// principal is barred at the world's IC now; `Ok` for one never revoked or past its term.
///
/// # Errors
/// `Unavailable` for a barred principal, an unreadable world or an unreadable row.
pub(crate) fn refuse(
    ctx: &PrepareCtx<'_>,
    txn: &mut ActionTxn<'_>,
    families: &super::super::families::CommerceFamilies,
    holder: mush_economy::Holder,
) -> Result<(), PublicationError> {
    let now = ctx
        .world()
        .map_err(|_| PublicationError::Unavailable)?
        .world_ic_now()
        .at;
    if barred(txn, families, holder, now)? {
        return Err(PublicationError::Unavailable);
    }
    Ok(())
}

/// **The use bar** (D4): whether `holder`'s principal is barred now, read through the
/// holding projections' own reader.
///
/// # Errors
/// `Unavailable` when the row cannot be read.
pub(crate) fn barred_read(
    ctx: &impl super::super::read::CommerceRead,
    families: &super::super::families::CommerceFamilies,
    holder: mush_economy::Holder,
) -> Result<bool, PublicationError> {
    let Some(key) = principal_key(&holder.principal()) else {
        return Ok(false);
    };
    let now = ctx.world().world_ic_now().at;
    Ok(ctx
        .get(&families.revoked_principals, &key)
        .map_err(|_| PublicationError::Unavailable)?
        .is_some_and(|record| record.principal == holder.principal() && record.bars(now)))
}

/// **The move bar** (D4): `Err(Unavailable)` when any character party to a sale, resale,
/// charter offer or transfer is barred now.
///
/// # Errors
/// `Unavailable` for a barred party or an unreadable world or row.
pub(crate) fn refuse_parties(
    ctx: &PrepareCtx<'_>,
    txn: &mut ActionTxn<'_>,
    families: &super::super::families::CommerceFamilies,
    parties: &[ObjId],
) -> Result<(), PublicationError> {
    for party in parties {
        refuse(ctx, txn, families, mush_economy::Holder::Character(*party))?;
    }
    Ok(())
}

/// Whether `holder`'s principal — the character, or the organisation whoever acts for it — is
/// barred at World IC second `now`.
///
/// # Errors
/// `Unavailable` when the row cannot be read.
pub(crate) fn barred(
    txn: &mut ActionTxn<'_>,
    families: &super::super::families::CommerceFamilies,
    holder: mush_economy::Holder,
    now: i64,
) -> Result<bool, PublicationError> {
    let Some(key) = principal_key(&holder.principal()) else {
        return Ok(false);
    };
    Ok(txn
        .get(&families.revoked_principals, &key)
        .map_err(|_| PublicationError::Unavailable)?
        .is_some_and(|record| record.principal == holder.principal() && record.bars(now)))
}

/// The key shape the family admits: `{character:020}` or `org/{org:020}`.
pub(crate) fn valid_key(key: &str, person: fn(&str) -> bool, org: fn(&str) -> bool) -> bool {
    let mut parts = key.split('/');
    let head = parts.next().unwrap_or("");
    (person(head) && parts.next().is_none())
        || (head == "org" && parts.next().is_some_and(org) && parts.next().is_none())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(principal: PrincipalId) -> RevokedPrincipal {
        RevokedPrincipal {
            format: 1,
            principal,
            from_ic: 100,
            until_ic: 100 + 90 * 86_400,
            inspection: CommitToken::from_bytes([1; 16]),
            hull: ObjId(12),
            seq: 3,
            operation: CommitToken::from_bytes([2; 16]),
            operator: ObjId(5),
        }
    }

    /// **M8d D4**: the window bars from its first instant to just before its end; a character
    /// and an organisation key as the holder indexes do; a system principal is refused.
    #[test]
    fn m8d_a_revoked_principals_window_edges() {
        let r = record(PrincipalId::Object(ObjId(7)));
        assert!(r.validate().is_ok());
        assert!(!r.bars(99), "before the revocation");
        assert!(r.bars(100), "its first instant");
        assert!(r.bars(100 + 90 * 86_400 - 1), "its last instant");
        assert!(!r.bars(100 + 90 * 86_400), "the end is exclusive");
        assert_eq!(r.key(), format!("{:020}", 7));
        let org = record(PrincipalId::Organization(mush_org::OrgId(0)));
        assert!(org.validate().is_ok(), "organisation 0 is an organisation");
        assert_eq!(org.key(), format!("org/{:020}", 0));
        for bad in [
            record(PrincipalId::Object(ObjId(0))),
            RevokedPrincipal {
                until_ic: 100,
                ..record(PrincipalId::Object(ObjId(7)))
            },
        ] {
            assert!(bad.validate().is_err());
        }
        assert!(
            principal_key(&PrincipalId::System(
                mush_org::SystemPrincipalId::new("registry").unwrap()
            ))
            .is_none()
        );
    }

    /// **M8d D4**: the refusal reads the family in a real Roll transaction: a barred character
    /// and a barred organisation under any acting character, from the term's first instant to
    /// before its end; another principal, a character of the organisation's number, and the
    /// instant the term ends are not barred.
    #[test]
    fn m8d_a_revoked_principal_is_barred_in_the_rolls_own_transaction() {
        let mut f = super::super::super::filing_tests::Fixture::new(true);
        let family = f.families.revoked_principals.clone();
        let character = record(PrincipalId::Object(ObjId(7)));
        let org = record(PrincipalId::Organization(mush_org::OrgId(3)));
        let mut txn = f.engine.action_transaction(&family).unwrap();
        txn.insert(&family, &character.key(), character.clone())
            .unwrap();
        txn.insert(&family, &org.key(), org.clone()).unwrap();
        let end = character.until_ic;
        let cases = [
            (mush_economy::Holder::Character(ObjId(7)), 100, true),
            (mush_economy::Holder::Character(ObjId(7)), end - 1, true),
            (mush_economy::Holder::Character(ObjId(7)), end, false),
            (mush_economy::Holder::Character(ObjId(7)), 99, false),
            (mush_economy::Holder::Character(ObjId(8)), 150, false),
            (mush_economy::Holder::Character(ObjId(3)), 150, false),
            (
                mush_economy::Holder::Organization {
                    org: mush_org::OrgId(3),
                    acting: ObjId(9),
                },
                150,
                true,
            ),
            (
                mush_economy::Holder::Organization {
                    org: mush_org::OrgId(4),
                    acting: ObjId(9),
                },
                150,
                false,
            ),
        ];
        for (holder, now, expected) in cases {
            assert_eq!(
                barred(&mut txn, &f.families, holder, now).unwrap(),
                expected,
                "{holder:?} at {now}"
            );
        }
    }
}
