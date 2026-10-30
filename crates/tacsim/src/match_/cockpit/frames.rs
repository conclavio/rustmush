//! Structured `arena.cockpit`, `arena.map`, and `arena.view` panel frame builders.

use super::*;

// ---------------------------------------------------------------------------
// The frame (doc 16 §5)
// ---------------------------------------------------------------------------

/// The frame's schema id, which every consumer keys its decode on.
pub const FRAME_SCHEMA: &str = "arena.cockpit";

/// Its version. Cuts S and W add fields; a field added is not a version.
///
/// **17b cut M1a.3 bumped it to 2** (doc 17 §3, D-UM-4, D-UM-5): the fields a
/// mind reads joined the frame — a field added is still not a version, but the
/// first fields added FOR a reader that is not a person are, so a client can
/// say which frame it was built against; every consumer ignores unknown keys
/// and the version, so an older client prints the old row.
///
/// **17b cut M1b.3 bumped it to 3** (doc 17 §3's parity rule, §5, D-UM-4,
/// D-UM-17): two more fields added for a reader that is not a person — each
/// weapon row's `armed` (the condition slot the weapon is armed in, or `null`)
/// and the top-level `odds_floor_permille` (the match's Compact floor) — so a
/// mind reads its own slots and the floor exactly as a pilot does. Every
/// consumer ignores unknown keys and the version, so an older client prints
/// the old rows.
///
/// **17b cut M5.4 kept it at 3** while adding the top-level `mind` object
/// (doc 17 §12; F7 of the M5 design note): a field added for the person in
/// the seat is not a version, and a mind's view never carries it.
///
/// **M5rules bumped it to 4** for the shared typed ordered-rule rows.
/// **M5r bumped it to 5** for public objective/motor facts and own-shot origin.
/// **M6 bumps it to 6** for generic owned-mind card values/schema, profiles and
/// full sealed-rule readback. Source and private memory stay out of the frame.
/// **M7 bumps it to 7** for the shared public odds projection at the actual
/// effective class's already-advertised ceiling. Class I/native stay at two rows.
/// **DT.1b bumps it to 8** for an explicit, opaque order identity shared with
/// mind API 6. Legacy orders carry null; standing orders do not use expiry as id.
pub const FRAME_VERSION: u32 = 8;

/// **The terrain frame's schema id** (16b cut Q.3), carried on
/// [`crate::arena_bridge::MAP_FEED`].
pub const MAP_SCHEMA: &str = "arena.map";

/// Its version. The map is terrain and an origin; a field added is not a
/// version, exactly as [`FRAME_VERSION`]'s comment says of the cockpit frame.
pub const MAP_VERSION: u32 = 1;

/// **The bound a pulled keyframe is held to**, bytes of encoded JSON (16b cut
/// Q.3's 256 KiB; made a rule by 16b cut U.4, 16c `T-57`).
///
/// [`map_frame`] is the largest message the arena produces and it is only ever
/// the answer to a query, so this is the most one `arena.map` reply may cost a
/// session's output queue. It is enforced **once, when an arena definition is
/// applied** (`crate::definitions::arena::arena_map_within_bound`, called by the
/// arena applier), not every time a map is built: a loaded arena cannot exceed
/// it, so the handler that sends one never has to ask.
pub const MAP_MAX_BYTES: usize = 256 * 1024;

/// **The terrain frame's shape** (16b cut Q.3).
///
/// Deliberately **not** `grid`: `mush_ansi::panel` renders a `grid` keyframe as
/// twenty rows of text, and the whole arena is 256 by 256. A line client that
/// somehow received one must print nothing, and a shape it does not know is how
/// it does that.
pub const MAP_SHAPE: &str = "terrain";

/// **The `schema.retain` marker that makes a keyframe replace its own
/// predecessors for a session's resume** (16b cut Q.3; the rule is implemented
/// in `mush_session::SessionManager`, which cannot see this crate — the string
/// is the contract). Since 16b cut B.5 a marked keyframe lives in a per-feed
/// latest slot beside the resume ring rather than in it.
///
/// Both arena feeds carry it, and so does [`view_frame`]'s replacement schema
/// (`arena.map`'s never fires: it is only ever a conn-scoped query reply, which
/// is never recorded — see [`crate::arena_bridge::MAP_FEED`]). Two keyframes a
/// second of `Engaged` play would otherwise evict everything else from a
/// 512-entry ring in about four minutes; every frame either feed sends is the
/// whole state, so the older ones are not history and nothing is lost by
/// dropping them.
pub const RETAIN_LATEST: &str = "latest";

