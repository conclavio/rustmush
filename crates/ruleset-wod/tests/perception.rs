//! M11 Obfuscate-vs-Auspex perception vertical, driven through the real
//! [`Engine`](mush_core) via [`mush_testkit::TestWorld`].
//!
//! Everyone shares one room (Limbo). A vampire cloaks with `+obfuscate on`;
//! from that instant a watcher perceives them only if they rolled well enough
//! (Perception + Awareness + Auspex) to pierce the veil — and a watcher with no
//! Auspex never can. The gate is proven on every leak path the milestone names:
//! the room `look`, live `say` fan-out (via [`TestWorld::seen_by`], which
//! mirrors the world task's projection), and `examine`'s raw contents. De-cloak
//! restores visibility, and a re-cloak leaves no stale verdict behind.
//!
//! Dice are deterministic (fixed RNG seed). Piercing scenarios use an
//! overwhelming pool at low difficulty so the outcome is stable across the
//! stream position; the *cannot-pierce* cases are auto-fails that spend no dice
//! at all.
//!
//! NOTE (deviation from the brief's wording): Auspex is a Discipline, so it
//! lives on the vampire/ghoul `disciplines` map — a mage's schema has no such
//! field. The piercing observer here is therefore a Kindred seer (and, for the
//! NPC, a ghoul), not a "mage"; the mechanic (a preternatural sense pierces the
//! veil, mundane senses do not) is exactly as specified.

use mush_core::{GOD, LIMBO, ObjId, ObjKind, can_perceive, is_concealed};
use mush_testkit::TestWorld;
use mush_jobs::JobsModule;
use ruleset_wod::WodRuleset;
use ruleset_wod::state::WodState;

const S: u64 = 12345;

// ----------------------------------------------------------------------
// Harness
// ----------------------------------------------------------------------

fn world() -> TestWorld {
    TestWorld::with_modules(vec![
        Box::new(JobsModule::new()),
        Box::new(WodRuleset::with_seed(S)),
    ])
}

fn set(tw: &mut TestWorld, cmd: &str) {
    tw.run(GOD, cmd);
}

/// Make `name` a vampire with `obfuscate` dots of Obfuscate.
fn make_vampire(tw: &mut TestWorld, name: &str, obfuscate: i32) -> ObjId {
    let id = tw.spawn_player(name);
    set(tw, &format!("@sheet/set {name}/template=vampire"));
    set(
        tw,
        &format!("@sheet/set {name}/disciplines:Obfuscate={obfuscate}"),
    );
    id
}

/// Make `name` a Kindred seer: perception/awareness 5 and `auspex` dots of
/// Auspex — an overwhelming pool so a successful pierce is stable.
fn make_seer(tw: &mut TestWorld, name: &str, auspex: i32) -> ObjId {
    let id = tw.spawn_player(name);
    set(tw, &format!("@sheet/set {name}/template=vampire"));
    set(tw, &format!("@sheet/set {name}/perception=5"));
    set(tw, &format!("@sheet/set {name}/awareness=5"));
    set(tw, &format!("@sheet/set {name}/disciplines:Auspex={auspex}"));
    id
}

/// A plain mortal with keen mundane senses but no Auspex.
fn make_mortal(tw: &mut TestWorld, name: &str) -> ObjId {
    let id = tw.spawn_player(name);
    set(tw, &format!("@sheet/set {name}/perception=5"));
    set(tw, &format!("@sheet/set {name}/awareness=5"));
    id
}

fn concealed(tw: &TestWorld, who: ObjId) -> bool {
    tw.world()
        .module_state::<WodState>()
        .expect("WodState")
        .is_concealed(who)
}

fn verdict(tw: &TestWorld, observer: ObjId, subject: ObjId) -> Option<bool> {
    tw.world()
        .module_state::<WodState>()
        .expect("WodState")
        .perception_verdict(observer, subject)
}

// ----------------------------------------------------------------------
// The vertical
// ----------------------------------------------------------------------

