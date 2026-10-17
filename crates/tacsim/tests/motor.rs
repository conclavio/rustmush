//! **The motor's audit** (17b cut M1a.1; doc 17 §4 "The motor", D-UM-3: *the
//! motor's reads need an audit in M1a*).
//!
//! `crates/tacsim/src/arena/motor.rs` is what drives a hull to a goal, and from
//! 17b cut M1a.4 on it drives a pilot's `DRIVE` and a mind's `goto` as well as
//! the reflex controller's goals. Doc 17 §1's observer rule then bites on it
//! twice over: a kernel that read another unit's live pose, or the spatial
//! index, would hand a pilot or a mind knowledge no sensor gave it. So each
//! kernel the motor exposes is held here, on the map the owner plays on
//! ([`support::shipped_arena`]), to what it may read — the ground, the hull's
//! own pose and row, the plot it is handed and the cells it was refused — and
//! the two pins of the cut are asserted: the shipped duel hashes as it did at
//! `32a1cc23`, and the doctrine the motor reads is the arena's. Since 17b cut
//! M1b.0 the duel's every ledger line is pinned with its hash as well
//! ([`m1b0_the_production_line_hashes_do_not_move`]).
//!
//! The kernels are `pub` on [`Arena`] since this cut for exactly this file (and
//! for 17b cut M1a.6, which lifts them into free functions).

mod support;

use mush_core::ObjId;
use support::{CELL_CM, Muster, booted_world, cell_of, rig, run_until, shipped_arena, tank};
use tacsim::arena::CostRow;
use tacsim::arena::motion::BlockedCause;
use tacsim::arena::motor::MotorCounters;
use tacsim::arena::motor::{MotorRow, Refusal};
use tacsim::arena::state::{Arena, HoldState, Power, SeenPose, Team};

/// The final state hash of [`duel`] stepped at 1,000 ms for 60 s, read at
/// `32a1cc23` — before `arena/motor.rs` existed — off this very run.
///
/// **The cut's one declared re-bless** (17b cut M3.6: `mind_sealed` and
/// `sortie_closed` shift the folded ledger sequence (`hash.rs:897`) of every
/// match; the mind row's fold gains D-UM-23's answer, `dropped` and the seal
/// digest; the chassis' core consequence words ride the hit lines and the
/// pilot's sentence; every other byte's encoding is unchanged). Re-blessed
/// here: `0xd3e7_ee71_474b_6738` became `0xd25d_39ba_9b04_7064` (the duel's
/// four `mind_sealed` lines shift the folded ledger sequence; its tape's diff
/// is `M1B0_DUEL_LINES`').
///
/// **17b cut M4.3 — the flip's one production re-bless** (declared with
/// `tests/minds.rs`' `m1a5_the_production_hash_does_not_move`, the same
/// constant): the duel's four machines are minds sealed as the production
/// fill is (`Muster::fill`), so the controller byte, the mind rows, the
/// production engine's seal digests and the script's digest fold where the
/// reflex rows did: `0xd25d_39ba_9b04_7064` became `0x65eb_692a_b7a8_a72d`.
///
/// **The fill decision — re-pinned, hashes only** (2026-10-09; declared with
/// `tests/minds.rs`' `m1a5_the_production_hash_does_not_move`, the same
/// constant): the production fill is sealed `native`
/// (`PRODUCTION_FILL_ENGINE`, the owner's three ALIEN18 runs all `held=no`),
/// so the duel's four `Muster::fill` hulls carry native seals and think on
/// the native stock mind, which Gate A holds equal to the Rhai one they ran
/// on. The seal's engine is folded into its digest and the digest into the
/// mind row's hash (17b cut M3.6), and nothing else moved: the duel's tape
/// ([`M1B0_DUEL_LINES`]) is byte-equal to the M4.3 record but for its `hash`
/// values. `0x65eb_692a_b7a8_a72d` became `0x999b_45cb_390f_4af8`. **Hash v2** (16c T-104, 2026-10-12): the state hash's fold moved — `Fold`, eight bytes a multiply, the notices by `fold_value` — so every `hash` moved and nothing else did:
/// `0x999b_45cb_390f_4af8` became `0xa4cd_3db1_3485_19ef`, the tape byte-equal but for its
/// `hash` values again.
/// 17d M5rules: `0xa4cd_3db1_3485_19ef` becomes `0x81b3_3391_16fe_cdaa`
/// under the compatibility re-bless described at `M1B0_DUEL_LINES`.
const DUEL_HASH_AT_HEAD: u64 = 0x81b3_3391_16fe_cdaa;

/// Two machines a side at their authored musters on the proving ground —
/// **17b cut M4.3**: every one a mind sealed as the production fill is
/// (`Muster::fill`: Standard Doctrine on `PRODUCTION_FILL_ENGINE` at the
/// default `fill_class`, the production installer run over it — installing
/// nothing since the fill decision of 2026-10-09 made the engine `native`),
/// the shape a production match fields.
fn duel(world: &mush_testkit::TestWorld) -> Arena {
    shipped_arena(
        world,
        &[
            &[Muster::fill(Team::Red, 1), Muster::fill(Team::Red, 2)],
            &[Muster::fill(Team::Blue, 1), Muster::fill(Team::Blue, 2)],
        ],
    )
}

/// The centre of `cell`, centimetres.
fn centre(cell: (u16, u16)) -> (i64, i64) {
    let side = i64::from(CELL_CM);
    (
        i64::from(cell.0) * side + side / 2,
        i64::from(cell.1) * side + side / 2,
    )
}

/// Put `unit` at `cell`'s centre and rebuild the index, as a staff place would
/// leave it (minus the ledger line, which nothing here reads).
fn put(arena: &mut Arena, unit: ObjId, cell: (u16, u16)) {
    let (x, y) = centre(cell);
    if let Some(state) = arena.unit_mut(unit) {
        state.pose.x_cm = x;
        state.pose.y_cm = y;
    }
    arena.index.rebuild(&arena.definition.terrain, &arena.units);
}

/// **17b cut M1a.1's second pin: the shipped duel hashes as it did** (the M1a
/// plan's head, "the production hash"). Four stock machines on the proving
/// ground for a minute at a one-second cadence — patrols, searches, routes and
/// the steer, every one of them now the motor's — end in the state hash read at
/// `32a1cc23` before the motor was split out. 17b cuts M1a.4, M1a.5 and M1a.6
/// re-assert it; a move is a defect in the move, never a re-bless.
#[test]
fn m1a1_the_shipped_duel_hashes_as_before() {
    let world = booted_world();
    let mut arena = duel(&world);
    let mut searched = 0_u32;
    let _ = run_until(&mut arena, 60_000, 1_000, |_, out| {
        searched = searched.saturating_add(out.motor.path_queries);
    });
    assert_eq!(arena.now_ms, 60_000);
    assert!(searched > 0, "the motor searched: the pin covers a drive");
    assert!(
        arena.motors.rows().any(|(_, motor)| motor.goal.is_some()),
        "the machines are driving somewhere"
    );
    assert_eq!(
        tacsim::arena::hash::state_hash(&arena),
        DUEL_HASH_AT_HEAD,
        "the shipped duel's hash moved"
    );
}

/// **The shipped duel's ledger, every line whole with its hash, recorded at
/// `e23694b0`** (17b cut M1b.0; the M1b plan's §1(a) and (c)) — before the cut
/// touched the due loop, off [`duel`] stepped at 1,000 ms for 60 s, each line
/// as `serde_json` writes a [`LedgerEvent`](tacsim::arena::ledger::LedgerEvent).
/// The minute holds no shot (the four machines start, see, classify and
/// drive), so its only action-run lines are the four reflex `startup`s; every
/// other line carries its substep's state hash, which no action hash reaches
/// (16b cut M.3: `Arena::event_hash` alone reads it). What it pins is the
/// shipped duel's bytes; the reflex shots' lines are `tests/reflex.rs`' half
/// of the pin, and that no trigger opens `ActionHash::Pending` for a hull with
/// no slot is asserted where a trigger is built (17b cut M1b.1), not here.
///
/// **The cut's one declared re-bless** (17b cut M3.6: `mind_sealed` and
/// `sortie_closed` shift the folded ledger sequence (`hash.rs:897`) of every
/// match; the mind row's fold gains D-UM-23's answer, `dropped` and the seal
/// digest; the chassis' core consequence words ride the hit lines and the
/// pilot's sentence; every other byte's encoding is unchanged). Re-recorded
/// here from 30 lines to 34: four `mind_sealed` lines after `match_started`
/// (the minute does not end, so no close; no turret dies), every line's `seq`
/// and `hash` re-blessed with them and every other key byte-equal (the diff,
/// ignoring `seq` and `hash`, is the four inserted lines alone).
///
/// **17b cut M4.3 — re-recorded with the flip's one production re-bless**
/// (34 lines to 34): the duel's four machines are minds on the production
/// fill's seal (`Muster::fill`), their `mind_sealed` lines' `source` the
/// script's new digest, their drives the minds' own, every `hash` re-blessed
/// under the one reason.
///
/// **The fill decision — re-recorded, hashes only** (2026-10-09; 34 lines to
/// 34): the production fill is sealed `native`, so the four machines' seal
/// digests — folded into every substep's hash — moved, and nothing else did:
/// compared with every `hash` taken out, the 34 lines are byte-equal to the
/// M4.3 record (the same `seq`, kinds, units, poses, draws and `mind_sealed`
/// fields — the `mind_sealed` line names no engine), and all 34 `hash`
/// values are re-blessed under that one reason.
///
/// **Hash v2** (16c T-104, 2026-10-12): the state hash's fold moved — `Fold`, eight bytes a multiply, the notices by `fold_value` — so every `hash` moved and nothing else did (34 lines to 34): compared with every `hash` taken out, the 34 lines
/// are byte-equal to the fill-decision record, and all 34 `hash` values are
/// re-blessed under that one reason.
/// **17d M5rules compatibility re-bless** (2026-10-17): the ordered rule
/// shape is hashed and the mind API/source identity moved. The 34 observed
/// lines were compared field by field: only `hash` and the Standard Doctrine
/// `source` identifiers changed; timestamps, poses, actions and draws did not.
const M1B0_DUEL_LINES: &[&str] = &[
    r#"{"seq":1,"t_ms":0,"kind":"match_started","hash":13266666079423458445}"#,
    r#"{"seq":2,"t_ms":0,"kind":"mind_sealed","unit":3,"sortie":3,"mind":"Standard Doctrine","source":"55cabde517b6a08e","card_digest":"1a04f1f74acd80a328e492b698b258f521ba884bd390eaac1ad432fb41d5686b","class":"I","governed":true,"licence":"doctrine","hash":13266666079423458445}"#,
    r#"{"seq":3,"t_ms":0,"kind":"mind_sealed","unit":4,"sortie":4,"mind":"Standard Doctrine","source":"55cabde517b6a08e","card_digest":"1a04f1f74acd80a328e492b698b258f521ba884bd390eaac1ad432fb41d5686b","class":"I","governed":true,"licence":"doctrine","hash":13266666079423458445}"#,
    r#"{"seq":4,"t_ms":0,"kind":"mind_sealed","unit":7,"sortie":7,"mind":"Standard Doctrine","source":"55cabde517b6a08e","card_digest":"1a04f1f74acd80a328e492b698b258f521ba884bd390eaac1ad432fb41d5686b","class":"I","governed":true,"licence":"doctrine","hash":13266666079423458445}"#,
    r#"{"seq":5,"t_ms":0,"kind":"mind_sealed","unit":8,"sortie":8,"mind":"Standard Doctrine","source":"55cabde517b6a08e","card_digest":"1a04f1f74acd80a328e492b698b258f521ba884bd390eaac1ad432fb41d5686b","class":"I","governed":true,"licence":"doctrine","hash":13266666079423458445}"#,
    r#"{"seq":6,"t_ms":258,"kind":"detected","viewer":8,"contact":8314046734819043908,"target":7,"hash":1029804625480590775}"#,
    r#"{"seq":7,"t_ms":421,"kind":"detected","viewer":3,"contact":5844251635052541762,"target":4,"hash":1029804625480590775}"#,
    r#"{"seq":8,"t_ms":482,"kind":"detected","viewer":4,"contact":5992554990774319474,"target":3,"hash":1029804625480590775}"#,
    r#"{"seq":9,"t_ms":508,"kind":"startup","unit":8,"hash":17102355816561113278}"#,
    r#"{"seq":10,"t_ms":671,"kind":"startup","unit":3,"hash":17102355816561113278}"#,
    r#"{"seq":11,"t_ms":732,"kind":"startup","unit":4,"hash":17102355816561113278}"#,
    r#"{"seq":12,"t_ms":594,"kind":"detected","viewer":7,"contact":3822974778006219272,"target":8,"hash":11008735619465374941}"#,
    r#"{"seq":13,"t_ms":844,"kind":"startup","unit":7,"hash":8562584081519029154}"#,
    r#"{"seq":14,"t_ms":1000,"kind":"tempo","from":"quiet","to":"alert","hash":17033045555161340674}"#,
    r#"{"seq":15,"t_ms":3258,"kind":"classified","viewer":8,"contact":8314046734819043908,"target":7,"team":"blue","draws":[["classify:8:7:4",3884165266672269218]],"hash":8836143993225336419}"#,
    r#"{"seq":16,"t_ms":3421,"kind":"classified","viewer":3,"contact":5844251635052541762,"target":4,"team":"red","draws":[["classify:3:4:4",16846213216163885984]],"hash":8836143993225336419}"#,
    r#"{"seq":17,"t_ms":3482,"kind":"classified","viewer":4,"contact":5992554990774319474,"target":3,"team":"red","draws":[["classify:4:3:4",2094150884743799771]],"hash":8836143993225336419}"#,
    r#"{"seq":18,"t_ms":3594,"kind":"classified","viewer":7,"contact":3822974778006219272,"target":8,"team":"blue","draws":[["classify:7:8:4",989486057018502202]],"hash":13096396768504441538}"#,
    r#"{"seq":19,"t_ms":5000,"kind":"position_summary","units":[[3,6500,97500,100,90000],[4,6500,95500,100,90000],[7,249500,97500,-100,270000],[8,249500,95500,-100,270000]],"hash":5972273398650895137}"#,
    r#"{"seq":20,"t_ms":10000,"kind":"position_summary","units":[[3,6500,97500,100,90000],[4,6500,95500,100,90000],[7,249500,97500,-100,270000],[8,249500,95500,-100,270000]],"hash":9596573233090139287}"#,
    r#"{"seq":21,"t_ms":10508,"kind":"power_online","unit":8,"hash":12200621359235327173}"#,
    r#"{"seq":22,"t_ms":10671,"kind":"power_online","unit":3,"hash":12200621359235327173}"#,
    r#"{"seq":23,"t_ms":10732,"kind":"power_online","unit":4,"hash":12200621359235327173}"#,
    r#"{"seq":24,"t_ms":10844,"kind":"power_online","unit":7,"hash":3373630082977431115}"#,
    r#"{"seq":25,"t_ms":15000,"kind":"position_summary","units":[[3,7068,97500,100,90000],[4,6911,95885,100,43754],[7,249293,97902,-100,0],[8,249088,95885,-100,316287]],"hash":17029130154154180068}"#,
    r#"{"seq":26,"t_ms":20000,"kind":"position_summary","units":[[3,7740,99285,100,322500],[4,9358,96498,100,89214],[7,249500,100700,-100,0],[8,246641,96498,-200,270899]],"hash":4693514629893429416}"#,
    r#"{"seq":27,"t_ms":25000,"kind":"position_summary","units":[[3,6551,103217,100,358159],[4,14421,96500,100,90000],[7,249500,104240,-200,0],[8,241579,96500,-200,270000]],"hash":14299548699920893706}"#,
    r#"{"seq":28,"t_ms":30000,"kind":"position_summary","units":[[3,5181,104876,100,260070],[4,21522,96500,200,90000],[7,251648,104201,-100,150000],[8,234360,96500,-200,270000]],"hash":5635703159909164060}"#,
    r#"{"seq":29,"t_ms":35000,"kind":"position_summary","units":[[3,5467,101252,100,126057],[4,28493,96500,200,90000],[7,249341,100316,-100,219608],[8,227758,96500,-200,270000]],"hash":9550452240827907831}"#,
    r#"{"seq":30,"t_ms":40000,"kind":"position_summary","units":[[3,7887,98377,100,112170],[4,31801,96500,200,90000],[7,248134,98228,-100,221052],[8,225080,96500,-200,270000]],"hash":11682514796800710836}"#,
    r#"{"seq":31,"t_ms":45000,"kind":"position_summary","units":[[3,12238,98497,100,89366],[4,35823,96500,200,90000],[7,244589,97500,-200,270000],[8,221603,96500,-300,270000]],"hash":6468311564491595757}"#,
    r#"{"seq":32,"t_ms":50000,"kind":"position_summary","units":[[3,18787,97828,200,114667],[4,42331,96500,200,90000],[7,238307,97500,-200,270000],[8,215625,96500,-300,270000]],"hash":2678641767323847235}"#,
    r#"{"seq":33,"t_ms":55000,"kind":"position_summary","units":[[3,26125,97497,200,89684],[4,49781,96638,200,75000],[7,230819,97500,-200,270000],[8,208140,96500,-300,270000]],"hash":14143606315329224747}"#,
    r#"{"seq":34,"t_ms":60000,"kind":"position_summary","units":[[3,30797,97500,200,90000],[4,54548,97521,200,91283],[7,226176,97500,-200,270000],[8,202598,96500,-300,270000]],"hash":8421236263452177361}"#,
];

/// **17b cut M1b.0: the production line-hash pin, the reflex half.** The
/// shipped duel — four stock machines on the reflex controller on the proving
/// ground — journals exactly the lines it journalled at `e23694b0`, each with
/// its hash; [`m1a1_the_shipped_duel_hashes_as_before`] pins its final state
/// hash, this pins every line's. Every later sub-cut of M1b re-asserts it
/// (`tests/weapons.rs` holds the seat half); a move is a defect, never a
/// re-bless. `tests/reflex.rs` holds the reflex shots' half.
#[test]
fn m1b0_the_production_line_hashes_do_not_move() {
    let world = booted_world();
    let mut arena = duel(&world);
    let mut bytes = run_until(&mut arena, 60_000, 1_000, |_, _| {});
    assert_eq!(arena.now_ms, 60_000);
    let lines: Vec<String> = support::ledger_events(&mut arena, &mut bytes)
        .iter()
        .map(|event| serde_json::to_string(event).unwrap_or_default())
        .collect();
    assert_eq!(
        lines,
        M1B0_DUEL_LINES
            .iter()
            .map(|line| (*line).to_owned())
            .collect::<Vec<_>>(),
        "the shipped duel's ledger lines moved"
    );
}

/// **`stand_off_cell` reads the plot and not the target** (doc 17 §4's audit;
/// 16b cut F.3's rule). Red Tank 1 holds Blue Tank 1 as a classified contact
/// whose plot is where Blue stands; Blue is then moved two cells by writing its
/// live pose, and the stand-off cell off the held plot is unchanged — while a
/// plot read off Blue's new pose would have given another cell, so the
/// assertion could see a leak. (17b cut M4.5a: Red is a stock mind's hull,
/// `Muster::mind`, where it was the reflex machine's — the kernel mirror of
/// `tests/reflex.rs`' deleted row 3 no longer fields the reflex; it is never
/// stepped, and the kernel reads no controller.)
#[test]
fn m1a1_stand_off_cell_reads_the_plot_and_not_the_target() {
    let world = booted_world();
    let mut arena = shipped_arena(
        &world,
        &[
            &[Muster::mind(Team::Red, 1)],
            &[Muster::piloted(Team::Blue, 1)],
        ],
    );
    let red = tank(&world, Team::Red, 1);
    let blue = tank(&world, Team::Blue, 1);
    let seen = arena.unit(blue).map(SeenPose::of).expect("blue");
    let id = tacsim::arena::rng::draw_contact_id(support::SEED, red, blue);
    arena.contacts.insert(
        red,
        id,
        tacsim::arena::state::ShadowContact::sighted(
            blue,
            tacsim::arena::state::Classification::Classified { team: Team::Blue },
            0,
            seen,
        ),
    );
    let plot = |arena: &Arena| arena.contacts.resolve(red, id).map(|held| held.seen_at);
    let want = 39_000_u64;
    let before = plot(&arena).and_then(|at| arena.stand_off_cell(red, &at, want));
    assert!(before.is_some(), "a stand-off cell on the shipped ground");

    let moved_to = cell_of(&arena, blue).map(|(x, y)| (x, y.saturating_add(2)));
    put(&mut arena, blue, moved_to.expect("blue is on the window"));
    assert_eq!(cell_of(&arena, blue), moved_to, "blue moved");
    assert_eq!(plot(&arena), Some(seen), "and its plot did not");
    assert_eq!(
        plot(&arena).and_then(|at| arena.stand_off_cell(red, &at, want)),
        before,
        "the stand-off cell followed the target's live pose"
    );
    let live = arena.unit(blue).map(SeenPose::of).expect("blue");
    assert_ne!(
        arena.stand_off_cell(red, &live, want),
        before,
        "the probe could not have seen a leak: the live pose gives the same cell"
    );
}

/// The four pickers' answers: the entry cell, the far entry cell, the row the
/// waypoint walk leaves, and whether the hull has arrived at the Hold spot.
type Picks = (Option<(u16, u16)>, Option<(u16, u16)>, MotorRow, bool);

/// What the four navigation pickers answer for `unit`, standing where it stands.
fn picks(arena: &Arena, unit: ObjId) -> Picks {
    let here = cell_of(arena, unit);
    let mut row = MotorRow::default();
    arena.patrol_step(unit, &mut row, here);
    (
        arena.entry_cell(unit),
        arena.far_entry_cell(unit, &[]),
        row,
        arena.hold_arrived(unit, here),
    )
}

/// **The navigation pickers read the ground and the own pose only** (doc 17
/// §4's audit). `entry_cell` (a withdrawal's goal), `far_entry_cell` (a hunt's),
/// `patrol_step` (the waypoint walk) and `hold_arrived` (a Hold match's arrival,
/// with a Hold objective written onto the arena whose spot is the unit's own
/// cell) answer for Red Tank 1 exactly what they answered with every other unit
/// removed from the arena, and with every other unit standing on each cell
/// they picked.
#[test]
fn m1a1_the_nav_pickers_read_the_ground_and_the_own_pose_only() {
    let world = booted_world();
    let red = tank(&world, Team::Red, 1);
    let field = || {
        let mut arena = duel(&world);
        let here = cell_of(&arena, red).expect("red is on the window");
        arena.hold = Some(HoldState {
            spot: here,
            letter: None,
            radius_cells: 3,
            hold_ms: 60_000,
            held_by: None,
        });
        arena
    };
    let arena = field();
    let base = picks(&arena, red);
    assert!(base.0.is_some() && base.1.is_some(), "{base:?}");
    assert!(base.2.goal.is_some(), "the walk has a node: {base:?}");
    assert!(base.3, "standing on the spot is arrived");

    // Alone.
    let mut alone = field();
    alone.units.retain(|unit| unit.id == red);
    alone.index.rebuild(&alone.definition.terrain, &alone.units);
    assert_eq!(alone.units.len(), 1);
    assert_eq!(picks(&alone, red), base, "the pickers read another unit");

    // Everybody else on each picked cell in turn.
    let others: Vec<ObjId> = arena
        .units
        .iter()
        .map(|unit| unit.id)
        .filter(|id| *id != red)
        .collect();
    assert_eq!(others.len(), 3);
    let picked = [base.0, base.1, base.2.goal];
    for cell in picked.into_iter().flatten() {
        let mut crowded = field();
        for other in &others {
            put(&mut crowded, *other, cell);
        }
        assert_eq!(
            picks(&crowded, red),
            base,
            "the pickers read the units standing on {cell:?}"
        );
    }
}

/// The four open-line kernels' answers for `unit` toward `next`.
fn walks(arena: &Arena, unit: ObjId, next: (u16, u16), avoid: &[(u16, u16)]) -> [bool; 5] {
    let state = arena.unit(unit).cloned().expect("the unit");
    let here = cell_of(arena, unit).expect("on the window");
    let heading = state.pose.heading_mdeg;
    let (to_x, to_y) = centre(next);
    let (dx, dy) = (to_x - state.pose.x_cm, to_y - state.pose.y_cm);
    let reach = (dx * dx + dy * dy).isqrt();
    let mut counters = MotorCounters::default();
    [
        arena.step_is_drivable(&state, here, next, avoid),
        arena.first_step_is_open(&state, here, heading, avoid, &mut counters),
        arena.line_is_open(&state, here, heading, 5_000, avoid, false),
        arena.line_is_open(&state, here, heading, 5_000, avoid, true),
        arena.walk_is_open(&state, here, (dx, dy, reach), reach, avoid, true),
    ]
}

/// **The open-line walks read terrain and the avoid list, and never the
/// index** (doc 17 §4's audit; doc 16 §3.6: a hull knows it was refused a
/// cell, never what refused it). Red Tank 1 drives a route on the proving
/// ground; Blue Tank 1 is put on the route's next cell, the index rebuilt, and
/// `step_is_drivable`, `first_step_is_open`, `line_is_open` (both walks) and
/// `walk_is_open` answer what they answered before. The hull then drives on,
/// is stopped by the occupant, and `note_blocked` writes the cell onto its
/// motor row's avoid list — the one way traffic reaches the motor. (17b cut
/// M4.5a: Red is a stock mind's hull, `Muster::mind`, where it was the reflex
/// machine's: the route it runs is the motor's for the walk goal the mind's
/// landing handed it, and the stop is learned by the same `note_blocked`;
/// stepped through `Arena::step`, whose head settles the last pulse's thinks,
/// with the loop's 300,000 ms guard unchanged.)
#[test]
fn m1a1_the_open_line_walks_read_terrain_and_the_avoid_list_and_never_the_index() {
    let world = booted_world();
    let mut arena = shipped_arena(
        &world,
        &[
            &[Muster::mind(Team::Red, 1)],
            &[Muster::piloted(Team::Blue, 1)],
        ],
    );
    let red = tank(&world, Team::Red, 1);
    let blue = tank(&world, Team::Blue, 1);
    // Under way on a straight run of its route, pointed at the next cell: the
    // route's first three cells one step apart in one direction, that step's
    // compass heading within ten degrees of the hull's, the hull in the cell one
    // step short of the first and still with cells to go past it — so the next
    // cell is one motion must enter, and the hull is not about to arrive and
    // stop short of it on its own. And every one of the five walks open there
    // before anything stands on the cell: an occupant can only close a line, so
    // a walk already closed by the ground could not show an index read.
    let mut wall = 0_i64;
    let next = loop {
        let _ = arena.step(wall, &[], &[]);
        wall += 250;
        assert!(
            arena.now_ms < 300_000,
            "red never ran straight at a route cell"
        );
        let Some(state) = arena.unit(red) else {
            continue;
        };
        let (Some(here), Some(motor)) = (cell_of(&arena, red), arena.motors.row(red)) else {
            continue;
        };
        let [a, b, c, _, ..] = motor.route.as_slice() else {
            continue;
        };
        let step = |from: (u16, u16), to: (u16, u16)| {
            (
                i32::from(to.0) - i32::from(from.0),
                i32::from(to.1) - i32::from(from.1),
            )
        };
        let way = step(here, *a);
        let compass = match way {
            (0, 1) => 0_u32,
            (1, 1) => 45_000,
            (1, 0) => 90_000,
            (1, -1) => 135_000,
            (0, -1) => 180_000,
            (-1, -1) => 225_000,
            (-1, 0) => 270_000,
            (-1, 1) => 315_000,
            _ => continue,
        };
        let off = state.pose.heading_mdeg.abs_diff(compass);
        let off = off.min(360_000_u32.saturating_sub(off));
        let remembered: Vec<(u16, u16)> = motor.avoid.iter().map(|(cell, _)| *cell).collect();
        if state.speed_cms > 300
            && off <= 10_000
            && step(*a, *b) == way
            && step(*b, *c) == way
            && walks(&arena, red, *a, &remembered) == [true; 5]
        {
            break *a;
        }
    };
    let avoid: Vec<(u16, u16)> = arena
        .motors
        .row(red)
        .map(|motor| motor.avoid.iter().map(|(cell, _)| *cell).collect())
        .unwrap_or_default();
    assert!(
        !avoid.contains(&next),
        "the premise: nothing remembered there"
    );
    let before = walks(&arena, red, next, &avoid);
    assert_eq!(
        before, [true; 5],
        "the premise: every walk is open before the occupant, so an index read could only close one"
    );

    put(&mut arena, blue, next);
    assert_eq!(arena.index.cell_of_unit(blue), Some(next), "blue is on it");
    assert_eq!(
        walks(&arena, red, next, &avoid),
        before,
        "an open-line walk read the index"
    );

    // And the hull, which cannot see the occupant in any kernel, bumps it. Its
    // sight is taken away first, so the hostile in the road is a stone and not
    // a target: a machine that classified it would stop to fight it, and what
    // is under test here is the drive.
    if let Some(state) = arena.unit_mut(red) {
        state.sensor = None;
    }
    let until = arena.now_ms + 30_000;
    while arena.now_ms < until {
        let _ = arena.step(wall, &[], &[]);
        wall += 250;
        if arena
            .motors
            .row(red)
            .is_some_and(|motor| motor.avoid.iter().any(|(cell, _)| *cell == next))
        {
            break;
        }
    }
    assert!(
        arena
            .motors
            .row(red)
            .is_some_and(|motor| motor.avoid.iter().any(|(cell, _)| *cell == next)),
        "the stop at {next:?} was not learned: {:?}",
        arena.motors.row(red)
    );
}