/// **The cockpit keyframe** (doc 16 §5): where the tank is, how fast, which way,
/// whether it is awake, whether it is held, how often it is being told things,
/// what it holds and what it carries.
///
/// **16b cut S.2 filled `contacts` and added `notices`; 16b cut W.1 made `weapons`
/// live and filled `lock`.**
///
/// One contact is `{ id, class, bearing_deg, relative_deg, band, age_ms, age_s,
/// cell, friendly }` — no object number, no name and **no exact range**, which is
/// doc 16 §3.6's *range band* read strictly. `cell` is the **arena window's** own
/// cell, so it is comparable with `x_cm / cell_cm`; [`view_frame`]'s `units` carry
/// the same marker at a cell relative to *that* frame's `origin`, which is what the
/// grid renderer draws from. `notices` is the lines this frame owes
/// its pilot, already rendered ([`notice_line`]), so a web or TUI panel prints the
/// same sentence the line client was sent; a frame read back out of the world's
/// mirror carries none, because the mirror is stored with them cleared.
///
/// One weapon is `{ id, ready, ready_in_ms, rounds, rounds_max, range_band }`, and
/// `range_band` is `{ index, of, max_range_cm, dispersion_mrad }` for the band the
/// platform's lock falls in — `null` with no lock, which is also when `lock` is
/// `null`. It takes no world handle: the arena is the single writer of both cells
/// while a match runs, so the frame is the whole answer.
///
/// **16b cut W.2** added `lock` a companion: `lock_p_permille` and `lock_percent`,
/// doc 16 §3.5 step 5's chance against the held lock, per mille and as the whole per
/// cent the line client prints. Both are `null` with no lock and when the arena has
/// not resolved one, which is what keeps a panel from drawing a 0 % that means
/// *unknown*. **16b cut F.2** made them the chance of *the weapon a bare `FIRE`
/// would fire* rather than the best of the platform's, and gave every weapon row
/// its own `p_permille` and `percent`.
///
/// **16b cut F.3** added `range_m` to each contact: the coarse range
/// `CONTACTS` prints, metres rounded to [`CONTACT_RANGE_STEP_M`]. It
/// also changed what every one of a contact's numbers is *of* — `bearing_deg`,
/// `range_m`, `band` and `cell` are read off the **plot** the arena recorded at
/// the sighting that last refreshed the contact, and `age_ms`/`age_s` are the age
/// of that sighting rather than the hold time since first detection. A panel is
/// therefore drawing what the platform knows, exactly as the line client is.
///
/// **16b cut Q.2** added `condition` to each contact — `{ table, bands, age_ms }`
/// or `null` — and `scan_tables` beside `contacts`. `bands` is the compact
/// spelling (`"ggyrd"`, one letter a section: green, yellow, red, destroyed) in
/// the order of `scan_tables[table]`, and `age_ms` is the age of the **look** and
/// not of the plot. The table index lives inside the object on purpose: a contact
/// with no reading can publish no section vocabulary by construction, and
/// `scan_tables` lists a chassis only for a contact that carries one. Each notice
/// also gained `section` — the section of the **target** a gunner's line is
/// about, `null` on every other kind — so the new `target_section_lost` notice
/// carries its section as a field and not only inside `text`.
///
/// **16b cut B.1** (`T-86`) widened each notice's `section` to the victim's own
/// `struck`, `struck_spent` and `section_lost` (this platform's section, `kind`
/// says whose) and added `was`, the old designator on an `identified` or
/// `unidentified again` line and `null` on every other.
///
/// **16b cut F.2** also added `sections` — doc 16 §5's status block, one entry per
/// section as `{ id, condition, internals, internals_max, facets[{ id, armor,
/// armor_max }] }` — and `heat` as `{ heat, max, overheated }` or `null`, which
/// cut X.1 put on `STATUS`'s text and not on the payload, so the two renderers had
/// drifted.
///
/// **One owner per sentence.** `notices` carries the same rendered line the bridge
/// sends the pilot as a `Sys` line, and both go to the same audience: a panel
/// renderer that prints this array **as well as** the scrollback will print every
/// notice twice. The array is the shape a panel should *draw* from — kinds, ids,
/// and the text as a fallback — and a renderer that owns the scrollback should not
/// print the text again. A frame read back out of the world's mirror carries no
/// notices at all, because the mirror is stored with them cleared. **16b cut
/// B.1**: the bridge's `Sys` copy of each notice is tagged
/// [`crate::arena_bridge::NOTICE_SENTENCE_TAG`], so a panel ring that also takes
/// the pilot's replies (tagged [`crate::arena_bridge::NOTICE_CATEGORY`]) can tell
/// the sentence it composes from this array from an answer it has to ring.
///
/// **16b cut D.1** added two top-level keys: `sight_m`, the platform's own
/// nominal sensor reach in metres (`0` for a platform with no sensor), and
/// `scope`, the pilot's own alert state — `clear`, `unidentified` or `hostile`
/// ([`own_scope`]). Each contact's `band` is [`contact_band`]'s word: thirds of
/// the lesser of `sight_m` and the fired weapon's reach. `tempo` is unchanged and
/// stays the arena's global word.
///
/// **16b cut B.4** (`T-60`): this is [`status_frame_held`] with no range memory,
/// for the fixtures and every pure reading of a lone frame.
///
/// **16b cut Y.4** added one top-level key, `catchup`: `null`, or on the first
/// frame after a reclaim `{ away_s, cell_then, cell_now, power_then, power_now,
/// wreck, contacts_new, contacts_lost, contacts_more, sections[{ id, then, now
/// }], sections_more, rounds_spent[{ weapon, rounds }], rounds_more, machine }`
/// ([`catchup_lines`] is its wording). [`FRAME_VERSION`] stays `1`: a field
/// added is not a version.
///
/// **17b cut M1a.3** (doc 17 §3, D-UM-4, D-UM-5) added what a mind reads, and
/// bumped [`FRAME_VERSION`] to `2`. Each contact gained `aspect` — the **facet**
/// word its plot was showing (`front`, `left`, `right`, `rear`; the row prints
/// the side word) — and `fatal`, the target chassis's fatal section words beside
/// a held reading and `null` without one; on a hull with a weapons station each
/// contact also gained `reads_dead` (the own kill, or a held reading showing a
/// fatal section destroyed) and `odds`, `[{ weapon, p_permille, percent }]` for
/// the nearest `STOCK_ODDS_ROWS` classified contacts, the default weapon first,
/// `[]` for every other row. Each weapon gained `bands_cm`. The top level gained
/// `t_ms`, the arena millisecond of the build, and `order`: `null` — every
/// piloted hull — or `{ squad, objective, target, to, facing_mdeg, of,
/// hold_fire, max_range_band, expires_in_ms }`, `target` and `of` the
/// designators of the hull's own rows. **A hull with no weapons station
/// carries no `weapons` key** and no contact carries `odds` or `reads_dead`
/// (doc 17 §22's italic line); `lock`, `lock_*`, `heat`, `scan_tables` and
/// `fatal` stay.
///
/// **17b cut M1b.3** (doc 17 §3, §5, D-UM-4, D-UM-17) bumped [`FRAME_VERSION`]
/// to `3`. Each weapon gained `armed`: `null`, or the slot it is armed in as
/// `{ slot, target, min_permille, percent, aspects, keeps, min_range_m,
/// max_range_m }` — `target` the KIND (`lock`, `best` or `contact`) and never a
/// designator, `aspects` the facet words in the mask in `front`, `left`,
/// `right`, `rear` order, `keeps` the rounds the slot keeps back, `min_range_m`
/// `null` for no lower bound and `max_range_m` `null` for no upper one
/// ([`arming_json`]). The top level gained `odds_floor_permille`, the match's
/// Compact floor captured at check-out, beside `t_ms`. Each contact's `odds`
/// now ride the lock first and only classified contacts of another side
/// (`Arena::frame_odds`, Q2): a friend's row carries `[]`.
///
/// **17b cut M5.4** (doc 17 §12 "The `MIND` verb") added one top-level key,
/// `mind`, for the person in the seat — `null` on a hull no seal reached, else
/// `{ name, profiles, active, class, core, channels, next_switch_ms, off }`
/// ([`mind_json`]) — and [`FRAME_VERSION`] stays `3`: a field added for the
/// person in the seat is not a version. It rides only this, the seated
/// pilot's own frame (the replay builds its window from the tape, and a
/// mind's view never copies it).
///
/// **Doc 21 cut G.14** (2026-10-27) added one key per contact, `health`: the
/// one word the picture gauges the hull by — `intact`, `damaged`, `crippled`
/// or `wreck` ([`contact_health`]) — and `null` on every row the platform
/// holds no reading of, which is every unidentified return and every
/// classified one it has not been close enough to for a look. A summary of
/// the row's own `condition` bands and `reads_dead`, never a number, and on
/// every hull armed or not; [`FRAME_VERSION`] stays `7`: a field added is not
/// a version.
#[must_use]
pub fn status_frame(match_id: MatchId, frame: &Frame) -> serde_json::Value {
    status_frame_held(match_id, frame, &BTreeMap::new())
}

/// **The one word the picture gauges a contact by** (doc 21 cut G.14;
/// [`ConditionTier`]), or `None`.
///
/// The frame's own `reads_dead` says `wreck` outright — the platform's own
/// kill, or a held reading that shows a fatal section destroyed
/// ([`crate::arena::perception::FrameContact::reads_dead`]). Otherwise it is
/// the held reading's [`crate::arena::condition::Condition::tier`], the fatal mask resolved from the
/// row's `fatal` words through its scan table's order — so an unarmed hull,
/// whose rows carry no `reads_dead`, reads a wreck off the same reading an
/// armed one would. A row with no reading has no word: every unidentified
/// return (a reading is taken only while classified) and every classified
/// contact the platform has not had close enough for a look. The picture
/// then draws no gauge, which is the observer rule on this surface: a return
/// the scope has not read is not shown whole.
///
/// Never a number. The tier is the worst band the platform already holds and
/// nothing finer, so no exact figure reaches a surface that was not given
/// one by `SCAN`.
#[must_use]
pub fn contact_health(
    frame: &Frame,
    held: &crate::arena::perception::FrameContact,
) -> Option<ConditionTier> {
    if held.reads_dead == Some(true) {
        return Some(ConditionTier::Wreck);
    }
    let reading = held.condition?;
    let names: &[SmolStr] = frame
        .scan_tables
        .get(usize::from(reading.table))
        .map_or(&[], Vec::as_slice);
    let fatal: &[SmolStr] = held.fatal.as_deref().unwrap_or(&[]);
    let mut mask = 0_u32;
    for (index, name) in names.iter().enumerate().take(MAX_SCANNED_SECTIONS) {
        if fatal.contains(name) {
            mask |= 1_u32 << index;
        }
    }
    Some(reading.bands.tier(mask))
}

