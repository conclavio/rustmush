//! **Doc 17f §9, acceptance story 2: private and derivative, on the production composition**
//! (doc 47 unit CO.6; doc 44 unit MW.5, item 1; doc 17g "C4 as built", row 2; 17f §10's
//! paid-derivative ruling).
//!
//! Ada files a mind. Bo holds an open licence of it, forks it, has the Registry rate the fork
//! and files it: privately, because an ordinary open licence gives no right to sell a
//! derivative. Bo forks it again under an open-resale licence, and that fork lists and Cy buys
//! it; the price pays Ada her royalty. The Registry revokes Bo's open-resale licence: Bo's own
//! use of the fork stops, and new sales of it stop, while Cy's licence, and the claim Cy has
//! open under it, go on as they were. Bo buys open-resale rights again and the fork sells
//! again, its ancestry and its terms as they were.
//!
//! The world is the combined M8 world (`m8_world`: the production `LaunchComposition`). The
//! story takes three genuine Registry ratings (Ada's mind and Bo's two forks; about 10 minutes
//! on conclav.dev), so the CI dispatch runs it. Two forks, because an open fork stays private
//! even when open-resale rights are later made its qualifying parent (the story shows that
//! refusal too), so the listable derivative is forked under the open-resale licence itself.
//! A fork is filed only once Bo has changed it: a filing whose source is another filing's is a
//! duplicate.
//!
//! **No player is staff.** Ada, Bo, Cy and Dee are characters on accounts of their own; every
//! act of theirs is a registered command typed by them and landed on the composed path, with
//! one exception, named here: **the replacement of a fork's qualifying parent licence**
//! (`roll:replace_owner_parent_qualification`) has no typed verb, so Bo takes it through its
//! signed offer, as a client's `+act` does (`M8World::perform`, the action-offer path; Bo is
//! the actor and the offer's principal). The revocation is the Registry operator's own verb,
//! `@registry/revoke` (a registered role the launch's runbook grants on the Registry's scope),
//! not a fiat. Each player pays from the wallet their Co-op charter opened: nothing is minted.
//!
//! **What staff do, and why** (none of it is under test):
//!
//! 1. *Two cores.* No ship carries a core and no route in play fits one (MW.13's cut note);
//!    staff bind Bo's and Cy's Drays to the Dray's governed Class II root
//!    (`ic_sortie_surface::fit`), so a licence can be sealed aboard and a claim opened.
//! 2. *The way to the counter.* The launch's counter is a room staff made with no door (the
//!    M8 rig's venue): staff carry each player there and back to the berth gallery; each
//!    leaves and boards their ship by their own verbs.
//! 3. *Their tanks and lockers*, stowed by staff's hand (`excursions::docked_dray`).
//!
//! **What the story does by hand.** It restarts the server over its kept store once.

use mush_core::GOD;
use serde_json::{Value, json};
use tacsim::definitions::core_replacement::SHIP_II;

use crate::c4_story_kit::{
    Player, acts, attempts, back_aboard, buys, filed_source, fires, held_by, keyed, listing_code,
    nobody_is_staff, nothing_landed, paid_term, saves_rates_and_files, says, seal, to_the_counter,
    unseal, world,
};
use crate::m8_world::M8World;
use crate::mw21_completion::{fits, number, wallet, warden, writes_rates_and_files};

/// The market's price, centicredits, and what of it is the Registry's (5%).
const PRICE_MINOR: i64 = 40_000;
const LEVY_MINOR: i64 = 2_000;

/// Ada's royalty on her open-resale listing, per mille, and what it is of the price.
const ROYALTY_PERMILLE: i64 = 100;
const ROYALTY_MINOR: i64 = PRICE_MINOR * ROYALTY_PERMILLE / 1_000;

/// The rows of `family` that name `licence` under `field`.
fn naming(world: &M8World, family: &str, field: &str, licence: u64) -> Vec<Value> {
    world
        .rows(family)
        .into_iter()
        .filter(|row| row.get(field).is_some() && number(row, field) == licence)
        .collect()
}

/// The Roll's claim book rows that name `hull`.
fn claims(world: &M8World, hull: mush_core::ObjId) -> Vec<Value> {
    world
        .rows("roll_mind_ic_claim")
        .into_iter()
        .filter(|row| {
            row["open"]["unit"] == json!(hull.0) || row["closed"]["unit"] == json!(hull.0)
        })
        .collect()
}

/// Whether `hull` has an open claim.
fn claim_open(world: &M8World, hull: mush_core::ObjId) -> bool {
    matches!(
        tacsim::ic_claim_of(world.rig.tw.world(), hull),
        tacsim::IcClaimRead::Active { .. }
    )
}

