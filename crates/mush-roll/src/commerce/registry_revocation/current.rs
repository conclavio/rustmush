//! Current admission only. Historical grant/claim/refund readers never call
//! this module. A sanctioned old generation is never restored by clock changes.
use super::super::{
    TermGeneration, WorldIcSeconds, families::CommerceFamilies, read::CommerceRead,
};
use super::*;
use records::{RevocationOperation, RevokedTerm};
fn check_pair(
    status: &RevokedTerm,
    cause: &RevocationOperation,
    licence: LicenceId,
    generation: TermGeneration,
) -> Result<(), PublicationError> {
    status.matches(cause)?;
    if status.licence != licence || status.generation != generation {
        return Err(PublicationError::Mismatch);
    }
    Ok(())
}
fn gate(
    status: Option<&RevokedTerm>,
    successor: Option<WorldIcSeconds>,
) -> Result<(), PublicationError> {
    match (status, successor) {
        (None, _) => Ok(()),
        (Some(s), Some(now)) if s.permits_successor(now) => Ok(()),
        _ => Err(PublicationError::Unavailable),
    }
}
pub(crate) fn require(
    ctx: &impl CommerceRead,
    f: &CommerceFamilies,
    licence: LicenceId,
    generation: TermGeneration,
    successor: Option<WorldIcSeconds>,
) -> Result<(), PublicationError> {
    let key = RevokedTerm::key_for(licence, generation);
    let status = ctx.get(&f.revoked_terms, &key).map_err(bad)?;
    if let Some(s) = status {
        let cause = ctx
            .get(&f.registry_revocations, &s.operation.to_hex())
            .map_err(bad)?
            .ok_or(PublicationError::Missing)?;
        check_pair(s, cause, licence, generation)?;
    }
    gate(status, successor)
}
pub(crate) fn require_in_txn(
    txn: &mut mush_core::ActionTxn<'_>,
    f: &CommerceFamilies,
    licence: LicenceId,
    generation: TermGeneration,
    successor: Option<WorldIcSeconds>,
) -> Result<(), PublicationError> {
    let status = txn
        .get(&f.revoked_terms, &RevokedTerm::key_for(licence, generation))
        .map_err(bad)?
        .cloned();
    if let Some(s) = &status {
        let cause = txn
            .get(&f.registry_revocations, &s.operation.to_hex())
            .map_err(bad)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        check_pair(s, &cause, licence, generation)?;
    }
    gate(status.as_ref(), successor)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sanctioned_predecessor_use_never_restores_but_successor_waits_for_original_grace() {
        let window = super::super::super::TermWindow::new(
            WorldIcSeconds(0),
            WorldIcSeconds(0),
            WorldIcSeconds(10),
            WorldIcSeconds(20),
        )
        .unwrap();
        let s = RevokedTerm {
            format: 1,
            licence: LicenceId::new(1).unwrap(),
            generation: TermGeneration::new(1).unwrap(),
            operation: CommitToken::from_bytes([1; 16]),
            term_digest: mush_core::DefinitionDigest::from_bytes([2; 32]),
            window,
        };
        for now in [-1, 0, 10, 19, 20, 21, i64::MAX] {
            assert!(
                gate(Some(&s), None).is_err(),
                "old accepted term is never restored"
            );
            assert_eq!(gate(Some(&s), Some(WorldIcSeconds(now))).is_ok(), now >= 20);
        }
        assert!(
            gate(None, None).is_ok(),
            "a different unsanctioned generation is not borrowed authority"
        );
    }
}
