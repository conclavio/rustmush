//! E7 cut 11 **independent verify fold** — G16 (a reproducible
//! `TimelineChanged` tripwire) and §14's missing L4 writer census
//! (10b §3.11 B6, §15.1 item 8, §14.1).

// ---- E7 cut 11 independent verify fold ----

use std::path::{Path, PathBuf};

fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .to_path_buf()
}

fn production(rel: &str) -> String {
    let text = std::fs::read_to_string(crates_dir().join(rel)).expect("a source file");
    let boundary = text.find("#[cfg(test)]").unwrap_or(text.len());
    text[..boundary].to_owned()
}

/// **G16. `InterruptionCause::TimelineChanged` is declared and never
/// constructed — and the two failures are now distinguishable.**
///
/// §3.11's B6 row and §15.1 item 8 say *"a grep for
/// `InterruptionCause::TimelineChanged` finds only its own declaration"*. The
/// substance holds — there is no producer — but the stated grep finds **zero**
/// hits, because the variant is declared **bare** inside the enum
/// (`workorder.rs:450`) and the only other occurrence of the word in the tree is
/// a doc comment in `ruleset-tacsim/src/config.rs`. A later cut re-running the
/// cited grep gets an empty result and cannot tell "declared and unset" from
/// "deleted". This is the tripwire it was meant to be: **deletion** and **first
/// production use** fail differently.
#[test]
fn vf_g16_timeline_changed_is_declared_and_constructed_nowhere() {
    // (1) It still EXISTS. Deleting the variant fails to compile.
    let _declared = mush_industry::InterruptionCause::TimelineChanged;

    // (2) It is CONSTRUCTED nowhere in any crate's production source.
    let mut constructed: Vec<String> = Vec::new();
    let mut declarations = 0usize;
    let entries = std::fs::read_dir(crates_dir()).expect("crates/");
    for entry in entries.flatten() {
        let src = entry.path().join("src");
        if !src.is_dir() {
            continue;
        }
        let mut stack = vec![src];
        while let Some(dir) = stack.pop() {
            let Ok(read) = std::fs::read_dir(&dir) else {
                continue;
            };
            for file in read.flatten() {
                let path = file.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                let boundary = text.find("#[cfg(test)]").unwrap_or(text.len());
                for line in text[..boundary].lines() {
                    let code = line.trim_start();
                    if code.starts_with("//") || !code.contains("TimelineChanged") {
                        continue;
                    }
                    if code.contains("InterruptionCause::TimelineChanged") {
                        constructed.push(format!("{}: {code}", path.display()));
                    } else if code.starts_with("TimelineChanged") {
                        declarations += 1;
                    }
                }
            }
        }
    }
    assert_eq!(
        declarations, 1,
        "the bare declaration in `mush-industry/src/workorder.rs` — if this is 0 \
         the variant was DELETED, which is a different failure from a producer \
         appearing"
    );
    // **DELIBERATELY FLIPPED by E7 residue pass 1, R5(b)** (10b §15.1 item 8).
    //
    // Cut 11's fold pinned this at ZERO and said so precisely: *declared and
    // constructed nowhere*, with the tripwire's whole point being that **deletion**
    // and **first production use** fail differently. This is the first production
    // use, and it is one site: `provider::wo_deliver` pauses an order whose
    // production was started on a world-time generation that is no longer live,
    // which is the D-E7-8 default the variant was declared for. The pin moves from
    // "nowhere" to "exactly one place, and here it is", so a SECOND producer — two
    // places deciding when production pauses — still fails this test.
    assert_eq!(
        constructed.len(),
        1,
        "exactly one producer, in `wo_deliver`: {constructed:?}"
    );
    assert!(
        constructed[0].contains("mush-industry") && constructed[0].contains("provider.rs"),
        "and it is the provider's own transition, never a command's: {constructed:?}"
    );
}

