//! Doc 46 CT.0: the personal renewal walk, pinned in `tests/ct0/renewal_personal.snap`. The
//! walk is shared with the House twin (`renewal_twin/tests/ct0_walk.rs`).
//!
//! Beside it, `ct0_capacity_wire` pins the coordinator's own strict persisted metadata (the
//! capacity `Reservation`, its `Target`s and `Key`s, and the `DefinitionRef` / `ClockStamp`
//! the renewal and purchase validators decode) in `tests/ct0/wire_capacity.snap`, from the
//! reservation a real personal Draft wrote.
use super::*;
use crate::action_state::terminal_capacity::tests::ct0_kit as kit;
use mush_testkit::snapshot::Snapshot;

#[test]
fn ct0_renewal_personal_walk() -> TestResult {
    twin::ct0_walk::walk::<Personal>("personal", "tests/ct0/renewal_personal.snap")
}

#[test]
fn ct0_capacity_wire() -> TestResult {
    let mut f = twin::fixture::<Personal>()?;
    let prepared = twin::draft(&mut f, 31, None)?;
    twin::install(&mut f, prepared)?;
    let stored = f
        .store
        .capacity
        .records
        .values()
        .next()
        .ok_or("reservation")?;
    let record = stored.record.clone();
    let mut snap = Snapshot::new();
    snap.section("capacity metadata identity");
    snap.line("metadata key prefix", PREFIX);
    snap.line("metadata path", stored.path.as_str());
    snap.line(
        "is capacity key",
        &is_capacity_key(stored.path.as_str()).to_string(),
    );
    for mode in [
        SlotMode::Create,
        SlotMode::UpdateExisting,
        SlotMode::IndustryOutput,
    ] {
        snap.wire("slot mode", &mode)?;
    }
    let bad_family = |json: &mut Value| json["family"] = json!("Not A Family!");
    snap.codec("Key", &record.sponsor, bad_family)?;
    let created = record
        .targets
        .iter()
        .find(|target| target.mode == SlotMode::Create)
        .ok_or("a created target")?;
    let existing = record
        .targets
        .iter()
        .find(|target| target.mode == SlotMode::UpdateExisting)
        .ok_or("an existing target")?;
    for (name, target) in [("Target (create)", created), ("Target (update)", existing)] {
        snap.codec(name, target, |json| json["mode"] = json!("overwrite"))?;
    }
    snap.codec("Reservation (fingerprint)", &record, |json| {
        json["targets"][0]["key"]["owner"] = json!("Not An Owner!");
    })?;
    let mut bare = record.clone();
    bare.protocol_fingerprint = None;
    snap.codec("Reservation (no fingerprint)", &bare, |json| {
        json["sponsor"]["family"] = json!("Not A Family!");
    })?;
    snap.section("Reservation encode");
    let encoded = encode(record.clone())?;
    snap.bytes("canonical", &encoded.canonical);
    snap.wire("document", &encoded.document)?;
    let mut wrong = record.clone();
    wrong.format = 2;
    kit::refusal(
        &mut snap,
        "format 2",
        encode(wrong).map(|_| ()).map_err(Into::into),
    )?;
    let mut empty = record.clone();
    empty.targets.clear();
    kit::refusal(
        &mut snap,
        "no targets",
        encode(empty).map(|_| ()).map_err(Into::into),
    )?;
    snap.codec("DefinitionRef", &record.effect, |json| {
        json["digest"] = json!("zz");
    })?;
    let stamp = crate::ClockStamp {
        clock: crate::ClockRef::WorldIc { generation: 1 },
        at: 1031,
    };
    snap.codec("ClockStamp", &stamp, |json| {
        json["clock"]["kind"] = json!("sundial");
    })?;
    snap.check(env!("CARGO_MANIFEST_DIR"), "tests/ct0/wire_capacity.snap")?;
    Ok(())
}