/// **Every record of the fork and its parent the story asserts is kept**, by family: the
/// filings, the owner terms and parent bindings, the paid terms and licences, the
/// qualification heads and the claims.
fn durable(world: &M8World) -> String {
    let mut out = String::new();
    for family in [
        "roll_mind_initial_filing",
        "roll_mind_filed",
        "roll_mind_owner_term",
        "roll_mind_owner_parent",
        "roll_mind_draft_parent",
        "roll_mind_owner_qualification_head",
        "roll_mind_paid_term",
        "roll_mind_personal_paid_licence",
        "roll_mind_ic_claim",
    ] {
        out.push_str(&format!("{family}: {:?}\n", keyed(world, family)));
    }
    out
}

/// **Bo replaces his fork's qualifying parent licence** with `parent`, through the action's
/// signed offer (the one act of the story with no typed verb; the module head).
fn requalifies(world: &mut M8World, bo: Player, owner_licence: u64, parent: u64) {
    let (_, outcome) = world.perform(
        "roll",
        "replace_owner_parent_qualification",
        bo.who,
        json!({"owner_licence": owner_licence, "parent_licence": parent}),
    );
    eprintln!("C4S Bo requalifies {owner_licence} on {parent}: {outcome}");
}

/// **Doc 17f §9 story 2: private and derivative** (the module head has what staff do).
///
/// 1. *Filed and unlisted.* Ada's filed mind has no form offered to anyone: the counter lists
///    nothing of it, and Bo cannot fork it from her owner's licence.
/// 2. *An ordinary open fork is private.* Bo buys an open licence of it, forks it, changes,
///    saves, has the fork rated and files it, his own owner's licence bound to that parent
///    grant, its source his to read; it cannot be listed in any form, and the counter shows
///    nothing of it.
/// 3. *An open-resale derivative lists and pays its parent chain.* Bo buys an open-resale
///    licence. Made the open fork's qualifying parent, it still does not list that fork. Bo
///    forks again under it, and that fork lists, marked derived; its terms name the parent
///    royalty. Cy buys it: Cy pays the price, Ada takes her royalty, Bo the rest less the
///    Registry's share, to the centicredit.
/// 4. *Parent revocation.* Cy seals the fork aboard; Bo seals his owner's licence aboard and
///    unseals it; Dee asks a quote. The Registry operator revokes Bo's open-resale licence.
///    Then: Dee's payment on that quote takes nothing and gives nothing; Bo's source is sealed
///    to him and his owner's seal is refused; Cy's claim is the claim it was, and Cy unseals
///    and seals again under the same licence and term.
/// 5. *A restart.* Every record above reads the same; Bo's seal is still refused, Cy's claim
///    still open.
/// 6. *Restoration.* Bo buys open-resale rights again and requalifies the fork. Its ancestry,
///    its owner's term and Cy's term are as they were: nothing is extended. Bo reads his
///    source again, and Dee buys the fork, the price paying Ada's royalty once more.
#[tokio::test(flavor = "current_thread")]
#[ignore = "CO.6 story 2: three genuine Registry ratings; the CI dispatch runs it"]
#[allow(clippy::too_many_lines)]
async fn c4_story_2_a_fork_is_private_its_resale_derivative_pays_its_parent_and_survives_revocation()
 {
    let (mut world, cast) = world(&[
        ("Bo", 1, "Salt Moth"),
        ("Ada", 2, "Ink Moth"),
        ("Cy", 3, "Glass Moth"),
        ("Dee", 4, "Paper Moth"),
    ]);
    let [bo, ada, cy, dee]: [Player; 4] = cast.clone().try_into().expect("four players");
    nobody_is_staff(&mut world, &cast);
    let operator = world
        .deployment
        .market
        .as_ref()
        .expect("the player market")
        .registry_authority
        .operators[0];
    let _ = world.rig.tw.connect(operator);
    // Fiat 1: the two cores.
    for player in [bo, cy] {
        crate::ic_sortie_surface::fit(&mut world.rig, player.hull, SHIP_II);
    }

    // Ada writes, rates and files her mind.
    to_the_counter(&mut world, ada.who);
    let (parent, parent_owner) =
        writes_rates_and_files(&mut world, ada.who, "Parent", &warden()).await;

    // 1. Filed and unlisted: nothing is offered.
    for player in [bo, cy, dee] {
        to_the_counter(&mut world, player.who);
    }
    let board = says(&mut world, bo.who, "+minds");
    fits(&board);
    assert!(
        !board.contains("parent"),
        "nothing of it is listed: {board}"
    );
    let (refused, landed) = attempts(
        &mut world,
        bo.who,
        &format!("@mind/fork {parent_owner}=Stolen"),
    );
    assert!(
        nothing_landed(&landed),
        "Bo cannot fork it from Ada's owner's license: {refused}"
    );
    assert!(
        world.rows("roll_mind_draft_parent").is_empty(),
        "and has no draft of it"
    );

    // Ada lists it, open and open-resale.
    for (form, said) in [
        ("open".to_owned(), "open"),
        (
            format!("open-resale royalty={ROYALTY_PERMILLE}"),
            "open-resale",
        ),
    ] {
        let listed = says(
            &mut world,
            ada.who,
            &format!("@mind/publish {parent} form={form}"),
        );
        assert!(
            listed.contains(&format!("Listing filed mind {parent} here as {said}.")),
            "{listed}"
        );
    }
    let open_code = listing_code(&mut world, bo.who, &["parent", " open "]);
    let resale_code = listing_code(&mut world, bo.who, &["parent", "open-resale"]);

    // 2. An ordinary open fork files privately and cannot list.
    let [open] = buys(&mut world, bo.who, &open_code)[..] else {
        panic!("Bo holds one open license")
    };
    let (forking, landed) = acts(&mut world, bo.who, &format!("@mind/fork {open}=private"));
    assert!(
        forking.contains(&format!("Forking license {open} as private.")) && landed.len() == 1,
        "{forking}"
    );
    let (private, private_owner) = saves_rates_and_files(
        &mut world,
        bo.who,
        "private",
        &["@mind/insert private 1=// Bo keeps this fork for himself"],
    )
    .await;
    assert_eq!(
        naming(&world, "roll_mind_owner_parent", "licence", private_owner).len(),
        1,
        "the open fork's owner's license names its parent grant"
    );
    assert!(
        filed_source(&mut world, bo.who, private_owner).is_ok(),
        "Bo reads his private fork's source"
    );
    for form in ["sealed", "open", "open-resale"] {
        let (refused, landed) = attempts(
            &mut world,
            bo.who,
            &format!("@mind/publish {private} form={form}"),
        );
        assert!(
            nothing_landed(&landed),
            "an ordinary open fork cannot be listed {form}: {refused}"
        );
    }
    let board = says(&mut world, cy.who, "+minds");
    assert!(
        !board.contains("private"),
        "nothing of the fork is listed: {board}"
    );

    // 3. An open-resale derivative lists and pays its parent.
    let [resale] = buys(&mut world, bo.who, &resale_code)[..] else {
        panic!("Bo holds one open-resale license")
    };
    // Open-resale rights bought later do not make the open fork listable.
    requalifies(&mut world, bo, private_owner, resale);
    let (refused, landed) = attempts(
        &mut world,
        bo.who,
        &format!("@mind/publish {private} form=open-resale royalty={ROYALTY_PERMILLE}"),
    );
    assert!(
        nothing_landed(&landed),
        "an open fork requalified on open-resale rights still cannot list: {refused}"
    );
    let (forking, landed) = acts(&mut world, bo.who, &format!("@mind/fork {resale}=child"));
    assert!(
        forking.contains(&format!("Forking license {resale} as child.")) && landed.len() == 1,
        "{forking}"
    );
    let (child, child_owner) = saves_rates_and_files(
        &mut world,
        bo.who,
        "child",
        &["@mind/insert child 1=// Bo forks the parent to sell it on"],
    )
    .await;
    let binding = naming(&world, "roll_mind_owner_parent", "licence", child_owner);
    assert_eq!(
        binding.len(),
        1,
        "the fork's owner's license names its parent grant"
    );
    let own_source = filed_source(&mut world, bo.who, child_owner)
        .unwrap_or_else(|refused| panic!("Bo reads his fork's source: {refused}"));
    let listed = says(
        &mut world,
        bo.who,
        &format!("@mind/publish {child} form=sealed"),
    );
    fits(&listed);
    assert!(
        listed.contains(&format!(
            "Listing filed mind {child} here as sealed; derived."
        )),
        "{listed}"
    );
    let child_code = listing_code(&mut world, cy.who, &["child", "derived"]);
    let terms = says(&mut world, cy.who, &format!("+minds {child_code}"));
    fits(&terms);
    assert!(terms.contains("parent royalties 10%"), "{terms}");
    let before = [ada, bo, cy].map(|p| wallet(&world, p.who));
    let [paid] = buys(&mut world, cy.who, &child_code)[..] else {
        panic!("Cy holds one license of the fork")
    };
    assert_eq!(
        [ada, bo, cy].map(|p| wallet(&world, p.who)),
        [
            before[0] + ROYALTY_MINOR,
            before[1] + PRICE_MINOR - LEVY_MINOR - ROYALTY_MINOR,
            before[2] - PRICE_MINOR,
        ],
        "Cy paid the price; Ada took her royalty and Bo the rest less the Registry's share"
    );
    let cy_term = paid_term(&world, paid, 1).expect("Cy's term");
    let owner_term = naming(&world, "roll_mind_owner_term", "licence", child_owner);
    assert_eq!(owner_term.len(), 1, "the fork's owner's term");

    // 4. The Registry revokes Bo's open-resale licence.
    back_aboard(&mut world, cy);
    let (_, sealed) = seal(&mut world, cy, paid);
    assert!(sealed, "Cy seals the fork aboard; the claim opens");
    let cy_claim = claims(&world, cy.hull);
    back_aboard(&mut world, bo);
    let (_, sealed) = seal(&mut world, bo, child_owner);
    assert!(sealed, "Bo's own use of his fork, before the loss");
    assert!(unseal(&mut world, bo), "and Bo unseals it");
    let (quoting, _) = acts(&mut world, dee.who, &format!("+minds/buy {child_code}"));
    let dee_before = wallet(&world, dee.who);
    let (revoking, landed) = acts(&mut world, operator, &format!("@registry/revoke {resale}"));
    assert!(
        revoking.contains(&format!("Revoking license {resale}.")) && landed.len() == 1,
        "{revoking}"
    );
    let (_, paying) = attempts(&mut world, dee.who, &format!("+minds/buy/pay {child_code}"));
    let (_, taking) = attempts(
        &mut world,
        dee.who,
        &format!("+minds/buy/complete {child_code}"),
    );
    assert!(
        held_by(&world, dee.who).is_empty() && wallet(&world, dee.who) == dee_before,
        "a quote asked before the loss buys nothing after it: {quoting} {paying:?} {taking:?}"
    );
    let sealed_source = filed_source(&mut world, bo.who, child_owner);
    assert!(sealed_source.is_err(), "Bo's source is sealed to him");
    let (refused, sealed) = seal(&mut world, bo, child_owner);
    assert!(!sealed, "Bo's own seal is refused: {refused}");
    assert!(!claim_open(&world, bo.hull));
    assert!(claim_open(&world, cy.hull), "Cy's claim stands");
    assert_eq!(claims(&world, cy.hull), cy_claim, "as it was");
    assert!(unseal(&mut world, cy), "Cy unseals");
    let (_, sealed) = seal(&mut world, cy, paid);
    assert!(sealed, "and seals again: an existing customer's new sortie");
    let cy_claim = claims(&world, cy.hull);
    assert_eq!(paid_term(&world, paid, 1).as_ref(), Some(&cy_term));

    // 5. A restart.
    let kept = durable(&world);
    let mut world = world.reboot_reconnecting(&[bo.who, ada.who, cy.who, dee.who, operator, GOD]);
    assert_eq!(durable(&world), kept, "the same records after the restart");
    assert!(claim_open(&world, cy.hull) && claims(&world, cy.hull) == cy_claim);
    let (refused, sealed) = seal(&mut world, bo, child_owner);
    assert!(!sealed, "Bo's own seal is still refused: {refused}");
    assert!(filed_source(&mut world, bo.who, child_owner).is_err());

    // 6. Restoration.
    to_the_counter(&mut world, bo.who);
    let [again] = buys(&mut world, bo.who, &resale_code)[..] else {
        panic!("Bo holds open-resale rights again")
    };
    requalifies(&mut world, bo, child_owner, again);
    assert_eq!(
        filed_source(&mut world, bo.who, child_owner),
        Ok(own_source),
        "Bo reads his fork's source again"
    );
    assert_eq!(
        naming(&world, "roll_mind_owner_term", "licence", child_owner),
        owner_term,
        "the fork's owner's term is not extended"
    );
    let ancestry: Vec<Value> = naming(&world, "roll_mind_owner_parent", "licence", child_owner);
    assert_eq!(
        ancestry, binding,
        "and its ancestry is the one it was filed with"
    );
    assert_eq!(
        paid_term(&world, paid, 1).as_ref(),
        Some(&cy_term),
        "nor is Cy's"
    );
    let before = [ada, bo, dee].map(|p| wallet(&world, p.who));
    let [bought] = buys(&mut world, dee.who, &child_code)[..] else {
        panic!("Dee buys the fork now")
    };
    assert_ne!(bought, paid);
    assert_eq!(
        [ada, bo, dee].map(|p| wallet(&world, p.who)),
        [
            before[0] + ROYALTY_MINOR,
            before[1] + PRICE_MINOR - LEVY_MINOR - ROYALTY_MINOR,
            before[2] - PRICE_MINOR,
        ],
        "and the sale pays Ada's royalty again"
    );
    fires(&mut world, 2);
    assert!(claim_open(&world, cy.hull), "Cy's claim is still open");
    let _ = parent;
}
