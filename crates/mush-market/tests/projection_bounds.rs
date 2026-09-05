//! **D8 — what the projection's bound actually bounds** (E6c independent verify fold).
//!
//! `PROJECTION_VISIT`'s doc comment called itself "the most records one projection may
//! **visit** (D-LBK-5's rule)" and the code placed `.take(PROJECTION_VISIT)` **after**
//! the `.filter(..)`, so it bounded records *matched*. The two disagreed, and only one
//! of them can be true.
//!
//! The `take` stays after the filter, because moving it before would let a wall of
//! hidden listings displace the public ones out of the published band — hidden state
//! moving public output, which is the one thing the gate forbids. So the constant is
//! documented for what it does, and the **visit** bound is stated where it really lives:
//! the store's own `MAX_LISTINGS` / `MAX_QUOTES` caps, asked over the `(catalog, market)`
//! index rather than over the whole map.
//!
//! **Deviation from the fold brief.** It proposed exposing a `visited` counter on
//! `Projection` "the way `MarketPage::visited` already is". That cannot be done:
//! `Projection` is the value the gate compares **byte for byte** between two worlds
//! differing only in hidden state, and the number of records visited is a function of
//! exactly that hidden state — a hidden listing is visited and then dropped. Publishing
//! it inside `Projection` would make the gate's own test fail for the right reason.
//! `MarketStore::projection_visits` is the same tripwire as a separate read.

use mush_economy::CatalogRef;
use mush_market::{MAX_LISTINGS, MarketId, PROJECTION_VISIT};
use smol_str::SmolStr;

fn catalog() -> CatalogRef {
    CatalogRef {
        ruleset: SmolStr::new_static("sr"),
        code: SmolStr::new_static("ammo"),
    }
}

fn venue() -> MarketId {
    MarketId::new("seattle").expect("a venue")
}

/// The catalogue entry every listing here names. The projection skips a listing whose
/// entry is gone (the fold's R6), so the fixture has to define it.
fn entry_doc() -> (SmolStr, serde_json::Value) {
    (
        SmolStr::new("market:catalog:sr.core:ammo"),
        serde_json::json!({
            "catalog": { "ruleset": "sr", "code": "ammo" },
            "pack": "sr.core", "definition_version": 1, "name": "Ammo",
            "representation": "instrument"
        }),
    )
}

/// One listing document: `hidden` ones are the wall, the last one is public.
fn listing_doc(id: u64, hidden: bool) -> serde_json::Value {
    serde_json::json!({
        "id": id, "seller": { "Organization": 1 }, "vendor": id, "market": "seattle",
        "catalog": { "ruleset": "sr", "code": "ammo" },
        "source": { "stock": { "vault": 1 } },
        "price": 10, "quantity": 5,
        "visibility": if hidden { "hidden" } else { "public" },
        "expires": { "clock": "RealUtc", "at": 1_000_000 }, "revision": 1
    })
}

/// A pair holding `MAX_LISTINGS` listings, all but one `Hidden`.
///
/// Every record is **visited** — that is what makes filter-then-take safe — and the
/// visit count is bounded by the store's own cap and by nothing else, which is what the
/// constant's doc comment now says.
#[test]
fn vf_d8_the_projection_visit_bound_is_the_store_cap_not_the_take() {
    // One listing per vendor, so the per-vendor cap the loader re-runs cannot trim the
    // wall out from under the test.
    let mut kv: Vec<(SmolStr, serde_json::Value)> = vec![entry_doc()];
    kv.extend((1..=MAX_LISTINGS as u64).map(|id| {
        (
            SmolStr::new(format!("market:listing:{id}")),
            listing_doc(id, id != MAX_LISTINGS as u64),
        )
    }));
    let store = mush_market::load(&kv);
    assert_eq!(store.live_listings().count(), MAX_LISTINGS);

    let visits = store.projection_visits(&catalog(), &venue());
    assert_eq!(
        visits, MAX_LISTINGS,
        "the walk is over the pair index and touches every candidate exactly once"
    );
    assert!(
        visits <= MAX_LISTINGS.saturating_add(mush_market::MAX_QUOTES),
        "a projection visited more records than the store can hold: {visits}"
    );
    assert!(
        visits > PROJECTION_VISIT,
        "the fixture is not the shape the finding is about: the take bounds MATCHES, and \
         `PROJECTION_VISIT` is not a visit bound"
    );

    // …and filter-then-take is what makes that safe: the one public listing is in the
    // band even though 1 023 hidden ones were visited before it.
    let snapshot = mush_economy::EconomySnapshot::default();
    let p = store.projection(&catalog(), &venue(), &snapshot, &[], 0);
    assert_eq!(
        p.band,
        (
            mush_economy::Amount::from_minor(10),
            mush_economy::Amount::from_minor(10)
        ),
        "a wall of hidden listings displaced the public one out of the band"
    );
}

/// A catalogue with **no** listings in a venue visits nothing at all — the pair index is
/// the walk, so a long tail of other catalogues costs exactly zero.
#[test]
fn vf_d8_a_pair_with_no_listings_visits_no_records() {
    let mut kv: Vec<(SmolStr, serde_json::Value)> = vec![entry_doc()];
    kv.extend((1..=64u64).map(|id| {
        (
            SmolStr::new(format!("market:listing:{id}")),
            listing_doc(id, false),
        )
    }));
    let store = mush_market::load(&kv);
    let other = CatalogRef {
        ruleset: SmolStr::new_static("sr"),
        code: SmolStr::new_static("nothing.here"),
    };
    assert_eq!(store.projection_visits(&other, &venue()), 0);
    assert_eq!(
        store.projection_visits(&catalog(), &MarketId::new("elsewhere").expect("a venue")),
        0
    );
    // The whole store is still there; it simply was not walked.
    assert_eq!(store.live_listings().count(), 64);
}
