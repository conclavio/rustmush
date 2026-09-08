//! E7 residue pass 2's sim half: **the `fail` anomaly names `@salvage/recover`
//! first** (P2; 10b §3.6's amendment).
//!
//! Cut 6 quarantines a `Failed` or `Captured` carrier root into the recovery room and
//! writes an anomaly with the recommended command filled in — but **both** commands it
//! recommended are staff-only (`@journey/divert` and `@spatial/place/domain`), so cut
//! 6's own recovery path made doc 10 §11's *"no arrow needs a staff act"* false for
//! every lost hull. That is the sentence 10b §15.1 item 4 calls out as "recoverable
//! today — by staff, which is exactly what §11 forbids".
//!
//! E7 residue pass 2's `@salvage/recover` is the owner's own route to the **cargo**,
//! so it goes first; the two staff commands stay for the **hull**, which is still
//! theirs to place. This file pins that ordering, because the ordering is the whole
//! claim: a page that mentions the owner's route *after* two staff commands is a page
//! whose reader still calls staff.

use std::path::Path;

/// The production half of one `mush-sim` source file — everything before its first
/// `#[cfg(test)]`.
fn production(relative: &str) -> String {
    let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(relative))
        .unwrap_or_else(|error| panic!("cannot read {relative}: {error}"));
    match text.find("#[cfg(test)]") {
        Some(at) => text[..at].to_owned(),
        None => text,
    }
}

/// The body of one free function, from its `fn <name>(` to the next top-level `fn`.
fn body_of<'s>(source: &'s str, name: &str) -> &'s str {
    let at = source
        .find(&format!("fn {name}("))
        .unwrap_or_else(|| panic!("`fn {name}` is gone"));
    let end = source[at..]
        .find("\n/// ")
        .map_or(source.len(), |offset| at + offset);
    &source[at..end]
}

/// **The stopped-journey page recommends the CUSTODIAN's own route before the staff
/// one**, and still recommends the staff one for the hull.
///
/// Three claims, and each is a separate way the amendment could rot: the salvage verb
/// is named at all; it is named **before** `@journey/divert`; and the staff route is
/// still there, because placing the hull is genuinely staff's and pretending otherwise
/// would leave a reader with no command for it.
#[test]
fn e7r2_p2_the_stopped_journey_page_names_salvage_before_the_staff_route() {
    let source = production("src/journey.rs");
    let body = body_of(&source, "fail");
    let salvage = body
        .find("@salvage/recover")
        .expect("the anomaly must name the custodian's own route to the cargo");
    let divert = body
        .find("@journey/divert")
        .expect("…and must keep the staff route to the hull");
    assert!(
        salvage < divert,
        "the owner's own route comes FIRST: a page that names two staff commands \
         before it is a page whose reader still calls staff"
    );
    assert!(
        body.contains("@spatial/place/domain"),
        "the second staff route survives too: placing the hull is still staff's"
    );
    assert!(
        body.contains("@salvage/status"),
        "and the reader is told how to see what there is to recover before recovering it"
    );
}

/// **The quarantine's own reason still names the staff commands and only those** —
/// because it is about the **hull**, which salvage does not move.
///
/// Salvage recovers the cargo out of the wreck; the wreck itself stays quarantined
/// until staff place it, and doc 10 T3 keeps its title exactly where it was. Naming
/// `@salvage/recover` here would promise something this pass deliberately did not
/// build (the salvage *claim* contract, with its contest window and named
/// adjudicator).
#[test]
fn e7r2_p2_the_hull_quarantine_reason_is_unchanged_and_still_staffs() {
    let source = production("src/journey.rs");
    let body = body_of(&source, "fail");
    let at = body
        .find("set_carrier_root_quarantine")
        .expect("a stopped ship is never loose");
    let reason = &body[at..];
    assert!(
        reason.contains("staff must place it"),
        "the HULL is still staff's to place: {reason}"
    );
    assert!(
        !reason.contains("@salvage"),
        "and salvage is not promised for it — recovering the cargo moves no hull and \
         no title (doc 10 T3)"
    );
}
