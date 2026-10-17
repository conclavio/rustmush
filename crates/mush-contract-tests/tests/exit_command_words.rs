//! **16c `B-11`: the shipped exits that take a module's verb are counted, not
//! ignored** (16b cut D.5).
//!
//! Dispatch matches exits at step 5 and the command table at step 6, so an exit
//! named for a verb is that verb gone in that room. Pack apply refuses one that
//! takes a word the **kernel or the host** registered — `l`, `home`, `inv` —
//! because those work in every room on every composition and no room may take
//! one.
//!
//! A **module's** verb is let through, on cut F.4's ruling: both shipped bus
//! exits are named `Board the Bus <BUS>;bus;board` on purpose, `board` is
//! `tacsim`'s own verb, and `CockpitGate` was taught to stand aside outside the
//! simulator centre precisely so a newcomer at the Bus Stop who types `board`
//! gets on the bus. A module's verb is contextual; its owner gates where it
//! means anything.
//!
//! That leniency is what this file makes visible. It walks every `;`-separated
//! segment of every shipped exit — the starter lot's three packs and, since doc 18
//! cut IC1, the trade run's four station packs
//! (`tacsim::definitions::content::station_packs`), whose Limbo door `c` and
//! twenty-three room exits boot on every production world with `[tacsim].enabled`,
//! and, since doc 18 cut IC8, the Dray's interior exits (laid up in every programme
//! hull; IC4's C7, which is why the Airlock's `lock` alias went at Dray v2)
//! — against every verb a **module** registered in
//! the production composition, and pins the overlap to the exact two rows F.4
//! decided to keep. The next one — an exit called `sim` in the Simulator Lobby
//! would swallow `SIM` — fails here and is ruled on by a person, which is the
//! protection the blanket rule was reaching for without refusing content that
//! was decided on.
//!
//! The kernel half is asserted here too, and for a reason worth writing down:
//! `Registry::kernel_command_named` answers for every entry with `module:
//! None`, which is the core command set **and** the host's own verbs
//! (`mush_server::staff::register_server_commands`). `mush-atlas`'s
//! `no_shipped_exit_segment_is_a_word_the_kernel_owns` composes that crate's
//! own modules, whose registry has no host verb in it at all — so nothing there
//! can see a shipped exit segment collide with one. Today nothing does only
//! because every host verb is `@`-prefixed; a bare one (`stop`, `out` and
//! `hall` are all plausible operator words, and all three are live exit
//! segments) would refuse the starter lot at a fresh production boot, which
//! `mush_content` logs and does not fail on. The second case below is the gate
//! that sees it.

use std::collections::BTreeSet;

use mush_atlas::def::ExitDef;
use mush_atlas::starter;

mod production_registry;

use production_registry::production_composition;

/// The overlap that is allowed to exist, as `(entry id, segment, verb)`.
///
/// Each row is a decision, and the reason is in the comment beside it. A row
/// added here without one is a defect.
const DECIDED: [(&str, &str, &str); 2] = [
    // F.4: the bus's own word at the Bus Stop. `BOARD` means something only in a
    // team room, and no exit in a team room is called that.
    ("exit.bus_stop_bus", "board", "board"),
    // The same exit's twin in the staff lobby's forecourt, for the same reason.
    ("exit.forecourt_bus", "board", "board"),
];

