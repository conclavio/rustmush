//! Pilot and mind frame types and construction (`Frame`, `FrameSection`, `FrameOdds`, etc.).

use super::*;

/// **One pilot's latest-wins view** (doc 16 §3.1's `Frames`).
///
/// The minimal slot cut A can honestly fill: where the tank is, how fast it is
/// going, whether its power is up, whether it is held against something, and the
/// tempo the arena is running at. Cut K builds the real cockpit keyframe and delta
/// on exactly this seam, and cut S is what makes a frame *per observer* rather
/// than per unit.
///
/// It is not `PartialEq`: [`Frame::same_as`] compares the fields that decide
/// whether a pilot is told anything, and deliberately not the pose's opaque
/// posture — a frame is a view, and comparing an opaque rules blob every pulse for
/// every pilot would be the most expensive thing in phase 5.
#[derive(Clone, Debug)]
pub struct Frame {
    /// The tank the pilot is in.
    pub unit: ObjId,
    /// **What everybody who has told this tank apart calls it** (16b cut Q.1):
    /// its own team letter and centre slot. `STATUS` opens with it, so a pilot
    /// can answer "who is `RA`?" about themselves.
    pub callsign: crate::arena::state::Designator,
    /// **The arena cell the tank is standing in** (16b cut Q.1), or `None` for a
    /// pose off the window.
    ///
    /// The window's own resolution and no finer, exactly as a contact's `cell`
    /// is: it is what a panel's map draws the pilot's own marker at without
    /// having to divide a pose by a cell size it would have to be told.
    pub cell: Option<(u16, u16)>,
    /// The public match mode. On the wire as `"mode"`; 16c `Q-PT1` has the
    /// seat print `RANGE` off it (`crate::match_::cockpit::on_range`), so a
    /// range is never read as a fight.
    pub mode: crate::arena::state::MatchMode,
    /// Last accepted hit on this own hull, in arena time.
    pub own_last_hit_ms: Option<i64>,
    /// Public Hold geometry, excluding ownership and capture clocks.
    pub objective: Option<crate::arena::state::HoldObjective>,
    /// This hull's requested motor goal and latest outcome.
    pub motor: Option<FrameMotor>,
    /// **The side it is on** (16b cut S.2), so the cockpit can draw a classified
    /// contact as a friend or as a hostile without asking the world which team the
    /// pilot's slot was on. The platform's own side is never a secret from its
    /// pilot.
    pub team: Team,
    /// **Its own unit type** (doc 22): its definition id, which names the look
    /// its pilot's doll is drawn from. Never a secret from its pilot.
    pub unit_type: SmolStr,
    /// **How high its eye sits above its cell, centimetres** (doc 21 cut G.7b):
    /// the height its own sight is traced from, so the pilot's picture is drawn
    /// from the same eye — a biped's nine metres, a tank's one and a half.
    pub sensor_height_cm: u32,
    /// Where it is.
    pub pose: Pose,
    /// How fast, cm/s; negative is reverse.
    ///
    /// **16b cut E.3 decided what this reads while a tank is held against another
    /// unit** ([`crate::arena::state::UnitState::occupied_hold`]): the speed it
    /// had. A hold keeps `speed_cms` and does not set
    /// [`crate::arena::state::UnitState::held`], so for up to
    /// `[tacsim.arena].occupied_stop_ms` the pilot carries the speed they ordered,
    /// a pose that is not advancing, and no `HELD` — and then, at the stop, `0`
    /// and `HELD` together. That is deliberate. Reporting `0` while holding would
    /// make the speed of every tank following another in column flicker to zero
    /// for a substep at a time, and it would disclose nothing a pilot is not
    /// already told sooner: the stop's own `Held: something is in the way.` is a
    /// sentence only [`crate::arena::motion::BlockedCause::Occupied`] produces,
    /// and it arrives no later than it ever did plus the hold.
    pub speed_cms: i32,
    /// Whether its power is up.
    pub power: Power,
    /// Whether it is held against an obstacle, an occupied cell or the window's
    /// edge.
    ///
    /// **16b cut F.2** re-sourced it from [`crate::arena::state::UnitState::held`]
    /// — the *condition* — rather than from `blocked`, the flag for the substep
    /// just integrated: a tank ordered forward against a river bank covers no
    /// whole centimetre on some substeps and read `0 kph` with no `HELD` on those.
    ///
    /// **16b cut M.3** (16c `T-04`) renamed the Rust field from `blocked`, which
    /// named the flag it no longer read, to the condition it does read:
    /// `unit.held.is_some()`. Only the presence goes out — the held cell and its
    /// cause stay in the arena, the cause having already reached the pilot as the
    /// `Held:` sentence of the stop. The **wire key** `status_frame` writes is
    /// still `"blocked"`, because both panels read that key and renaming it is a
    /// client cut's (`crate::match_::cockpit::status_frame`).
    pub held: bool,
    /// **The tank is a wreck** (16b cut D.2, `B-03`; doc 16 §3.3 "A wreck": *no
    /// longer the subject of any order*).
    ///
    /// The pilot's own tank, which is physical knowledge and outside the observer
    /// rule entirely — the same kind of fact [`Frame::sections`] and
    /// [`Frame::heat`] are. It is what lets the cockpit refuse an order at once
    /// (`crate::match_::cockpit::CockpitError::Destroyed`) instead of a pulse
    /// later, which is the rule every other refusal on this surface follows.
    pub wreck: bool,
    /// **The tank cannot move** (16b cut D.2, `B-03`):
    /// [`crate::arena::state::MotionLimits::motion_disabled`], a destroyed engine or
    /// a biped's second leg.
    ///
    /// The pilot's own tank, for the reason [`Frame::wreck`] is.
    /// `crate::match_::cockpit::CockpitError::Immobile` reads it, and prints the
    /// sentence `crate::match_::cockpit::refusal_line` already says for
    /// [`crate::arena::intents::Refusal::MotionDisabled`].
    pub immobile: bool,
    /// The arena's tempo, so a cockpit can say how often it is being told things.
    pub tempo: Tempo,
    /// **How far the platform's own sight reaches**, centimetres (16b cut D.1;
    /// doc 16 §3.6), or `0` for a platform that carries no sensor at all.
    ///
    /// The **nominal** reach, and deliberately so: it is
    /// `SensorProfile::max_range_cells × cell_cm`, the same product
    /// `Arena::detections` calls `max_range_cm` *before* the target's signature,
    /// the thermal channel and any jamming shorten it. The reach a real detection
    /// gets is that figure scaled; this is the size of the scope and not what the
    /// scope can resolve this second, and it must not be scaled here — a jammed
    /// reach on the frame is its own piece of work.
    ///
    /// It is a fact about the pilot's own machine — the same kind of thing
    /// [`Frame::sections`] and [`Frame::heat`] are — so it is outside the observer
    /// rule entirely. [`crate::match_::cockpit::contact_band`] is its reader on
    /// this surface: the band word is thirds of the lesser of this and the fired
    /// weapon's last band. **16b cut E.1** made the product itself
    /// [`crate::arena::state::UnitState::sight_cm`], which the reflex
    /// controller's own fighting reach is priced off as well — one number, so a
    /// machine closes to a band its pilot's `CONTACTS` could name.
    pub sight_cm: u32,
    /// **What the platform holds** (16b cut S.1; doc 16 §3.6 "What a pilot sees"),
    /// in contact-id order — the same order [`crate::arena::state::ShadowContacts`]
    /// iterates in, so two pulses that hold the same set produce the same list.
    ///
    /// It is the platform's contacts and not the observer's: doc 16 §3.7 puts the
    /// station on the tank, so two pilots in one tank see one scope. Cut S.2
    /// renders it; nothing about an undetected unit is in it, which is the whole
    /// observer rule on this surface.
    pub contacts: Vec<FrameContact>,
    /// **The section vocabularies the readings on [`Frame::contacts`] are in**
    /// (16b cut Q.2; doc 16 §3.6), one entry per distinct section-id list, indexed
    /// by [`crate::arena::perception::FrameCondition::table`].
    ///
    /// Built from the contacts **whose `condition` is `Some`** and from no
    /// others. Not from the classified ones: a contact classified over the mesh
    /// that nobody has had a close look at carries no reading, and publishing its
    /// chassis would hand the pilot the one fact the close look is the price of.
    /// A frame whose scope holds no reading carries an empty table list.
    ///
    /// A target's section ids are read off the live unit as a **static fact about
    /// the machine** — the same declared exception the silhouette is — because the
    /// vocabulary is what the bands are in the order of and it does not move while
    /// a match runs.
    pub scan_tables: Vec<Vec<SmolStr>>,
    /// **What the pilot is told once** (16b cut S.2; doc 16 §3.6: *notices fire
    /// once per transition with hysteresis*), in the order the transitions
    /// happened.
    ///
    /// Deliberately outside [`Frame::same_as`]: a frame is sent when its numbers
    /// changed *or* when it carries a notice, and the copy remembered in
    /// `last_frames` has this emptied, so a notice is delivered exactly once and a
    /// later `CONTACTS` re-rendering the remembered frame does not repeat it.
    pub notices: Vec<FrameNotice>,
    /// **The contact the platform is locked on** (16b cut W.1; doc 16 §5's status
    /// block: *current lock*), by its row key, and always one of
    /// [`Frame::contacts`].
    ///
    /// **16b cut Q.1**: the key is not a pilot's handle any more. A pilot names a
    /// contact by the short designator its row carries
    /// ([`crate::arena::perception::FrameContact::designator`]), and a lock taken
    /// under a provisional number survives the rename precisely because what the
    /// arena holds is this id and not the word that was typed.
    pub lock: Option<ContactId>,
    /// **What the platform carries** (16b cut W.1; doc 16 §5: *ammunition and ready
    /// state per weapon*), in component order, with the lock's band where there is a
    /// lock. It is what `WEAPONSTATUS` prints while a match runs.
    pub weapons: Vec<FrameWeapon>,
    /// Every active native rule in stable slot order, including Sensors rules.
    pub rules: Vec<crate::arena::weapons::FrameRule>,
    /// **What the tank has left** (16b cut F.2; doc 16 §5's status block: *armor
    /// and internals per section as current over max*), in **section-id order**:
    /// [`mush_tactics::TacticalUnit`] holds its sections and their facets in
    /// ordered maps, so a stock tank reads `engine, hull, left_track, right_track,
    /// turret` and not the order its pack authored them in.
    ///
    /// The whole of the recorded open point K7: a pilot shot to pieces had no
    /// surface at all on which to read it. It is a projection of the M32 cells the
    /// impact writes and never a second model — built per pilot per pulse, which is
    /// why a platform nobody is seated on pays nothing for it — and it names only
    /// the pilot's **own** tank, which is physical knowledge and outside the
    /// observer rule entirely.
    pub sections: Vec<FrameSection>,
    /// **What the platform is carrying, and what it may** (16b cut X.1; doc 16 §9
    /// item 1), or `None` for a platform whose content declares no heat — which is
    /// every stock tank, so nothing about a stock cockpit changes.
    ///
    /// It is what lets the cockpit refuse `FIRE` for heat at once instead of a
    /// pulse later, which is the rule every other `FIRE` refusal already follows
    /// (`crate::match_::cockpit`).
    pub heat: Option<FrameHeat>,
    /// **What the platform's own receiver is running under** (16b cut D.4,
    /// `B-22`; doc 16 §9 items 4 and 5), or `None` for a receiver that is clean —
    /// or off.
    ///
    /// [`crate::arena::state::UnitState::receiver`] as its last pass left it. It
    /// is a fact about the pilot's own machine, the same kind of thing
    /// [`Frame::sections`], [`Frame::heat`] and [`Frame::sight_cm`] are, so it is
    /// outside the observer rule entirely — and it carries no source, no bearing,
    /// no side and no object number, which is what keeps it there. See
    /// [`crate::arena::perception::Receiver`] for the two declared limits.
    pub receiver: Option<crate::arena::perception::Receiver>,
    /// **The platform's own jamming pod's switch** (16b cut O.2, 16c N12):
    /// `None` for a platform that carries no pod — every stock tank — and
    /// `Some(`[`crate::arena::state::UnitState::ecm_on`]`)` for one that does.
    ///
    /// The pilot's own machine, like [`Frame::receiver`] beside it, so it is
    /// outside the observer rule; it is what `STATUS`'s `emcon` line, the
    /// panels' scope-row suffix and the cockpit's local `EMCON` refusals read.
    pub ecm: Option<bool>,
    /// **What happened while the pilot could not see** (16b cut Y.4; doc 07 §1.3
    /// step 5), on the first frame after a reclaim and on no other — `None`
    /// everywhere else.
    ///
    /// One-shot, as [`Frame::notices`] is: `Arena::frames` takes it out of
    /// `Arena::pending_catchup` onto the frame and the copy remembered in
    /// `last_frames` has it cleared, and the world's mirror stores it cleared
    /// too, so `STATUS` and `VIEW` never re-say it. [`Frame::same_as`] answers
    /// `false` whenever either side carries one, so a frame that carries it is
    /// always sent. The pilot's own tank, plus the platform's own designators
    /// for the contacts it held or holds — the observer rule, as
    /// [`crate::arena::catchup`] states it.
    pub catchup: Option<crate::arena::catchup::Catchup>,
    /// **The platform's advertised chance per weapon against its lock and its
    /// nearest classified hostiles** (17b cut M1a.3; doc 17 §3, D-UM-5; the
    /// lock-first, hostiles-only rule is 17b cut M1b.3's, Q2) — what
    /// `Arena::frame_odds` builds: at most [`STOCK_ODDS_ROWS`] contacts, a
    /// hostile lock first and then nearest first, each contact's entries led
    /// by the weapon a bare `FIRE` would fire. Empty on a hull with no weapons
    /// station and when no classified hostile is held; never an entry for an
    /// unclassified contact or one of the hull's own side.
    ///
    /// Advice off the plot, as the lock's percentage is
    /// ([`crate::arena::Arena::aim_at`], [`crate::arena::Arena::resolve_seen`]).
    pub odds: Vec<FrameOdds>,
    /// **The standing order the hull is under, re-keyed to its own contacts**
    /// (17b cut M1a.3; doc 17 §3 "Two more fields", D-UM-15), or `None` —
    /// which is every piloted hull: see `Arena::frame_order`.
    pub order: Option<FrameOrder>,
    /// **The arena millisecond the frame was built at** (17b cut M1a.3; doc 17
    /// §3) — `Arena::now_ms` at the build, which inside the due loop is the
    /// substep's end: the state's own clock, the one every age on the frame is
    /// measured from. [`Frame::same_as`] never compares it.
    pub t_ms: i64,
    /// **The match's Compact floor, per mille** (17b cut M1b.3; doc 17 §5 as
    /// amended, D-UM-4, D-UM-17), captured at check-out
    /// ([`Arena::odds_floor_permille`], 17b cut M1b.1): the figure the
    /// cockpit's `FIRE … WHEN odds >= n` pre-check reads (the O.2 `NoJammer`
    /// shape: a local refusal off the held frame, never the live config) and a
    /// mind's `arm` is held to. A per-match constant and a public one, so
    /// [`Frame::same_as`] does not compare it and the observer rule has nothing
    /// to say about it.
    pub odds_floor_permille: u32,
    /// **The hull's sealed mind, as the pilot in the seat reads it** (17b cut
    /// M5.4; doc 17 §12 "The `MIND` verb"), or `None` on a hull no seal
    /// reached ([`crate::arena::mind::SealRef::none`]'s zero digest).
    ///
    /// Built by `Arena::frame_for` off the unit's own
    /// [`crate::arena::mind::SealRef`] (`doctrine`, `profiles`), its
    /// [`crate::arena::state::UnitState::card_profile`] and its mind row —
    /// see [`FrameMind`]. The pilot's own hull, which is physical knowledge
    /// and outside the observer rule, as [`Frame::sections`] is.
    /// [`Frame::same_as`] compares it. **Never copied into
    /// `crate::arena::mind::MindView`** (D-UM-4: a tier truncates the
    /// keyframe, never extends it), so a mind reads nothing of its own seal
    /// here and `crate::minds::api::PARITY` is untouched.
    pub mind: Option<FrameMind>,
    /// **Whether the seat's master gate is at weapons free** (doc 21 cut
    /// G.12a, §7.3): `!`[`crate::arena::state::UnitState::weapons_hold`] —
    /// `true` unless the pilot typed `FIRE HOLD`. The pilot's own switch,
    /// outside the observer rule; the HUD says which and the `f` key flips it.
    /// [`Frame::same_as`] compares it, so a flip is sent at once.
    pub weapons_free: bool,
    /// **The hull's forward speed ceiling, cm/s** (doc 21 cut G.12a), the
    /// figure the arena clamps a `SPEED` to at apply
    /// ([`crate::arena::state::TrackedRules::max_speed_cms`]) and the `SPEED`
    /// refusal names — so a client steps and clamps its speed keys without a
    /// round trip. A fit constant for the match.
    pub max_speed_cms: u32,
    /// **The hull's reverse speed ceiling, cm/s, as a magnitude** (doc 21 cut
    /// G.12a): [`crate::arena::state::TrackedRules::reverse_ceiling_cms`], the
    /// other half of [`Frame::max_speed_cms`].
    pub reverse_cms: u32,
}