#[test]
fn look_and_speech_are_gated_by_obfuscate() {
    let mut tw = world();
    let vlad = make_vampire(&mut tw, "Vlad", 2);
    let mort = make_mortal(&mut tw, "Mort");
    let seer = make_seer(&mut tw, "Seer", 5);

    // Before cloaking: everyone perceives Vlad, and Vlad appears in a look.
    let ev = tw.run(mort, "look");
    assert!(
        tw.seen_by(&ev, mort).iter().any(|l| l.contains("Vlad")),
        "Vlad is plainly visible before cloaking"
    );

    // Cloak. Every present observer rolls once, now.
    tw.run(vlad, "+obfuscate on");
    assert!(concealed(&tw, vlad));
    // The mortal has no Auspex: an auto-fail, no dice, definitively concealed.
    assert_eq!(verdict(&tw, mort, vlad), Some(false));
    // The seer's Auspex pierced the veil (Auspex 5 + Per/Awa 5 vs a low diff).
    assert_eq!(
        verdict(&tw, seer, vlad),
        Some(true),
        "a high-Auspex seer pierces Obfuscate"
    );

    // LOOK: the mortal no longer sees Vlad; the seer still does.
    let ev = tw.run(mort, "look");
    assert!(
        !tw.seen_by(&ev, mort).iter().any(|l| l.contains("Vlad")),
        "the mortal cannot see the cloaked vampire"
    );
    let ev = tw.run(seer, "look");
    assert!(
        tw.seen_by(&ev, seer).iter().any(|l| l.contains("Vlad")),
        "the seer still sees the cloaked vampire"
    );

    // SAY fan-out: the mortal does not overhear Vlad; the seer does; Vlad
    // always hears himself.
    let ev = tw.run(vlad, "say The night is mine.");
    assert!(
        !tw.seen_by(&ev, mort)
            .iter()
            .any(|l| l.contains("night is mine")),
        "the mortal does not overhear the unseen speaker"
    );
    assert!(
        tw.seen_by(&ev, seer)
            .iter()
            .any(|l| l.contains("night is mine")),
        "the seer overhears the speaker they perceive"
    );
    assert!(
        tw.seen_by(&ev, vlad)
            .iter()
            .any(|l| l.contains("night is mine")),
        "a cloaked speaker always hears their own speech"
    );
}

#[test]
fn no_auspex_never_pierces_however_keen_the_mundane_senses() {
    let mut tw = world();
    let vlad = make_vampire(&mut tw, "Vlad", 1); // the thinnest possible veil
    let mort = make_mortal(&mut tw, "Mort"); // Perception 5 + Awareness 5, no Auspex

    tw.run(vlad, "+obfuscate on");
    // Auto-fail: no Auspex, so no dice were even rolled.
    assert_eq!(verdict(&tw, mort, vlad), Some(false));
    let ev = tw.run(vlad, "say You cannot hear me.");
    assert!(
        !tw.seen_by(&ev, mort)
            .iter()
            .any(|l| l.contains("cannot hear me")),
        "mundane senses alone never pierce Obfuscate"
    );
}

#[test]
fn examine_contents_do_not_leak_a_cloaked_occupant() {
    let mut tw = world();
    let vlad = make_vampire(&mut tw, "Vlad", 3);

    // GOD controls Limbo, so `examine here` dumps its raw contents — a leak path
    // the gate must also close. GOD has no Auspex, so cannot perceive Vlad.
    tw.run(vlad, "+obfuscate on");
    let ev = tw.run(GOD, "examine here");
    assert!(
        !tw.seen_by(&ev, GOD).iter().any(|l| l.contains("Vlad")),
        "examine must not leak the cloaked occupant"
    );

    // De-cloak: examine lists Vlad again.
    tw.run(vlad, "+obfuscate off");
    let ev = tw.run(GOD, "examine here");
    assert!(
        tw.seen_by(&ev, GOD).iter().any(|l| l.contains("Vlad")),
        "examine lists the occupant once revealed"
    );
}

#[test]
fn decloak_restores_visibility() {
    let mut tw = world();
    let vlad = make_vampire(&mut tw, "Vlad", 3);
    let mort = make_mortal(&mut tw, "Mort");

    tw.run(vlad, "+obfuscate on");
    let ev = tw.run(mort, "look");
    assert!(!tw.seen_by(&ev, mort).iter().any(|l| l.contains("Vlad")));

    tw.run(vlad, "+obfuscate off");
    assert!(!concealed(&tw, vlad));
    // De-cloak drops the pierced verdicts about Vlad entirely.
    assert_eq!(verdict(&tw, mort, vlad), None);
    let ev = tw.run(mort, "look");
    assert!(
        tw.seen_by(&ev, mort).iter().any(|l| l.contains("Vlad")),
        "the mortal sees Vlad again after he reappears"
    );
}

