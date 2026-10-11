// Fixture I (17b cut M5.1): the scripted fixture mind of every non-Doctrine
// lobby path. Test content, never shipped: held only by the test provider
// (tests/support/holdings.rs), rated Class I, presets Line and Sniper, a
// charter and line that are not the Registry's. It starts its hull up until
// it is online and then holds — a few ops a think, far under Class I's 1,500.
// Its digest is pinned in the provider.
fn think(view, card) {
    if view.power == "online" { hold(); } else { startup(); }
}
