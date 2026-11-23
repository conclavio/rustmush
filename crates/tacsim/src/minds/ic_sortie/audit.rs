//! Actual owner inventory comparison, including the all-missing typed case.
use super::{
    records::{CompactedRecord, Record, StoredRecord, pruned_chain_digest},
    state::IcSorties,
};
use mush_core::{ModError, World};
use mush_roll::commerce::ic_claims::IcClaimHistory;
use std::collections::BTreeMap;
fn bad(_: impl std::fmt::Display) -> ModError {
    ModError::msg("IC seal and retained owner claim disagree")
}
pub(super) fn claims(
    world: &World,
    claims: &[IcClaimHistory],
    next_transition: u64,
) -> Result<(), ModError> {
    if world
        .module_state_owner(std::any::TypeId::of::<IcSorties>())
        .is_none_or(|v| v.as_str() != crate::NS)
    {
        return Err(bad("owner"));
    }
    let state = world
        .module_state::<IcSorties>()
        .ok_or_else(|| bad("state"))?;
    let expected_claims = state
        .rows
        .len()
        .checked_add(usize::try_from(state.pruned_count).map_err(bad)?)
        .ok_or_else(|| bad("inventory overflow"))?;
    if !state.loaded || state.next_transition != next_transition || expected_claims != claims.len()
    {
        return Err(bad("inventory"));
    }
    let units = world
        .module_state::<mush_tactics::UnitStore>()
        .ok_or_else(|| bad("units"))?
        .ic_custody_inventory()
        .map_err(bad)?;
    if units.len() != claims.len() {
        return Err(bad("units"));
    }
    // Index physical custody rows once so boot verification is O(N log N) instead of O(N^2).
    let mut units_by_open = BTreeMap::new();
    for unit in &units {
        if units_by_open.insert(unit.open(), unit).is_some() {
            return Err(bad("duplicate physical open"));
        }
    }
    let mut retained_matched = 0usize;
    let mut pruned = Vec::new();
    for claim in claims {
        let unit = units_by_open
            .get(&claim.open())
            .copied()
            .ok_or_else(|| bad("physical open"))?;
        match state.rows.get(&claim.open()) {
            Some(record) => {
                retained_matched += 1;
                match record.as_ref() {
                    StoredRecord::Full(row) => {
                        verify(row, claim)?;
                        unit.verify_open_unit(&row.open.unit).map_err(bad)?;
                        match (&row.closed, unit.closed()) {
                            (None, None) => {}
                            (Some(c), Some((_, _, _, digest))) if c.unit_digest == digest => {}
                            _ => return Err(bad("retained terminal unit differs")),
                        }
                    }
                    StoredRecord::Compacted(row) => {
                        verify_compacted(row, claim)?;
                        if unit.unit() != row.open.unit
                            || unit.sortie() != row.open.transition
                            || unit.seal_digest() != row.open.seal_digest
                        {
                            return Err(bad("compacted physical open differs"));
                        }
                        match unit.closed() {
                            Some((_, _, _, digest)) if row.closed.unit_digest == digest => {}
                            _ => return Err(bad("retained terminal unit differs")),
                        }
                    }
                }
            }
            None if state.pruned_count > 0 => {
                let Some((close_op, _, close_transition, _, _, _)) = claim.closed() else {
                    return Err(bad("pruned open claim"));
                };
                let Some((_, _, _, close_unit_digest)) = unit.closed() else {
                    return Err(bad("pruned open physical unit"));
                };
                if unit.unit() != claim.unit()
                    || unit.sortie() != claim.sortie()
                    || unit.seal_digest() != claim.seal_digest()
                {
                    return Err(bad("pruned physical open differs"));
                }
                pruned.push((
                    close_transition,
                    claim.open(),
                    close_op,
                    claim.sortie(),
                    claim.unit(),
                    claim.seal_digest(),
                    close_unit_digest,
                ));
            }
            None => return Err(bad("missing")),
        }
    }
    if retained_matched != state.rows.len()
        || u64::try_from(pruned.len()).map_err(bad)? != state.pruned_count
    {
        return Err(bad("pruned count differs"));
    }
    if state.pruned_count > 0 {
        pruned.sort_by_key(|row| row.0);
        let mut digest = None;
        for (
            close_transition,
            open_op,
            close_op,
            open_transition,
            unit,
            seal_digest,
            close_unit_digest,
        ) in pruned
        {
            digest = Some(
                pruned_chain_digest(
                    digest,
                    open_op,
                    close_op,
                    open_transition,
                    close_transition,
                    unit,
                    seal_digest,
                    close_unit_digest,
                )
                .map_err(bad)?,
            );
        }
        if digest != state.pruned_digest {
            return Err(bad("pruned checkpoint differs"));
        }
    }
    Ok(())
}
/// **An uncertified seal's audit** (doc 17h cut M8b G4): admitted only on an ungoverned core,
/// and only for the claim's own source digest.
pub(super) fn unfiled_matches(
    seal: &crate::arena::mind::MindSeal,
    source: &mush_core::DefinitionDigest,
) -> bool {
    seal.lifts_floor() && seal.requested.source_sha256 == *source.as_bytes()
}