/// **The cockpit keyframe with each contact's `range_m` held** (16b cut B.4,
/// `T-60`) against `ranges` — [`status_frame`]'s payload, its rows built by
/// [`frame_contacts_held`]. What the bridge sends every pulse and what every read
/// verb re-pushes off the mirror, so the panel rows, `CONTACTS` and `RANGE` read
/// one figure. `lock_range_m` keeps its own step and no memory.
#[must_use]
pub fn status_frame_held(
    match_id: MatchId,
    frame: &Frame,
    ranges: &BTreeMap<ContactId, u64>,
) -> serde_json::Value {
    let rows = frame_contacts_held(frame, ranges);
    // **17b cut M1a.3** (doc 17 §22's italic line): a hull with no weapons
    // station carries no `weapons` block, and its contacts no `odds` and no
    // `reads_dead`.
    let armed = !frame.weapons.is_empty();
    let contacts: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let held = frame.contacts.iter().find(|held| held.id == row.id);
            let mut contact = serde_json::json!({
                // 16b cut Q.1: `id` stays the stable row key a panel matches on
                // across pulses and is never displayed; `designator` is what a
                // person reads and types, and `alias` is the number behind a
                // renamed row.
                "id": row.id.to_string(),
                "designator": row.designator.to_string(),
                "alias": row.alias.map(|n| Designator::Provisional(n).to_string()),
                "class": row.class.as_str(),
                "bearing_deg": row.bearing_deg,
                "relative_deg": row.relative_deg,
                "band": row.band.as_str(),
                // 16b cut F.3: the coarse range, the same figure `CONTACTS`
                // prints, so a panel and a line client read one number.
                "range_m": row.range_m,
                "age_ms": row.age_ms,
                "age_s": row.age_ms / 1_000,
                "cell": row.cell.map(|(x, y)| [x, y]),
                "friendly": row.friendly,
                "moving": held.is_some_and(|held| held.moving),
                // 16b cut Q.2: what the platform has read off this contact's
                // hull, or `null`. The table index is inside the object, so a
                // row with no reading carries no section vocabulary at all —
                // which is the observer rule on this surface, by construction.
                "condition": held
                    .and_then(|held| held.condition)
                    .map(|reading| serde_json::json!({
                        "table": reading.table,
                        "bands": reading.bands.letters(),
                        "age_ms": reading.age_ms,
                    })),
                // **Doc 21 cut G.14**: the one word the picture gauges this
                // hull by, summed from the bands above and `reads_dead`
                // ([`contact_health`]), or `null` with no reading — so a
                // return the scope has not read gets no gauge rather than a
                // full one.
                "health": held
                    .and_then(|held| contact_health(frame, held))
                    .map(|tier| tier.as_str()),
                // **17b cut M1a.3** (doc 17 §3, D-UM-5): the FACET word the
                // contact's plot was showing — `front`, `left`, `right` or
                // `rear`; the row prints its side word — and (doc 22 D-22-9)
                // `facing`, the eight-way direction the picture picks a sprite
                // facing from (`front`, `front_right`, `right`, `rear_right`,
                // `rear`, `rear_left`, `left`, `front_left`); beside a held
                // reading only, the chassis's fatal section words: what a
                // reading would have to show destroyed for `reads_dead`.
                "aspect": held.map_or("front", |held| held.aspect.as_str()),
                "facing": held.map_or("front", |held| held.facing.as_str()),
                "fatal": held.and_then(|held| held.fatal.as_ref()),
                // **Doc 22 D-22-5**: the contact's unit type, its definition
                // id, from classification on (with its team), `null` before —
                // the look a renderer draws it with.
                "unit_type": held.and_then(|held| held.unit_type.as_deref()),
            });
            if armed && let Some(object) = contact.as_object_mut() {
                // **17b cut M1a.3**: the weapons station's two fields. Every
                // `FrameOdds` of this row, the default weapon first (the order
                // `Arena::frame_odds` builds), and `[]` for a row outside the
                // odds rows.
                object.insert(
                    "reads_dead".to_owned(),
                    serde_json::json!(held.and_then(|held| held.reads_dead).unwrap_or(false)),
                );
                object.insert(
                    "odds".to_owned(),
                    serde_json::Value::Array(
                        frame
                            .odds
                            .iter()
                            .filter(|odds| odds.contact == row.id)
                            .map(|odds| {
                                serde_json::json!({
                                    "weapon": odds.weapon.as_str(),
                                    "p_permille": odds.p_permille,
                                    "percent": odds.p_permille / 10,
                                })
                            })
                            .collect(),
                    ),
                );
            }
            contact
        })
        .collect();
    // 16b cut Q.1: a notice that renders to nothing owes the pilot nothing, so it
    // is left out here for the reason `notice_lines` drops it — a panel drawing
    // this array would otherwise draw a blank row.
    let notices: Vec<serde_json::Value> = frame
        .notices
        .iter()
        .filter_map(|notice| {
            let text = notice_line(frame, notice, &rows);
            if text.is_empty() {
                return None;
            }
            let mut object = serde_json::json!({
                "kind": notice.kind().as_str(),
                "contact": notice.contact().map(|id| id.to_string()),
                "designator": notice.designator().map(|named| named.to_string()),
                // **16b cut B.1** (`T-86`): the old name on an `identified` or
                // an `unidentified again` line — the provisional number a
                // contact was told apart from, or the name a flicker cost it —
                // so a panel can say which name the pilot had. `null` on every
                // other notice. A designator this platform issued or read,
                // never an object number.
                "was": notice.was().map(|designator| designator.to_string()),
                // 16b cut Q.2, widened by **16b cut B.1** (`T-86`): the section
                // this notice is about — the target's on the gunner's `hit` and
                // `target_section_lost`, this platform's own on the victim's
                // `struck`, `struck_spent` and `section_lost` — so a panel never
                // regexes it back out of `text`; `kind` says whose it is. The
                // section word only: the facet, the numbers and the cost of a
                // lost section stay in `sections` above and in the sentence.
                "section": notice.section().map(SmolStr::as_str),
                // **16b cut D.2** (`B-07`): the sentence's own parts. Until this
                // cut the whole of a refusal and the whole of a block lived in
                // `text`, which a panel correctly never reads (one sentence, one
                // owner) — so the web cockpit printed a bare `refused` with no
                // order, no weapon and no reason. Every key here is `null` on a
                // notice it does not apply to.
                "cause": notice.cause().map(|cause| cause.as_str()),
                "refusal": notice.refusal().map(|refusal| refusal.as_str()),
                "order": notice.order().map(SmolStr::as_str),
                "weapon": notice.weapon().map(SmolStr::as_str),
                // **16b cut O.2**: an `emcon_on`/`emcon_off` notice carries the
                // pod's radius here, so a panel composes its own line without
                // reading `text`.
                "detail": match (notice, notice.detail()) {
                    (FrameNotice::Emcon { radius_m, .. }, _) => {
                        serde_json::json!({ "radius_m": radius_m })
                    }
                    // **17b cut M1b.1**: a held slot's reason, every why's keys
                    // present and the other whys' `null`, so a panel composes the
                    // sentence without reading `text`.
                    (FrameNotice::SlotHeld { why, .. }, _) => held_detail(why),
                    (_, RefusalDetail::None) => serde_json::Value::Null,
                    (_, RefusalDetail::Cycling { secs }) => serde_json::json!({ "secs": secs }),
                    (_, RefusalDetail::Overheated { heat, max }) => {
                        serde_json::json!({ "heat": heat, "max": max })
                    }
                    // **17b cut M1b.1**: the floor an `unlawful` refusal was held to.
                    (_, RefusalDetail::Floor { permille }) => {
                        serde_json::json!({ "floor_permille": permille })
                    }
                },
                // 16b cut D.2 (`B-18`): whether the contact this notice names is
                // on the platform's own side, off the frame's own row — so a
                // panel marks a friendly lock without a table of its own. Always
                // `false` for a notice that names no contact, and for every
                // unidentified one.
                "friendly": notice
                    .contact()
                    .is_some_and(|id| !friendly_mark(frame, id).is_empty()),
                // 16b cut D.2 (`B-15`): the round found a section that was
                // already destroyed.
                "spent": notice.spent(),
                "text": text,
            });
            // Variant-specific own facts: M5r's firing cell and 17b M1b.1's
            // slot notice's own parts, on their own objects only. The own slot, the
            // chance and the facet word the row earned; the weapon and the
            // designator ride the keys above.
            if let Some(map) = object.as_object_mut() {
                match notice {
                    FrameNotice::Miss { launch_cell, .. } => {
                        // The gunner's own firing cell is still known after
                        // contact loss redacts the target and miss reason.
                        map.insert("launch_cell".to_owned(), serde_json::json!(launch_cell));
                    }
                    FrameNotice::SlotFired {
                        slot,
                        p_permille,
                        aspect,
                        ..
                    } => {
                        map.insert("slot".to_owned(), serde_json::json!(slot));
                        map.insert("percent".to_owned(), serde_json::json!(p_permille / 10));
                        map.insert("aspect".to_owned(), serde_json::json!(aspect.as_str()));
                    }
                    FrameNotice::SlotHeld { slot, .. } => {
                        map.insert("slot".to_owned(), serde_json::json!(slot));
                    }
                    _ => {}
                }
            }
            Some(object)
        })
        .collect();
    let weapons: Vec<serde_json::Value> = frame_weapons(frame)
        .iter()
        .zip(frame.weapons.iter())
        .map(|(weapon, carried)| {
            serde_json::json!({
                "id": weapon.id.as_str(),
                "ready": weapon.ready,
                "ready_in_ms": weapon.ready_in_ms,
                "rounds": weapon.rounds,
                "rounds_max": weapon.rounds_max,
                "range_band": weapon.range_band.map(|band| serde_json::json!({
                    "index": band.index,
                    "of": band.of,
                    "max_range_cm": band.max_range_cm,
                    "dispersion_mrad": band.dispersion_mrad,
                })),
                "p_permille": weapon.p_permille,
                "percent": weapon.p_permille.map(|permille| permille / 10),
                // 16b cut D.2 (`B-16`): what one shot of it costs the platform,
                // and `0` for a cold gun — which is what tells a panel why an
                // overheated tank may still fire this one.
                "heat_per_shot": weapon.heat_per_shot,
                // **17b cut M1a.3** (D-UM-4): the band edges, centimetres — what
                // the view's band word and a mind's reach arithmetic read, and
                // which the keyframe did not carry until a reader that is not a
                // person needed it.
                "bands_cm": carried.bands_cm,
                // **17b cut M1b.3** (D-UM-4): the slot this weapon is armed in,
                // or `null` — the pilot's own slot, and never a designator.
                "armed": weapon.armed.as_ref().map(arming_json),
            })
        })
        .collect();
    // **16b cut F.2**: doc 16 §5's status block, the same numbers `STATUS` prints
    // and from the same frame — so the line client, the TUI and the web draw one
    // tank (§5: *the TUI and web render the same structure; parity is a golden
    // test in each renderer*).
    let sections: Vec<serde_json::Value> = frame
        .sections
        .iter()
        .map(|section| {
            serde_json::json!({
                "id": section.id.as_str(),
                "condition": section.condition.as_str(),
                "internals": section.internals.map(|(current, _)| current),
                "internals_max": section.internals.map(|(_, max)| max),
                "facets": section.facets.iter().map(|facet| serde_json::json!({
                    "id": facet.id.as_str(),
                    "armor": facet.armor,
                    "armor_max": facet.armor_max,
                })).collect::<Vec<serde_json::Value>>(),
            })
        })
        .collect();
    let chosen = fired_by_default(&frame.weapons).and_then(|weapon| weapon.p_permille);
    // 16b cut Q.1: the lock's own name and its finer range, resolved once.
    let lock_designator = lock_designator(frame);
    // **16b cut D.4** (`T-20`): the same step `RANGE` uses, off the same row's own
    // age — one function, so the verb, the frame and the two panels' lock lines
    // cannot give two distances.
    let lock_range_m = frame_lock_range_m(frame);
    let mut payload = serde_json::json!({
        // 16b cut Q.3: `retain` tells the session layer that this keyframe
        // supersedes the ones before it on this feed — it replaces the feed's
        // entry in the session's latest slot (16b cut B.5; see
        // [`RETAIN_LATEST`]).
        "schema": { "id": FRAME_SCHEMA, "version": FRAME_VERSION, "retain": RETAIN_LATEST },
        "match": match_id.0,
        "unit": frame.unit.0,
        // 16b cut Q.1: the platform's own public name, and the arena cell it is
        // standing in — what a panel's map draws the `@` at.
        "callsign": frame.callsign.to_string(),
        "cell": frame.cell.map(|(x, y)| [x, y]),
        "x_cm": frame.pose.x_cm,
        "y_cm": frame.pose.y_cm,
        "z_cm": frame.pose.z_cm,
        "heading_mdeg": frame.pose.heading_mdeg,
        "heading_deg": frame.pose.heading_mdeg / 1_000,
        "speed_cms": frame.speed_cms,
        "speed_kph": cms_to_kph(frame.speed_cms),
        "power": frame.power.as_str(),
        // **16b cut M.3** (16c `T-04`): the Rust field is `Frame::held` now, and
        // this wire key is deliberately still `"blocked"` — `web/src/lib/cockpit.ts`
        // and the TUI renderer both read it, and renaming it is a client cut's.
        "blocked": frame.held,
        // **16b cut D.2** (`B-03`): the pilot's own two limits. A pilot's tank is
        // outside the observer rule entirely, and these are what a panel greys
        // the order controls on.
        "wreck": frame.wreck,
        "immobile": frame.immobile,
        "tempo": frame.tempo.as_str(),
        // **16b cut D.1**: the platform's own nominal sight, metres — what each
        // contact's `band` is read against ([`contact_band`]), so a panel can say
        // why a contact at 450 m reads `far`. A fact about the pilot's own
        // machine, outside the observer rule.
        "sight_m": frame.sight_cm / 100,
        // **16b cut D.1**: the pilot's own alert state ([`own_scope`]) —
        // `clear`, `unidentified` or `hostile`. `tempo` above stays, because a
        // panel's own timers read the cadence off it, but it is the arena's one
        // global word and not this platform's.
        "scope": own_scope(frame),
        "lock": frame_lock(frame).map(|id| id.to_string()),
        "lock_designator": lock_designator.map(|named| named.to_string()),
        "lock_range_m": lock_range_m,
        "lock_p_permille": chosen,
        "lock_percent": chosen.map(|permille| permille / 10),
        "contacts": contacts,
        // 16b cut Q.2: the section vocabularies the readings above are in the
        // order of, one entry per distinct chassis and only for contacts that
        // carry a reading. One name in Rust and on the wire.
        "scan_tables": frame.scan_tables,
        "notices": notices,
        "weapons": weapons,
        "sections": sections,
        // **17b cut M1a.3** (doc 17 §3 "Two more fields"): the arena millisecond
        // the frame was built at, and the hull's standing order re-keyed to its
        // own contacts — `null` on every piloted hull (`Arena::frame_order`).
        "t_ms": frame.t_ms,
        // **17b cut M1b.3** (doc 17 §5 as amended, D-UM-17): the match's
        // Compact floor, captured at check-out — a public constant.
        "odds_floor_permille": frame.odds_floor_permille,
        "rules": frame.rules.iter().map(rule_json).collect::<Vec<_>>(),
        "order": frame.order.as_ref().map(|order| order_json(frame, order)),
        // **17b cut M5.4** (doc 17 §12): the hull's sealed mind, for the
        // person in the seat — `null` on a hull no seal reached. Its classes
        // are the frame's live ones; `channels` follows the weapons block's
        // own rule (`fire` on an armed hull only).
        "mind": frame.mind.as_ref().map(|mind| mind_json(mind, armed)),
        "heat": frame.heat.map(|heat| serde_json::json!({
            "heat": heat.heat,
            "max": heat.max,
            "overheated": heat.overheated(),
        })),
        // **16b cut D.4** (`B-22`): what the platform's own receiver is running
        // under, or `null`. A fact about the pilot's own machine, with no source,
        // no bearing, no side and no object number on it
        // ([`crate::arena::perception::Receiver`]).
        "receiver": frame.receiver.map(|receiver| serde_json::json!({
            "budget": receiver.budget,
            "budget_max": receiver.budget_max,
            "reach_percent": receiver.reach_permille / 10,
        })),
        // **16b cut O.2** (16c N12): the pilot's own pod's switch, `"on"` or
        // `"off"`, and `null` for a platform that carries no pod — every stock
        // tank. The key is on every frame, as `receiver` is; only its value is
        // the pilot's own.
        "ecm": frame.ecm.map(|on| if on { "on" } else { "off" }),
        // **16b cut Y.4** (doc 07 §1.3 step 5): what happened while the pilot
        // could not see, on the first frame after a reclaim and `null` on
        // every other — the mirror stores it cleared, so a read verb's frame
        // is `null` too. The pilot's own tank and the platform's own
        // designators (`catchup_json`); both panels compose their lines from
        // it and the line client was sent the same lines as one `Sys` block.
        "catchup": frame.catchup.as_ref().map(catchup_json),
    });
    // Keep the public objective and own motor facts together. Inserting this
    // block separately keeps the large frame macro within its recursion bound.
    if let Some(object) = payload.as_object_mut() {
        // **Doc 22**: the pilot's own side and own unit type — never secrets
        // from the pilot — so a picture draws bodies in team colour and the
        // doll from the right look.
        object.insert("team".into(), frame.team.as_str().into());
        object.insert("unit_type".into(), frame.unit_type.as_str().into());
        // Doc 21 cut G.7b: the eye the pilot's own sight is traced from.
        object.insert("sensor_height_cm".into(), frame.sensor_height_cm.into());
        // **Doc 21 cut G.12a** (§7.3): the seat's master gate, and the two
        // `SPEED` ceilings in whole kph as the verb's refusal states them
        // (`reverse_kph` a magnitude) — so a client says which gate is up and
        // steps and clamps its speed keys without a round trip.
        object.insert("weapons_free".into(), frame.weapons_free.into());
        object.insert(
            "max_speed_kph".into(),
            cms_to_kph(i32::try_from(frame.max_speed_cms).unwrap_or(i32::MAX)).into(),
        );
        object.insert(
            "reverse_kph".into(),
            cms_to_kph(i32::try_from(frame.reverse_cms).unwrap_or(i32::MAX)).into(),
        );
        object.insert("mode".into(), frame.mode.as_str().into());
        object.insert(
            "objective".into(),
            frame
                .objective
                .map_or(serde_json::Value::Null, |objective| {
                    serde_json::json!({
                        "cell": objective.spot,
                        "letter": objective.letter.map(|letter| letter.to_string()),
                        "radius_cells": objective.radius_cells,
                    })
                }),
        );
        object.insert(
            "motor".into(),
            frame.motor.map_or(serde_json::Value::Null, |motor| {
                serde_json::json!({
                    "goal": motor.requested_goal,
                    "outcome": motor.outcome.map(|outcome| outcome.as_str()),
                })
            }),
        );
    }
    // **17b cut M1a.3** (doc 17 §22's italic line): *the keyframe drops the
    // weapons block on one without*. `lock`, `lock_*`, `heat`, `scan_tables`
    // and each contact's `fatal` stay: the lock is the sensors station's.
    if !armed && let Some(object) = payload.as_object_mut() {
        object.remove("weapons");
    }
    payload
}