/// **The hull's sealed mind on the pilot's frame** (17b cut M5.4; doc 17 §12
/// "The `MIND` verb", §11; the M5 design note's M5.4 item 2): what `MIND`
/// prints and the keyframe's `mind` object carries.
///
/// `doctrine` and `profiles` are the seal's own, per-match constants
/// ([`crate::arena::mind::SealRef::doctrine`],
/// [`crate::arena::mind::SealRef::profiles`]). **Both classes are read LIVE**,
/// never [`crate::arena::mind::SealRef`]'s figures fixed at the seal: `class`
/// is [`Arena::mind_class`] (the sealed class against the core NOW; `Zero`
/// once the row is dropped) and `core_class` the class of
/// [`Arena::core_of`] (`Zero` for a hull with no core) — so a heavy whose core
/// a `degrade_core` has lowered reads the class it now is; only the seal line,
/// said once at check-out, keeps the sealed figures. `active` and
/// `switch_at_ms` are [`crate::arena::state::UnitState::card_profile`]'s:
/// the index flown and the ABSOLUTE arena millisecond the next switch is
/// allowed — the last switch's stamp plus
/// [`crate::arena::intents::PROFILE_SWITCH_MS`], `None` before any switch — so
/// it changes only at a switch, never with the clock. `off` is the mind row's
/// drop to the ROM ([`crate::arena::mind::MindState::dropped`]; `false` with
/// no row).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameMind {
    /// Selected authored display name, retained even while captured ROM runs.
    pub name: SmolStr,
    /// Whether the mind sealed is Standard Doctrine.
    pub doctrine: bool,
    /// The profiles sealed, the first the one that flies at the release.
    pub profiles: crate::arena::runtime_card::RuntimeProfiles,
    /// Complete active own-hull card, sharing the catalog's immutable cache.
    pub card: crate::arena::runtime_card::RuntimeCard,
    /// Complete bound native/think program, distinct from mutable current slots.
    pub program: crate::arena::profile_rules::BoundProgram,
    /// The index into [`FrameMind::profiles`] the mind will fly.
    pub active: u8,
    /// The class the mind thinks at now ([`Arena::mind_class`]).
    pub class: crate::arena::core::CoreClass,
    /// The class of the hull's core now ([`Arena::core_of`]).
    pub core_class: crate::arena::core::CoreClass,
    /// The arena millisecond the next switch is allowed, or `None` before
    /// any switch.
    pub switch_at_ms: Option<i64>,
    /// Whether the mind row is dropped to the ROM.
    pub off: bool,
}

