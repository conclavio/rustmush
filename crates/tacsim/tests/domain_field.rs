//! tacsim's integration tests: the `domain_field` binary, the IC field, crew, bay, escort, sortie and salvage journeys.
//!
//! One of four domain binaries (doc 43 cut TS.12, adopted by D-43-5 on 2026-11-04). Each module
//! below was a `tests/<name>.rs` target of its own until test-strategy cut T.3 merged them (every
//! integration binary links the whole engine), and it lives at `tests/domain_field/<name>.rs`. The CI's ten
//! `tacsim-integration-N` cells build all four binaries and run `cargo nextest run --partition
//! hash:N/10` over them, so the balance is the hash's and nothing is ever resharded: **a new test is
//! a `mod` line here**, in the domain it belongs to (rule 2.3 #3). A module imports only from its
//! own binary; the field helpers are the `*_rig` modules under tests/support/field/, declared in
//! every root.
//!
//! One module: `cargo nextest run -p tacsim --test domain_field -E 'test(/^<module>::/)'` or
//! `cargo test -p tacsim --test domain_field <module>::` (a substring filter, so no module here may be a
//! suffix of another's name; `ci_tacsim_shards` refuses that). One test: `... --test domain_field
//! <module>::<fn> -- --exact`. Sweeps run under nextest, one process per test; under plain
//! multi-threaded `cargo test` these modules share one process and its statics.

#[path = "support/field/claim_rig.rs"]
mod claim_rig;
#[path = "support/field/custody_rig.rs"]
mod custody_rig;
#[path = "support/field/driving_rig.rs"]
mod driving_rig;
#[path = "support/field/field_host_rig.rs"]
mod field_host_rig;
#[path = "support/field/field_raiders_rig.rs"]
mod field_raiders_rig;
#[path = "support/geology_keyed.rs"]
mod geology_keyed;
#[path = "support/field/grounds_rig.rs"]
mod grounds_rig;
#[path = "support/field/roster_rig.rs"]
mod roster_rig;
#[path = "support/field/site_record_rig.rs"]
mod site_record_rig;
mod support;
#[path = "support/field/surrender_rig.rs"]
mod surrender_rig;
#[path = "support/vertex.rs"]
mod vertex;

#[path = "domain_field/bay_36a.rs"]
mod bay_36a;
#[path = "domain_field/bay_gc8.rs"]
mod bay_gc8;
#[path = "domain_field/c4_hardware.rs"]
mod c4_hardware;
#[path = "domain_field/crew_custody.rs"]
mod crew_custody;
#[path = "domain_field/crew_exit.rs"]
mod crew_exit;
#[path = "domain_field/crew_treatment.rs"]
mod crew_treatment;
#[path = "domain_field/driving.rs"]
mod driving;
#[path = "domain_field/escort_lease.rs"]
mod escort_lease;
#[path = "domain_field/excursions.rs"]
mod excursions;
#[path = "domain_field/field_ap1.rs"]
mod field_ap1;
#[path = "domain_field/field_ap2.rs"]
mod field_ap2;
#[path = "domain_field/field_ap3.rs"]
mod field_ap3;
#[path = "domain_field/field_ap5.rs"]
mod field_ap5;
#[path = "domain_field/field_ap8.rs"]
mod field_ap8;
#[path = "domain_field/field_boarding.rs"]
mod field_boarding;
#[path = "domain_field/field_carrier.rs"]
mod field_carrier;
#[path = "domain_field/field_crash.rs"]
mod field_crash;
#[path = "domain_field/field_director.rs"]
mod field_director;
#[path = "domain_field/field_fr1.rs"]
mod field_fr1;
#[path = "domain_field/field_fr2.rs"]
mod field_fr2;
#[path = "domain_field/field_fr4.rs"]
mod field_fr4;
#[path = "domain_field/field_fr5.rs"]
mod field_fr5;
#[path = "domain_field/field_gc10.rs"]
mod field_gc10;
#[path = "domain_field/field_gc2.rs"]
mod field_gc2;
#[path = "domain_field/field_gc3.rs"]
mod field_gc3;
#[path = "domain_field/field_gc4.rs"]
mod field_gc4;
#[path = "domain_field/field_gc5.rs"]
mod field_gc5;
#[path = "domain_field/field_gc6.rs"]
mod field_gc6;
#[path = "domain_field/field_host.rs"]
mod field_host;
#[path = "domain_field/field_mw14a.rs"]
mod field_mw14a;
#[path = "domain_field/field_mw15.rs"]
mod field_mw15;
#[path = "domain_field/field_mw7.rs"]
mod field_mw7;
#[path = "domain_field/field_raiders.rs"]
mod field_raiders;
#[path = "domain_field/field_rs1.rs"]
mod field_rs1;
#[path = "domain_field/field_rs2.rs"]
mod field_rs2;
#[path = "domain_field/field_rs5.rs"]
mod field_rs5;
#[path = "domain_field/field_story.rs"]
mod field_story;
#[path = "domain_field/ic_sortie_surface.rs"]
mod ic_sortie_surface;
#[path = "domain_field/m8_end_user.rs"]
mod m8_end_user;
#[path = "domain_field/m8_leaks.rs"]
mod m8_leaks;
#[path = "domain_field/m8_world.rs"]
mod m8_world;
#[path = "domain_field/one_picture.rs"]
mod one_picture;
#[path = "domain_field/review_36a.rs"]
mod review_36a;
#[path = "domain_field/salvage_close.rs"]
mod salvage_close;
#[path = "domain_field/site_record.rs"]
mod site_record;
#[path = "domain_field/story_d_rs9.rs"]
mod story_d_rs9;
#[path = "domain_field/surrender.rs"]
mod surrender;
#[path = "domain_field/testimony.rs"]
mod testimony;
#[path = "domain_field/vacated_return_rs4.rs"]
mod vacated_return_rs4;