/// **The hull's sealed mind as the keyframe carries it** (17b cut M5.4; doc 17
/// §12; the M5 design note's M5.4 item 3):
/// `{"name": "Standard Doctrine", "profiles": ["Sniper", "Line"], "active": 0,
/// "class": "I", "core": "II", "channels": ["helm", "sensors", "fire"],
/// "next_switch_ms": null, "off": false}`.
///
/// * `name` is `Standard Doctrine` for a doctrine seal and `null` for any other
///   — the arena keeps no other mind's name, and every renderer prints `your
///   own mind`;
/// * `profiles` the sealed presets in order, `active` the index flown;
/// * `class` and `core` the class words ([`crate::arena::core::CoreClass::as_str`])
///   of the frame's LIVE classes ([`FrameMind`]);
/// * `channels` the stations the pilot holds: `helm` and `sensors`, and `fire`
///   on a hull with a weapons station (`armed`, the frame's own rule);
/// * `next_switch_ms` the absolute arena millisecond the next switch is
///   allowed, `null` before any switch;
/// * `off` whether the mind is dropped to the ROM.
#[must_use]
pub fn mind_json(mind: &FrameMind, armed: bool) -> serde_json::Value {
    let mut channels = vec!["helm", "sensors"];
    if armed {
        channels.push("fire");
    }
    serde_json::json!({
        "name": mind.name,
        "card": {
            "schema": mind.card.schema(),
            "values": mind.card.values(),
        },
        "sealed_rules": mind.card.rules().unwrap_or_default().iter()
            .map(crate::arena::schema::rule_words).collect::<Vec<_>>(),
        "profiles": mind.profiles.names().collect::<Vec<&str>>(),
        "active": mind.active,
        "class": mind.class.as_str(),
        "core": mind.core_class.as_str(),
        "channels": channels,
        "next_switch_ms": mind.switch_at_ms,
        "off": mind.off,
    })
}