/// **How many contacts a frame carries odds for** (17b cut M1a.3; doc 17 §9,
/// D-UM-35): the stock tank's class I core's `odds_rows` — read in one place,
/// `Arena::frame_odds`. **The Class I row of `ClassTable::STOCK`; production
/// reads the profile, the tests' arithmetic keeps the constant** (17b cut
/// M3.1): a mind's view keeps the odds of the first `odds_rows` of the
/// frame's rows (`Arena::mind_view`). The FRAME itself stays at this width —
/// it is the pilot's picture, and it does not move before the cut that
/// rates a mind above Class I (the owner's OQ-10 (c), re-pointed by 17b cut
/// M5.4) — so a Class II or III mind reads at most these two (D-UM-4: a tier
/// never extends the keyframe).
pub const STOCK_ODDS_ROWS: usize = 2;

/// **One facet's armour, as the pilot's own status block carries it** (16b cut
/// F.2; doc 16 §5: *armor ... per section as current over max*).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameFacet {
    /// The facet's id (`front`, `left`, `right`, `rear`, `top`).
    pub id: SmolStr,
    /// What it has left.
    pub armor: i64,
    /// What it holds when it is whole.
    pub armor_max: i64,
}

/// **One section of the pilot's own tank** (16b cut F.2; doc 16 §5's status
/// block), read off the M32 cells the damage pipeline writes.
///
/// `internals` is `None` for a section whose content declares no internals cell —
/// there is nothing to report rather than a nought that would read as destroyed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameSection {
    /// The section's id (`hull`, `turret`, `left_track`, …).
    pub id: SmolStr,
    /// Its opaque condition, as M32 holds it (`intact`, `destroyed`, `wreck`).
    pub condition: SmolStr,
    /// Its internals as current over max.
    pub internals: Option<(i64, i64)>,
    /// Its facets, in **facet-id order** (the unit's own ordered map's), so the
    /// hull reads `front, left, rear, right, top`.
    pub facets: Vec<FrameFacet>,
}