/// **G13/L4. §14 names synchronous writers for L1 only; here is L4's, and it is
/// two functions rather than "101 sites over 8 files".**
///
/// L4 (`industry.workorder`) is the largest single-crate lane and §14.2 gives it
/// no writer enumeration at all, so §14.3 criterion 2 ("the lane moves whole")
/// cannot be checked for it. Every mutation of the family goes through exactly
/// two `IndustryFamilies` methods, called from six `provider.rs` sites — which
/// is a citable symbol list of the kind §14.1's own rule demands.
#[test]
fn vf_g13_l4_the_work_order_writer_census_is_pinned() {
    let families = production("mush-industry/src/families.rs");
    assert_eq!(
        families.matches("txn.insert(&self.work_orders").count(),
        1,
        "`IndustryFamilies::open_work_order` is the only insert"
    );
    assert_eq!(
        families.matches("txn.replace(&self.work_orders").count(),
        1,
        "`IndustryFamilies::put_work_order` is the only replace"
    );
    assert_eq!(
        families.matches("txn.remove(&self.work_orders").count(),
        0,
        "a work order is never removed: it goes terminal in place"
    );

    let provider = production("mush-industry/src/provider.rs");
    assert_eq!(
        provider.matches(".open_work_order(").count(),
        1,
        "one opener (`wo_open`'s prepare)"
    );
    // **Six since E7 residue pass 1, R5(b)**: the sixth is `wo_deliver`'s
    // `Interrupted { TimelineChanged }` pause, which is the one writer that variant
    // never had. Still all from an industry-owned action's prepare, still under a
    // checked revision, and still no site outside `provider.rs`.
    assert_eq!(
        provider.matches(".put_work_order(").count(),
        6,
        "six checked-revision replacements, all from an industry-owned action's \
         prepare"
    );
    // No other crate writes the family at all: the lane is `mush-industry`'s.
    for rel in [
        "mush-industry/src/commands.rs",
        "mush-industry/src/store.rs",
    ] {
        let text = production(rel);
        assert!(
            !text.contains(".open_work_order(") && !text.contains(".put_work_order("),
            "{rel} reads the work-order family and never writes it"
        );
    }
}

/// **G1. Nothing forbids the `mush-market -> mush-inventory` edge the corrected
/// §15.5 prescribes — and this fold does not add it.**
///
/// §15.5 concluded the `Representation::Lot` lift was unbuildable because
/// `mush-inventory` "registers no verb that would serve" it. The delegated path
/// needs no verb: `InventoryModule::with_quantity_delegates` over
/// `Registrar::delegate_state_families` is in production today and
/// **`mush-industry` uses it**, calling `reserve` / `consume` / `release` from an
/// industry-owned action's prepare on an endpoint the acting player does not
/// own. The two manifest tripwires point the OTHER way; neither bans the
/// direction the lift needs, and the lift adds the edge in its own commit.
#[test]
fn vf_g1_the_delegated_quantity_seam_is_in_production_and_unblocked() {
    // The seam, and its production consumer.
    assert!(
        production("mush-inventory/src/module.rs").contains("delegate_state_families("),
        "`with_quantity_delegates` delegates the families at composition"
    );
    let provider = production("mush-industry/src/provider.rs");
    for needle in [".reserve(txn", ".consume(", ".release(txn"] {
        assert!(
            provider.contains(needle),
            "an industry-owned prepare already drives the delegated inventory \
             families: {needle}"
        );
    }
    assert!(
        production("mush-industry/src/families.rs").contains("inventory: InventoryFamilies"),
        "…on cut 2's handles, delegated to this owner at composition"
    );
    // And the direction the lift needs is banned by nothing: the two tripwires
    // forbid `mush-inventory -> mush-market` and `mush-industry -> mush-market`.
    let manifest = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"),
    )
    .expect("mush-industry/Cargo.toml");
    assert!(
        !manifest
            .lines()
            .any(|line| line.trim_start().starts_with("mush-market")),
        "the industry tripwire's premise still holds: no `mush-industry -> \
         mush-market` dependency"
    );
    // This fold adds NO crate edge: `mush-market` still declares none on
    // `mush-inventory`, and it lands with the lift.
    let market = std::fs::read_to_string(crates_dir().join("mush-market/Cargo.toml"))
        .expect("mush-market/Cargo.toml");
    assert!(
        !market.contains("mush-inventory"),
        "the `mush-market -> mush-inventory` edge lands in the LIFT's own commit \
         (10b §15.5, as rewritten by fold F1), never in this fold"
    );
}
