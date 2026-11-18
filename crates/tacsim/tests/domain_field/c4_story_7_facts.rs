//! **Doc 17f §9, acceptance story 7: the fact interface, on the production composition**
//! (doc 47 unit CO.6; doc 44 unit MW.5, item 1; doc 17g "C4 as built", row 7).
//!
//! A contract is how the world consumes a mind fact: the Economy's fact table is
//! crate-private, so every reading here is a contract with one `minds` condition, drafted,
//! offered and accepted by its two parties and settled (or not) by the Economy's own sweep.
//! The facts are the Roll's committed records: a filing, a licence's transfer receipt and a
//! term generation's lapse.
//!
//! The world is the combined M8 world (`m8_world`: the production `LaunchComposition`). The
//! story takes one genuine Registry rating, so the CI dispatch runs it.
//!
//! **No player is staff.** Ada writes, rates, files and lists a mind; Bo buys a licence of it
//! and resells it to Cy; Cy renews it after it lapsed. Every act of theirs, every contract
//! verb among them, is a registered command typed by a character on an account of their own
//! and landed on the composed path. Each pays from the wallet their Co-op charter opened:
//! nothing is minted.
//!
//! **What staff do, and why** (none of it is under test):
//!
//! 1. *Bo's core.* No ship carries a core and no route in play fits one (MW.13's cut note);
//!    staff bind Bo's Dray to the Dray's governed Class II root (`ic_sortie_surface::fit`), so
//!    the licence can be sealed aboard and a claim can hold the resale pending.
//! 2. *The way to the counter.* The launch's counter is a room staff made with no door (the
//!    M8 rig's venue): staff carry each player there, and Bo back to the berth gallery; Bo
//!    leaves and boards his ship by his own verbs.
//! 3. *Their tanks and lockers*, stowed by staff's hand (`excursions::docked_dray`).
//!
//! **What the story does by hand.** It restarts the server over its kept store (twice) and
//! moves the clock to the instants a term's Grace ends. A transfer fact's id is the
//! acceptance's commit token, a contract's record and never a player surface; the story reads
//! it from the acceptance it landed and the player types it.

use mush_core::GOD;
use tacsim::definitions::core_replacement::SHIP_II;

use crate::c4_story_kit::{
    Player, acts, attempts, back_aboard, buys, conditioned, contract, draft, fact, filed_key,
    fires, holder, keyed, licence_key, listing_code, nobody_is_staff, nothing_landed, paid_term,
    says, seal, settled, to_the_counter, unseal, until_settled, world,
};
use crate::m8_world::M8World;
use crate::mw21_completion::{fits, number, warden, writes_rates_and_files};

/// The resale's price, in credits as a player types it.
const RESALE_PRICE: &str = "5";

/// The lapse occurrence rows of `licence`'s generation `generation`.
fn occurrences(world: &M8World, licence: u64, generation: u64) -> Vec<serde_json::Value> {
    world
        .rows("roll_mind_lapse_occurrences")
        .into_iter()
        .filter(|row| {
            number(&row["term"], "licence") == licence
                && number(&row["term"], "generation") == generation
        })
        .collect()
}

/// **Every record story 7 reads, by family**: the filings, the transfer receipts, the lapse
/// occurrences, the paid terms and licences, and the contracts' states.
fn durable(world: &M8World, contracts: &[mush_economy::ContractId]) -> String {
    let mut out = String::new();
    for family in [
        "roll_mind_initial_filing",
        "roll_mind_filed",
        "market_mind_paid_resale_receipts",
        "roll_mind_paid_holders",
        "roll_mind_lapse_occurrences",
        "roll_mind_paid_term",
        "roll_mind_personal_paid_licence",
    ] {
        out.push_str(&format!("{family}: {:?}\n", keyed(world, family)));
    }
    for id in contracts {
        let c = crate::excursions::economy(&world.rig.tw)
            .snapshot()
            .contract(*id)
            .map(|c| (c.state, c.satisfied.clone()));
        out.push_str(&format!("contract {id}: {c:?}\n"));
    }
    out
}

/// Fire until `done` reads true, at most `bound` fires; whether it did.
fn until(world: &mut M8World, bound: usize, mut done: impl FnMut(&M8World) -> bool) -> bool {
    for _ in 0..bound {
        if done(world) {
            return true;
        }
        fires(world, 1);
    }
    done(world)
}