/// **The routeless pre-pass holds any steered hull, whatever its controller**
/// (17b cut M1a.1: `hold_routeless` reads the motor rows, and a row exists for
/// a hull something is steering). A **pilot's** tank — no machine, no mind
/// row (17b cut M4.5a: the reflex row's vacuous `is_none` probe re-read as the
/// mind row's, the one machine a production hull carries) — is given a motor
/// row with a goal and no route, the shape 17b cut M1a.4's `DRIVE` lays down,
/// stands facing a cell its row cannot enter, and is ordered on; one substep
/// later the pre-pass has ordered it to stop (`StepOutput::motor`'s `intents`
/// counts the order, and the hull's desired speed is nought). What this cannot
/// show is the order's source: a continuous kind returns from `apply_intent`
/// before the seat's rate is counted whatever its source, so
/// `orders_in_window` staying nought proves only that the stop cost the seat
/// nothing, not that it went in as a machine order.
#[test]
fn m1a1_hold_routeless_holds_a_routeless_row_whatever_the_controller() {
    let world = booted_world();
    let mut arena = shipped_arena(&world, &[&[Muster::piloted(Team::Red, 1)]]);
    let red = tank(&world, Team::Red, 1);
    let terrain = arena.definition.terrain.clone();
    let row = arena.unit(red).map(|unit| unit.cost_row).expect("red");
    assert_eq!(row, CostRow::Tracked);
    // A cell its row may stand in with one it may not enter due north of it.
    let facing = (0..terrain.height().saturating_sub(1))
        .flat_map(|y| (0..terrain.width()).map(move |x| (x, y)))
        .find(|&(x, y)| terrain.passable_for(row, x, y) && !terrain.passable_for(row, x, y + 1));
    let Some(cell) = facing else {
        panic!("the proving ground has no impassable cell north of a passable one");
    };
    put(&mut arena, red, cell);
    if let Some(state) = arena.unit_mut(red) {
        state.pose.heading_mdeg = 0;
        state.power = Power::Online;
        state.desired_speed_cms = 800;
    }
    assert_eq!(
        arena
            .unit(red)
            .map(|unit| (unit.controller, unit.orders_in_window)),
        Some((tacsim::arena::state::Controller::Pilot, 0)),
        "a pilot's tank, with no seat order counted"
    );
    assert!(!arena.minds.contains_key(&red), "and no mind row");
    arena.motors.ensure(red).goal = Some((cell.0, cell.1.saturating_add(10)));

    let substep = i64::from(tacsim::arena::clock::substep_ms(
        &arena.arena_cfg.tempo,
        arena.cell_cm(),
        arena.max_speed_cms(),
    ));
    let _ = arena.step(0, &[], &[]);
    let out = arena.step(substep, &[], &[]);
    assert!(arena.now_ms > 0, "a substep ran");
    assert!(out.motor.intents >= 1, "the pre-pass ordered nothing");
    assert_eq!(
        arena
            .unit(red)
            .map(|unit| (unit.desired_speed_cms, unit.orders_in_window)),
        Some((0, 0)),
        "held, and the stop counted against no seat rate"
    );
}

/// **`age_avoid` ages a pilot's row** (17b cut M1a.1): the sweep that stood at
/// the top of the reflex decision, lifted onto [`MotorRow`] so a row no
/// decision runs — a pilot's `DRIVE`, from 17b cut M1a.4 — ages its stops. Two
/// stops and a remembered refusal: before the TTL nothing goes and the answer
/// is `false`; at the first stop's TTL that stop goes, the refusal with it, and
/// the answer is `true`; a sweep that ages nothing answers `false` again. (17b
/// cut M4.5a: "a pilot's tank" is read off the controller and the absent mind
/// row, not the reflex row's vacuous probe, and the TTL off the motor, its
/// home.)
#[test]
fn m1a1_age_avoid_ages_a_pilot_rows_stops_and_drops_its_refusal() {
    let world = booted_world();
    let mut arena = shipped_arena(&world, &[&[Muster::piloted(Team::Red, 1)]]);
    let red = tank(&world, Team::Red, 1);
    assert_eq!(
        arena
            .unit(red)
            .map(|unit| (unit.controller, arena.minds.contains_key(&red))),
        Some((tacsim::arena::state::Controller::Pilot, false)),
        "a pilot's tank, with no mind row"
    );
    let ttl = tacsim::arena::motor::REFLEX_AVOID_TTL_MS;
    let refusal = Refusal {
        at: (3, 3),
        cell: (3, 4),
        cause: BlockedCause::Impassable,
        goal: (3, 9),
        until_ms: ttl,
    };
    {
        let motor = arena.motors.ensure(red);
        motor.avoid = vec![((1, 1), 0), ((2, 2), 20_000)];
        motor.refused = Some(refusal);
    }
    let Some(motor) = arena.motors.row_mut(red) else {
        panic!("the pilot's row");
    };
    assert!(
        !motor.age_avoid(ttl - 1),
        "nothing aged a millisecond early"
    );
    assert_eq!(motor.avoid, vec![((1, 1), 0), ((2, 2), 20_000)]);
    assert_eq!(motor.refused, Some(refusal));

    assert!(motor.age_avoid(ttl), "the first stop aged at its TTL");
    assert_eq!(motor.avoid, vec![((2, 2), 20_000)]);
    assert_eq!(motor.refused, None, "and took the refusal with it");

    assert!(!motor.age_avoid(ttl + 1), "nothing more to age");
    assert!(motor.age_avoid(20_000 + ttl), "the second one's turn");
    assert!(motor.avoid.is_empty());
}

// ---------------------------------------------------------------------------
// 17b cut M1a.4 — a pilot's `DRIVE` (doc 17 §4 D-UM-3, D-UM-10)
// ---------------------------------------------------------------------------

/// The pilot a [`drive_run`] seats in Red Tank 1, so the arena builds frames.
const M1A4_PILOT: ObjId = ObjId(9_001);

