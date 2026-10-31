//! **Doc 32 cut CF.1 — the site record and the presence picture**, over the composed world
//! (`driving`'s keyed rig: two hulls on Ahuna Flank's pad).
//!
//! The cut's gate: a staff fixture leaves a wreck and a crate on a site; the server restarts; both
//! are still on the record and seen on `+tv/map` from a rig within sight; `+tv/map` shows the
//! presence bands of the other party's rig; and with the site flagged promoted every surface act
//! refuses with the field named — at the command, and at the start's prepare for a start queued
//! before the flag landed. Clearing the flag lets work resume with the ground unchanged.

use mush_core::ObjId;
use tacsim::surface::site_record::{
    FIELD_MAX_UNITS, FIELD_OPEN_CODE, GroundKind, LeftCondition, SiteKey, SiteRecord,
};

use crate::driving::{
    AHUNA, Rig, cid, command, handles, land, land_all, launched, on_pad, on_pad_as, reboot, run,
    stocked, to_cell, typed, warm,
};

/// Ahuna Flank's record, as the family holds it.
fn site_record(rig: &mut Rig) -> Option<SiteRecord> {
    let h = handles(&rig.tw);
    typed(&mut rig.tw, &h.families.sites, &SiteKey(cid(AHUNA)))
}

/// A staff character (`SIM_ADMIN`): a submitting hand of its own, never GOD (whom the harness
/// forbids to submit).
fn staff_member(rig: &mut Rig) -> ObjId {
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
fn staff(rig: &mut Rig, who: ObjId, line: &str) {
    let (out, queued) = command(rig, who, line);
    assert_eq!(queued.len(), 1, "{line}: one write: {out}");
    land_all(rig, &queued);
}

/// **CF.1's gate** (doc 32 §6; Q-CF-15, Q-CF-21, Q-CF-24, Q-CF-28, Q-CF-30): A on the pad with two
/// rigs — rig 1 out at the ramp, rig 2's unit left by staff as a wreck at (120, 124) beside a crate
/// at (130, 124) — and B's rig 32 cells east. A sees the crate and the wreck (in sight of rig 1)
/// and B's rig as a band; B sees A's rig as a band and neither object (out of sight). The world
/// restarts: the record, the staff view and both prints are unchanged. Staff flag the site: a
/// start A queued before the flag is refused at the prepare, every `+tv` act refuses naming field
/// 1, the map names it. Staff clear it: the ground is as it was and A's drive starts.
#[test]
#[allow(clippy::too_many_lines)]
fn cf1_the_ground_survives_a_restart_and_a_promoted_site_refuses() {
    assert_eq!(tacsim::surface::MAX_EXCURSIONS_PER_SITE, FIELD_MAX_UNITS);
    assert_eq!(FIELD_MAX_UNITS, 24, "the per-site cap is the field cap");
    let mut rig = stocked();
    let (a, _, a_tvs) = on_pad(&mut rig, 2);
    let (b, _, b_tvs) = on_pad_as(&mut rig, "Other", 2, "Tally Moth", 1);
    launched(&mut rig, a, a_tvs[0]);
    launched(&mut rig, b, b_tvs[0]);
    to_cell(&mut rig, b_tvs[0], (160, 124));
    let s = staff_member(&mut rig);
    // A non-staff hand is refused before anything is parsed.
    let (out, queued) = command(&mut rig, a, "@tacsim/site ahuna_port promote");
    assert!(out.contains("Permission denied."), "{out}");
    assert!(queued.is_empty());
    // The fixture: a wreck and a crate.
    assert!(site_record(&mut rig).is_none(), "nothing yet");
    let wreck = a_tvs[1];
    staff(
        &mut rig,
        s,
        &format!("@tacsim/site ahuna_port leave #{} wreck 120,124", wreck.0),
    );
    staff(
        &mut rig,
        s,
        "@tacsim/site ahuna_port drop crate 2 metal_ore 130,124",
    );
    let record = site_record(&mut rig).expect("the site's record");
    assert_eq!(record.check(), Ok(()));
    assert_eq!(record.units.len(), 1);
    assert_eq!(record.units[0].unit, wreck);
    assert_eq!(record.units[0].condition, LeftCondition::Wreck);
    assert_eq!(record.units[0].pose.cell(), (120, 124));
    assert_eq!(record.objects.len(), 1);
    assert_eq!(record.objects[0].pose.cell(), (130, 124));
    assert!(matches!(
        &record.objects[0].kind,
        GroundKind::Cargo { manifest, .. } if manifest.get(&cid("metal_ore")) == Some(&2)
    ));
    assert_eq!(record.open_field(), None);
    let ground = "On the ground: a crate of 2 metal_ore 20 m E; a wreck 80 m W.";
    let check_prints = |rig: &mut Rig, s: ObjId| {
        let print_a = run(&mut rig.tw, a, "+tv/map");
        assert!(print_a.contains(ground), "{print_a}");
        assert!(print_a.contains("Presence: E 200-500 m."), "{print_a}");
        let print_b = run(&mut rig.tw, b, "+tv/map");
        assert!(print_b.contains("Presence: W 200-500 m."), "{print_b}");
        assert!(
            !print_b.contains("On the ground:"),
            "out of B's sight: {print_b}"
        );
        let truth = run(&mut rig.tw, s, "@tacsim/site ahuna_port");
        assert!(
            truth.contains(&format!("Unit #{}: wreck at (120, 124)", wreck.0)),
            "{truth}"
        );
        assert!(
            truth.contains("Object 1: a crate of 2 metal_ore at (130, 124)"),
            "{truth}"
        );
    };
    warm(&mut rig);
    check_prints(&mut rig, s);
    // The restart.
    let mut rig = reboot(rig, &[a, b, s]);
    assert_eq!(
        site_record(&mut rig),
        Some(record.clone()),
        "the record survives"
    );
    warm(&mut rig);
    check_prints(&mut rig, s);
    // A drive queued before the flag lands after it: the prepare refuses on the record.
    let (out, early) = command(&mut rig, a, "+tv/1 drive 140,124 go");
    assert_eq!(early.len(), 1, "a drive start: {out}");
    staff(&mut rig, s, "@tacsim/site ahuna_port promote");
    assert_eq!(
        site_record(&mut rig)
            .and_then(|r| r.open_field())
            .map(|f| f.ordinal),
        Some(1)
    );
    let refused = land(&mut rig, &early[0]).expect_err("the start refuses on a promoted site");
    assert!(refused.contains(FIELD_OPEN_CODE), "{refused}");
    // Every surface act refuses at the command, naming the fight.
    let line = "Field 1 is being fought on Ahuna Flank. Surface work there waits until it settles.";
    for act in [
        "+tv/1 drive 140,124 go",
        "+tv/1 recover go",
        "+tv/1 scan go",
        "+tv/2 launch go",
        "+tv/1 mine go",
    ] {
        let (out, queued) = command(&mut rig, a, act);
        assert_eq!(out.trim(), line, "{act}");
        assert!(queued.is_empty(), "{act}: nothing submitted");
    }
    let print_a = run(&mut rig.tw, a, "+tv/map");
    assert!(
        print_a.contains("Field 1 is open on Ahuna Flank: no surface work until it settles."),
        "{print_a}"
    );
    // Cleared: the ground is as it was, and work starts again.
    staff(&mut rig, s, "@tacsim/site ahuna_port demote");
    let after = site_record(&mut rig).expect("the record");
    assert_eq!(after.open_field(), None);
    assert_eq!(after.ordinal, 1);
    assert!(after.demoted_ic.is_some());
    assert_eq!(
        (after.units.clone(), after.objects.clone()),
        (record.units, record.objects)
    );
    let (out, queued) = command(&mut rig, a, "+tv/1 drive 140,124 go");
    assert_eq!(queued.len(), 1, "work resumes: {out}");
    land_all(&mut rig, &queued);
    let truth = run(&mut rig.tw, s, "@tacsim/site ahuna_port");
    assert!(
        truth.contains("no field open; 1 so far, the last closed at IC "),
        "{truth}"
    );
    let _: ObjId = b;
}