/// **A standing order as the keyframe carries it** (17b cut M1a.3; doc 17 §3,
/// D-UM-4): the objective's word, its payloads (`to`, `facing_mdeg`, and the
/// contacts `target` and `of` as the **designator** of the hull's own row the
/// order names, `null` where it names none), the two constraints, and the time
/// left as `expires_in_ms` — `expires_ms` less the frame's own `t_ms`, never
/// below nought — so the view carries nothing the keyframe lacks.
pub(super) fn order_json(
    frame: &Frame,
    order: &crate::arena::perception::FrameOrder,
) -> serde_json::Value {
    use crate::arena::perception::MindObjective;
    let named = |contact: Option<ContactId>| {
        contact.and_then(|id| {
            frame
                .contacts
                .iter()
                .find(|held| held.id == id)
                .map(|held| held.designator.to_string())
        })
    };
    let (target, to, facing_mdeg, of) = match order.objective {
        MindObjective::Engage { target } => (named(target), None, None, None),
        MindObjective::Advance { to } => (None, Some([to.0, to.1]), None, None),
        MindObjective::Screen { facing_mdeg } => (None, None, Some(facing_mdeg), None),
        MindObjective::Support { of } => (None, None, None, named(of)),
        MindObjective::Hold | MindObjective::Withdraw => (None, None, None, None),
    };
    serde_json::json!({
        "squad": order.squad.as_str(),
        "identity": order.identity.as_deref(),
        "objective": order.objective.as_str(),
        "target": target,
        "to": to,
        "facing_mdeg": facing_mdeg,
        "of": of,
        "hold_fire": order.hold_fire,
        "max_range_band": order.max_range_band.map(|band| band.as_str()),
        "expires_in_ms": order.expires_ms.saturating_sub(frame.t_ms).max(0),
    })
}