/// **The sealed identity against the claim** (doc 17h cut M8b G4): a filed certificate
/// matches the claim's revision and rating exactly; a seal with no certificate — an unfiled
/// personal draft, which IC execution admits only on an ungoverned core (D-UM-41, D-UM-42) —
/// is held to the claim's source digest alone, and refused on a governed seal.
fn verify_identity(
    seal: &crate::arena::mind::MindSeal,
    claim: &IcClaimHistory,
) -> Result<(), ModError> {
    let identity = claim.revision();
    let Some(certificate) = seal.requested.origin.certificate() else {
        if !unfiled_matches(seal, &identity.source()) {
            return Err(bad("certificate"));
        }
        return Ok(());
    };
    let revision = identity.revision();
    let rating = claim.rating();
    if certificate.revision_character != revision.key.character
        || certificate.revision_mind != revision.key.mind
        || certificate.revision_number != revision.key.number
        || certificate.revision_digest != revision.digest
        || certificate.source_digest != identity.source()
        || certificate.rating_job != rating.job()
        || certificate.rating_id != rating.id().digest()
        || certificate.rating_schema != rating.schema()
        || certificate.rating_digest != rating.receipt_digest()
        || certificate.compatibility != claim.manifest().digest()
    {
        return Err(bad("immutable certificate"));
    }
    Ok(())
}
fn verify(row: &Record, claim: &IcClaimHistory) -> Result<(), ModError> {
    row.validate().map_err(bad)?;
    let open = &row.open;
    if open.action != *claim.action()
        || open.protocol.manifest != *claim.manifest()
        || open.protocol.participants != claim.participants()
        || open.dispatcher != claim.dispatcher()
        || open.admission_digest != claim.admission_digest().map_err(bad)?
        || open.at_ic != *claim.opened_at_ic()
        || open.execution.seal.unit != claim.unit()
        || open.transition != claim.sortie()
        || open.execution.digest() != claim.seal_digest()
    {
        return Err(bad("open"));
    }
    verify_identity(&open.execution.seal, claim)?;
    let core = crate::arena::core::read_core(&open.unit);
    let (component, class, governed) = core
        .map_or((None, crate::arena::core::CoreClass::Zero, true), |c| {
            (Some(c.component), c.class, c.governed)
        });
    if open.execution.seal.core != component
        || open.execution.seal.core_class != class
        || open.execution.seal.governed != governed
    {
        return Err(bad("hardware"));
    }
    match (&row.closed, claim.closed()) {
        (None, None) => {}
        (Some(c), Some((commit, action, transition, event, kind, at)))
            if c.operation == commit
                && &c.action == action
                && c.transition == transition
                && c.sequence == event.sequence
                && c.event_digest == event.digest
                && c.kind.data() == kind
                && &c.at_ic == at => {}
        _ => return Err(bad("close")),
    }
    // Compilation uses only the captured source/configuration. It never asks
    // the current seller/holder/parent for permission to resume a held sortie.
    // Doc 23 cut S.4b-3 (D-23-17 (b)): a row whose source no longer compiles is recorded, not a
    // refusal of the whole book — its hull flies the ROM when it is next installed.
    // P2-TSC-6: restrict synchronous Rhai compilation on boot strictly to active unclosed rows;
    // terminal closed sorties never execute again.
    row.open.execution.validate().map_err(bad)?;
    if row.closed.is_none() && row.open.execution.compile().is_err() {
        tracing::warn!(
            unit = row.open.unit.object().0,
            "tacsim: a retained IC mind no longer compiles; the book loads and its hull drops to the ROM at its next install"
        );
    }
    Ok(())
}
fn verify_compacted(row: &CompactedRecord, claim: &IcClaimHistory) -> Result<(), ModError> {
    row.validate().map_err(bad)?;
    let open = &row.open;
    if open.action != *claim.action()
        || open.protocol.manifest != *claim.manifest()
        || open.protocol.participants != claim.participants()
        || open.dispatcher != claim.dispatcher()
        || open.admission_digest != claim.admission_digest().map_err(bad)?
        || open.at_ic != *claim.opened_at_ic()
        || open.unit != claim.unit()
        || open.transition != claim.sortie()
        || open.seal_digest != claim.seal_digest()
    {
        return Err(bad("compacted open"));
    }
    let c = &row.closed;
    match claim.closed() {
        Some((commit, action, transition, event, kind, at))
            if c.operation == commit
                && &c.action == action
                && c.transition == transition
                && c.sequence == event.sequence
                && c.event_digest == event.digest
                && c.kind.data() == kind
                && &c.at_ic == at =>
        {
            Ok(())
        }
        _ => Err(bad("compacted close")),
    }
}
