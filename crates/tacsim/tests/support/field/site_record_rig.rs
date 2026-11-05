//! The helpers of `site_record` (crates/tacsim/tests/domain_field/site_record.rs), hoisted
//! here on 2026-11-03 so that the modules which import them can live in any
//! shard binary: a test module can only import from a module of its own
//! binary, and with the helpers inside the test file every importer was
//! pinned to the file's shard (the field cluster was 62 summed test-minutes
//! in one shard). Every item and import is `pub(crate)`; the test half keeps
//! its name and takes them back with `use crate::site_record_rig::*`. Each domain
//! root declares this module by path, like support/vertex.rs.
#![allow(dead_code, unused_imports)]

pub(crate) use mush_core::ObjId;
pub(crate) use tacsim::surface::site_record::{
    FIELD_MAX_UNITS, FIELD_OPEN_CODE, GroundKind, LeftCondition, SiteKey, SiteRecord,
};

pub(crate) use crate::driving_rig::{
    AHUNA, Rig, cid, command, handles, land, land_all, launched, on_pad, on_pad_as, reboot, run,
    stocked, to_cell, typed, warm,
};

/// Ahuna Flank's record, as the family holds it.
pub(crate) fn site_record(rig: &mut Rig) -> Option<SiteRecord> {
    let h = handles(&rig.tw);
    typed(&mut rig.tw, &h.families.sites, &SiteKey(cid(AHUNA)))
}

/// A staff character (`SIM_ADMIN`): a submitting hand of its own, never GOD (whom the harness
/// forbids to submit).
pub(crate) fn staff_member(rig: &mut Rig) -> ObjId {
    let staff = rig.tw.spawn_player("Site staff");
    rig.tw
        .world_mut()
        .get_mut(staff)
        .expect("the staff character")
        .powers
        .insert(mush_core::Powers::SIM_ADMIN);
    staff
}

/// One staff fixture write by `who`, landed.
pub(crate) fn staff(rig: &mut Rig, who: ObjId, line: &str) {
    let (out, queued) = command(rig, who, line);
    assert_eq!(queued.len(), 1, "{line}: one write: {out}");
    land_all(rig, &queued);
}