#[test]
fn recloak_leaves_no_stale_verdict() {
    let mut tw = world();
    let vlad = make_vampire(&mut tw, "Vlad", 2);
    let seer = make_seer(&mut tw, "Seer", 5);

    // First cloak: the seer pierces.
    tw.run(vlad, "+obfuscate on");
    assert_eq!(verdict(&tw, seer, vlad), Some(true));

    // Reveal wipes the verdict — no residue to go stale.
    tw.run(vlad, "+obfuscate off");
    assert_eq!(
        verdict(&tw, seer, vlad),
        None,
        "reveal clears the pierced cache for the subject"
    );

    // Re-cloak rolls afresh (again a guaranteed pierce for this seer).
    tw.run(vlad, "+obfuscate on");
    assert_eq!(
        verdict(&tw, seer, vlad),
        Some(true),
        "a re-cloak re-rolls; the verdict is present and fresh"
    );
}

#[test]
fn is_concealed_query_tracks_active_obfuscate() {
    // The ruleset-agnostic notability query (D1): false for a subject in plain
    // sight, true while they are actively Obfuscated, false again once revealed.
    // Unlike `can_perceive`, it is observer-independent.
    let mut tw = world();
    let vlad = make_vampire(&mut tw, "Vlad", 2);
    let _mort = make_mortal(&mut tw, "Mort");

    assert!(
        !is_concealed(tw.registry(), tw.world(), vlad),
        "a subject in plain sight is not concealed"
    );

    tw.run(vlad, "+obfuscate on");
    assert!(
        is_concealed(tw.registry(), tw.world(), vlad),
        "an actively Obfuscated subject reads as concealed"
    );

    tw.run(vlad, "+obfuscate off");
    assert!(
        !is_concealed(tw.registry(), tw.world(), vlad),
        "revealing drops the concealment signal"
    );
}

#[test]
fn a_ghoul_bartender_reads_the_same_way_as_a_player() {
    // An NPC is just another occupant: a ghoul bartender with Auspex is rolled
    // by the same cache maintenance and answers `can_perceive` identically, so
    // the NPC-observer gate in mush-npc reads a real verdict.
    let mut tw = world();
    let vlad = make_vampire(&mut tw, "Vlad", 2);

    // A ghoul bartender (Auspex) and a plain human bouncer (no Auspex), both
    // NPCs present in the room.
    let barkeep = tw
        .world_mut()
        .create(ObjKind::Npc, "Barkeep", GOD, Some(LIMBO))
        .unwrap();
    let bouncer = tw
        .world_mut()
        .create(ObjKind::Npc, "Bouncer", GOD, Some(LIMBO))
        .unwrap();
    set(&mut tw, "@sheet/set Barkeep/template=ghoul");
    set(&mut tw, "@sheet/set Barkeep/perception=5");
    set(&mut tw, "@sheet/set Barkeep/awareness=5");
    set(&mut tw, "@sheet/set Barkeep/disciplines:Auspex=5");
    set(&mut tw, "@sheet/set Bouncer/perception=5");
    set(&mut tw, "@sheet/set Bouncer/awareness=5");

    tw.run(vlad, "+obfuscate on");

    let reg = tw.registry();
    let w = tw.world();
    assert!(
        can_perceive(reg, w, barkeep, vlad),
        "the Auspex ghoul bartender pierces the veil"
    );
    assert!(
        !can_perceive(reg, w, bouncer, vlad),
        "the mundane bouncer cannot perceive the cloaked vampire"
    );
}

#[test]
fn no_concealment_advances_no_rng_and_hides_nobody() {
    // I1/I4: with nobody cloaked, the perception observer never touches the RNG
    // during spawns/connects, and the gate hides no one — so a subsequent roll
    // reproduces the pristine seeded stream exactly as before M11.
    let mut tw = world();
    let nick = tw.spawn_player("Nick");
    let _bob = tw.spawn_player("Bob");
    set(&mut tw, "@sheet/set Nick/strength=3");
    set(&mut tw, "@sheet/set Nick/brawl=2");

    // The first real roll must consume the stream from offset 0.
    use mush_dice::owod20::{Owod20Opts, Owod20Outcome, Owod20Pool};
    use rand::{RngCore, SeedableRng};
    use rand_chacha::ChaCha8Rng;
    let mut rng = ChaCha8Rng::seed_from_u64(S);
    let faces: Vec<u8> = (0..5).map(|_| (rng.next_u32() % 10) as u8 + 1).collect();
    let expected = Owod20Pool::score(&faces, &Owod20Opts::new(6));

    tw.run(nick, "+roll strength+brawl");
    let got: Owod20Outcome = tw
        .world()
        .module_state::<WodState>()
        .unwrap()
        .recent_logs(nick, 1)
        .first()
        .unwrap()
        .outcome
        .clone();
    assert_eq!(
        got, expected,
        "no concealment ⇒ the perception observer drew no dice"
    );
}