/// **The status block by one ordered pass** (16c T-110a): `sections`, in
/// section-id order, merged with the unit's cell map read through two cursors
/// over it — `section_run` for the `Section` entries and `facet_run` for the
/// `Facet` entries — in place of one `BTreeMap` probe per section and per
/// facet, each probe an address built from two or three cloned ids.
///
/// Both cursors are the one map's own ordered iterator
/// (`TacticalUnit::cells`), so each is in `CellAddr`'s derived order: every
/// `Unit` entry, then every `Section` entry by section id then cell id, then
/// every `Facet` entry by section id, facet id then cell id, then every
/// `Component` entry. The targets are asked in that same order —
/// `(section, internals)` ascends with `sections()` (a `BTreeMap`), and
/// `(section, facet, armor)` with `sections()` then each section's `facets` (a
/// `BTreeSet`) — so a cursor only ever moves forward: [`take_at`] skips the
/// entries that sort before its target, takes the one equal to it, and stops
/// at the first that sorts after, which is what `BTreeMap::get` answers for
/// that address. The section cursor halts at the first `Facet` entry; the
/// facet cursor passes every `Section` entry on its variant alone
/// ([`rank_facet`]), without a string compared.
///
/// The membership filters are the probe walk's: a `Section` entry is read only
/// for a section `sections` names, and a `Facet` entry only for a facet in
/// that section's `facets`. A cell map could hold an address its sections do
/// not name — nothing `from_spec` builds does, and `validate` refuses one on
/// load, but the claim does not rest on that:
/// `t110a_the_merge_is_the_lookup_walk_on_hand_built_maps` feeds both. A
/// facet whose armour cell is missing is dropped as it was; a section with no
/// internals cell reads `None` as it did; the ids on the rows are the
/// `SmolStr::new(id.as_str())` copies they were.
///
/// Per stock-tank frame: one section `Vec` sized to the map's own count (the
/// iterator's lower bound, exact for a `BTreeMap`), one facet `Vec` per
/// section sized to its facets, and the id copies on the rows — where the
/// probes spent 43 `SmolStr` clones, 16 searches and two `canon` strings.
pub(super) fn frame_sections_merged<'a>(
    sections: impl Iterator<Item = (&'a SectionId, &'a SectionState)>,
    section_run: impl Iterator<Item = (&'a CellAddr, &'a ScalarCell)>,
    facet_run: impl Iterator<Item = (&'a CellAddr, &'a ScalarCell)>,
    armor: &CellId,
    internals: &CellId,
) -> Vec<FrameSection> {
    let mut section_run = section_run.peekable();
    let mut facet_run = facet_run.peekable();
    let mut out = Vec::with_capacity(sections.size_hint().0);
    for (id, section) in sections {
        let internals = take_at(&mut section_run, |addr| rank_section(addr, id, internals))
            .map(|cell| (cell.current, cell.max));
        let mut facets = Vec::with_capacity(section.facets.len());
        for facet in &section.facets {
            if let Some(cell) = take_at(&mut facet_run, |addr| rank_facet(addr, id, facet, armor)) {
                facets.push(FrameFacet {
                    id: SmolStr::new(facet.as_str()),
                    armor: cell.current,
                    armor_max: cell.max,
                });
            }
        }
        out.push(FrameSection {
            id: SmolStr::new(id.as_str()),
            condition: SmolStr::new(section.condition.as_str()),
            internals,
            facets,
        });
    }
    out
}

/// **Advance one cursor to `rank`'s target** (16c T-110a): consume the entries
/// that rank `Less`, answer the cell of the one that ranks `Equal` (consumed),
/// stop without consuming at the first that ranks `Greater`, and answer `None`
/// at the end of the run or at that stop.
///
/// Over a run in the target's ascending order, with distinct targets asked in
/// ascending order, this is `BTreeMap::get` for each target: an entry consumed
/// for one target sorts below it and so below every later target, and the
/// entry it stops at is the first a later target can equal. The map's keys are
/// unique, so the `Equal` entry is the only one.
pub(super) fn take_at<'a, I>(
    run: &mut std::iter::Peekable<I>,
    mut rank: impl FnMut(&CellAddr) -> std::cmp::Ordering,
) -> Option<&'a ScalarCell>
where
    I: Iterator<Item = (&'a CellAddr, &'a ScalarCell)>,
{
    while let Some(&(addr, cell)) = run.peek() {
        match rank(addr) {
            std::cmp::Ordering::Less => {
                run.next();
            }
            std::cmp::Ordering::Equal => {
                run.next();
                return Some(cell);
            }
            std::cmp::Ordering::Greater => return None,
        }
    }
    None
}

/// **Where a cell address sorts against the section cell `(section, cell)`**
/// (16c T-110a): `CellAddr`'s own derived order without the address built —
/// `Unit` before every `Section`, `Facet` and `Component` after every one, and
/// a `Section` entry by its section id then its cell id, the ids compared as
/// the text they are (`SmolStr`'s order is `str`'s).
/// `t110a_the_cursor_ranks_as_the_address_orders` holds it to `CellAddr::cmp`
/// on every address of the stock tank against every target, present or not.
pub(super) fn rank_section(
    addr: &CellAddr,
    section: &SectionId,
    cell: &CellId,
) -> std::cmp::Ordering {
    match addr {
        CellAddr::Unit { .. } => std::cmp::Ordering::Less,
        CellAddr::Section {
            section: at,
            cell: of,
        } => (at, of).cmp(&(section, cell)),
        CellAddr::Facet { .. } | CellAddr::Component { .. } => std::cmp::Ordering::Greater,
    }
}

/// **Where a cell address sorts against the facet cell `(section, facet,
/// cell)`** (16c T-110a): [`rank_section`]'s twin — `Unit` and every `Section`
/// before, `Component` after, and a `Facet` entry by its section id, facet id
/// then cell id.
pub(super) fn rank_facet(
    addr: &CellAddr,
    section: &SectionId,
    facet: &FacetId,
    cell: &CellId,
) -> std::cmp::Ordering {
    match addr {
        CellAddr::Unit { .. } | CellAddr::Section { .. } => std::cmp::Ordering::Less,
        CellAddr::Facet {
            section: at,
            facet: on,
            cell: of,
        } => (at, on, of).cmp(&(section, facet, cell)),
        CellAddr::Component { .. } => std::cmp::Ordering::Greater,
    }
}

