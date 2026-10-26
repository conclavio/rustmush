//! Actual owner inventory comparison, including the all-missing typed case.
use super::{records::Record, state::IcSorties};
use mush_core::{ModError, World};
use mush_roll::commerce::ic_claims::IcClaimHistory;
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
    if !state.loaded || state.next_transition != next_transition || state.rows.len() != claims.len()
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
    for claim in claims {
        let record = state
            .rows
            .get(&claim.open())
            .ok_or_else(|| bad("missing"))?;
        verify(record, claim)?;
        let unit = units
            .iter()
            .find(|u| u.open() == claim.open())
            .ok_or_else(|| bad("physical open"))?;
        unit.verify_open_unit(&record.open.unit).map_err(bad)?;
        match (&record.closed, unit.closed()) {
            (None, None) => {}
            (Some(c), Some((_, _, _, digest))) if c.unit_digest == digest => {}
            _ => return Err(bad("retained terminal unit differs")),
        }
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
        || open.execution.seal.sortie.0 != claim.sortie()
        || open.execution.digest() != claim.seal_digest()
    {
        return Err(bad("open"));
    }
    let certificate = open
        .execution
        .seal
        .requested
        .origin
        .certificate()
        .ok_or_else(|| bad("certificate"))?;
    let identity = claim.revision();
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
    row.open.execution.compile().map_err(bad)
}