/// **One rectangle of terrain, run-length encoded** (16b cut Q.3), and the
/// classes it actually carries in first-seen order.
///
/// Row `0` of the answer is the **south** edge (`y0`), which is what the grid
/// shape means by `rows` and what [`map_frame`] and [`view_window`] both publish.
/// One glyph per terrain class, from the atlas's own table, so the arena's map
/// and the atlas's map read alike.
///
/// **16b cut I.1** made it `pub(crate)`: a replay's window
/// (`crate::match_::replay::replay_window`) draws its ground with this same
/// builder rather than a copy of it.
pub(crate) fn terrain_rows(
    terrain: &ArenaTerrain,
    origin: (u16, u16),
    size: (u16, u16),
) -> (Vec<serde_json::Value>, Vec<mush_atlas::TerrainClass>) {
    let (x0, y0) = origin;
    let (width, height) = size;
    let mut rows: Vec<serde_json::Value> = Vec::with_capacity(usize::from(height));
    let mut classes: Vec<mush_atlas::TerrainClass> = Vec::new();
    for y in y0..y0.saturating_add(height) {
        // Run-length, as the grid shape wants it: one pair per run of a glyph.
        let mut runs: Vec<serde_json::Value> = Vec::new();
        let mut run: Option<(char, u32)> = None;
        for x in x0..x0.saturating_add(width) {
            let class = terrain
                .get(x, y)
                .map_or(mush_atlas::TerrainClass::Void, |cell| cell.class);
            if !classes.contains(&class) {
                classes.push(class);
            }
            let glyph = class.glyph();
            match run {
                Some((held, n)) if held == glyph => run = Some((held, n.saturating_add(1))),
                Some((held, n)) => {
                    runs.push(serde_json::json!([held.to_string(), n]));
                    run = Some((glyph, 1));
                }
                None => run = Some((glyph, 1)),
            }
        }
        if let Some((held, n)) = run {
            runs.push(serde_json::json!([held.to_string(), n]));
        }
        rows.push(serde_json::Value::Array(runs));
    }
    (rows, classes)
}

/// **The whole arena's terrain, and nothing else** (16b cut Q.3; doc 16 §3.2,
/// §3.6).
///
/// The knowledge rule it rests on is the cut Q design note's, and doc 16 §3.2
/// and §3.6 are owed the sentence: *the proving ground is surveyed ground — its
/// terrain is known to every pilot at check-out, its units are not*. Terrain
/// names no unit, and `VIEW` has drawn ground behind hills since cut K.2.
///
/// There is no `units` key, no `you` key and nothing about any unit **by
/// construction**: the whole observer rule on this surface is that the map is a
/// fact about the ground. What moves goes on the cockpit frame, which is
/// per-pilot and projected; a panel client draws the markers of
/// [`status_frame`] over the rows of this one.
///
/// It is only ever the answer to a query
/// ([`crate::arena_bridge::MAP_FEED`]'s handler) and is never pushed: it is the
/// largest message the arena produces and a match's opening pulse is the least
/// reliable moment to send one, because a full per-connection output queue drops
/// an event silently and nothing retries. A client that holds a cockpit frame of
/// match M and no map of M asks, and asks again until it has one.
///
/// **Total, and within [`MAP_MAX_BYTES`] for every loaded arena** (16b cut U.4):
/// the bound is checked when the arena is applied, over this very function at
/// the widest `match` and `surface` the handler can send, so an arena whose map
/// would pass it never loads and nothing here needs to refuse.
///
/// **The second thing the map may say: the named spots** (16b cut V.1). One key
/// after `legend`, `spots`, is `[{ "letter": "A", "name": "The Ford", "cell":
/// [64, 97] }, …]` — the arena's authored `key_spots` in authored order, in
/// arena cells, each with the marker letter [`spot_letter`] gives its index. They
/// are surveyed ground exactly as the terrain is (doc 16 §3.2: known to every
/// pilot at check-out), and they name no unit. A spot past [`SPOT_LETTERS`] —
/// which the load-time cap refuses — is left out rather than drawn with no
/// letter. [`MAP_VERSION`] stays `1`: a field added is not a version.
#[must_use]
pub fn map_frame(
    match_id: MatchId,
    surface: &str,
    terrain: &ArenaTerrain,
    spots: &[KeySpot],
) -> serde_json::Value {
    let (rows, classes) = terrain_rows(terrain, (0, 0), (terrain.width(), terrain.height()));
    let legend: Vec<serde_json::Value> = classes
        .iter()
        .map(|class| serde_json::json!([class.glyph().to_string(), class.label()]))
        .collect();
    let spots: Vec<serde_json::Value> = spots
        .iter()
        .zip(SPOT_LETTERS)
        .map(|(spot, letter)| {
            serde_json::json!({
                "letter": letter.to_string(),
                "name": spot.name.as_str(),
                "cell": [spot.x, spot.y],
            })
        })
        .collect();
    serde_json::json!({
        "schema": {
            "id": MAP_SCHEMA,
            "version": MAP_VERSION,
            "shape": MAP_SHAPE,
            "retain": RETAIN_LATEST,
        },
        "match": match_id.0,
        "surface": surface,
        "biome": terrain.biome(),
        "cell_cm": terrain.cell_cm(),
        "width": terrain.width(),
        "height": terrain.height(),
        // Always the arena's own corner today. It is in the payload because it
        // is what a windowed map would carry: doc 16 §11's 10 km path would
        // answer a `window` parameter with a slice of the same shape, and a
        // renderer written against this key keeps working when it does.
        "origin_cell": [0, 0],
        "rows": rows,
        "legend": legend,
        "spots": spots,
        // Doc 21 cut G.7b: the ground's relief, so the picture draws the
        // hills the sim already sees with (a field added, not a version).
        "elev": elev_field(terrain),
    })
}

/// The encoding [`elev_field`] writes.
pub const ELEV_ENCODING: &str = "zigzag-leb128-delta";

/// **The terrain's elevations for the map frame** (doc 21 cut G.7b): every
/// cell's `elev_cm` in the order `rows` lists them (the south row first, west
/// to east), each as its difference from the cell before it (the first from
/// zero), zigzag-folded and written as a LEB128 varint, the bytes base64'd.
/// Ground that rolls costs about a byte a cell; flat ground compresses to
/// runs of zero bytes. `{"encoding", "unit_cm", "data"}`, `unit_cm` always 1.
#[must_use]
pub fn elev_field(terrain: &ArenaTerrain) -> serde_json::Value {
    let mut bytes: Vec<u8> = Vec::with_capacity(terrain.cells().len());
    let mut last: i32 = 0;
    for y in 0..terrain.height() {
        for x in 0..terrain.width() {
            let elev = i32::from(terrain.get(x, y).map_or(0, |cell| cell.elev_cm));
            let delta = elev - last;
            last = elev;
            let mut zig = ((delta << 1) ^ (delta >> 31)) as u32;
            loop {
                let low = (zig & 0x7f) as u8;
                zig >>= 7;
                if zig == 0 {
                    bytes.push(low);
                    break;
                }
                bytes.push(low | 0x80);
            }
        }
    }
    serde_json::json!({
        "encoding": ELEV_ENCODING,
        "unit_cm": 1,
        "data": mush_atlas::cells::base64_encode(&bytes),
    })
}