/// **Doc 17f §9 story 7: the fact interface** (the module head has what staff do).
///
/// 1. *The filing fact.* Ada's filing is a public fact the moment it commits: a contract Cy
///    draws on it with Ada settles in the Economy's sweep.
/// 2. *A foreign caller.* Cy, who holds nothing of the licence Bo bought, is refused its
///    lapse fact with the very line a licence that does not exist gets, and neither refusal
///    attaches anything. Bo, its holder, attaches it.
/// 3. *Before the receipt.* Bo seals the licence aboard, so the claim it opens holds a resale
///    pending. Cy's acceptance at a price Bo did not ask is refused and submits nothing. At
///    Bo's price it is funded and pending: a contract on its transfer fact stays Active
///    through the sweeps, and through a restart, while the licence is Bo's.
/// 4. *The receipt.* Bo unseals; the claim closes, the resale settles, the licence is Cy's,
///    and the contract settles. The wrong holder generation is not that fact: it attaches to
///    nothing, with the line a missing reference gets.
/// 5. *The lapse.* Cy's contract on the licence's first generation lapsing stays Active two
///    days before Grace ends, with no occurrence on record; at the end of Grace the Roll
///    records the lapse and the contract settles.
/// 6. *A renewal after it.* Ada attests her own owner's licence inside its Grace (a paid
///    licence of an original line renews only while its seller's owner's licence stands:
///    without the attestation Cy's renewal is refused). Cy renews the lapsed licence by
///    `+minds/renew` and its `/pay`. Its second generation is on record
///    beside the first; the lapse occurrence is the row it was; a new contract on the first
///    generation's lapse settles, and one on the second's does not.
/// 7. *The same facts after a restart.* Every record above reads the same, the contracts are
///    in the states they were, and a new contract on the filing settles.
#[tokio::test(flavor = "current_thread")]
#[ignore = "CO.6 story 7: one genuine Registry rating; the CI dispatch runs it"]
#[allow(clippy::too_many_lines)]
async fn c4_story_7_minds_facts_settle_contracts_on_the_launch_and_survive_a_restart() {
    let (mut world, cast) = world(&[
        ("Bo", 1, "Salt Moth"),
        ("Ada", 2, "Ink Moth"),
        ("Cy", 3, "Glass Moth"),
    ]);
    let [bo, ada, cy]: [Player; 3] = cast.clone().try_into().expect("three players");
    nobody_is_staff(&mut world, &cast);
    // Fiat 1: Bo's core.
    crate::ic_sortie_surface::fit(&mut world.rig, bo.hull, SHIP_II);

    // Ada writes, rates and files her mind, from her own wallet.
    to_the_counter(&mut world, ada.who);
    let (filed, _) = writes_rates_and_files(&mut world, ada.who, "Ledger", &warden()).await;

    // 1. The filing fact, public.
    to_the_counter(&mut world, cy.who);
    let on_filing = conditioned(
        &mut world,
        (cy.who, ada.who),
        "Ada filed",
        &fact("filed", &filed_key(filed), 1),
        100,
    );
    assert!(
        until_settled(&mut world, on_filing, 20),
        "the filing fact settles Cy's contract: {:?}",
        contract(&world, on_filing)
    );

    // Ada lists it; Bo buys a licence.
    let listed = says(
        &mut world,
        ada.who,
        &format!("@mind/publish {filed} form=sealed"),
    );
    fits(&listed);
    assert!(
        listed.contains(&format!("Listing filed mind {filed} here as sealed.")),
        "{listed}"
    );
    to_the_counter(&mut world, bo.who);
    let code = listing_code(&mut world, bo.who, &["ledger", "sealed"]);
    let [licence] = buys(&mut world, bo.who, &code)[..] else {
        panic!("Bo holds one new license")
    };
    back_aboard(&mut world, bo);
    let lapse_1 = fact("licence.lapsed", &licence_key(licence), 1);

    // 2. A foreign caller cannot tell a private licence from a missing one.
    let foreign = draft(&mut world, cy.who, "Not my license");
    let before = contract(&world, foreign);
    let private = says(
        &mut world,
        cy.who,
        &format!("@contract/condition {foreign}={lapse_1}"),
    );
    let missing = says(
        &mut world,
        cy.who,
        &format!(
            "@contract/condition {foreign}={}",
            fact("licence.lapsed", "licence/ffffffffffffffff", 1)
        ),
    );
    assert_eq!(
        private, missing,
        "another holder's license and none at all read alike"
    );
    assert_eq!(
        contract(&world, foreign),
        before,
        "and neither attached anything"
    );
    let own = draft(&mut world, bo.who, "My own license");
    let attached = says(
        &mut world,
        bo.who,
        &format!("@contract/condition {own}={lapse_1}"),
    );
    let mine = contract(&world, own);
    assert!(
        mine.consent.conditions.len() == 1 && mine.consent.fact_consent.is_some(),
        "its holder attaches it, with the subject's consent: {attached}"
    );
    let _ = says(&mut world, bo.who, &format!("@contract/cancel {own}"));

    // 3. Before the receipt: the resale pending under Bo's open claim.
    let (_, sealed) = seal(&mut world, bo, licence);
    assert!(sealed, "Bo seals the license aboard and the claim opens");
    let (offered, staged) = acts(
        &mut world,
        bo.who,
        &format!("+minds/resell {licence}=Cy price={RESALE_PRICE}"),
    );
    assert_eq!(staged.len(), 1, "Bo offers it to Cy: {offered}");
    let (refused, staged) = acts(
        &mut world,
        cy.who,
        &format!("+minds/resell/accept {licence} price=4"),
    );
    assert!(
        staged.is_empty(),
        "an acceptance at a price Bo did not ask submits nothing: {refused}"
    );
    assert!(
        world.rows("market_mind_paid_resale_receipts").is_empty(),
        "and leaves no receipt"
    );
    let (accepted, staged) = acts(
        &mut world,
        cy.who,
        &format!("+minds/resell/accept {licence} price={RESALE_PRICE}"),
    );
    let [acceptance] = staged[..] else {
        panic!("Cy's acceptance is one submission: {accepted}")
    };
    assert_eq!(holder(&world, licence), u64::from(bo.who.0), "still Bo's");
    let transferred = fact("licence.transferred", &acceptance.to_hex(), 2);
    let on_transfer = conditioned(
        &mut world,
        (bo.who, cy.who),
        "Cy took it",
        &transferred,
        100,
    );
    fires(&mut world, 12);
    assert!(
        !settled(&world, on_transfer),
        "a pending transfer is not a completed one: {:?}",
        contract(&world, on_transfer)
    );
    assert_eq!(holder(&world, licence), u64::from(bo.who.0));

    // A restart while it is pending.
    let pending = durable(&world, &[on_filing, on_transfer]);
    let mut world = world.reboot_reconnecting(&[bo.who, ada.who, cy.who, GOD]);
    assert_eq!(durable(&world, &[on_filing, on_transfer]), pending);
    assert!(
        matches!(
            tacsim::ic_claim_of(world.rig.tw.world(), bo.hull),
            tacsim::IcClaimRead::Active { .. }
        ),
        "Bo's claim is open after the restart"
    );
    fires(&mut world, 6);
    assert!(!settled(&world, on_transfer), "still pending");

    // 4. The receipt.
    assert!(unseal(&mut world, bo), "Bo unseals at his berth");
    assert!(
        until(&mut world, 40, |world| holder(world, licence)
            == u64::from(cy.who.0)),
        "the resale settles to Cy once the claim has closed"
    );
    assert_eq!(
        world.rows("market_mind_paid_resale_receipts").len(),
        1,
        "one receipt"
    );
    assert!(
        until_settled(&mut world, on_transfer, 20),
        "the receipt settles the contract on it: {:?}",
        contract(&world, on_transfer)
    );
    // The wrong holder generation is not that fact: it attaches to nothing, with the line a
    // contract that does not exist gets.
    let wrong = draft(&mut world, bo.who, "Cy took it twice");
    let unattached = contract(&world, wrong);
    let refused = says(
        &mut world,
        bo.who,
        &format!(
            "@contract/condition {wrong}={}",
            fact("licence.transferred", &acceptance.to_hex(), 3)
        ),
    );
    assert_eq!(contract(&world, wrong), unattached, "{refused}");
    assert_eq!(
        refused, missing,
        "it reads as a reference that does not exist"
    );

    // 5. The lapse, Cy's now.
    let on_lapse = conditioned(&mut world, (cy.who, bo.who), "It lapsed", &lapse_1, 100);
    let grace_end = paid_term(&world, licence, 1)
        .expect("the first term")
        .window()
        .grace_end()
        .get();
    crate::c4_story_kit::to_ic(&mut world, grace_end - 2 * 86_400);
    fires(&mut world, 2);
    assert!(
        crate::excursions::ic_now(&world.rig.tw) < grace_end,
        "still inside Grace"
    );
    assert!(
        occurrences(&world, licence, 1).is_empty() && !settled(&world, on_lapse),
        "no lapse yet: {:?}",
        contract(&world, on_lapse)
    );
    // Ada keeps her own licence of her mind: its term ended before the one Cy holds, and she
    // attests it inside its Grace.
    let (attested, landed) = attempts(&mut world, ada.who, &format!("@mind/attest {filed}"));
    fires(&mut world, 2);
    assert!(
        !nothing_landed(&landed),
        "Ada attests her license: {attested}"
    );
    crate::c4_story_kit::to_ic(&mut world, grace_end + 60);
    let recorded = until(&mut world, 40, |world| {
        !occurrences(world, licence, 1).is_empty()
    });
    eprintln!(
        "C4S lapse: ic {} grace_end {grace_end} occurrences {:?}",
        crate::excursions::ic_now(&world.rig.tw),
        world.rows("roll_mind_lapse_occurrences"),
    );
    assert!(recorded, "the Roll records the lapse at the end of Grace");
    let occurrence = occurrences(&world, licence, 1);
    assert!(
        until_settled(&mut world, on_lapse, 20),
        "the lapse settles Cy's contract: {:?}",
        contract(&world, on_lapse)
    );

    // 6. A renewal after the lapse.
    let asked = says(&mut world, cy.who, &format!("+minds/renew {licence}"));
    fits(&asked);
    // The renewal service takes the step on its own schedule.
    fires(&mut world, 4);
    let offer = says(&mut world, cy.who, &format!("+minds/renew {licence}"));
    fits(&offer);
    let paying = says(&mut world, cy.who, &format!("+minds/renew/pay {licence}"));
    assert!(
        until(&mut world, 40, |world| paid_term(world, licence, 2)
            .is_some()),
        "the renewal grants a second term: {asked} / {offer} / {paying}"
    );
    assert_eq!(
        occurrences(&world, licence, 1),
        occurrence,
        "the renewal leaves the lapse on record"
    );
    let again = conditioned(
        &mut world,
        (cy.who, bo.who),
        "It lapsed once",
        &lapse_1,
        100,
    );
    let on_second = conditioned(
        &mut world,
        (cy.who, bo.who),
        "It lapsed twice",
        &fact("licence.lapsed", &licence_key(licence), 2),
        100,
    );
    assert!(
        until_settled(&mut world, again, 20),
        "the first generation's lapse still settles a contract"
    );
    assert!(
        !settled(&world, on_second),
        "the second generation has not lapsed"
    );

    // 7. A restart, and the same facts.
    // (The draft the wrong generation was refused on is not among them: a Draft lapses.)
    let every = [on_filing, on_transfer, on_lapse, again, on_second];
    let kept = durable(&world, &every);
    eprintln!("C4S story 7 durable before the second restart: {kept}");
    let mut world = world.reboot_reconnecting(&[bo.who, ada.who, cy.who, GOD]);
    assert_eq!(durable(&world, &every), kept, "the same records after it");
    let after = conditioned(
        &mut world,
        (cy.who, ada.who),
        "Ada filed, after",
        &fact("filed", &filed_key(filed), 1),
        100,
    );
    assert!(
        until_settled(&mut world, after, 20),
        "the filing fact settles a new contract after the restart"
    );
    fires(&mut world, 6);
    assert!(
        !settled(&world, on_second),
        "the second generation has still not lapsed"
    );
    assert_eq!(
        occurrences(&world, licence, 1),
        occurrence,
        "and the lapse is the one recorded"
    );
}
