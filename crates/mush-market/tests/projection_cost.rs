//! **D5 — `+market/depth` must not copy the whole economy** (E6c independent verify
//! fold).
//!
//! `commands::depth` held `module_state::<EconomyStore>().map(|s| s.snapshot().clone())`
//! — the only `.snapshot().clone()` in the tree. `EconomySnapshot` is some twenty owned
//! `BTreeMap`/`HashMap` fields (instruments, accounts and their indexes, idempotency,
//! assets, vaults, approvals, holds, cadences, obligations, contracts) with no `Arc` and
//! no copy-on-write, so a `Role::Player` verb with no rate limit deep-copied the whole
//! world's economy on **every call**, on the single-writer world task: measured at 200
//! calls going 2.8 ms → 31.3 ms after 300 extra *accounts*, with the market state
//! unchanged, while `+market/list` stayed flat.
//!
//! The clone bought nothing, either: `module_state::<EconomyStore>()` and
//! `module_state::<MarketStore>()` are both shared reborrows of the same `&World` and
//! can simply be held at once — which is what the fix does.
//!
//! A timing assertion would be a flaky test on a shared runner, so the durable tripwire
//! is the source itself: the cheapest honest statement of "no per-call whole-world copy
//! on a player verb" is that the file does not contain the construct. The behavioural
//! half of the same property — that `+market/depth` still renders byte-identically for
//! every viewer — is `ruleset-shadowrun`'s
//! `e6c_b1_depth_is_byte_identical_for_a_staffer_and_a_stranger`.

/// The market's command surface holds **no** whole-snapshot clone.
#[test]
fn vf_d5_depth_never_deep_copies_the_economy_snapshot() {
    let commands = include_str!("../src/commands.rs");
    assert!(
        !commands.contains(".snapshot().clone()"),
        "a per-call deep copy of the whole economy is back on a player verb in commands.rs"
    );
    // …and nowhere else in the crate either, so the construct cannot migrate one file
    // over and keep the tripwire green.
    for (name, src) in [
        ("api.rs", include_str!("../src/api.rs")),
        ("boot.rs", include_str!("../src/boot.rs")),
        ("module.rs", include_str!("../src/module.rs")),
        ("projection.rs", include_str!("../src/projection.rs")),
        ("store.rs", include_str!("../src/store.rs")),
        ("trade.rs", include_str!("../src/trade.rs")),
    ] {
        assert!(
            !src.contains(".snapshot().clone()"),
            "{name} deep-copies the whole economy snapshot"
        );
    }
}