/// **The answer to an `arena.map` query from somebody who is not flying**
/// (16b cut Q.3): the schema, and the one fact that is theirs to have.
///
/// Nothing about any match: not its id, not its ground, not whether one is
/// running. A bystander, a staffer and a pilot who has typed `EXIT` read the
/// same two keys.
#[must_use]
pub fn map_unseated() -> serde_json::Value {
    serde_json::json!({
        "schema": {
            "id": MAP_SCHEMA,
            "version": MAP_VERSION,
            "shape": MAP_SHAPE,
            "retain": RETAIN_LATEST,
        },
        "seated": false,
    })
}

/// **The empty cockpit frame** (16b cut Q.3) — what a panel is sent when its
/// pilot stops being one, and what an `arena.cockpit` query answers anybody who
/// is not flying.
///
/// The bridge sends it once through the ordinary output path when a pilot is
/// unbound (`EXIT`, a released seat, the match ending, a void). It prints
/// nothing on a line client, because it declares no shape; a panel clears; and —
/// because it carries [`RETAIN_LATEST`] like every frame before it — it
/// replaces the live frame in that session's per-feed latest slot, the one
/// `arena.cockpit` frame a resume replays, kept beside the ring rather than in
/// it (16b cut B.5), UNTIL the pilot is seated again, when the next match's
/// first frame replaces it in turn — which is why a panel resets its own
/// notices ring on a change of match and never relies on this frame arriving
/// (16b cut B.2).
#[must_use]
pub fn cockpit_cleared() -> serde_json::Value {
    serde_json::json!({
        "schema": { "id": FRAME_SCHEMA, "version": FRAME_VERSION, "retain": RETAIN_LATEST },
        "seated": false,
    })
}

/// **The cockpit keyframe with the window in it** — [`status_frame`] plus the
/// `grid` shape `mush_ansi::panel` renders as a text map and the web draws as a
/// map panel (doc 16 §5: *the ANSI line renderer prints the frame as text; the
/// TUI and web render the same structure*).
///
/// **16b cut Q.3** split the window itself out into [`view_window`], which is a
/// pure function of a frame and a terrain: it is what the canonical fixtures
/// call, and what makes the three renderers' compositions comparable. All this
/// adds is the two things a window cannot know without a world — which ground
/// the match is being fought over, and what the pilot's own tank is called.
#[must_use]
pub fn view_frame(ctx: &Ctx<'_>, match_id: MatchId, frame: &Frame) -> serde_json::Value {
    view_frame_held(ctx, match_id, frame, &BTreeMap::new())
}

/// [`view_frame`] with each contact's range held against `ranges` (16b cut B.4,
/// `T-60`) — what `VIEW` pushes off the mirror, so its payload's rows agree with
/// the frame the pilot was last sent.
#[must_use]
pub fn view_frame_held(
    ctx: &Ctx<'_>,
    match_id: MatchId,
    frame: &Frame,
    ranges: &BTreeMap<ContactId, u64>,
) -> serde_json::Value {
    let payload = crate::detachment::commands::enrich(
        ctx.w,
        ctx.actor,
        status_frame_held(match_id, frame, ranges),
    );
    let Some(ground) = ctx
        .w
        .module_state::<ArenaRegistry>()
        .and_then(|registry| registry.arena_of(match_id))
    else {
        return payload;
    };
    let mut payload = view_window(
        payload,
        ground.id.as_str(),
        &ground.terrain,
        &ground.key_spots,
        frame,
    );
    // The pilot's own tank's name, which is theirs to have and which a pure
    // window has no world to ask for: [`view_window`] leaves the key out rather
    // than inventing a value the live feed does not send, and this is the one
    // place it is filled in. The whole payload is byte for byte what it was
    // before cut Q.3 apart from `schema.retain`.
    //
    // **16b cut V.1**: the own marker is found by its `you` flag. It used to be
    // `units.first()`, and the first marker is now a spot whenever the window
    // holds one.
    if let Some(you) = payload
        .get_mut("units")
        .and_then(serde_json::Value::as_array_mut)
        .and_then(|units| {
            units
                .iter_mut()
                .find(|unit| unit.get("you").and_then(serde_json::Value::as_bool) == Some(true))
        })
        .and_then(serde_json::Value::as_object_mut)
    {
        you.insert(
            "name".to_owned(),
            serde_json::json!(display_name(ctx.w, frame.unit)),
        );
    }
    payload
}

