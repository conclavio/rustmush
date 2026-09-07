//! E7 cut 11 **independent verify fold** — the mechanical and census half
//! (10b §14, §15.4).
//!
//! Nothing here drives a command: these are the source censuses §14's own rule
//! (*"a migration commit cites the **symbols** below, never these numbers"*)
//! requires, plus the pair of facts that refute §15.4 item 29's premise. Each
//! one **asserts something that was false before the fold**, so a probe that
//! passes on the pre-fold tree is a bug in the probe.

// ---- E7 cut 11 independent verify fold ----

use std::path::{Path, PathBuf};

/// The `crates/` directory this test's package lives under.
fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .to_path_buf()
}

/// One production source, truncated at the first `#[cfg(test)]` so a module's
/// own tests never count as writers.
fn production(rel: &str) -> String {
    let text = std::fs::read_to_string(crates_dir().join(rel)).expect("a source file");
    let boundary = text.find("#[cfg(test)]").unwrap_or(text.len());
    text[..boundary].to_owned()
}

/// Every line matching one of `needles`, with the name of the `fn` that
/// encloses it — the census shape §14 needs, because a lane is defined by its
/// writers and not by a line count.
fn writer_sites(text: &str, needles: &[&str]) -> Vec<(String, String)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let code = line.trim_start();
        if code.starts_with("//") || code.starts_with("///") {
            continue;
        }
        if !needles.iter().any(|needle| code.contains(needle)) {
            continue;
        }
        let mut enclosing = String::from("<none>");
        for candidate in lines[..=i].iter().rev() {
            let trimmed = candidate.trim_start();
            let rest = trimmed
                .strip_prefix("pub(crate) ")
                .or_else(|| trimmed.strip_prefix("pub "))
                .unwrap_or(trimmed);
            if let Some(tail) = rest.strip_prefix("fn ") {
                enclosing = tail
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                break;
            }
        }
        out.push((enclosing, code.to_owned()));
    }
    out
}

fn enclosing_set(text: &str, needles: &[&str]) -> Vec<String> {
    let mut names: Vec<String> = writer_sites(text, needles)
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    names.sort();
    names.dedup();
    names
}

/// **G3. §15.4 item 29's premise is refuted: the tick window is pre-opened and
/// its id is a pure derivation.**
///
/// Item 29 files the missing `AuthorityGuardKind::Window` guard as **K** work,
/// blocked on *"a kernel affordance for a guard that binds to a to-be-created
/// window"* because *"the window is opened lazily by the first intent of a
/// tick"*. Three facts in `mush-sim`'s own file contradict it, and two of them
/// are asserted here: `domain_tick`'s prepare pre-opens the NEXT tick's window
/// inside the same bundle, and `tick_window_id` is a pure derivation over
/// `(domain, generation, tick)` that reads nothing at all. (The third —
/// `AuthorityGuard::Window` carrying no record revision — is `mush-core`'s
/// `ActionWindowGuardValidator`, and `mush-sim` registers that validator
/// itself, which is asserted below.) The guard is therefore E-side work wholly
/// inside `mush-sim`, and the only residue is pre-opening the FIRST window of a
/// generation.
#[test]
fn vf_g3_the_next_tick_window_is_pre_opened_and_its_id_is_pure() {
    // (1) Purity: no world, no engine, no store — and it is total over its four
    // inputs, so a guard may be declared for a window that does not exist yet.
    let owner = mush_core::OwnerId::new("sim").expect("the sim owner");
    let domain = mush_sim::DomainId(7);
    let a = mush_sim::tick::tick_window_id(&owner, domain, 1, 4_096).expect("a window id");
    let b = mush_sim::tick::tick_window_id(&owner, domain, 1, 4_096).expect("a window id");
    assert_eq!(a, b, "one (domain, generation, tick) derives one window id");
    assert_ne!(
        a,
        mush_sim::tick::tick_window_id(&owner, domain, 1, 4_097).expect("a window id"),
        "the id is a function of the tick, not of anything stored"
    );
    // The id for a tick that has never been reached is computable NOW, which is
    // exactly what item 29 says is impossible.
    assert!(mush_sim::tick::tick_window_id(&owner, domain, 9, i64::MAX - 1).is_some());

    // (2) The pre-open, in `domain_tick`'s own prepare. The measured bundle the
    // §7 capacity table is built from carries the next window as a post-image,
    // which is the same write the prepare makes.
    let measured = mush_sim::tick::measure_tick_bundle(1).expect("the tick bundle encodes");
    assert!(
        measured.replay_writes >= 5,
        "the tick's five fixed writes include the NEXT window's post-image"
    );
    let tick_src = production("mush-sim/src/tick.rs");
    let prepare_at = tick_src
        .find("fn prepare_authoritative_with_context")
        .expect("domain_tick's prepare");
    let body = &tick_src[prepare_at..];
    let preopen = body
        .find("let next_tick = plan.tick.saturating_add(1);")
        .expect("the next tick's window is pre-opened in the same bundle");
    assert!(
        body[preopen..preopen + 800].contains("ActionWindowRecord::open(window)"),
        "…and it is INSERTED, so the following tick's submissions have somewhere \
         to go without a second authority opening it"
    );
    // (3) `mush-sim` registers the window guard's validator itself, so declaring
    // `AuthorityGuard::Window` at `project_offer` needs no kernel affordance.
    assert!(
        tick_src.contains("reg.authority_guard_validator(\n        AuthorityGuardKind::Window,"),
        "the window guard validator is registered by this crate"
    );
}