#[test]
fn movement_arrival_is_gated_for_a_cloaked_mover() {
    // I2 ("among arrival"): a cloaked vampire's arrival echo must be gated
    // per-observer, not fanned out to everyone. This line rides an
    // *unattributed* narrative (`actor: None`); the mover travels in the
    // routing layer (`Audience::Room::subject`) so the fan-out still gates it
    // while the wire event stays unchanged.
    let mut tw = world();
    let vlad = make_vampire(&mut tw, "Vlad", 3);
    let mort = make_mortal(&mut tw, "Mort"); // waits in the Cellar
    let seer = make_seer(&mut tw, "Seer", 5); // waits in the Cellar, will pierce
    let cellar = tw
        .world_mut()
        .create(ObjKind::Room, "Cellar", GOD, None)
        .unwrap();
    set(&mut tw, &format!("@open down={cellar}"));
    tw.world_mut().move_to(mort, cellar).unwrap();
    tw.world_mut().move_to(seer, cellar).unwrap();

    // Cloak in Limbo, then walk down into the Cellar.
    tw.run(vlad, "+obfuscate on");
    let arrival = tw.run(vlad, "down");

    // The Moved maintenance rolled both watchers against Vlad in the Cellar.
    assert_eq!(verdict(&tw, mort, vlad), Some(false));
    assert_eq!(verdict(&tw, seer, vlad), Some(true));

    // The mortal never hears it; the seer does (per-observer, not globally
    // suppressed).
    let mort_saw = tw.seen_by(&arrival, mort);
    assert!(
        !mort_saw.iter().any(|l| l.contains("Vlad") || l.contains("arrived")),
        "arrival of a cloaked mover leaked to the mortal: {mort_saw:?}"
    );
    assert!(
        tw.seen_by(&arrival, seer)
            .iter()
            .any(|l| l.contains("Vlad") && l.contains("arrived")),
        "the piercing seer should still perceive the arrival"
    );
}

#[test]
fn movement_departure_is_gated_for_a_cloaked_mover() {
    // The departure echo ("goes down.") is emitted in the room the mover LEAVES.
    // A mortal left behind who cannot perceive the cloaked mover must not hear
    // it (fail-safe: the mover's verdicts for the old room are cleared as it
    // relocates, and it is still concealed, so the line is withheld).
    let mut tw = world();
    let vlad = make_vampire(&mut tw, "Vlad", 3);
    let mort = make_mortal(&mut tw, "Mort"); // stays in Limbo, the origin
    let cellar = tw
        .world_mut()
        .create(ObjKind::Room, "Cellar", GOD, None)
        .unwrap();
    set(&mut tw, &format!("@open down={cellar}"));

    tw.run(vlad, "+obfuscate on");
    assert_eq!(verdict(&tw, mort, vlad), Some(false));
    let departure = tw.run(vlad, "down");
    let mort_saw = tw.seen_by(&departure, mort);
    assert!(
        !mort_saw.iter().any(|l| l.contains("Vlad") || l.contains("goes")),
        "departure of a cloaked mover leaked to the mortal: {mort_saw:?}"
    );
}

#[test]
fn direct_look_and_examine_cannot_probe_a_cloaked_subject() {
    // I2 ("via look, examine"): resolving a concealed subject by name or dbref
    // must not render it. Gating only the room listing / container contents
    // left `look <name>` / `examine <name>` as a one-line probe; both now treat
    // an imperceptible target as a no-match (true silence).
    let mut tw = world();
    let vlad = make_vampire(&mut tw, "Vlad", 3);
    let mort = make_mortal(&mut tw, "Mort");
    tw.run(vlad, "+obfuscate on");
    assert_eq!(verdict(&tw, mort, vlad), Some(false));

    // `look Vlad`: no description render, no name echo.
    let ev = tw.run(mort, "look Vlad");
    let seen = tw.seen_by(&ev, mort);
    assert!(
        !seen.iter().any(|l| l.contains("nothing special") || l.contains(" -- ")),
        "direct look leaked the cloaked subject: {seen:?}"
    );

    // `examine Vlad` — even for GOD, a controller with no piercing sense.
    let ev = tw.run(GOD, "examine Vlad");
    let seen = tw.seen_by(&ev, GOD);
    assert!(
        !seen.iter().any(|l| l.contains("owned by") || l.contains("Vlad(")),
        "direct examine leaked the cloaked subject: {seen:?}"
    );

    // De-cloak restores the ordinary probe.
    tw.run(vlad, "+obfuscate off");
    let ev = tw.run(mort, "look Vlad");
    assert!(
        tw.seen_by(&ev, mort).iter().any(|l| l.contains("Vlad")),
        "look resolves the subject once revealed"
    );
}