/// **The arena window, as a pure function of the frame and the ground** (16b cut
/// Q.3).
///
/// `payload` is a [`status_frame`] this widens in place: the `schema` object is
/// **replaced** by one carrying `shape: "grid"` — and [`RETAIN_LATEST`] with it,
/// or a typed `VIEW` would re-create the ring flood one frame at a time — and
/// the window's own keys are added beside the cockpit's.
///
/// The window is [`VIEW_COLUMNS`] by [`VIEW_ROWS`] cells around the platform,
/// clamped to the arena: the most a line client prints at
/// [`VIEW_CELL_COLUMNS`] columns a cell (16b cut B.4), so the renderer only has
/// to clip it to a narrow terminal and never has to ask for more. A pose off the
/// window answers the payload untouched.
///
/// **The marker order is the rule every compositor follows** — since **16b cut
/// V.1**: the named spots inside the window in authored order
/// ([`spot_markers`]), then the pilot's own marker, then the contacts in frame
/// order, a later marker overwriting an earlier one on the same cell (which is
/// what `mush_ansi::panel::expand_rows` does with `units`, and what the TUI and
/// the web must do too). So a contact standing on a spot hides its letter — the
/// spot is ground and the contact is what the pilot holds — and so does the
/// pilot's own `@`.
///
/// **The naming order is the reverse of the drawing** (16b cut V.1). `legend`
/// is the terrain classes, then `["@", "you"]`, then the contact glyph labels,
/// and THEN `["A", "The Ford"]` for each spot that **owns its cell** — that no
/// later marker landed on, computed here after all three groups are placed. A
/// covered spot is still in `units` (the grid renderer draws in order and the
/// later write wins, which is the rule) but is named nowhere, so the line
/// client's, the TUI's and the web's legends name the same spots. Contacts come
/// first because a narrow legend is cut from the back — the TUI's `fit_legend`
/// keeps entries from the front and a zoomed canvas's `labelPlan` places labels
/// in order — and a narrow pane must keep `x BA` before the name of a field.
///
/// The own marker carries **no `name`**: a window has no world, and the name the
/// live feed sends there is the tank's display name, which only [`view_frame`]
/// can ask for. An absent key is honest; a callsign in its place would teach
/// every renderer written against the fixtures a value the server never sends.
#[must_use]
pub fn view_window(
    payload: serde_json::Value,
    surface: &str,
    terrain: &ArenaTerrain,
    spots: &[KeySpot],
    frame: &Frame,
) -> serde_json::Value {
    let mut payload = payload;
    let Some((cx, cy)) = terrain.cell_of(&frame.pose) else {
        return payload;
    };
    let width = VIEW_COLUMNS.min(terrain.width());
    let height = VIEW_ROWS.min(terrain.height());
    let x0 = cx
        .saturating_sub(width / 2)
        .min(terrain.width().saturating_sub(width));
    let y0 = cy
        .saturating_sub(height / 2)
        .min(terrain.height().saturating_sub(height));
    let (rows, classes) = terrain_rows(terrain, (x0, y0), (width, height));
    let mut legend: Vec<serde_json::Value> = classes
        .iter()
        .map(|class| serde_json::json!([class.glyph().to_string(), class.label()]))
        .collect();
    legend.push(serde_json::json!([YOU_GLYPH, "you"]));
    // 16b cut V.1: the named spots inside the window go down FIRST, then the
    // platform at the centre, then one marker per contact. Cells are relative to
    // the window's own origin, which is what the grid renderer centres on.
    let (spot_units, spot_legend, _drawn) = spot_markers(spots, (x0, y0), (width, height));
    let spot_count = spot_units.len();
    let you = [
        u32::from(cx.saturating_sub(x0)),
        u32::from(cy.saturating_sub(y0)),
    ];
    let mut units = spot_units;
    units.push(serde_json::json!({
        "id": format!("#{}", frame.unit.0),
        "cell": you,
        "glyph": YOU_GLYPH,
        "you": true,
    }));
    let (markers, glyphs) = contact_markers(frame, (x0, y0), (width, height));
    units.extend(markers);
    for entry in glyphs {
        if !legend.contains(&entry) {
            legend.push(entry);
        }
    }
    // The spots are NAMED last, and only those that own their cell: a spot
    // marker any later marker (the `@`, a contact, or a later spot) landed on is
    // drawn and then covered, so it is named nowhere — the owner rule the TUI's
    // and the web's legends apply, applied here once every marker is placed.
    for (index, entry) in spot_legend.into_iter().enumerate().take(spot_count) {
        let cell = units.get(index).and_then(|unit| unit.get("cell"));
        let covered = units
            .iter()
            .skip(index.saturating_add(1))
            .any(|later| later.get("cell") == cell);
        if !covered {
            legend.push(entry);
        }
    }
    if let Some(object) = payload.as_object_mut() {
        object.insert(
            "schema".to_owned(),
            serde_json::json!({
                "id": FRAME_SCHEMA,
                "version": FRAME_VERSION,
                "shape": "grid",
                "retain": RETAIN_LATEST,
            }),
        );
        object.insert("title".to_owned(), serde_json::json!("Arena window"));
        object.insert("surface".to_owned(), serde_json::json!(surface));
        object.insert("width".to_owned(), serde_json::json!(width));
        object.insert("height".to_owned(), serde_json::json!(height));
        object.insert("cell_cm".to_owned(), serde_json::json!(terrain.cell_cm()));
        object.insert(
            "origin".to_owned(),
            serde_json::json!([
                i64::from(x0) * i64::from(terrain.cell_cm()),
                i64::from(y0) * i64::from(terrain.cell_cm())
            ]),
        );
        object.insert("rows".to_owned(), serde_json::Value::Array(rows));
        object.insert("legend".to_owned(), serde_json::Value::Array(legend));
        object.insert("units".to_owned(), serde_json::Value::Array(units));
        object.insert("you".to_owned(), serde_json::json!(you));
        // **16b cut B.4** (`B-24`, the `T-60` footer slip): three keys for the
        // grid renderer. `cell_columns` — a line client prints each cell this
        // many columns wide, so the ground is square; `origin_cell` and
        // `extent` — the window's corner and the arena's size in arena cells,
        // so the footer names arena cells and not window cells. The size is
        // lobby knowledge (doc 16 §3.2: the proving ground is surveyed ground)
        // and the corner is read off the pilot's own pose; neither is a fact
        // about any other unit.
        object.insert(
            "cell_columns".to_owned(),
            serde_json::json!(VIEW_CELL_COLUMNS),
        );
        object.insert("origin_cell".to_owned(), serde_json::json!([x0, y0]));
        object.insert(
            "extent".to_owned(),
            serde_json::json!([terrain.width(), terrain.height()]),
        );
    }
    payload
}

/// **The one honest line `VIEW` owes a pilot whose scope is wider than their
/// terminal** (16b cut Q.3; doc 16 §3.6).
///
/// A sensor reaches sixty cells every way and the line client's window is
/// [`VIEW_COLUMNS`] by [`VIEW_ROWS`], because a terminal is eighty by
/// twenty-four and each cell is printed two columns wide (16b cut B.4: a
/// close-in window, so this line is what covers the rest of the sight). A contact whose plot falls outside that window — or off the
/// arena entirely, which is a plot with no cell — is on `CONTACTS` and not on
/// the map, and the pilot is told so rather than left to conclude the scope is
/// empty. `None` when every held contact is drawn.
///
/// `window` is the payload [`view_window`] just produced: its `units` are the
/// markers that were drawn, so the count is the drawing's own and cannot drift
/// from it. A payload with no window at all owes nothing. **16b cut V.1**: a
/// marker is a contact only when it is neither the pilot's own (`you`) nor a
/// named spot (`spot`); the spots have their own line, [`off_window_spots_line`].
#[must_use]
pub fn beyond_window_line(frame: &Frame, window: &serde_json::Value) -> Option<String> {
    let flag = |unit: &serde_json::Value, key: &str| {
        unit.get(key).and_then(serde_json::Value::as_bool) == Some(true)
    };
    let drawn = window
        .get("units")
        .and_then(serde_json::Value::as_array)?
        .iter()
        .filter(|unit| !flag(unit, "you") && !flag(unit, "spot"))
        .count();
    match frame.contacts.len().saturating_sub(drawn) {
        0 => None,
        1 => Some("1 contact plots beyond this window; CONTACTS lists it.".to_owned()),
        many => Some(format!(
            "{many} contacts plot beyond this window; CONTACTS lists them."
        )),
    }
}

/// **The contact markers a window carries, and the legend entries they earn**
/// (16b cut S.2), clipped to the window `origin` and `size` name and with cells
/// relative to the window's own corner.
///
/// Split out of [`view_frame`] because it is the one part of a window that is a
/// pure function of the frame — the terrain, the pilot's own marker and the
/// display name all need a world, and the observer rule on the map is entirely
/// about what goes in *here*.
///
/// **The observer rule on this surface.** A marker carries a cell, a glyph, the
/// class word as its `name`, and — since **16b cut Q.1** — the short designator
/// as its `id`: the grid shape's `id` is display text (the atlas's own markers
/// carry `#5` in it), so a panel that labels a marker labels it with the word the
/// pilot types and never with the sixteen digits. No object number, no unit name,
/// nothing the platform has not resolved.
///
/// The legend gains a glyph only when the window actually carries one: a legend
/// that always advertised `hostile` would tell a pilot with an empty scope what a
/// full one looks like, which is one more thing than they hold.
#[must_use]
pub fn contact_markers(
    frame: &Frame,
    origin: (u16, u16),
    size: (u16, u16),
) -> (Vec<serde_json::Value>, Vec<serde_json::Value>) {
    let (x0, y0) = origin;
    let (width, height) = size;
    let mut markers: Vec<serde_json::Value> = Vec::new();
    let mut legend: Vec<serde_json::Value> = Vec::new();
    for row in frame_contacts(frame) {
        let Some((mx, my)) = row.cell else {
            continue;
        };
        if mx < x0 || my < y0 {
            continue;
        }
        let (dx, dy) = (mx.saturating_sub(x0), my.saturating_sub(y0));
        if dx >= width || dy >= height {
            continue;
        }
        markers.push(serde_json::json!({
            "id": row.designator.to_string(),
            "name": row.class.as_str(),
            "cell": [u32::from(dx), u32::from(dy)],
            "glyph": row.glyph(),
            "you": false,
        }));
        let label = match row.glyph() {
            FRIEND_GLYPH => "friendly",
            HOSTILE_GLYPH => "hostile",
            _ => "contact",
        };
        let entry = serde_json::json!([row.glyph(), label]);
        if !legend.contains(&entry) {
            legend.push(entry);
        }
    }
    (markers, legend)
}