/// **The platform's heat, as its pilots' frame carries it** (16b cut X.1).
///
/// Two integers and nothing derived: the ceiling is the cell's own maximum, so
/// the cockpit's predicate and [`crate::arena::state::UnitState::overheated`] are
/// the same comparison over the same pair and cannot drift.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameHeat {
    /// What it is carrying.
    pub heat: u32,
    /// The ceiling it refuses to fire at.
    pub max: u32,
}

impl FrameHeat {
    /// Whether the platform is at its ceiling — doc 16 §9 item 1's refusal, read
    /// off the frame.
    #[must_use]
    pub const fn overheated(&self) -> bool {
        self.heat >= self.max
    }
}

/// A hull's own motor status. Formation-adjusted internal goals stay private.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameMotor {
    /// The requested cell, before formation fan-out.
    pub requested_goal: (u16, u16),
    /// None while driving; the retained result until a new request or disengagement.
    pub outcome: Option<crate::arena::perception::DriveOutcome>,
}

impl Frame {
    /// Whether this frame says the same thing as `other`: the unit, the four pose
    /// integers, the speed, the power, the block, the tempo and the contact list.
    ///
    /// **The contacts are compared as a pilot reads them** (16b cut U.2, 16c
    /// `T-30`, Q27, Q28, `T-20`; doc 16 §3.1's latest-wins slot). A row's age
    /// moves every millisecond it is held, and it reaches a pilot through three
    /// surfaces: the **whole second** every panel and `CONTACTS` print, the
    /// **stale clause** the read verbs add once the plot is older than
    /// `pass_ms`, and on the locked row the **lock's range step**, which moves
    /// below a second. A frame is sent when any of them would print differently
    /// ([`crate::arena::perception::FrameContact::same_as`], through
    /// [`crate::arena::perception::age_key`] and
    /// [`crate::arena::perception::lock_range_step_m`]) and not on every pulse
    /// the milliseconds moved. The keyframe still carries `age_ms` to the
    /// millisecond, so what a pilot **may read** is unchanged on every surface;
    /// only what they are **sent** is. Until this cut the rows were compared in
    /// full, and every pilot holding any contact was sent a whole keyframe every
    /// wake.
    ///
    /// `pass_ms` is the arena's own `[tacsim.sensors].pass_ms`
    /// (`Arena::sensors_cfg`), the same number the world's `stale_clause` reads
    /// off its configuration.
    ///
    /// The notices are **not** compared: see [`Frame::notices`]. Nor is a weapon's
    /// `ready_in_ms` countdown: see [`FrameWeapon::same_as`].
    ///
    /// **16b cut X.1** put the heat in: a platform that is cooling changes a
    /// number every substep and its pilot is told, because a heat gauge that
    /// lagged a tempo behind would be a lie about the one thing standing between
    /// them and the next shot. A platform with no heat compares `None` to `None`
    /// and costs nothing.
    ///
    /// **16b cut Y.4**: a frame carrying a catch-up report is never the same as
    /// anything ([`Frame::catchup`]) — a re-seated pilot has no remembered frame
    /// anyway, and this pins that the report is sent.
    ///
    /// **17b cut M1a.3** (doc 17 §3): the order is compared exactly, and the
    /// odds **at the whole percent** — the same length, each entry's contact and
    /// weapon equal, and `p_permille / 10` equal — because a frame is sent when
    /// the figure a pilot is printed moves (`cannon 62%`), and a per-mille wobble
    /// inside one percent prints nothing new. The row's new fields ride
    /// [`crate::arena::perception::FrameContact::same_as`]. [`Frame::t_ms`] is
    /// never compared: it moves every pulse and prints nowhere, so comparing it
    /// would send every pilot a keyframe every wake, which is what 16b cut U.2
    /// took out.
    ///
    /// **17b cut M1b.3**: each weapon's arming rides
    /// [`FrameWeapon::same_as`]; [`Frame::odds_floor_permille`] is never
    /// compared — a per-match constant, captured at check-out, which cannot
    /// differ between two frames of one match.
    ///
    /// **17b cut M5.4**: [`Frame::mind`] is compared exactly. A switch, the
    /// next-switch moment, a degraded core and a drop to the ROM are what
    /// `MIND` prints; without it a switch on a quiet, stationary hull sends
    /// no frame and every reader of the held frame reads the old profile.
    #[must_use]
    pub fn same_as(&self, other: &Frame, pass_ms: i64) -> bool {
        self.catchup.is_none()
            && other.catchup.is_none()
            && self.unit == other.unit
            && self.team == other.team
            // 16b cut Q.1: both are surfaces a pilot reads — the callsign on
            // `STATUS`'s first line, the cell on a panel's map — so both decide
            // whether a frame is sent. The callsign never moves in a match and
            // the cell moves once a cell, so neither costs a frame anybody would
            // not already have been sent.
            && self.callsign == other.callsign
            && self.cell == other.cell
            && self.mode == other.mode
            && self.objective == other.objective
            && self.motor == other.motor
            && self.own_last_hit_ms == other.own_last_hit_ms
            && self.pose.x_cm == other.pose.x_cm
            && self.pose.y_cm == other.pose.y_cm
            && self.pose.z_cm == other.pose.z_cm
            && self.pose.heading_mdeg == other.pose.heading_mdeg
            && self.speed_cms == other.speed_cms
            && self.power == other.power
            && self.held == other.held
            // **16b cut D.2**: both decide what the cockpit refuses locally, and
            // both are one-way flips that happen at most once in a match — so
            // comparing them costs no frame anybody would not already have been
            // sent, and a pilot whose tank has just died is sent one at once.
            && self.wreck == other.wreck
            && self.immobile == other.immobile
            && self.tempo == other.tempo
            // **16b cut D.1**: the pilot's band word is read against it, so a
            // frame that carried a different sight would print different words.
            // It never moves inside a match — the sensor row and the cell size
            // are both fixed at check-out — so comparing it costs no frame
            // anybody would not already have been sent.
            && self.sight_cm == other.sight_cm
            // **16b cut U.2** (16c `T-30`): row by row, at the resolutions a
            // surface prints the age at; the locked row at the lock's step too.
            && self.contacts.len() == other.contacts.len()
            && self
                .contacts
                .iter()
                .zip(other.contacts.iter())
                .all(|(mine, theirs)| mine.same_as(theirs, pass_ms, self.lock == Some(mine.id)))
            // **16b cut Q.2**: the readings ride the contacts, which are already
            // compared row by row, and the tables are what those readings are in the
            // order of — so a table list that moved with no contact moving would
            // be a panel drawing the wrong section names. Two `Vec<Vec<SmolStr>>`
            // of at most one entry per distinct chassis in the scope.
            && self.scan_tables == other.scan_tables
            && self.lock == other.lock
            && self.heat == other.heat
            // **16b cut D.4**: `STATUS` prints it and a panel draws it, so a
            // receiver that changed degree inside one bubble — which journals
            // nothing and says nothing — still sends the pilot the numbers.
            && self.receiver == other.receiver
            // **16b cut O.2**: `STATUS` prints it and both panels draw it, and
            // it moves only on a pilot's own flip, so comparing it costs no
            // frame anybody would not already have been sent.
            && self.ecm == other.ecm
            // **Doc 21 cut G.12a**: the master gate moves only on the pilot's
            // own flip and the ceilings never, so neither costs a frame anybody
            // would not already have been sent.
            && self.weapons_free == other.weapons_free
            && self.max_speed_cms == other.max_speed_cms
            && self.reverse_cms == other.reverse_cms
            // **16b cut F.2**: the damage surface is compared, so a round that
            // came off the armour sends a frame. It is the cheapest comparison on
            // this list after the integers — a handful of small structs per pilot
            // — and a status block that lagged a pulse behind the tank would be
            // the same lie the heat gauge would be.
            && self.sections == other.sections
            && self.rules == other.rules
            && self.weapons.len() == other.weapons.len()
            && self
                .weapons
                .iter()
                .zip(other.weapons.iter())
                .all(|(mine, theirs)| mine.same_as(theirs))
            // **17b cut M1a.3**: at the printed resolution, and the order exactly.
            && self.order == other.order
            && self.odds.len() == other.odds.len()
            && self.odds.iter().zip(other.odds.iter()).all(|(mine, theirs)| {
                mine.contact == theirs.contact
                    && mine.weapon == theirs.weapon
                    && mine.p_permille / 10 == theirs.p_permille / 10
            })
            // **17b cut M5.4**: a switch, the next-switch moment, a degraded
            // core and a drop to the ROM are what `MIND` prints; the sealed
            // set is a per-match constant and compares equal.
            && self.mind == other.mind
    }
}

