//! E7 cut 11 **independent verify fold** — G2: `ENGINE_OWNED_RESERVATIONS` is a
//! MONEY table, and §15.5 step (b) must not reach for it (10b §15.5).

// ---- E7 cut 11 independent verify fold ----

/// **G2. `ENGINE_OWNED_RESERVATIONS` cannot bind a lot, and the lockout §15.5
/// step (b) wanted already holds structurally.**
///
/// §15.5 step (b) said: *"register `"market.listing"` in
/// `ENGINE_OWNED_RESERVATIONS` and settle it through a
/// `settle_engine_reservation`-shaped seam … which is also what stops a player
/// releasing a listing's hold from under the market."* Both halves are wrong for
/// the thing being lifted. The table is matched only by
/// `is_engine_owned_reservation` against `HoldPurpose::Reservation` — a **money
/// hold on an account** — and `settle_engine_reservation` posts through
/// `PostAuth::Injected(&EngineReservationAuth)`; it cannot reach a K4b
/// `ReservationRecord`, which is written only by `InventoryFamilies::{reserve,
/// consume, release, expire}`, each taking `&mut ActionTxn`.
///
/// The lockout is already structural: inventory reservations are owner-scoped by
/// `(OwnerId, AuthorityGuardKind::Reservation)` and
/// `ReservationGuardValidator::is_current` answers `Ok(false)` for any guard
/// whose provider is not the book's owner, while `InventoryVerb` registers no
/// reserve or release verb at all — so there is no player-reachable path to
/// release a lot reservation.
///
/// This test is the tripwire that keeps a future `"market.listing"` entry from
/// being added without re-reading §15.5.
#[test]
fn vf_g2_engine_owned_reservations_is_a_money_table() {
    // E7 residue pass 1, R1 added the second entry — the work order's budget hold
    // on `WorkOrder::budget_account`, which is a MONEY hold and so belongs here.
    // The tripwire's point is unchanged: every entry must be a money hold, and a
    // lot reservation may never be named.
    // Doc 20 cut SM.3a added the third — the terrain vehicle's deposit on the
    // pilot's own account, a MONEY hold.
    // R8a adds the relay message's prepaid account reservation, also MONEY.
    assert_eq!(
        mush_economy::ENGINE_OWNED_RESERVATIONS,
        [
            "campaign.operation",
            "industry.workorder",
            "tacsim.tv.deposit",
            "relay.message"
        ],
        "four entries, all MONEY holds: the campaign operation's bond, the work \
         order's budget, the terrain vehicle's deposit and a relay message's prepaid fee. A lot reservation is a K4b \
         `ReservationRecord` and cannot be named here (10b §15.5, fold G2)"
    );
    for kind in mush_economy::ENGINE_OWNED_RESERVATIONS {
        assert!(
            !kind.contains("market") && !kind.contains("listing") && !kind.contains("lot"),
            "{kind} names the lot lane; §15.5 step (b′) says the table is not what \
             guards it"
        );
    }
    // The table's only matcher is over a money hold's purpose, which is what
    // makes the entry above the only kind of thing that can appear in it.
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/api.rs"),
    )
    .expect("mush-economy/src/api.rs");
    let boundary = src.find("#[cfg(test)]").unwrap_or(src.len());
    let production = &src[..boundary];
    let at = production
        .find("fn is_engine_owned_reservation")
        .expect("the matcher");
    assert!(
        production[at..at + 600].contains("HoldPurpose::Reservation"),
        "the table is matched against a money hold's purpose and nothing else"
    );
    assert_eq!(
        production.matches("ENGINE_OWNED_RESERVATIONS").count(),
        5,
        "the declaration, its doc references and the one matcher — no other \
         reader may bind the table to a non-money hold"
    );
}