/// **G12. §14.1's L1 writer set, pinned by enclosing function.**
///
/// §14.1 advertised *"the five synchronous placement writers"* and mislabelled
/// boot's three distinct writes (the removes are the DELETE set, not quarantine;
/// the quarantine path INSERTS a revision-bumped record; the hydration write is
/// a whole-map assignment), omitted `held_placements` — a map that is itself one
/// of L1's own needles and that no other site writes — and described
/// `commands.rs` as "a single direct `persist_placement`" when it is also the
/// tree's only `placements.get_mut`. A new writer breaks this and forces §14's
/// paragraph to be edited in the same commit.
#[test]
fn vf_g12_the_placement_writer_census_is_pinned() {
    // Only writes to the LIVE `SpatialState` map count: `post.placements` is a
    // plan post-image and boot's unqualified local is the staging map the
    // hydration assignment below installs. The whole-map assignment and
    // `spatial_recover`'s `get_mut` both wrap across lines, so the count runs
    // over a whitespace-flattened copy.
    const OWNED: &[&str] = &[
        "st.placements.insert(",
        "st.placements.remove(",
        "st.placements.retain(",
        "st.placements =",
        "st.held_placements.insert(",
        "s.placements.insert(",
        "s.placements.remove(",
        "s.placements.get_mut(",
        "s.held_placements.insert(",
    ];
    let flatten = |rel: &str| -> String {
        production(rel)
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join(" ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .replace(" .", ".")
    };
    let files = [
        "mush-sim/src/boot.rs",
        "mush-sim/src/transition.rs",
        "mush-sim/src/lib.rs",
        "mush-sim/src/commands.rs",
    ];
    let counts: Vec<(&str, usize)> = files
        .into_iter()
        .map(|rel| {
            let flat = flatten(rel);
            let raw: usize = OWNED
                .iter()
                .map(|needle| flat.matches(needle).count())
                .sum();
            // `post.placements.insert(` ends in `st.placements.insert(`.
            (rel, raw - flat.matches("post.placements.insert(").count())
        })
        .collect();
    assert_eq!(
        counts,
        vec![
            // the hydration assignment, the delete-set removes, the quarantine
            // insert of a revision-bumped record, and the `held_placements`
            // inserts §14 did not name.
            ("mush-sim/src/boot.rs", 4),
            // `transition::apply`, the ONE checked path.
            ("mush-sim/src/transition.rs", 1),
            // destroy-time removal.
            ("mush-sim/src/lib.rs", 1),
            // `spatial_recover`'s `get_mut` — the tree's only one.
            ("mush-sim/src/commands.rs", 1),
        ],
        "L1's synchronous writers, by file"
    );
    assert_eq!(
        enclosing_set(
            &production("mush-sim/src/boot.rs"),
            &[
                "placements.insert(",
                "placements.remove(",
                "held_placements.insert("
            ],
        ),
        vec!["load_boot".to_owned()],
        "boot's three distinct placement writes plus `held_placements`, all in          `load_boot`"
    );
    assert_eq!(
        enclosing_set(
            &production("mush-sim/src/lib.rs"),
            &["s.placements.remove("]
        ),
        vec!["on_obj_destroyed".to_owned()]
    );
    assert_eq!(
        enclosing_set(&production("mush-sim/src/commands.rs"), &[".get_mut(&obj)"],),
        vec!["spatial_recover".to_owned()],
        "the tree's only `placements.get_mut`"
    );
    // `held_placements` really is written in exactly one place.
    let held: usize = files
        .into_iter()
        .map(|rel| production(rel).matches("held_placements.insert(").count())
        .sum();
    assert_eq!(held, 1, "`held_placements` has exactly one writer");
}

/// **G13. §14.1 names synchronous writers for L1 only — here is L2's.**
///
/// L2 is declared inseparable from L1 (*"they move TOGETHER or not at all"*)
/// and §14 enumerates none of its writers at all, so §14.3 criterion 2 (*"the
/// lane moves whole"*) is unenforceable for it. These five functions are the
/// whole of the `sim.station_assignment` lane's synchronous write surface, and
/// `seat::seat_billet` is the *"exactly one writer"* of the `granted` narrowing
/// §4.1's cut-10 row names.
#[test]
fn vf_g13_the_station_assignment_writer_census_is_pinned() {
    const NEEDLES: &[&str] = &[
        "held_assignments.insert(",
        "assignments.insert(",
        "assignments.remove(",
        "assignments.get_mut(",
        "seats.insert(",
        "seats.retain(",
    ];
    let per_file: Vec<(&str, usize, Vec<String>)> = [
        "mush-sim/src/boot.rs",
        "mush-sim/src/commands.rs",
        "mush-sim/src/lib.rs",
        "mush-sim/src/seat.rs",
    ]
    .into_iter()
    .map(|rel| {
        let text = production(rel);
        (
            rel,
            writer_sites(&text, NEEDLES).len(),
            enclosing_set(&text, NEEDLES),
        )
    })
    .collect();
    assert_eq!(
        per_file,
        vec![
            // hydration insert, the seat-cap `retain` pass, and the B4 hold.
            ("mush-sim/src/boot.rs", 3, vec!["load_boot".to_owned()]),
            // `@spatial/assign` and `@spatial/unassign`.
            (
                "mush-sim/src/commands.rs",
                2,
                vec!["spatial_assign".to_owned(), "spatial_unassign".to_owned()]
            ),
            // destroy, plus cut 10's A4 actor-death seat cleanup.
            (
                "mush-sim/src/lib.rs",
                4,
                vec!["on_obj_destroyed".to_owned()]
            ),
            // the `granted` narrowing: exactly one writer.
            ("mush-sim/src/seat.rs", 1, vec!["seat_billet".to_owned()]),
        ],
        "L2's synchronous writers, by file and by enclosing function"
    );
    // The cross-crate read §14.1 described as "derived" is the one that is not:
    // `mush-propagation` reads `assignments_by_actor` (a DIFFERENT index from
    // the `seats_by_actor` §14 named), and `mush-campaign` reads the raw map.
    let prop = production("mush-propagation/src/commands.rs");
    assert_eq!(prop.matches("derived.assignments_by_actor").count(), 1);
    assert_eq!(prop.matches("derived.seats_by_actor").count(), 0);
}
