// The Class III load mind (17b cut M4.2; the M4 row's (ii); doc 17 §18's ceiling
// variant). Test content, never shipped: a legal intent set with equal cost on
// both branches, and a loop calibrated to the band 4,300 to 4,499 ops + fees on
// the Class III row (4,500 ops a think; the think fee is 20). Never a kernel
// (fees), never lock, emcon, goto or rand. One op is one of the budget's
// progress callbacks: the two station calls cost 8 either arm, `let x = 0;` 2,
// the loop 2 + 3 N, the memory write 4 — 17 + 3 N, so N = 1,460 is 4,397 ops
// and 4,417 with the fee, 117 above the band's floor and 82 under its ceiling
// (the row's 4,500 less the fee is the last clean figure, 4,480). Pinned by
// crates/tacsim/tests/harness.rs (the digest, the exact ops and fee, equal cost
// off, starting and online, the `ops` fault on Class I's row).
fn think(view, card) {
    if view.power == "online" { hold(); } else { startup(); }
    let x = 0;
    for i in 0..1460 { x += 1; }
    this.x = x;
}