/// The remembered frames, as [`Arena`] holds them — a type alias so the field's
/// declaration reads as one thing rather than as a nested generic.
pub type FrameSlots = BTreeMap<ObjId, Frame>;

impl Arena {
    /// One frame per pilot whose view changed, in pilot-id order (doc 16 §3.1's
    /// latest-wins slots).
    ///
    /// Two passes on purpose: the first reads the units, the second writes the
    /// remembered frames. A pilot seated on a unit this arena does not hold gets
    /// nothing rather than an empty frame.
    ///
    /// **17b cut M1a.3** factored the first pass's body out as
    /// [`Arena::frame_for`], so a mind's view (17b cut M1a.5) is built by the
    /// same function a pilot's frame is; this keeps its two passes and the one
    /// `pending_notices.clear()`.
    pub(super) fn frames(&mut self, tempo: Tempo) -> Vec<(ObjId, Frame)> {
        let mut candidates = self.frame_candidates(tempo);
        // One pulse's worth, and no more: a platform nobody is seated on
        // accumulates nothing.
        self.pending_notices.clear();
        candidates.sort_by_key(|(pilot, _)| *pilot);
        // 16b cut U.2 (16c `T-30`): the cadence the stale clause is read against,
        // as `Frame::same_as` compares an age.
        let pass_ms = i64::from(self.sensors_cfg.pass_ms.max(1));
        let mut changed: Vec<(ObjId, Frame)> = Vec::new();
        for (pilot, mut frame) in candidates {
            // **16b cut Y.4**: the catch-up report a reclaim left this pilot,
            // once — taken here, so it rides exactly one frame, and that frame
            // is always sent (`Frame::same_as`).
            frame.catchup = self.pending_catchup.remove(&pilot);
            let told = !frame.notices.is_empty();
            if told
                || self
                    .last_frames
                    .get(&pilot)
                    .is_none_or(|last| !last.same_as(&frame, pass_ms))
            {
                // The remembered copy carries no notice: `CONTACTS` and `VIEW`
                // re-render it, and a notice said twice is a notice said wrong.
                // 16b cut Y.4: nor a catch-up, for the same reason.
                let mut remembered = frame.clone();
                remembered.notices.clear();
                remembered.catchup = None;
                self.last_frames.insert(pilot, remembered);
                changed.push((pilot, frame));
            }
        }
        changed
    }

    /// Select a bounded host policy for pure frame construction. The standalone
    /// library defaults to one worker; task hosts set their available CPU budget.
    /// This changes no saved state, draw, ledger record, or ordering.
    pub fn set_frame_workers(&mut self, workers: usize) {
        self.frame_workers = workers.clamp(1, 4);
    }