/// Every `;`-separated segment of every shipped exit, folded, as
/// `(entry id, segment)` — the same walk pack apply's check makes. The starter
/// lot's packs, then the station packs (doc 18 cut IC1).
fn shipped_exit_segments() -> Vec<(String, String)> {
    let mut rows = Vec::new();
    let mut packs = starter::packs().expect("the shipped packs load");
    packs.extend(tacsim::definitions::content::station_packs().expect("the station packs load"));
    for pack in packs {
        for (id, entry) in pack.entries() {
            if entry.kind().as_str() != "exit" {
                continue;
            }
            let def: ExitDef = entry.decode().expect("a shipped exit decodes");
            for name in &def.names {
                for segment in name.split(';').map(str::trim).filter(|s| !s.is_empty()) {
                    rows.push((id.to_string(), segment.to_lowercase()));
                }
            }
        }
    }
    // Doc 18 cut IC8 (IC4 C7): the Dray's interior exits, laid up in every programme
    // hull, as `tacsim.unit.dray/<from>-<to>`.
    let dray = tacsim::pack::load_pack(tacsim::definitions::content::DRAY_JSON)
        .expect("the Dray pack loads");
    let unit = mush_core::CanonicalId::new(tacsim::definitions::content::DRAY).expect("the id");
    let Some(tacsim::pack::PackEntry::Unit(spec)) = dray.entry(&unit) else {
        panic!("the Dray pack carries the Dray unit");
    };
    let interior = spec
        .interior
        .as_ref()
        .expect("the Dray authors an interior");
    for exit in &interior.exits {
        let id = format!(
            "{}/{}-{}",
            tacsim::definitions::content::DRAY,
            exit.from,
            exit.to
        );
        for name in &exit.names {
            for segment in name.split(';').map(str::trim).filter(|s| !s.is_empty()) {
                rows.push((id.clone(), segment.to_lowercase()));
            }
        }
    }
    rows
}

#[test]
fn the_shipped_exits_take_exactly_the_module_verbs_that_were_ruled_on() {
    let composition = production_composition();
    let registry = composition.registry();

    // Every name and alias a MODULE registered, folded. A kernel or host entry
    // (`module: None`) is not here: those are what apply refuses outright.
    let mut module_words: BTreeSet<String> = BTreeSet::new();
    for (spec, module, _) in registry.command_registrations() {
        if module.is_none() {
            continue;
        }
        module_words.insert(spec.name.to_lowercase());
        module_words.extend(spec.aliases.iter().map(|a| a.to_lowercase()));
    }
    assert!(
        module_words.contains("board"),
        "the production composition registers no BOARD; this gate proves nothing"
    );

    let mut found: BTreeSet<(String, String, String)> = BTreeSet::new();
    for (id, folded) in shipped_exit_segments() {
        if !module_words.contains(&folded) {
            continue;
        }
        let verb = registry
            .spec_of(&folded)
            .map_or_else(|| folded.clone(), |spec| spec.name.to_string());
        found.insert((id, folded, verb));
    }

    let decided: BTreeSet<(String, String, String)> = DECIDED
        .iter()
        .map(|(id, segment, verb)| {
            (
                (*id).to_string(),
                (*segment).to_string(),
                (*verb).to_string(),
            )
        })
        .collect();
    assert_eq!(
        found, decided,
        "a shipped exit's overlap with a module verb changed. A NEW row is a \
         decision a person makes: either rename the exit segment, or add the row \
         here with the reason it is safe (which room the verb means anything in, \
         and why no exit there is called that)."
    );
}

/// **No shipped exit takes a word the kernel OR THE HOST registered**, over the
/// production registry — the one composition that has both.
///
/// Pack apply refuses such an exit, so a collision arriving here would refuse
/// the starter lot at a fresh production boot. `mush_content`'s boot pass logs
/// that refusal and leaves the world as it was (an error line, not a boot
/// failure), so the first symptom would be a production world with no lot in
/// it. This gate is upstream of that.
#[test]
fn no_shipped_exit_segment_is_a_word_the_kernel_or_the_host_owns() {
    let composition = production_composition();
    let registry = composition.registry();
    // The host's verbs are in this registry and in no fixture's: prove they
    // composed, or an empty answer below would mean nothing.
    assert!(
        registry.kernel_command_named("@boot").is_some(),
        "the host's own staff verbs did not register; this gate proves nothing"
    );

    let taken: Vec<String> = shipped_exit_segments()
        .into_iter()
        .filter_map(|(id, segment)| {
            registry
                .kernel_command_named(&segment)
                .map(|command| format!("{id}: `{segment}` is the command `{command}`"))
        })
        .collect();
    assert!(
        taken.is_empty(),
        "a shipped exit takes a word that works in every room, so pack apply \
         refuses the pack and a fresh production boot comes up with no lot:\n{}",
        taken.join("\n")
    );
}