/// What one [`drive_run`] produced.
struct DriveRun {
    /// Every ledger line: its time, its kind as JSON, its substep hash.
    events: Vec<(i64, String, u64)>,
    /// The state hash of every substep, in order.
    hashes: Vec<u64>,
    /// How many pulses it took — three cadences really are three numbers.
    pulses: u32,
    /// Every `drive_*` notice the pilot's frames carried, with the arena
    /// window `[start, end]` of the pulse that built the frame.
    told: Vec<(&'static str, i64, i64)>,
    /// After each pulse: the arena time it started at, and the motor's goal.
    goals: Vec<(i64, Option<(u16, u16)>)>,
    /// After each pulse: the arena time it started at, and the speed the tank
    /// was ordered.
    speeds: Vec<(i64, i32)>,
    /// Where the tank ended, and what it was last ordered.
    cell: Option<(u16, u16)>,
    desired_heading_mdeg: u32,
    desired_speed_cms: i32,
    /// Motor ticks left on the heap at the end.
    ticks: usize,
}

/// **Red Tank 1, a pilot's, on the proving ground**, driven at `cadence_ms`
/// to `until_ms` through `beats` — each `(t_ms, kind)` delivered as a seat's
/// order, at the unit's current epoch, in the first pulse whose arena time has
/// reached `t_ms` (every `t_ms` a multiple of 5,000, so one script means one
/// thing at all three cadences: `tests/arena.rs`'s `Beat` rule).
fn drive_run(
    world: &mush_testkit::TestWorld,
    cadence_ms: i64,
    until_ms: i64,
    beats: &[(i64, tacsim::arena::intents::IntentKind)],
) -> DriveRun {
    use tacsim::arena::intents::Intent;
    use tacsim::arena::perception::FrameNotice;
    let mut arena = shipped_arena(world, &[&[Muster::piloted(Team::Red, 1)]]);
    let red = tank(world, Team::Red, 1);
    let _ = arena.seat_pilot(
        M1A4_PILOT,
        red,
        arena
            .epochs
            .current(red, tacsim::arena::state::Channel::Helm),
    );
    arena
        .clock
        .pin_tempo_for_tests(tacsim::arena::clock::Tempo::Quiet);
    let mut bytes: Vec<u8> = Vec::new();
    let mut run = DriveRun {
        events: Vec::new(),
        hashes: Vec::new(),
        pulses: 0,
        told: Vec::new(),
        goals: Vec::new(),
        speeds: Vec::new(),
        cell: None,
        desired_heading_mdeg: 0,
        desired_speed_cms: 0,
        ticks: 0,
    };
    let mut next = 0_usize;
    let mut seq = 0_u64;
    let mut wall = 0_i64;
    loop {
        let start = arena.now_ms;
        let mut intents: Vec<Intent> = Vec::new();
        while let Some((at, kind)) = beats.get(next) {
            if *at > arena.now_ms {
                break;
            }
            seq += 1;
            let channel = kind.channel_required();
            intents.push(Intent {
                unit: red,
                channel,
                epoch: arena.epochs.current(red, channel),
                kind: kind.clone(),
                seq,
            });
            next += 1;
        }
        let out = arena.step(wall, &[], &intents);
        run.pulses += 1;
        for chunk in &out.ledger_chunks {
            bytes.extend_from_slice(&chunk.bytes);
        }
        run.hashes.extend(arena.substep_hashes.iter().copied());
        for (_, frame) in &out.frames {
            for notice in &frame.notices {
                if let FrameNotice::Drive { .. } = notice {
                    run.told.push((notice.kind().as_str(), start, arena.now_ms));
                }
            }
        }
        run.goals
            .push((start, arena.motors.row(red).and_then(|motor| motor.goal)));
        run.speeds.push((
            start,
            arena.unit(red).map_or(0, |unit| unit.desired_speed_cms),
        ));
        if arena.now_ms >= until_ms || out.ended.is_some() {
            break;
        }
        wall += cadence_ms;
    }
    run.events = support::ledger_events(&mut arena, &mut bytes)
        .into_iter()
        .map(|event| {
            (
                event.t_ms,
                serde_json::to_string(&event.kind).unwrap_or_default(),
                event.hash,
            )
        })
        .collect();
    run.cell = cell_of(&arena, red);
    if let Some(unit) = arena.unit(red) {
        run.desired_heading_mdeg = unit.desired_heading_mdeg;
        run.desired_speed_cms = unit.desired_speed_cms;
    }
    run.ticks = arena
        .due
        .iter()
        .filter(|event| event.kind == tacsim::arena::state::DueKind::MotorTick { unit: red })
        .count();
    run
}

/// The three cadences of one script, held to one event list and one hash
/// sequence, answered for the case's own assertions.
fn drive_at_three_cadences(
    world: &mush_testkit::TestWorld,
    until_ms: i64,
    beats: &[(i64, tacsim::arena::intents::IntentKind)],
    case: &str,
) -> Vec<DriveRun> {
    let runs: Vec<DriveRun> = [5_000_i64, 1_000, 500]
        .into_iter()
        .map(|cadence| drive_run(world, cadence, until_ms, beats))
        .collect();
    let [slow, mid, fast] = runs.as_slice() else {
        panic!("three runs");
    };
    assert!(
        slow.pulses < mid.pulses && mid.pulses < fast.pulses,
        "{case}: three cadences, three pulse counts"
    );
    for (cadence, run) in [(1_000, mid), (500, fast)] {
        assert_eq!(
            run.events, slow.events,
            "{case}: the event list at {cadence} ms"
        );
        assert_eq!(
            run.hashes, slow.hashes,
            "{case}: the hash sequence at {cadence} ms"
        );
        assert_eq!(
            run.cell, slow.cell,
            "{case}: the final cell at {cadence} ms"
        );
        let kinds = |run: &DriveRun| run.told.iter().map(|(kind, ..)| *kind).collect::<Vec<_>>();
        assert_eq!(
            kinds(run),
            kinds(slow),
            "{case}: the notices at {cadence} ms"
        );
    }
    // Each notice was staged at one arena millisecond: the windows of the
    // pulses that carried it, one per cadence, have a millisecond in common.
    for index in 0..slow.told.len() {
        let windows: Vec<(i64, i64)> = runs
            .iter()
            .filter_map(|run| run.told.get(index).map(|(_, start, end)| (*start, *end)))
            .collect();
        let from = windows.iter().map(|(start, _)| *start).max().unwrap_or(0);
        let to = windows.iter().map(|(_, end)| *end).min().unwrap_or(0);
        assert!(from <= to, "{case}: notice {index} in windows {windows:?}");
    }
    runs
}

/// **A pilot's `DRIVE` drives the tank to the cell, at three cadences** (17b
/// cut M1a.4; doc 17 §4 D-UM-3 and D-UM-10; doc 16 §3.1's D-16-2 — 16b §11's
/// mandatory cross-tempo fixture for a cut that changes arena behaviour).
///
/// Red Tank 1 on the proving ground, a pilot's, powered up by a scripted
/// `STARTUP` at nought; four scripts, each run at 5,000, 1,000 and 500 ms and
/// held to one event list, one hash sequence and one final cell:
///
/// 1. a `Goal { Cell }` at 15,000 to a cell twenty cells along the road: the
///    tank ends within `ARRIVE_CELLS` of it, the pilot is told `drive_arrived`
///    exactly once (the pulses that carried it share a millisecond at every
///    cadence), the goal is gone and no motor tick is left on the heap;
/// 2. a tank already driving (a `SPEED 300` at 10,000) given a goal on ground
///    its row cannot stand on: `drive_no_route` once, at the landing, the goal
///    dropped, the ordered speed nought — the "holding" of the sentence — no
///    tick left;
/// 3. the first script with a `HEADING 180` at 20,000 — a heading the road's
///    route never gives — the goal is still held after the pulse that ends at
///    20,000 and gone after the one that starts there, at every cadence, and
///    the tank is ordered the typed heading — no notice;
/// 4. the first script with a `DRIVE HOLD` at 20,000: no goal, speed nought,
///    no notice.
#[test]
fn m1a4_a_piloted_drive_drives_to_the_cell_at_three_cadences() {
    use tacsim::arena::intents::{IntentKind, MotorGoal};
    use tacsim::arena::motor::ARRIVE_CELLS;

    let world = booted_world();
    let probe = shipped_arena(&world, &[&[Muster::piloted(Team::Red, 1)]]);
    let red = tank(&world, Team::Red, 1);
    let from = cell_of(&probe, red).expect("red is on the window");
    let terrain = &probe.definition.terrain;
    let row = probe.unit(red).map(|unit| unit.cost_row).expect("red");
    let to = (from.0 + 20, from.1);
    assert!(terrain.passable_for(row, to.0, to.1), "the road at {to:?}");
    // The nearest cell five or more cells off that the tank's row cannot
    // stand on: no route reaches it.
    let sealed = (0..terrain.height())
        .flat_map(|y| (0..terrain.width()).map(move |x| (x, y)))
        .filter(|&(x, y)| !terrain.passable_for(row, x, y))
        .filter(|&(x, y)| x.abs_diff(from.0).max(y.abs_diff(from.1)) >= 5)
        .min_by_key(|&(x, y)| (x.abs_diff(from.0).max(y.abs_diff(from.1)), x, y))
        .expect("the proving ground has ground a tank cannot stand on");
    let cell = |at: (u16, u16)| IntentKind::Goal(MotorGoal::Cell { x: at.0, y: at.1 });

    // 1. Arrival.
    let runs = drive_at_three_cadences(
        &world,
        90_000,
        &[(0, IntentKind::Startup), (15_000, cell(to))],
        "arrival",
    );
    for run in &runs {
        let at = run.cell.expect("on the window");
        assert!(
            at.0.abs_diff(to.0).max(at.1.abs_diff(to.1)) <= ARRIVE_CELLS,
            "arrived at {at:?} for {to:?}"
        );
        let told: Vec<&str> = run.told.iter().map(|(kind, ..)| *kind).collect();
        assert_eq!(told, ["drive_arrived"], "said once");
        assert_eq!(run.goals.last().and_then(|(_, goal)| *goal), None);
        assert_eq!(run.ticks, 0, "no motor tick after arrival");
        assert!(
            run.speeds.iter().any(|(_, speed)| *speed > 0),
            "the motor drove it there"
        );
        assert_eq!(run.desired_speed_cms, 0, "and stopped it on arrival");
    }

    // 2. No route.
    let runs = drive_at_three_cadences(
        &world,
        30_000,
        &[
            (0, IntentKind::Startup),
            (10_000, IntentKind::Speed { cms: 300 }),
            (15_000, cell(sealed)),
        ],
        "no route",
    );
    for run in &runs {
        let moving = run
            .speeds
            .iter()
            .rev()
            .find(|(start, _)| *start < 15_000)
            .map(|(_, speed)| *speed);
        assert_eq!(moving, Some(300), "driving when the goal lands");
        let told: Vec<&str> = run.told.iter().map(|(kind, ..)| *kind).collect();
        assert_eq!(told, ["drive_no_route"], "said once");
        let (_, start, _) = run.told.first().copied().unwrap_or(("", 0, 0));
        assert_eq!(start, 15_000, "said at the landing");
        assert_eq!(run.desired_speed_cms, 0);
        assert_eq!(run.goals.last().and_then(|(_, goal)| *goal), None);
        assert_eq!(run.ticks, 0);
    }

    // 3. A `HEADING` mid-route takes the helm back at its own millisecond.
    let runs = drive_at_three_cadences(
        &world,
        30_000,
        &[
            (0, IntentKind::Startup),
            (15_000, cell(to)),
            (20_000, IntentKind::Heading { mdeg: 180_000 }),
        ],
        "heading",
    );
    for run in &runs {
        let before = run
            .goals
            .iter()
            .rev()
            .find(|(start, _)| *start < 20_000)
            .and_then(|(_, goal)| *goal);
        let after = run
            .goals
            .iter()
            .find(|(start, _)| *start == 20_000)
            .map(|(_, goal)| *goal);
        assert_eq!(before, Some(to), "still driving until the HEADING");
        assert_eq!(after, Some(None), "the HEADING's pulse dropped the goal");
        assert!(run.told.is_empty(), "nothing said: {:?}", run.told);
        assert_eq!(run.desired_heading_mdeg, 180_000, "steering what was typed");
        assert_eq!(run.ticks, 0, "the cadence ended with the goal");
    }

    // 4. `DRIVE HOLD`: a stop, no goal, nothing said.
    let runs = drive_at_three_cadences(
        &world,
        30_000,
        &[
            (0, IntentKind::Startup),
            (15_000, cell(to)),
            (20_000, IntentKind::Goal(MotorGoal::Hold)),
        ],
        "hold",
    );
    for run in &runs {
        assert_eq!(run.desired_speed_cms, 0);
        assert_eq!(run.goals.last().and_then(|(_, goal)| *goal), None);
        assert!(run.told.is_empty(), "a hold says nothing: {:?}", run.told);
    }
}

/// **The fan keeps two same-side goals apart in a far zone** (17b cut M1a.4;
/// the M1a plan's departure (d); 16b cut AI.1's `HUNT_FAN_ROWS`). Two pilots of
/// red `DRIVE` the same passable cell of blue's entry zone in one pulse: the
/// first is given the cell, the second a passable cell `HUNT_FAN_ROWS` or more
/// rows from it (Chebyshev), so their arrival squares do not overlap. The same
/// two orders to a cell outside every far zone are never fanned. And a
/// machine's own hunt fan — `far_entry_cell`, which reads the side's hunt
/// cells it is handed and never a motor row — answers as it did before anyone
/// drove (17b cut M4.5a: blue is a stock mind's hull, `Muster::mind`, where
/// it was the reflex machine's; the kernel never read the controller).
#[test]
fn m1a4_the_fan_keeps_two_same_side_goals_apart_in_a_far_zone() {
    use tacsim::arena::intents::{Intent, IntentKind, MotorGoal};
    use tacsim::arena::motor::HUNT_FAN_ROWS;
    use tacsim::arena::state::Channel;

    let world = booted_world();
    let field = || {
        let mut arena = shipped_arena(
            &world,
            &[
                &[Muster::piloted(Team::Red, 1), Muster::piloted(Team::Red, 2)],
                &[Muster::mind(Team::Blue, 1)],
            ],
        );
        for unit in [tank(&world, Team::Red, 1), tank(&world, Team::Red, 2)] {
            if let Some(state) = arena.unit_mut(unit) {
                state.power = Power::Online;
            }
        }
        arena
    };
    let (one, two) = (tank(&world, Team::Red, 1), tank(&world, Team::Red, 2));
    let blue = tank(&world, Team::Blue, 1);
    let probe = field();
    let row = probe.unit(one).map(|unit| unit.cost_row).expect("red");
    let zone = probe
        .definition
        .entry_zones
        .iter()
        .find(|zone| zone.team == Team::Blue)
        .copied()
        .expect("blue's entry zone");
    let (x0, x1) = (zone.x0.min(zone.x1), zone.x0.max(zone.x1));
    let (y0, y1) = (zone.y0.min(zone.y1), zone.y0.max(zone.y1));
    let mid_y = y0 + (y1 - y0) / 2;
    let target = (x0..=x1)
        .find(|&x| probe.definition.terrain.passable_for(row, x, mid_y))
        .map(|x| (x, mid_y))
        .expect("a passable cell on the zone's middle row");
    let hunt_before = probe.far_entry_cell(blue, &[]);

    let drive_both = |arena: &mut Arena, at: (u16, u16)| {
        let intents: Vec<Intent> = [one, two]
            .into_iter()
            .enumerate()
            .map(|(seq, unit)| {
                let kind = IntentKind::Goal(MotorGoal::Cell { x: at.0, y: at.1 });
                Intent {
                    unit,
                    channel: Channel::Helm,
                    epoch: arena.epochs.current(unit, Channel::Helm),
                    kind,
                    seq: u64::try_from(seq).unwrap_or(0) + 1,
                }
            })
            .collect();
        let _ = arena.step(0, &[], &intents);
        (
            arena.motors.row(one).and_then(|motor| motor.goal),
            arena.motors.row(two).and_then(|motor| motor.goal),
        )
    };

    let mut arena = field();
    let (first, second) = drive_both(&mut arena, target);
    assert_eq!(first, Some(target), "the first is given the cell");
    let second = second.expect("the second has a goal");
    assert!(
        second.0.abs_diff(target.0).max(second.1.abs_diff(target.1)) >= HUNT_FAN_ROWS,
        "{second:?} overlaps {target:?}"
    );
    assert!(
        (x0..=x1).contains(&second.0) && (y0..=y1).contains(&second.1),
        "fanned inside the zone: {second:?}"
    );
    assert!(
        arena
            .definition
            .terrain
            .passable_for(row, second.0, second.1),
        "fanned onto passable ground: {second:?}"
    );
    assert_eq!(
        arena.fan_goal(blue, target),
        target,
        "blue's own zone fans nothing"
    );
    assert_eq!(
        arena.far_entry_cell(blue, &[]),
        hunt_before,
        "the machine's hunt reads no motor row"
    );

    // Outside every far zone: never fanned.
    let mut arena = field();
    let open = cell_of(&arena, one)
        .map(|(x, y)| (x + 3, y))
        .expect("red is on the window");
    assert!(
        !arena
            .definition
            .entry_zones
            .iter()
            .any(|zone| zone.team != Team::Red
                && (zone.x0.min(zone.x1)..=zone.x0.max(zone.x1)).contains(&open.0)
                && (zone.y0.min(zone.y1)..=zone.y0.max(zone.y1)).contains(&open.1)),
        "the premise: {open:?} is in no far zone"
    );
    assert_eq!(drive_both(&mut arena, open), (Some(open), Some(open)));
}

/// **A pilot's motor row ages its stops on the motor's own tick** (17b cut
/// M1a.4; the M1a plan's §4 D2): a row no reflex decision runs is swept by
/// `motor_step`, so a remembered stop thirty seconds old and the refusal it
/// carried are gone at the next `MotorTick`, as a machine's are at its tick.
#[test]
fn m1a4_a_pilots_motor_row_ages_its_stops_on_the_tick() {
    use tacsim::arena::intents::{Intent, IntentKind, MotorGoal};
    use tacsim::arena::state::{Channel, DueKind};

    let world = booted_world();
    let mut arena = shipped_arena(&world, &[&[Muster::piloted(Team::Red, 1)]]);
    let red = tank(&world, Team::Red, 1);
    if let Some(state) = arena.unit_mut(red) {
        state.power = Power::Online;
    }
    let from = cell_of(&arena, red).expect("on the window");
    let to = (from.0 + 20, from.1);
    let _ = arena.step(
        0,
        &[],
        &[Intent {
            unit: red,
            channel: Channel::Helm,
            epoch: arena.epochs.current(red, Channel::Helm),
            kind: IntentKind::Goal(MotorGoal::Cell { x: to.0, y: to.1 }),
            seq: 1,
        }],
    );
    let tick = arena
        .due
        .iter()
        .find(|event| event.kind == DueKind::MotorTick { unit: red })
        .map(|event| event.t_ms)
        .expect("the goal put the hull on the motor's cadence");
    let ttl = tacsim::arena::motor::REFLEX_AVOID_TTL_MS;
    {
        let Some(motor) = arena.motors.row_mut(red) else {
            panic!("the pilot's row");
        };
        motor.avoid = vec![((1, 1), tick - ttl)];
        motor.refused = Some(Refusal {
            at: (1, 1),
            cell: (1, 2),
            cause: BlockedCause::Impassable,
            goal: to,
            until_ms: tick,
        });
    }
    let substep = i64::from(tacsim::arena::clock::substep_ms(
        &arena.arena_cfg.tempo,
        arena.cell_cm(),
        arena.max_speed_cms(),
    ));
    let mut wall = 0_i64;
    while arena.now_ms < tick {
        wall += substep;
        let _ = arena.step(wall, &[], &[]);
    }
    let motor = arena.motors.row(red).cloned().expect("the pilot's row");
    assert!(
        motor.avoid.is_empty(),
        "the old stop aged: {:?}",
        motor.avoid
    );
    assert_eq!(motor.refused, None, "and took its refusal with it");
    assert_eq!(motor.goal, Some(to), "the drive goes on");
}

/// **A goal is driven at its landing millisecond** (17b cut M1a.4; doc 17 §4
/// D-UM-10: *when a `Goal` lands, the motor runs one `follow` at the landing
/// millisecond, so a new goal takes effect at N+1 and does not wait for the
/// next motor tick*). A pilot's powered tank is given a goal twenty cells along
/// the road in the opening pulse, which ends at the millisecond it began: the
/// hull's first `MotorTick` is still on the heap, inside the first `decide_ms`
/// (so it has not run), and yet the motor has already searched at nought, laid
/// a route and ordered the tank a speed — which only the landing's own step
/// can have done.
#[test]
fn m1a4_a_goal_is_driven_at_its_landing_millisecond() {
    use tacsim::arena::intents::{Intent, IntentKind, MotorGoal};
    use tacsim::arena::state::{Channel, DueKind};

    let world = booted_world();
    let mut arena = shipped_arena(&world, &[&[Muster::piloted(Team::Red, 1)]]);
    let red = tank(&world, Team::Red, 1);
    if let Some(state) = arena.unit_mut(red) {
        state.power = Power::Online;
    }
    let from = cell_of(&arena, red).expect("on the window");
    let to = (from.0 + 20, from.1);
    let _ = arena.step(
        0,
        &[],
        &[Intent {
            unit: red,
            channel: Channel::Helm,
            epoch: arena.epochs.current(red, Channel::Helm),
            kind: IntentKind::Goal(MotorGoal::Cell { x: to.0, y: to.1 }),
            seq: 1,
        }],
    );
    assert_eq!(arena.now_ms, 0, "the opening pulse ends where it began");
    let ticks: Vec<i64> = arena
        .due
        .iter()
        .filter(|event| event.kind == DueKind::MotorTick { unit: red })
        .map(|event| event.t_ms)
        .collect();
    let decide_ms = i64::from(arena.arena_cfg.reflex.decide_ms);
    assert!(
        matches!(ticks.as_slice(), [tick] if *tick < decide_ms),
        "the first motor tick has not run: {ticks:?}"
    );
    let motor = arena.motors.row(red).cloned().expect("the pilot's row");
    assert_eq!(motor.goal, Some(to));
    assert_eq!(motor.searched_ms, Some(0), "searched at the landing");
    assert!(!motor.route.is_empty(), "and a route laid");
    assert!(
        arena
            .unit(red)
            .is_some_and(|unit| unit.desired_speed_cms > 0),
        "and the tank ordered on"
    );
}

/// **An engine killed under a `DRIVE` lets go of the goal and says so** (17b
/// cut M1a.4). A motion kill leaves no wreck, the pilot cannot let go of the
/// goal (the cockpit's and the arena's motion gates refuse `DRIVE OFF` on a
/// tank that cannot move), and the motor could never reach it — so the next
/// `MotorTick` drops it, the pilot is told `drive_no_route` once, and the
/// cadence ends with it rather than driving a dead hull every `decide_ms` for
/// the rest of the match.
#[test]
fn m1a4_an_engine_killed_under_a_drive_lets_go_and_says_so() {
    use tacsim::arena::intents::{Intent, IntentKind, MotorGoal};
    use tacsim::arena::perception::FrameNotice;
    use tacsim::arena::state::{Channel, DueKind};

    let world = booted_world();
    let mut arena = shipped_arena(&world, &[&[Muster::piloted(Team::Red, 1)]]);
    let red = tank(&world, Team::Red, 1);
    let _ = arena.seat_pilot(M1A4_PILOT, red, arena.epochs.current(red, Channel::Helm));
    if let Some(state) = arena.unit_mut(red) {
        state.power = Power::Online;
    }
    let from = cell_of(&arena, red).expect("on the window");
    let to = (from.0 + 20, from.1);
    let _ = arena.step(
        0,
        &[],
        &[Intent {
            unit: red,
            channel: Channel::Helm,
            epoch: arena.epochs.current(red, Channel::Helm),
            kind: IntentKind::Goal(MotorGoal::Cell { x: to.0, y: to.1 }),
            seq: 1,
        }],
    );
    let motor_ticks = |arena: &Arena| {
        arena
            .due
            .iter()
            .filter(|event| event.kind == DueKind::MotorTick { unit: red })
            .count()
    };
    assert_eq!(motor_ticks(&arena), 1, "driving");
    if let Some(state) = arena.unit_mut(red) {
        state.limits.motion_disabled = true;
    }
    let mut told: Vec<&'static str> = Vec::new();
    let mut wall = 0_i64;
    while arena.now_ms < 10_000 {
        wall += 500;
        let out = arena.step(wall, &[], &[]);
        for (_, frame) in &out.frames {
            for notice in &frame.notices {
                if let FrameNotice::Drive { .. } = notice {
                    told.push(notice.kind().as_str());
                }
            }
        }
    }
    assert_eq!(told, ["drive_no_route"], "said once");
    assert_eq!(arena.motors.row(red).and_then(|motor| motor.goal), None);
    assert_eq!(motor_ticks(&arena), 0, "the cadence ended with the goal");
}

// ---------------------------------------------------------------------------
// 17b cut M4.4 — motor family A (bands, wreck, follower, bluff)
// ---------------------------------------------------------------------------
//
// The reflex fixtures of `tests/reflex.rs` whose every edge was the motor's
// (`Arena::follow`, `Arena::note_blocked`, `MotorRow::age_avoid`,
// `Arena::refresh_held_avoid`, `MotorTable::may_search`, the routeless steer
// and `Arena::hold_routeless`), moved here by 17b cut M4.4 onto the
// sixteen-cell rig (`support::rig`) with the hull a `Controller::Mind`'s,
// flown by a scripted mind (`support::scripted::ScriptedMind`) whose goals
// land at the reflex's own milliseconds: think `n` at `think_at(unit, n)`
// (`stagger_ms(SEED, unit, 1_000) + n × 1_000`, `RED`'s at 482), its answer
// landing one substep (250 ms) on and driven there (`Arena::motor_step` at
// the landing, D-UM-10), then the hull's own `MotorTick` every `decide_ms`
// (500) on the stagger grid — the reflex tick's own slots. The reflex machine
// set its first goal at its tick 982 ms in, online; the scripted mind's first
// goal lands at 1,732 (think 1, the first online think: think 0's `Startup`
// lands at 732 and completes at 1,232), so each window below is the reflex
// fixture's own, grown by that three quarters of a second only where a claim
// needed it, and the edges — the stop, the learned cell, the re-search within
// one tick, the other gap taken, the deferred search, the hold at the bluff,
// the stamp a held hull keeps — do not depend on the millisecond. What the
// reflex fixtures read off the controller (`state == Patrol`,
// `reflex.unit(RED).is_some()`) becomes "the goal is still held" where the
// claim needs it and is dropped where the claim was the motor's alone.

/// One scripted goal cell.
fn m44a_goal(x: u16, y: u16) -> tacsim::arena::intents::IntentKind {
    tacsim::arena::intents::IntentKind::Goal(tacsim::arena::intents::MotorGoal::Cell { x, y })
}

/// **The family's script on `unit`**: `Startup` at think 0 and one intent
/// list at each think `lines` names.
fn m44a_script(
    unit: ObjId,
    lines: &[(u64, Vec<tacsim::arena::intents::IntentKind>)],
) -> std::sync::Arc<support::scripted::ScriptedMind> {
    let mut script = vec![support::scripted::says(
        0,
        vec![tacsim::arena::intents::IntentKind::Startup],
    )];
    for (think, intents) in lines {
        script.push(support::scripted::says(*think, intents.clone()));
    }
    support::scripted::ScriptedMind::new(&[(unit, script)])
}

/// The millisecond `unit`'s think `n` lands at — its answer driven there.
fn m44a_lands_at(arena: &Arena, unit: ObjId, n: u64) -> i64 {
    support::scripted::think_at(unit, i64::try_from(n).unwrap_or(0)) + arena.substep_len_ms()
}

/// Step `arena` to `until_ms` in pulses of `pulse_ms`, settled between pulses
/// as the task's pool settles (M4.1's gate), absorbing the ledger and the
/// counters, and answer the west-most centimetre `RED` reached — which is
/// "did it get past", for a hull that may be driving back east at the end.
fn m44a_drive(
    arena: &mut Arena,
    until_ms: i64,
    pulse_ms: i64,
    bytes: &mut Vec<u8>,
    counters: &mut MotorCounters,
) -> i64 {
    let mut wall = arena.now_ms;
    let mut west_most = i64::MAX;
    while arena.now_ms < until_ms {
        let out = arena.step_settled(wall, &[], &[]);
        rig::absorb(&out, bytes);
        counters.absorb(&out.motor);
        west_most = west_most.min(arena.unit(rig::RED).map_or(i64::MAX, |unit| unit.pose.x_cm));
        if out.ended.is_some() {
            break;
        }
        wall += pulse_ms;
    }
    west_most
}

/// The think whose `Goal(Cell (14, 8))` is the band fixture's turnaround:
/// the first think after the hull let go of `west` on arrival (a mind's goal
/// is dropped at arrival under `Fallback::Notice`, and the motor's
/// `drive_arrived` is drained into exactly the next think's view) — measured
/// on this run (the arrival falls between 19,732 and 20,000; think 20 is at
/// 20,482, its goal landing at 20,732) and asserted by the test, so the
/// second goal is the reflex's `Patrol` turnaround and never a redirect of a
/// hull still on its way.
const M44A_TURNAROUND_THINK: u64 = 20;

/// **A hull held against a band keeps or re-plans its route and never gives
/// up** — 16b cut N.1's (f) as 16b cut AI.1 left it (`tests/reflex.rs`'
/// `a_tank_held_against_a_band_keeps_or_re_plans_its_route_and_never_gives_up`,
/// moved here by 17b cut M4.4). An impassable band down column 4 with one
/// gap at row 0: `Goal(Cell (1, 8))` — the graph's `west` node — lands at the
/// first online think (1,732) and the hull finds its way round the band
/// rather than sitting against it: searches spent, west of column 4, and
/// never stopped on the way (the gap's corner is stepped through its open
/// shoulder). The reflex machine turned round at `west` for `east` at about
/// 19 s (its `Patrol` walk); here the turnaround is the script's,
/// `Goal(Cell (14, 8))` at [`M44A_TURNAROUND_THINK`], the first think landing
/// after the hull let the first goal go on arrival, so at 30 s it is back
/// east of the band, under way on a route to a goal it still holds, every
/// cell of it passable — the reflex's "still patrolling toward a goal it has
/// a route to" (`state == Patrol`, `reflex.unit(RED).is_some()`) read as the
/// held goal. Everything else is `Arena::follow`'s and read as it was.
#[test]
fn m44a_a_hull_held_against_a_band_keeps_or_re_plans_its_route_and_never_gives_up() {
    use tacsim::arena::perception::{DriveOutcome, FrameNotice};
    use tacsim::arena::state::{Controller, TerrainCell};

    let band: Vec<(u16, u16, TerrainCell)> =
        (1..rig::SIDE).map(|y| (4_u16, y, rig::wall())).collect();
    let mut arena = rig::arena_of(
        vec![rig::tank(
            rig::RED,
            Team::Red,
            rig::centre(6, 8),
            Vec::new(),
            Controller::Mind,
        )],
        &band,
        &rig::arena_cfg(rig::reflex_cfg()),
        &[],
    );
    let mind = m44a_script(
        rig::RED,
        &[
            (1, vec![m44a_goal(1, 8)]),
            (M44A_TURNAROUND_THINK, vec![m44a_goal(14, 8)]),
        ],
    );
    arena.mind_runner = mind.clone();
    let mut bytes: Vec<u8> = Vec::new();
    let mut counters = MotorCounters::default();
    // Where it got to is read pulse by pulse, not only at the end (16b cut
    // AI.1): through the gap, the hull reaches `west` and is back east of the
    // band by 30 s, so a reading at the end alone would miss the crossing.
    let west_most = m44a_drive(&mut arena, 30_000, 500, &mut bytes, &mut counters);
    assert!(counters.path_queries > 0, "{counters:?}");
    let log = rig::events(&mut arena, &mut bytes);
    let blocked = rig::count(&log, "blocked");
    assert!(
        west_most < 4_000,
        "it never got past the band: west-most {west_most} cm, {blocked} stops"
    );
    assert_eq!(
        blocked, 0,
        "it was stopped on its way through the gap: {blocked} stops"
    );
    // The arrival at `west`: the motor tells the mind `drive_arrived` once,
    // drained into the view of the next think — and that think is the
    // turnaround's, so the second goal was thought, let alone landed, only
    // after the hull had reached `west` and let the first go.
    let told_arrived: Vec<u64> = mind
        .seen()
        .iter()
        .filter(|job| {
            job.unit == rig::RED
                && job.view.notices.iter().any(|notice| {
                    matches!(
                        notice,
                        FrameNotice::Drive {
                            outcome: DriveOutcome::Arrived
                        }
                    )
                })
        })
        .map(|job| job.think_seq)
        .collect();
    assert_eq!(
        told_arrived,
        vec![M44A_TURNAROUND_THINK],
        "the think handed the arrival at `west` is the turnaround's"
    );
    // At the end of the run it is not sitting against the band: under way on
    // a route, bound for the goal it holds.
    assert!(
        arena
            .unit(rig::RED)
            .is_some_and(|unit| unit.speed_cms > 0 && unit.desired_speed_cms > 0),
        "the run ended with the hull standing: {:?}",
        arena
            .unit(rig::RED)
            .map(|unit| (unit.pose.x_cm, unit.pose.y_cm, unit.speed_cms))
    );
    let drive = rig::motor(&arena, rig::RED);
    assert!(
        !drive.route.is_empty(),
        "the run ended with the hull routeless"
    );
    assert_eq!(drive.goal, Some((14, 8)), "the goal is still held");
    for (x, y) in &drive.route {
        assert!(
            arena
                .definition
                .terrain
                .get(*x, *y)
                .is_some_and(TerrainCell::passable),
            "the route crosses the band at ({x}, {y})"
        );
    }
}

/// A hull of the path-budget field: id, side, start cell and goal cell.
type M44Hull = (ObjId, Team, (u16, u16), (u16, u16));

/// **The path budget is a bound the arena applies per window of arena time**
/// — 16b cut N.1's (i) (`tests/reflex.rs`'
/// `the_path_budget_is_a_bound_the_arena_applies_per_window_of_arena_time`,
/// moved here by 17b cut M4.4). Four hulls, two a side, each a mind's, each
/// handed a far goal at its first online think, on a budget of one search a
/// `decide_ms` (500) window: `MotorTable::may_search` is spent by
/// `Arena::follow` alone and nothing reads the controller, so the bound is
/// the pulse's count — never more than one search in a 500 ms pulse — and it
/// bites: three of the four goals land inside one window (ids 4, 5 and 7
/// land at 1,732, 1,638 and 1,844; id 6 at 1,254), so the landings after the
/// first are deferred, and their own ticks retry them a window on. Every
/// hull still holds its goal and a route at the end: a drive the budget
/// refused keeps the path it has and carries on — the reflex's "still
/// patrolling" read as the held goal and route.
#[test]
fn m44a_the_path_budget_is_a_bound_the_arena_applies_per_window_of_arena_time() {
    use support::scripted::{Says, ScriptedMind, says};
    use tacsim::arena::intents::IntentKind;
    use tacsim::arena::state::Controller;

    let cfg = rig::arena_cfg(tacsim::config::ReflexConfig {
        max_path_queries: 1,
        ..rig::reflex_cfg()
    });
    // Each in its own column, driving nine cells south: nobody arrives inside
    // the six seconds and nobody is in anybody's way.
    let hulls: [M44Hull; 4] = [
        (ObjId(4), Team::Red, (6, 6), (6, 15)),
        (ObjId(5), Team::Red, (7, 6), (7, 15)),
        (ObjId(6), Team::Blue, (8, 6), (8, 15)),
        (ObjId(7), Team::Blue, (9, 6), (9, 15)),
    ];
    let mut arena = rig::arena_of(
        hulls
            .iter()
            .map(|(id, team, at, _)| {
                rig::tank(
                    *id,
                    *team,
                    rig::centre(at.0, at.1),
                    Vec::new(),
                    Controller::Mind,
                )
            })
            .collect(),
        &[],
        &cfg,
        &[],
    );
    let script: Vec<(ObjId, Vec<Says>)> = hulls
        .iter()
        .map(|(id, _, _, to)| {
            (
                *id,
                vec![
                    says(0, vec![IntentKind::Startup]),
                    says(1, vec![m44a_goal(to.0, to.1)]),
                ],
            )
        })
        .collect();
    arena.mind_runner = ScriptedMind::new(&script);
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall = 0_i64;
    let mut worst = 0_u32;
    let mut deferred = 0_u32;
    while arena.now_ms < 6_000 {
        let out = arena.step_settled(wall, &[], &[]);
        rig::absorb(&out, &mut bytes);
        // The window is `decide_ms` (500) and the pulse is 500, so one pulse
        // holds at most one window and the per-pulse count is the per-window
        // bound.
        assert!(
            out.motor.path_queries <= 1,
            "{} searches in one pulse at a budget of 1",
            out.motor.path_queries
        );
        worst = worst.max(out.motor.path_queries);
        deferred = deferred.saturating_add(out.motor.path_deferred);
        wall += 500;
    }
    assert!(worst > 0, "no search ran at all, so nothing was bounded");
    assert!(
        deferred > 0,
        "nobody was ever refused, so the bound was never reached"
    );
    // Every hull is still driving: one the budget refused keeps its path and
    // carries on rather than stopping.
    for (id, _, _, to) in &hulls {
        let drive = rig::motor(&arena, *id);
        assert_eq!(drive.goal, Some(*to), "{id} holds its goal");
        assert!(!drive.route.is_empty(), "{id} holds a route");
    }
}

/// **A hull stopped by a parked one learns the cell and goes round it** (16b
/// cut Q.7's blocked edge learned at the event; `tests/reflex.rs`'
/// `a_reflex_tank_stopped_by_a_parked_one_learns_the_cell_and_goes_round_it`,
/// moved here by 17b cut M4.4). Doc 16 §3.4 step 5: one unit per cell. The
/// blue tank is parked in the red one's way and nobody is driving it; the
/// landing's search (the search reads the ground and the avoid list, never
/// the index) routes through (5, 8), motion refuses the crossing `Occupied`,
/// the hold matures into one `blocked` line, `Arena::note_blocked` remembers
/// the cell and clears the route and the search stamp, and the next
/// `MotorTick` (the reflex's own stagger slot, within `decide_ms`) searches
/// round it: one stop at (5, 8), a second search, and on to `west` at (1, 8)
/// — cell 2, under `ARRIVE_CELLS`. The reflex's `state == Patrol` is dropped:
/// the goal is the script's.
#[test]
fn m44a_a_hull_stopped_by_a_parked_one_learns_the_cell_and_goes_round_it() {
    use tacsim::arena::state::Controller;

    let mut arena = rig::arena_of(
        vec![
            rig::tank(
                rig::RED,
                Team::Red,
                rig::centre(6, 8),
                Vec::new(),
                Controller::Mind,
            ),
            rig::tank(
                rig::BLUE,
                Team::Blue,
                rig::centre(5, 8),
                Vec::new(),
                Controller::Pilot,
            ),
        ],
        &[],
        &rig::arena_cfg(rig::reflex_cfg()),
        &[rig::BLUE],
    );
    arena.mind_runner = m44a_script(rig::RED, &[(1, vec![m44a_goal(1, 8)])]);
    let mut bytes: Vec<u8> = Vec::new();
    let mut counters = MotorCounters::default();
    let west_most = m44a_drive(&mut arena, 30_000, 500, &mut bytes, &mut counters);
    let log = rig::events(&mut arena, &mut bytes);
    assert!(
        rig::blocked_at(&log, rig::RED, (5, 8)) <= 2,
        "one obstacle is one stop, not a stop every pulse: {} stops at (5, 8)",
        rig::blocked_at(&log, rig::RED, (5, 8))
    );
    assert!(
        counters.path_queries >= 2,
        "it searched again after the stop: {counters:?}"
    );
    assert!(
        west_most < 3_000,
        "it went round the parked tank and on to `west` at (1, 8), which `ARRIVE_CELLS` \
         makes cell 2: west-most {west_most} cm"
    );
}

/// **A stop and a shove are both re-planned within one decide tick** at the
/// *shipped* `repath_ms` (16b cut Q.7; `tests/reflex.rs`'
/// `a_stop_and_a_shove_are_both_re_planned_within_one_decide_tick`, moved
/// here by 17b cut M4.4), which is the clearing of `searched_ms` and nothing
/// else. Every other test of the family runs at `repath_ms` 0, where a
/// route-empty drive searches whether or not anything cleared the stamp, so
/// none of them can see the clearing; this one runs at the shipped four
/// seconds, where the damper is real. Both clearing sites are the motor's:
/// (a) the stop, through `Arena::note_blocked`'s `Occupied` arm; (b) the
/// shove, through `Arena::follow`'s wandered-off branch — a hull moved four
/// cells off its route holds a route whose next cell is not a neighbour and
/// says so at once. The `MotorTick` cadence is `decide_ms` on the stagger
/// grid, so "within one decide tick" is the same bound it was.
#[test]
fn m44a_a_stop_and_a_shove_are_both_re_planned_within_one_decide_tick() {
    use tacsim::arena::state::Controller;

    const DECIDE_MS: i64 = 500;
    let cfg = rig::arena_cfg(tacsim::config::ReflexConfig {
        decide_ms: 500,
        repath_ms: 4_000,
        ..tacsim::config::ReflexConfig::default()
    });
    let mut arena = rig::arena_of(
        vec![
            rig::tank(
                rig::RED,
                Team::Red,
                rig::centre(6, 8),
                Vec::new(),
                Controller::Mind,
            ),
            rig::tank(
                rig::BLUE,
                Team::Blue,
                rig::centre(5, 8),
                Vec::new(),
                Controller::Pilot,
            ),
        ],
        &[],
        &cfg,
        &[rig::BLUE],
    );
    arena.mind_runner = m44a_script(rig::RED, &[(1, vec![m44a_goal(1, 8)])]);
    let mut bytes: Vec<u8> = Vec::new();
    let searched = |arena: &Arena| arena.motors.row(rig::RED).and_then(|row| row.searched_ms);

    // (a) the stop. The stamp the stop threw away is read from the pulse
    // before it, because `Arena::note_blocked` is applied ahead of the due
    // loop and the very substep that stopped the hull may already carry the
    // tick that re-plans it.
    let mut wall_ms = 0_i64;
    let mut held = searched(&arena);
    let mut learned_at: Option<i64> = None;
    let mut spent: Option<i64> = None;
    let mut replanned: Option<i64> = None;
    while arena.now_ms < 30_000 {
        let before = held;
        let out = arena.step_settled(wall_ms, &[], &[]);
        rig::absorb(&out, &mut bytes);
        wall_ms += 100;
        held = searched(&arena);
        if learned_at.is_none() && !rig::avoided(&arena, rig::RED).is_empty() {
            learned_at = Some(arena.now_ms);
            spent = before;
        }
        if learned_at.is_some() && held.is_some() && held != spent {
            replanned = held;
            break;
        }
    }
    let learned = learned_at.expect("the parked tank stopped it");
    assert!(
        spent.is_some(),
        "it was following a route it had searched for"
    );
    let replanned = replanned.expect("it never searched again");
    assert!(
        replanned - learned <= DECIDE_MS,
        "the re-plan is the next tick and not the next `repath_ms` ({} ms): \
         stopped at {learned} ms, searched at {replanned} ms",
        cfg.reflex.repath_ms
    );

    // (b) the shove. It holds a fresh route now; four cells north of it is a
    // route whose next cell is nobody's neighbour.
    let stamp = searched(&arena);
    assert_eq!(
        stamp,
        Some(replanned),
        "it holds the search it just paid for"
    );
    assert!(
        arena
            .motors
            .row(rig::RED)
            .is_some_and(|row| !row.route.is_empty()),
        "and the route that came out of it"
    );
    if let Some(state) = arena.unit_mut(rig::RED) {
        state.pose = rig::centre(6, 12);
    }
    let shoved_at = arena.now_ms;
    let mut re_searched: Option<i64> = None;
    while arena.now_ms < shoved_at + 3_000 {
        let out = arena.step_settled(wall_ms, &[], &[]);
        rig::absorb(&out, &mut bytes);
        wall_ms += 100;
        if searched(&arena) != stamp && searched(&arena).is_some() {
            re_searched = Some(arena.now_ms);
            break;
        }
    }
    let re_searched = re_searched.expect("a shoved tank never asked for a new route");
    assert!(
        re_searched - shoved_at <= DECIDE_MS,
        "a tank pushed off its route re-plans on its next tick and not in \
         {}: shoved at {shoved_at} ms, searched at {re_searched} ms",
        cfg.reflex.repath_ms
    );
}

/// **The band with two gaps and a wreck in the nearer one**, a mind's hull
/// scripted to `west` (`tests/reflex.rs`' `wreck_detour`, moved here by 17b
/// cut M4.4): a band down column 4 with gaps at rows 8 and 2, the red hull on
/// row 8 and a wreck — a pilot's hull, wrecked by hand — standing in the gap
/// in front of it; `Goal(Cell (1, 8))` at the first online think, the tempo
/// pinned `Quiet` so replays of it differ in cadence and in nothing else.
fn m44a_wreck_detour() -> Arena {
    use tacsim::arena::state::{Controller, TerrainCell};

    let band: Vec<(u16, u16, TerrainCell)> = (0..rig::SIDE)
        .filter(|y| *y != 8 && *y != 2)
        .map(|y| (4_u16, y, rig::wall()))
        .collect();
    let mut arena = rig::arena_of(
        vec![
            rig::tank(
                rig::RED,
                Team::Red,
                rig::centre(6, 8),
                Vec::new(),
                Controller::Mind,
            ),
            rig::tank(
                rig::RED2,
                Team::Red,
                rig::centre(4, 8),
                Vec::new(),
                Controller::Pilot,
            ),
        ],
        &band,
        &rig::arena_cfg(rig::reflex_cfg()),
        &[],
    );
    rig::wreck_it(&mut arena, rig::RED2);
    arena.mind_runner = m44a_script(rig::RED, &[(1, vec![m44a_goal(1, 8)])]);
    arena
        .clock
        .pin_tempo_for_tests(tacsim::arena::clock::Tempo::Quiet);
    arena
}

/// **A wreck in the causeway is learned and the other gap is taken** (16b cut
/// Q.7; `tests/reflex.rs`'
/// `a_wreck_in_the_causeway_is_learned_and_the_other_gap_is_taken`, moved
/// here by 17b cut M4.4). The shorter way is through the gap in front of the
/// hull, which a wreck is standing in: the first route goes through (4, 8),
/// one `Occupied` refusal matures into one stop, `Arena::note_blocked`
/// learns the cell, and the re-search avoids it and plans via (4, 2) — one
/// stop at (4, 8), and through the other gap before arrival drops the goal.
#[test]
fn m44a_a_wreck_in_the_causeway_is_learned_and_the_other_gap_is_taken() {
    let mut arena = m44a_wreck_detour();
    let mut bytes: Vec<u8> = Vec::new();
    let mut counters = MotorCounters::default();
    let west_most = m44a_drive(&mut arena, 60_000, 500, &mut bytes, &mut counters);
    let log = rig::events(&mut arena, &mut bytes);
    assert!(
        rig::blocked_at(&log, rig::RED, (4, 8)) <= 2,
        "one wreck is one stop: {} stops at (4, 8)",
        rig::blocked_at(&log, rig::RED, (4, 8))
    );
    assert!(
        west_most < 4_000,
        "it is through the other gap: west-most {west_most} cm"
    );
}

/// **The avoided cells survive a goal that moves every think** (16b cut Q.7's
/// "entries age, never cleared on goal change", `MotorRow::avoid`;
/// `tests/reflex.rs`'
/// `the_avoided_cells_survive_an_approach_whose_goal_moves_every_tick`, moved
/// here by 17b cut M4.4). The case the owner hit: the reflex's `Approach`
/// recomputed its goal from the contact's plot at every tick, so a list
/// cleared on a goal change would be empty again about once a second and the
/// hull would drive back into the wreck it had just learned about, for ever.
/// Here the goal moves every think — the script alternates `Goal(Cell (12,
/// 8))` and `Goal(Cell (13, 8))`, two cells beyond the wreck at (7, 8) on the
/// hull's own row — and each landing clears the route and the remembered
/// stop (`land_goal`) and re-searches (`repath_ms` 0) with the avoid list
/// kept: the wreck's cell is remembered, and the hull gets past it rather
/// than queueing behind it. The reflex's `Approach` assertion is dropped —
/// the goal is the script's, the hull is unarmed.
#[test]
fn m44a_the_avoided_cells_survive_a_goal_that_moves_every_think() {
    use tacsim::arena::state::Controller;

    let mut arena = rig::arena_of(
        vec![
            rig::tank(
                rig::RED,
                Team::Red,
                rig::centre(6, 8),
                Vec::new(),
                Controller::Mind,
            ),
            rig::tank(
                rig::RED2,
                Team::Red,
                rig::centre(7, 8),
                Vec::new(),
                Controller::Pilot,
            ),
        ],
        &[],
        &rig::arena_cfg(rig::reflex_cfg()),
        &[],
    );
    rig::wreck_it(&mut arena, rig::RED2);
    let lines: Vec<(u64, Vec<tacsim::arena::intents::IntentKind>)> = (1..=24)
        .map(|think| {
            (
                think,
                vec![m44a_goal(if think % 2 == 1 { 12 } else { 13 }, 8)],
            )
        })
        .collect();
    arena.mind_runner = m44a_script(rig::RED, &lines);
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall_ms = 0_i64;
    let mut ever_avoided = false;
    while arena.now_ms < 25_000 {
        let out = arena.step_settled(wall_ms, &[], &[]);
        rig::absorb(&out, &mut bytes);
        ever_avoided |= !rig::avoided(&arena, rig::RED).is_empty();
        wall_ms += 500;
    }
    assert!(
        ever_avoided,
        "the wreck's cell was never remembered at all, so nothing was proved"
    );
    assert!(
        arena
            .unit(rig::RED)
            .is_some_and(|unit| unit.pose.x_cm > 7_999),
        "it got past the wreck rather than queueing behind it: {:?}",
        arena.unit(rig::RED).map(|unit| unit.pose.x_cm)
    );
}

/// **A remembered stop ages out and the cell is planned through again** (16b
/// cut Q.7; `tests/reflex.rs`'
/// `a_remembered_stop_ages_out_and_the_cell_is_planned_through_again`, moved
/// here by 17b cut M4.4). The real stop against the parked tank learns (5,
/// 8); the blocker leaves, and `REFLEX_AVOID_TTL_MS` plus two seconds of
/// arena time pass: the entry has aged out (`MotorRow::age_avoid`, run at
/// every `MotorTick` and at every landing's drive), so the search may use the
/// cell again — what stops a wreck that has been cleared from being a
/// permanent hole in the map. The script keeps a goal STANDING across the
/// thirty seconds — `Goal(Cell (1, 8))` through think 8, then `Goal(Cell (14,
/// 8))` and `Goal(Cell (1, 8))` alternately every think — because under
/// `Fallback::Notice` arrival drops a mind's goal and the `MotorTick` cadence
/// with it, and only the drive's sweep ages the list; the reflex's `Patrol`
/// walk kept a goal of its own.
#[test]
fn m44a_a_remembered_stop_ages_out_and_the_cell_is_planned_through_again() {
    use tacsim::arena::motor::REFLEX_AVOID_TTL_MS;
    use tacsim::arena::state::Controller;

    let mut arena = rig::arena_of(
        vec![
            rig::tank(
                rig::RED,
                Team::Red,
                rig::centre(6, 8),
                Vec::new(),
                Controller::Mind,
            ),
            rig::tank(
                rig::BLUE,
                Team::Blue,
                rig::centre(5, 8),
                Vec::new(),
                Controller::Pilot,
            ),
        ],
        &[],
        &rig::arena_cfg(rig::reflex_cfg()),
        &[rig::BLUE],
    );
    let lines: Vec<(u64, Vec<tacsim::arena::intents::IntentKind>)> = (1..=45)
        .map(|think| {
            let x = if think <= 8 || think % 2 == 1 { 1 } else { 14 };
            (think, vec![m44a_goal(x, 8)])
        })
        .collect();
    arena.mind_runner = m44a_script(rig::RED, &lines);
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall_ms = 0_i64;
    let mut learned: Option<(u16, u16)> = None;
    // The reflex fixture's sixteen pulses, and two for the 750 ms by which the
    // goal's landing (1,732) is later than the reflex's first tick (982).
    for _ in 0..18 {
        let out = arena.step_settled(wall_ms, &[], &[]);
        rig::absorb(&out, &mut bytes);
        if let Some(cell) = rig::avoided(&arena, rig::RED).first().copied() {
            learned = Some(cell);
            break;
        }
        wall_ms += 500;
    }
    assert_eq!(learned, Some((5, 8)), "the cell it could not enter");
    // The blocker leaves, and thirty seconds of arena time pass.
    if let Some(state) = arena.unit_mut(rig::BLUE) {
        state.pose = rig::centre(15, 15);
    }
    let at = arena.now_ms;
    let mut counters = MotorCounters::default();
    let _ = m44a_drive(
        &mut arena,
        at + REFLEX_AVOID_TTL_MS + 2_000,
        500,
        &mut bytes,
        &mut counters,
    );
    assert!(
        rig::avoided(&arena, rig::RED).is_empty(),
        "the entry should have aged out at {REFLEX_AVOID_TTL_MS} ms: {:?}",
        rig::avoided(&arena, rig::RED)
    );
}

/// **A hull still held against a wreck past the TTL still avoids its cell**
/// (16b cut M.3, 16c `T-04`, `B-43`'s guarantee under the one-edge rule;
/// `tests/reflex.rs`'
/// `m3_a_machine_held_against_a_wreck_past_the_ttl_still_avoids_its_cell`,
/// moved here by 17b cut M4.4). The `blocked` line and `Arena::note_blocked`
/// fire once per edge, so the stamp refresh a pinned hull used to get from
/// every re-collision is gone; the drive refreshes the stamp off
/// `UnitState::held` instead (`Arena::refresh_held_avoid`, step 1 of
/// `Arena::motor_step`), before its TTL sweep (step 2). The two halves differ
/// in nothing but `held`, set by hand to the value motion leaves a unit
/// pinned against an occupied cell with, and the drive runs at its own
/// arena millisecond past the TTL.
///
/// The drive is a mind hull's today and reached through `Arena::step`: the
/// stop is planted on the row at nought as the reflex fixture planted it
/// (`MotorTable::ensure`, the door a controller takes a hull over through,
/// then `Arena::note_blocked`); think 29's orders (landing at 29,732) push
/// the hull west at the wreck, so motion's parked clause — which clears
/// `held` of a hull ordered nowhere, every substep — leaves the hand-set
/// condition as it finds it (a push toward the held cell inside its own cell
/// is no clear, `motion::moved_away`), and think 30's `Goal(Cell (1, 8))`
/// lands at 30,732, `REFLEX_AVOID_TTL_MS` + 732 after the stop, where the
/// landing runs the drive (D-UM-10) whose step 1 reads `held` and whose step
/// 2 sweeps. The held hull's stamp is the landing's own millisecond; the
/// other's entry is gone. The goal held afterwards is the kernel's
/// precondition met and the drive run (its route is the search's, which the
/// stamp read before the sweep).
#[test]
fn m44a_a_hull_held_against_a_wreck_past_the_ttl_still_avoids_its_cell() {
    use tacsim::arena::intents::IntentKind;
    use tacsim::arena::motion::Held;
    use tacsim::arena::motor::REFLEX_AVOID_TTL_MS;
    use tacsim::arena::state::Controller;

    let wreck_cell = (5_u16, 8_u16);
    for still_held in [true, false] {
        let mut arena = rig::arena_of(
            vec![
                rig::tank(
                    rig::RED,
                    Team::Red,
                    rig::centre(6, 8),
                    Vec::new(),
                    Controller::Mind,
                ),
                // A same-side pilot's hull, wrecked by hand (the reflex
                // fixture wrecked the blue one, and never stepped the arena:
                // stepped, a side with no hull left ends the match).
                rig::tank(
                    rig::RED2,
                    Team::Red,
                    rig::centre(wreck_cell.0, wreck_cell.1),
                    Vec::new(),
                    Controller::Pilot,
                ),
            ],
            &[],
            &rig::arena_cfg(rig::reflex_cfg()),
            &[],
        );
        rig::wreck_it(&mut arena, rig::RED2);
        let push = vec![
            IntentKind::Heading { mdeg: 270_000 },
            IntentKind::Speed { cms: 1_000 },
        ];
        arena.mind_runner = m44a_script(rig::RED, &[(29, push), (30, vec![m44a_goal(1, 8)])]);
        let stopped_at = arena.now_ms;
        arena.motors.ensure(rig::RED);
        arena.note_blocked(rig::RED, wreck_cell, BlockedCause::Occupied, stopped_at);
        assert_eq!(
            rig::avoided(&arena, rig::RED),
            vec![wreck_cell],
            "the stop's own edge"
        );
        let tick_at = m44a_lands_at(&arena, rig::RED, 30);
        assert!(
            tick_at > stopped_at + REFLEX_AVOID_TTL_MS,
            "the drive at {tick_at} ms is past the TTL"
        );
        let substep = arena.substep_len_ms();
        let before = tick_at - tick_at.rem_euclid(substep);
        let mut wall = 0_i64;
        while arena.now_ms < before {
            let out = arena.step_settled(wall, &[], &[]);
            assert!(out.ended.is_none(), "the match ended at {}", arena.now_ms);
            wall += 500;
        }
        assert_eq!(arena.now_ms, before, "the substep before the drive's");
        let unit = arena.unit(rig::RED).expect("the hull");
        assert_eq!(
            arena.definition.terrain.cell_of(&unit.pose),
            Some((6, 8)),
            "pushing at the wreck from its own cell"
        );
        assert!(unit.desired_speed_cms > 0, "ordered on, so not parked");
        assert_eq!(unit.held, None, "no condition of its own yet");
        if let Some(unit) = arena.unit_mut(rig::RED) {
            unit.held = still_held.then_some(Held {
                x: wreck_cell.0,
                y: wreck_cell.1,
                cause: BlockedCause::Occupied,
            });
        }
        let _ = arena.step_settled(before + substep, &[], &[]);
        assert_eq!(arena.now_ms, before + substep, "the drive's substep");
        assert_eq!(
            rig::motor(&arena, rig::RED).goal,
            Some((1, 8)),
            "the goal landed and the drive ran under it"
        );
        let stamps: Vec<((u16, u16), i64)> = arena
            .motors
            .row(rig::RED)
            .map(|row| row.avoid.clone())
            .unwrap_or_default();
        if still_held {
            assert_eq!(
                stamps,
                vec![(wreck_cell, tick_at)],
                "a hull still pushing at the wreck forgot it"
            );
        } else {
            assert!(
                stamps.is_empty(),
                "a hull no longer held against the wreck kept it past the TTL: {stamps:?}"
            );
        }
    }
}

/// **The follower holds rather than drive at a bluff** (16b cut Q.7, closing
/// 16b cut X.6's deviation 13; `tests/reflex.rs`'
/// `the_follower_holds_rather_than_drive_at_a_bluff`'s motor half, moved
/// here by 17b cut M4.4 — its controller half, "a tank with nothing to fire
/// withdraws", is `tests/reflex.rs`'
/// `t1_a_dry_hull_withdraws_to_its_entry_zone`). With no route and no search
/// this drive, the follower used to aim at the **goal**, which on relief is
/// a direction and not a way: the hull drove at the bluff, motion refused the
/// edge, and the whole thing happened again at the next tick. The window has
/// no waypoint graph and the goal — `Goal(Cell (0, 8))`, the entry zone the
/// dry reflex machine withdrew to — is the far side of a bluff down column 4;
/// the search is held DEFERRED (`max_path_queries` 0: `MotorTable::may_search`
/// refuses every drive, `path_deferred` counts it) so the routeless steer
/// (`Arena::follow` step 5) and the one-cell test every substep
/// (`Arena::hold_routeless`) are the only thing between the hull and the
/// cliff — where the reflex's fallback kept a goal a failed search could not
/// reach, a mind's goal would be dropped by `Fallback::Notice` and the hull
/// stopped where it stood. It walks up to the last cell it can enter, (5, 8),
/// and waits there for a route rather than pushing: no `blocked` line, speed
/// nought ordered, the goal still held.
#[test]
fn m44a_the_follower_holds_rather_than_drive_at_a_bluff() {
    use tacsim::arena::state::{Controller, TerrainCell, Waypoints};

    let bluff_wall: Vec<(u16, u16, TerrainCell)> =
        (0..rig::SIDE).map(|y| (4_u16, y, rig::bluff())).collect();
    let cfg = rig::arena_cfg(tacsim::config::ReflexConfig {
        max_path_queries: 0,
        ..rig::reflex_cfg()
    });
    let mut arena = rig::arena_on(
        rig::definition_with(&bluff_wall, Waypoints::new(Vec::new(), &[])),
        vec![rig::tank(
            rig::RED,
            Team::Red,
            rig::centre(6, 8),
            Vec::new(),
            Controller::Mind,
        )],
        &cfg,
        &[],
    );
    if let Some(state) = arena.unit_mut(rig::RED) {
        // The stock locomotion's own limit; the rig's hand-made unit declares
        // no mobility, so it is written on here rather than read off a spec.
        state.slope_limit_pct = Some(30);
    }
    arena.mind_runner = m44a_script(rig::RED, &[(1, vec![m44a_goal(0, 8)])]);
    let mut bytes: Vec<u8> = Vec::new();
    let mut counters = MotorCounters::default();
    let _ = m44a_drive(&mut arena, 30_000, 500, &mut bytes, &mut counters);
    assert_eq!(
        counters.path_queries, 0,
        "no search was ever spent: {counters:?}"
    );
    assert!(
        counters.path_deferred > 0,
        "every drive asked and was deferred: {counters:?}"
    );
    assert_eq!(
        rig::motor(&arena, rig::RED).goal,
        Some((0, 8)),
        "the goal stands: nothing failed a search for it"
    );
    let log = rig::events(&mut arena, &mut bytes);
    assert_eq!(
        rig::count(&log, "blocked"),
        0,
        "it never drove at the bluff, so motion never refused it an edge"
    );
    let cell = arena
        .unit(rig::RED)
        .and_then(|unit| arena.definition.terrain.cell_of(&unit.pose));
    assert_eq!(
        cell,
        Some((5, 8)),
        "it walked up to the last cell it could enter and stopped there"
    );
    assert_eq!(
        arena.unit(rig::RED).map(|unit| unit.desired_speed_cms),
        Some(0),
        "and it is waiting for a route rather than pushing"
    );
}

/// What one replay of [`m44a_wreck_detour`] produced: the event list, the
/// per-substep hash sequence, the finals — where each hull is, how fast, and
/// the goal its motor holds; the reflex's state column dropped, the goal
/// being the script's — and how many pulses it took (three cadences really
/// are three different numbers of pulses, so an equality of the three is a
/// property and not three copies of one run).
struct M44aReplay {
    events: Vec<(i64, String, u64)>,
    hashes: Vec<u64>,
    finals: Vec<M44aFinal>,
    pulses: u32,
}

/// A hull's end in an [`M44aReplay`]: id, pose, heading, speed and goal.
type M44aFinal = (ObjId, i64, i64, u32, i32, Option<(u16, u16)>);

/// [`m44a_wreck_detour`] replayed at `cadence_ms` until `until_ms`, settled
/// between pulses.
fn m44a_replay(cadence_ms: i64, until_ms: i64) -> M44aReplay {
    let mut arena = m44a_wreck_detour();
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut wall = 0_i64;
    let mut pulses = 0_u32;
    loop {
        let out = arena.step_settled(wall, &[], &[]);
        pulses = pulses.saturating_add(1);
        rig::absorb(&out, &mut bytes);
        hashes.extend(arena.substep_hashes.iter().copied());
        if arena.now_ms >= until_ms || out.ended.is_some() {
            break;
        }
        wall += cadence_ms;
    }
    let events = rig::events(&mut arena, &mut bytes)
        .into_iter()
        .map(|event| {
            (
                event.t_ms,
                serde_json::to_string(&event.kind).unwrap_or_default(),
                event.hash,
            )
        })
        .collect();
    let finals = arena
        .units
        .iter()
        .map(|unit| {
            (
                unit.id,
                unit.pose.x_cm,
                unit.pose.y_cm,
                unit.pose.heading_mdeg,
                unit.speed_cms,
                rig::motor(&arena, unit.id).goal,
            )
        })
        .collect();
    M44aReplay {
        events,
        hashes,
        finals,
        pulses,
    }
}

/// **The wreck detour is one event list and one hash sequence at three
/// cadences** (16b cut Q.7's cross-tempo property, 16b §11 mandatory;
/// `tests/reflex.rs`'
/// `the_wreck_detour_is_one_event_list_and_one_hash_sequence_at_three_cadences`,
/// moved here by 17b cut M4.4 in
/// [`m1a4_a_piloted_drive_drives_to_the_cell_at_three_cadences`]' shape).
/// From ONE scripted goal, `Arena::note_blocked` and the hull's own
/// `MotorTick` re-search reproduce the stop, the learned cell and the other
/// gap, and the replay at 5,000, 1,000 and 500 ms is one event list (with a
/// `blocked` line in it), one per-substep hash sequence and one set of finals.
#[test]
fn m44a_the_wreck_detour_is_one_event_list_and_one_hash_sequence_at_three_cadences() {
    let runs: Vec<M44aReplay> = [5_000_i64, 1_000, 500]
        .into_iter()
        .map(|cadence| m44a_replay(cadence, 40_000))
        .collect();
    let first = runs.first().expect("three replays");
    let kinds: Vec<&str> = first
        .events
        .iter()
        .map(|(_, kind, _)| kind.as_str())
        .collect();
    assert!(
        kinds.iter().any(|kind| kind.contains("blocked")),
        "the replay never hit the wreck, so nothing was compared: {kinds:?}"
    );
    assert!(first.hashes.len() >= 100, "{} substeps", first.hashes.len());
    assert!(
        runs.windows(2).all(|pair| pair[0].pulses < pair[1].pulses),
        "three cadences, three pulse counts"
    );
    for (at, run) in runs.iter().enumerate().skip(1) {
        let cadence = [5_000_i64, 1_000, 500].get(at).copied().unwrap_or(0);
        assert_eq!(
            run.events, first.events,
            "the event list diverged at {cadence} ms"
        );
        assert_eq!(
            run.hashes, first.hashes,
            "the hash sequence diverged at {cadence} ms"
        );
        assert_eq!(
            run.finals, first.finals,
            "the field ended differently at {cadence} ms"
        );
    }
}

/// **What one run of [`m44a_wreck_detour`] did** (16b cut E.3 gave it a third
/// field): every cell the hull remembered being stopped at, sorted; the
/// west-most centimetre it reached; and the arena millisecond the hold
/// against the wreck opened at and the one the stop it matured into was
/// journalled at.
struct M44aDetour {
    avoided: Vec<(u16, u16)>,
    west_most: i64,
    matured: Option<(i64, i64)>,
}

/// The hold is read at each pulse boundary, which is coarser than the substep
/// it opens on; that is enough, because a hold lives two seconds and every
/// pulse here is under six hundred milliseconds, so the reading taken before
/// the stop is the hold that matured into it.
fn m44a_avoided_and_arrival(cadence_ms: i64) -> M44aDetour {
    let mut arena = m44a_wreck_detour();
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall = 0_i64;
    let mut seen: Vec<(u16, u16)> = Vec::new();
    let mut west_most = i64::MAX;
    let mut holds: Vec<(i64, i64)> = Vec::new();
    while arena.now_ms < 40_000 {
        let out = arena.step_settled(wall, &[], &[]);
        rig::absorb(&out, &mut bytes);
        for cell in rig::avoided(&arena, rig::RED) {
            if !seen.contains(&cell) {
                seen.push(cell);
            }
        }
        if let Some(hold) = arena.unit(rig::RED).and_then(|unit| unit.occupied_hold) {
            holds.push((arena.now_ms, hold.since_ms));
        }
        west_most = west_most.min(arena.unit(rig::RED).map_or(i64::MAX, |unit| unit.pose.x_cm));
        wall += cadence_ms;
    }
    let log = rig::events(&mut arena, &mut bytes);
    let stopped_at = log
        .iter()
        .find(|event| {
            matches!(
                event.kind,
                tacsim::arena::ledger::EventKind::Blocked { unit, .. } if unit == rig::RED
            )
        })
        .map(|event| event.t_ms);
    let matured = stopped_at.and_then(|at| {
        holds
            .iter()
            .rfind(|(read_at, _)| *read_at <= at)
            .map(|(_, since_ms)| (*since_ms, at))
    });
    seen.sort_unstable();
    M44aDetour {
        avoided: seen,
        west_most,
        matured,
    }
}

/// **A ragged partition reaches the same place with the same avoided cells**
/// (16b cut Q.6 and Q.7, 16b cut E.3's hold maturity; `tests/reflex.rs`'
/// `a_ragged_partition_reaches_the_same_place_with_the_same_avoided_cells`,
/// moved here by 17b cut M4.4). 370 ms is not a whole multiple of the 250 ms
/// substep, so the substep boundaries fall elsewhere and every event
/// timestamp moves with them: the event list, the hash sequence and the
/// exact pose are NOT the same (16b cut Q.6 says why). What the model does
/// promise is the OUTCOME — the hull learns the same cell, goes round the
/// same wreck and gets through the same causeway — and that a traffic hold's
/// stop is not early and is inside one substep of its due time: the hold
/// opens at a substep boundary and matures at the first boundary at or after
/// `since_ms + occupied_stop_ms`, so under a 370 ms pulse the stamp moves with
/// the partition exactly as every other timestamp does. The scripted thinks
/// sit on the absolute lattice; their landing moves with the partition as
/// the hold's stamp does, and every read here is the motor's and motion's.
#[test]
fn m44a_a_ragged_partition_reaches_the_same_place_with_the_same_avoided_cells() {
    let regular = m44a_avoided_and_arrival(500);
    let ragged = m44a_avoided_and_arrival(370);
    assert!(
        !regular.avoided.is_empty(),
        "nothing was avoided, so nothing was compared"
    );
    assert_eq!(
        regular.avoided, ragged.avoided,
        "the set of avoided cells moved with the partition"
    );
    assert!(
        regular.west_most < 4_000 && ragged.west_most < 4_000,
        "both runs are through the causeway: {} cm regular, {} cm ragged",
        regular.west_most,
        ragged.west_most
    );
    for (what, run) in [("regular", regular.matured), ("ragged", ragged.matured)] {
        let (since_ms, stopped_at) =
            run.unwrap_or_else(|| panic!("the {what} run never matured a hold into a stop"));
        let due = since_ms + 2_000;
        assert!(
            stopped_at >= due,
            "the {what} run stopped at {stopped_at} ms, before the hold opened at {since_ms} ms \
             was due at {due} ms"
        );
        assert!(
            stopped_at - due < 250,
            "the {what} run stopped {} ms past its due time, which is more than one substep",
            stopped_at - due
        );
    }
}

// ---------------------------------------------------------------------------
// 17b cut M4.4 — motor family B (ford, column, brake, routeless)
// ---------------------------------------------------------------------------
//
// The reflex fixtures of `tests/reflex.rs`' 16b cut E.3 driving family and
// the endgame's one-cell hold, every edge of which is the motor's or
// motion's — the traffic hold and its maturity (`motion.rs`'
// `occupied_hold`), the `blocked` line (`step.rs`), `Arena::note_blocked`,
// `Arena::hold_routeless` and `Arena::first_step_is_open`,
// `MotorRow::age_avoid`, `braking_speed_cms` and `Arena::follow` — moved
// here by 17b cut M4.4 onto the sixteen-cell rig (`support::rig`) with
// every hull a `Controller::Mind`'s, flown by a scripted mind
// (`support::scripted::ScriptedMind`) whose goals land at the reflex's own
// milliseconds: think `n` at `think_at(unit, n)` (`stagger_ms(SEED, unit,
// 1_000) + n × 1_000`), its answer landing one substep (250 ms) on and
// driven there (`Arena::motor_step` at the landing, D-UM-10), then the
// hull's own `MotorTick` every `decide_ms` on the stagger grid — the reflex
// tick's own slots. Every script is `Startup` at think 0 and the goal at
// think 1, the first online think (`RED`'s `Startup` lands at 732, the hull
// is online at 1,232 and the goal lands at 1,732; the reflex machine on the
// same hull set its first goal at its tick 982 ms in). Each window below is
// the reflex fixture's own; the constants that moved with the cadence are
// named in the test that moved them, and none of the claims did.

/// A hull's later thinks: `(think, intents)` lines after the family's two.
type M44bLater = Vec<(u64, Vec<tacsim::arena::intents::IntentKind>)>;

/// One scripted goal cell.
fn m44b_goal(x: u16, y: u16) -> tacsim::arena::intents::IntentKind {
    tacsim::arena::intents::IntentKind::Goal(tacsim::arena::intents::MotorGoal::Cell { x, y })
}

/// **The family's script**: on every hull `units` names, `Startup` at think
/// 0 and `Goal(Cell (1, 8))` — the `west` node, the reflex machine's first
/// `Patrol` goal on every window of the family — at think 1, then the hull's
/// own later thinks.
fn m44b_west_script(
    units: &[(ObjId, M44bLater)],
) -> std::sync::Arc<support::scripted::ScriptedMind> {
    use support::scripted::{Says, says};
    use tacsim::arena::intents::IntentKind;

    let script: Vec<(ObjId, Vec<Says>)> = units
        .iter()
        .map(|(unit, later)| {
            let mut lines = vec![
                says(0, vec![IntentKind::Startup]),
                says(1, vec![m44b_goal(1, 8)]),
            ];
            for (think, intents) in later {
                lines.push(says(*think, intents.clone()));
            }
            (*unit, lines)
        })
        .collect();
    support::scripted::ScriptedMind::new(&script)
}

/// The millisecond `unit`'s think `n` lands at — its answer driven there.
fn m44b_lands_at(arena: &Arena, unit: ObjId, n: u64) -> i64 {
    support::scripted::think_at(unit, i64::try_from(n).unwrap_or(0)) + arena.substep_len_ms()
}

/// **The script, with Standard Doctrine's one rule for the `unreachable`
/// edge over it**: a think whose view carries the motor's `drive_no_route`
/// notice answers `Goal(Cell (1, 8))` again — `stock_mind.rs`' departure
/// (i): a mind's goal is dropped with the notice (`Fallback::Notice`), the
/// stock mind reads `held = None` off it and pushes the fallback waypoint as
/// the next goal at that think, where the reflex machine's `Patrol` fell
/// back inside the tick (`Fallback::Waypoint`) and re-derived the same node
/// at its next. The column fixtures need it: a stop in the one-cell-wide
/// corridor puts the cell ahead on the hull's list, the re-search finds no
/// way round it until the entry ages off, and a hull whose goal was dropped
/// for good would stand in the ford for the rest of the run where the reflex
/// machine (and Standard Doctrine) asked again every tick and drove on at
/// the TTL. A think the script answers is answered as scripted.
#[derive(Debug)]
struct M44bPersistent {
    inner: std::sync::Arc<support::scripted::ScriptedMind>,
}

impl tacsim::arena::mind::MindRunner for M44bPersistent {
    fn think(&self, job: &tacsim::arena::mind::ThinkJob<'_>) -> tacsim::arena::mind::ThinkResult {
        use tacsim::arena::perception::{DriveOutcome, FrameNotice};

        let mut result = self.inner.think(job);
        let no_route = job.view.notices.iter().any(|notice| {
            matches!(
                notice,
                FrameNotice::Drive {
                    outcome: DriveOutcome::NoRoute
                }
            )
        });
        if no_route && result.intents.is_empty() {
            result.intents.push(m44b_goal(1, 8));
        }
        result
    }
}

/// **A one-cell-wide causeway** (`tests/reflex.rs`' `causeway`): walls over
/// columns 6 to 9 on every row but 8, so the only way west is a four-cell
/// corridor a column has to file through — the ford of the shipped proving
/// ground in fixture form, the one place every solo match crosses, and the
/// one `B-46` is about.
fn m44b_causeway() -> Vec<(u16, u16, tacsim::arena::state::TerrainCell)> {
    (0..rig::SIDE)
        .flat_map(|y| (6..10_u16).map(move |x| (x, y, rig::wall())))
        .filter(|(_, y, _)| *y != 8)
        .collect()
}

/// A window whose patrol graph is **one node** (`tests/reflex.rs`'
/// `one_way_west`): the reflex machines on it all drove the same way rather
/// than splitting at the nearest waypoint; the scripted minds here are
/// handed that one node, and the window is kept as the fixture's.
fn m44b_one_way_west(
    paint: &[(u16, u16, tacsim::arena::state::TerrainCell)],
) -> tacsim::arena::state::ArenaDefinition {
    use tacsim::arena::state::{Waypoint, Waypoints};

    rig::definition_with(
        paint,
        Waypoints::new(
            vec![Waypoint {
                id: smol_str::SmolStr::new("west"),
                x: 1,
                y: 8,
            }],
            &[],
        ),
    )
}

/// The machine `n` places behind the head of a [`m44b_column_of`].
fn m44b_in_column(n: u16) -> ObjId {
    ObjId(200 - u32::from(n))
}

/// **A column of `count` minds nose to tail on row 8, east of the causeway,
/// with the lowest id at the REAR** (`tests/reflex.rs`' `column_of`) — the
/// worst case for the release (16b cut E.3): units integrate in unit-id
/// order, so a follower whose id sorts below its leader's integrates before
/// the leader has moved and reaches a cell the leader is still standing in.
/// Every hull is flown by [`m44b_west_script`] under [`M44bPersistent`];
/// `head_later` is the head's own later thinks.
fn m44b_column_of(
    count: u16,
    head_later: &[(u64, Vec<tacsim::arena::intents::IntentKind>)],
) -> Arena {
    use tacsim::arena::state::{Controller, CustodyUnit};

    let units: Vec<CustodyUnit> = (0..count)
        .map(|n| {
            // `n` is the place in the column from the head: head at (10, 8),
            // rear furthest east, and the id falls as the place rises.
            rig::tank(
                m44b_in_column(n),
                Team::Red,
                rig::centre(10 + n, 8),
                Vec::new(),
                Controller::Mind,
            )
        })
        .collect();
    let mut arena = rig::arena_on(
        m44b_one_way_west(&m44b_causeway()),
        units,
        &rig::arena_cfg(rig::reflex_cfg()),
        &[],
    );
    let script: Vec<(ObjId, M44bLater)> = (0..count)
        .map(|n| {
            (
                m44b_in_column(n),
                if n == 0 {
                    head_later.to_vec()
                } else {
                    Vec::new()
                },
            )
        })
        .collect();
    arena.mind_runner = std::sync::Arc::new(M44bPersistent {
        inner: m44b_west_script(&script),
    });
    arena
}

/// **`B-46`: a follower at the ford is held and never stopped** (16b cut
/// E.3; `tests/reflex.rs`'
/// `a_follower_at_the_ford_is_held_and_journals_nothing`, moved here by 17b
/// cut M4.4). Two minds nose to tail on the causeway, the follower with the
/// lower id so that it integrates first and meets the cell its leader has
/// not vacated. Before E.3 it was stopped dead at every cell of the
/// corridor, journalled a `blocked` line for each, was told `Held: something
/// is in the way.` and taught its own motor to avoid the cell its leader was
/// about to leave. The hold (`motion.rs`' `occupied_hold`), the `blocked`
/// line and `remember_avoid` read no controller: the same 500 ms loop and
/// the same four claims — never stalled while the head was moving, no
/// `blocked` line before the head arrived, nothing learned, and through the
/// causeway at the end. The two goals land at the hulls' own think
/// staggers (the head's at 1,656, the follower's at 2,170) where the reflex
/// machines' first ticks spread them by their 500 ms stagger; the `rolling`
/// latch tolerates either.
#[test]
fn m44b_a_follower_at_the_ford_is_held_and_journals_nothing() {
    let mut arena = m44b_column_of(2, &[]);
    let (head, follower) = (m44b_in_column(0), m44b_in_column(1));
    assert!(follower.0 < head.0, "the follower must integrate first");
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall = 0_i64;
    let mut stalled: Vec<i64> = Vec::new();
    let mut rolling = false;
    let mut learned: Vec<(u16, u16)> = Vec::new();
    // **The window is the crossing, and it ends when the head arrives**: once
    // the head has stopped on the node they are both driving at, the follower
    // is standing behind a machine that is not going to move again, and doc
    // 16 §3.4 step 4's stop is exactly what it is owed. What the cut is about
    // is the ford, not the destination.
    let mut arrived_at: Option<i64> = None;
    while arena.now_ms < 30_000 {
        let out = arena.step_settled(wall, &[], &[]);
        rig::absorb(&out, &mut bytes);
        let head_speed = arena.unit(head).map_or(0, |unit| unit.speed_cms);
        let follower_speed = arena.unit(follower).map_or(0, |unit| unit.speed_cms);
        rolling |= follower_speed > 0;
        if arrived_at.is_none() && arena.unit(head).is_some_and(|unit| unit.pose.x_cm < 3_000) {
            arrived_at = Some(arena.now_ms);
        }
        if arrived_at.is_none() {
            for cell in rig::avoided(&arena, follower) {
                if !learned.contains(&cell) {
                    learned.push(cell);
                }
            }
            if rolling && head_speed > 0 && follower_speed == 0 {
                stalled.push(arena.now_ms);
            }
        }
        wall += 500;
    }
    let arrived_at = arrived_at.expect("the head crossed the causeway and reached its node");
    let log = rig::events(&mut arena, &mut bytes);
    assert!(
        stalled.is_empty(),
        "the follower lost its way while the leader was still moving, at {stalled:?} ms"
    );
    assert_eq!(
        log.iter()
            .filter(|event| event.kind.as_str() == "blocked" && event.t_ms <= arrived_at)
            .count(),
        0,
        "a column that never stopped journalled a stop before the head arrived"
    );
    assert!(
        learned.is_empty(),
        "the follower learned to avoid a cell its own leader was standing in: {learned:?}"
    );
    assert!(
        arena
            .unit(follower)
            .is_some_and(|unit| unit.pose.x_cm < 6_000),
        "the follower never got through the causeway: {:?}",
        arena.unit(follower).map(|unit| unit.pose.x_cm)
    );
}

/// **The long column** (16b cut E.3, review E-04; `tests/reflex.rs`'
/// `a_six_machine_column_files_through_the_ford_learning_nothing`, moved
/// here by 17b cut M4.4): six minds nose to tail on the causeway, the lowest
/// id at the rear, the head free to move. Two seconds of hold covers a
/// release that travels rearward one substep per unit (motion's), and six is
/// inside the shipped roster's twelve either way. Nothing is stopped,
/// nothing is journalled and nobody learns a teammate's transient cell: the
/// same loop and the same three claims, the six goals landing inside the
/// half-second their think staggers span (1,656 to 2,170) where the reflex
/// ticks spread them inside theirs.
#[test]
fn m44b_a_six_machine_column_files_through_the_ford_learning_nothing() {
    let mut arena = m44b_column_of(6, &[]);
    let ids: Vec<ObjId> = (0..6).map(m44b_in_column).collect();
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall = 0_i64;
    let mut rolling = false;
    let mut stalled: Vec<(i64, usize)> = Vec::new();
    let mut arrived_at: Option<i64> = None;
    while arena.now_ms < 60_000 {
        let out = arena.step_settled(wall, &[], &[]);
        rig::absorb(&out, &mut bytes);
        let speeds: Vec<i32> = ids
            .iter()
            .map(|id| arena.unit(*id).map_or(0, |unit| unit.speed_cms))
            .collect();
        rolling |= speeds.iter().all(|speed| *speed > 0);
        if arrived_at.is_none()
            && arena
                .unit(ids[0])
                .is_some_and(|unit| unit.pose.x_cm < 3_000)
        {
            arrived_at = Some(arena.now_ms);
        }
        if rolling && arrived_at.is_none() {
            for (place, speed) in speeds.iter().enumerate() {
                if *speed == 0 {
                    stalled.push((arena.now_ms, place));
                }
            }
        }
        wall += 500;
    }
    let arrived_at = arrived_at.expect("the head crossed the causeway and reached its node");
    let log = rig::events(&mut arena, &mut bytes);
    assert!(
        stalled.is_empty(),
        "machines in the column lost their way before the head arrived: {stalled:?}"
    );
    assert_eq!(
        log.iter()
            .filter(|event| event.kind.as_str() == "blocked" && event.t_ms <= arrived_at)
            .count(),
        0,
        "the column journalled a stop before the head arrived"
    );
    // The corridor is (6, 8) to (9, 8) and the column's own cells are (10, 8)
    // east: no machine ever learned one of them.
    for id in &ids {
        let learned = rig::avoided(&arena, *id);
        assert!(
            learned
                .iter()
                .all(|(x, y)| !(*y == 8 && (6..rig::SIDE).contains(x))),
            "#{} learned a cell a teammate was standing in: {learned:?}",
            id.0
        );
    }
}

/// The head's think nearest 8,000 — the column's head is `#200`, its thinks
/// at 406 + n × 1,000, so think 8 at 8,406 — at which it answers
/// `Goal(Hold)`: the goal dropped and a standstill ordered at the landing,
/// 8,656.
const M44B_PARK_THINK: u64 = 8;

/// The head's think nearest 13,000 (13,406), at which it answers
/// `Goal(Cell (1, 8))` again: the release, driven at the landing, 13,656.
const M44B_RELEASE_THINK: u64 = 13;

/// **The parked head, MEASURED and not asserted** (16b cut E.3, review
/// E-04; `tests/reflex.rs`' `e3_a_parked_head_is_measured_and_reported`,
/// moved here by 17b cut M4.4). The same six, with the head held where it
/// stands for five seconds of arena time and then released. Two seconds of
/// hold is clean for a column of up to eight being released; a column whose
/// head is itself stopped for longer than that matures its tail into stops,
/// and each of those teaches a teammate's transient cell for
/// `REFLEX_AVOID_TTL_MS`. That is HEAD's behaviour delayed and made rare,
/// not removed, and the three numbers this prints are the evidence for or
/// against the second-stop rule 16b cut E.3 deliberately did not build.
/// What moved: the reflex fixture parked the head by seating a pilot on it
/// at 8,000 (`set_controller`, which 17b cut M4.6 retires) and ordering it
/// `desired_speed 0` every pulse, then handed it back at 13,000; here the
/// head's own mind answers `Goal(Hold)` at its think nearest 8,000 — a hold
/// drops the goal and orders the standstill (`land_goal`) — and
/// `Goal(Cell (1, 8))` at its think nearest 13,000, so the park and the
/// release are the two landings ([`M44B_PARK_THINK`],
/// [`M44B_RELEASE_THINK`]: 8,656 and 13,656), the head's think stagger plus
/// a substep past the reflex fixture's round seconds. The printed numbers
/// may shift with them — declared measured, not asserted.
#[test]
fn m44b_e3_a_parked_head_is_measured_and_reported() {
    use tacsim::arena::intents::{IntentKind, MotorGoal};

    let mut arena = m44b_column_of(
        6,
        &[
            (M44B_PARK_THINK, vec![IntentKind::Goal(MotorGoal::Hold)]),
            (M44B_RELEASE_THINK, vec![m44b_goal(1, 8)]),
        ],
    );
    let ids: Vec<ObjId> = (0..6).map(m44b_in_column).collect();
    let head = ids[0];
    let parked_from = m44b_lands_at(&arena, head, M44B_PARK_THINK);
    let released_at = m44b_lands_at(&arena, head, M44B_RELEASE_THINK);
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall = 0_i64;
    // The fixture's own guard: the hold landed — the head held no goal and
    // was ordered a standstill between the park and the release.
    let mut parked = false;
    let mut through: Option<i64> = None;
    // Every teammate's cell any of the six ever remembered, over the whole
    // episode: the entries age out after `REFLEX_AVOID_TTL_MS`, so a reading
    // taken at the end would under-count.
    let mut taught: Vec<(ObjId, (u16, u16))> = Vec::new();
    // (1) the column gets under way and files into the corridor; (2) the head
    // stops where it stands for five seconds — a tank parked in the ford,
    // which is longer than the hold and so compresses the column into real
    // stops; (3) the release.
    while arena.now_ms < released_at + 60_000 {
        let out = arena.step_settled(wall, &[], &[]);
        rig::absorb(&out, &mut bytes);
        if arena.now_ms > parked_from && arena.now_ms <= released_at {
            parked |= rig::motor(&arena, head).goal.is_none()
                && arena
                    .unit(head)
                    .is_some_and(|unit| unit.desired_speed_cms == 0);
        }
        if arena.now_ms > released_at {
            // Only while the release is still travelling down the column:
            // past that the six converge on one node and queue at the
            // destination, which is doc 16 §3.4 step 4 doing its job and not
            // what this measures.
            if arena.now_ms <= released_at + 10_000 {
                for id in &ids {
                    for cell in rig::avoided(&arena, *id) {
                        if !taught.contains(&(*id, cell)) {
                            taught.push((*id, cell));
                        }
                    }
                }
            }
            if through.is_none()
                && ids
                    .iter()
                    .all(|id| arena.unit(*id).is_some_and(|unit| unit.pose.x_cm < 6_000))
            {
                through = Some(arena.now_ms);
            }
        }
        wall += 500;
    }
    let log = rig::events(&mut arena, &mut bytes);
    let matured = log
        .iter()
        .filter(|event| {
            event.kind.as_str() == "blocked"
                && (parked_from..=released_at + 10_000).contains(&event.t_ms)
        })
        .count();
    let learners: usize = ids
        .iter()
        .filter(|id| taught.iter().any(|(who, _)| who == *id))
        .count();
    println!(
        "m44b_e3_parked_head: parked at {parked_from} ms, released at {released_at} ms; \
         {matured} stops matured between the park and ten seconds past the release, \
         {learners} of 6 machines learned a cell in that window ({taught:?}), the last was \
         through {} ms after the release",
        through.map_or(-1, |at| at - released_at)
    );
    assert!(
        parked,
        "the head's hold never landed, so nothing was measured"
    );
    assert!(
        through.is_some(),
        "the column never cleared the causeway after the release"
    );
}

/// The think at which the ford fixture's mind turns the hull round for the
/// `east` node, `Goal(Cell (14, 8))`: the first think after the hull let go
/// of `west` on arrival (a mind's goal is dropped at arrival under
/// `Fallback::Notice`, on the `MotorTick` that reads it) — measured on this
/// run (the arrival is read at 20,500, on the tick at 20,482; think 20 is at
/// 20,482, after the tick, its goal landing at 20,732) and guarded by the
/// test. The reflex machine's `Patrol` turned round on the arriving tick
/// itself; a mind's turnaround is the next landing.
const M44B_FORD_EAST_THINK: u64 = 20;

/// The think at which it turns back for `west`, `Goal(Cell (1, 8))`: the
/// first think after the arrival at `east` — measured on this run (the
/// arrival is read at 38,000, on the tick at 37,982; think 38 is at 38,482,
/// landing 38,732) and guarded by the test. Its return pass through column
/// 4 falls after the (4, 8) entry, stamped at the first stop (6,250), has
/// aged off the list at 36,250: the second stop is at 50,250, where the
/// reflex fixture measured 5,500 and 47,250.
const M44B_FORD_WEST_THINK: u64 = 38;

/// **`B-46` at a wreck**: the hull learns the cell two seconds later and
/// takes the other gap, which is
/// [`m44a_a_wreck_in_the_causeway_is_learned_and_the_other_gap_is_taken`]
/// with the hold in front of it (16b cut E.3; `tests/reflex.rs`'
/// `a_wreck_in_the_ford_is_held_against_for_two_seconds_and_then_learned`,
/// moved here by 17b cut M4.4). The reflex machine's `Patrol` walked `west`,
/// `east`, `west` inside the minute; the scripted mind lands the same three
/// legs as goals — `west` at think 1, `east` at [`M44B_FORD_EAST_THINK`] and
/// `west` at [`M44B_FORD_WEST_THINK`], each turnaround the first think after
/// the arrival let the last goal go, which the test reads off the motor row
/// before each landing — so the hold, the `blocked` line,
/// `Arena::note_blocked`, the no-orbit detour (16b cut AI.1, `B-84`) and
/// the stop count are read as they were: two stops in the minute, the second
/// no sooner than the memory of the first has lapsed, the first no sooner
/// than the hold, one remembered cell and through the other gap.
#[test]
fn m44b_a_wreck_in_the_ford_is_held_against_for_two_seconds_and_then_learned() {
    use tacsim::arena::state::{Controller, TerrainCell};

    let band: Vec<(u16, u16, TerrainCell)> = (0..rig::SIDE)
        .filter(|y| *y != 8 && *y != 2)
        .map(|y| (4_u16, y, rig::wall()))
        .collect();
    let mut arena = rig::arena_of(
        vec![
            rig::tank(
                rig::RED,
                Team::Red,
                rig::centre(6, 8),
                Vec::new(),
                Controller::Mind,
            ),
            rig::tank(
                rig::RED2,
                Team::Red,
                rig::centre(4, 8),
                Vec::new(),
                Controller::Pilot,
            ),
        ],
        &band,
        &rig::arena_cfg(rig::reflex_cfg()),
        &[],
    );
    rig::wreck_it(&mut arena, rig::RED2);
    arena.mind_runner = m44b_west_script(&[(
        rig::RED,
        vec![
            (M44B_FORD_EAST_THINK, vec![m44b_goal(14, 8)]),
            (M44B_FORD_WEST_THINK, vec![m44b_goal(1, 8)]),
        ],
    )]);
    let turnarounds = [
        m44b_lands_at(&arena, rig::RED, M44B_FORD_EAST_THINK),
        m44b_lands_at(&arena, rig::RED, M44B_FORD_WEST_THINK),
    ];
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall = 0_i64;
    let mut met_at: Option<i64> = None;
    let mut west_most = i64::MAX;
    // The entries age out after `REFLEX_AVOID_TTL_MS`, so what the hull
    // learned is the union over the run and not a reading taken at the end.
    let mut learned: Vec<(u16, u16)> = Vec::new();
    // The motor's goal after every pulse: each turnaround is a turnaround
    // only if the goal before it had already been let go on arrival.
    let mut goals: Vec<(i64, Option<(u16, u16)>)> = Vec::new();
    // Searches that failed: none, so every goal let go is an arrival and
    // never a no-route's drop.
    let mut path_failures = 0_u32;
    while arena.now_ms < 60_000 {
        let out = arena.step_settled(wall, &[], &[]);
        rig::absorb(&out, &mut bytes);
        path_failures += out.motor.path_failures;
        // The first pulse that ended with the hull standing against a cell.
        if met_at.is_none()
            && arena
                .unit(rig::RED)
                .is_some_and(|unit| unit.occupied_hold.is_some())
        {
            met_at = Some(arena.now_ms);
        }
        for cell in rig::avoided(&arena, rig::RED) {
            if !learned.contains(&cell) {
                learned.push(cell);
            }
        }
        west_most = west_most.min(arena.unit(rig::RED).map_or(i64::MAX, |unit| unit.pose.x_cm));
        goals.push((arena.now_ms, rig::motor(&arena, rig::RED).goal));
        wall += 500;
    }
    let met_at = met_at.expect("the hull never met the wreck");
    // The pulses at which a goal was read let go (an arrival): where the
    // turnaround thinks above were measured from.
    let arrivals: Vec<i64> = goals
        .windows(2)
        .filter(|pair| pair[0].1.is_some() && pair[1].1.is_none())
        .map(|pair| pair[1].0)
        .collect();
    assert_eq!(
        path_failures, 0,
        "a goal let go by a failed search is not an arrival"
    );
    let mut since = 0_i64;
    for landing in turnarounds {
        let before = goals
            .iter()
            .rfind(|(at, _)| *at < landing)
            .map(|(_, goal)| *goal);
        assert_eq!(
            before,
            Some(None),
            "the turnaround landing at {landing} ms redirected a hull still on its way: the \
             legs are the arrivals' ({goals:?})"
        );
        assert!(
            arrivals.iter().any(|at| (since..landing).contains(at)),
            "no arrival between {since} and the turnaround landing at {landing} ms: \
             {arrivals:?}"
        );
        since = landing;
    }
    let log = rig::events(&mut arena, &mut bytes);
    let stops: Vec<i64> = log
        .iter()
        .filter(|event| {
            matches!(
                event.kind,
                tacsim::arena::ledger::EventKind::Blocked { unit, x, y, .. }
                    if unit == rig::RED && (x, y) == (4, 8)
            )
        })
        .map(|event| event.t_ms)
        .collect();
    println!(
        "m44b_ford: met the wreck at {met_at} ms, arrivals read at {arrivals:?} ms, stops at \
         (4, 8) at {stops:?} ms, west-most {west_most} cm"
    );
    // **16b cut AI.1** (16c `B-84`): one wreck is one stop **per remembered
    // stop**. The hull no longer circles the gap's corners, so it finishes
    // its leg inside the minute, and back at the wreck after the cell has
    // aged off its list (`REFLEX_AVOID_TTL_MS`) it is stopped once more — by
    // design, a remembered stop is thirty seconds and not for ever. What
    // stays pinned is exact: **two** stops in the minute, the second no
    // sooner than the memory of the first has lapsed — so exactly one inside
    // the memory's lifetime, and the hold adds none. A third stop, or a
    // return inside the thirty seconds, fails here.
    assert_eq!(
        stops.len(),
        2,
        "one wreck is one stop per remembered stop, and the minute holds two: {stops:?}"
    );
    assert!(
        stops.windows(2).all(|pair| match pair {
            [a, b] => b - a >= tacsim::arena::motor::REFLEX_AVOID_TTL_MS,
            _ => true,
        }),
        "one wreck is one stop, and the hold does not add another: {stops:?}"
    );
    assert!(
        stops[0] >= met_at + 1_500,
        "the stop arrived {} ms after the hull was first read standing against the wreck, \
         which is inside the two-second hold (the hold is read at the end of the pulse it opened \
         in, so the margin a 500 ms pulse can promise is a pulse short of 2,000)",
        stops[0] - met_at
    );
    assert_eq!(
        learned,
        vec![(4, 8)],
        "one remembered cell, and it is the wreck's"
    );
    assert!(
        west_most < 4_000,
        "it is through the other gap: west-most {west_most} cm"
    );
}

/// A long east-west road, `len` cells by eight, with one patrol node at
/// `node` (`tests/reflex.rs`' `road`, which stays there under its
/// `road_home`).
fn m44b_road(len: u16, node: (u16, u16)) -> tacsim::arena::state::ArenaDefinition {
    use tacsim::arena::state::{ArenaDefinition, ArenaTerrain, EntryZone, Waypoint, Waypoints};

    let cells = vec![rig::ground(); usize::from(len) * 8];
    ArenaDefinition {
        id: smol_str::SmolStr::new("tacsim.arena.motor.road"),
        terrain: ArenaTerrain::new(len, 8, rig::CELL_CM, cells).expect("the road window"),
        entry_zones: vec![
            EntryZone {
                team: Team::Red,
                x0: 0,
                y0: 0,
                x1: 1,
                y1: 7,
            },
            EntryZone {
                team: Team::Blue,
                x0: len - 2,
                y0: 0,
                x1: len - 1,
                y1: 7,
            },
        ],
        key_spots: Vec::new(),
        waypoints: Waypoints::new(
            vec![Waypoint {
                id: smol_str::SmolStr::new("goal"),
                x: node.0,
                y: node.1,
            }],
            &[],
        ),
        modes: Vec::new(),
    }
}

/// **`B-42`: a machine brakes for its goal instead of driving through it**
/// (16b cut E.3; `tests/reflex.rs`'
/// `a_machine_brakes_into_its_goal_and_does_not_overshoot_it`, moved here by
/// 17b cut M4.4). Forty cells of road and a tank with the shipped ratio —
/// fifteen metres a second, two metres a second squared of brake, which is
/// fifty-six metres and seven and a half seconds to stop. Before E.3 the
/// controller ordered `max_speed_cms` flat until the unit was inside the
/// one-cell arrive window and then ordered zero, so the machine coasted five
/// cells past everywhere it meant to be. The brake is `braking_speed_cms`,
/// asked by `Arena::follow` on the hull's `MotorTick` every `decide_ms` —
/// `arena_cfg.reflex.decide_ms`, the reflex's own 500 — so the measured
/// figure holds. What moved: the goal is the script's `Goal(Cell (4, 4))`,
/// landing at think 1, and the reflex fixture's `hunt_after_ms 0` is gone
/// with the hunt — a mind's goal is let go at arrival and the tick is not
/// rescheduled, so the 180 s run holds no second arrival. The heavy is
/// re-sealed over its mobility.
#[test]
fn m44b_a_machine_brakes_into_its_goal_and_does_not_overshoot_it() {
    use support::scripted::{ScriptedMind, says};
    use tacsim::arena::intents::IntentKind;
    use tacsim::arena::state::{Controller, CustodyUnit, TrackedRules};

    let heavy = CustodyUnit {
        mobility: TrackedRules {
            max_speed_cms: 1_500,
            accel_cms2: 100,
            brake_cms2: 200,
            turn_rate_mdeg_s: 30_000,
            reverse_factor_pct: 40,
        },
        ..rig::tank(
            rig::RED,
            Team::Red,
            rig::centre(44, 4),
            Vec::new(),
            Controller::Mind,
        )
    }
    .sealed();
    let mut arena = rig::arena_on(
        m44b_road(48, (4, 4)),
        vec![heavy],
        &rig::arena_cfg(rig::reflex_cfg()),
        &[],
    );
    arena.mind_runner = ScriptedMind::new(&[(
        rig::RED,
        vec![
            says(0, vec![IntentKind::Startup]),
            says(1, vec![m44b_goal(4, 4)]),
        ],
    )]);
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall = 0_i64;
    let mut at_arrival: Option<i32> = None;
    let mut west_most = u16::MAX;
    let mut top = 0_i32;
    while arena.now_ms < 180_000 {
        let out = arena.step_settled(wall, &[], &[]);
        rig::absorb(&out, &mut bytes);
        let Some(unit) = arena.unit(rig::RED) else {
            break;
        };
        top = top.max(unit.speed_cms);
        if let Some((x, y)) = arena.definition.terrain.cell_of(&unit.pose) {
            west_most = west_most.min(x);
            // The pulse it first reads arrived: inside `ARRIVE_CELLS` of (4, 4).
            if at_arrival.is_none() && x.abs_diff(4).max(y.abs_diff(4)) <= 1 {
                at_arrival = Some(unit.speed_cms);
            }
        }
        wall += 500;
    }
    let at_arrival = at_arrival.expect("the machine never reached its goal");
    println!(
        "m44b_e3_braking: top {top} cm/s, {at_arrival} cm/s on the pulse it first read arrived, \
         west-most cell {west_most}"
    );
    assert!(
        top >= 1_400,
        "the machine never got up to speed, so nothing was measured: {top} cm/s"
    );
    // **The measured figure is 415 cm/s**, and the number is worth being
    // exact about rather than rounding to a slogan. The profile is the
    // maximum-braking curve, `v = isqrt(2 × brake × d)`, and a tank on it
    // sheds exactly `brake_cms2` a second — which is all the tank has. The
    // motor decides twice a second, so the last tick before the arrive window
    // still carries up to a tick of the curve it cannot have shed yet. A
    // quarter of `max_speed` at the window, against the whole of it before
    // E.3, is the claim; the tank is standing still a second and a half
    // later, which the last assertion is.
    assert!(
        at_arrival <= 500,
        "it arrived at {at_arrival} cm/s, which is not braking for the goal"
    );
    assert!(
        west_most >= 3,
        "it overshot its goal at (4, 4) to cell {west_most}, past the arrive window"
    );
    assert_eq!(
        arena.unit(rig::RED).map(|unit| unit.speed_cms),
        Some(0),
        "and it is standing on it at the end"
    );
}

/// **The convoy and the wreck** (16b cut E.3; `tests/reflex.rs`'
/// `convoy_and_wreck`): a band down column 4 with gaps at rows 8 and 2, a
/// leader at (6, 8) and a follower at (7, 8) that sorts below it and so
/// integrates first, both minds on [`m44b_west_script`] (the follower's goal
/// lands at 1,732, the leader's at 1,254), the tempo pinned `Quiet` (D-16-2:
/// the tempo decides only how often the arena steps).
fn m44b_convoy_and_wreck() -> Arena {
    use tacsim::arena::state::{Controller, TerrainCell};

    let band: Vec<(u16, u16, TerrainCell)> = (0..rig::SIDE)
        .filter(|y| *y != 8 && *y != 2)
        .map(|y| (4_u16, y, rig::wall()))
        .collect();
    let mut arena = rig::arena_on(
        m44b_one_way_west(&band),
        vec![
            // The follower sorts below the leader, so it integrates first.
            rig::tank(
                rig::RED,
                Team::Red,
                rig::centre(7, 8),
                Vec::new(),
                Controller::Mind,
            ),
            rig::tank(
                rig::RED2,
                Team::Red,
                rig::centre(6, 8),
                Vec::new(),
                Controller::Mind,
            ),
        ],
        &rig::arena_cfg(rig::reflex_cfg()),
        &[],
    );
    arena.mind_runner = m44b_west_script(&[(rig::RED, Vec::new()), (rig::RED2, Vec::new())]);
    arena
        .clock
        .pin_tempo_for_tests(tacsim::arena::clock::Tempo::Quiet);
    arena
}

/// The arena millisecond the convoy's leader is wrecked where it stands — a
/// multiple of all three cadences, so it is one fact of arena time in all
/// three logs (`tests/reflex.rs`' `SCRIPT_AT_MS`).
const M44B_WRECK_AT_MS: i64 = 20_000;

/// What one replay of [`m44b_convoy_and_wreck`] produced: the event list,
/// the per-substep hash sequence, the finals — where each hull is, its
/// heading, how fast, and the goal its motor holds; the reflex's state
/// column dropped, the goals being the script's — and how many pulses it
/// took (three cadences really are three different numbers of pulses, so an
/// equality of the three is a property and not three copies of one run).
struct M44bReplay {
    events: Vec<(i64, String, u64)>,
    hashes: Vec<u64>,
    finals: Vec<M44bFinal>,
    pulses: u32,
}

/// One hull's final: id, pose, heading, speed and motor goal.
type M44bFinal = (ObjId, i64, i64, u32, i32, Option<(u16, u16)>);

/// [`m44b_convoy_and_wreck`] replayed at `cadence_ms` until `until_ms`,
/// settled between pulses, the leader wrecked after the pulse that closes at
/// [`M44B_WRECK_AT_MS`].
fn m44b_replay(cadence_ms: i64, until_ms: i64) -> M44bReplay {
    let mut arena = m44b_convoy_and_wreck();
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut wall = 0_i64;
    let mut pulses = 0_u32;
    loop {
        let out = arena.step_settled(wall, &[], &[]);
        pulses = pulses.saturating_add(1);
        rig::absorb(&out, &mut bytes);
        hashes.extend(arena.substep_hashes.iter().copied());
        if arena.now_ms == M44B_WRECK_AT_MS {
            rig::wreck_it(&mut arena, rig::RED2);
        }
        if arena.now_ms >= until_ms || out.ended.is_some() {
            break;
        }
        wall += cadence_ms;
    }
    let events = rig::events(&mut arena, &mut bytes)
        .into_iter()
        .map(|event| {
            (
                event.t_ms,
                serde_json::to_string(&event.kind).unwrap_or_default(),
                event.hash,
            )
        })
        .collect();
    let finals = arena
        .units
        .iter()
        .map(|unit| {
            (
                unit.id,
                unit.pose.x_cm,
                unit.pose.y_cm,
                unit.pose.heading_mdeg,
                unit.speed_cms,
                rig::motor(&arena, unit.id).goal,
            )
        })
        .collect();
    M44bReplay {
        events,
        hashes,
        finals,
        pulses,
    }
}

/// **The convoy and the wreck, at three cadences** (16b cut E.3; 16b §11's
/// mandatory cross-tempo test; `tests/reflex.rs`'
/// `the_convoy_and_the_wreck_are_one_event_list_and_one_hash_sequence_at_three_cadences`,
/// moved here by 17b cut M4.4 in
/// [`m44a_the_wreck_detour_is_one_event_list_and_one_hash_sequence_at_three_cadences`]'
/// shape). One scripted log in which a follower holds behind its leader
/// through the gap, the leader stops on the node and the follower's hold
/// against it matures into a stop, learns the cell and detours round it to
/// its own arrival; the leader is then wrecked where it stands. One event
/// list — a `blocked` line in it — and one per-substep hash sequence at
/// 5,000, 1,000 and 500 ms, which is the whole of the rate-independence
/// claim for the hold, because all three cadences are whole multiples of
/// the 250 ms substep and therefore share one partition. Every edge is the
/// motor's (`Arena::follow`, `Arena::first_step_is_open`,
/// `Arena::hold_routeless`, `Arena::note_blocked`) or motion's; the finals
/// are pose, heading, speed and motor goal.
#[test]
fn m44b_the_convoy_and_the_wreck_are_one_event_list_and_one_hash_sequence_at_three_cadences() {
    let runs: Vec<M44bReplay> = [5_000_i64, 1_000, 500]
        .into_iter()
        .map(|cadence| m44b_replay(cadence, 60_000))
        .collect();
    let first = runs.first().expect("three replays");
    let kinds: Vec<&str> = first
        .events
        .iter()
        .map(|(_, kind, _)| kind.as_str())
        .collect();
    assert!(
        kinds.iter().any(|kind| kind.contains("blocked")),
        "the follower never matured a stop, so nothing was compared: {kinds:?}"
    );
    assert!(first.hashes.len() >= 100, "{} substeps", first.hashes.len());
    assert!(
        runs.windows(2).all(|pair| pair[0].pulses < pair[1].pulses),
        "three cadences, three pulse counts"
    );
    for (at, run) in runs.iter().enumerate().skip(1) {
        let cadence = [5_000_i64, 1_000, 500].get(at).copied().unwrap_or(0);
        assert_eq!(
            run.events, first.events,
            "the event list diverged at {cadence} ms"
        );
        assert_eq!(
            run.hashes, first.hashes,
            "the hash sequence diverged at {cadence} ms"
        );
        assert_eq!(
            run.finals, first.finals,
            "the field ended differently at {cadence} ms"
        );
    }
}

/// **`B-44`: a routeless machine is checked every substep, not every tick**
/// (16b cut E.3; `tests/reflex.rs`'
/// `a_routeless_machine_is_ordered_to_stop_within_one_substep_of_a_stop`,
/// moved here by 17b cut M4.4). The case `Arena::follow` cannot see,
/// because it happens between two of its ticks. A stop throws the hull's
/// route away and puts the refused cell on its remembered list, both at the
/// substep the stop happened on — and the hull is then routeless, with a
/// goal, with a standing speed order and pointed straight at a cell it has
/// just learned to avoid. Until E.3 it re-accelerated and pushed at that
/// cell for the rest of the decide tick; now the pre-pass at the top of the
/// next substep (`Arena::hold_routeless`, for every routeless row with a
/// goal whatever steers it) orders `Speed 0` through the same door a pilot's
/// own order goes through, and the hull stands until its next tick can plan.
/// The measure is the arena time between the `blocked` line and the speed
/// order coming off: one substep, not one `decide_ms`. The decide grid is
/// printed and asserted with it, so that the reading cannot be a tick that
/// happened to fall inside the substep: the hull's `MotorTick` is scheduled
/// at `grid_at(unit, reflex.decide_ms)` (`Arena::engage_motor`), the stagger
/// grid the reflex tick ran on, so the grid is read as it was.
/// `arena::motor`'s own
/// `e3_the_pre_pass_holds_a_routeless_machine_and_skips_everyone_else` is
/// what holds the pass's five guards. What moved: the hull starts half a
/// cell east of (6, 8)'s centre, on its seam with (7, 8)
/// (`pose(7_000, 8_500)`). The reflex machine's goal was at its tick 1,482;
/// from the centre its hull reached the parked tank's seam in the substep
/// ending 3,000, the hold matured into the stop in the substep ending
/// 5,000, and the 5,250 read caught the pre-pass's standstill a substep
/// before the 5,482 tick. A mind's goal lands at 1,732 and its drive starts
/// in the substep after; from the centre the hull reaches the seam in the
/// substep ending 3,250, the stop falls in the substep ending 5,250, and the
/// 5,482 tick runs in the substep after — after that substep's pre-pass has
/// ordered the standstill — re-planning round the parked tank and ordering
/// a speed again before the pulse's end can read it (the standstill is in
/// that pulse's intent count, not on the hull at its end); a quarter of a
/// cell further east puts the stop in the tick's own substep, where motion
/// stops the hull and the tick re-plans before the next pre-pass asks. A
/// think and its hull's motor tick share one stagger, so on this seed no
/// cell-centred start of the rig's tanks keeps the stop and the substep
/// after it clear of a tick; half a cell does: the seam is reached in the
/// substep ending 3,750, the stop falls in the one ending 5,750, and
/// neither it nor the substep after holds a tick (5,482 before, 6,482
/// after).
#[test]
fn m44b_a_routeless_machine_is_ordered_to_stop_within_one_substep_of_a_stop() {
    use tacsim::arena::rng::stagger_ms;
    use tacsim::arena::state::Controller;
    use tacsim::config::ReflexConfig;

    // A whole second between decisions, so "the next tick" and "the next
    // substep" are four substeps apart and the difference is visible.
    let cfg = rig::arena_cfg(ReflexConfig {
        decide_ms: 1_000,
        repath_ms: 4_000,
        ..ReflexConfig::default()
    });
    let mut arena = rig::arena_of(
        vec![
            rig::tank(
                rig::RED,
                Team::Red,
                rig::pose(7_000, 8_500),
                Vec::new(),
                Controller::Mind,
            ),
            rig::tank(
                rig::BLUE,
                Team::Blue,
                rig::centre(5, 8),
                Vec::new(),
                Controller::Pilot,
            ),
        ],
        &[],
        &cfg,
        &[rig::BLUE],
    );
    arena.mind_runner = m44b_west_script(&[(rig::RED, Vec::new())]);
    let hull = rig::RED;
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall = 0_i64;
    // Pulses of 250 ms, so every pulse is exactly one substep and the reading
    // is at the granularity the pre-pass runs at.
    let mut stopped_at: Option<i64> = None;
    let mut ordered_off_at: Option<i64> = None;
    let mut intents_on_the_order = 0_u32;
    while arena.now_ms < 30_000 {
        let out = arena.step_settled(wall, &[], &[]);
        rig::absorb(&out, &mut bytes);
        // The stop is read off the unit's own per-substep flag rather than off
        // the ledger, because a 250 ms pulse is one substep and the ledger's
        // chunks are flushed on their own cadence.
        if stopped_at.is_none() {
            if arena.unit(hull).is_some_and(|unit| unit.blocked) {
                stopped_at = Some(arena.now_ms);
            }
        } else if ordered_off_at.is_none()
            && arena
                .unit(hull)
                .is_some_and(|unit| unit.desired_speed_cms == 0)
        {
            ordered_off_at = Some(arena.now_ms);
            intents_on_the_order = out.motor.intents;
        }
        if ordered_off_at.is_some() {
            break;
        }
        wall += 250;
    }
    let stopped_at = stopped_at.expect("the parked tank stopped the machine");
    let ordered_off_at = ordered_off_at.expect("the machine was never ordered to stand");
    // **The decide grid, so that the 250 ms reading cannot be a coincidence.**
    // A hull's motor ticks are at its stagger — `stagger_ms(seed, id,
    // decide_ms)` — and every `decide_ms` after it (`Arena::grid_at`), so
    // this is exactly the arena time at which `follow` could have ordered the
    // same speed on its own.
    let grid = |at: i64| (at - stagger_ms(rig::SEED, hull, 1_000)).rem_euclid(1_000);
    println!(
        "m44b_e3_b44: stopped at {stopped_at} ms, ordered off at {ordered_off_at} ms, \
         {} ms past that pulse's decide tick, {intents_on_the_order} intents on the pulse",
        grid(ordered_off_at)
    );
    assert!(
        ordered_off_at - stopped_at <= 250,
        "the machine kept its speed order for {} ms after it was stopped, which is the decide \
         tick and not the substep: stopped at {stopped_at} ms, ordered off at {ordered_off_at} ms",
        ordered_off_at - stopped_at
    );
    assert_ne!(
        grid(ordered_off_at),
        0,
        "the order came off on a decide tick, where `follow` would have ordered it too: this \
         fixture is not measuring the pre-pass"
    );
    assert!(
        intents_on_the_order > 0,
        "the pulse the order came off on counted no intent, so nothing went through `apply_intent`"
    );
    // And the order came through the pilot's own door: the motor counted it.
    assert!(
        arena
            .motors
            .row(hull)
            .is_some_and(|row| row.route.is_empty() && row.goal.is_some()),
        "the machine was not routeless with a goal, so this is not `B-44`'s case"
    );
}

/// The pulse the one-cell hold's note is made at: the first pulse after the
/// goal landed (1,732), the hull a quarter-second into its acceleration and
/// still in its own cell. See the test for why it is not the reflex
/// fixture's 4,000.
const M44B_NOTE_AT_MS: i64 = 2_000;

/// **The `avoid` clause of `first_step_is_open`** (16c `T-42`;
/// `tests/reflex.rs`'
/// `e4_t42_a_routeless_machine_holds_at_a_cell_it_was_stopped_at`, moved
/// here by 17b cut M4.4): a routeless hull holds rather than drive into a
/// cell it was stopped at, and drives again when the entry ages out. The
/// path budget is nought, so the hull never has a route and every step it
/// takes is a step on a bearing nobody has checked — which is the whole case
/// the one-cell test exists for. The cell in front of it is then written
/// into `MotorRow::avoid` through `Arena::note_blocked`, the public door the
/// `Blocked` ledger event uses; `Arena::hold_routeless` asks
/// `Arena::first_step_is_open` of every steered row at every substep and
/// `Arena::follow`'s routeless steer asks it on every `MotorTick`, and
/// `MotorRow::age_avoid` runs on the tick for any row. The same four
/// claims: no route and driving at the goal before the note, ordered a
/// standstill three seconds after it, driving again three seconds after the
/// entry's `REFLEX_AVOID_TTL_MS` — with one guard added, that the goal is
/// still held at the standstill, so the hold read is the avoid list's and
/// never an arrival's. What moved: the goal is the script's `Goal(Cell (1,
/// 8))`, the `west` node the reflex's `Patrol` picked, landing at 1,732; and
/// the note is made at [`M44B_NOTE_AT_MS`], 2,000, not the reflex fixture's
/// 4,000. The reflex hull, under way since 982, was at full speed a few
/// hundred centimetres short of the cell at 4,000, and a hull at 1,000 cm/s
/// with 500 cm/s² of brake needs a whole cell to stop: it crossed into the
/// cell it had been told to avoid, drove on from inside it, arrived at
/// `west`, and what held it at 7,000 was its `Patrol` turnaround for `east`
/// against the same remembered cell from the other side — a walk a scripted
/// goal does not make (a mind's goal is let go at arrival). At 2,000 the
/// hull is one substep into its acceleration (125 cm/s, a few centimetres
/// from where it started) in its own cell, stops short of the cell it is
/// told to avoid, and the hold the claim names is the one read; at the TTL
/// it drives the leg it was given.
#[test]
fn m44b_e4_t42_a_routeless_machine_holds_at_a_cell_it_was_stopped_at() {
    use tacsim::arena::state::Controller;
    use tacsim::config::ReflexConfig;

    let mut arena = rig::arena_of(
        vec![rig::tank(
            rig::RED,
            Team::Red,
            rig::centre(6, 8),
            Vec::new(),
            Controller::Mind,
        )],
        &[],
        &rig::arena_cfg(ReflexConfig {
            // No search at all: a hull that never routes is the subject.
            max_path_queries: 0,
            ..rig::reflex_cfg()
        }),
        &[],
    );
    arena.mind_runner = m44b_west_script(&[(rig::RED, Vec::new())]);
    let mut bytes: Vec<u8> = Vec::new();
    let mut counters = MotorCounters::default();
    let _ = m44b_drive_to(&mut arena, M44B_NOTE_AT_MS, &mut bytes, &mut counters);
    let here = arena
        .unit(rig::RED)
        .and_then(|unit| arena.definition.terrain.cell_of(&unit.pose))
        .expect("the hull is on the window");
    assert!(
        arena
            .motors
            .row(rig::RED)
            .is_some_and(|row| row.route.is_empty()),
        "with no path budget the hull has no route, which is what makes this the one-cell case"
    );
    assert!(
        arena
            .unit(rig::RED)
            .is_some_and(|unit| unit.desired_speed_cms > 0),
        "and it is driving at its goal"
    );

    // The cell in front of it — the one step on the bearing to its goal,
    // which is the `west` node and therefore the cell to the **west**. Taking
    // `here.0 + 1` on faith would write a cell behind the hull into the list
    // and assert nothing at all.
    let goal = rig::motor(&arena, rig::RED)
        .goal
        .expect("the script's goal is held");
    assert_eq!(goal.1, here.1, "the drive is along the hull's own row");
    let ahead = if goal.0 < here.0 {
        (here.0.saturating_sub(1), here.1)
    } else {
        (here.0.saturating_add(1), here.1)
    };
    let at_ms = arena.now_ms;
    arena.note_blocked(rig::RED, ahead, BlockedCause::Occupied, at_ms);
    let _ = m44b_drive_to(&mut arena, at_ms + 3_000, &mut bytes, &mut counters);
    let held = arena
        .unit(rig::RED)
        .map_or(-1, |unit| unit.desired_speed_cms);
    assert_eq!(
        held, 0,
        "a routeless hull whose next cell is one it was stopped at should hold, and it was \
         ordered {held} cm/s at {ahead:?}"
    );
    assert_eq!(
        rig::motor(&arena, rig::RED).goal,
        Some(goal),
        "the standstill is the avoid list's and not an arrival's: the goal is still held"
    );

    // And it goes again when the entry ages out — the list ages, it is not
    // cleared on a goal change.
    let _ = m44b_drive_to(
        &mut arena,
        at_ms + tacsim::arena::motor::REFLEX_AVOID_TTL_MS + 3_000,
        &mut bytes,
        &mut counters,
    );
    assert!(
        arena
            .unit(rig::RED)
            .is_some_and(|unit| unit.desired_speed_cms > 0),
        "the remembered stop should have aged out and the hull driven on"
    );
}

/// Step `arena` to `until_ms` in pulses of 500 ms, settled between pulses,
/// absorbing the ledger and the counters.
fn m44b_drive_to(
    arena: &mut Arena,
    until_ms: i64,
    bytes: &mut Vec<u8>,
    counters: &mut MotorCounters,
) -> i64 {
    let mut wall = arena.now_ms;
    while arena.now_ms < until_ms {
        let out = arena.step_settled(wall, &[], &[]);
        rig::absorb(&out, bytes);
        counters.absorb(&out.motor);
        if out.ended.is_some() {
            break;
        }
        wall += 500;
    }
    arena.now_ms
}

// ---------------------------------------------------------------------------
// 17b cut M4.4 — motor family C (16b cut AI.1: the orbit, the corner, the
// pinch and the stop beside the route)
// ---------------------------------------------------------------------------
//
// The reflex fixtures of `tests/reflex.rs`' 16b cut AI.1 family (16c `B-84`,
// "the machines stop searching in circles"), every edge of which is the
// motor's — `Arena::follow`'s circle rule (the route kept while the hull is
// inside its turning circle, the aim at a route cell it can turn onto, the
// slowing for one it cannot), its wander rule (the `2 r + cell_cm`
// diameter), its bend, the unlimited hull's hold at a corner closed on both
// shoulders with its route kept, `Arena::first_step_is_open`'s open-shoulder
// aim and the search's shoulder reads (`corner_refusals`), A*'s refused-edge
// detour and the follower's "follow, don't shorten" — moved here by 17b cut
// M4.4 onto the AI.1 window with every machine hull a `Controller::Mind`'s,
// flown by a scripted mind (`support::scripted::ScriptedMind`) whose goals
// land at the reflex's own milliseconds: think `n` at `think_at(unit, n)`
// (`stagger_ms(SEED, unit, 1_000) + n × 1_000`), its answer landing one
// substep on (250 ms: the slow tank's 1,500 cm/s on 1,000 cm cells is still
// bound by the 250 ms cap) and driven there (`Arena::motor_step` at the
// landing, D-UM-10), then the hull's own `MotorTick` every `decide_ms` (500)
// on the stagger grid — the reflex tick's own slots. The reflex fixtures'
// graph of nodes joined to nothing handed each machine exactly one goal (the
// node nearest it, its `Patrol`'s pick); the script hands the same cell, and
// the window keeps its nodes. A hull fielded online — at speed, or at rest —
// is handed its goal at think 0, landing one substep after the reflex's
// first tick at the same stagger (`RED`'s think 482 lands 732 against the
// tick at 482); a cold hull is handed `Startup` at think 0 and its goal at
// think 1, the first online think (a `Goal` is a continuous intent, refused
// `PowerNotOnline` on a cold hull: `RED`'s `Startup` lands 732, the hull is
// online at 1,232 and the goal lands 1,732, where the reflex machine set its
// own at its second tick, 982). Every window below is the reflex fixture's
// own; the one fixture constant that moved with the cadence is named in the
// test that moved it ([`M44C_CORNER_AT_SPEED_Y`]), and none of the claims
// did. The counters are read off `StepOutput::reflex` as the motor's
// (`MotorCounters`, M4.5's `MotorCounters`).

/// **A tank on the shipped line tank's numbers** (16b cut AI.1;
/// `tests/reflex.rs`' `slow_tank`): 1,500 cm/s at 30,000 mdeg/s, 100 / 200
/// cm/s² — a turning circle of 2,865 cm at speed, so a neighbour cell can be
/// inside it. The rig's [`rig::tank`] turns at 180 degrees a second (318 cm
/// at speed), inside which no neighbour cell ever falls. Unarmed, a mind's.
fn m44c_slow_tank(
    id: ObjId,
    team: Team,
    pose: mush_sim::data::Pose,
) -> tacsim::arena::state::CustodyUnit {
    use tacsim::arena::state::{Controller, CustodyUnit, TrackedRules};

    CustodyUnit {
        mobility: TrackedRules {
            max_speed_cms: 1_500,
            accel_cms2: 100,
            brake_cms2: 200,
            turn_rate_mdeg_s: 30_000,
            reverse_factor_pct: 40,
        },
        ..rig::tank(id, team, pose, Vec::new(), Controller::Mind)
    }
    .sealed()
}

/// **A window of `width` by `height`** (16b cut AI.1; `tests/reflex.rs`'
/// `ai1_window`): a turning circle of 29 m needs more room than the rig's
/// sixteen cells, and a graph of `nodes` **joined to nothing** — under the
/// reflex each node was the goal of the machine standing nearest it; under a
/// mind the script hands that cell, and the nodes stay for the window's
/// identity (a mind's goal falls back to no waypoint: `Fallback::Notice`).
fn m44c_window(
    width: u16,
    height: u16,
    paint: &[(u16, u16, tacsim::arena::state::TerrainCell)],
    nodes: &[(u16, u16)],
) -> tacsim::arena::state::ArenaDefinition {
    use smol_str::SmolStr;
    use tacsim::arena::state::{ArenaDefinition, ArenaTerrain, EntryZone, Waypoint, Waypoints};

    let mut cells = vec![rig::ground(); usize::from(width) * usize::from(height)];
    for (x, y, cell) in paint {
        if let Some(slot) = cells.get_mut(usize::from(*y) * usize::from(width) + usize::from(*x)) {
            *slot = *cell;
        }
    }
    let ids: Vec<SmolStr> = (0..nodes.len())
        .map(|n| SmolStr::new(format!("n{n}")))
        .collect();
    let points = nodes
        .iter()
        .zip(&ids)
        .map(|((x, y), id)| Waypoint {
            id: id.clone(),
            x: *x,
            y: *y,
        })
        .collect();
    let edges: Vec<(SmolStr, SmolStr)> = Vec::new();
    ArenaDefinition {
        id: SmolStr::new("tacsim.arena.reflex.ai1"),
        terrain: ArenaTerrain::new(width, height, rig::CELL_CM, cells).expect("the AI.1 window"),
        entry_zones: vec![
            EntryZone {
                team: Team::Red,
                x0: 0,
                y0: 0,
                x1: 1,
                y1: height - 1,
            },
            EntryZone {
                team: Team::Blue,
                x0: width - 2,
                y0: 0,
                x1: width - 1,
                y1: height - 1,
            },
        ],
        key_spots: Vec::new(),
        waypoints: Waypoints::new(points, &edges),
        modes: Vec::new(),
    }
}

/// The doctrine the AI.1 fixtures run on (`tests/reflex.rs`' `ai1_cfg`): a
/// `MotorTick` every 500 ms and the stock `repath_ms`, so a search spent is a
/// search counted and a route kept is a search saved.
fn m44c_cfg() -> tacsim::config::ArenaConfig {
    rig::arena_cfg(tacsim::config::ReflexConfig {
        decide_ms: 500,
        repath_ms: 4_000,
        ..tacsim::config::ReflexConfig::default()
    })
}

/// **Put `unit` online and at full speed on `heading_mdeg`** before the first
/// pulse (`tests/reflex.rs`' `at_speed`) — a hull that is already driving
/// when its goal lands, which is where the turning circle matters.
fn m44c_at_speed(arena: &mut Arena, unit: ObjId, heading_mdeg: u32) {
    if let Some(state) = arena.unit_mut(unit) {
        state.power = Power::Online;
        state.pose.heading_mdeg = heading_mdeg;
        state.desired_heading_mdeg = heading_mdeg;
        state.speed_cms = i32::try_from(state.mobility.max_speed_cms).unwrap_or(0);
        state.desired_speed_cms = state.speed_cms;
    }
}

/// Make `unit` a limited one — the stock locomotion's 45 per cent — the way
/// every shipped chassis is (`tests/reflex.rs`' `limited`: a fixture's
/// hand-made unit declares no mobility, so it is written on here rather than
/// read off a spec).
fn m44c_limited(arena: &mut Arena, unit: ObjId) {
    if let Some(state) = arena.unit_mut(unit) {
        state.slope_limit_pct = Some(45);
    }
}

/// One scripted goal cell.
fn m44c_goal(x: u16, y: u16) -> tacsim::arena::intents::IntentKind {
    tacsim::arena::intents::IntentKind::Goal(tacsim::arena::intents::MotorGoal::Cell { x, y })
}

/// The script of a hull fielded **online**: its goal at think 0, landing one
/// substep after the reflex's first tick at the same stagger.
fn m44c_hot(goal: (u16, u16)) -> Vec<support::scripted::Says> {
    vec![support::scripted::says(0, vec![m44c_goal(goal.0, goal.1)])]
}

/// The script of a **cold** hull: `Startup` at think 0 and its goal at think
/// 1, the first online think.
fn m44c_cold(goal: (u16, u16)) -> Vec<support::scripted::Says> {
    vec![
        support::scripted::says(0, vec![tacsim::arena::intents::IntentKind::Startup]),
        support::scripted::says(1, vec![m44c_goal(goal.0, goal.1)]),
    ]
}

/// Install `script` as the arena's mind runner — every machine hull of the
/// field reads its answers off it.
fn m44c_script(arena: &mut Arena, script: &[(ObjId, Vec<support::scripted::Says>)]) {
    arena.mind_runner = support::scripted::ScriptedMind::new(script);
}

/// What one AI.1 run leaves (`tests/reflex.rs`' `Ai1Run`): the counters, the
/// cell track pulse by pulse, the easting pulse by pulse, and the `blocked`
/// lines.
struct M44cRun {
    counters: MotorCounters,
    track: Vec<(u16, u16)>,
    xs: Vec<i64>,
    blocked: usize,
}

/// Step `arena` to `until_ms` at 500 ms, settled between pulses, recording
/// `unit`'s cell after every pulse and running `script` after each
/// (`tests/reflex.rs`' `ai1_run`).
fn m44c_run(
    arena: &mut Arena,
    unit: ObjId,
    until_ms: i64,
    script: &mut dyn FnMut(&mut Arena),
) -> M44cRun {
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall = arena.now_ms;
    let mut counters = MotorCounters::default();
    let mut track: Vec<(u16, u16)> = Vec::new();
    let mut xs: Vec<i64> = Vec::new();
    while arena.now_ms < until_ms {
        let out = arena.step_settled(wall, &[], &[]);
        rig::absorb(&out, &mut bytes);
        counters.absorb(&out.motor);
        if let Some(cell) = cell_of(arena, unit)
            && track.last() != Some(&cell)
        {
            track.push(cell);
        }
        if let Some(state) = arena.unit(unit) {
            xs.push(state.pose.x_cm);
        }
        script(arena);
        wall += 500;
    }
    let log = rig::events(arena, &mut bytes);
    M44cRun {
        counters,
        track,
        xs,
        blocked: rig::count(&log, "blocked"),
    }
}

/// Whether `unit` stands within [`tacsim::arena::motor::ARRIVE_CELLS`] of
/// `goal`.
fn m44c_arrived_at(arena: &Arena, unit: ObjId, goal: (u16, u16)) -> bool {
    cell_of(arena, unit).is_some_and(|(x, y)| {
        x.abs_diff(goal.0).max(y.abs_diff(goal.1)) <= tacsim::arena::motor::ARRIVE_CELLS
    })
}

/// Whether a cell track ever comes back to a cell it has left.
fn m44c_revisits(track: &[(u16, u16)]) -> Option<(u16, u16)> {
    track
        .iter()
        .enumerate()
        .find(|(at, cell)| track.iter().skip(at + 1).any(|later| later == *cell))
        .map(|(_, cell)| *cell)
}

/// How many `blocked` lines of `unit` the log holds for `cause`, or for every
/// cause but `Occupied` with `None` (`tests/reflex.rs`' `stops_of`).
fn m44c_stops_of(
    events: &[tacsim::arena::ledger::LedgerEvent],
    unit: ObjId,
    cause: Option<BlockedCause>,
) -> Vec<(i64, (u16, u16))> {
    events
        .iter()
        .filter_map(|event| match event.kind {
            tacsim::arena::ledger::EventKind::Blocked {
                unit: who,
                x,
                y,
                cause: why,
            } if who == unit
                && cause.map_or(why != BlockedCause::Occupied, |cause| why == cause) =>
            {
                Some((event.t_ms, (x, y)))
            }
            _ => None,
        })
        .collect()
}

/// The unit a `startup` line names.
fn m44c_startup_unit(event: &tacsim::arena::ledger::LedgerEvent) -> Option<ObjId> {
    match event.kind {
        tacsim::arena::ledger::EventKind::Startup { unit, .. } => Some(unit),
        _ => None,
    }
}

/// **16b cut AI.1 (16c `B-84`, the orbit)** — `tests/reflex.rs`'
/// `ai1_a_follower_inside_its_circle_keeps_its_route_and_arrives`, moved
/// here by 17b cut M4.4: a hull at full speed whose goal is behind it. Its
/// route's first cells are behind it, and every one it turns toward comes
/// inside its 29 m circle on the way round: before AI.1 the follower steered
/// at the first route cell at full speed, circled it, drifted more than a
/// cell off it and threw the route and its search stamp away — a fresh
/// search every tick for as long as the orbit lasted. `Arena::follow` keeps
/// its route while it is inside the circle, aims at a route cell it can turn
/// onto, slows for one it cannot, and arrives on its one search. What moved:
/// the hull is a mind's, online at full speed east, and the goal is the
/// script's `Goal(Cell (4, 16))` — the window's one node, the reflex
/// `Patrol`'s pick — landing at 732 (think 0 at 482, one substep on) against
/// the reflex's tick at 482: 375 cm further round a 29 m circle, which the
/// circle rule is indifferent to. The four claims are the reflex fixture's:
/// arrived, at most two searches, at most four deferred ticks, no `blocked`
/// line.
#[test]
fn m44c_ai1_a_follower_inside_its_circle_keeps_its_route_and_arrives() {
    let goal = (4_u16, 16_u16);
    let mut arena = rig::arena_on(
        m44c_window(32, 32, &[], &[goal]),
        vec![m44c_slow_tank(rig::RED, Team::Red, rig::centre(20, 16))],
        &m44c_cfg(),
        &[],
    );
    m44c_script(&mut arena, &[(rig::RED, m44c_hot(goal))]);
    m44c_at_speed(&mut arena, rig::RED, 90_000);
    let run = m44c_run(&mut arena, rig::RED, 40_000, &mut |_| {});
    assert!(
        m44c_arrived_at(&arena, rig::RED, goal),
        "the hull never arrived: it stands at {:?} after {:?}; {:?}",
        cell_of(&arena, rig::RED),
        run.track,
        run.counters
    );
    assert!(
        run.counters.path_queries <= 2,
        "the hull searched {} times for one goal: {:?}",
        run.counters.path_queries,
        run.counters
    );
    assert!(
        run.counters.path_deferred <= 4,
        "the hull sat routeless tick after tick: {:?}",
        run.counters
    );
    assert_eq!(run.blocked, 0, "nothing stands in its way");
}

/// **16b cut AI.1, the orbit's two ordinary shapes** — `tests/reflex.rs`'
/// `ai1_a_bend_at_speed_and_a_push_beside_the_route_never_turn_back`, moved
/// here by 17b cut M4.4: a 90° bend round a wall taken at speed, and a hull
/// pushed a cell off a straight route at speed. Each arrives on at most two
/// searches, and neither turns back along its route to a cell it has passed
/// — `Arena::follow`'s bend and wander rules. What moved: both hulls are a
/// mind's, online at full speed, each goal the script's (the window's one
/// node), landing at 732 against the reflex's tick at 482; the hand shove at
/// 4 s is the fixture's own millisecond, and the claims are its own.
#[test]
fn m44c_ai1_a_bend_at_speed_and_a_push_beside_the_route_never_turn_back() {
    // The bend: a wall down column 8 from row 0 to row 20, the hull at (4, 4)
    // driving north, the goal at (20, 24) past the wall's end. Its route runs
    // north beside the wall and turns east round its end.
    let goal = (20_u16, 24_u16);
    let wall_cells: Vec<(u16, u16, tacsim::arena::state::TerrainCell)> =
        (0..=20).map(|y| (8_u16, y, rig::wall())).collect();
    let mut arena = rig::arena_on(
        m44c_window(32, 32, &wall_cells, &[goal]),
        vec![m44c_slow_tank(rig::RED, Team::Red, rig::centre(4, 4))],
        &m44c_cfg(),
        &[],
    );
    m44c_script(&mut arena, &[(rig::RED, m44c_hot(goal))]);
    m44c_at_speed(&mut arena, rig::RED, 0);
    let run = m44c_run(&mut arena, rig::RED, 40_000, &mut |_| {});
    assert!(
        m44c_arrived_at(&arena, rig::RED, goal),
        "the bend: never arrived, at {:?} after {:?}; {:?}",
        cell_of(&arena, rig::RED),
        run.track,
        run.counters
    );
    assert!(
        run.counters.path_queries <= 2,
        "the bend: {:?}",
        run.counters
    );
    assert_eq!(
        m44c_revisits(&run.track),
        None,
        "the bend: the hull came back to a cell it had left: {:?}",
        run.track
    );
    assert_eq!(run.blocked, 0, "the bend: the wall stopped it");

    // The push: the hull drives east along row 16 at speed to (28, 16); at
    // 4 s it is shoved one cell north, beside its route. It carries on east.
    let goal = (28_u16, 16_u16);
    let mut arena = rig::arena_on(
        m44c_window(32, 32, &[], &[goal]),
        vec![m44c_slow_tank(rig::RED, Team::Red, rig::centre(4, 16))],
        &m44c_cfg(),
        &[],
    );
    m44c_script(&mut arena, &[(rig::RED, m44c_hot(goal))]);
    m44c_at_speed(&mut arena, rig::RED, 90_000);
    let mut shoved = false;
    let run = m44c_run(&mut arena, rig::RED, 40_000, &mut |arena: &mut Arena| {
        if !shoved && arena.now_ms >= 4_000 {
            shoved = true;
            if let Some(state) = arena.unit_mut(rig::RED) {
                state.pose.y_cm += i64::from(rig::CELL_CM);
            }
        }
    });
    assert!(
        m44c_arrived_at(&arena, rig::RED, goal),
        "the push: never arrived, at {:?} after {:?}; {:?}",
        cell_of(&arena, rig::RED),
        run.track,
        run.counters
    );
    assert!(
        run.counters.path_queries <= 2,
        "the push: {:?}",
        run.counters
    );
    assert!(
        run.xs.windows(2).all(|pair| match pair {
            [a, b] => b >= a,
            _ => true,
        }),
        "the push: the hull turned back west along its route: {:?}",
        run.xs
    );
}

/// **The at-speed corner hull's start, one substep further back than the
/// reflex fixture's** (`y_cm` 7,625 against 8,000) — a fixture constant
/// re-derived for the cadence, not the pin. The reflex's tick at 482 found
/// the hull, driving north at 1,500 cm/s from `y_cm` 8,000, 723 cm up and
/// "still in (16, 8)"; the mind's goal lands at 732, 1,098 cm up — cell 9
/// from 8,000. From 7,625 the landing's search still runs from (16, 8)
/// (8,723 at 732), and the per-pulse track reads (16, 7) at the opening
/// pulse, (16, 8) at 500 and (16, 9) at 1,000: the cell entered **from the
/// corner** is the open shoulder, as before, one index on.
const M44C_CORNER_AT_SPEED_Y: i64 = 7_625;

/// **The corner of `B-84`** (16b cut AI.1; `tests/reflex.rs`' `ai1_corner`):
/// the route's first step is the diagonal `(15, 9)` from `(16, 8)`, whose
/// west shoulder `(15, 8)` is a wall and whose north shoulder `(16, 9)` is
/// open, and the hull stands with its `x_cm` exactly on the column boundary
/// — the shape the AE sweep found unit 100164 in, at `x_cm` 206,000. Any
/// line from there toward the diagonal cell enters the wall first. From rest
/// (online, facing north) or driving north up the boundary at full speed
/// (from [`M44C_CORNER_AT_SPEED_Y`]); the goal is the script's `Goal(Cell
/// (6, 18))` at think 0.
fn m44c_corner(at_speed_north: bool) -> Arena {
    let goal = (6_u16, 18_u16);
    let pose = if at_speed_north {
        rig::pose(16_000, M44C_CORNER_AT_SPEED_Y)
    } else {
        rig::pose(16_000, 8_500)
    };
    let mut arena = rig::arena_on(
        m44c_window(32, 32, &[(15, 8, rig::wall())], &[goal]),
        vec![m44c_slow_tank(rig::RED, Team::Red, pose)],
        &m44c_cfg(),
        &[],
    );
    m44c_script(&mut arena, &[(rig::RED, m44c_hot(goal))]);
    m44c_limited(&mut arena, rig::RED);
    if at_speed_north {
        m44c_at_speed(&mut arena, rig::RED, 0);
    } else if let Some(state) = arena.unit_mut(rig::RED) {
        // At rest, online, facing north.
        state.power = Power::Online;
    }
    arena
}

/// **16b cut AI.1 (16c `B-84`, the corner cut)** — `tests/reflex.rs`'
/// `ai1_a_unit_on_a_corner_steps_through_the_open_shoulder`, moved here by
/// 17b cut M4.4: a limited hull on a corner one of whose shoulders is a wall
/// steps through the open shoulder. Before AI.1 the follower steered at the
/// diagonal cell's centre, the line crossed the west boundary first, motion
/// stopped the hull `impassable` on the wall shoulder, `note_blocked` threw
/// the route away, and after `repath_ms` the identical route was searched
/// and driven into the same wall. `Arena::first_step_is_open`'s aim is the
/// open shoulder's centre: no `blocked` line, one search, the open shoulder
/// entered first, and no cell entered twice — from rest, and driving up the
/// boundary at full speed. What moved: the hull is a mind's, online in both
/// shapes, its goal the script's at think 0 (landing 732 against the
/// reflex's tick at 482), and the at-speed start is one substep further back
/// ([`M44C_CORNER_AT_SPEED_Y`]) so the landing's search runs from the same
/// cell the reflex's tick searched from — which puts the opening pulse's
/// cell, (16, 7), at the head of the at-speed track, so the "first cell
/// entered" read is the track's prefix per shape, the corner cell then the
/// open shoulder: `[(16, 8), (16, 9)]` from rest, `[(16, 7), (16, 8), (16,
/// 9)]` at speed. The six claims are the fixture's.
#[test]
fn m44c_ai1_a_unit_on_a_corner_steps_through_the_open_shoulder() {
    for at_speed_north in [false, true] {
        let mut arena = m44c_corner(at_speed_north);
        let run = m44c_run(&mut arena, rig::RED, 30_000, &mut |_| {});
        let (how, prefix): (&str, &[(u16, u16)]) = if at_speed_north {
            ("at speed", &[(16, 7), (16, 8), (16, 9)])
        } else {
            ("from rest", &[(16, 8), (16, 9)])
        };
        assert_eq!(
            run.blocked, 0,
            "{how}: the hull was stopped on the corner: {:?}",
            run.track
        );
        assert_eq!(
            run.counters.path_queries, 1,
            "{how}: {:?} after {:?}",
            run.counters, run.track
        );
        assert!(
            run.track.starts_with(prefix),
            "{how}: the first cell it entered from the corner was not the open shoulder: {:?}",
            run.track
        );
        assert!(
            !run.track.contains(&(15, 8)),
            "{how}: it entered the wall: {:?}",
            run.track
        );
        assert_eq!(
            m44c_revisits(&run.track),
            None,
            "{how}: a lap round the shoulder: {:?}",
            run.track
        );
        assert!(
            m44c_arrived_at(&arena, rig::RED, (6, 18)),
            "{how}: never arrived: {:?}",
            run.track
        );
    }
}

/// **16b cut AI.1** — `tests/reflex.rs`'
/// `ai1_a_corner_closed_on_both_shoulders_holds_an_unlimited_unit_and_ages_an_avoided_one`,
/// moved here by 17b cut M4.4: a corner closed on both shoulders, and an
/// unlimited hull whose search (which reads no shoulder without a limit)
/// plans straight across it. `Arena::follow`'s own rule refuses the step —
/// neither shoulder is open, so motion would stop it on one — and an
/// unlimited hull **holds**: its route kept, no search spent, nothing
/// remembered, never past the boundary. Then the same corner closed by two
/// **parked** tanks: the hull is stopped against one shoulder (`Occupied`,
/// remembered), steps through the other and is stopped against that too,
/// and holds with both remembered. When one tank drives off, the hull
/// proceeds once its memory of that cell has aged out
/// ([`tacsim::arena::motor::REFLEX_AVOID_TTL_MS`]) — through the shoulder
/// that is now empty. What moved: the hull is a mind's and cold on both
/// fields, so its `Startup` is the script's at think 0 and its goal the
/// script's `Goal(Cell (12, 12))` at think 1, landing at 1,732 where the
/// reflex machine set its own at its second tick, 982 — a think period and
/// a substep later, inside the 10 s window's and the 30 s loop's slack; the
/// reflex row's "a machine" (`reflex.unit(RED).is_some()`) is the goal still
/// held on the motor row (the kept route raises no `NoRoute` under
/// `Fallback::Notice`, so the one scripted goal stands across the TTL); every
/// other claim is the fixture's.
#[test]
fn m44c_ai1_a_corner_closed_on_both_shoulders_holds_an_unlimited_unit_and_ages_an_avoided_one() {
    use tacsim::arena::state::Controller;

    let goal = (12_u16, 12_u16);
    // The walls.
    let mut arena = rig::arena_on(
        m44c_window(16, 16, &[(6, 5, rig::wall()), (5, 6, rig::wall())], &[goal]),
        vec![rig::tank(
            rig::RED,
            Team::Red,
            rig::centre(5, 5),
            Vec::new(),
            Controller::Mind,
        )],
        &m44c_cfg(),
        &[],
    );
    m44c_script(&mut arena, &[(rig::RED, m44c_cold(goal))]);
    let run = m44c_run(&mut arena, rig::RED, 10_000, &mut |_| {});
    assert_eq!(run.counters.path_queries, 1, "{:?}", run.counters);
    assert_eq!(run.counters.path_refused_unchanged, 0, "{:?}", run.counters);
    assert_eq!(run.blocked, 0);
    assert_eq!(run.track, vec![(5, 5)], "the hull went past the boundary");
    let drive = rig::motor(&arena, rig::RED);
    assert_eq!(drive.goal, Some(goal), "the goal is still held");
    assert_eq!(
        drive.route.first().copied(),
        Some((6, 6)),
        "the route across the corner is kept: {:?}",
        drive.route
    );
    assert!(drive.avoid.is_empty());
    assert_eq!(drive.refused, None);

    // The parked tanks. The hull is posed south-east of its cell's centre, so
    // its line to the diagonal enters the east shoulder first. The traffic
    // hold is off (`occupied_stop_ms` 0, the rule before 16b cut E.3): a held
    // hull slides along the boundary and can cross the corner diagonally in
    // one substep — motion tests the diagonal cell alone — and this fixture
    // is about the stops.
    let mut cfg = m44c_cfg();
    cfg.occupied_stop_ms = 0;
    let mut arena = rig::arena_on(
        m44c_window(16, 16, &[], &[goal]),
        vec![
            rig::tank(
                rig::RED,
                Team::Red,
                rig::pose(5_800, 5_200),
                Vec::new(),
                Controller::Mind,
            ),
            rig::tank(
                rig::RED2,
                Team::Red,
                rig::centre(6, 5),
                Vec::new(),
                Controller::Pilot,
            ),
            rig::tank(
                rig::BLUE,
                Team::Red,
                rig::centre(5, 6),
                Vec::new(),
                Controller::Pilot,
            ),
        ],
        &cfg,
        &[],
    );
    m44c_script(&mut arena, &[(rig::RED, m44c_cold(goal))]);
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall_ms = 0_i64;
    let mut both_learned_at: Option<i64> = None;
    let mut held_both: Vec<(i64, (u16, u16))> = Vec::new();
    while arena.now_ms < 30_000 {
        let out = arena.step_settled(wall_ms, &[], &[]);
        rig::absorb(&out, &mut bytes);
        let learned = rig::avoided(&arena, rig::RED);
        if both_learned_at.is_none() && learned.contains(&(6, 5)) && learned.contains(&(5, 6)) {
            both_learned_at = Some(arena.now_ms);
        }
        if both_learned_at.is_some()
            && let Some(cell) = cell_of(&arena, rig::RED)
        {
            held_both.push((arena.now_ms, cell));
        }
        wall_ms += 500;
    }
    let both_learned_at = both_learned_at.expect("the hull never learned both shoulders");
    let log = rig::events(&mut arena, &mut bytes);
    let occupied = m44c_stops_of(&log, rig::RED, Some(BlockedCause::Occupied));
    assert_eq!(
        occupied.iter().map(|(_, cell)| *cell).collect::<Vec<_>>(),
        vec![(6, 5), (5, 6)],
        "stopped once against each parked tank: {occupied:?}"
    );
    assert!(
        held_both.iter().all(|(_, cell)| *cell == (5, 5)),
        "with both shoulders remembered it held: {held_both:?}"
    );
    assert_eq!(
        arena
            .motors
            .row(rig::RED)
            .and_then(|row| row.route.first().copied()),
        Some((6, 6)),
        "and held with its route"
    );
    assert_eq!(
        rig::motor(&arena, rig::RED).goal,
        Some(goal),
        "and with its goal: the hold is the shoulders' and not an arrival's"
    );
    // One tank drives off; the hull's memory of its cell ages out and it
    // proceeds through the empty shoulder.
    if let Some(state) = arena.unit_mut(rig::BLUE) {
        state.pose = rig::centre(1, 14);
    }
    let until = both_learned_at + tacsim::arena::motor::REFLEX_AVOID_TTL_MS + 20_000;
    let run = m44c_run(&mut arena, rig::RED, until, &mut |_| {});
    assert!(
        m44c_arrived_at(&arena, rig::RED, goal),
        "the hull never proceeded: {:?}; {:?}",
        run.track,
        run.counters
    );
}

/// **16b cut AI.1 (16c `B-84`, the wander rule)** — `tests/reflex.rs`'
/// `ai1_a_shove_inside_the_circle_keeps_the_route_and_beyond_it_re_plans`,
/// moved here by 17b cut M4.4: a hull shoved two cells off its route at full
/// speed is inside its turning circle's diameter of the route (`2 r +
/// cell_cm` = 6,730 cm at 1,500 cm/s) and keeps the route: no search for
/// `repath_ms`. Shoved nine cells it is not, and its next tick searches —
/// `Arena::follow`'s step 3. What moved: the hull is a mind's, online at
/// full speed east, its goal the script's `Goal(Cell (28, 8))` at think 0
/// (landing 732 against the reflex's tick at 482: the one search before the
/// 4 s shove); the two windows are the fixture's own — the `MotorTick`
/// cadence is `reflex.decide_ms` on the reflex tick's own grid, so "one
/// decide tick and the pulse that carries it" is the same bound.
#[test]
fn m44c_ai1_a_shove_inside_the_circle_keeps_the_route_and_beyond_it_re_plans() {
    for (cells, searches) in [(2_i64, 0_u32), (9, 1)] {
        let goal = (28_u16, 8_u16);
        let mut arena = rig::arena_on(
            m44c_window(32, 32, &[], &[goal]),
            vec![m44c_slow_tank(rig::RED, Team::Red, rig::centre(3, 8))],
            &m44c_cfg(),
            &[],
        );
        m44c_script(&mut arena, &[(rig::RED, m44c_hot(goal))]);
        m44c_at_speed(&mut arena, rig::RED, 90_000);
        // To the shove: the one search of the run, and the route.
        let before = m44c_run(&mut arena, rig::RED, 4_000, &mut |_| {});
        assert_eq!(before.counters.path_queries, 1, "{:?}", before.counters);
        if let Some(state) = arena.unit_mut(rig::RED) {
            state.pose.y_cm += cells * i64::from(rig::CELL_CM);
        }
        let window = if searches == 0 {
            // `repath_ms`: a search inside it would be the wander rule's.
            4_000 + i64::from(m44c_cfg().reflex.repath_ms)
        } else {
            // One motor tick, and the pulse that carries it.
            4_000 + i64::from(m44c_cfg().reflex.decide_ms) + 500
        };
        let after = m44c_run(&mut arena, rig::RED, window, &mut |_| {});
        assert_eq!(
            after.counters.path_queries, searches,
            "shoved {cells} cells: {:?} after {:?}",
            after.counters, after.track
        );
    }
}

// ---- the cross-tempo field (16b §11: mandatory) -----------------------------

/// The orbiting follower of the AI.1 field.
const M44C_ORBIT: ObjId = ObjId(21);
/// The corner unit.
const M44C_CORNER: ObjId = ObjId(22);
/// The pinch unit.
const M44C_PINCH: ObjId = ObjId(23);
/// The unit stopped by the ground beside its route.
const M44C_BESIDE: ObjId = ObjId(24);
/// The seated tank they detect.
const M44C_SEATED: ObjId = ObjId(25);

/// **The AI.1 field** (16b cut AI.1; `tests/reflex.rs`' `ai1_field`): one
/// window, four machine hulls each with a goal of its own — the node nearest
/// it, now the script's — and a seated tank.
///
/// - [`M44C_ORBIT`], the follower of
///   [`m44c_ai1_a_follower_inside_its_circle_keeps_its_route_and_arrives`]: at
///   full speed, its goal `(28, 56)` behind it, handed at think 0.
/// - [`M44C_CORNER`], limited, on the corner of
///   [`m44c_ai1_a_unit_on_a_corner_steps_through_the_open_shoulder`], at rest
///   and cold: `Startup` at think 0, `(42, 18)` at think 1.
/// - [`M44C_PINCH`], limited, cold, at a two-wall pinch on the window's south
///   edge: walls east and north of it and its goal `(12, 0)` two cells east,
///   so the search refuses the corner between the walls and plans round —
///   west, north, and back **beside its start** at the corner's far cell,
///   across the closed corner.
/// - [`M44C_BESIDE`], cold, a few centimetres below its cell's north
///   boundary, heading 30° with a wall to its north and its goal `(20, 30)`
///   to the east: its first substeps under power carry it into the wall
///   while its heading turns toward the route (the M.2 shape), which is one
///   `impassable` stop **beside** its route, and the route is kept.
/// - [`M44C_SEATED`], a seated tank of the other side, parked where the
///   orbit follower arrives, so the run detects and classifies.
fn m44c_field() -> Arena {
    use tacsim::arena::state::Controller;

    let paint = [
        // The corner.
        (51_u16, 8_u16, rig::wall()),
        // The pinch.
        (11, 0, rig::wall()),
        (10, 1, rig::wall()),
        // Beside the route.
        (10, 31, rig::wall()),
    ];
    let nodes = [(28_u16, 56_u16), (42, 18), (12, 0), (20, 30)];
    let mut arena = rig::arena_on(
        m44c_window(64, 64, &paint, &nodes),
        vec![
            m44c_slow_tank(M44C_ORBIT, Team::Red, rig::centre(44, 56)),
            m44c_slow_tank(M44C_CORNER, Team::Red, rig::pose(52_000, 8_500)),
            m44c_slow_tank(M44C_PINCH, Team::Red, rig::centre(10, 0)),
            m44c_slow_tank(M44C_BESIDE, Team::Red, rig::pose(10_500, 30_990)),
            rig::tank(
                M44C_SEATED,
                Team::Blue,
                rig::centre(26, 59),
                Vec::new(),
                Controller::Pilot,
            ),
        ],
        &m44c_cfg(),
        &[M44C_SEATED],
    );
    m44c_script(
        &mut arena,
        &[
            (M44C_ORBIT, m44c_hot((28, 56))),
            (M44C_CORNER, m44c_cold((42, 18))),
            (M44C_PINCH, m44c_cold((12, 0))),
            (M44C_BESIDE, m44c_cold((20, 30))),
        ],
    );
    m44c_at_speed(&mut arena, M44C_ORBIT, 90_000);
    for unit in [M44C_CORNER, M44C_PINCH] {
        m44c_limited(&mut arena, unit);
    }
    if let Some(state) = arena.unit_mut(M44C_PINCH) {
        state.pose.heading_mdeg = 315_000;
    }
    if let Some(state) = arena.unit_mut(M44C_BESIDE) {
        state.pose.heading_mdeg = 30_000;
    }
    arena
}

/// What one replay of an AI.1 field produced (`tests/reflex.rs`' `Replay`,
/// without the state column: the hulls are a script's, and what the script
/// decided is the goal): every ledger line with its hash, the per-substep
/// hash sequence, each unit's final pose, speed and goal, and how many
/// pulses it took (three cadences really are three different numbers of
/// pulses, so an equality of the three is a property and not three copies
/// of one run).
struct M44cReplay {
    events: Vec<(i64, String, u64)>,
    hashes: Vec<u64>,
    finals: Vec<M44cFinal>,
    pulses: u32,
}

/// One unit as a replay ends: where it is, how it is headed, how fast, and
/// the goal its script's mind holds.
type M44cFinal = (ObjId, i64, i64, u32, i32, Option<(u16, u16)>);

/// `build`'s field replayed at `cadence_ms` until `until_ms`, the tempo
/// pinned `Quiet` (D-16-2: the tempo decides only how often the arena steps)
/// and every pulse settled.
fn m44c_replay(cadence_ms: i64, until_ms: i64, build: fn() -> Arena) -> M44cReplay {
    let mut arena = build();
    arena
        .clock
        .pin_tempo_for_tests(tacsim::arena::clock::Tempo::Quiet);
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut wall = 0_i64;
    let mut pulses = 0_u32;
    loop {
        let out = arena.step_settled(wall, &[], &[]);
        pulses = pulses.saturating_add(1);
        rig::absorb(&out, &mut bytes);
        hashes.extend(arena.substep_hashes.iter().copied());
        if arena.now_ms >= until_ms || out.ended.is_some() {
            break;
        }
        wall += cadence_ms;
    }
    let events = rig::events(&mut arena, &mut bytes)
        .into_iter()
        .map(|event| {
            (
                event.t_ms,
                serde_json::to_string(&event.kind).unwrap_or_default(),
                event.hash,
            )
        })
        .collect();
    let finals = arena
        .units
        .iter()
        .map(|unit| {
            (
                unit.id,
                unit.pose.x_cm,
                unit.pose.y_cm,
                unit.pose.heading_mdeg,
                unit.speed_cms,
                rig::motor(&arena, unit.id).goal,
            )
        })
        .collect();
    M44cReplay {
        events,
        hashes,
        finals,
        pulses,
    }
}

/// The three cadences of `build`'s field to `until_ms`, held to one event
/// list, one hash sequence, one set of finals and three pulse counts.
fn m44c_three_cadences(until_ms: i64, build: fn() -> Arena) -> Vec<M44cReplay> {
    let runs: Vec<M44cReplay> = [5_000_i64, 1_000, 500]
        .into_iter()
        .map(|cadence| m44c_replay(cadence, until_ms, build))
        .collect();
    let first = runs.first().expect("three replays");
    for (at, run) in runs.iter().enumerate().skip(1) {
        let cadence = [5_000_i64, 1_000, 500].get(at).copied().unwrap_or(0);
        assert_eq!(
            run.events, first.events,
            "the event list diverged at {cadence} ms"
        );
        assert_eq!(
            run.hashes, first.hashes,
            "the hash sequence diverged at {cadence} ms"
        );
        assert_eq!(
            run.finals, first.finals,
            "the field ended differently at {cadence} ms"
        );
        assert_ne!(
            run.pulses, first.pulses,
            "three cadences, three pulse counts"
        );
    }
    runs
}

/// **16b cut AI.1's cross-tempo proof** (16b §11, mandatory; doc 16 §3.1's
/// D-16-2) — `tests/reflex.rs`'
/// `ai1_the_orbit_the_corner_and_a_stop_beside_the_route_are_one_log_at_three_cadences`,
/// moved here by 17b cut M4.4: the orbit, the corner, the pinch and a stop
/// beside the route, in one field, are one event list, one per-substep hash
/// sequence and one set of finals at 5,000, 1,000 and 500 ms — and the run
/// is worth comparing. Every edge is `Arena::follow`'s, the search's or
/// motion's (the circle, the open shoulder, the pinch's refused corner, the
/// one `impassable` stop beside the route with the route kept); the
/// controller supplied one goal each, and here the script does: the hot
/// orbit's at think 0, the three cold hulls' `Startup` at think 0 and goal
/// at think 1. What moved: the finals carry no state column (a scripted
/// mind has none to read), and the reflex's tick stamp — a `startup` line
/// at its unit's own tick — becomes the mind's form: every `startup` line
/// one substep after its unit's `stagger_ms(SEED, unit, 1_000)`, never on
/// it. Every other claim is the fixture's, the counters read off the
/// motor's.
#[test]
fn m44c_ai1_the_orbit_the_corner_and_a_stop_beside_the_route_are_one_log_at_three_cadences() {
    let runs = m44c_three_cadences(40_000, m44c_field);
    let first = runs.first().expect("three replays");

    // Worth comparing. The run again at 500 ms, watched pulse by pulse: the
    // counters, each hull's searches (a pulse at this cadence holds at most
    // one tick of each), and where each arrived.
    let mut arena = m44c_field();
    arena
        .clock
        .pin_tempo_for_tests(tacsim::arena::clock::Tempo::Quiet);
    let mut counters = MotorCounters::default();
    let mut searches: std::collections::BTreeMap<ObjId, u32> = std::collections::BTreeMap::new();
    let mut stamps: std::collections::BTreeMap<ObjId, Option<i64>> =
        std::collections::BTreeMap::new();
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall_ms = 0_i64;
    while arena.now_ms < 40_000 {
        let out = arena.step_settled(wall_ms, &[], &[]);
        rig::absorb(&out, &mut bytes);
        counters.absorb(&out.motor);
        for unit in [M44C_ORBIT, M44C_CORNER, M44C_PINCH, M44C_BESIDE] {
            let stamp = arena.motors.row(unit).and_then(|row| row.searched_ms);
            if stamp.is_some() && stamps.get(&unit).copied().flatten() != stamp {
                *searches.entry(unit).or_default() += 1;
            }
            stamps.insert(unit, stamp);
        }
        wall_ms += 500;
    }
    let log = rig::events(&mut arena, &mut bytes);
    assert!(counters.path_queries >= 1, "{counters:?}");
    // The counter is the field's, and it is the pinch's: the pinch's two walls
    // are the only pair on the window diagonal to each other, so no other
    // hull's search or one-cell test can meet a corner closed on both
    // shoulders.
    assert!(
        counters.corner_refusals > 0,
        "the pinch unit's search refused no corner: {counters:?}"
    );
    assert_eq!(counters.path_refused_unchanged, 0, "{counters:?}");
    assert!(
        m44c_arrived_at(&arena, M44C_ORBIT, (28, 56))
            || m44c_arrived_at(&arena, M44C_BESIDE, (20, 30)),
        "no hull arrived"
    );
    for unit in [M44C_ORBIT, M44C_CORNER, M44C_PINCH] {
        assert!(
            searches.get(&unit).copied().unwrap_or(0) <= 2,
            "#{} searched {:?} times",
            unit.0,
            searches.get(&unit)
        );
    }
    // The stop beside the route, and it is `M44C_BESIDE`'s: exactly one
    // `impassable` line, on the wall to its north, at one arena millisecond
    // at all three cadences (the event lists are equal) — and the route it
    // was stopped beside is **kept**, so that one stop costs no second
    // search.
    let beside = m44c_stops_of(&log, M44C_BESIDE, Some(BlockedCause::Impassable));
    assert_eq!(
        beside.iter().map(|(_, cell)| *cell).collect::<Vec<_>>(),
        vec![(10, 31)],
        "the unit beside its route was not stopped once on the wall to its north: {beside:?}"
    );
    assert_eq!(
        m44c_stops_of(&log, M44C_BESIDE, None),
        beside,
        "the unit beside its route was stopped by something else too"
    );
    assert_eq!(
        searches.get(&M44C_BESIDE).copied(),
        Some(1),
        "the stop beside the route threw the route away and bought a second search: {beside:?}"
    );
    // The corner and the pinch are never stopped by the ground: the corner
    // unit steps through its open shoulder and the pinch unit leaves by the
    // open side — a stop on a wall shoulder there would be `B-84`'s corner
    // stop, kept silent by the route-keeping rule above.
    for unit in [M44C_CORNER, M44C_PINCH] {
        assert!(
            m44c_stops_of(&log, unit, None).is_empty(),
            "#{} was stopped by the ground: {:?}",
            unit.0,
            m44c_stops_of(&log, unit, None)
        );
    }
    for unit in [M44C_ORBIT, M44C_CORNER, M44C_PINCH, M44C_BESIDE] {
        assert!(
            m44c_stops_of(&log, unit, Some(BlockedCause::TooSteep)).is_empty(),
            "#{} was stopped `too_steep`",
            unit.0
        );
    }
    // A mind's own action at its own landing: the cold hulls' `startup`
    // lines (16b cut M.2's stamp; the heading and speed orders journal no
    // line) — each one substep after its unit's think 0, never on it.
    let substep = arena.substep_len_ms();
    let startups: Vec<(ObjId, i64)> = log
        .iter()
        .filter(|event| event.kind.as_str() == "startup")
        .filter_map(|event| m44c_startup_unit(event).map(|unit| (unit, event.t_ms)))
        .collect();
    assert!(!startups.is_empty(), "no hull's startup was journalled");
    for (unit, t_ms) in &startups {
        let past_think = (t_ms - support::scripted::think_at(*unit, 0))
            .rem_euclid(tacsim::arena::mind::STOCK_THINK_MS);
        assert_ne!(
            past_think, 0,
            "#{}'s startup was stamped on its think, not its landing: {t_ms}",
            unit.0
        );
        assert_eq!(
            past_think, substep,
            "#{}'s startup was not stamped one substep after its think: {t_ms}",
            unit.0
        );
    }
    for kind in ["detected", "classified"] {
        assert!(
            first.events.iter().any(|(_, json, _)| json.contains(kind)),
            "the field never journalled a {kind}"
        );
    }
}

/// **The two routes that come back beside their start** (16b cut AI.1;
/// `tests/reflex.rs`' `ai1_come_back`): a limited hull at a two-wall pinch
/// on the window's south edge (walls east and north, its goal two cells
/// east), and a limited hull at the foot of an 800 cm plateau whose only way
/// up is a 400 cm ramp cell to its north-west. Both cold: `Startup` at think
/// 0 and the goal — `(12, 0)` and `(32, 11)`, the nodes — at think 1.
fn m44c_come_back() -> Arena {
    // The pinch, at (10, 0).
    let mut paint = vec![(11_u16, 0_u16, rig::wall()), (10, 1, rig::wall())];
    // The plateau: columns 29 to 36, rows 11 to 17, 800 cm up, with the ramp
    // at (29, 11), 400 cm up. The foot is (30, 10).
    for x in 29..=36_u16 {
        for y in 11..=17_u16 {
            let elev_cm = if (x, y) == (29, 11) { 400 } else { 800 };
            paint.push((
                x,
                y,
                tacsim::arena::state::TerrainCell {
                    elev_cm,
                    ..rig::ground()
                },
            ));
        }
    }
    let nodes = [(12_u16, 0_u16), (32, 11)];
    let mut arena = rig::arena_on(
        m44c_window(48, 48, &paint, &nodes),
        vec![
            m44c_slow_tank(M44C_PINCH, Team::Red, rig::centre(10, 0)),
            m44c_slow_tank(M44C_CORNER, Team::Red, rig::centre(30, 10)),
        ],
        &m44c_cfg(),
        &[],
    );
    m44c_script(
        &mut arena,
        &[
            (M44C_PINCH, m44c_cold((12, 0))),
            (M44C_CORNER, m44c_cold((32, 11))),
        ],
    );
    for unit in [M44C_PINCH, M44C_CORNER] {
        m44c_limited(&mut arena, unit);
    }
    if let Some(state) = arena.unit_mut(M44C_PINCH) {
        state.pose.heading_mdeg = 315_000;
    }
    arena
}

/// **16b cut AI.1: a route that comes back beside its start across an edge
/// the search refused is followed, not shortened** — `tests/reflex.rs`'
/// `ai1_a_limited_unit_whose_route_comes_back_beside_it_does_not_re_search`,
/// moved here by 17b cut M4.4. A\* detours to an adjacent cell exactly when
/// the direct edge is refused, so a follower that drained its route to any
/// adjacent later cell would aim across the closed corner (and re-plan every
/// tick) or up the bluff (and be stopped `too_steep` at its route's own next
/// cell). Neither is: each hull searches once, leaves its cell by the open
/// side, is never stopped by the ground, and arrives — at 5,000, 1,000 and
/// 500 ms alike. What moved: both hulls are a mind's and cold, their
/// `Startup` the script's at think 0 and their goals the script's at think 1
/// (the nodes the reflex's `Patrol` picked); the finals carry no state
/// column; every claim is the fixture's.
#[test]
fn m44c_ai1_a_limited_unit_whose_route_comes_back_beside_it_does_not_re_search() {
    let _runs = m44c_three_cadences(30_000, m44c_come_back);

    let mut arena = m44c_come_back();
    arena
        .clock
        .pin_tempo_for_tests(tacsim::arena::clock::Tempo::Quiet);
    // The routes as first planned, to prove the premise: each comes back
    // beside its start.
    let mut planned: std::collections::BTreeMap<ObjId, Vec<(u16, u16)>> =
        std::collections::BTreeMap::new();
    let mut searches: std::collections::BTreeMap<ObjId, u32> = std::collections::BTreeMap::new();
    let mut stamps: std::collections::BTreeMap<ObjId, Option<i64>> =
        std::collections::BTreeMap::new();
    let mut tracks: std::collections::BTreeMap<ObjId, Vec<(u16, u16)>> =
        std::collections::BTreeMap::new();
    let mut counters = MotorCounters::default();
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall_ms = 0_i64;
    while arena.now_ms < 30_000 {
        let out = arena.step_settled(wall_ms, &[], &[]);
        rig::absorb(&out, &mut bytes);
        counters.absorb(&out.motor);
        for unit in [M44C_PINCH, M44C_CORNER] {
            let drive = rig::motor(&arena, unit);
            if drive.searched_ms.is_some()
                && stamps.get(&unit).copied().flatten() != drive.searched_ms
            {
                *searches.entry(unit).or_default() += 1;
                planned.entry(unit).or_insert_with(|| drive.route.clone());
            }
            stamps.insert(unit, drive.searched_ms);
            if let Some(cell) = cell_of(&arena, unit) {
                let track = tracks.entry(unit).or_default();
                if track.last() != Some(&cell) {
                    track.push(cell);
                }
            }
        }
        wall_ms += 500;
    }
    let log = rig::events(&mut arena, &mut bytes);
    // The premises: a route cell adjacent to the start across a refused edge.
    let pinch = planned.get(&M44C_PINCH).cloned().unwrap_or_default();
    assert!(
        pinch.contains(&(11, 1)),
        "the pinch's route does not come back beside its start: {pinch:?}"
    );
    let plateau = planned.get(&M44C_CORNER).cloned().unwrap_or_default();
    assert!(
        plateau.contains(&(29, 11)) && plateau.contains(&(30, 11)),
        "the plateau's route does not climb the ramp and come back above its start: {plateau:?}"
    );
    assert!(counters.corner_refusals > 0, "{counters:?}");
    for (unit, goal, open) in [
        (M44C_PINCH, (12, 0), (9, 0)),
        (M44C_CORNER, (32, 11), (29, 10)),
    ] {
        assert_eq!(
            searches.get(&unit).copied(),
            Some(1),
            "#{} searched more than once: {counters:?}",
            unit.0
        );
        assert!(
            m44c_stops_of(&log, unit, None).is_empty(),
            "#{} was stopped by the ground: {:?}",
            unit.0,
            m44c_stops_of(&log, unit, None)
        );
        let track = tracks.get(&unit).cloned().unwrap_or_default();
        assert_eq!(
            track.get(1).copied(),
            Some(open),
            "#{} did not leave its cell by the open side: {track:?}",
            unit.0
        );
        assert!(
            m44c_arrived_at(&arena, unit, goal),
            "#{} never arrived: {track:?}",
            unit.0
        );
    }
}

// ---------------------------------------------------------------------------
// 17b cut M4.5a — the routeless steer-on (`tests/extensions.rs`' STOP H)
// ---------------------------------------------------------------------------
//
// The X.6/E.3/M.2/AE.1 fixtures of `tests/extensions.rs` whose machine steered
// at a goal no route reached, rebuilt here by 17b cut M4.5a (the owner's
// ruling Q13 on STOP H). The reflex machine's `Fallback::Waypoint` kept its
// goal past a failed search and steered on at it routeless; a mind's goal is
// followed under `Fallback::Notice`, which drops the goal and stops the hull
// at every failed search, so the reflex's routelessness — a search that
// FAILED — has no mind path. What the fixtures assert of it is the motor's:
// the routeless steer (`Arena::follow` step 5), the one-cell openness test
// and the look-ahead pre-pass (`Arena::hold_routeless`), which read the motor
// row whatever flies the hull. So each is flown by a scripted mind
// (`support::scripted::ScriptedMind`) on its fixture's own window, unit,
// doctrine and match terms (`MatchConfig::default()`: a ten-second startup),
// with two inputs that are not the originals': the units are checked out of
// the FIELDED catalog (`Catalog::build_fielded`, the AE.1 fixtures' own,
// where E.3 and M.2 used `tests/extensions.rs`' `catalog()` — `build_game`
// with the extension packs; the stock line tank row, the same in both), and
// the hull is UNSEATED, where `tests/extensions.rs`' `arena_over` seated a
// pilot on every unit (a seat that carried no input) —
// with the routelessness made by a search that is DEFERRED instead —
// `max_path_queries` 0, so `MotorTable::may_search` refuses every drive,
// `path_deferred` counts it, the goal is never dropped and the steer runs at
// every `MotorTick` — the device 17b cut M4.4 accepted in
// [`m44a_the_follower_holds_rather_than_drive_at_a_bluff`]. A fixture's
// `path_failures > 0` ("the machine was routeless") is re-pinned as
// `path_deferred > 0` with no search spent; every other claim is the
// original's. The script: `Startup` at think 0 (`RED`'s at 482, landing at
// 732, online at 10,732) and the node the reflex's one-node `Patrol` walked
// to as `Goal(Cell)` at [`M45A_FIRST_ONLINE_THINK`], the first think that
// reads the hull online — landing at 11,732, where the reflex machine set it
// at its tick 10,482: one think and one substep later, the landing lattice.

/// The first think of `rig::RED` that reads its hull online under
/// `MatchConfig::default()`'s ten-second startup: think 0's `Startup` lands
/// at 732 and completes at 10,732; think 10 (10,482) still reads `Starting`,
/// think 11 (11,482) reads `Online`, its goal landing at 11,732.
const M45A_FIRST_ONLINE_THINK: u64 = 11;

/// The fielded catalog the AE.1 fixtures check their units out of — and,
/// here, the E.3 and M.2 rebuilds too (see the section head).
fn m45a_catalog() -> tacsim::definitions::Catalog {
    tacsim::definitions::Catalog::build_fielded(&[], tacsim::NS).expect("the fielded content binds")
}

/// **One `entry` checked out as `match_::start` checks one out**
/// (`tests/extensions.rs`' `unit_of`, `blind` and `under_mind` in one),
/// at the middle of `cell`, facing north: no sight at all (a fixture about
/// motion mixes no contact into its event list) and flown by a mind.
fn m45a_unit(
    catalog: &tacsim::definitions::Catalog,
    entry: &str,
    id: ObjId,
    cell: (u16, u16),
) -> tacsim::arena::state::CustodyUnit {
    let key = mush_core::CanonicalId::new(entry).expect("a canonical entry id");
    let spec = catalog.specs.get(&key).expect("the unit entry");
    let binding = catalog.bindings.get(&key).expect("the unit binding");
    let cells = tacsim::definitions::to_tactical_unit(spec, binding, id).expect("the unit");
    tacsim::arena::state::CustodyUnit {
        id,
        team: Team::Red,
        pose: rig::centre(cell.0, cell.1),
        mobility: tacsim::match_::tracked_rules(&cells),
        sensor_height_cm: tacsim::match_::sensor_height_cm(catalog, &cells),
        sensor: None,
        weapons: tacsim::match_::weapon_summaries(catalog, &cells),
        profile: tacsim::match_::unit_profile(catalog, &cells),
        gunnery: 0,
        controller: tacsim::arena::state::Controller::Mind,
        slot: 0,
        unit: cells,
        seal: tacsim::arena::MindSeal::placeholder(),
    }
    .sealed()
}

/// The same unit with `tests/reflex.rs`' fixture helm (`tests/extensions.rs`'
/// `nimble`): 10 m/s and 180 degrees a second, the mobility profile — and so
/// the row's slope limit — untouched.
fn m45a_nimble(unit: tacsim::arena::state::CustodyUnit) -> tacsim::arena::state::CustodyUnit {
    tacsim::arena::state::CustodyUnit {
        mobility: tacsim::arena::state::TrackedRules {
            max_speed_cms: 1_000,
            turn_rate_mdeg_s: 180_000,
            ..unit.mobility
        },
        ..unit
    }
    .sealed()
}

/// A patrol graph of one node — the reflex's `Patrol` goal on the original
/// window; nothing walks it here (the script names the node), and it is laid
/// so the window is the fixture's own.
fn m45a_one_node(at: (u16, u16)) -> tacsim::arena::state::Waypoints {
    tacsim::arena::state::Waypoints::new(
        vec![tacsim::arena::state::Waypoint {
            id: smol_str::SmolStr::new("far"),
            x: at.0,
            y: at.1,
        }],
        &[],
    )
}

/// **A relief window** (`tests/extensions.rs`' `relief`): `width` by
/// `height` cells of 1,000 cm of ground at the elevation `profile` gives,
/// red's entry zone the west two columns and blue's the east two.
fn m45a_relief(
    width: u16,
    height: u16,
    profile: &dyn Fn(u16, u16) -> i16,
) -> tacsim::arena::state::ArenaDefinition {
    use tacsim::arena::state::{ArenaDefinition, ArenaTerrain, EntryZone, TerrainCell};
    let mut cells = Vec::with_capacity(usize::from(width) * usize::from(height));
    for y in 0..height {
        for x in 0..width {
            cells.push(TerrainCell {
                elev_cm: profile(x, y),
                ..rig::ground()
            });
        }
    }
    ArenaDefinition {
        id: smol_str::SmolStr::new("tacsim.arena.relief"),
        terrain: ArenaTerrain::new(width, height, rig::CELL_CM, cells).expect("the relief window"),
        entry_zones: vec![
            EntryZone {
                team: Team::Red,
                x0: 0,
                y0: 0,
                x1: 1,
                y1: height - 1,
            },
            EntryZone {
                team: Team::Blue,
                x0: width - 2,
                y0: 0,
                x1: width - 1,
                y1: height - 1,
            },
        ],
        key_spots: Vec::new(),
        waypoints: tacsim::arena::state::Waypoints::default(),
        modes: Vec::new(),
    }
}

/// **The sealed plateau** (`tests/extensions.rs`' `plateau(false)`): 48 by 8,
/// a plateau 800 cm up behind an unbroken cliff line at column 20 that no
/// limited unit climbs from the low ground in any direction, its one node at
/// (40, 2) behind the cliff.
fn m45a_sealed_plateau() -> tacsim::arena::state::ArenaDefinition {
    let mut definition = m45a_relief(48, 8, &|x, _| if x < 20 { 0 } else { 800 });
    definition.waypoints = m45a_one_node((40, 2));
    definition
}

/// One painted box of [`m45a_priced_window`]: `(x0, x1, y0, y1)` inclusive,
/// and its class (`tests/extensions.rs`' `PricedBox`).
type M45aBox = ((u16, u16, u16, u16), mush_atlas::TerrainClass);

/// **A level window priced per row** (`tests/extensions.rs`'
/// `priced_window`): 48 by 8 of grass, each box of `paint` its class, through
/// `ArenaTerrain::priced` and the shipped table, its one node at (40, 3).
fn m45a_priced_window(paint: &[M45aBox]) -> tacsim::arena::state::ArenaDefinition {
    let (width, height) = (48_u16, 8_u16);
    let table = mush_atlas::physics::LocomotionTable::shipped();
    let mut cells = Vec::with_capacity(usize::from(width) * usize::from(height));
    for y in 0..height {
        for x in 0..width {
            let class = paint
                .iter()
                .rev()
                .find(|((x0, x1, y0, y1), _)| (*x0..=*x1).contains(&x) && (*y0..=*y1).contains(&y))
                .map_or(mush_atlas::TerrainClass::Grass, |(_, class)| *class);
            cells.push(tacsim::arena::state::TerrainCell {
                class,
                elev_cm: 0,
                move_cost: table
                    .cost(class.label(), "tracked", &[])
                    .map_or(0, |cost| u16::try_from(cost).unwrap_or(u16::MAX)),
            });
        }
    }
    let mut definition = m45a_relief(width, height, &|_, _| 0);
    definition.terrain = tacsim::arena::state::ArenaTerrain::priced(
        width,
        height,
        rig::CELL_CM,
        cells,
        mush_atlas::physics::LocomotionTable::shipped(),
    )
    .expect("the priced window");
    definition.waypoints = m45a_one_node((40, 3));
    definition
}

/// **The arena a routeless fixture runs on**: `units` over `definition` on
/// `cfg`'s doctrine with `max_path_queries` 0 (every search deferred), the
/// fixture's own match terms (`MatchConfig::default()`, the ten-second
/// startup) and the catalog's resolution, nobody seated, `RED` — the one
/// hull on the field, so the arena-wide runner swapped for the script flies
/// nothing else — flown by the script: `Startup`, then `Goal(Cell node)` at
/// [`M45A_FIRST_ONLINE_THINK`].
fn m45a_routeless(
    catalog: &tacsim::definitions::Catalog,
    unit: tacsim::arena::state::CustodyUnit,
    definition: tacsim::arena::state::ArenaDefinition,
    cfg: &tacsim::config::ArenaConfig,
    node: (u16, u16),
) -> Arena {
    let cfg = tacsim::config::ArenaConfig {
        reflex: tacsim::config::ReflexConfig {
            max_path_queries: 0,
            ..cfg.reflex.clone()
        },
        ..cfg.clone()
    };
    let mut arena = Arena::new(
        tacsim::arena::state::CustodyOut {
            definition,
            units: vec![unit],
            pilots: Vec::new(),
            resolution: tacsim::match_::resolution_for(catalog),
            seed: support::SEED,
            mode: tacsim::arena::MatchMode::Skirmish,
        },
        &cfg,
        &tacsim::config::MatchConfig::default(),
        &tacsim::config::SensorsConfig::default(),
    )
    .expect("the routeless arena");
    arena.mind_runner = m44a_script(
        rig::RED,
        &[(M45A_FIRST_ONLINE_THINK, vec![m44a_goal(node.0, node.1)])],
    );
    arena
}

/// Step `arena` to `until_ms` in pulses of 500 ms — the last one cut at
/// `until_ms` — settled between pulses as the task's pool settles (17b cut
/// M4.1's gate), absorbing the ledger and the counters and calling `each`
/// after every pulse.
fn m45a_run(
    arena: &mut Arena,
    until_ms: i64,
    bytes: &mut Vec<u8>,
    each: &mut dyn FnMut(&Arena),
) -> MotorCounters {
    let mut counters = MotorCounters::default();
    let mut wall = arena.now_ms;
    if wall == 0 {
        let out = arena.step_settled(0, &[], &[]);
        rig::absorb(&out, bytes);
        counters.absorb(&out.motor);
    }
    while arena.now_ms < until_ms {
        wall = (wall + 500).min(until_ms);
        let out = arena.step_settled(wall, &[], &[]);
        rig::absorb(&out, bytes);
        counters.absorb(&out.motor);
        each(arena);
        if out.ended.is_some() {
            break;
        }
    }
    counters
}

/// Every `blocked` line naming `unit`, as `(x, y, cause)`.
fn m45a_blocked(
    events: &[tacsim::arena::ledger::LedgerEvent],
    unit: ObjId,
) -> Vec<(u16, u16, BlockedCause)> {
    events
        .iter()
        .filter_map(|event| match &event.kind {
            tacsim::arena::ledger::EventKind::Blocked {
                unit: at,
                x,
                y,
                cause,
            } if *at == unit => Some((*x, *y, *cause)),
            _ => None,
        })
        .collect()
}

/// A unit's cell and the elevation it stands at.
fn m45a_standing(arena: &Arena, unit: ObjId) -> ((u16, u16), i32) {
    let state = arena.unit(unit).expect("the unit");
    let cell = arena
        .definition
        .terrain
        .cell_of(&state.pose)
        .expect("on the window");
    (cell, state.pose.z_cm)
}

/// The sealed plateau's routeless line tank (E.3, M.2): the stock line tank
/// at (2, 4), its goal the one node at (40, 2) behind the cliff, on the
/// shipped doctrine with every search deferred.
fn m45a_plateau_arena() -> Arena {
    let catalog = m45a_catalog();
    m45a_routeless(
        &catalog,
        m45a_unit(
            &catalog,
            tacsim::definitions::content::LINE_TANK,
            rig::RED,
            (2, 4),
        ),
        m45a_sealed_plateau(),
        &tacsim::config::ArenaConfig::default(),
        (40, 2),
    )
}

/// **A routeless machine no longer drives at a bluff between its decide
/// ticks** (16b cut E.3; 16c `B-44`; `tests/extensions.rs`'
/// `e3_a_routeless_machine_is_held_off_the_cliff_every_substep`, rebuilt
/// here by 17b cut M4.5a under Q13 — see the section head).
///
/// 16b cut Q.7 put the crossing's own question into `follow`: a hull steering
/// at a goal with no route is steering at a *direction* nobody has checked
/// the ground along, so the one cell it would cross first is tested, and a
/// hull that would be refused it is ordered to stop. `follow` runs at the
/// motor's tick, and motion four times a second, so the pre-pass
/// (`Arena::hold_routeless`) asks every substep — and, since 16b cut M.2, as
/// far ahead as the tank needs to stop. The fixture is
/// `x6_a_refused_route_is_bounded_work`'s worst case: the sealed plateau,
/// the one node behind the cliff, a limited tank with nowhere else to be,
/// steering at the seam for the whole run, refused nothing at all (16c's
/// `B-44` row counted five `blocked` lines here) and never up onto the
/// plateau. M.2's correction stands: what holds this tank is the look-ahead,
/// and without it the refusal at (20, 3) falls inside the window.
///
/// **What moved.** The routelessness is a deferred search (`path_deferred`
/// counted, no search spent — asserted as the premise) where the reflex's
/// search failed and its fallback steered on; the goal lands at 11,732
/// where the reflex machine set it at 10,482 — the landing lattice's one
/// think and one substep. What was measured: the ACCELERATION is the
/// reflex's shifted by that 1,250 ms (full speed at 26,750 where the
/// reflex's was at 25,500, the hold from the half second after each); the
/// braking then runs on the tick grid the mind's motor shares with the
/// reflex's and ends 19 cm further east (M.2's run: at rest at x 19,867
/// against 19,848). So the window is the original's 30,000 ms grown by the
/// same 1,250 to 31,250, and it still covers the approach — measured once,
/// with the look-ahead (`hold_routeless`' `line_is_open` conjunct)
/// disabled in a throwaway local build that was never committed: this test
/// went red with one `blocked` line at (20, 3) at 31,000 (the reflex's at
/// 29,500), inside the window by one substep; at 30,000 it would not have.
/// The window checks its own coverage rather than resting on that
/// measurement: the tank reaches full speed inside it, comes east of
/// 14,500 (inside its 56 m stopping distance of the seam) before it
/// closes, and is last flat out (27,000, at x 14,208) close enough that a
/// hull held at full speed from there reaches the seam (x 20,583) by
/// 31,250 — a later approach fails these loudly instead of passing
/// vacuously. "The controller ran at all" (`ticks > 0`) is read as
/// the mind's thinks and landings (the owner's ruling Q8), the goal still
/// held at the end.
#[test]
fn m45a_e3_a_routeless_machine_is_held_off_the_cliff_every_substep() {
    const UNTIL: i64 = 31_250;
    // The cliff line: column 20's west edge.
    const SEAM_CM: i64 = 20 * 1_000;
    let mut arena = m45a_plateau_arena();
    let top = arena.unit(rig::RED).map_or(0, |unit| {
        i32::try_from(unit.mobility.max_speed_cms).unwrap_or(0)
    });
    let mut bytes: Vec<u8> = Vec::new();
    // The window's own coverage, sampled after every pulse: the fastest the
    // tank went, the furthest east it got, and the last sample at full speed.
    let mut fastest = 0_i32;
    let mut eastmost = i64::MIN;
    let mut last_flat_out: Option<(i64, i64)> = None;
    let mut trace: Vec<(i64, i64, i32)> = Vec::new();
    let counters = m45a_run(&mut arena, UNTIL, &mut bytes, &mut |arena| {
        if let Some(unit) = arena.unit(rig::RED) {
            fastest = fastest.max(unit.speed_cms);
            eastmost = eastmost.max(unit.pose.x_cm);
            if unit.speed_cms == top {
                last_flat_out = Some((arena.now_ms, unit.pose.x_cm));
            }
            trace.push((arena.now_ms, unit.pose.x_cm, unit.speed_cms));
        }
    });
    assert_eq!(arena.now_ms, UNTIL, "the run ended short of its window");
    // The window covers the approach the look-ahead must hold — checked here,
    // not inferred: the tank reached full speed inside it, came inside its
    // stopping distance of the seam (56 m from 15 m/s at 2 m/s², so east of
    // 14,500 — five and a half cells short of the seam — is inside it), and
    // was at full speed late enough and far enough east that a hull held
    // flat out from there would have reached the seam before the window
    // closed, which is where the refusal falls without the look-ahead.
    assert!(
        top > 0 && fastest == top,
        "the window closed before the tank reached full speed ({fastest} of {top}): {trace:?}"
    );
    assert!(
        eastmost >= 14_500,
        "the window closed before the tank came within its stopping distance of the seam (east-most x {eastmost}): {trace:?}"
    );
    let Some((flat_out_ms, flat_out_x)) = last_flat_out else {
        panic!("never at full speed: {trace:?}");
    };
    assert!(
        flat_out_x + i64::from(top) * (UNTIL - flat_out_ms) / 1_000 >= SEAM_CM,
        "held flat out from x {flat_out_x} at {flat_out_ms}, the tank would not reach the seam by {UNTIL}: the window no longer covers the approach: {trace:?}"
    );
    let log = rig::events(&mut arena, &mut bytes);
    let stops = m45a_blocked(&log, rig::RED);
    println!(
        "m45a_e3_routeless: blocked={} intents={} thinks={} landings={} path_deferred={} fastest={fastest} eastmost={eastmost} flat_out=({flat_out_ms}, {flat_out_x})",
        stops.len(),
        counters.intents,
        counters.thinks,
        counters.landings,
        counters.path_deferred
    );
    assert!(
        counters.path_queries == 0 && counters.path_deferred > 0,
        "the premise: every drive routeless, its search deferred: {counters:?}"
    );
    assert!(
        stops.is_empty(),
        "a routeless machine was refused a cell it should never have driven at: {stops:?}"
    );
    // The tank never got up onto the plateau either, which is the other half
    // of the claim: it was held off the seam, not carried over it.
    let (cell, z) = m45a_standing(&arena, rig::RED);
    assert!(
        cell.0 < 20 && z == 0,
        "the tank got onto a plateau nothing can climb: {cell:?} at {z} cm"
    );
    // What is asserted is that the machine ran at all, so that
    // `stops.is_empty()` is a fact about a hull that was being driven and not
    // about one that never moved.
    assert!(
        counters.thinks > 0 && counters.landings > 0,
        "the mind never ran, so nothing was measured: {counters:?}"
    );
    assert_eq!(
        rig::motor(&arena, rig::RED).goal,
        Some((40, 2)),
        "and it was still steering at the node"
    );
}

/// **A routeless machine at full speed stops short of the seam it steers
/// at** (16b cut M.2, reopening E.3's `B-44`; `tests/extensions.rs`'
/// `m2_a_routeless_machine_stops_short_of_the_seam_it_is_steering_at`,
/// rebuilt here by 17b cut M4.5a under Q13 — see the section head).
///
/// The E.3 fixture run on to a standstill. The tank reaches fifteen metres
/// a second on open ground steering at a goal behind the cliff; braking from
/// there at two metres a second squared takes fifty-six metres, five and a
/// half cells, so a pre-pass that asked only the cell in front ordered the
/// stop with the seam already inside the stopping distance and the tank was
/// refused at `(20, 3)` anyway. Looking as far ahead as it needs to stop, it
/// is held early, brakes on its own profile, and comes to rest in the last
/// cell before the cliff with no `blocked` line — and stays there, because
/// the next tick orders it on only while the cell in front is open.
///
/// **What moved.** This was STOP H: under `Fallback::Notice` a mind's hull
/// is stopped at every failed search, four seconds apart, and never reaches
/// 1,500 cm/s; with the search deferred instead it steers on as the reflex's
/// fallback did, and every claim is the original's (full speed on the
/// approach, no `blocked` line, at rest in column 19 at ground level). The
/// acceleration is the reflex's shifted by the landing lattice's 1,250 ms
/// (measured: flat out at 26,750 and held in the half second after — the
/// reflex's 25,500 and the half second after); the braking then runs on the
/// tick grid the mind's motor shares with the reflex's and ends 19 cm
/// further east (at rest by the 35,000 sample at x 19,867 — the reflex's by
/// 34,000 at 19,848), inside the original's 45-second window. Without the
/// look-ahead this is red: the refusal at `(20, 3)` at 31,000 (the reflex's
/// at 29,500; measured once in a throwaway local build with
/// `hold_routeless`' `line_is_open` conjunct disabled, never committed).
#[test]
fn m45a_m2_a_routeless_machine_stops_short_of_the_seam_it_is_steering_at() {
    let mut arena = m45a_plateau_arena();
    let mut bytes: Vec<u8> = Vec::new();
    // Forty-five seconds: the tank is on the move from twelve, flat out by
    // twenty-seven, and at rest by thirty-five.
    let mut fastest = 0_i32;
    let counters = m45a_run(&mut arena, 45_000, &mut bytes, &mut |arena| {
        fastest = fastest.max(arena.unit(rig::RED).map_or(0, |unit| unit.speed_cms));
    });
    assert_eq!(arena.now_ms, 45_000, "the run ended short of its window");
    assert!(
        counters.path_queries == 0 && counters.path_deferred > 0,
        "the premise: every drive routeless, its search deferred: {counters:?}"
    );
    let top = arena
        .unit(rig::RED)
        .map_or(0, |unit| unit.mobility.max_speed_cms);
    assert_eq!(
        u32::try_from(fastest).unwrap_or(0),
        top,
        "the fixture needs the tank at full speed on the approach"
    );
    let log = rig::events(&mut arena, &mut bytes);
    let stops = m45a_blocked(&log, rig::RED);
    assert!(
        stops.is_empty(),
        "the tank outran its brakes into the seam: {stops:?}"
    );
    let (cell, z) = m45a_standing(&arena, rig::RED);
    assert_eq!(
        (cell.0, z),
        (19, 0),
        "the tank stopped in the last cell before the cliff: {cell:?} at {z} cm"
    );
    assert_eq!(
        arena.unit(rig::RED).map(|unit| unit.speed_cms),
        Some(0),
        "and it is standing there"
    );
}

/// **A routeless machine is held at the Bluff by the one-cell test, on the
/// shipped ground** (16b cut AE.1; 16c `T-18`; the routeless leg of
/// `tests/extensions.rs`'
/// `ae1_the_bluff_refuses_a_tank_and_a_car_and_admits_a_walker_on_the_shipped_ground`,
/// rebuilt here by 17b cut M4.5a under Q13 — see the section head; the
/// pilot-driven legs and the searches stay there).
///
/// A routeless line tank steered at the node on top of the Bluff, (144,
/// 111), from (130, 111) on the proving ground's west face, is held by
/// `first_step_is_open` — 16b cut Q.7's cliff test: slope refusals counted,
/// no `blocked` line, never up on the 500 cm top, and an empty avoid list —
/// `note_blocked` records nothing for a slope, so the openness test is the
/// whole of the hull's cliff memory. **What moved**: the reflex fixture made
/// the machine routeless with a search that could not finish
/// (`path_max_cells` 1) and asserted `path_failures > 0`; a mind's failed
/// search would drop the goal (`Fallback::Notice`), so the search is held
/// deferred (`max_path_queries` 0) and the premise is `path_deferred > 0`
/// with no search spent. Every other claim is the original's, over its 60
/// seconds.
#[test]
fn m45a_ae1_a_routeless_machine_is_held_at_the_bluff_by_the_one_cell_test() {
    let world = booted_world();
    let mut shipped = tacsim::match_::definition_for(
        world.world(),
        &smol_str::SmolStr::new(tacsim::match_::PROVING_GROUND),
    )
    .expect("the proving ground's ground builds")
    .expect("the proving ground is shipped");
    assert_eq!(
        (
            shipped.terrain.get(141, 111).map(|cell| cell.elev_cm),
            shipped.terrain.get(140, 111).map(|cell| cell.elev_cm)
        ),
        (Some(500), Some(-200)),
        "the Bluff's west face: a 700 cm rim"
    );
    let goal = (144_u16, 111_u16);
    shipped.waypoints = m45a_one_node(goal);
    let catalog = m45a_catalog();
    let mut arena = m45a_routeless(
        &catalog,
        m45a_unit(
            &catalog,
            tacsim::definitions::content::LINE_TANK,
            rig::RED,
            (130, 111),
        ),
        shipped,
        &tacsim::config::ArenaConfig::default(),
        goal,
    );
    let mut bytes: Vec<u8> = Vec::new();
    let counters = m45a_run(&mut arena, 60_000, &mut bytes, &mut |_| {});
    assert_eq!(arena.now_ms, 60_000, "the run ended short of its window");
    let log = rig::events(&mut arena, &mut bytes);
    assert!(
        counters.path_queries == 0 && counters.path_deferred > 0,
        "the search was never deferred, so the machine was never routeless: {counters:?}"
    );
    assert!(
        counters.slope_refusals > 0,
        "the one-cell test never refused the face"
    );
    assert!(
        m45a_blocked(&log, rig::RED).is_empty(),
        "the routeless machine was refused at the face: {:?}",
        m45a_blocked(&log, rig::RED)
    );
    let (cell, z) = m45a_standing(&arena, rig::RED);
    assert!(
        cell.0 <= 140 && z < 500,
        "the machine got onto the top: {cell:?} at {z} cm"
    );
    assert_eq!(
        arena
            .motors
            .row(rig::RED)
            .map(|motor| motor.avoid.is_empty()),
        Some(true),
        "a slope teaches the avoid list nothing"
    );
}

/// One ledger line of an [`M45aReplay`]: `(t_ms, kind json, draws, hash)`.
type M45aLine = (i64, String, Vec<(String, u64)>, u64);

/// One hull's end in an [`M45aReplay`]: `(unit, x, y, z, grade, heading,
/// speed, blocked)`.
type M45aFinal = (ObjId, i64, i64, i32, u16, u32, i32, bool);

/// What one [`m45a_replay`] produced: the ledger lines, the per-substep
/// hashes, the finals, the counters, where `RED` ended, whether its avoid
/// list is empty, and the arena clock it stopped at (a replay that ended
/// early is caught by its reader).
struct M45aReplay {
    events: Vec<M45aLine>,
    hashes: Vec<u64>,
    finals: Vec<M45aFinal>,
    counters: MotorCounters,
    cell: (u16, u16),
    avoid_empty: bool,
    now_ms: i64,
}

/// `arena` replayed at `cadence_ms` to `until_ms`, the tempo pinned quiet and
/// settled between pulses (`tests/extensions.rs`' `relief_replay` with no
/// script).
fn m45a_replay(mut arena: Arena, cadence_ms: i64, until_ms: i64) -> M45aReplay {
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut counters = MotorCounters::default();
    let mut wall = 0_i64;
    loop {
        arena
            .clock
            .pin_tempo_for_tests(tacsim::arena::clock::Tempo::Quiet);
        let out = arena.step_settled(wall, &[], &[]);
        rig::absorb(&out, &mut bytes);
        counters.absorb(&out.motor);
        hashes.extend(arena.substep_hashes.iter().copied());
        if arena.now_ms >= until_ms || out.ended.is_some() {
            break;
        }
        wall += cadence_ms;
    }
    let events = rig::events(&mut arena, &mut bytes)
        .into_iter()
        .map(|event| {
            (
                event.t_ms,
                serde_json::to_string(&event.kind).unwrap_or_default(),
                event.draws,
                event.hash,
            )
        })
        .collect();
    let finals = arena
        .units
        .iter()
        .map(|unit| {
            (
                unit.id,
                unit.pose.x_cm,
                unit.pose.y_cm,
                unit.pose.z_cm,
                unit.grade_pct,
                unit.pose.heading_mdeg,
                unit.speed_cms,
                unit.blocked,
            )
        })
        .collect();
    M45aReplay {
        events,
        hashes,
        finals,
        counters,
        cell: m45a_standing(&arena, rig::RED).0,
        avoid_empty: arena
            .motors
            .row(rig::RED)
            .is_some_and(|motor| motor.avoid.is_empty()),
        now_ms: arena.now_ms,
    }
}

/// **A routeless walker walks through the woods and a routeless car is held
/// short of the rubble, each on its own row, at every cadence** (16b cut
/// AE.1; 16c `B-58`; 16b §11's cross-tempo property; the two routeless
/// scenes of `tests/extensions.rs`'
/// `ae1_a_machine_walker_and_a_machine_car_steer_on_their_own_rows_at_every_cadence`,
/// rebuilt here by 17b cut M4.5a under Q13 — see the section head; the routed
/// scenes and the search's scale stay there).
///
/// On the level 48-by-8 window priced per row, the nimble machine steered
/// straight at the node at (40, 3) with no route: the walker's one-cell test
/// lets it into a thick-woods band across the whole window (columns 20–27),
/// which a tank's row has no way over, and it walks through them; the car's
/// refuses it a rubble wall across the window, so it is held short of it —
/// no `blocked` line, and an empty avoid list. Each scene is one event list,
/// one per-substep hash sequence and one set of finals at 5,000, 1,000 and
/// 500 ms. And the walker is never held on the way: the look-ahead
/// (`way_ahead_is_open`) asks every cell within its stopping distance on its
/// OWN row, so it slows for the woods once, on the ground's own price, and
/// walks through them at `1,000 × 100 / 200` — asked on a tank's row, the
/// woods two cells ahead would hold it, it would brake, be let go, speed up
/// and be held again. **What moved**: the reflex made each machine
/// routeless with a search that could not finish (`path_max_cells` 1) and
/// asserted `path_failures > 0`; a mind's failed search would drop the goal
/// (`Fallback::Notice`), so the search is held deferred (`max_path_queries`
/// 0) and the premise is `path_deferred > 0` with no search spent. Every
/// other claim is the original's, over its 60 seconds; the goal lands one
/// think and one substep after the reflex's tick set it.
#[test]
fn m45a_ae1_a_routeless_walker_and_a_routeless_car_steer_on_their_own_rows_at_every_cadence() {
    use mush_atlas::TerrainClass;
    use tacsim::arena::CostRow;
    use tacsim::definitions::content;
    const UNTIL: i64 = 60_000;

    let catalog = m45a_catalog();
    let start = (2_u16, 3_u16);
    let node = (40_u16, 3_u16);
    let woods: &[M45aBox] = &[((20, 27, 0, 7), TerrainClass::WoodsHeavy)];
    let rubble_wall: &[M45aBox] = &[((20, 27, 0, 7), TerrainClass::Rubble)];
    let open = |entry: &str, paint: &[M45aBox]| {
        m45a_routeless(
            &catalog,
            m45a_nimble(m45a_unit(&catalog, entry, rig::RED, start)),
            m45a_priced_window(paint),
            &tacsim::config::ArenaConfig::default(),
            node,
        )
    };

    // One scene at three cadences: the same history, and where it ended.
    let three = |entry: &str, paint: &[M45aBox], row: CostRow| {
        let runs: Vec<(i64, M45aReplay)> = [5_000_i64, 1_000, 500]
            .into_iter()
            .map(|cadence| {
                let arena = open(entry, paint);
                assert_eq!(arena.unit(rig::RED).map(|unit| unit.cost_row), Some(row));
                (cadence, m45a_replay(arena, cadence, UNTIL))
            })
            .collect();
        for (cadence, run) in &runs {
            assert_eq!(run.now_ms, UNTIL, "{entry}: cadence {cadence} ran short");
        }
        let Some((_, first)) = runs.first() else {
            panic!("three replays");
        };
        for (cadence, run) in runs.iter().skip(1) {
            assert_eq!(
                run.events, first.events,
                "{entry}: the event list differs at {cadence} ms"
            );
            assert_eq!(
                run.hashes,
                first.hashes,
                "{entry}: the per-substep hash sequence differs at {cadence} ms ({} against {})",
                run.hashes.len(),
                first.hashes.len()
            );
            assert_eq!(
                run.finals, first.finals,
                "{entry}: a final pose, z, grade or speed differs at {cadence} ms"
            );
        }
        let stops = first
            .events
            .iter()
            .filter(|(_, kind, _, _)| kind.starts_with("{\"kind\":\"blocked\""))
            .count();
        (first.counters, stops, first.cell, first.avoid_empty)
    };

    // The walker's one-cell test lets it into the woods.
    let (counters, stops, cell, _) = three(content::BIPED_MK1, woods, CostRow::Legged);
    assert!(
        counters.path_queries == 0 && counters.path_deferred > 0,
        "the walker was never routeless: {counters:?}"
    );
    assert_eq!(stops, 0, "the routeless walker was stopped");
    assert!(
        cell.0 >= 28,
        "the routeless walker was held at the woods: {cell:?}"
    );
    // And it is never held on the way.
    let speeds_in = |columns: std::ops::RangeInclusive<u16>| {
        let mut arena = open(content::BIPED_MK1, woods);
        let mut bytes: Vec<u8> = Vec::new();
        let mut seen: Vec<i32> = Vec::new();
        let _ = m45a_run(&mut arena, UNTIL, &mut bytes, &mut |arena| {
            if columns.contains(&m45a_standing(arena, rig::RED).0.0) {
                seen.extend(arena.unit(rig::RED).map(|unit| unit.speed_cms));
            }
        });
        assert_eq!(
            arena.now_ms, UNTIL,
            "the walker's run over {columns:?} ended short"
        );
        seen
    };
    let near = speeds_in(14..=19);
    let deep = speeds_in(22..=26);
    assert!(
        !near.is_empty() && !deep.is_empty(),
        "the routeless walker never came up to the woods and through them"
    );
    assert!(
        near.windows(2)
            .all(|pair| pair.get(1).copied() <= pair.first().copied()),
        "the routeless walker was held short of the woods and let go: {near:?}"
    );
    assert!(
        deep.iter().all(|speed| *speed == 1_000 * 100 / 200),
        "the routeless walker was held in the woods: {deep:?}"
    );

    // The car's refuses it the rubble, and it is held short of it.
    let (counters, stops, cell, avoid_empty) =
        three(content::SCOUT_MK1, rubble_wall, CostRow::Wheeled);
    assert!(
        counters.path_queries == 0 && counters.path_deferred > 0,
        "the car was never routeless: {counters:?}"
    );
    assert_eq!(stops, 0, "the routeless car drove into the rubble");
    assert!(
        cell.0 > start.0 && cell.0 < 20,
        "the routeless car is not held short of the rubble: {cell:?}"
    );
    assert!(avoid_empty, "a held car learnt a stop it never had");
}