    pub(super) fn frame_candidates(&self, tempo: Tempo) -> Vec<(ObjId, Frame)> {
        let build = |pilots: &[crate::arena::state::Pilot]| {
            // Preserve the serial path's one allocation, including short cohorts.
            let mut frames = Vec::with_capacity(pilots.len());
            for pilot in pilots {
                if let Some(frame) = self.frame_for(pilot.unit, tempo) {
                    frames.push((pilot.pilot, frame));
                }
            }
            frames
        };
        // At least four pilots per lane, at most three lanes. This admits the
        // shipped twelve-seat lobby. Only immutable reads run here;
        // consumption and latest-frame publication follow in pilot-id order.
        let workers = self.frame_workers.min(3).min(self.pilots.len() / 4).max(1);
        if workers == 1 {
            return build(&self.pilots);
        }
        // Reuse one bounded process-wide pool: creating OS threads every pulse
        // cost more than the work saved in the dev1 A/B measurement. The caller
        // builds one chunk; at most three pool workers serve all arenas. A host
        // unable to create the pool retains the identical serial path.
        static POOL: std::sync::OnceLock<Option<rayon::ThreadPool>> = std::sync::OnceLock::new();
        let Some(pool) = POOL.get_or_init(|| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(
                    std::thread::available_parallelism()
                        .map_or(1, usize::from)
                        .saturating_sub(1)
                        .clamp(1, 3),
                )
                .thread_name(|index| format!("tacsim-frame-{index}"))
                .build()
                .ok()
        }) else {
            return build(&self.pilots);
        };
        let chunk_size = self.pilots.len().div_ceil(workers);
        let (first, rest) = self.pilots.split_at(chunk_size);
        let mut outputs: Vec<Vec<(ObjId, Frame)>> =
            rest.chunks(chunk_size).map(|_| Vec::new()).collect();
        let mut candidates = Vec::with_capacity(self.pilots.len());
        pool.in_place_scope(|scope| {
            for (chunk, output) in rest.chunks(chunk_size).zip(&mut outputs) {
                scope.spawn(move |_| *output = build(chunk));
            }
            candidates.extend(build(first));
        });
        // Scope joins every borrowed chunk and propagates panics to the arena's
        // existing unwind boundary before any mutable publication can happen.
        for output in outputs {
            candidates.extend(output);
        }
        candidates
    }

    /// **One hull's frame, as the pulse would build it now** (17b cut M1a.3; doc
    /// 17 §3), or `None` for a unit this arena does not hold.
    ///
    /// Everything [`Arena::frames`] builds per pilot, factored out so 17b cut
    /// M1a.5's `mind_view` is the same function a pilot's keyframe is (doc 17 §3's
    /// parity rule: a mind reads exactly what the keyframe carries). Two things
    /// it deliberately does not do: it reads `pending_notices` **without
    /// clearing** them — the clear is `frames`' own, once a pulse — and it leaves
    /// `catchup` `None`, which `frames`' second pass takes. Its `t_ms` is
    /// `self.now_ms`: inside the due loop that is the substep's end, the state's
    /// own clock, which 17b cut M1a.5 relies on.
    pub(crate) fn frame_for(&self, unit: ObjId, tempo: Tempo) -> Option<Frame> {
        let unit = self.unit(unit)?;
        let lock = unit.lock;
        // 16b cut Q.2: the scope and the vocabularies its readings are in,
        // built in one walk (`Arena::frame_scope`).
        let (contacts, scan_tables) = self.frame_scope(unit.id);
        // **16b cut W.1, re-shaped by cut F.2**: the lock's own target, range
        // and cover. Cut W.1 needed only the range — the band each weapon's
        // lock falls in — and read it off the contact list the frame already
        // carries; cut F.2 resolves **each weapon's** own chance, which needs
        // the target and one trace. It is one trace per *seated* platform per
        // pulse, which is what `lock_p_permille` already cost.
        let aim = self.lock_aim(unit.id);
        Some(Frame {
            unit: unit.id,
            team: unit.team,
            unit_type: SmolStr::new(unit.unit.schema().key().id.as_str()),
            sensor_height_cm: unit.sensor_height_cm,
            // 16b cut Q.1: the platform's own public name, and the cell
            // it is standing in.
            callsign: unit.callsign,
            cell: self.definition.terrain.cell_of(&unit.pose),
            mode: self.mode,
            own_last_hit_ms: unit.own_last_hit_ms,
            objective: self
                .hold
                .as_ref()
                .map(crate::arena::state::HoldState::objective),
            motor: self.motors.row(unit.id).and_then(|row| {
                row.requested_goal.map(|requested_goal| FrameMotor {
                    requested_goal,
                    outcome: row.outcome,
                })
            }),
            pose: unit.pose.clone(),
            speed_cms: unit.speed_cms,
            power: unit.power,
            // 16b cut F.2: the condition, not the substep's flag.
            held: unit.held.is_some(),
            // 16b cut D.2: the pilot's own two limits, so the cockpit can
            // refuse an order to a dead tank at once and say why.
            wreck: unit.limits.wreck,
            immobile: unit.limits.motion_disabled,
            tempo,
            // **16b cut D.1**: the nominal reach of the platform's own
            // sight, `max_range_cells × cell_cm`, and `0` for a hull
            // that carries no sensor. The same product
            // `Arena::detections` starts from, unscaled — see
            // [`Frame::sight_cm`]. **16b cut E.1** made it one product
            // in one place ([`UnitState::sight_cm`]), because the
            // machine's own fighting reach is priced off it too.
            sight_cm: unit.sight_cm(self.definition.terrain.cell_cm()),
            contacts,
            scan_tables,
            // Per platform, so two pilots in one tank are both told.
            notices: self
                .pending_notices
                .get(&unit.id)
                .cloned()
                .unwrap_or_default(),
            lock,
            weapons: self.frame_weapons(unit.id, aim.as_ref()),
            rules: self.frame_rules(unit.id),
            // 16b cut F.2: doc 16 §5's status block, off the same M32
            // cells the impact writes. 16c T-110a: off the unit in hand,
            // not a second search for it.
            sections: Self::frame_sections_of(&unit.unit),
            // 16b cut X.1: two integers off the unit's own cells, and
            // `None` for a platform that carries no heat at all.
            heat: match (unit.heat_now(), unit.heat_max()) {
                (Some(heat), Some(max)) => Some(FrameHeat { heat, max }),
                _ => None,
            },
            // 16b cut D.4: the platform's own receiver, as its last
            // sensor pass left it.
            receiver: unit.receiver,
            // **16b cut O.2**: the pod's switch, for a platform that
            // carries a pod.
            ecm: unit.jammer.map(|_| unit.ecm_on),
            // 16b cut Y.4: taken in `frames`' second pass, which may
            // write the arena.
            catchup: None,
            // **17b cut M1a.3** (doc 17 §3, D-UM-5): the odds against the
            // lock and the nearest classified hostiles (17b cut M1b.3's
            // Q2), the standing order re-keyed to
            // this hull's own rows, and the clock the ages above are read on.
            odds: self.frame_odds(unit.id),
            order: self.frame_order(unit.id),
            t_ms: self.now_ms,
            // **17b cut M1b.3** (D-UM-4, D-UM-17): the floor the match was
            // checked out with — never the live config.
            odds_floor_permille: self.odds_floor_permille,
            // **17b cut M5.4** (doc 17 §12): the hull's sealed mind, its
            // classes read live.
            mind: self.frame_mind(unit),
            // **Doc 21 cut G.12a** (§7.3): the seat's master gate and the two
            // speed ceilings the arena's own `SPEED` clamp reads.
            weapons_free: !unit.weapons_hold,
            max_speed_cms: unit.mobility.max_speed_cms,
            reverse_cms: unit.mobility.reverse_ceiling_cms(),
        })
    }

    /// **The hull's sealed mind as its frame carries it** (17b cut M5.4; see
    /// [`FrameMind`]), or `None` on a hull no seal reached (the zero digest
    /// of [`crate::arena::mind::SealRef::none`], the test `apply_profile`
    /// refuses a switch on).
    pub(super) fn frame_mind(&self, unit: &crate::arena::state::UnitState) -> Option<FrameMind> {
        let seal = &unit.seal;
        if seal.digest == [0; 32] {
            return None;
        }
        let runtime = self.profiles.get(&unit.id)?;
        Some(FrameMind {
            name: seal.name.clone(),
            doctrine: seal.doctrine,
            profiles: runtime.profiles.clone(),
            card: runtime.card.clone(),
            program: runtime.program.clone(),
            active: unit.card_profile.index,
            // Both LIVE: the sealed class against the core now, and the core
            // now — never `seal.class` or `seal.core_class`.
            class: self.mind_class(unit.id),
            core_class: self
                .core_of(unit.id)
                .map_or(crate::arena::core::CoreClass::Zero, |core| core.class),
            switch_at_ms: unit
                .card_profile
                .switched_ms
                .map(|at| at.saturating_add(crate::arena::intents::PROFILE_SWITCH_MS)),
            off: self
                .minds
                .get(&unit.id)
                .is_some_and(crate::arena::mind::MindState::dropped),
        })
    }

    /// **The odds a frame carries** (17b cut M1a.3; doc 17 §3, D-UM-5): per
    /// weapon, against the platform's lock and its nearest **classified
    /// hostile** contacts. Keep the existing two-row pilot floor; a current
    /// effective sealed Class II/III may publish its configured wider odds
    /// capacity. This same projection feeds the cockpit and the mind view.
    ///
    /// * **Which contacts** (the lead's ruling on 17b's Q2, built in 17b cut
    ///   M1b.3). Only a held row whose class is `Classified { team }` with
    ///   `team` **another side than the hull's own**, and whose target is still
    ///   in the match: the fire control resolves hostiles, so a wingman between
    ///   the hull and the enemy never takes a row. **The hull's lock first** when
    ///   it is such a row — the contact a pilot is about to shoot is the one the
    ///   rows must quote, however far it is; then every other such row, nearest
    ///   first by the range to the **plot**, `ContactId` ascending on a tie; the
    ///   first `max(STOCK_ODDS_ROWS, effective_class.odds_rows)`. A friendly
    ///   lock takes no row (its `LOCK`
    ///   line still quotes the lock's own aim). Never an unclassified contact:
    ///   [`Arena::resolve_seen`] reads the target's silhouette off the unit, and
    ///   for a return the platform has not told apart that table is the chassis
    ///   it has not earned. Every row outside the odds rows prints `-`.
    /// * **What each costs.** One [`Arena::aim_at`] per contact — one trace —
    ///   then one [`Arena::resolve_seen`] per weapon, so at most two traces and
    ///   six resolutions per frame on a stock hull.
    /// * **In what order.** For each contact, the weapon a bare `FIRE` would fire
    ///   ([`crate::arena::view::fired_by_default`]) **first**, then the rest in
    ///   component order — so a client reading a contact's first entry and line
    ///   mode, which prints `fired_by_default`'s, quote the same gun.
    ///
    /// Empty for a platform with no weapons — no trace paid — and when no
    /// classified contact is held.
    #[must_use]
    pub fn frame_odds(&self, platform: ObjId) -> Vec<FrameOdds> {
        let Some(from) = self.unit(platform) else {
            return Vec::new();
        };
        if from.weapons.is_empty() {
            return Vec::new();
        }
        // **17b cut M1b.3** (Q2): classified, of another side, still in the
        // match — the one predicate the lock and the rest are both held to.
        let hostile = |held: &crate::arena::state::ShadowContact| {
            matches!(held.class, Classification::Classified { team } if team != from.team)
                && self.unit(held.target).is_some()
        };
        let locked = from
            .lock
            .filter(|id| self.contacts.get(platform, *id).is_some_and(hostile));
        let mut nearest: Vec<(u64, ContactId)> = self
            .contacts
            .of(platform)
            .filter(|(id, held)| Some(**id) != locked && hostile(held))
            .map(|(id, held)| (held.seen_at.range_from(&from.pose), *id))
            .collect();
        // Range, then the id: a total order over integers, so two equally
        // distant contacts are taken the same way on every replay.
        nearest.sort_unstable();
        let rows: Vec<ContactId> = locked
            .into_iter()
            .chain(nearest.into_iter().map(|(_, id)| id))
            .take(STOCK_ODDS_ROWS.max(self.mind_profile(platform).odds_rows))
            .collect();
        // The weapon a bare `FIRE` would fire, through the one rule
        // (`UnitState::default_weapon`, 16b cut F.3) the view's
        // `fired_by_default` mirrors — never a fourth spelling of it.
        let default = from.default_weapon().map(|weapon| weapon.id.clone());
        let ordered: Vec<&SmolStr> = default
            .iter()
            .chain(
                from.weapons
                    .iter()
                    .map(|weapon| &weapon.id)
                    .filter(|id| Some(*id) != default.as_ref()),
            )
            .collect();
        let mut odds: Vec<FrameOdds> = Vec::new();
        for contact in rows {
            let Some(aim) = self.aim_at(platform, contact) else {
                continue;
            };
            for weapon in &ordered {
                odds.push(FrameOdds {
                    contact,
                    weapon: (*weapon).clone(),
                    p_permille: self.resolve_seen(platform, &aim, weapon).p_permille,
                });
            }
        }
        odds
    }

    /// **What one tank has left, as its own pilots' frame carries it** (16b cut
    /// F.2; doc 16 §5's status block: *armor and internals per section as current
    /// over max*), in **section-id then facet-id order** — which is the order
    /// [`mush_tactics::TacticalUnit`]'s own ordered maps hold them in, and not the
    /// order the pack authored them in.
    ///
    /// A projection of the M32 cells and never a second model: the impact's plan
    /// is the single writer of both (doc 16 §3.5 "Damage"), so there is nothing
    /// here to keep in step. It is built **per pilot per pulse** and the walk is
    /// the unit's own five sections and their facets — a handful of small reads,
    /// which is why a platform nobody is seated on pays nothing for it.
    ///
    /// It names the pilot's **own** tank and is therefore outside the observer
    /// rule: what a crew can read off its own gauges is physical knowledge.
    ///
    /// **16c T-110a — one ordered pass.** Until this cut the block was sixteen
    /// `BTreeMap` probes a frame on the stock tank — `TacticalUnit::cell` once
    /// per section for its internals and once per facet for its armour, each
    /// probe an address built from two or three cloned ids and compared by
    /// `CellAddr`'s derived order — under two `CellId::new`s, each a fresh
    /// `String` through `canon`: 9.4 per cent of the CI step and 6.1 of the
    /// authored at the hash v2 tree (`canon` 2.1–3.6, `CellAddr::cmp` 2.7–3.7
    /// beside it), run inside every mind view (about 74 a wake) and every
    /// pilot frame (24). Now the two ids are built once for the process
    /// ([`crate::definitions::unit::armor_cell`] and
    /// [`crate::definitions::unit::internals_cell`]) and the block is one
    /// forward pass over `sections()` merged with the unit's cell map in that
    /// map's own order (`frame_sections_merged`): no address is built, no id
    /// is cloned, and the rows carry the same `SmolStr::new(id.as_str())`
    /// copies in the same order. The output is the probe walk's exactly —
    /// `t110a_frame_sections_is_the_lookup_walk` holds it to that walk, kept
    /// in the tests as the reference, over a shipped fight under fire — so no
    /// frame byte moves; the frame is never hashed or journalled by these
    /// fields, and no draw, fold, ledger line or counter reads this path.
    /// [`Arena::frame_sections_of`] is the walk on a unit in hand, which is
    /// how `frame_for` reads it — off the unit it has already resolved, with
    /// no second search of the unit table for the same row; this is that
    /// search followed by it.
    #[must_use]
    pub fn frame_sections(&self, platform: ObjId) -> Vec<FrameSection> {
        self.unit(platform)
            .map_or_else(Vec::new, |state| Self::frame_sections_of(&state.unit))
    }

    /// **The status block of a unit in hand** (16c T-110a):
    /// [`Arena::frame_sections`] without the search — the merged walk
    /// (`frame_sections_merged`) over the unit's sections and cells under the
    /// once-built ids, and an empty block if either id were ever refused, as
    /// before. `frame_for` reads it off the unit it has already resolved, where
    /// it searched the unit table a second time for the same row (one of the
    /// six such searches a frame makes; the other five remain);
    /// `frame_sections` is the search followed by it, so the two cannot drift.
    #[must_use]
    pub fn frame_sections_of(unit: &mush_tactics::TacticalUnit) -> Vec<FrameSection> {
        let (Some(armor), Some(internals)) = (armor_cell(), internals_cell()) else {
            return Vec::new();
        };
        frame_sections_merged(
            unit.sections(),
            unit.cells(),
            unit.cells(),
            armor,
            internals,
        )
    }
}
