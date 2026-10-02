//! The pilot (cut AJ.2; cut AK.1): a deterministic tactician over the
//! [`Picture`].
//!
//! [`Pilot::decide`] reads the picture and a [`Doctrine`] and says what to type
//! next — a heading, a speed, a lock, the guns to fire, whether to ask where The
//! Crossroads lies. No I/O and no clock: `now_ms` is the runner's session clock,
//! and the lane decides which of the orders go out this tick and tells the pilot
//! ([`Pilot::sent_heading`], [`Pilot::sent_speed`], [`Pilot::sent_lock`]). What
//! it did in words ([`Pilot::take_notes`]) the lane hands the runner as `arena`
//! events.
//!
//! **Written from the player help alone.** Nothing of the machines' own
//! controller is read, imported or mirrored. Every number below is a sentence
//! of `help simulator` ("Fighting the machines", "The ground") or `help cockpit`
//! ("Driving", "Seeing", "Shooting", "Refusals"), or a reading of the bot's own
//! transcripts:
//!
//! * a machine "closes: to about seven tenths of what its own sight and its
//!   gun's inner band allow (420 m for the stock tank), where it stops, turns
//!   onto you and shoots" — so the pilot does not stand and trade at that range;
//! * "on grass a stopped tank can be seen out to 80 per cent of that (480 m)
//!   and a moving one out to all of it" (600 m) — so the default band,
//!   **450 m**, sits just outside the machines' stand-off and inside the
//!   stopped-tank sight: the pilot keeps a stopped machine in view. The
//!   machines stop to shoot, so the stopped tank's 480 m, less a margin for the
//!   scope's 50 m rounding and the tank's own drift, is the **ceiling** of every
//!   band the pilot flies ([`SIGHT_BAND_MAX_M`], 450 m), the fall back's
//!   included: the moving sight's 600 m is where it loses them;
//! * **"The facet you show decides what a hit costs you"** (`help cockpit`,
//!   "Seeing"): "from within 45 degrees of your bow it strikes the front", "a
//!   side is twice the target, 7 m of flank against 3.5 m of bow", the stock
//!   hull carries 400 in front, 250 a side and 150 behind, and a side shot
//!   lands on that side's track three times in ten — "Turn into the shot". So
//!   the pilot's default engagement (cut AL.2, [`Stance::Hold`]) **stops at the
//!   band with its bow on the hostile** and changes range only **along the
//!   line of fire**: ahead to close, in reverse to open ("a negative speed
//!   reverses"; "the stock tank makes 54 kph forward and 27 astern"). The bow
//!   is held within [`HOLD_BOW_DEG`] of the hostile, inside the help's 45°
//!   front arc. The orbit showed the machine a side the whole fight: of the
//!   hits the bot took in its own measured run, hull left 97, hull right 80,
//!   turret left 64 and right 60, the tracks 61 and 58, and the front only 14
//!   (hull) and 9 (turret). The range moves only past the band's slack and a
//!   hysteresis of half the scope's 50 m rounding ([`HOLD_HYSTERESIS_M`]), so a
//!   machine at the help's 420 m, read `400 m`, never sets the tank reversing
//!   and back;
//! * a side or rear hit turns the bow to the shooter ([`THREAT_TURN_MS`]): the
//!   struck facet is the help's 45-degree face (`left` 225..=315 degrees
//!   clockwise from the bow, `right` 45..=135, `rear` 135..=225), and a hostile
//!   is in that arc when its own row's relative bearing (`rel`, read every
//!   second) lies in it; the hit line says nothing of who fired, so with none
//!   in the arc the bow turns to the arc's centre, and with only the held
//!   target to blame it is taken as the shooter (the hold already faces it).
//!   A hit read after its turn would be over, one with nothing to blame while
//!   the current turn is not yet read by `STATUS`, and with one track gone one
//!   whose turn goes the dead way turn nothing; the turn's sentence is said
//!   when it is flown, never for a turn a held stop or a withdrawal outranks;
//! * the **orbit** is a stance a commander or `[arena] engagement = "orbit"`
//!   asks for, no longer the default: it circles toward the target's **rear**
//!   when the picture holds a track on it ("one caught from behind spends its
//!   thinnest" plate), and without a track **reverses** every 20 s. (That this
//!   spoils the machines' aim was not borne out: every one of the bot's own
//!   113 misses in its measured run read `the round went wide`, none `the
//!   target moved`.)
//! * "Lose one track and the tank is slowed and will not turn that way" — so
//!   with one track gone ([`live_turn`]) every turn the pilot makes on its own
//!   (the held turn, the stall's, the sweep's legs, the detour round held
//!   ground) goes the live way, and an orbit circles the way that turns the
//!   live way (a named change to the orbit, with one track gone only). Two
//!   signs are kept apart: the orbit's circling sense ([`Pilot::sense`], `+1`
//!   counter-clockwise round the target) and the compass turn sign
//!   ([`Pilot::turn_sign`], `+1` clockwise, the heading growing). The bot's own
//!   transcript shows the server turning a one-track tank the long, live way
//!   when ordered a heading on the dead side, about 30° a second, so a small
//!   correction toward the dead side is a whole circle: a heading whose turn
//!   from the [`Pilot::reference_heading`] is toward the dead side by at most
//!   [`ONE_TRACK_DEADBAND_DEG`] (the help's front arc) is not ordered at all —
//!   with one track gone the hold's bow may stand up to that far off the
//!   target on the dead side, still inside the front arc — and a larger one is
//!   ordered once and then no heading goes until `STATUS` reads it or
//!   [`ONE_TRACK_COMMIT_MS`] (a whole circle at that rate) has passed. "Or
//!   reverses to turn" is not built: how a one-track tank reverses is
//!   unobserved;
//! * "`SPEED` says both when you ask for more than it has" — so the first speed
//!   order is `999`, which reads the tank's own ceilings; until they are known
//!   "full ahead" is 40 kph, under the stock tank's 41 on grass;
//! * "Three seconds of holding a return is enough to tell it apart" whatever is
//!   locked — so an unidentified contact is never locked, and never fired on;
//! * the `% to hit` is printed only for the lock and the band word is the
//!   scope's ("It is **not** a judgement of your gun") — so the target is the
//!   nearest hostile with a fresh plot, and a held lock is kept unless another
//!   is at least 100 m nearer;
//! * "Past a gun's last band the chance is 0%: the round is still fired and
//!   spent" — so a gun whose last read said 0% is not fired;
//! * **each gun fires on its own clock** ([`super::picture`]'s module head): the
//!   seconds of `cycling (N s)` and `Still cycling: N s to go.` are rounded up,
//!   a row read inside the first second after the ack prints the whole cycle,
//!   and the shot goes on the arena's next 250 ms lattice point after the ack
//!   (all read from the bot's own transcripts; "you and the arena are a pulse
//!   apart"), so a gun is believed ready only when its one clock,
//!   [`super::picture::Weapon`]'s `next_ready_ms`, says so, or — with no clock
//!   yet — when its row says `ready` well after the ack;
//! * a gun the tank says it does not carry, a gun whose rounds the picture
//!   counts down to none ([`super::picture::Weapon::rounds_left`]), and every
//!   gun while a lost section's `its weapons are gone` waits for `WEAPONSTATUS`
//!   are not fired (cut AL.1; the bot's own transcripts typed `FIRE` at guns
//!   lost with the turret for seconds);
//! * "Ground a tank cannot enter stops it, and the cockpit says so once per
//!   stop", and "`STATUS` carries `HELD` until you order something that frees
//!   it" — so a `Held:` notice turns the tank 45° for 3 s and orders the speed
//!   again (insurance: no transcript shows a `SPEED` freeing a held tank), and a
//!   tank `STATUS` still reads held at 0 kph is turned further **from the
//!   heading it was stopped on** (90°, 135°, 180°, each judged a whole
//!   [`STALL_MS`] later, above the 3.1 s p90 and 4.0 s longest lag the bot's
//!   own transcript shows from `Steering to D°.` to a `STATUS` reading D), and
//!   at last backs off ("a negative speed reverses", `help cockpit`). A tank
//!   the pilot itself stopped or reverses is never "stalled": the watchdog
//!   opens only after a forward speed of the pilot's own choosing;
//! * held ground is **remembered** (cut AL.2, [`HeldMark`]): the bot's own
//!   transcript shows the search driving back into one slope ten times, 12 s
//!   apart. A stop on ground — `too steep`, `will not take the tank` or `the
//!   edge`, never `something is in the way` (another unit, which moves) —
//!   marks the point [`HELD_MARK_AHEAD_M`] ahead of where the tank stopped
//!   (behind it, when it stopped reversing), and a forward heading whose next
//!   [`HELD_MARK_NEAR_M`] passes within [`HELD_MARK_HALF_WIDTH_M`] of a mark
//!   more than [`HELD_MARK_CLEAR_M`] off is turned aside by
//!   [`HELD_MARK_DETOUR_DEG`] (up to three times) the pilot's turn way. A mark
//!   is **a point, not a bank**: a detour can meet the same bank further
//!   along, and that is a new stop and a new mark. A stop while reversing (a
//!   guard: no transcript holds one) runs no forward held turn — the tank
//!   stops with its bow kept — and a reverse whose ray behind the tank meets a
//!   mark stops instead;
//! * "Hurt badly enough it breaks off for its own entry zone", "Open the range
//!   past its sight and it comes after you again", and "A machine that loses you
//!   goes to where it last saw you" — so a lost hostile is **hunted**: to its
//!   last plot, then along its estimated course (its track, or the way the
//!   range was opening) while the plot is young, then one bounded sweep, and
//!   only then the search. "The proving ground is 2.56 km square" and `STATUS`
//!   counts `at {x} m east, {y} m north` from its south-west corner (the bot's
//!   own transcripts read a tank on cell (100, 97) at (1005, 974)), so no hunt
//!   aim ever lies past the edge, and the course leg, driven on along the
//!   course once level with its moving aim, ends where the course's next
//!   100 m would cross the edge or once it is that far past the aim; a wreck
//!   is never hunted, targeted or shot at (its row stays listed until it
//!   expires);
//! * **"Stop before you are found"** (cut AL.3; `help cockpit`, "Seeing": "A
//!   tank on the move is seen from further away than a tank standing still …
//!   on grass a stopped tank can be seen out to 80 per cent of that (480 m)
//!   and a moving one out to all of it; bare ground stops at 90 per cent; …
//!   heavy woods stop at 50 per cent (300 m) and move at 80"). No ground lets
//!   a mover hide better than a stopped tank, so the pilot stops wherever it
//!   is, and these stops read no ground (only the opening, below, reads its
//!   own brief). A **return not yet told apart** (a row read, its
//!   plot fresh) is faced, stopped, for at most [`UNIDENTIFIED_WAIT_MS`] from
//!   when it was first seen ("Three seconds of holding a return is enough to
//!   tell it apart"), with a rest as long between two such stops and at most
//!   [`UNIDENTIFIED_STOPS_MAX`] a match; a return lost or told apart is
//!   forgotten, so one found again — "If you lose the contact and find it
//!   again it is an unknown return once more, so it reads `X2` again" — is
//!   first seen afresh, and stopped for again. A **hunted hostile the pilot has not
//!   hurt and that is not going away**, expected back inside
//!   [`SEEN_SECOND_M`], is waited for, stopped and facing its estimate, for
//!   [`AMBUSH_WAIT_MS`] once a trail: the help's machine "drives at where it
//!   last saw you", and the pilot used to arrive moving and be seen first
//!   (every hunt of AK.3's runs began 586 to 608 m from the last plot). One
//!   the pilot has hurt (the stream said `{d}'s {section} is destroyed.`) is
//!   chased at once — "Hurt badly enough it breaks off for its own entry zone
//!   … on the way home it turns and fights only what is close (inside about
//!   200 m)", and every hunted machine in AK.3's runs was one whose turret the
//!   bot had destroyed — and so is one whose trail's track was going away or
//!   across when the hunt began (judged once, there: a tank at or past the
//!   plot has the bearing to it turned round). **Any hit taken ends a wait**
//!   for good: the tank has been found;
//! * **The opening** (cut AN.2; [`OpeningStage`], `[arena] opening = "wait"`,
//!   the default): "on grass a stopped tank can be seen out to 80 per cent of
//!   that (480 m) and a moving one out to all of it; … a road hides nothing
//!   either way" and "Stop before you are found" (`help cockpit`, "Seeing");
//!   "two sides fly stock tanks against each other on the proving ground"
//!   (`help simulator`), and a machine "that has seen nobody for a minute and
//!   a half crosses to the other side's entry zone to look". So in a skirmish
//!   (the board's rule, [`super::picture::Picture::match_mode`]) whose own
//!   `Power online.` the pilot read, from the muster it then stood at
//!   ([`Pilot::muster_m`]), with nothing but teammates on its first
//!   `CONTACTS` (cut AO.1), it types `VIEW BRIEF` once (and once more after
//!   each move in for room, cut AO.1, below), steps off the road onto ground
//!   that hides a stopped
//!   tank ([`HIDING`]) past that ground's true edge — never toward the
//!   machines — and holds there stopped, bow toward the other side's entry
//!   ([`other_entry`]: the muster mirrored, the pilot's inference from the two
//!   sides flying identical tanks from opposite ends; no contact bearing is
//!   used), until a hostile is in sight, a hit or [`OPENING_MAX_MS`]. A
//!   return not yet told apart does not end it;
//! * **Fire from the halt** (cut AN.2; [`Halt`]): the machine "closes: to
//!   about … 420 m …, where it stops, turns onto you and shoots" (`help
//!   simulator`), and [`Pilot::hold`] closes past the band, its slack and its
//!   hysteresis (525 m at 450) while a mover is seen to 600 m. So a hostile
//!   first seen inside the stock sight while the pilot waits (the opening, or
//!   the re-approach's wait) is fired on with the tank stopped, bow on, until
//!   it stops coming on (its row's range not shortening and no track published
//!   for [`HALT_STILL_MS`]), comes to the band, finds the tank or
//!   [`HALT_MAX_MS`] passes: the stopped tank lets it come. A row past the
//!   stock sight is a teammate's report and arms no halt;
//! * **The re-approach** (cut AN.2; [`Reapproach`], `[arena] reapproach`, on
//!   by default): "Open the range past its sight and it comes after you
//!   again; let the ground hide you and it drives at where it last saw you"
//!   (`help simulator`). A stopped machine (no published track) inside the
//!   stock sight that out-hits the pilot over [`EXCHANGE_WINDOW_MS`] is backed
//!   away from bow on, steering on its last plot (the pilot is blind between
//!   480 and 600 m of a stopped machine: "a stopped tank can be seen out to …
//!   480 m and a moving one out to all of it"), to [`REAPPROACH_OPEN_M`] from
//!   that plot, and then waited for, stopped and facing it, for
//!   [`REAPPROACH_WAIT_MS`]; whatever comes on inside the stock sight in that
//!   wait is fired on from the halt. At most [`REAPPROACH_PER_MATCH`] a match, a rest of
//!   [`REAPPROACH_REST_MS`] after each;
//! * **The broken machine** (cut AN.3; [`Follow`], [`Return`], `[arena]
//!   follow_broken`, on by default): "Hurt badly enough it breaks off for its
//!   own entry zone — but it is not out of the match: on the way home it turns
//!   and fights only what is close (inside about 200 m) and only while it has
//!   armour left to do it with, … once home it shoots back at whatever comes
//!   for it, and after twenty quiet seconds with nothing hostile in sight it
//!   turns round and comes back in, once" (`help simulator`). A live hostile
//!   held young that the stream said lost a section ([`Pilot::hurt`]) or that
//!   the pilot has hit [`BROKEN_HITS_MIN`] times this match
//!   ([`Pilot::scored_on`]), publishing a track (the picture publishes one
//!   only for a mover) whose course lies within [`BROKEN_AWAY_DEG`] of the
//!   bearing from the own tank to it, is **broken** (`Pilot::broken`): it is
//!   driving home. It is **followed** bow on at [`FOLLOW_BAND_M`] — outside the
//!   help's 200 m and inside the stopped tank's 480 m — and fired on from
//!   behind, but never into its entry zone: **the help names the entry zone
//!   but not its shape**, so the pilot keeps out of a band across the whole
//!   approach axis, a line through the other side's entry ([`other_entry`])
//!   square to the axis from the own muster to it, and stops the follow once
//!   the machine is within [`ENTRY_KEEP_OUT_M`] of that line or the own tank
//!   within that and the band. Then it **backs out of the machine's sight**,
//!   bow on its last plot, to [`RETURN_STANDOFF_M`] from that plot (past the
//!   600 m a mover is seen at), and **waits** there, stopped and facing it, up
//!   to [`RETURN_WAIT_MS`] for the one return: what comes on inside the stock
//!   sight is fired on from the halt; a hit ends the wait; at its time the
//!   trail of the machine is dropped and the search flies (a hunt would drive
//!   at its last plot, near the entry line, and on into the entry zone — the
//!   drive the follow refuses). A followed machine lost goes the same way, to
//!   the backing off; one that publishes no track for [`FOLLOW_STOPPED_MS`]
//!   has stopped, and the stance flies again; one no longer the target (the
//!   pilot has turned to another hostile) is no longer followed, and the stance
//!   flies for the new target. A machine that breaks while the pilot is
//!   opening the range on it (the re-approach) or firing from the halt at it
//!   is followed at once, ending either (a halt at another hostile holds the
//!   follow off): the machine is no longer stopped or
//!   coming on, and the hunt that would follow either is the drive into the
//!   entry zone. With no muster known
//!   ([`Pilot::muster_m`]: a match joined running) there is no entry line, so
//!   nothing is followed;
//! * **Off the wall** (cut AO.1; under the knobs that exist, so
//!   [`Pilot::new`] is unchanged). The bot's own runs backed into their own
//!   edge (`Held: this is the edge of the arena.`) every re-approach, from an
//!   opening held 15 to 67 m off it. **The opening's room**: with a back-off
//!   knob on, the opening holds with [`OPENING_ROOM_M`] of ground a tank
//!   crosses ([`PASSABLE`]) behind it, read from its own brief
//!   ([`opening_room`]); from a muster with less, it first drives in along
//!   the line to the other side's entry ([`opening_inward`],
//!   [`OpeningStage::Inward`]), stops, reads a second `VIEW BRIEF` there (at
//!   most [`OPENING_INWARD_LEGS_MAX`] legs), and only then steps off the road
//!   and holds. **The back-offs** (the re-approach's and the return's, not
//!   the hold's, the fall back's or a `withdraw`'s): a tank reverses only
//!   along its bow line, so each reverses on the smallest bow slant within
//!   [`HOLD_BOW_DEG`] that leaves [`EDGE_BACK_MARGIN_M`] astern
//!   (`back_off_bow`), and **ends and holds** — its own wait at once, `No
//!   room behind…` said — when none does, on a `Held:` while it reversed, or
//!   when held ground behind stopped it; their bounds,
//!   [`REAPPROACH_MAX_MS`] and [`RETURN_BACK_MS`], cover the drive the help
//!   and the transcripts give (about 20 kph astern on grass). **The return**
//!   keeps its bow on the machine while it is held young, and its plot moves
//!   only farther: a machine coming back never lengthens the back-off; and its
//!   wait, which can now begin with the machine still in sight and leaving,
//!   reads that machine as coming on only once its row is nearer than the
//!   plot the wait faces (else the halt, the follow's begin at the keep-out
//!   and the hold again would go round every few decisions). **The
//!   broken machine**: a track faster than [`BROKEN_TRACK_MAX_MPS`] is no
//!   evidence, and a hurt machine with none is broken when its row's range
//!   from where the tank stands now opens by [`BROKEN_OPEN_M`]
//!   ([`Pilot::leaving`]); no follow begins inside the keep-out. **A followed
//!   row** is flown to [`HUNT_STALE_PLOT_MS`] under every doctrine. **The
//!   wingman's opening** survives a teammate at the muster
//!   ([`OPENING_TEAMMATE_M`]);
//! * "A machine that loses you … crosses to the other side's entry zone to
//!   look", and the proving ground's roads meet at The Crossroads (B) — so with
//!   no identified hostile held for 5 s (an unidentified contact does not
//!   count), no hunt left and the opening over, the pilot drives for `B`, past
//!   it 800 m, and back.
//!
//! **The pilot's own rule above any doctrine.** Hurt to half its hull's
//! internals, or with a plate **of its hull** at a quarter or gone, it falls
//! back: **in reverse with its bow on the target** (cut AL.2: it used to turn
//! its stern, or its damaged flank, to the machine) for 10 s or until it has
//! opened to its widened band, whichever is first; the widened band is 150 m
//! wider than the doctrine's, never narrower than it and never past
//! [`SIGHT_BAND_MAX_M`]. After that it flies its doctrine again at the widened
//! band, `close` flown as `hold` there (a failing hull does not drive in to
//! 200 m).
//!
//! **One stance precedence** (`Pilot::motion`, cut AL.2), each step returning
//! when it applies: (1) the held turn (or, after a stop while reversing, a
//! stop with the bow kept), the back-off and an open stall; (2) the pilot's
//! own fall-back withdrawal, in reverse bow-on (the current heading at full
//! ahead with no target to face); (3) a doctrine `withdraw`, the same; (3a,
//! cut AN.2) the re-approach: in reverse bow-on to its plot's distance, then
//! stopped facing the plot — or (cut AN.3; never both in force) the return
//! after a broken machine's follow, "once home it shoots back at whatever
//! comes for it, and after twenty quiet seconds … comes back in, once": in
//! reverse bow on the machine (cut AO.1; its last plot once it is not held
//! young) to [`RETURN_STANDOFF_M`] from its last plot, then stopped facing
//! that plot — each back-off on `back_off_bow`'s slant, a stop when none
//! leaves room (cut AO.1); (3b) the turn into a side or rear hit, stopped,
//! for [`THREAT_TURN_MS`] (never under an `orbit` doctrine, and nothing when
//! the threat is within [`HOLD_BOW_DEG`] of a young target the pilot already
//! faces); (3c, cut AN.2) the halt: stopped, bow on its hostile while that
//! hostile is held young; (4) the doctrine's stance round a hostile whose
//! plot is younger than [`HUNT_STALE_PLOT_MS`] — for a broken machine
//! followed home (cut AN.3; "on the way home it turns and fights only what is
//! close (inside about 200 m)"), in its place, a hold bow on at
//! [`FOLLOW_BAND_M`] behind it; only while no hostile is held
//! that young (cut AL.3), (4a) a stop facing a return not yet told apart and
//! (4b) a stop facing a hunted hostile expected back inside the sight, the
//! hunt's legs frozen through it; (5) the hunt; (4 again) the doctrine's
//! stance round an older row never seen fresh; with nothing to fly at, (6a,
//! cut AN.2) the opening while it is not over — still before its brief, a
//! move in for room at full ahead and stopped for the brief after it (cut
//! AO.1), its step off the road at [`OPENING_STEP_KPH`], then stopped bow
//! toward the other side's entry — and then (6) the search. [`Pilot::rule`] names the
//! rule step 3a, 3c, 4 or 6a flew at the last decision (`reapproach`,
//! `approach_wait`, `return_back`, `return_wait`, `halt`, `follow`,
//! `opening`). Every heading steps (2) to (6) return
//! at a forward speed is steered round held ground; every reverse
//! (the hold's, a withdrawal's, the back-off) whose ray behind the tank meets a
//! mark is a stop instead, the bow kept. The stances: `hold` stops at the band
//! bow-on ([`Pilot::hold`]); `close` is a hold at [`CLOSE_M`]; `hunt` drives at
//! a fresh hostile and holds at the band; `withdraw` backs away bow-on;
//! `orbit` circles as cut AK built it. A doctrine `withdraw` reaches step 3
//! only as a bounded hold on a failing hull (the lane enforces it, cut AK.2),
//! and it outranks the widened band after a fall back. A doctrine `hunt` (cut
//! AK.2) holds a hostile only while its plot is fresh ([`FRESH_PLOT_MS`]):
//! past that it hunts at once from the trail, and with no trail it searches.
//! No hunt starts while a re-approach is in force (cut AN.2), nor while a
//! broken machine's follow or its return is (cut AN.3), as none starts while
//! a withdrawal flies.
//!
//! **The knobs** (cut AN.2; [`Knobs`]): [`Pilot::new`] flies [`Knobs::off`],
//! the pilot as cut AM left it (no opening, no halt, no re-approach, no
//! follow), so every older unit test flies the pilot it was written for; the
//! lane builds [`Pilot::with_knobs`] from `[arena] opening`, `reapproach` and
//! (cut AN.3) `follow_broken`. The exchange the re-approach reads, and the
//! hits scored on each hostile that a broken machine is judged by
//! ([`Pilot::scored_on`]), are counted whatever the knobs say.
//!
//! **What a tuning round may move** (cut AL.3, between its measurement runs,
//! and nothing else). **AK's constants are frozen**: the hunt's, the fall
//! back's, the stall's and the held turn's stay as AK.3 left them. AL.3 may
//! move only AL's named constants within their bounds; AL.1's are, in the
//! picture, [`super::picture::FIRE_SLACK_START_MS`],
//! [`super::picture::FIRE_SLACK_UP_MS`] and [`super::picture::FIRE_RETRY_MS`],
//! and in the lane [`super::lane::WEAPONS_LEARNED_MS`]. The brief bounds them
//! 0..=500, 100..=300, 150..=500 and 5 000..=10 000; **the lane's tests admit
//! less**, one constant moved at a time from the built tree (the rest as
//! built): `FIRE_SLACK_START_MS` 50..=410 (seen passing at 50, 100, 150, 200,
//! 250 built, 300, 350, 400 and 410; at 0 the lattice rig's cannon is refused
//! 3 lines of 22, over its one in ten, and from 420 its median gap is over
//! its bound), `FIRE_SLACK_UP_MS` 100..=200 (100, 150 built and 200 pass; at
//! 250 and 300 the lattice rig's cannon median is over its bound),
//! `FIRE_RETRY_MS` 150..=500 (both ends and 250 built pass; under
//! [`super::lane::FIRE_GAP_MS`] the lane's gap after the refused line governs
//! the retry instead), and `WEAPONS_LEARNED_MS` 6 001..=10 000 (6 001, 7 000,
//! 8 000 built and 10 000 pass; the lock-line test needs it above
//! `WEAPONS_LOCKED_MS + POLL_STARVE_MS + LIMIT_WINDOW_MS`, 6 000, and does not
//! compile at or under it). Two moved together are untried. **Since AL.3**
//! (whose stop facing a return not yet told apart moved the scripted fights'
//! line phase) the lane's tests admit less of `FIRE_SLACK_START_MS`: 300 (now
//! built) passes; at 400 the withdraw's lapse sends its `SPEED 0` a tick
//! before that test's window opens, and at 350 that and the crowded minute's
//! held-back `STATUS` both fail (phase-sensitive tests, not the clock).
//! `FIRE_SLACK_UP_MS` at 200 alone passes; with `FIRE_SLACK_START_MS` at 400
//! as well it fails the lapse. Not tunable:
//! [`super::picture::ENGAGED_PULSE_MS`],
//! [`super::picture::FIRE_SLACK_MAX_MS`] (a ruling: one pulse),
//! [`super::picture::FIRE_SLACK_MIN_MS`],
//! [`super::picture::FIRE_SLACK_DOWN_MS`], [`super::lane::FIRE_GAP_MS`],
//! [`super::lane::POLL_STARVE_MS`], and
//! [`super::picture::CYCLE_READ_WITHIN_MS`] and
//! [`super::lane::WEAPONS_AFTER_SHOT_MS`] (both read from the transcript); nor
//! [`SIGHT_BAND_MAX_M`] and [`FALL_BACK_MAX_M`] (a ruling) or
//! [`HUNT_STALE_PLOT_MS`] (a guard: no live row read older than 2 s).
//!
//! AL.2's, in this module, within these bounds: [`HOLD_SLACK_M`] 50..=100,
//! [`THREAT_TURN_MS`] 2 000..=8 000, [`ONE_TRACK_DEADBAND_DEG`] 45..=90,
//! [`ONE_TRACK_COMMIT_MS`] 8 000..=16 000, [`HELD_MARK_NEAR_M`] 80..=200,
//! [`HELD_MARK_HALF_WIDTH_M`] 20..=60 and [`HELD_MARK_DETOUR_DEG`] 30..=90.
//! Not tunable: [`HOLD_BOW_DEG`] (a ruling: the help's 45° front arc less a
//! margin), [`HOLD_HYSTERESIS_M`] (half the scope's rounding),
//! [`HELD_MARK_AHEAD_M`], [`HELD_MARK_CLEAR_M`] (derived) and
//! [`HELD_MARKS_MAX`].
//!
//! AL.3's, in this module: [`SEEN_SECOND_M`] 600..=800 and
//! [`AMBUSH_WAIT_MS`] 10 000..=30 000 (each tried at both ends, one at a time,
//! and passing). Not tunable: [`UNIDENTIFIED_WAIT_MS`] (twice the help's three
//! seconds) and [`UNIDENTIFIED_STOPS_MAX`] (a bound, not a tactic). AL.3's own
//! runs never reached the wait (every hunted machine was one the bot had
//! hurt), so neither moved.
//!
//! **The tests admit less of two of them**, one constant moved at a time from
//! the built tree (the rest as built; every other AL.2 constant passes at both
//! ends of its bound):
//! `HELD_MARK_NEAR_M` 90..=200 (80 fails: the bot's own run-2 case, `091` from
//! (1 330, 1 052), meets that stop's point 89.7 m ahead, and the brief fixes
//! that geometry; 90, 120 built and 200 pass), and `HELD_MARK_HALF_WIDTH_M`
//! 25..=60 (at 20 the clear distance falls to 40 m, and a mark dead-reckoned
//! up to 2 s of driving past the true stop lies beyond it: the back-off from
//! that forward stop is then stopped by the stop's own mark, and a search
//! drifting on its dead reckoning slips past a mark's narrower line; 25, 28,
//! 30 built and 60 pass). Also tried at both ends and passing:
//! [`HOLD_SLACK_M`] (and 75), [`THREAT_TURN_MS`], [`ONE_TRACK_DEADBAND_DEG`]
//! (and 60), [`ONE_TRACK_COMMIT_MS`] and [`HELD_MARK_DETOUR_DEG`]. Two moved
//! together are untried.
//!
//! **AN.2's**, in this module, within these bounds: [`OPENING_MAX_MS`]
//! 240 000..=480 000, [`OPENING_COVER_M`] 20..=40 (never below 20: with
//! [`OPENING_ARRIVE_M`] at 10 the tank could stop on the road),
//! [`OPENING_STEP_MAX_M`] 60..=120, [`HALT_STILL_MS`] 4 000..=10 000,
//! [`EXCHANGE_WINDOW_MS`] 10 000..=30 000, [`EXCHANGE_MIN_TAKEN`] 4..=12,
//! [`EXCHANGE_MARGIN`] 2..=8, [`REAPPROACH_OPEN_M`] 620..=700,
//! [`REAPPROACH_MAX_MS`] 30 000..=90 000 (widened by cut AO.1, which raised it
//! to 60 000 to cover the drive), [`REAPPROACH_WAIT_MS`]
//! 30 000..=60 000 and [`REAPPROACH_PER_MATCH`] 1..=3. Not tunable:
//! [`OPENING_BRIEF_WAIT_MS`], [`OPENING_STEP_KPH`], [`OPENING_ARRIVE_M`],
//! [`OPENING_STEP_MS`], [`STOCK_SIGHT_M`] (the help's), [`HIDING`] (the
//! help's), [`HALT_MAX_MS`] and [`REAPPROACH_REST_MS`]; and [`HUNT_TRAIL_MS`]
//! stays frozen with AK's.
//!
//! **AN.3's**, in this module, within these bounds (AN.3's runs may move these
//! and AN.2's, one at a time, and nothing else): [`BROKEN_HITS_MIN`] 4..=12,
//! [`BROKEN_AWAY_DEG`] 45..=90, [`FOLLOW_BAND_M`] 260..=440 (outside the
//! help's 200 m and inside the stopped tank's 480 m), [`ENTRY_KEEP_OUT_M`]
//! 250..=600, [`RETURN_STANDOFF_M`] 620..=750 (past the 600 m moving sight)
//! and [`RETURN_WAIT_MS`] 60 000..=120 000; and in the picture
//! [`super::picture::LATTICE_GUARD_MS`] 150..=300 (widened by cut AO.2, which
//! raised it to 275, past the edge the transcripts measured). Not tunable:
//! [`FOLLOW_STOPPED_MS`] and [`RETURN_BACK_MS`] (raised by cut AO.1 to
//! 65 000, derived from the drive), and the picture's other lattice
//! constants.
//!
//! **AO.1's**, in this module, within these bounds: [`OPENING_ROOM_MARGIN_M`]
//! 30..=120, [`EDGE_BACK_MARGIN_M`] 20..=60 and [`BROKEN_OPEN_M`] 50..=150
//! (and AN.2's [`REAPPROACH_MAX_MS`] above, its bound widened). Not tunable:
//! [`MACHINE_STANDOFF_M`] and [`PASSABLE`] (the help's), [`OPENING_ROOM_M`]
//! and [`OPENING_ROOM_SLACK_M`] (derived), [`OPENING_INWARD_MIN_M`],
//! [`OPENING_INWARD_MS`], [`OPENING_INWARD_LEGS_MAX`] (a bound),
//! [`BACK_OFF_SLANT_STEP_DEG`], [`BROKEN_OPEN_WINDOW_MS`] (the track's own
//! window), [`BROKEN_OPEN_SPAN_MS`], [`BROKEN_TRACK_MAX_MPS`] (the help's
//! forward speed and a third) and [`OPENING_TEAMMATE_M`].
//!
//! Every test asserts through these names, never their values; the one
//! exception is the lead's own bounds on the offline median gaps (1 200 ms for
//! the machine gun, 6 500 for the cannon), which a round must keep.
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::Serialize;

use super::{
    Side,
    picture::{
        Contact, EventKind, Ground, MatchMode, Phase, Picture, Power, TRACK_WINDOW_MS, Track,
        bearing_deg, distance_m,
    },
    settings::Opening,
};

/// "Full ahead" before the tank's own ceilings are read.
pub const UNKNOWN_FULL_KPH: i32 = 40;
/// The speed that asks the tank for its ceilings.
pub const CEILING_ASK_KPH: i32 = 999;
/// A plot older than this is not a fresh one.
pub const FRESH_PLOT_MS: u64 = 3_000;
/// …and a row read longer ago than this is not a current reading of it.
pub const FRESH_READ_MS: u64 = 2_000;
/// A held lock is kept unless another hostile is at least this much nearer.
pub const RETARGET_NEARER_M: u32 = 100;
/// The least time between two `LOCK` orders.
pub const LOCK_EVERY_MS: u64 = 1_500;
/// A `ready` read this soon after an ack may be the pre-shot frame.
pub const READY_AFTER_ACK_MS: u64 = 1_500;
/// The orbit's radial correction: at most this many degrees off the tangent…
pub const ORBIT_CORRECTION_DEG: f64 = 60.0;
/// …reached this many metres off the band.
pub const ORBIT_CORRECTION_SPAN_M: f64 = 150.0;
/// `Close` drives straight in to this range bow-on, then holds there (cut
/// AL.2: it used to circle there).
pub const CLOSE_M: f64 = 200.0;
/// **The hold's slack** (cut AL.2; renamed from `STAND_OFF_SLACK_M`): a hold
/// changing range stops once within this of its band. Tunable 50..=100.
pub const HOLD_SLACK_M: f64 = 50.0;
/// **The hold's hysteresis** (cut AL.2): a stopped hold changes range only
/// past the slack and this much more. Half the scope's rounding: a row's range
/// is "rounded to 50 m" (`help cockpit`), so a machine at the help's 420 m
/// reads `400 m`, and a threshold on that step would toggle the speed every
/// read. Not tunable.
pub const HOLD_HYSTERESIS_M: f64 = 25.0;
/// **The bow's tolerance** (cut AL.2): a hold faces within this of the
/// hostile, and a threat this close to the held target's bearing is the
/// target. A ruling, not tunable: "from within 45 degrees of your bow it
/// strikes the front" (`help cockpit`, "Seeing"), less a margin for
/// whole-degree bearings and a machine that moves.
pub const HOLD_BOW_DEG: u32 = 30;
/// **The turn into a side or rear hit** lasts this long after the hit (cut
/// AL.2). Tunable 2 000..=8 000.
pub const THREAT_TURN_MS: u64 = 4_000;
/// **One track gone** (cut AL.2): a heading toward the dead side by at most
/// this is not ordered. The help's front arc: a dead-side error inside it
/// still shows the front. Tunable 45..=90.
pub const ONE_TRACK_DEADBAND_DEG: u32 = 45;
/// …and after a larger one is ordered, no heading goes for this long unless
/// `STATUS` reads it first. The bot's own transcript turned a one-track tank
/// about 30° a second (307° to 164° counter-clockwise in 4.8 s), so a whole
/// circle is about 12 s. Tunable 8 000..=16 000.
pub const ONE_TRACK_COMMIT_MS: u64 = 12_000;
/// **Held ground** (cut AL.2): a mark is the point this far ahead of where the
/// tank stopped (behind it, stopped reversing). Not tunable.
pub const HELD_MARK_AHEAD_M: f64 = 20.0;
/// A forward heading is blocked by a mark at most this far ahead along it…
/// Tunable 80..=200.
pub const HELD_MARK_NEAR_M: f64 = 120.0;
/// …and within this of its line. Tunable 20..=60.
pub const HELD_MARK_HALF_WIDTH_M: f64 = 30.0;
/// A mark this close to the own tank does not block a forward heading: it is
/// the held turn's and the stall's business, so a tank at its own stop is not
/// boxed in. Derived, not tunable.
pub const HELD_MARK_CLEAR_M: f64 = HELD_MARK_AHEAD_M + HELD_MARK_HALF_WIDTH_M;
/// A blocked heading is turned aside this much at a time, up to three times.
/// Tunable 30..=90.
pub const HELD_MARK_DETOUR_DEG: f64 = 45.0;
/// The most marks a match keeps; the oldest goes first. Not tunable.
pub const HELD_MARKS_MAX: usize = 16;
/// The three `Held:` sentences that name ground (cut AL.2): only these are
/// marked. `Held: something is in the way.` is another unit, which moves, and
/// `Held: the tank is held.` names nothing.
pub const HELD_GROUND: [&str; 3] = [
    "Held: the slope ahead is too steep for the tank.",
    "Held: the ground ahead will not take the tank.",
    "Held: this is the edge of the arena.",
];
/// The orbit reverses after this long without a track to steer by.
pub const FLIP_EVERY_MS: u64 = 20_000;
/// A track is advanced by its plot's age, at most this far.
pub const TRACK_LEAD_MAX_MS: u64 = 3_000;
/// **The widest band the pilot flies** (cut AK.1): the 480 m at which a
/// stopped tank on grass is seen (`help cockpit`, "Seeing"), less a margin for
/// the scope's 50 m rounding and the tank's own drift. The machines stop to
/// shoot ("where it stops, turns onto you and shoots", `help simulator`), so
/// the moving sight's 600 m is the wrong ceiling. A ruling, not a tuning
/// constant: `[arena] band_m` is refused above it.
pub const SIGHT_BAND_MAX_M: u32 = 450;
/// Falling back: the withdrawal's longest length (it ends sooner once the
/// target is at the widened band)…
pub const WITHDRAW_MS: u64 = 10_000;
/// …the band's widening after it…
pub const FALL_BACK_WIDEN_M: u32 = 150;
/// …and the widest band it widens to: the sight's own ceiling (not tunable).
pub const FALL_BACK_MAX_M: u32 = SIGHT_BAND_MAX_M;
/// A plate of the hull at or under `1 / FALL_BACK_FACET_DIVISOR` of its
/// armour falls the pilot back.
pub const FALL_BACK_FACET_DIVISOR: i64 = 4;
/// A `Held:` notice turns the tank this far…
pub const HELD_TURN_DEG: f64 = 45.0;
/// …and holds the new heading this long.
pub const HELD_FOR_MS: u64 = 3_000;
/// A tank still held at 0 kph this long after the stall was last judged is
/// turned further (see the module head for the lag it sits above).
pub const STALL_MS: u64 = 4_000;
/// The back-off's length, astern, when three turns have not freed the tank.
pub const HELD_REVERSE_MS: u64 = 3_000;
/// Astern before `SPEED`'s usage line has read the tank's own ceiling: a slow
/// reverse, well inside any tank's. The ceilings are asked first thing
/// (`SPEED 999`), so it is rarely used.
pub const UNKNOWN_ASTERN_KPH: i32 = 10;
/// With no hostile held this long the pilot searches.
pub const SEARCH_AFTER_MS: u64 = 5_000;
/// Within this of The Crossroads the search drives on past it…
pub const SEARCH_ARRIVE_M: f64 = 100.0;
/// …this far, then turns back.
pub const SEARCH_BEYOND_M: f64 = 800.0;
/// A heading is re-ordered only when it differs by this much…
pub const HEADING_STEP_DEG: u32 = 8;
/// …or by any amount after this long.
pub const HEADING_REFRESH_MS: u64 = 4_000;
/// The spot the search drives for (`help simulator`, "The ground").
pub const SEARCH_SPOT: char = 'B';
/// **A hostile row older than this is not held** for the hunt's purposes: it
/// is hunted rather than orbited round a stale plot, and it no longer counts
/// as a hostile held for the search. A guard, not a tuning constant: in the
/// live smoke no `CONTACTS` row read older than 2 s (a contact goes as
/// `Contact BA lost.`).
pub const HUNT_STALE_PLOT_MS: u64 = 6_000;
/// A hunt starts only from a trail whose plot is at most this old; the course
/// leg runs while it is younger. At its ceiling since the re-measure, which
/// raised it from 45 s after a tank slowed by a lost track took 88 s to reach
/// its plot; the course leg still never ran at 90 s (every hunting tank took
/// 88 to 211 s to reach its plot), so the value was kept on the tie-breakers
/// of three runs (fewer timeouts), not on a measured effect.
pub const HUNT_TRAIL_MS: u64 = 90_000;
/// The estimated speed of a contact lost with no published track.
pub const HUNT_ASSUMED_MPS: f64 = 8.0;
/// The course leg's aim runs at most this far past the last plot.
pub const HUNT_ALONG_MAX_M: f64 = 1_200.0;
/// Within this of a leg's aim the leg is done.
pub const HUNT_ARRIVE_M: f64 = 100.0;
/// The sweep's legs…
pub const HUNT_SWEEP_LEGS: u8 = 4;
/// …each this long…
pub const HUNT_SWEEP_LEG_M: f64 = 300.0;
/// …and the whole sweep at most this long.
pub const HUNT_SWEEP_MS: u64 = 60_000;
/// "The proving ground is 2.56 km square" (`help simulator`, "The ground"),
/// counted from its south-west corner as `STATUS` counts.
pub const ARENA_SIDE_M: f64 = 2_560.0;
/// No hunt aim lies nearer the edge than this.
pub const EDGE_MARGIN_M: f64 = 60.0;
/// **Seen second** (cut AL.3): a hunted hostile whose estimate is at most this
/// far off is waited for, stopped. The stock sight's 600 m against a mover
/// ("on grass a stopped tank can be seen out to 80 per cent of that (480 m)
/// and a moving one out to all of it", `help cockpit`, "Seeing"), plus a
/// margin for the estimate's error. No ground in the help lets a moving tank
/// hide better than a stopped one, so stopping is never worse wherever the
/// tank stands: the ground is not read. Tunable 600..=800.
pub const SEEN_SECOND_M: f64 = 700.0;
/// …waited for this long, once a trail: "after twenty quiet seconds with
/// nothing hostile in sight it turns round and comes back in, once" (`help
/// simulator`, "Fighting the machines"). Tunable 10 000..=30 000.
pub const AMBUSH_WAIT_MS: u64 = 20_000;
/// **A return not yet told apart** (cut AL.3) is faced, stopped, for at most
/// this long from when it was first seen — twice "Three seconds of holding a
/// return is enough to tell it apart" (`help cockpit`, "Names") — and no
/// other such stop begins for this long after one ends. Not tunable.
pub const UNIDENTIFIED_WAIT_MS: u64 = 6_000;
/// …and at most this many such stops a match. A bound, not a tactic: a stream
/// of returns (a flicker at the sight's edge, a crowd of friends and machines)
/// cannot hold the tank still for a match. Not tunable.
pub const UNIDENTIFIED_STOPS_MAX: u8 = 4;
/// **The opening's longest hold** (cut AN.2), from power online; then the
/// search. "one that has seen nobody for a minute and a half crosses to the
/// other side's entry zone to look" (`help simulator`): the machine comes to
/// the muster's side. Tunable 240 000..=480 000.
pub const OPENING_MAX_MS: u64 = 360_000;
/// **How far into hiding ground the step goes** past where that ground's
/// run may truly start (cut AN.2): the run's start and the cell allowance,
/// then this. Tunable 20..=40, never below 20: the step is done
/// [`OPENING_ARRIVE_M`] short of its aim, so the tank then stands at least
/// 10 m past the run's true start, off the road.
pub const OPENING_COVER_M: f64 = 20.0;
/// The longest step off the road, metres (cut AN.2). Tunable 60..=120.
pub const OPENING_STEP_MAX_M: f64 = 80.0;
/// **The opening's waits** (cut AN.2): for the first `CONTACTS` reply after
/// power and for the brief's ground, each at most this long; and a start
/// position read more than this after power online is no muster. Not
/// tunable.
pub const OPENING_BRIEF_WAIT_MS: u64 = 5_000;
/// The step's speed, kph (cut AN.2). Not tunable.
pub const OPENING_STEP_KPH: i32 = 15;
/// The step is done this short of its aim, metres (cut AN.2). Not tunable.
pub const OPENING_ARRIVE_M: f64 = 10.0;
/// …or this long after it began (cut AN.2). Not tunable.
pub const OPENING_STEP_MS: u64 = 20_000;
/// **The stock sight** (cut AN.2): "Against the stock sight's 600 m … a
/// moving one out to all of it" (`help cockpit`, "Seeing"). A hostile row
/// farther out is a teammate's report ("anything a teammate is passing you
/// from beyond it"). Not tunable.
pub const STOCK_SIGHT_M: f64 = 600.0;
/// **Ground that hides a stopped tank** (cut AN.2): `help cockpit`'s
/// stopped-tank ceilings under 100 per cent on ground a tank enters — grass
/// 80, bare ground 90, rubble 70, light woods 65. Heavy woods are left out ("A
/// walker takes the thick woods that stop a tank", `help simulator`), and "a
/// road hides nothing either way". The brief's own class words. Not tunable.
pub const HIDING: [&str; 4] = ["grass", "bare ground", "rubble", "light woods"];
/// **The halt ends when its hostile has neither closed nor published a track
/// for this long** (cut AN.2): it has stopped coming on. "Closed" is its row's
/// range read shorter than the shortest read before it in the halt: the scope
/// rounds a range to 50 m (`help cockpit`), so a machine coming on at 10 m a
/// second reads 50 m nearer every five seconds, while its row-only fixes
/// (about 55 m of sigma at 500 to 600 m) seldom carry a track in the window.
/// Tunable 4 000..=10 000.
pub const HALT_STILL_MS: u64 = 6_000;
/// …and at this, whatever. Not tunable.
pub const HALT_MAX_MS: u64 = 60_000;
/// **The exchange's window** (cut AN.2): hits taken and scored in the last
/// this long. Tunable 10 000..=30 000.
pub const EXCHANGE_WINDOW_MS: u64 = 20_000;
/// A re-approach needs at least this many hits taken in the window…
/// Tunable 4..=12.
pub const EXCHANGE_MIN_TAKEN: usize = 8;
/// …and at least this many more taken than scored. Tunable 2..=8.
pub const EXCHANGE_MARGIN: usize = 4;
/// **The re-approach opens to this far from the machine's last plot**
/// (cut AN.2): past the 600 m a mover is seen at, so the machine coming on
/// is seen first. Tunable 620..=700.
pub const REAPPROACH_OPEN_M: f64 = 650.0;
/// The backing off is given up this long after it began (cut AN.2; raised by
/// cut AO.1 to cover the drive). `help cockpit` gives the stock tank 27 kph
/// astern and caps grass as 54 is to about 41, so about 20.5 kph astern on
/// grass; the bot's own transcript reads `STATUS` -8, -18 and -20 kph about
/// 2.5, 5.3 and 7.3 s after `SPEED -27`, then -20 steady (about 5.6 m/s). A
/// machine at the help's 420 m reads `400 m` on the scope, so the back-off to
/// [`REAPPROACH_OPEN_M`] is about 250 m and takes about 46 s; 60 s covers it.
/// Tunable 30 000..=90 000.
pub const REAPPROACH_MAX_MS: u64 = 60_000;
/// The wait for the machine to come on lasts at most this long (cut AN.2).
/// Tunable 30 000..=60 000.
pub const REAPPROACH_WAIT_MS: u64 = 45_000;
/// At most this many re-approaches a match (cut AN.2). Tunable 1..=3.
pub const REAPPROACH_PER_MATCH: u8 = 2;
/// No re-approach begins for this long after one ends (cut AN.2). Not
/// tunable.
pub const REAPPROACH_REST_MS: u64 = 30_000;
/// **A broken machine** (cut AN.3; [`Pilot::broken`]): one the stream said
/// lost a section, or one the pilot has hit at least this many times this
/// match ([`Pilot::scored_on`]) — "Hurt badly enough it breaks off for its own
/// entry zone" (`help simulator`), and the help does not say how badly, so
/// hits scored stand in for the hurt the stream does not name. Tunable 4..=12.
pub const BROKEN_HITS_MIN: u32 = 8;
/// …driving away: its published course within this of the bearing from the
/// own tank to it (cut AN.3). Tunable 45..=90.
pub const BROKEN_AWAY_DEG: u32 = 60;
/// **The follow's band** (cut AN.3): the hold bow on behind a broken machine,
/// outside the help's "inside about 200 m" its way home is fought at and
/// inside the 480 m a stopped tank on grass is seen at. Tunable 260..=440.
pub const FOLLOW_BAND_M: f64 = 350.0;
/// **The keep-out** (cut AN.3): the follow ends once the machine is within
/// this of the other side's entry line, or the own tank within this and
/// [`FOLLOW_BAND_M`]. Tunable 250..=600.
pub const ENTRY_KEEP_OUT_M: f64 = 400.0;
/// A followed machine that publishes no track this long, outside the
/// keep-out, has stopped: the follow ends and the stance flies (cut AN.3).
/// Not tunable.
pub const FOLLOW_STOPPED_MS: u64 = 5_000;
/// **The return's standoff** (cut AN.3): the backing off ends this far from
/// the machine's last plot, past the 600 m a mover is seen at ("a moving one
/// out to all of it", `help cockpit`), so the machine coming back in is seen
/// first. Tunable 620..=750.
pub const RETURN_STANDOFF_M: f64 = 650.0;
/// …or this long after it began (cut AN.3; raised by cut AO.1 to cover the
/// drive): the same reverse on grass as [`REAPPROACH_MAX_MS`]'s, about 5.6 m/s
/// once up to speed, over the return's [`RETURN_STANDOFF_M`] less
/// [`FOLLOW_BAND_M`] (300 m at the shipped figures) takes about 55 s. Not
/// tunable.
pub const RETURN_BACK_MS: u64 = 65_000;
/// **The wait for the one return** (cut AN.3): "after twenty quiet seconds
/// with nothing hostile in sight it turns round and comes back in, once"
/// (`help simulator`), plus its drive back. Tunable 60 000..=120 000.
pub const RETURN_WAIT_MS: u64 = 90_000;
/// **The machine's stand-off** (cut AO.1): a machine "closes: to about seven
/// tenths of what its own sight and its gun's inner band allow (420 m for the
/// stock tank), where it stops, turns onto you and shoots" (`help
/// simulator`): where a re-approach's back-off begins. Not tunable.
pub const MACHINE_STANDOFF_M: f64 = 420.0;
/// **The opening's room margin** (cut AO.1): the scope rounds a range to
/// 50 m (`help cockpit`), so a machine at 420 m reads `400 m` and the back-off
/// from its row is that much longer, plus the ground a back-off keeps astern
/// ([`EDGE_BACK_MARGIN_M`]). Tunable 30..=120.
pub const OPENING_ROOM_MARGIN_M: f64 = 60.0;
/// **The ground the opening holds with behind it** (cut AO.1): the longest
/// back-off the pilot orders, from the machine's [`MACHINE_STANDOFF_M`] out
/// to [`REAPPROACH_OPEN_M`] of its plot, plus [`OPENING_ROOM_MARGIN_M`] (290 m
/// at the shipped figures). Derived, not tunable.
pub const OPENING_ROOM_M: f64 = REAPPROACH_OPEN_M - MACHINE_STANDOFF_M + OPENING_ROOM_MARGIN_M;
/// **A move in shorter than this is not driven** (cut AO.1): each leg is
/// followed by a stop and a fresh `VIEW BRIEF`, which the lane paces
/// [`OPENING_BRIEF_WAIT_MS`] apart. Not tunable.
pub const OPENING_INWARD_MIN_M: f64 = 40.0;
/// **A shortfall of room this small is room enough** (cut AO.1): its leg,
/// the shortfall and [`OPENING_ARRIVE_M`], would be under
/// [`OPENING_INWARD_MIN_M`]. Derived (30 m), not tunable.
pub const OPENING_ROOM_SLACK_M: f64 = OPENING_INWARD_MIN_M - OPENING_ARRIVE_M;
/// A move in is done this long after it began, wherever the tank is (cut
/// AO.1). Not tunable.
pub const OPENING_INWARD_MS: u64 = 60_000;
/// At most this many moves in a match (cut AO.1). A bound, not a tactic. Not
/// tunable.
pub const OPENING_INWARD_LEGS_MAX: u8 = 3;
/// **Ground a tank crosses** (cut AO.1): the brief's own class words for the
/// ground the help lets a tracked tank drive on. "Deep water is impassable to
/// a tracked tank; the proving ground has one ford" (`help cockpit`: the ford
/// is plain `water`), and "A walker takes the thick woods that stop a tank"
/// (`help simulator`). **Any other word blocks** — `deep water`, `heavy
/// woods`, and a word the help never named. Not tunable.
pub const PASSABLE: [&str; 6] = [
    "road",
    "grass",
    "bare ground",
    "rubble",
    "light woods",
    "water",
];
/// **The ground a back-off keeps astern** (cut AO.1): the re-approach's and
/// the return's back-offs slant or end before the ground behind the tank,
/// to the arena's edge, is shorter than this. Tunable 20..=60.
pub const EDGE_BACK_MARGIN_M: f64 = 30.0;
/// A back-off's bow slants off the machine this much at a time, up to
/// [`HOLD_BOW_DEG`] (cut AO.1). Not tunable.
pub const BACK_OFF_SLANT_STEP_DEG: u32 = 10;
/// **A machine leaving with no track** (cut AO.1; [`Pilot::leaving`]): its
/// row's range from where the tank stands now opened by at least this — two
/// of the scope's 50 m steps (`help cockpit`: a range is "rounded to 50 m").
/// Tunable 50..=150.
pub const BROKEN_OPEN_M: f64 = 100.0;
/// …over the fixes of the track's own window
/// ([`super::picture::TRACK_WINDOW_MS`]) (cut AO.1). Not tunable.
pub const BROKEN_OPEN_WINDOW_MS: u64 = TRACK_WINDOW_MS;
/// …the nearer fix at least this much older than the newest (cut AO.1). Not
/// tunable.
pub const BROKEN_OPEN_SPAN_MS: u64 = 2_000;
/// **A track faster than this is no evidence** (cut AO.1): "the stock tank
/// makes 54 kph forward" (`help cockpit`), 15 m/s, plus a third for the fit's
/// noise. The bot's own transcript showed a 181 m/s track on a machine 50 m
/// off. Not tunable.
pub const BROKEN_TRACK_MAX_MPS: f64 = 20.0;
/// **A teammate at the muster** (cut AO.1): the two sides muster at opposite
/// ends of the ground ("two sides fly stock tanks against each other on the
/// proving ground", `help simulator`), so a return not yet told apart this
/// close at the first `CONTACTS` is a teammate. Not tunable.
pub const OPENING_TEAMMATE_M: u32 = 100;

// The doctrine the pilot flies moved to the commander's module (cut AJ.3),
// which parses and clamps the model's answer into it; these names stay here.
pub use super::commander::{Doctrine, Sense, Stance};

/// What to type this tick; everything optional.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Orders {
    pub heading: Option<u32>,
    pub speed: Option<i32>,
    pub lock: Option<String>,
    /// Full weapon ids, main gun first.
    pub fire: Vec<String>,
    /// Ask `BEARING B` (the lane paces it).
    pub ask_spot_b: bool,
    /// Ask `VIEW BRIEF` (cut AN.2; the opening's brief, the lane paces it).
    pub ask_view_brief: bool,
}

/// **The pilot's knobs** (cut AN.2; `[arena] opening` and `reapproach`; cut
/// AN.3: `follow_broken`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Knobs {
    pub opening: Opening,
    pub reapproach: bool,
    /// Follow a broken machine home and wait for its return (cut AN.3).
    pub follow_broken: bool,
}

impl Knobs {
    /// The shipped defaults: the opening waits, the re-approach and the
    /// follow are on.
    pub fn on() -> Knobs {
        Knobs {
            opening: Opening::Wait,
            reapproach: true,
            follow_broken: true,
        }
    }

    /// The pilot as it was before them: the search at once, no re-approach,
    /// no follow.
    pub fn off() -> Knobs {
        Knobs {
            opening: Opening::Search,
            reapproach: false,
            follow_broken: false,
        }
    }
}

/// **Where the opening is** (cut AN.2; the module head).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum OpeningStage {
    /// Waiting for the muster's position and the first `CONTACTS` after
    /// power.
    Fresh,
    /// `VIEW BRIEF` asked, since then.
    Brief { since_ms: u64 },
    /// Stepping off the road from `from_m` to `aim_m`, since then.
    Step {
        aim_m: (f64, f64),
        from_m: (f64, f64),
        since_ms: u64,
    },
    /// **Moving in for room behind** (cut AO.1): driving from `from_m` to
    /// `aim_m` toward the other side's entry, since then; then stopped for
    /// the next brief.
    Inward {
        aim_m: (f64, f64),
        from_m: (f64, f64),
        since_ms: u64,
    },
    /// Holding stopped, bow toward the other side's entry (`facing_deg` as it
    /// was first faced), since then.
    Hold { since_ms: u64, facing_deg: u32 },
    /// Over, or never begun: the search flies.
    Done,
}

impl OpeningStage {
    /// `fresh`, `brief`, `inward` (cut AO.1), `step`, `hold` or `done` (the
    /// `arena-picture` record's word).
    pub fn word(&self) -> &'static str {
        match self {
            OpeningStage::Fresh => "fresh",
            OpeningStage::Brief { .. } => "brief",
            OpeningStage::Inward { .. } => "inward",
            OpeningStage::Step { .. } => "step",
            OpeningStage::Hold { .. } => "hold",
            OpeningStage::Done => "done",
        }
    }
}

/// **Fire from the halt** (cut AN.2): stopped, bow on `d`, since then.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Halt {
    /// A designator the stream named.
    pub d: String,
    pub since_ms: u64,
    /// The shortest range `d`'s row has read in the halt (the arming row's
    /// at first)…
    pub nearest_m: u32,
    /// …and when it was first read that short: [`HALT_STILL_MS`] from this
    /// with no track published, `d` has stopped coming on.
    pub nearest_ms: u64,
}

/// **The re-approach's stage** (cut AN.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum ReapproachStage {
    /// Backing away bow on, since then.
    Open { since_ms: u64 },
    /// Stopped, facing the plot, waiting for it to come on, since then.
    Wait { since_ms: u64 },
}

/// **A re-approach** (cut AN.2): on `d` (a designator the stream named), its
/// stage, and `d`'s last plot, followed while `d` is seen.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Reapproach {
    pub d: String,
    pub stage: ReapproachStage,
    pub plot_m: (f64, f64),
}

/// **A broken machine followed home** (cut AN.3; the module head): `d` (a
/// designator the stream named), since then, and the last decision `d`
/// published a track ([`FOLLOW_STOPPED_MS`] after it with none, it has
/// stopped).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Follow {
    pub d: String,
    pub since_ms: u64,
    pub tracked_ms: u64,
}

/// **The return's stage** (cut AN.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum ReturnStage {
    /// Backing out of the machine's sight bow on its last plot, since then.
    Back { since_ms: u64 },
    /// Stopped, facing the plot, waiting for the one return, since then.
    Wait { since_ms: u64 },
}

/// **The return** (cut AN.3): after a broken machine's follow ended at its
/// entry line or with it lost, `d` (a designator the stream named), its last
/// plot and the stage. Never in force with a [`Reapproach`].
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Return {
    pub d: String,
    pub plot_m: (f64, f64),
    pub stage: ReturnStage,
}

/// The search's leg.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub enum SearchLeg {
    /// Driving for The Crossroads.
    ToB,
    /// Past it, on a fixed heading, from `origin`.
    Beyond {
        origin: (f64, f64),
        heading_deg: f64,
    },
}

/// Why the pilot falls back ([`fall_back_due`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum FallBackWhy {
    /// The hull's internals are at half or less ([`hull_failing`]).
    HullHalf,
    /// A plate of the hull is at a quarter (`gone: false`) or gone.
    Plate { facet: String, gone: bool },
}

/// The last hostile the pilot held, as its own scope last plotted it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Trail {
    /// A designator the stream named (a contact row the pilot targeted),
    /// never invented.
    pub designator: String,
    /// The last plot: the contact's last fix, else its row from where the
    /// pilot was.
    pub plot_m: (f64, f64),
    /// When that plot was taken: the row's read less its printed age.
    pub plot_ms: u64,
    /// The picture's published track at the last fresh read.
    pub track: Option<Track>,
    /// Where the pilot was when that plot was read.
    pub seen_from_m: Option<(f64, f64)>,
}

/// The hunt's leg.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum HuntLeg {
    /// To the last plot.
    ToPlot,
    /// Along the estimated course from the last plot, while the plot is young.
    Along,
    /// One leg of the sweep: from `origin` on `heading_deg`.
    Sweep {
        leg: u8,
        origin: (f64, f64),
        heading_deg: f64,
    },
}

/// A hunt for a lost hostile.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Hunt {
    pub trail: Trail,
    pub leg: HuntLeg,
    /// When the hunt began.
    pub since_ms: u64,
    /// When the current leg began.
    pub leg_since_ms: u64,
    /// The estimated course and speed, fixed at the start.
    pub course_deg: f64,
    pub speed_mps: f64,
    /// When the sweep began, once it has.
    pub sweep_since_ms: Option<u64>,
    /// **The wait before the hostile finds us** (cut AL.3): when it began,
    /// once it has…
    pub wait_since_ms: Option<u64>,
    /// …and whether it is spent (its time up, or a hit taken after it began):
    /// a trail is waited for once.
    pub wait_spent: bool,
    /// The trail's track was going away or across when the hunt began (its
    /// course within 90° of the bearing from the own tank, then, to the
    /// plot): never waited for. Judged once, at the start — a tank that has
    /// reached or passed the plot has the bearing to it turned round, and a
    /// machine driving away from the plot is still driving away.
    pub going_away: bool,
}

/// **What a hold is doing with the range** (cut AL.2, [`Pilot::hold`]):
/// stopped at the band, closing it ahead, or opening it in reverse — always
/// with the bow on the hostile. Remembered between decisions so the range
/// moves only past the hysteresis, and back to `Stopped` whenever the pilot
/// flies anything but a hold.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub enum HoldMotion {
    #[default]
    Stopped,
    Ahead,
    Astern,
}

impl HoldMotion {
    /// `stopped`, `ahead` or `astern` (the `arena-picture` record's word).
    pub fn as_str(self) -> &'static str {
        match self {
            HoldMotion::Stopped => "stopped",
            HoldMotion::Ahead => "ahead",
            HoldMotion::Astern => "astern",
        }
    }
}

/// **A compass turn** (cut AL.2): clockwise is the heading growing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Turn {
    Clockwise,
    CounterClockwise,
}

impl Turn {
    /// The compass turn sign: `+1` clockwise, `-1` counter-clockwise.
    pub fn compass_sign(self) -> f64 {
        match self {
            Turn::Clockwise => 1.0,
            Turn::CounterClockwise => -1.0,
        }
    }

    /// `cw` or `ccw` (the `arena-picture` record's word).
    pub fn as_str(self) -> &'static str {
        match self {
            Turn::Clockwise => "cw",
            Turn::CounterClockwise => "ccw",
        }
    }

    /// `clockwise` or `counter-clockwise`.
    fn words(self) -> &'static str {
        match self {
            Turn::Clockwise => "clockwise",
            Turn::CounterClockwise => "counter-clockwise",
        }
    }

    /// The track whose loss leaves this the only way to turn.
    fn gone_track(self) -> &'static str {
        match self {
            Turn::Clockwise => "left_track",
            Turn::CounterClockwise => "right_track",
        }
    }

    /// Whether a signed turn `s` ([`signed_turn`], positive clockwise) goes
    /// the dead way when this is the only live one.
    fn is_dead(self, s: i64) -> bool {
        match self {
            Turn::Clockwise => s < 0,
            Turn::CounterClockwise => s > 0,
        }
    }
}

/// **The only way a one-track tank turns** (cut AL.2): exactly one of the own
/// sections `left_track` and `right_track` reads `destroyed` → the other way
/// ("Lose one track and the tank is slowed and will not turn that way", `help
/// cockpit`; the bot's own transcript turned a right-track tank the long way,
/// counter-clockwise). Both or neither → `None`.
pub fn live_turn(picture: &Picture) -> Option<Turn> {
    let gone = |track: &str| {
        picture
            .own
            .sections
            .get(track)
            .is_some_and(|section| section.condition == "destroyed")
    };
    match (gone("left_track"), gone("right_track")) {
        (true, false) => Some(Turn::Clockwise),
        (false, true) => Some(Turn::CounterClockwise),
        _ => None,
    }
}

/// **Held ground** (cut AL.2): where the tank stopped on ground it could not
/// enter, the heading it stopped on, whether it was reversing, and the point
/// it could not enter — [`HELD_MARK_AHEAD_M`] ahead of it, or behind it when
/// it stopped reversing. Positions, a heading and a direction only: a mark
/// names no unit. A point, not a bank.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HeldMark {
    pub at_m: (f64, f64),
    pub heading_deg: u32,
    pub astern: bool,
    pub blocked_m: (f64, f64),
}

/// The pilot's own state. One per match.
#[derive(Clone, Debug, Serialize)]
pub struct Pilot {
    /// **The orbit's circling sense**: `+1` counter-clockwise round the target,
    /// `-1` clockwise. Not a compass turn: the held turn, the stall, the sweep
    /// and the detour turn by [`Pilot::turn_sign`] (cut AL.2).
    pub sense: i8,
    /// When the current stretch of orbiting began.
    pub orbit_since_ms: Option<u64>,
    /// When the sense last reversed (or the orbit began).
    pub last_flip_ms: u64,
    /// Withdrawing until then (falling back), unless the target reaches the
    /// widened band first.
    pub withdraw_until_ms: Option<u64>,
    /// The pilot has fallen back; its band is widened for the match.
    pub fallen_back: bool,
    /// Holding `held_heading` until then after a `Held:` notice.
    pub held_until_ms: u64,
    pub held_heading: Option<u32>,
    /// The newest `Held:` event already turned for.
    pub held_seen_ms: u64,
    /// The heading the tank was stopped on at the last `Held:` notice.
    pub held_from_deg: Option<u32>,
    /// The stall watchdog: when the stall was last opened or raised…
    pub stall_since_ms: Option<u64>,
    /// …how far it has escalated (1 to 3 turned further, 4 backed off)…
    pub stall_level: u8,
    /// …the heading it holds…
    pub stall_heading: Option<u32>,
    /// …the `STATUS` read (its position's time) it last judged…
    pub stall_status_ms: u64,
    /// …the stall heading kept until then after the tank was freed…
    pub stall_clear_until_ms: u64,
    /// …and backing off until then.
    pub reverse_until_ms: u64,
    pub search: SearchLeg,
    /// The own position at `Power online.` (the first `STATUS` after it).
    pub start_pos: Option<(f64, f64)>,
    /// Where The Crossroads is, once a `STATUS` has named it nearest
    /// ([`super::picture::Spot::plotted_m`]). The search steers by it, which is
    /// the latest `BEARING B` answer made current: the answer is up to 5 s old,
    /// the plotted spot is where the ground is.
    pub spot_b_pos: Option<(f64, f64)>,
    /// When power came online, as the pilot first saw it.
    pub power_ms: Option<u64>,
    /// The last time a hostile row no older than [`HUNT_STALE_PLOT_MS`] was
    /// held.
    pub last_hostile_ms: Option<u64>,
    /// The last hostile held fresh, as last plotted.
    pub trail: Option<Trail>,
    /// The hunt in progress.
    pub hunt: Option<Hunt>,
    /// Every designator the pilot has held fresh this match: a row of one of
    /// them gone stale with no hunt left is not orbited but searched for.
    pub trailed: BTreeSet<String>,
    /// Designators the stream said were destroyed: never trailed or hunted.
    pub wrecks: BTreeSet<String>,
    /// The newest kill event already read.
    pub kill_seen_ms: u64,
    /// The last lock the picture held (an unnamed `Target destroyed.` names
    /// it).
    pub lock_seen: Option<String>,
    /// The last heading ordered, and when.
    pub last_heading_order: Option<(u32, u64)>,
    pub last_speed_order: Option<i32>,
    pub last_lock_order_ms: Option<u64>,
    /// `SPEED 999` has gone out (or the ceilings were known).
    pub ceilings_asked: bool,
    /// **The hold's range motion** (cut AL.2; [`Pilot::hold`]).
    pub hold_motion: HoldMotion,
    /// A hold was flown this decision (else `hold_motion` goes back to
    /// `Stopped`).
    #[serde(skip)]
    hold_flown: bool,
    /// **The turn into a side or rear hit** (cut AL.2): the newest facet hit
    /// already read…
    pub threat_seen_ms: u64,
    /// …and the bearing to turn to, until when.
    pub threat: Option<(f64, u64)>,
    /// The threat's `Turning into the shot: …` sentence, said only when step
    /// 3b first flies the turn: a turn outranked for its whole time (a held
    /// stop, the back-off, a withdrawal) or dropped is never said.
    #[serde(skip)]
    threat_note: Option<String>,
    /// **One track gone** (cut AL.2): a dead-side heading ordered, and when;
    /// no heading goes until `STATUS` reads it or [`ONE_TRACK_COMMIT_MS`]
    /// passes.
    pub turn_commit: Option<(u32, u64)>,
    /// The dead-side heading this decision would order, committed when the
    /// lane types it.
    #[serde(skip)]
    turn_wanted: Option<u32>,
    /// The one-track sentence has been said this match.
    pub one_track_said: bool,
    /// **Held ground** (cut AL.2), oldest first, at most [`HELD_MARKS_MAX`].
    pub held_marks: Vec<HeldMark>,
    /// The newest `Held:` came while reversing: step 1 stops the tank, the
    /// bow kept, instead of the forward held turn.
    pub held_astern: bool,
    /// A heading is being steered round held ground now.
    pub detour: bool,
    /// The speed the pilot's own motion last chose (not `SPEED 999`, and not
    /// forgotten with an unanswered order): the stall opens only after a
    /// forward one.
    pub last_chosen_speed: Option<i32>,
    /// **Seen second** (cut AL.3): every hostile the stream said lost a
    /// section this match (`{d}'s {section} is destroyed.`). Such a machine is
    /// hunted at once, never waited for: "Hurt badly enough it breaks off for
    /// its own entry zone … on the way home it turns and fights only what is
    /// close (inside about 200 m)" (`help simulator`).
    pub hurt: BTreeSet<String>,
    /// The newest section-lost event already read.
    pub hurt_seen_ms: u64,
    /// The newest hit taken (a `Hit on the …` line) read this match: a wait
    /// that began before it is over for good — the tank has been found.
    pub hit_taken_ms: u64,
    /// When each contact not yet told apart was first seen this match, while
    /// it is one (lost or told apart, it is forgotten: found again, it is a
    /// new return).
    pub unknown_since: BTreeMap<String, u64>,
    /// The stop for a return not yet told apart now in force: whom, and when
    /// it began.
    pub unknown_stop: Option<(String, u64)>,
    /// How many such stops this match (at most [`UNIDENTIFIED_STOPS_MAX`])…
    pub unknown_stops: u8,
    /// …and no new one before then: [`UNIDENTIFIED_WAIT_MS`] after the last
    /// one ended.
    pub unknown_rest_until_ms: u64,
    /// The designators whose stop has been said (forgotten with
    /// [`Pilot::unknown_since`], so a return found again is said again).
    #[serde(skip)]
    unknown_said: BTreeSet<String>,
    /// A stop for a return was flown this decision (else the one in force
    /// ends, and the rest begins).
    #[serde(skip)]
    unknown_flown: bool,
    /// **The wait flown at the last decision** (cut AL.3; either kind): whom
    /// it is for, and when it ends.
    pub wait: Option<(String, u64)>,
    /// **The knobs** this pilot flies (cut AN.2).
    pub knobs: Knobs,
    /// **The own muster** (cut AN.2): [`Pilot::start_pos`] when the picture
    /// saw this match's own `Power online.`
    /// ([`super::picture::Picture::powered_here`]) and the start position was
    /// read within [`OPENING_BRIEF_WAIT_MS`] of it; else `None` for the match
    /// (a bot restarted or attached into a running match would take a
    /// mid-field point for it). With none there is no opening and no other
    /// entry.
    pub muster_m: Option<(f64, f64)>,
    /// **The opening** (cut AN.2): [`OpeningStage::Done`] from the start under
    /// `opening = "search"`.
    pub opening: OpeningStage,
    /// The picture's [`super::picture::Picture::contacts_read`] at the first
    /// decision: the opening waits for the first reply after it.
    opening_contacts_seen: u64,
    /// **The moves in for room behind** begun this match (cut AO.1), at most
    /// [`OPENING_INWARD_LEGS_MAX`].
    pub opening_legs: u8,
    /// The opening's `no way in` sentence has been said this match (cut
    /// AO.1).
    opening_no_way_said: bool,
    /// **The last decision the shared reverse path turned a back-off's
    /// reverse into a stop** (cut AO.1): held ground behind the tank, while the
    /// rule was `reapproach` or `return_back`. A back-off begun before it ends
    /// at the next decision.
    pub back_off_blocked_ms: u64,
    /// **Fire from the halt** in force (cut AN.2).
    pub halt: Option<Halt>,
    /// **The re-approach** in force (cut AN.2)…
    pub reapproach: Option<Reapproach>,
    /// …how many began this match (at most [`REAPPROACH_PER_MATCH`])…
    pub reapproaches: u8,
    /// …and none begins before then.
    pub reapproach_rest_until_ms: u64,
    /// **The exchange** (cut AN.2): each hit taken (`true`) and each hit
    /// scored on a named hostile (`false`) this match, oldest first, pruned to
    /// [`EXCHANGE_WINDOW_MS`]. Fed every decision whatever the knobs say. It
    /// holds times only, never a designator.
    pub exchange: VecDeque<(u64, bool)>,
    /// The newest event the exchange has read.
    exchange_seen_ms: u64,
    /// **The hits scored on each hostile this match** (cut AN.3): every
    /// `Hit on {d}…` not marked `(FRIENDLY)`, by `d` (a designator the stream
    /// named), read with the exchange every decision whatever the knobs say.
    /// [`BROKEN_HITS_MIN`] of them make a machine driving away broken.
    pub scored_on: BTreeMap<String, u32>,
    /// **A broken machine followed home** in force (cut AN.3)…
    pub follow: Option<Follow>,
    /// …and **the return** after it (cut AN.3).
    pub ret: Option<Return>,
    /// **The rule flown at the last decision** (cut AN.2; [`Pilot::rule`]).
    rule: Option<&'static str>,
    /// What the pilot said since [`Pilot::take_notes`].
    #[serde(skip)]
    notes: Vec<String>,
}

impl Default for Pilot {
    fn default() -> Self {
        Pilot {
            sense: 1,
            orbit_since_ms: None,
            last_flip_ms: 0,
            withdraw_until_ms: None,
            fallen_back: false,
            held_until_ms: 0,
            held_heading: None,
            held_seen_ms: 0,
            held_from_deg: None,
            stall_since_ms: None,
            stall_level: 0,
            stall_heading: None,
            stall_status_ms: 0,
            stall_clear_until_ms: 0,
            reverse_until_ms: 0,
            search: SearchLeg::ToB,
            start_pos: None,
            spot_b_pos: None,
            power_ms: None,
            last_hostile_ms: None,
            trail: None,
            hunt: None,
            trailed: BTreeSet::new(),
            wrecks: BTreeSet::new(),
            kill_seen_ms: 0,
            lock_seen: None,
            last_heading_order: None,
            last_speed_order: None,
            last_lock_order_ms: None,
            ceilings_asked: false,
            hold_motion: HoldMotion::Stopped,
            hold_flown: false,
            threat_seen_ms: 0,
            threat: None,
            threat_note: None,
            turn_commit: None,
            turn_wanted: None,
            one_track_said: false,
            held_marks: Vec::new(),
            held_astern: false,
            detour: false,
            last_chosen_speed: None,
            hurt: BTreeSet::new(),
            hurt_seen_ms: 0,
            hit_taken_ms: 0,
            unknown_since: BTreeMap::new(),
            unknown_stop: None,
            unknown_stops: 0,
            unknown_rest_until_ms: 0,
            unknown_said: BTreeSet::new(),
            unknown_flown: false,
            wait: None,
            knobs: Knobs::off(),
            muster_m: None,
            opening: OpeningStage::Done,
            opening_contacts_seen: 0,
            opening_legs: 0,
            opening_no_way_said: false,
            back_off_blocked_ms: 0,
            halt: None,
            reapproach: None,
            reapproaches: 0,
            reapproach_rest_until_ms: 0,
            exchange: VecDeque::new(),
            exchange_seen_ms: 0,
            scored_on: BTreeMap::new(),
            follow: None,
            ret: None,
            rule: None,
            notes: Vec::new(),
        }
    }
}

/// **The other side's entry** (cut AN.2): the own muster `start` mirrored
/// across the ground's centre line that lies between it and the nearer pair
/// of edges, on a ground `side` metres east-west by north-south — `(side.0 −
/// x, y)` when `|x − side.0 / 2| ≥ |y − side.1 / 2|`, else `(x, side.1 − y)`.
/// Both sides fly identical stock tanks from opposite ends of the ground
/// ("two sides fly stock tanks against each other on the proving ground",
/// `help simulator`): the reflection is the pilot's inference from that, and
/// no contact bearing is used (the first contact comes from wherever the
/// machine's patrol is). Wrong only on a ground whose sides do not muster at
/// mirrored ends, and then only in where the pilot faces while it waits.
pub fn other_entry(start: (f64, f64), side: (f64, f64)) -> (f64, f64) {
    let (x, y) = start;
    if (x - side.0 / 2.0).abs() >= (y - side.1 / 2.0).abs() {
        (side.0 - x, y)
    } else {
        (x, side.1 - y)
    }
}

/// A brief's bearing in degrees as its word in lower case (`north`,
/// `north-east`, …); `that way` for a bearing no brief prints.
fn bearing_word(deg: u32) -> String {
    super::lines::BRIEF_BEARINGS
        .iter()
        .find(|(_, at)| *at == deg)
        .map_or_else(|| "that way".to_owned(), |(word, _)| word.to_lowercase())
}

/// **The step off the road** the brief's `ground` offers (cut AN.2), as
/// `(bearing, metres)`, every ray judged from the ground's own `from_m`;
/// `None` to hold where the tank stands.
///
/// * **Already hidden**: at least 6 of the 8 rays begin on [`HIDING`] ground.
///   A proxy read from the eight neighbours: a ray starts from the pilot's own
///   cell and never describes it.
/// * **Otherwise**, on each ray more than 45° from `away_deg` (the bearing
///   from the muster to the other side's entry: never toward the machines),
///   the first run on hiding ground; its start `s` is 0 when it is the first
///   run, else the run before's reach. A reach is counted from the own cell's
///   centre and rounded to the brief's 10 m, so the true end of the run
///   before can lie up to half a cell past `s` on an axis and up to a
///   diagonal cell past it on a diagonal: the allowance `a` is `cell_m / 2`
///   on an axis and `cell_m × 1.5` on a diagonal (`cell_m` from the header,
///   10 without one). The ray qualifies when `s + a + `[`OPENING_COVER_M`]` ≤
///   `[`OPENING_STEP_MAX_M`] and that run reaches at least `s + a +
///   `[`OPENING_COVER_M`]` + 10`. The smallest `s` wins, ties by compass order
///   from north; the step is `s + a + `[`OPENING_COVER_M`] long.
pub fn opening_step(ground: &Ground, away_deg: u32) -> Option<(u32, f64)> {
    let hides = |class: &str| HIDING.contains(&class);
    let hidden = ground
        .rays
        .values()
        .filter(|(runs, _)| runs.first().is_some_and(|(class, _)| hides(class)))
        .count();
    if hidden >= 6 {
        return None;
    }
    let cell = f64::from(ground.cell_m.unwrap_or(10));
    // (s, bearing, metres); the rays iterate in compass order from north.
    let mut best: Option<(f64, u32, f64)> = None;
    for (&bearing, (runs, _)) in &ground.rays {
        if angle_between(bearing, away_deg) <= 45 {
            continue;
        }
        let Some(index) = runs.iter().position(|(class, _)| hides(class)) else {
            continue;
        };
        let start = index
            .checked_sub(1)
            .and_then(|before| runs.get(before))
            .map_or(0.0, |(_, reach)| f64::from(*reach));
        let Some((_, reach)) = runs.get(index) else {
            continue;
        };
        let allowance = if bearing % 90 == 0 {
            cell / 2.0
        } else {
            cell * 1.5
        };
        let metres = start + allowance + OPENING_COVER_M;
        if metres <= OPENING_STEP_MAX_M
            && f64::from(*reach) >= metres + 10.0
            && best.is_none_or(|(s, _, _)| start < s)
        {
            best = Some((start, bearing, metres));
        }
    }
    best.map(|(_, bearing, metres)| (bearing, metres))
}

/// **The ground's size** (cut AO.1): the brief's header's
/// ([`Ground::side_m`]), else `help simulator`'s "2.56 km square"
/// ([`ARENA_SIDE_M`]). The one source of it: [`Pilot::other_entry_now`] and
/// the back-offs read it here.
pub fn ground_side(picture: &Picture) -> (f64, f64) {
    picture
        .ground
        .as_ref()
        .and_then(|ground| ground.side_m)
        .unwrap_or((ARENA_SIDE_M, ARENA_SIDE_M))
}

/// **The ground astern** (cut AO.1): the distance from `own` along
/// `bow_deg + 180°` to the edge of the ground `[0, side.0] × [0, side.1]`
/// (`STATUS` counts from its south-west corner): the smallest exit over the two axes,
/// an axis the direction does not move along never exiting; `0.0` when `own`
/// lies outside the ground.
pub fn room_astern(own: (f64, f64), bow_deg: f64, side: (f64, f64)) -> f64 {
    let inside = (0.0..=side.0).contains(&own.0) && (0.0..=side.1).contains(&own.1);
    if !inside {
        return 0.0;
    }
    let astern = (bow_deg + 180.0).to_radians();
    let exit = |at: f64, step: f64, high: f64| {
        if step > f64::EPSILON {
            (high - at) / step
        } else if step < -f64::EPSILON {
            at / -step
        } else {
            f64::INFINITY
        }
    };
    exit(own.0, astern.sin(), side.0).min(exit(own.1, astern.cos(), side.1))
}

/// The brief's ray nearest a whole bearing (cut AO.1): 0, 45, … 315.
fn nearest_ray(deg: u32) -> u32 {
    ((deg % 360 + 22) / 45 * 45) % 360
}

/// **Where a ray's ground first stops a tank** (cut AO.1): walking its runs
/// from the own cell, the start (0, else the reach of the run before) of the
/// first run whose class is not [`PASSABLE`]; `None` when every run is.
fn first_block_m(runs: &[(String, u32)]) -> Option<f64> {
    let mut start = 0.0;
    for (class, reach) in runs {
        if !PASSABLE.contains(&class.as_str()) {
            return Some(start);
        }
        start = f64::from(*reach);
    }
    None
}

/// **How far a ray of the brief is clear** (cut AO.1): its
/// [`first_block_m`], else its last run's reach (0 with no run); `None` with
/// no such ray. A clear ray's reach ends at the window or the arena's edge,
/// and nothing past it is known.
fn ray_clear_m(ground: &Ground, bearing: u32) -> Option<f64> {
    let (runs, _) = ground.rays.get(&bearing)?;
    Some(
        first_block_m(runs)
            .unwrap_or_else(|| runs.last().map_or(0.0, |(_, reach)| f64::from(*reach))),
    )
}

/// **The ground behind the opening** (cut AO.1): from the brief's own
/// `from_m`, the [`room_astern`] with the bow on `away_deg` (the bearing from
/// the muster to the other side's entry) on the header's ground (else the
/// help's square), cut short by the ray nearest astern where its ground first
/// stops a tank (`first_block_m`). `None` without `from_m`.
pub fn opening_room(ground: &Ground, away_deg: u32) -> Option<f64> {
    let from = ground.from_m?;
    let side = ground.side_m.unwrap_or((ARENA_SIDE_M, ARENA_SIDE_M));
    let room = room_astern(from, f64::from(away_deg), side);
    let behind = ground
        .rays
        .get(&nearest_ray(away_deg.saturating_add(180)))
        .and_then(|(runs, _)| first_block_m(runs));
    Some(behind.map_or(room, |block| room.min(block)))
}

/// **What the ground behind the opening says** (cut AO.1; [`opening_inward`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RoomVerdict {
    /// Room enough: the shortfall is at most [`OPENING_ROOM_SLACK_M`].
    Enough,
    /// Move in `leg_m` toward the other side's entry; `room_m` behind now.
    MoveIn { leg_m: f64, room_m: f64 },
    /// Too little room, and the ground ahead refuses a leg of
    /// [`OPENING_INWARD_MIN_M`]; `room_m` behind now.
    NoWayIn { room_m: f64 },
}

/// **Whether the opening moves in for room** (cut AO.1), from the brief's
/// `ground` with the bow on `away_deg`; `None` without `from_m`. With `room`
/// its [`opening_room`] and `need` [`OPENING_ROOM_M`] less it: a `need` of at
/// most [`OPENING_ROOM_SLACK_M`] is [`RoomVerdict::Enough`]; otherwise the ray
/// nearest `away_deg` clears `reach` (`ray_clear_m`, less
/// [`OPENING_COVER_M`] so the leg stops short of ground that stops a tank),
/// and a `reach` under [`OPENING_INWARD_MIN_M`] is
/// [`RoomVerdict::NoWayIn`], else the leg is `need` and [`OPENING_ARRIVE_M`],
/// at most `reach` — never under the minimum, since `need` is over the slack.
pub fn opening_inward(ground: &Ground, away_deg: u32) -> Option<RoomVerdict> {
    let room_m = opening_room(ground, away_deg)?;
    let need = OPENING_ROOM_M - room_m;
    if need <= OPENING_ROOM_SLACK_M {
        return Some(RoomVerdict::Enough);
    }
    let reach = ray_clear_m(ground, nearest_ray(away_deg)).unwrap_or(0.0) - OPENING_COVER_M;
    if reach < OPENING_INWARD_MIN_M {
        return Some(RoomVerdict::NoWayIn { room_m });
    }
    Some(RoomVerdict::MoveIn {
        leg_m: (need + OPENING_ARRIVE_M).min(reach),
        room_m,
    })
}

/// **The bow a back-off reverses on** (cut AO.1): the re-approach's and the
/// return's, from `own` with the machine at `aim_deg`, on a ground `side`.
/// `aim_deg` itself when [`room_astern`] of it leaves [`EDGE_BACK_MARGIN_M`];
/// else the **smallest** slant that does, over `k = 1, 2, …` while `k ×`
/// [`BACK_OFF_SLANT_STEP_DEG`] is at most [`HOLD_BOW_DEG`], `aim_deg − k ×
/// step` before `aim_deg + k × step`; else `None` (the back-off ends and
/// holds). A tank reverses only along its bow line ("a negative speed
/// reverses", `help cockpit`), so "along the edge" is a slant of the bow
/// inside the help's front arc ("from within 45 degrees of your bow it
/// strikes the front"), never a flank shown; the smallest slant keeps most of
/// the bow's margin. At an edge square to the line of fire a slant gains
/// little (about 4 m at 30° from 26 m); it is for a machine off the axis.
fn back_off_bow(own: (f64, f64), aim_deg: f64, side: (f64, f64)) -> Option<f64> {
    let roomy = |bow: f64| room_astern(own, bow, side) >= EDGE_BACK_MARGIN_M;
    if roomy(aim_deg) {
        return Some(aim_deg);
    }
    let mut k = 1_u32;
    while k * BACK_OFF_SLANT_STEP_DEG <= HOLD_BOW_DEG {
        let slant = f64::from(k * BACK_OFF_SLANT_STEP_DEG);
        for bow in [aim_deg - slant, aim_deg + slant] {
            if roomy(bow) {
                return Some(norm(bow));
            }
        }
        k += 1;
    }
    None
}

/// How far `own` has come along the leg from `from_m` to `aim_m` (the step's
/// and the move in's projection; 0 with no own position or a leg of no
/// length).
fn progress_along(own: Option<(f64, f64)>, from_m: (f64, f64), aim_m: (f64, f64)) -> f64 {
    let length = distance_m(from_m, aim_m);
    own.map_or(0.0, |own| {
        if length <= f64::EPSILON {
            return 0.0;
        }
        ((own.0 - from_m.0) * (aim_m.0 - from_m.0) + (own.1 - from_m.1) * (aim_m.1 - from_m.1))
            / length
    })
}

/// How old a contact's plot is now: its age when read, and the time since.
fn plot_age_ms(contact: &Contact, now_ms: u64) -> u64 {
    contact
        .age_ms
        .saturating_add(now_ms.saturating_sub(contact.read_ms))
}

/// Degrees into `0..360`.
fn norm(deg: f64) -> f64 {
    deg.rem_euclid(360.0)
}

/// A heading as `HEADING` takes it.
fn whole(deg: f64) -> u32 {
    // `norm` is in 0..360, so the rounded value is 0..=360 and fits.
    (norm(deg).round() as u32) % 360
}

/// The smaller angle between two whole headings.
fn angle_between(a: u32, b: u32) -> u32 {
    let d = a.abs_diff(b) % 360;
    d.min(360 - d)
}

/// `pos` moved `metres` along `bearing`.
fn along(pos: (f64, f64), bearing: f64, metres: f64) -> (f64, f64) {
    let r = bearing.to_radians();
    (pos.0 + metres * r.sin(), pos.1 + metres * r.cos())
}

/// The signed turn from one whole heading to another, in `-179..=180`: positive
/// is clockwise (the heading growing).
fn signed_turn(from: u32, to: u32) -> i64 {
    let d = (i64::from(to) - i64::from(from)).rem_euclid(360);
    if d > 180 { d - 360 } else { d }
}

/// Whether the ray from `o` along `heading` meets `point`: at most
/// [`HELD_MARK_NEAR_M`] ahead along it and within [`HELD_MARK_HALF_WIDTH_M`]
/// of its line — and, with a `clear` distance, only a point more than that
/// from `o`.
fn ray_meets(o: (f64, f64), heading: f64, point: (f64, f64), clear: Option<f64>) -> bool {
    let (dx, dy) = (point.0 - o.0, point.1 - o.1);
    if clear.is_some_and(|clear| dx.hypot(dy) <= clear) {
        return false;
    }
    let r = heading.to_radians();
    let ahead = dx * r.sin() + dy * r.cos();
    let aside = (dx * r.cos() - dy * r.sin()).abs();
    (0.0..=HELD_MARK_NEAR_M).contains(&ahead) && aside <= HELD_MARK_HALF_WIDTH_M
}

/// **Never past the edge**: a point clamped into the proving ground, at least
/// [`EDGE_MARGIN_M`] inside each side of its [`ARENA_SIDE_M`] square.
pub fn clamp_arena(p: (f64, f64)) -> (f64, f64) {
    let (lo, hi) = (EDGE_MARGIN_M, ARENA_SIDE_M - EDGE_MARGIN_M);
    (p.0.clamp(lo, hi), p.1.clamp(lo, hi))
}

/// Whether [`clamp_arena`] moves `p`.
fn past_the_edge(p: (f64, f64)) -> bool {
    let inside = EDGE_MARGIN_M..=ARENA_SIDE_M - EDGE_MARGIN_M;
    !inside.contains(&p.0) || !inside.contains(&p.1)
}

/// **The hull is failing**: the `hull` section's internals `(left, max)` are
/// at half or less (`max > 0 && left * 2 <= max`). The fall back's first rule,
/// and the one a commander's `withdraw` is kept on (cut AK.2).
pub fn hull_failing(picture: &Picture) -> bool {
    picture
        .own
        .sections
        .get("hull")
        .and_then(|section| section.internals)
        .is_some_and(|(left, max)| max > 0 && left.saturating_mul(2) <= max)
}

/// **Whether the pilot falls back, and why**: [`hull_failing`], or else the
/// first facet **of the `hull` section only** whose plate is gone (`left ==
/// 0`, which also reads a `(0, 0)` stored by an `armour gone` line on a facet
/// never read) or at a quarter (`max > 0 && left * FALL_BACK_FACET_DIVISOR <=
/// max`). A turret's or a track's facet never falls it back.
pub fn fall_back_due(picture: &Picture) -> Option<FallBackWhy> {
    if hull_failing(picture) {
        return Some(FallBackWhy::HullHalf);
    }
    let hull = picture.own.sections.get("hull")?;
    hull.facets.iter().find_map(|(facet, (left, max))| {
        let gone = *left == 0;
        (gone || (*max > 0 && left.saturating_mul(FALL_BACK_FACET_DIVISOR) <= *max)).then(|| {
            FallBackWhy::Plate {
                facet: facet.clone(),
                gone,
            }
        })
    })
}

/// **The widened band** after a fall back: [`FALL_BACK_WIDEN_M`] wider than
/// `band_m`, never past [`FALL_BACK_MAX_M`], and never narrower than `band_m`.
pub fn widened_band(band_m: u32) -> u32 {
    band_m.max(
        band_m
            .saturating_add(FALL_BACK_WIDEN_M)
            .min(FALL_BACK_MAX_M),
    )
}

/// The course leg's moving aim: the last plot dead-reckoned along the
/// estimated course by the plot's age, at most [`HUNT_ALONG_MAX_M`] on.
fn along_aim(hunt: &Hunt, now_ms: u64) -> (f64, f64) {
    let run_m = hunt.speed_mps * now_ms.saturating_sub(hunt.trail.plot_ms) as f64 / 1_000.0;
    along(
        hunt.trail.plot_m,
        hunt.course_deg,
        run_m.min(HUNT_ALONG_MAX_M),
    )
}

/// A leg's aim, clamped into the ground, and whether the clamp moved it.
fn leg_aim(hunt: &Hunt, now_ms: u64) -> ((f64, f64), bool) {
    let raw = match &hunt.leg {
        HuntLeg::ToPlot => hunt.trail.plot_m,
        HuntLeg::Along => along_aim(hunt, now_ms),
        HuntLeg::Sweep {
            origin,
            heading_deg,
            ..
        } => along(*origin, *heading_deg, HUNT_SWEEP_LEG_M),
    };
    (clamp_arena(raw), past_the_edge(raw))
}

/// **Where the hunted hostile is estimated to be** (cut AL.3), as the
/// current leg aims before clamping: the last plot on the way to it and on the
/// sweep, the dead-reckoned point on the course leg.
fn wait_estimate(hunt: &Hunt, now_ms: u64) -> (f64, f64) {
    match &hunt.leg {
        HuntLeg::ToPlot | HuntLeg::Sweep { .. } => hunt.trail.plot_m,
        HuntLeg::Along => along_aim(hunt, now_ms),
    }
}

/// **How old a hostile's plot may be and still be fought** rather than hunted:
/// [`HUNT_STALE_PLOT_MS`], or under a `hunt` doctrine (cut AK.2) only a fresh
/// plot, [`FRESH_PLOT_MS`] — a commander that says hunt wants the last one
/// lost hunted at once, not a row a few seconds old circled.
fn held_limit_ms(doctrine: &Doctrine) -> u64 {
    if doctrine.stance == Stance::Hunt {
        FRESH_PLOT_MS
    } else {
        HUNT_STALE_PLOT_MS
    }
}

/// Whether a weapon is believed ready now: its one clock
/// ([`super::picture::Weapon::next_ready_ms`]) when anything has set it, else
/// a `ready` row read well after the last ack.
fn believed_ready(weapon: &super::picture::Weapon, now_ms: u64) -> bool {
    match weapon.next_ready_ms {
        Some(at) => now_ms >= at,
        None => {
            weapon.ready
                && weapon.read_ms > 0
                && weapon
                    .fired_ms
                    .is_none_or(|fired| weapon.read_ms > fired.saturating_add(READY_AFTER_ACK_MS))
        }
    }
}

impl Pilot {
    /// **The pilot as it was before cut AN.2** ([`Knobs::off`]): every older
    /// unit test flies it.
    pub fn new() -> Pilot {
        Pilot::default()
    }

    /// **A pilot flying `knobs`** (cut AN.2): what the lane builds from
    /// `[arena]` ([`super::settings::Settings::knobs`]).
    pub fn with_knobs(knobs: Knobs) -> Pilot {
        Pilot {
            knobs,
            opening: match knobs.opening {
                Opening::Wait => OpeningStage::Fresh,
                Opening::Search => OpeningStage::Done,
            },
            ..Pilot::default()
        }
    }

    /// **The rule flown at the last decision** (cut AN.2): `opening` (step
    /// 6a), `halt` (3c), `reapproach` (3a, backing off) or `approach_wait`
    /// (3a, waiting); cut AN.3: `follow` (4, a broken machine followed home),
    /// `return_back` (3a, backing out of its sight) or `return_wait` (3a,
    /// waiting for its return); else `None`. A word, never a designator.
    /// Cut AO.1 adds no word: a move in for room is `opening`, a back-off's
    /// hold at the edge is its stage's own wait (`approach_wait`,
    /// `return_wait`).
    pub fn rule(&self) -> Option<&'static str> {
        self.rule
    }

    /// **Whether `contact` is broken** (cut AN.3; cut AO.1; the module head),
    /// seen from `own` at `now_ms`: a live hostile held young
    /// ([`HUNT_STALE_PLOT_MS`]), hurt ([`Pilot::hurt`]) or hit at least
    /// [`BROKEN_HITS_MIN`] times this match ([`Pilot::scored_on`]), and
    /// driving away — either a **plausible** track (at most
    /// [`BROKEN_TRACK_MAX_MPS`]: a faster one is read as none) whose course
    /// lies within [`BROKEN_AWAY_DEG`] of the bearing from `own` to where it is
    /// now ([`target_pos`]), or no plausible track and [`Pilot::leaving`].
    pub fn broken(
        &self,
        picture: &Picture,
        contact: &Contact,
        own: (f64, f64),
        now_ms: u64,
    ) -> bool {
        let Some(other) = picture.side.map(Side::other) else {
            return false;
        };
        let d = &contact.designator;
        let hurt = self.hurt.contains(d)
            || self
                .scored_on
                .get(d)
                .is_some_and(|hits| *hits >= BROKEN_HITS_MIN);
        let away = match plausible_track(contact) {
            Some(track) => target_pos(picture, contact, now_ms).is_some_and(|pos| {
                angle_between(whole(track.course_deg), whole(bearing_deg(own, pos)))
                    <= BROKEN_AWAY_DEG
            }),
            None => self.leaving(contact, own, now_ms),
        };
        self.live_hostile(contact, other) && contact.age_ms <= HUNT_STALE_PLOT_MS && hurt && away
    }

    /// **A machine leaving with no track** (cut AO.1): over `contact`'s fixes
    /// of the last [`BROKEN_OPEN_WINDOW_MS`], the newest fix lies at least
    /// [`BROKEN_OPEN_M`] farther from `own` than the nearest of those at least
    /// [`BROKEN_OPEN_SPAN_MS`] older than it. Both distances are from **the
    /// own position now**, so the tank's own motion cancels and only the
    /// machine moving away counts. The picture publishes a track only for
    /// motion its fixes can carry, and a row-only fix at 500 to 600 m has
    /// about 55 m of error, so a 10 m/s machine driving home seldom shows one
    /// (the bot's own runs hunted five such machines into their entry zones).
    pub fn leaving(&self, contact: &Contact, own: (f64, f64), now_ms: u64) -> bool {
        let floor = now_ms.saturating_sub(BROKEN_OPEN_WINDOW_MS);
        let recent = || contact.fixes.iter().filter(move |fix| fix.at_ms >= floor);
        let Some(newest) = recent().max_by_key(|fix| fix.at_ms) else {
            return false;
        };
        let nearest = recent()
            .filter(|fix| fix.at_ms.saturating_add(BROKEN_OPEN_SPAN_MS) <= newest.at_ms)
            .map(|fix| distance_m(own, fix.pos_m))
            .min_by(f64::total_cmp);
        nearest.is_some_and(|nearest| distance_m(own, newest.pos_m) - nearest >= BROKEN_OPEN_M)
    }

    /// **How far `q` lies short of the other side's entry line** (cut AN.3):
    /// `(other − q) · u`, with `u` the unit vector from the own muster to the
    /// other side's entry ([`Pilot::other_entry_now`]). A line square to the
    /// approach axis, not a point: the help names the entry zone but not its
    /// shape, so the pilot keeps out of a band across the whole axis. `None`
    /// with no muster (no entry line), or with the two entries one point.
    fn short_of_entry(&self, picture: &Picture, q: (f64, f64)) -> Option<f64> {
        let muster = self.muster_m?;
        let other = self.other_entry_now(picture)?;
        let length = distance_m(muster, other);
        if length < 1.0 {
            return None;
        }
        let u = ((other.0 - muster.0) / length, (other.1 - muster.1) / length);
        Some((other.0 - q.0) * u.0 + (other.1 - q.1) * u.1)
    }

    /// **The exchange over its window** at `now_ms` (cut AN.2): `(taken,
    /// scored)`.
    pub fn exchange_counts(&self, now_ms: u64) -> (usize, usize) {
        let inside = |at: u64| now_ms.saturating_sub(at) <= EXCHANGE_WINDOW_MS;
        let taken = self
            .exchange
            .iter()
            .filter(|(at, taken)| *taken && inside(*at))
            .count();
        let scored = self
            .exchange
            .iter()
            .filter(|(at, taken)| !*taken && inside(*at))
            .count();
        (taken, scored)
    }

    /// **The other side's entry now** (cut AN.2): [`other_entry`] of the own
    /// muster, on the ground the brief's header sized, else on `help
    /// simulator`'s "2.56 km square" ([`ARENA_SIDE_M`]), as [`ground_side`]
    /// reads it; `None` with no muster.
    pub fn other_entry_now(&self, picture: &Picture) -> Option<(f64, f64)> {
        let side = ground_side(picture);
        self.muster_m.map(|muster| other_entry(muster, side))
    }

    /// **Decide.** What to type now, given the picture and the doctrine.
    pub fn decide(&mut self, picture: &Picture, doctrine: &Doctrine, now_ms: u64) -> Orders {
        let mut orders = Orders::default();
        // Rule 1: nothing unless flying a powered tank in a running match, and
        // nothing ever once it is destroyed.
        if picture.own.destroyed
            || !matches!(picture.phase, Phase::Running { .. })
            || !picture.aboard
            || picture.own.power != Power::Online
        {
            return orders;
        }
        let Some(side) = picture.side else {
            return orders;
        };
        let other = side.other();
        // The doctrine's own sense, when it names one, is `s` everywhere — the
        // orbit, a `Held:` turn, the stall and the sweep alike; `Auto` keeps
        // the pilot's running one. With one track gone the compass turns go the
        // live way instead (cut AL.2): the turn sign is taken once, here.
        match doctrine.sense {
            Sense::Cw => self.sense = -1,
            Sense::Ccw => self.sense = 1,
            Sense::Auto => {}
        }
        let live = live_turn(picture);
        let sign = self.turn_sign(picture);
        self.turn_wanted = None;
        if let Some(turn) = live
            && !self.one_track_said
        {
            self.one_track_said = true;
            self.note(format!(
                "The {} is gone: turning {} only.",
                turn.gone_track(),
                turn.words()
            ));
        }
        self.observe(picture, doctrine, other, sign, now_ms);

        // Rule 3: the target.
        let target = self.target(picture, doctrine, other);

        // Rule 4: the lock.
        if let Some(target) = &target
            && picture.lock.as_deref() != Some(target.as_str())
            && self
                .last_lock_order_ms
                .is_none_or(|at| now_ms >= at.saturating_add(LOCK_EVERY_MS))
        {
            orders.lock = Some(target.clone());
        }

        // Rule 5: fire.
        if let Some(target) = &target {
            orders.fire = fireable(picture, target, other, now_ms);
        }

        // Rules 2 and 6: the speed ceilings, then movement.
        if picture.own.immobile || picture.own.disabled {
            // The waits close while immobile (cut AM.2), as they do whenever
            // no step flies them.
            self.idle_motion(now_ms);
            return orders;
        }
        if picture.own.max_ahead_kph.is_some() {
            self.ceilings_asked = true;
        }
        self.take_hit(picture, doctrine, other, target.as_deref(), live, now_ms);
        // Cut AN.2: the opening, the re-approach and the halt are judged
        // before any step flies (and before the hunt's start, which a
        // re-approach holds back).
        self.judge(picture, doctrine, other, target.as_deref(), now_ms);
        self.follow(picture, doctrine, other, target.as_deref(), sign, now_ms);
        let full = picture.own.max_ahead_kph.unwrap_or(UNKNOWN_FULL_KPH);
        let motion = self.motion(
            picture,
            doctrine,
            target.as_deref(),
            full,
            live,
            sign,
            now_ms,
        );
        let (heading, mut speed, ask) = motion;
        orders.ask_spot_b = ask;
        // Cut AN.2: the opening asks for its brief until it has it.
        orders.ask_view_brief = matches!(self.opening, OpeningStage::Brief { .. });
        if !self.ceilings_asked {
            speed = Some(CEILING_ASK_KPH);
        }
        // One track gone (cut AL.2): a small turn toward the dead side is not
        // ordered, a large one once and then nothing until it is read.
        let heading = heading.and_then(|deg| match live {
            Some(turn) => self.one_track(picture, turn, whole(deg), now_ms),
            None => Some(whole(deg)),
        });
        if let Some(deg) = heading {
            let due = match self.last_heading_order {
                None => true,
                Some((last, at)) => {
                    let diff = angle_between(deg, last);
                    diff >= HEADING_STEP_DEG
                        || (diff > 0 && now_ms >= at.saturating_add(HEADING_REFRESH_MS))
                }
            };
            if due {
                orders.heading = Some(deg);
            }
        }
        if let Some(kph) = speed
            && self.last_speed_order != Some(kph)
        {
            orders.speed = Some(kph);
        }
        orders
    }

    /// The lane typed `HEADING deg` (with one track gone, a dead-side heading
    /// the pilot asked for is committed to from now, cut AL.2).
    pub fn sent_heading(&mut self, deg: u32, now_ms: u64) {
        self.last_heading_order = Some((deg, now_ms));
        if self.turn_wanted == Some(deg) {
            self.turn_wanted = None;
            self.turn_commit = Some((deg, now_ms));
        }
    }

    /// The lane typed `SPEED kph`.
    pub fn sent_speed(&mut self, kph: i32) {
        self.last_speed_order = Some(kph);
        if kph == CEILING_ASK_KPH {
            self.ceilings_asked = true;
        }
    }

    /// The lane typed a `LOCK`.
    pub fn sent_lock(&mut self, now_ms: u64) {
        self.last_lock_order_ms = Some(now_ms);
    }

    /// A heading order went unanswered: order it again when re-decided.
    pub fn forget_heading(&mut self) {
        self.last_heading_order = None;
    }

    /// A speed order went unanswered: order it again when re-decided (the
    /// ceilings too, if that is what was asked).
    pub fn forget_speed(&mut self) {
        if self.last_speed_order == Some(CEILING_ASK_KPH) {
            self.ceilings_asked = false;
        }
        self.last_speed_order = None;
    }

    /// **Back at the controls** (cut AL.2; the lane calls it where it forgets
    /// the heading and the speed): the one-track commit and the turn into a hit
    /// are dropped too, and the newest hit in `picture` is taken as read. The
    /// machine flew the tank meanwhile and it "is yours again where it
    /// stands", so the heading a commit waits for and the bearing a hit came
    /// from — a facet of the hull as it lay then — are not the tank's now.
    pub fn forget_turns(&mut self, picture: &Picture) {
        self.turn_commit = None;
        self.turn_wanted = None;
        self.threat = None;
        self.threat_note = None;
        if let Some(hit) = &picture.own.last_hit {
            self.threat_seen_ms = self.threat_seen_ms.max(hit.at_ms);
        }
    }

    /// **The compass turn sign** (cut AL.2): `+1` clockwise (the heading
    /// growing), `-1` counter-clockwise. With one track gone, the live turn's
    /// ([`live_turn`]); else the orbit's sense as a number, exactly as cut AK
    /// turned by it. Not the orbit's circling sense: the held turn, the
    /// stall's turns, the sweep's legs and the detour round held ground turn
    /// by this.
    pub fn turn_sign(&self, picture: &Picture) -> f64 {
        match live_turn(picture) {
            Some(turn) => turn.compass_sign(),
            None => f64::from(self.sense),
        }
    }

    /// **The reference heading** (cut AL.2): the last heading ordered when it
    /// went after the last `STATUS` read (at or after its position's time),
    /// else the heading that `STATUS` read. What a one-track turn, the arc of
    /// a side hit and a heading-less reverse are measured from.
    pub fn reference_heading(&self, picture: &Picture) -> u32 {
        match self.last_heading_order {
            Some((deg, at)) if at >= picture.own.pos_read_ms => deg,
            _ => picture.own.heading_deg,
        }
    }

    /// **Whether a forward heading `h` from `o` meets held ground**: some
    /// mark's blocked point more than [`HELD_MARK_CLEAR_M`] from `o` lies at
    /// most [`HELD_MARK_NEAR_M`] ahead along `h` and within
    /// [`HELD_MARK_HALF_WIDTH_M`] of its line.
    pub fn blocked(&self, o: (f64, f64), h: f64) -> bool {
        self.held_marks
            .iter()
            .any(|mark| ray_meets(o, h, mark.blocked_m, Some(HELD_MARK_CLEAR_M)))
    }

    /// **Whether a reverse with the bow on `bow` from `o` meets held ground**:
    /// the ray behind the tank (`bow + 180`). A mark taken while reversing is
    /// met however near (it lies just behind the tank that stopped on it); a
    /// mark taken driving ahead keeps its clear distance, as for a forward
    /// heading, so the back-off from a forward stop is not stopped by its own
    /// stop's mark.
    pub fn blocked_behind(&self, o: (f64, f64), bow: f64) -> bool {
        self.held_marks.iter().any(|mark| {
            let clear = (!mark.astern).then_some(HELD_MARK_CLEAR_M);
            ray_meets(o, bow + 180.0, mark.blocked_m, clear)
        })
    }

    /// **Steer round held ground** (cut AL.2): a forward heading `h` from `o`
    /// that [`Pilot::blocked`] is turned `k ×` [`HELD_MARK_DETOUR_DEG`] by the
    /// compass turn sign, `k` = 1, 2, 3, the first that is clear winning; all
    /// three blocked, `h` is left as it is. Says so when a detour starts.
    pub fn steer_round(&mut self, o: (f64, f64), h: f64, sign: f64) -> f64 {
        if !self.blocked(o, h) {
            self.detour = false;
            return h;
        }
        for k in 1..=3_u8 {
            let aside = norm(h + f64::from(k) * HELD_MARK_DETOUR_DEG * sign);
            if !self.blocked(o, aside) {
                if !self.detour {
                    self.note(format!(
                        "Steering round held ground: {:03}° for {:03}°.",
                        whole(aside),
                        whole(h)
                    ));
                }
                self.detour = true;
                return aside;
            }
        }
        self.detour = false;
        h
    }

    /// **What the pilot said** since the last call, oldest first, verbatim
    /// (the lane hands each to the runner as an `arena` event): a hunt's
    /// start, legs and end, a fall back and its reason, a held tank turned
    /// further or backed off. Designators in them are ones the stream named.
    pub fn take_notes(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notes)
    }

    // -- internals -------------------------------------------------------------

    fn note(&mut self, text: String) {
        self.notes.push(text);
    }

    /// Fold in what the picture has learned since the last decision: when power
    /// came up, where the pilot started, where The Crossroads is, whether a
    /// hostile is held, a `Held:` notice, the stall watchdog, and the fall-back
    /// rule.
    fn observe(
        &mut self,
        picture: &Picture,
        doctrine: &Doctrine,
        other: Side,
        sign: f64,
        now_ms: u64,
    ) {
        if self.power_ms.is_none() {
            // The first decision (cut AN.2): the opening waits for the first
            // `CONTACTS` reply after it.
            self.opening_contacts_seen = picture.contacts_read;
        }
        let power_ms = *self.power_ms.get_or_insert(now_ms);
        if self.last_hostile_ms.is_none() {
            self.last_hostile_ms = Some(power_ms);
        }
        // Events from before this match's power came up are not this
        // pilot's (the picture's ring outlives a match).
        self.held_seen_ms = self.held_seen_ms.max(power_ms);
        self.kill_seen_ms = self.kill_seen_ms.max(power_ms);
        if self.start_pos.is_none()
            && let Some(pos) = picture.own.pos_m
            && picture.own.pos_read_ms >= power_ms
        {
            self.start_pos = Some(pos);
            // Cut AN.2: only a start the pilot saw is a muster.
            let seen = picture.powered_here
                && picture.power_ms.is_some_and(|power| {
                    picture.own.pos_read_ms <= power.saturating_add(OPENING_BRIEF_WAIT_MS)
                });
            self.muster_m = seen.then_some(pos);
        }
        // The Crossroads' own position, as the picture plotted it from a
        // `STATUS` block that named it nearest (range and bearing from the
        // block's own position; a `BEARING B` answer never moves it).
        if let Some((x, y)) = picture
            .spots
            .get(&SEARCH_SPOT)
            .and_then(|spot| spot.plotted_m)
        {
            self.spot_b_pos = Some((x as f64, y as f64));
        }
        // A kill, before anything reads the contacts: its wreck is no hostile.
        self.take_kills(picture);
        // Seen second (cut AL.3): the hostiles hurt, the newest hit taken, and
        // when each return not yet told apart was first seen.
        self.take_seen_second(picture, power_ms, now_ms);
        // The exchange (cut AN.2), whatever the knobs.
        self.take_exchange(picture, power_ms, now_ms);
        // Only a row young enough to be held counts (cut AK.1): a stale row
        // still listed after a failed hunt does not keep the search away, and
        // nor does a wreck's row, listed until it expires.
        if picture
            .contacts
            .values()
            .any(|c| self.live_hostile(c, other) && c.age_ms <= HUNT_STALE_PLOT_MS)
        {
            self.last_hostile_ms = Some(now_ms);
        }
        if let Some((held, text)) = picture
            .events
            .iter()
            .filter(|event| event.kind == EventKind::Held && event.at_ms > self.held_seen_ms)
            .max_by_key(|event| event.at_ms)
            .map(|event| (event.at_ms, event.text.clone()))
        {
            self.held_seen_ms = held;
            // The heading the tank was stopped on: every further turn is
            // measured from it, never from a later reading.
            self.held_from_deg = Some(picture.own.heading_deg);
            // Stopped while reversing (cut AL.2; read before the speed is
            // forgotten below): no forward held turn — step 1 stops the tank
            // with its bow kept.
            let astern = self.last_speed_order.is_some_and(|kph| kph < 0);
            self.mark_ground(picture, &text, astern, now_ms);
            self.held_astern = astern;
            self.held_heading =
                (!astern).then(|| whole(f64::from(picture.own.heading_deg) + HELD_TURN_DEG * sign));
            self.held_until_ms = now_ms.saturating_add(HELD_FOR_MS);
            // Insurance, not the fix: the speed goes again straight after the
            // turn (no transcript shows a `SPEED` freeing a held tank).
            self.forget_speed();
            // A new stop is a new episode: any stall starts again from level
            // 0, measured from this stop's heading.
            self.stall_since_ms = None;
            self.stall_level = 0;
            self.stall_heading = None;
            self.stall_clear_until_ms = 0;
        }
        // The back-off is over: the forward speed goes again, and the stall
        // closes at level 0.
        if self.reverse_until_ms > 0 && now_ms >= self.reverse_until_ms {
            self.reverse_until_ms = 0;
            self.forget_speed();
            self.stall_level = 0;
            self.stall_since_ms = None;
        }
        self.watch_stall(picture, sign, now_ms);
        if !self.fallen_back
            && let Some(why) = fall_back_due(picture)
        {
            self.fallen_back = true;
            self.withdraw_until_ms = Some(now_ms.saturating_add(WITHDRAW_MS));
            let band = widened_band(doctrine.band_m);
            self.note(match why {
                FallBackWhy::HullHalf => format!("Falling back to {band} m: the hull is at half."),
                FallBackWhy::Plate { facet, gone: false } => {
                    format!("Falling back to {band} m: the hull's {facet} plate is at a quarter.")
                }
                FallBackWhy::Plate { facet, gone: true } => {
                    format!("Falling back to {band} m: the hull's {facet} plate is gone.")
                }
            });
        }
    }

    /// **The stall watchdog**, judged once per new `STATUS` read. Held at
    /// 0 kph after the held turn (and any back-off) opens a stall; still held
    /// [`STALL_MS`] or more after it was opened or last raised raises it: levels
    /// 1 to 3 turn to 90°, 135° and 180° from the heading the tank was stopped
    /// on, level 4 backs off for [`HELD_REVERSE_MS`] and closes it at level 0.
    /// A read that is not held at 0 kph closes it, keeping a turned heading
    /// [`HELD_FOR_MS`] more to clear the ground. A stall opens only while the
    /// speed the pilot's own motion last chose was ahead (cut AL.2): a tank
    /// the pilot itself stopped, or reverses, is never "stalled". The turns go
    /// by the compass turn sign `sign` ([`Pilot::turn_sign`]).
    fn watch_stall(&mut self, picture: &Picture, sign: f64, now_ms: u64) {
        if picture.own.pos_read_ms <= self.stall_status_ms {
            return;
        }
        self.stall_status_ms = picture.own.pos_read_ms;
        let held = picture.own.held.is_some() && picture.own.speed_kph == 0;
        if !held {
            if self.stall_level >= 1 && self.stall_heading.is_some() {
                self.stall_clear_until_ms = now_ms.saturating_add(HELD_FOR_MS);
            }
            // Free on a read taken after the held turn (a read inside it may
            // be a frame from before the stop — "you and the arena are a pulse
            // apart"): the episode is over, and a later stall with no notice
            // read is measured from its own heading, not this one's.
            if picture.own.pos_read_ms >= self.held_until_ms {
                self.held_from_deg = None;
            }
            self.stall_since_ms = None;
            self.stall_level = 0;
            return;
        }
        match self.stall_since_ms {
            None => {
                if now_ms >= self.held_until_ms
                    && now_ms >= self.reverse_until_ms
                    && self.last_chosen_speed.is_some_and(|kph| kph > 0)
                {
                    self.stall_since_ms = Some(now_ms);
                    // Held with no notice read (a re-board, say): the heading
                    // it reads now is the one it was stopped on.
                    self.held_from_deg.get_or_insert(picture.own.heading_deg);
                }
            }
            Some(since) if now_ms >= since.saturating_add(STALL_MS) => {
                self.stall_level = self.stall_level.saturating_add(1);
                self.stall_since_ms = Some(now_ms);
                self.forget_speed();
                if self.stall_level <= 3 {
                    let from = self.held_from_deg.unwrap_or(picture.own.heading_deg);
                    let deg = whole(
                        f64::from(from) + HELD_TURN_DEG * f64::from(self.stall_level + 1) * sign,
                    );
                    self.stall_heading = Some(deg);
                    self.forget_heading();
                    self.note(format!("Held at 0 kph: turning to {deg:03}°."));
                } else {
                    // Level 4 stands while the tank backs off; the back-off's
                    // end closes the stall (in `observe`).
                    self.reverse_until_ms = now_ms.saturating_add(HELD_REVERSE_MS);
                    self.stall_heading = None;
                    self.stall_since_ms = None;
                    self.note("Held at 0 kph: backing off.".to_owned());
                }
            }
            Some(_) => {}
        }
    }

    /// **Mark held ground** (cut AL.2): a `Held:` that names ground
    /// ([`HELD_GROUND`]), with a known position, is marked where the tank is
    /// now, on the heading it stopped on, the blocked point
    /// [`HELD_MARK_AHEAD_M`] ahead of it — or behind it, `astern`. At most
    /// [`HELD_MARKS_MAX`] are kept, the oldest dropped.
    fn mark_ground(&mut self, picture: &Picture, text: &str, astern: bool, now_ms: u64) {
        if !HELD_GROUND.contains(&text) {
            return;
        }
        let Some(at_m) = picture.own_pos_at(now_ms) else {
            return;
        };
        let heading_deg = picture.own.heading_deg;
        let way = f64::from(heading_deg) + if astern { 180.0 } else { 0.0 };
        self.held_marks.push(HeldMark {
            at_m,
            heading_deg,
            astern,
            blocked_m: along(at_m, way, HELD_MARK_AHEAD_M),
        });
        if self.held_marks.len() > HELD_MARKS_MAX {
            self.held_marks.remove(0);
        }
        self.note(format!(
            "Held ground marked at {:.0} m east, {:.0} m north, heading {heading_deg:03}°.",
            at_m.0, at_m.1
        ));
    }

    /// **A side or rear hit turns the bow to the shooter** (cut AL.2), read
    /// from the picture's newest facet hit ([`super::picture::Own::last_hit`])
    /// once. A `front` or `top` hit does nothing, and nor does any hit under an
    /// `orbit` doctrine. The struck arc is the help's 45-degree face of the
    /// facet, degrees clockwise from the bow (`left` 225..=315, `right`
    /// 45..=135, `rear` 135..=225); a hostile is in it when its own row's
    /// relative bearing lies in it (a live hostile with a row, not a wreck,
    /// its plot no older than [`HUNT_STALE_PLOT_MS`]). The threat is the
    /// nearest such hostile other than the held target; with a young target
    /// held and none other in the arc the target is taken as the shooter (the
    /// hold already faces it: any other turn is dropped); with none held and
    /// none in the arc, the arc's centre from the reference heading (the line
    /// says nothing of who fired). A newer hit re-arms the turn only when its
    /// threat is more than [`HOLD_BOW_DEG`] from the current one, so two
    /// shooters in a crossfire do not thrash it.
    ///
    /// Three hits turn nothing (fixes after review): one read when its turn
    /// would already be over (read late, after a stretch immobile or
    /// disabled); one with nothing to blame while the current turn's heading
    /// is ordered and not yet read by `STATUS` — the hull is somewhere in the
    /// turn, so the arc's centre from the ordered heading would send the bow
    /// round another quarter, away from the shooter, at every hit; and, with
    /// one track gone ([`live_turn`]), one whose turn goes the dead way — the
    /// server turns that the long, live way round, longer than the turn lasts
    /// and showing a side and the stern to the target on the way (within
    /// [`ONE_TRACK_DEADBAND_DEG`] it would not be ordered at all, and the
    /// front already faces it). The sentence is said when step 3b first flies
    /// the turn, never when a higher step outranks it for its whole time.
    #[allow(clippy::too_many_arguments)]
    fn take_hit(
        &mut self,
        picture: &Picture,
        doctrine: &Doctrine,
        other: Side,
        target: Option<&str>,
        live: Option<Turn>,
        now_ms: u64,
    ) {
        let Some(hit) = picture.own.last_hit.as_ref() else {
            return;
        };
        if hit.at_ms <= self.threat_seen_ms {
            return;
        }
        self.threat_seen_ms = hit.at_ms;
        if doctrine.stance == Stance::Orbit || now_ms >= hit.at_ms.saturating_add(THREAT_TURN_MS) {
            return;
        }
        let (arc, centre) = match hit.facet.as_str() {
            "left" => (225..=315, 270),
            "right" => (45..=135, 90),
            "rear" => (135..=225, 180),
            _ => return,
        };
        let Some(own) = picture.own_pos_at(now_ms) else {
            return;
        };
        let young = target
            .and_then(|t| picture.contacts.get(t))
            .filter(|c| self.live_hostile(c, other) && c.age_ms <= held_limit_ms(doctrine));
        let shooter = picture
            .contacts
            .values()
            .filter(|c| {
                self.live_hostile(c, other)
                    && c.range_m.is_some()
                    && c.age_ms <= HUNT_STALE_PLOT_MS
                    && arc.contains(&c.rel_deg)
                    && young.is_none_or(|t| t.designator != c.designator)
            })
            .min_by_key(|c| c.range_m.unwrap_or(u32::MAX));
        let (bearing, facing) = match (shooter, young) {
            (Some(c), _) => {
                let bearing = target_pos(picture, c, now_ms)
                    .map_or(f64::from(c.bearing_deg), |pos| bearing_deg(own, pos));
                (bearing, c.designator.clone())
            }
            (None, Some(t)) => {
                // The held target is the shooter: the hold already faces it.
                if let (Some((current, _)), Some(pos)) =
                    (self.threat, target_pos(picture, t, now_ms))
                    && angle_between(whole(current), whole(bearing_deg(own, pos))) > HOLD_BOW_DEG
                {
                    self.threat = None;
                    self.threat_note = None;
                }
                return;
            }
            (None, None) => {
                // Mid-turn, with the turn's heading ordered and not yet read:
                // where the hull lies is not known, so the arc is not either.
                if self.threat.is_some_and(|(_, until)| now_ms < until)
                    && self
                        .last_heading_order
                        .is_some_and(|(_, at)| at >= picture.own.pos_read_ms)
                {
                    return;
                }
                let bearing = norm(f64::from(self.reference_heading(picture)) + f64::from(centre));
                (bearing, format!("{:03}°", whole(bearing)))
            }
        };
        // One track gone: a turn the dead way is the long way round (a turn of
        // exactly 180° is as long either way, and goes).
        if let Some(turn) = live {
            let s = signed_turn(self.reference_heading(picture), whole(bearing));
            if turn.is_dead(s) && s.unsigned_abs() < 180 {
                return;
            }
        }
        if let Some((current, until)) = self.threat
            && now_ms < until
            && angle_between(whole(current), whole(bearing)) <= HOLD_BOW_DEG
        {
            return;
        }
        self.threat = Some((norm(bearing), hit.at_ms.saturating_add(THREAT_TURN_MS)));
        self.threat_note = Some(format!(
            "Turning into the shot: hit on the {}'s {} facet; facing {facing}.",
            hit.plate, hit.facet
        ));
    }

    /// **One track gone** (cut AL.2): whether to order `deg` now. While a
    /// dead-side heading is committed, nothing, until a `STATUS` read after it
    /// reads within [`HEADING_STEP_DEG`] of it or [`ONE_TRACK_COMMIT_MS`] has
    /// passed. Else a heading whose turn from the reference heading is toward
    /// the dead side by at most [`ONE_TRACK_DEADBAND_DEG`] is not ordered; one
    /// by more is ordered and committed to once the lane types it; a live-side
    /// heading is ordered as ever.
    fn one_track(&mut self, picture: &Picture, turn: Turn, deg: u32, now_ms: u64) -> Option<u32> {
        if let Some((committed, at)) = self.turn_commit {
            let read = picture.own.pos_read_ms > at
                && angle_between(picture.own.heading_deg, committed) <= HEADING_STEP_DEG;
            if !read && now_ms < at.saturating_add(ONE_TRACK_COMMIT_MS) {
                return None;
            }
            self.turn_commit = None;
        }
        let s = signed_turn(self.reference_heading(picture), deg);
        if !turn.is_dead(s) {
            return Some(deg);
        }
        if s.unsigned_abs() <= u64::from(ONE_TRACK_DEADBAND_DEG) {
            return None;
        }
        self.turn_wanted = Some(deg);
        Some(deg)
    }

    /// Rule 3: the doctrine's target when it is a held hostile young enough to
    /// be fought ([`held_limit_ms`]), or when no other hostile is; else the
    /// nearest fresh hostile, keeping the lock's contact unless another is at
    /// least 100 m nearer. Never an unidentified contact, never a friend, never
    /// a wreck (cut AK.1: its row stays listed until it expires, and the pilot
    /// neither orbits nor shoots it). A doctrine naming a row gone stale while
    /// another hostile is in sight (cut AK.2) would otherwise leave the pilot
    /// with nothing to fly at: too old to fight, its trail not hunted while
    /// the other is held, and the search kept away by the other.
    fn target(&self, picture: &Picture, doctrine: &Doctrine, other: Side) -> Option<String> {
        let young =
            |c: &Contact| self.live_hostile(c, other) && c.age_ms <= held_limit_ms(doctrine);
        if let Some(wanted) = &doctrine.target
            && let Some(contact) = picture.contacts.get(wanted)
            && self.live_hostile(contact, other)
            && (young(contact) || !picture.contacts.values().any(young))
        {
            return Some(wanted.clone());
        }
        let best = picture
            .contacts
            .values()
            .filter(|c| self.live_hostile(c, other))
            .min_by_key(|c| (c.age_ms > FRESH_PLOT_MS, c.range_m.unwrap_or(u32::MAX)))?;
        if let Some(lock) = &picture.lock
            && !picture.lock_friendly
            && let Some(held) = picture.contacts.get(lock)
            && self.live_hostile(held, other)
        {
            let held_range = held.range_m.unwrap_or(u32::MAX);
            let best_range = best.range_m.unwrap_or(u32::MAX);
            if best_range.saturating_add(RETARGET_NEARER_M) > held_range {
                return Some(lock.clone());
            }
        }
        Some(best.designator.clone())
    }

    /// A contact of the other side that is not a wreck this match has seen
    /// destroyed.
    fn live_hostile(&self, contact: &Contact, other: Side) -> bool {
        contact.side == Some(other) && !self.wrecks.contains(&contact.designator)
    }

    /// **The kills** since the last decision, `{d} destroyed.` or `Target
    /// destroyed.` of the lock: each wreck is remembered for the match (never
    /// targeted, trailed or hunted again), a kill of the trail's hostile
    /// drops the trail and ends any hunt, and (cut AN.3) a kill of a followed
    /// machine ends its follow or its return.
    fn take_kills(&mut self, picture: &Picture) {
        let lock = picture.lock.clone().or_else(|| self.lock_seen.clone());
        let kills: Vec<(u64, String)> = picture
            .events
            .iter()
            .filter(|event| event.kind == EventKind::Kill && event.at_ms > self.kill_seen_ms)
            .map(|event| (event.at_ms, event.text.clone()))
            .collect();
        for (at_ms, text) in kills {
            self.kill_seen_ms = self.kill_seen_ms.max(at_ms);
            let wreck = if text == "Target destroyed." {
                lock.clone()
            } else {
                text.strip_suffix(" destroyed.").map(str::to_owned)
            };
            let Some(wreck) = wreck else {
                continue;
            };
            if self
                .trail
                .as_ref()
                .is_some_and(|trail| trail.designator == wreck)
            {
                self.trail = None;
                if let Some(hunt) = self.hunt.take() {
                    let d = hunt.trail.designator;
                    self.note(format!("The hunt for {d} ends; searching."));
                }
            }
            // Cut AN.3: a kill ends the follow of that machine, and the wait
            // for its return.
            if self.follow.as_ref().is_some_and(|follow| follow.d == wreck) {
                self.follow = None;
            }
            if self.ret.as_ref().is_some_and(|ret| ret.d == wreck) {
                self.ret = None;
            }
            self.wrecks.insert(wreck);
        }
        if picture.lock.is_some() {
            self.lock_seen = picture.lock.clone();
        }
    }

    /// **What seen second reads** (cut AL.3), every decision, from this
    /// match's events only (the picture's ring outlives a match): each
    /// `{d}'s {section} is destroyed.` adds `d` to [`Pilot::hurt`]; the newest
    /// hit taken moves [`Pilot::hit_taken_ms`]; and every contact with no side
    /// is stamped the first time it is seen ([`Pilot::unknown_since`]), and
    /// forgotten once it is no longer an unknown return in the picture.
    fn take_seen_second(&mut self, picture: &Picture, power_ms: u64, now_ms: u64) {
        self.hurt_seen_ms = self.hurt_seen_ms.max(power_ms);
        self.hit_taken_ms = self.hit_taken_ms.max(power_ms);
        let (hurt_seen, hit_seen) = (self.hurt_seen_ms, self.hit_taken_ms);
        for event in &picture.events {
            match event.kind {
                EventKind::TargetSectionLost if event.at_ms > hurt_seen => {
                    self.hurt_seen_ms = self.hurt_seen_ms.max(event.at_ms);
                    if let Some((designator, _)) = event.text.split_once("'s ") {
                        self.hurt.insert(designator.to_owned());
                    }
                }
                EventKind::HitTaken if event.at_ms > hit_seen => {
                    self.hit_taken_ms = self.hit_taken_ms.max(event.at_ms);
                }
                _ => {}
            }
        }
        // A return lost or told apart is forgotten, its sentence with it: one
        // found again is a new return ("If you lose the contact and find it
        // again it is an unknown return once more, so it reads `X2` again",
        // `help cockpit`), first seen now.
        self.unknown_since.retain(|designator, _| {
            picture
                .contacts
                .get(designator)
                .is_some_and(|c| c.side.is_none())
        });
        let since = &self.unknown_since;
        self.unknown_said
            .retain(|designator| since.contains_key(designator));
        for contact in picture.contacts.values().filter(|c| c.side.is_none()) {
            self.unknown_since
                .entry(contact.designator.clone())
                .or_insert(now_ms);
        }
    }

    /// **The exchange** (cut AN.2), every decision, from this match's events
    /// newer than the last read: a hit scored on a named hostile (`Hit on
    /// {d}…` without `(FRIENDLY)`; a bare `Hit.` names nobody) is `(at,
    /// false)`, a hit taken `(at, true)`; entries older than
    /// [`EXCHANGE_WINDOW_MS`] are pruned. Each such hit scored also counts
    /// once for `d` in [`Pilot::scored_on`] (cut AN.3). Fed whatever the knobs
    /// say.
    fn take_exchange(&mut self, picture: &Picture, power_ms: u64, now_ms: u64) {
        self.exchange_seen_ms = self.exchange_seen_ms.max(power_ms);
        let seen = self.exchange_seen_ms;
        for event in picture.events.iter().filter(|event| event.at_ms > seen) {
            let entry = match event.kind {
                EventKind::HitScored
                    if event.text.starts_with("Hit on ") && !event.text.contains("(FRIENDLY)") =>
                {
                    Some(false)
                }
                EventKind::HitTaken => Some(true),
                _ => None,
            };
            if let Some(taken) = entry {
                self.exchange.push_back((event.at_ms, taken));
            }
            // Cut AN.3: the hits scored on each hostile, by the designator
            // the line names (`Hit on {d}: {section}.` or `Hit on {d}.`).
            if entry == Some(false)
                && let Some(d) = event
                    .text
                    .strip_prefix("Hit on ")
                    .and_then(|rest| rest.split([':', '.']).next())
                    .filter(|d| !d.is_empty())
            {
                let hits = self.scored_on.entry(d.to_owned()).or_default();
                *hits = hits.saturating_add(1);
            }
            self.exchange_seen_ms = self.exchange_seen_ms.max(event.at_ms);
        }
        while self
            .exchange
            .front()
            .is_some_and(|(at, _)| now_ms.saturating_sub(*at) > EXCHANGE_WINDOW_MS)
        {
            self.exchange.pop_front();
        }
    }

    /// **The rules of cut AN.2, judged** every decision before any step
    /// flies: the opening's stages and ends, the re-approach's begin, stages
    /// and ends, and the halt's ends, each saying what it did; and cut AN.3's,
    /// the return's stages and ends (before the re-approach, which never
    /// begins while a return is in force) and the follow's begin and ends
    /// (before the halt's, which a return's end may arm).
    fn judge(
        &mut self,
        picture: &Picture,
        doctrine: &Doctrine,
        other: Side,
        target: Option<&str>,
        now_ms: u64,
    ) {
        self.judge_opening(picture, other, target, now_ms);
        self.judge_return(picture, doctrine, other, now_ms);
        self.judge_reapproach(picture, doctrine, other, target, now_ms);
        self.judge_follow(picture, doctrine, other, target, now_ms);
        self.judge_halt(picture, doctrine, other, now_ms);
    }

    /// **A hostile coming on** (cut AN.2's wait, and cut AN.3's): the nearest
    /// live hostile held young with a row inside [`STOCK_SIGHT_M`], with its
    /// row's range. A row farther out is a teammate's report (`help cockpit`:
    /// `far` covers "anything a teammate is passing you from beyond it").
    /// Only rows `counts` keeps (the designator and the row's range) are
    /// read: the return's wait (cut AO.1) drops its own machine's row until it
    /// reads nearer than the plot the wait faces.
    fn coming_on(
        &self,
        picture: &Picture,
        doctrine: &Doctrine,
        other: Side,
        counts: impl Fn(&str, u32) -> bool,
    ) -> Option<(String, u32)> {
        picture
            .contacts
            .values()
            .filter(|c| self.live_hostile(c, other) && c.age_ms <= held_limit_ms(doctrine))
            .filter_map(|c| c.range_m.map(|range| (c.designator.clone(), range)))
            .filter(|(d, range)| f64::from(*range) <= STOCK_SIGHT_M && counts(d, *range))
            .min_by_key(|(_, range)| *range)
    }

    /// **The follow, judged** (cut AN.3; the module head). Not in force, it
    /// begins when the knob is on, the muster is known (an entry line to
    /// keep out of), the doctrine is not `withdraw`, no fall back flies, no
    /// return is in force (the machine's one return is being waited for), and
    /// the target is [`Pilot::broken`]: `Following {d} home: it is hurt and
    /// driving away; holding {FOLLOW_BAND_M} m behind it.` — but (cut AO.1)
    /// with the keep-out already reached, no follow begins: only the
    /// keep-out's sentence is said and the return begins from its last plot,
    /// so the two notes are never said in one decision. A re-approach in
    /// force ends then (rested, as any end of it is: it opens the range on a
    /// stopped machine, and this one drives away; the return and the
    /// re-approach are never both in force), and so does a halt on it (it
    /// fires on a machine coming on); a halt on another hostile holds the
    /// follow off. In force (from that decision on), it ends
    /// silently under a `withdraw` doctrine; with `d` lost (no row, or one
    /// older than [`HUNT_STALE_PLOT_MS`]) into the return from the trail's
    /// last plot of it, saying nothing more; silently when `d`, still held,
    /// is no longer the target (step 4 flies the stance for the new one, and a
    /// follow it never flies must neither hold off the hunt and the
    /// re-approach nor back the tank off bow on a machine it no longer
    /// faces); at the keep-out
    /// ([`ENTRY_KEEP_OUT_M`] from the entry line for `d`, that and
    /// [`FOLLOW_BAND_M`] for the own tank) into the return from its last plot,
    /// saying `Not following {d} into its entry zone; backing off out of its
    /// sight.`; and silently when `d` has shown neither a plausible track nor
    /// (cut AO.1) [`Pilot::leaving`] for [`FOLLOW_STOPPED_MS`] (it has
    /// stopped: the stance flies). A kill ends it in [`Pilot::take_kills`].
    fn judge_follow(
        &mut self,
        picture: &Picture,
        doctrine: &Doctrine,
        other: Side,
        target: Option<&str>,
        now_ms: u64,
    ) {
        let own = picture.own_pos_at(now_ms);
        if self.follow.is_none() {
            let falling_back = self.withdraw_until_ms.is_some_and(|until| now_ms < until);
            let begins = self.knobs.follow_broken
                && self.muster_m.is_some()
                && doctrine.stance != Stance::Withdraw
                && !falling_back
                && self.ret.is_none();
            let Some(contact) = target.and_then(|t| picture.contacts.get(t)) else {
                return;
            };
            let Some(own) = own else {
                return;
            };
            // A halt on another hostile, coming on, outranks the follow (step
            // 3c before 4): no follow begins under it.
            let halted_elsewhere = self
                .halt
                .as_ref()
                .is_some_and(|halt| halt.d != contact.designator);
            if !begins
                || halted_elsewhere
                || self.short_of_entry(picture, own).is_none()
                || !self.broken(picture, contact, own, now_ms)
            {
                return;
            }
            let d = contact.designator.clone();
            // The machine is driving away: no longer the stopped machine a
            // re-approach opens the range on, nor one coming on at a halt.
            if self.reapproach.is_some() {
                self.end_reapproach(now_ms);
            }
            if self
                .halt
                .as_ref()
                .is_some_and(|halt| halt.d == contact.designator)
            {
                self.halt = None;
            }
            // Cut AO.1: no follow begins inside the keep-out. The keep-out's
            // own sentence, and the return from its last plot, at once.
            if self.kept_out(picture, contact, Some(own), now_ms) {
                self.note(format!(
                    "Not following {d} into its entry zone; backing off out of its sight."
                ));
                if let Some(plot) =
                    last_plot(picture, contact).or_else(|| target_pos(picture, contact, now_ms))
                {
                    self.begin_return(picture, d, plot, now_ms);
                }
                return;
            }
            self.note(format!(
                "Following {d} home: it is hurt and driving away; holding {:.0} m behind it.",
                FOLLOW_BAND_M.round()
            ));
            self.follow = Some(Follow {
                d,
                since_ms: now_ms,
                tracked_ms: now_ms,
            });
        }
        let Some(follow) = self.follow.clone() else {
            return;
        };
        let d = follow.d;
        if doctrine.stance == Stance::Withdraw || !self.knobs.follow_broken {
            self.follow = None;
            return;
        }
        let young = picture
            .contacts
            .get(&d)
            .filter(|c| self.live_hostile(c, other) && c.age_ms <= HUNT_STALE_PLOT_MS);
        let Some(contact) = young else {
            // Lost: backing out of its sight from the trail's last plot.
            self.follow = None;
            if let Some(plot) = self
                .trail
                .as_ref()
                .filter(|trail| trail.designator == d)
                .map(|trail| trail.plot_m)
            {
                self.begin_return(picture, d, plot, now_ms);
            }
            return;
        };
        // Still held, but the pilot has turned to another hostile: step 4
        // flies the stance for that one, so the follow ends, silently.
        if target != Some(d.as_str()) {
            self.follow = None;
            return;
        }
        if self.kept_out(picture, contact, own, now_ms) {
            self.follow = None;
            self.note(format!(
                "Not following {d} into its entry zone; backing off out of its sight."
            ));
            if let Some(plot) =
                last_plot(picture, contact).or_else(|| target_pos(picture, contact, now_ms))
            {
                self.begin_return(picture, d, plot, now_ms);
            }
            return;
        }
        // Cut AO.1: still moving is a plausible track, or the row's range
        // from the own tank opening with none ([`Pilot::leaving`]).
        let moving = plausible_track(contact).is_some()
            || own.is_some_and(|own| self.leaving(contact, own, now_ms));
        if moving {
            if let Some(follow) = self.follow.as_mut() {
                follow.tracked_ms = now_ms;
            }
        } else if now_ms >= follow.tracked_ms.saturating_add(FOLLOW_STOPPED_MS) {
            self.follow = None;
        }
    }

    /// **The keep-out** (cut AN.3; a helper since cut AO.1, where the follow's
    /// begin reads it too): the machine within [`ENTRY_KEEP_OUT_M`] of the
    /// other side's entry line, or the own tank within that and
    /// [`FOLLOW_BAND_M`].
    fn kept_out(
        &self,
        picture: &Picture,
        contact: &Contact,
        own: Option<(f64, f64)>,
        now_ms: u64,
    ) -> bool {
        let short = |q: Option<(f64, f64)>| q.and_then(|q| self.short_of_entry(picture, q));
        short(target_pos(picture, contact, now_ms)).is_some_and(|m| m <= ENTRY_KEEP_OUT_M)
            || short(own).is_some_and(|m| m <= ENTRY_KEEP_OUT_M + FOLLOW_BAND_M)
    }

    /// **The return begins** (cut AN.3) for `d` from its last plot `plot_m`:
    /// backing out of its sight, or — already [`RETURN_STANDOFF_M`] from the
    /// plot — waiting at once.
    fn begin_return(&mut self, picture: &Picture, d: String, plot_m: (f64, f64), now_ms: u64) {
        let off = picture
            .own_pos_at(now_ms)
            .map(|own| distance_m(own, plot_m));
        self.ret = Some(Return {
            d,
            plot_m,
            stage: ReturnStage::Back { since_ms: now_ms },
        });
        if off.is_some_and(|off| off >= RETURN_STANDOFF_M) {
            self.return_waits(off, now_ms);
        }
    }

    /// **The return waits** (cut AN.3), `off_m` from the plot (cut AO.1:
    /// with no own position, `Waiting for {d} to come back.`), said once.
    fn return_waits(&mut self, off_m: Option<f64>, now_ms: u64) {
        let Some(ret) = self.ret.as_mut() else {
            return;
        };
        ret.stage = ReturnStage::Wait { since_ms: now_ms };
        let d = ret.d.clone();
        self.note(match off_m {
            Some(off) => format!(
                "Waiting for {d} to come back, {:.0} m from its last plot.",
                off.round()
            ),
            None => format!("Waiting for {d} to come back."),
        });
    }

    /// **Whether a back-off ends and holds** (cut AO.1), the re-approach's
    /// `Open` or the return's `Back` begun at `since_ms`, its machine at
    /// `aim`: (a) no bow within [`HOLD_BOW_DEG`] leaves [`EDGE_BACK_MARGIN_M`]
    /// astern ([`back_off_bow`] from the own position now, on
    /// [`ground_side`]); (b) a `Held:` read while it reversed; (c) the shared
    /// reverse path turned its reverse into a stop at an earlier decision
    /// (held ground behind, [`Pilot::back_off_blocked_ms`]).
    fn back_off_ends(
        &self,
        picture: &Picture,
        own: Option<(f64, f64)>,
        aim: (f64, f64),
        since_ms: u64,
    ) -> bool {
        let side = ground_side(picture);
        let no_bow =
            own.is_some_and(|own| back_off_bow(own, bearing_deg(own, aim), side).is_none());
        let held = self.held_astern && self.held_seen_ms > since_ms;
        no_bow || held || self.back_off_blocked_ms > since_ms
    }

    /// **A back-off's hold** (cut AO.1): `{head}: waiting {m} m from {d}'s
    /// last plot.`, `m` from `plot_m` in whole metres, or `{head}: waiting for
    /// {d}.` with no own position.
    fn no_room_note(&mut self, head: &str, d: &str, own: Option<(f64, f64)>, plot_m: (f64, f64)) {
        self.note(match own {
            Some(own) => format!(
                "{head}: waiting {:.0} m from {d}'s last plot.",
                distance_m(own, plot_m).round()
            ),
            None => format!("{head}: waiting for {d}."),
        });
    }

    /// **The return, judged** (cut AN.3; cut AO.1; the module head). Backing
    /// off, its plot follows a fresh row of the machine only **farther** from
    /// the own tank (a machine coming back never lengthens the back-off: the
    /// help's broken machine "comes back in, once", and the return exists to
    /// take that from a stopped tank); it ends and holds where
    /// [`Pilot::back_off_ends`] says (`No room behind for the return: …`, and
    /// its own wait, no other note); and it waits once [`RETURN_STANDOFF_M`]
    /// from the plot or [`RETURN_BACK_MS`] after it began. Waiting, a hostile
    /// coming on ([`Pilot::coming_on`]; `d` itself only once its row reads
    /// nearer than the plot, cut AO.1) arms the halt and ends it; a hit taken
    /// after the wait began ends it with nothing said; and [`RETURN_WAIT_MS`]
    /// ends it with `No return from {d} in {s} s; searching.` and the trail of
    /// `d` dropped, so no hunt drives at its last plot near the entry line and
    /// on into the entry zone — the search flies.
    fn judge_return(&mut self, picture: &Picture, doctrine: &Doctrine, other: Side, now_ms: u64) {
        let Some(ret) = self.ret.clone() else {
            return;
        };
        match ret.stage {
            ReturnStage::Back { since_ms } => {
                let own = picture.own_pos_at(now_ms);
                let mut plot = ret.plot_m;
                if let Some(own) = own
                    && let Some(contact) = picture.contacts.get(&ret.d)
                    && self.live_hostile(contact, other)
                    && plot_age_ms(contact, now_ms) <= FRESH_PLOT_MS
                    && let Some(fresh) = last_plot(picture, contact)
                    && distance_m(own, fresh) > distance_m(own, plot)
                {
                    plot = fresh;
                    if let Some(ret) = self.ret.as_mut() {
                        ret.plot_m = plot;
                    }
                }
                let aim = self
                    .young_pos(picture, doctrine, &ret.d, now_ms)
                    .unwrap_or(plot);
                if self.back_off_ends(picture, own, aim, since_ms) {
                    if let Some(ret) = self.ret.as_mut() {
                        ret.stage = ReturnStage::Wait { since_ms: now_ms };
                    }
                    self.no_room_note("No room behind for the return", &ret.d, own, plot);
                    return;
                }
                let off = own.map(|own| distance_m(own, plot));
                if off.is_some_and(|off| off >= RETURN_STANDOFF_M)
                    || now_ms >= since_ms.saturating_add(RETURN_BACK_MS)
                {
                    self.return_waits(off, now_ms);
                }
            }
            ReturnStage::Wait { since_ms } => {
                // Cut AO.1: a wait begun where the ground behind ran out can
                // begin with the machine still in sight and driving home. It
                // comes on only once its row reads nearer than the plot the
                // wait faces (the help's "comes back in, once"); a row no
                // nearer is the machine still leaving, and arming the halt on
                // it would end the wait only for the follow's begin to find
                // it broken at the keep-out and back off again, round and
                // round. Any other hostile in sight comes on as before.
                let off = picture
                    .own_pos_at(now_ms)
                    .map(|own| distance_m(own, ret.plot_m));
                let back = |d: &str, range: u32| {
                    d != ret.d || !off.is_some_and(|off| f64::from(range) >= off)
                };
                if let Some((come, range)) = self.coming_on(picture, doctrine, other, back) {
                    self.ret = None;
                    self.arm_halt(&come, range, now_ms);
                } else if self.hit_taken_ms > since_ms {
                    self.ret = None;
                } else if now_ms >= since_ms.saturating_add(RETURN_WAIT_MS) {
                    self.ret = None;
                    let d = ret.d;
                    self.note(format!(
                        "No return from {d} in {} s; searching.",
                        now_ms.saturating_sub(since_ms) / 1_000
                    ));
                    if self
                        .trail
                        .as_ref()
                        .is_some_and(|trail| trail.designator == d)
                    {
                        self.trail = None;
                    }
                }
            }
        }
    }

    /// **A live hostile in sight** (cut AN.2): one with a row read (its range
    /// known) whose plot is no older than [`FRESH_PLOT_MS`] — the target when
    /// it is one, else the nearest — with its row's range.
    fn in_sight(
        &self,
        picture: &Picture,
        other: Side,
        target: Option<&str>,
        now_ms: u64,
    ) -> Option<(String, u32)> {
        let sighted = |c: &&Contact| {
            self.live_hostile(c, other)
                && c.range_m.is_some()
                && plot_age_ms(c, now_ms) <= FRESH_PLOT_MS
        };
        let contact = target
            .and_then(|t| picture.contacts.get(t))
            .filter(sighted)
            .or_else(|| {
                picture
                    .contacts
                    .values()
                    .filter(sighted)
                    .min_by_key(|c| c.range_m)
            })?;
        Some((contact.designator.clone(), contact.range_m?))
    }

    /// **Arm the halt** for `d`, a row read `range_m` off (cut AN.2).
    fn arm_halt(&mut self, d: &str, range_m: u32, now_ms: u64) {
        self.note(format!("Firing from the halt at {d}, {range_m} m off."));
        self.halt = Some(Halt {
            d: d.to_owned(),
            since_ms: now_ms,
            nearest_m: range_m,
            nearest_ms: now_ms,
        });
    }

    /// **The opening holds** (cut AN.2): stopped, the bow toward the other
    /// side's entry, said once.
    fn enter_hold(&mut self, picture: &Picture, now_ms: u64) {
        let facing = match (picture.own_pos_at(now_ms), self.other_entry_now(picture)) {
            (Some(own), Some(entry)) => whole(bearing_deg(own, entry)),
            _ => self.reference_heading(picture),
        };
        self.opening = OpeningStage::Hold {
            since_ms: now_ms,
            facing_deg: facing,
        };
        self.note(format!(
            "Opening: holding near our entry, facing {facing:03}° toward the other side's entry."
        ));
    }

    /// **The opening, judged** (cut AN.2; the module head). `Fresh` never
    /// begins — it is over at once, saying nothing — outside a skirmish, when
    /// this match's own `Power online.` was not read, or with no muster
    /// ([`Pilot::muster_m`], known once the first position is read or
    /// [`OPENING_BRIEF_WAIT_MS`] after power); else it waits for the first
    /// `CONTACTS` reply after the first decision (at most
    /// [`OPENING_BRIEF_WAIT_MS`]) and is over, silently, when a contact that is
    /// no teammate is held or a `New contact` since power names one that is not
    /// now held as a teammate — a posed duel, where the machine is an
    /// unidentified return for a second or two — else it asks for the brief.
    /// A teammate (cut AO.1) is a contact the stream names as the own side, or
    /// one not yet told apart whose row reads at most [`OPENING_TEAMMATE_M`].
    /// **The room** (cut AO.1), only with a back-off knob on: a whole brief's
    /// [`opening_inward`] that says [`RoomVerdict::MoveIn`] (at most
    /// [`OPENING_INWARD_LEGS_MAX`] a match) drives the leg toward the other
    /// side's entry ([`OpeningStage::Inward`]), which ends as the step does,
    /// at [`OPENING_INWARD_MS`] or on a `Held:`, into a stop and a fresh brief
    /// (read only from the leg's end on); [`RoomVerdict::NoWayIn`] is said once
    /// a match, and the step or the hold follow as before. From then the
    /// opening ends on (a) a
    /// live hostile in sight ([`Pilot::in_sight`]; the halt armed for it only
    /// inside [`STOCK_SIGHT_M`]: a row farther out is a teammate's report),
    /// (b) a hit taken after power or (c) [`OPENING_MAX_MS`] after power; a
    /// return not yet told apart ends nothing. The brief is waited for until
    /// its ground is whole or [`OPENING_BRIEF_WAIT_MS`] has passed; the step
    /// is [`opening_step`] of a whole ground read this match (none, or a dark
    /// window: the hold where the tank stands); the step is done once the own
    /// position's progress along it is within [`OPENING_ARRIVE_M`] of its
    /// length, or [`OPENING_STEP_MS`] after it began.
    fn judge_opening(&mut self, picture: &Picture, other: Side, target: Option<&str>, now_ms: u64) {
        let first = self.power_ms.unwrap_or(now_ms);
        let power = picture.power_ms.unwrap_or(first);
        if self.opening == OpeningStage::Fresh {
            if picture.match_mode != MatchMode::Skirmish || !picture.powered_here {
                self.opening = OpeningStage::Done;
                return;
            }
            if self.start_pos.is_none() && now_ms < power.saturating_add(OPENING_BRIEF_WAIT_MS) {
                return;
            }
            if self.muster_m.is_none() {
                self.opening = OpeningStage::Done;
                return;
            }
            let replied = picture.contacts_read > self.opening_contacts_seen;
            if !replied && now_ms < first.saturating_add(OPENING_BRIEF_WAIT_MS) {
                return;
            }
            // Cut AO.1: a teammate at the muster ends nothing. The help's two
            // sides muster at opposite ends of the ground, so a return not yet
            // told apart within `OPENING_TEAMMATE_M` at the first `CONTACTS`
            // is a teammate; and a contact the stream itself names as the own
            // side is never a hostile.
            let own_side = other.other();
            let teammate = |c: &Contact| {
                c.side == Some(own_side)
                    || (c.side.is_none()
                        && c.range_m.is_some_and(|range| range <= OPENING_TEAMMATE_M))
            };
            let foe_held = picture.contacts.values().any(|c| !teammate(c));
            // AN.2's notice test, narrowed: a `New contact` since power whose
            // designator is not now held as a teammate (one the whole-list
            // rule dropped, an empty first reply, still counts). The notice
            // keeps the designator it was said under: a contact identified
            // since (`Contact X1 identified: RB, red.`) is held under its new
            // one and remembers the old one as `was`, so it is looked up by
            // either.
            let noticed = picture
                .events
                .iter()
                .filter(|event| event.kind == EventKind::ContactNew && event.at_ms >= power)
                .any(|event| {
                    let d = event
                        .text
                        .strip_prefix("New contact ")
                        .and_then(|rest| rest.split([':', '.']).next())
                        .unwrap_or_default();
                    !picture
                        .contacts
                        .values()
                        .any(|c| (c.designator == d || c.was.as_deref() == Some(d)) && teammate(c))
                });
            if noticed || foe_held {
                self.opening = OpeningStage::Done;
                return;
            }
            self.opening = OpeningStage::Brief { since_ms: now_ms };
        }
        if matches!(self.opening, OpeningStage::Fresh | OpeningStage::Done) {
            return;
        }
        let secs = now_ms.saturating_sub(power) / 1_000;
        // (a) A hostile in sight.
        if let Some((d, range)) = self.in_sight(picture, other, target, now_ms) {
            self.opening = OpeningStage::Done;
            self.note(format!("Opening over: {d} is in sight after {secs} s."));
            if f64::from(range) <= STOCK_SIGHT_M {
                self.arm_halt(&d, range, now_ms);
            }
            return;
        }
        // (b) A hit taken after power.
        if self.hit_taken_ms > first {
            self.opening = OpeningStage::Done;
            self.note(format!("Opening over: hit after {secs} s."));
            return;
        }
        // (c) Its time.
        if now_ms >= power.saturating_add(OPENING_MAX_MS) {
            self.opening = OpeningStage::Done;
            self.note(format!(
                "Opening over: nothing came in {secs} s; searching."
            ));
            return;
        }
        match self.opening.clone() {
            OpeningStage::Brief { since_ms } => {
                // The muster's first brief is read as AN.2 read it; a brief
                // after a move in (cut AO.1) only once read since it was
                // asked, never an earlier one's ground.
                let read_from = if self.opening_legs == 0 {
                    first
                } else {
                    since_ms
                };
                let ground = picture
                    .ground
                    .as_ref()
                    .filter(|ground| ground.complete && ground.read_ms >= read_from);
                if ground.is_none() && now_ms < since_ms.saturating_add(OPENING_BRIEF_WAIT_MS) {
                    return;
                }
                let away = self
                    .muster_m
                    .zip(self.other_entry_now(picture))
                    .map(|(muster, entry)| whole(bearing_deg(muster, entry)));
                // Cut AO.1: room behind for the longest back-off, only when a
                // back-off knob is on (a pilot that never backs off has no use
                // for it).
                let backs_off = self.knobs.reapproach || self.knobs.follow_broken;
                if backs_off
                    && let Some(ground) = ground
                    && let Some(away) = away
                {
                    match opening_inward(ground, away) {
                        Some(RoomVerdict::MoveIn { leg_m, room_m })
                            if self.opening_legs < OPENING_INWARD_LEGS_MAX =>
                        {
                            if let Some(from) = ground.from_m {
                                let aim = clamp_arena(along(from, f64::from(away), leg_m));
                                self.opening = OpeningStage::Inward {
                                    aim_m: aim,
                                    from_m: from,
                                    since_ms: now_ms,
                                };
                                self.opening_legs = self.opening_legs.saturating_add(1);
                                self.note(format!(
                                    "Opening: moving in {:.0} m toward the other side's entry: \
                                     {:.0} m of ground behind us, {:.0} m wanted.",
                                    leg_m.round(),
                                    room_m.round(),
                                    OPENING_ROOM_M.round()
                                ));
                                return;
                            }
                        }
                        Some(RoomVerdict::NoWayIn { room_m }) if !self.opening_no_way_said => {
                            self.opening_no_way_said = true;
                            self.note(format!(
                                "Opening: no way in toward the other side's entry; {:.0} m of \
                                 ground behind us.",
                                room_m.round()
                            ));
                        }
                        _ => {}
                    }
                }
                let step = ground.and_then(|ground| {
                    let from = ground.from_m?;
                    let (bearing, metres) = opening_step(ground, away?)?;
                    Some((from, bearing, metres))
                });
                match step {
                    Some((from, bearing, metres)) => {
                        let aim = clamp_arena(along(from, f64::from(bearing), metres));
                        self.opening = OpeningStage::Step {
                            aim_m: aim,
                            from_m: from,
                            since_ms: now_ms,
                        };
                        self.note(format!(
                            "Opening: stepping off the road to the {} ({:.0} m).",
                            bearing_word(bearing),
                            metres.round()
                        ));
                    }
                    None => self.enter_hold(picture, now_ms),
                }
            }
            OpeningStage::Step {
                aim_m,
                from_m,
                since_ms,
            } => {
                let length = distance_m(from_m, aim_m);
                let progress = progress_along(picture.own_pos_at(now_ms), from_m, aim_m);
                if progress >= length - OPENING_ARRIVE_M
                    || now_ms >= since_ms.saturating_add(OPENING_STEP_MS)
                {
                    self.enter_hold(picture, now_ms);
                }
            }
            // Cut AO.1: the move in ends as the step does, or at its time, or
            // on a `Held:` read since it began; then stopped for the next
            // brief, which the lane asks at its own pace.
            OpeningStage::Inward {
                aim_m,
                from_m,
                since_ms,
            } => {
                let length = distance_m(from_m, aim_m);
                let progress = progress_along(picture.own_pos_at(now_ms), from_m, aim_m);
                if progress >= length - OPENING_ARRIVE_M
                    || now_ms >= since_ms.saturating_add(OPENING_INWARD_MS)
                    || self.held_seen_ms > since_ms
                {
                    self.opening = OpeningStage::Brief { since_ms: now_ms };
                }
            }
            _ => {}
        }
    }

    /// **The re-approach, judged** (cut AN.2; the module head). In force:
    /// backing off, its plot follows the hostile while the hostile's plot is
    /// fresh, it ends and holds where [`Pilot::back_off_ends`] says (cut AO.1:
    /// `No room behind: …`, straight into its own wait with no other note),
    /// it waits once [`REAPPROACH_OPEN_M`] from that plot and is given
    /// up at [`REAPPROACH_MAX_MS`]; waiting, any live hostile held young with
    /// a row inside [`STOCK_SIGHT_M`] arms the halt and ends it (a row farther
    /// out is a teammate's report and ends nothing), a hit taken ends it with
    /// nothing said, and
    /// [`REAPPROACH_WAIT_MS`] ends it. Every end rests it for
    /// [`REAPPROACH_REST_MS`]. Not in force, it begins when the knob is on,
    /// no broken machine's follow or return is in force (cut AN.3),
    /// fewer than [`REAPPROACH_PER_MATCH`] began this match, the rest is over,
    /// no fall back flies, the doctrine is neither `orbit` nor `withdraw`, the
    /// target is a live hostile whose row is fresh ([`FRESH_PLOT_MS`]), with no
    /// published track (stopped) and within [`STOCK_SIGHT_M`], and the
    /// exchange's window holds at least [`EXCHANGE_MIN_TAKEN`] hits taken and
    /// [`EXCHANGE_MARGIN`] more taken than scored.
    fn judge_reapproach(
        &mut self,
        picture: &Picture,
        doctrine: &Doctrine,
        other: Side,
        target: Option<&str>,
        now_ms: u64,
    ) {
        if let Some(reapproach) = self.reapproach.clone() {
            let d = reapproach.d;
            match reapproach.stage {
                ReapproachStage::Open { since_ms } => {
                    let mut plot = reapproach.plot_m;
                    if let Some(contact) = picture.contacts.get(&d)
                        && self.live_hostile(contact, other)
                        && plot_age_ms(contact, now_ms) <= FRESH_PLOT_MS
                        && let Some(fresh) = last_plot(picture, contact)
                    {
                        plot = fresh;
                    }
                    if let Some(reapproach) = self.reapproach.as_mut() {
                        reapproach.plot_m = plot;
                    }
                    let own = picture.own_pos_at(now_ms);
                    // Cut AO.1: judged first, the back-off ends and holds at
                    // the edge, on a `Held:` or at held ground behind.
                    let aim = self
                        .young_pos(picture, doctrine, &d, now_ms)
                        .unwrap_or(plot);
                    let off = own.map(|own| distance_m(own, plot));
                    if self.back_off_ends(picture, own, aim, since_ms) {
                        if let Some(reapproach) = self.reapproach.as_mut() {
                            reapproach.stage = ReapproachStage::Wait { since_ms: now_ms };
                        }
                        self.no_room_note("No room behind", &d, own, plot);
                    } else if let Some(off) = off.filter(|off| *off >= REAPPROACH_OPEN_M) {
                        if let Some(reapproach) = self.reapproach.as_mut() {
                            reapproach.stage = ReapproachStage::Wait { since_ms: now_ms };
                        }
                        let off = off.round();
                        self.note(format!(
                            "Out of {d}'s sight at {off:.0} m from its last plot; waiting for it \
                             to come on."
                        ));
                    } else if now_ms >= since_ms.saturating_add(REAPPROACH_MAX_MS) {
                        self.note(format!(
                            "Could not open the range on {d} in {} s; holding.",
                            now_ms.saturating_sub(since_ms) / 1_000
                        ));
                        self.end_reapproach(now_ms);
                    }
                }
                ReapproachStage::Wait { since_ms } => {
                    // A hostile coming on is one held young with a row inside
                    // the stock sight: a row farther out is a teammate's
                    // report, as at the opening's end, and it ends nothing.
                    if let Some((come, range)) =
                        self.coming_on(picture, doctrine, other, |_, _| true)
                    {
                        self.arm_halt(&come, range, now_ms);
                        self.end_reapproach(now_ms);
                    } else if self.hit_taken_ms > since_ms {
                        self.end_reapproach(now_ms);
                    } else if now_ms >= since_ms.saturating_add(REAPPROACH_WAIT_MS) {
                        self.note(format!(
                            "No approach from {d} in {} s; moving on.",
                            now_ms.saturating_sub(since_ms) / 1_000
                        ));
                        self.end_reapproach(now_ms);
                    }
                }
            }
            return;
        }
        let falling_back = self.withdraw_until_ms.is_some_and(|until| now_ms < until);
        // Cut AN.3: never while a broken machine is followed or its return is
        // in force (the two are never both in force).
        if !self.knobs.reapproach
            || self.follow.is_some()
            || self.ret.is_some()
            || self.reapproaches >= REAPPROACH_PER_MATCH
            || now_ms < self.reapproach_rest_until_ms
            || falling_back
            || matches!(doctrine.stance, Stance::Orbit | Stance::Withdraw)
        {
            return;
        }
        let Some(contact) = target.and_then(|t| picture.contacts.get(t)) else {
            return;
        };
        let stopped_in_sight = self.live_hostile(contact, other)
            && plot_age_ms(contact, now_ms) <= FRESH_PLOT_MS
            && contact.track.is_none()
            && contact
                .range_m
                .is_some_and(|range| f64::from(range) <= STOCK_SIGHT_M);
        if !stopped_in_sight {
            return;
        }
        let (taken, scored) = self.exchange_counts(now_ms);
        if taken < EXCHANGE_MIN_TAKEN || taken < scored.saturating_add(EXCHANGE_MARGIN) {
            return;
        }
        let Some(plot_m) = last_plot(picture, contact) else {
            return;
        };
        let d = contact.designator.clone();
        self.reapproaches = self.reapproaches.saturating_add(1);
        self.reapproach = Some(Reapproach {
            d: d.clone(),
            stage: ReapproachStage::Open { since_ms: now_ms },
            plot_m,
        });
        let window_s = EXCHANGE_WINDOW_MS / 1_000;
        self.note(format!(
            "Opening the range on {d}: {taken} hits taken to {scored} scored in {window_s} s; \
             backing off bow on."
        ));
    }

    /// A re-approach is over (cut AN.2): the rest begins.
    fn end_reapproach(&mut self, now_ms: u64) {
        self.reapproach = None;
        self.reapproach_rest_until_ms = now_ms.saturating_add(REAPPROACH_REST_MS);
    }

    /// **The halt's ends** (cut AN.2), in this order, each said: a hit taken
    /// after it began (`we are found`); its hostile, held young, within the
    /// band and [`HOLD_SLACK_M`] (the doctrine's band, widened after a fall
    /// back as [`Pilot::engage`] widens it); [`HALT_STILL_MS`] in which its
    /// hostile's row has read no shorter range and it has published no track
    /// (`has stopped`: it is not coming on); [`HALT_MAX_MS`] after it began;
    /// and, with nothing said, its hostile no longer held young. A follow
    /// beginning (cut AN.3, [`Pilot::judge_follow`]) ends it too, the
    /// follow's own note said.
    fn judge_halt(&mut self, picture: &Picture, doctrine: &Doctrine, other: Side, now_ms: u64) {
        let Some(mut halt) = self.halt.clone() else {
            return;
        };
        let young = picture
            .contacts
            .get(&halt.d)
            .filter(|c| self.live_hostile(c, other) && c.age_ms <= held_limit_ms(doctrine));
        // Closing: a row read nearer than any before it in the halt.
        if let Some(range) = young.and_then(|c| c.range_m)
            && range < halt.nearest_m
        {
            halt.nearest_m = range;
            halt.nearest_ms = now_ms;
            self.halt = Some(halt.clone());
        }
        let d = halt.d;
        let band = f64::from(if self.fallen_back {
            widened_band(doctrine.band_m)
        } else {
            doctrine.band_m
        });
        let at_band = young.is_some_and(|c| {
            match (picture.own_pos_at(now_ms), target_pos(picture, c, now_ms)) {
                (Some(own), Some(pos)) => distance_m(own, pos) <= band + HOLD_SLACK_M,
                _ => false,
            }
        });
        let said = if self.hit_taken_ms > halt.since_ms {
            Some("The halt is over: we are found.".to_owned())
        } else if at_band {
            Some(format!("The halt is over: {d} is at the band."))
        } else if young.is_some_and(|c| c.track.is_none())
            && now_ms >= halt.nearest_ms.saturating_add(HALT_STILL_MS)
        {
            Some(format!("The halt is over: {d} has stopped."))
        } else if now_ms >= halt.since_ms.saturating_add(HALT_MAX_MS) {
            Some("The halt is over: its time is up.".to_owned())
        } else if young.is_none() {
            None
        } else {
            return;
        };
        self.halt = None;
        if let Some(said) = said {
            self.note(said);
        }
    }

    /// **The hunt's bookkeeping**, every decision: a fresh hostile target
    /// refreshes the trail; a fresh hostile ends a hunt; a `Held:` notice ends
    /// its leg; and with no hostile held young, a live trail and no withdrawal
    /// flying (nor a re-approach, a follow or a return in force), a hunt
    /// starts at the last plot. An unidentified contact neither
    /// starts nor ends one, and nor does a wreck (the kills are taken in
    /// `observe`).
    fn follow(
        &mut self,
        picture: &Picture,
        doctrine: &Doctrine,
        other: Side,
        target: Option<&str>,
        sign: f64,
        now_ms: u64,
    ) {
        // The trail: the target's last plot while that plot is fresh.
        if let Some(designator) = target
            && !self.wrecks.contains(designator)
            && let Some(contact) = picture.contacts.get(designator)
            && contact.side == Some(other)
            && contact.age_ms <= FRESH_PLOT_MS
            && let Some(plot_m) = last_plot(picture, contact)
        {
            self.trail = Some(Trail {
                designator: designator.to_owned(),
                plot_m,
                plot_ms: contact.read_ms.saturating_sub(contact.age_ms),
                track: contact.track.clone(),
                seen_from_m: picture.own_pos_at(contact.read_ms),
            });
            self.trailed.insert(designator.to_owned());
        }
        // A fresh hostile ends the hunt: the pilot engages it as ever.
        if let Some(hunt) = &self.hunt {
            let fresh = |c: &Contact| self.live_hostile(c, other) && c.age_ms <= FRESH_PLOT_MS;
            let seen = target
                .filter(|t| picture.contacts.get(*t).is_some_and(fresh))
                .map(str::to_owned)
                .or_else(|| {
                    picture
                        .contacts
                        .values()
                        .find(|c| fresh(c))
                        .map(|c| c.designator.clone())
                });
            if let Some(seen) = seen {
                let d = hunt.trail.designator.clone();
                self.hunt = None;
                self.note(format!("The hunt for {d} ends: {seen} is in sight."));
            }
        }
        let own = picture.own_pos_at(now_ms);
        // A `Held:` notice newer than the leg ends the leg (the held turn
        // still happens).
        if let Some(hunt) = &self.hunt
            && self.held_seen_ms > hunt.leg_since_ms
            && let Some(own) = own
        {
            self.next_leg(own, sign, now_ms);
        }
        // The start. A `hunt` doctrine (cut AK.2) hunts as soon as no plot is
        // fresh, rather than flying round a row a few seconds old.
        let held_young = picture
            .contacts
            .values()
            .any(|c| self.live_hostile(c, other) && c.age_ms <= held_limit_ms(doctrine));
        // Cut AN.2: nor while a re-approach is in force; cut AN.3: nor while a
        // broken machine is followed or its return is in force.
        let withdrawing = self.withdraw_until_ms.is_some_and(|until| now_ms < until)
            || doctrine.stance == Stance::Withdraw
            || self.reapproach.is_some()
            || self.follow.is_some()
            || self.ret.is_some();
        if self.hunt.is_none()
            && !held_young
            && !withdrawing
            && let Some(trail) = &self.trail
            && now_ms.saturating_sub(trail.plot_ms) <= HUNT_TRAIL_MS
            && let Some(own) = own
        {
            let trail = trail.clone();
            // The estimated course, fixed now: the track's; else the way the
            // range was opening when it was lost; else the own heading.
            let (course_deg, speed_mps) = match &trail.track {
                Some(track) => (track.course_deg, track.speed_mps),
                None => match trail.seen_from_m {
                    Some(from) if distance_m(from, trail.plot_m) > 1.0 => {
                        (bearing_deg(from, trail.plot_m), HUNT_ASSUMED_MPS)
                    }
                    _ => (f64::from(picture.own.heading_deg), HUNT_ASSUMED_MPS),
                },
            };
            let away = distance_m(own, clamp_arena(trail.plot_m)).round();
            // Cut AL.3: going away or across, judged from here and now (the
            // picture publishes a track only for a mover).
            let going_away = trail.track.as_ref().is_some_and(|track| {
                angle_between(
                    whole(track.course_deg),
                    whole(bearing_deg(own, trail.plot_m)),
                ) <= 90
            });
            self.note(format!(
                "Hunting {}: its last plot, {away:.0} m away.",
                trail.designator
            ));
            self.hunt = Some(Hunt {
                trail,
                leg: HuntLeg::ToPlot,
                since_ms: now_ms,
                leg_since_ms: now_ms,
                course_deg,
                speed_mps,
                sweep_since_ms: None,
                wait_since_ms: None,
                wait_spent: false,
                going_away,
            });
        }
    }

    /// The hunt's next leg: the plot → along the course → the sweep's legs, a
    /// quarter turn each by the compass turn sign `sign` (cut AL.2: the live
    /// way with one track gone) → the end (the trail dropped, the search next).
    fn next_leg(&mut self, own: (f64, f64), sign: f64, now_ms: u64) {
        let Some(hunt) = self.hunt.as_mut() else {
            return;
        };
        let d = hunt.trail.designator.clone();
        let next = match &hunt.leg {
            HuntLeg::ToPlot => {
                let course = whole(hunt.course_deg);
                self.notes.push(format!(
                    "Hunting {d}: along {course:03}° from its last plot."
                ));
                Some(HuntLeg::Along)
            }
            HuntLeg::Along => {
                hunt.sweep_since_ms = Some(now_ms);
                self.notes.push(format!("Sweeping for {d}."));
                Some(HuntLeg::Sweep {
                    leg: 0,
                    origin: clamp_arena(own),
                    heading_deg: norm(hunt.course_deg + 90.0 * sign),
                })
            }
            HuntLeg::Sweep {
                leg, heading_deg, ..
            } => (leg.saturating_add(1) < HUNT_SWEEP_LEGS).then(|| HuntLeg::Sweep {
                leg: leg.saturating_add(1),
                origin: clamp_arena(own),
                heading_deg: norm(heading_deg + 90.0 * sign),
            }),
        };
        match next {
            Some(leg) => {
                hunt.leg = leg;
                hunt.leg_since_ms = now_ms;
            }
            None => self.end_hunt(),
        }
    }

    /// The hunt is over with nothing found: the trail goes, the search follows.
    fn end_hunt(&mut self) {
        if let Some(hunt) = self.hunt.take() {
            self.trail = None;
            let d = hunt.trail.designator;
            self.note(format!("The hunt for {d} ends; searching."));
        }
    }

    /// The hunt's heading now, advancing its legs as they are done; `None`
    /// once it has ended.
    fn hunt_heading(&mut self, own: (f64, f64), sign: f64, now_ms: u64) -> Option<f64> {
        // Each pass either returns or moves the hunt one leg on (or ends it).
        for _ in 0..=usize::from(HUNT_SWEEP_LEGS) + 2 {
            let hunt = self.hunt.as_ref()?;
            match hunt.leg {
                HuntLeg::Along if now_ms.saturating_sub(hunt.trail.plot_ms) > HUNT_TRAIL_MS => {
                    self.next_leg(own, sign, now_ms);
                    continue;
                }
                HuntLeg::Sweep { .. }
                    if hunt
                        .sweep_since_ms
                        .is_some_and(|since| now_ms >= since.saturating_add(HUNT_SWEEP_MS)) =>
                {
                    self.end_hunt();
                    return None;
                }
                _ => {}
            }
            let (aim, clamped) = leg_aim(hunt, now_ms);
            let d = distance_m(own, aim);
            let along_leg = hunt.leg == HuntLeg::Along;
            // The course leg's aim moves; it is done only where the clamp
            // stopped it (the edge).
            if d <= HUNT_ARRIVE_M && (!along_leg || clamped) {
                self.next_leg(own, sign, now_ms);
                continue;
            }
            if along_leg && !clamped {
                let r = hunt.course_deg.to_radians();
                let (dx, dy) = (own.0 - aim.0, own.1 - aim.1);
                let ahead = dx * r.sin() + dy * r.cos();
                let aside = (dx * r.cos() - dy * r.sin()).abs();
                // Past the moving aim by more than the arrival: the estimate
                // is behind, and the course leg is done.
                if ahead > HUNT_ARRIVE_M && aside <= HUNT_ARRIVE_M {
                    self.next_leg(own, sign, now_ms);
                    continue;
                }
                // Caught up with it, or level with it and at most the arrival
                // ahead: drive on along the course rather than turn back to it
                // — never past the edge (the course's own next
                // `HUNT_ARRIVE_M` would cross it: that is the edge reached).
                if d <= HUNT_ARRIVE_M || (ahead >= 0.0 && aside <= HUNT_ARRIVE_M) {
                    if past_the_edge(along(own, hunt.course_deg, HUNT_ARRIVE_M)) {
                        self.next_leg(own, sign, now_ms);
                        continue;
                    }
                    return Some(hunt.course_deg);
                }
            }
            return Some(bearing_deg(own, aim));
        }
        None
    }

    /// Rule 6: `(heading, speed, ask the bearing to B)`, by the one stance
    /// precedence of the module head. A hold not flown this decision goes
    /// back to `Stopped`, and the speed chosen is remembered for the stall
    /// watchdog (cut AL.2).
    #[allow(clippy::too_many_arguments)]
    fn motion(
        &mut self,
        picture: &Picture,
        doctrine: &Doctrine,
        target: Option<&str>,
        full: i32,
        live: Option<Turn>,
        sign: f64,
        now_ms: u64,
    ) -> (Option<f64>, Option<i32>, bool) {
        self.motion_begun();
        let out = self.motion_steps(picture, doctrine, target, full, live, sign, now_ms);
        self.motion_ended(now_ms);
        if let Some(kph) = out.1 {
            self.last_chosen_speed = Some(kph);
        }
        out
    }

    /// **No motion this decision** (cut AM.2): an immobile or disabled tank
    /// flies no step, so its waits close as [`Pilot::motion`]'s do around the
    /// steps when none is flown — the hold back to `Stopped`, no wait, and a
    /// stop for a return over with the rest after it begun. Nothing is
    /// ordered.
    fn idle_motion(&mut self, now_ms: u64) {
        self.motion_begun();
        self.motion_ended(now_ms);
    }

    /// Before the steps: nothing flown yet this decision, and no wait.
    fn motion_begun(&mut self) {
        self.hold_flown = false;
        self.unknown_flown = false;
        self.wait = None;
        self.rule = None;
    }

    /// After the steps: a hold not flown goes back to `Stopped`, and a stop
    /// for a return not flown this decision is over (cut AL.3), for whatever
    /// reason — told apart, its time up, a hit, a higher step, the tank
    /// immobile — and the rest before the next one begins.
    fn motion_ended(&mut self, now_ms: u64) {
        if !self.hold_flown {
            self.hold_motion = HoldMotion::Stopped;
        }
        if !self.unknown_flown && self.unknown_stop.take().is_some() {
            self.unknown_rest_until_ms = now_ms.saturating_add(UNIDENTIFIED_WAIT_MS);
        }
    }

    /// The steps of the precedence. Step 1's own headings are never steered
    /// round held ground; every heading steps 2 to 6 return at a forward speed
    /// is, and every reverse they return whose ray behind the tank meets held
    /// ground is a stop with the bow kept (cut AL.2).
    #[allow(clippy::too_many_arguments)]
    fn motion_steps(
        &mut self,
        picture: &Picture,
        doctrine: &Doctrine,
        target: Option<&str>,
        full: i32,
        live: Option<Turn>,
        sign: f64,
        now_ms: u64,
    ) -> (Option<f64>, Option<i32>, bool) {
        let Some(own) = picture.own_pos_at(now_ms) else {
            self.detour = false;
            return (None, None, false);
        };
        let astern = picture.own.max_astern_kph.unwrap_or(UNKNOWN_ASTERN_KPH);
        if let Some(step_one) = self.step_one(picture, own, full, astern, now_ms) {
            self.detour = false;
            return step_one;
        }
        let (heading, speed, ask) = self.stance_motion(
            picture, doctrine, target, full, astern, live, sign, own, now_ms,
        );
        match (heading, speed) {
            (Some(h), Some(kph)) if kph > 0 => (Some(self.steer_round(own, h, sign)), speed, ask),
            (_, Some(kph)) if kph < 0 => {
                self.detour = false;
                let bow = heading.unwrap_or_else(|| f64::from(self.reference_heading(picture)));
                if self.blocked_behind(own, bow) {
                    // Cut AO.1: a back-off stopped here ends at the next
                    // decision ([`Pilot::back_off_ends`]).
                    if matches!(self.rule, Some("reapproach" | "return_back")) {
                        self.back_off_blocked_ms = now_ms;
                    }
                    (heading, Some(0), ask)
                } else {
                    (heading, speed, ask)
                }
            }
            _ => {
                self.detour = false;
                (heading, speed, ask)
            }
        }
    }

    /// Step 1: a `Held:` notice outranks everything for its three seconds —
    /// the held turn, or after a stop while reversing a stop with the bow kept
    /// (cut AL.2); then the back-off (a stop instead when the ray behind the
    /// tank meets held ground); then a stall's turn while it is open, and after
    /// it frees the tank until the ground is cleared.
    fn step_one(
        &mut self,
        picture: &Picture,
        own: (f64, f64),
        full: i32,
        astern: i32,
        now_ms: u64,
    ) -> Option<(Option<f64>, Option<i32>, bool)> {
        if now_ms < self.held_until_ms {
            if self.held_astern {
                return Some((None, Some(0), false));
            }
            if let Some(held) = self.held_heading {
                return Some((Some(f64::from(held)), Some(full), false));
            }
        }
        if now_ms < self.reverse_until_ms {
            let bow = f64::from(self.reference_heading(picture));
            let kph = if self.blocked_behind(own, bow) {
                0
            } else {
                -astern
            };
            return Some((None, Some(kph), false));
        }
        if let Some(stall) = self.stall_heading {
            if self.stall_level >= 1 || now_ms < self.stall_clear_until_ms {
                return Some((Some(f64::from(stall)), Some(full), false));
            }
            self.stall_heading = None;
        }
        None
    }

    /// Steps 2 to 6 of the precedence.
    #[allow(clippy::too_many_arguments)]
    fn stance_motion(
        &mut self,
        picture: &Picture,
        doctrine: &Doctrine,
        target: Option<&str>,
        full: i32,
        astern: i32,
        live: Option<Turn>,
        sign: f64,
        own: (f64, f64),
        now_ms: u64,
    ) -> (Option<f64>, Option<i32>, bool) {
        let contact = target.and_then(|t| picture.contacts.get(t));
        let aim = contact.and_then(|c| target_pos(picture, c, now_ms));
        // Every withdrawal backs away with the bow on the target (cut AL.2);
        // with no target to face, the current heading at full speed.
        let back = |aim: Option<(f64, f64)>| match aim {
            Some(aim) => (Some(bearing_deg(own, aim)), Some(-astern), false),
            None => (None, Some(full), false),
        };
        // 2. The pilot's own fall-back withdrawal: until its time is up or the
        // target is out at the widened band.
        if let Some(until) = self.withdraw_until_ms {
            let widened = f64::from(widened_band(doctrine.band_m));
            let opened = aim.is_some_and(|aim| distance_m(own, aim) >= widened);
            if now_ms >= until || opened {
                self.withdraw_until_ms = None;
            } else {
                self.orbit_since_ms = None;
                return back(aim);
            }
        }
        // 3. A doctrine `withdraw`, ahead of the widened band.
        if doctrine.stance == Stance::Withdraw {
            self.orbit_since_ms = None;
            return back(aim);
        }
        // 3a. The re-approach (cut AN.2): backing off bow on, steering on the
        // hostile while it is held young and on its plot after (the pilot is
        // blind between 480 and 600 m of a stopped machine); then stopped,
        // facing the plot. The reverse goes through the shared path below
        // (held ground behind turns it into a stop).
        // Cut AO.1: each back-off reverses on [`back_off_bow`] — the bearing to
        // the machine, or the smallest slant off it that leaves room astern —
        // the judge's own function, position and decision; with none (a
        // back-off begun this decision, not yet judged) it stops, the bow on.
        let side = ground_side(picture);
        let back_off = |aim: (f64, f64)| {
            let bearing = bearing_deg(own, aim);
            match back_off_bow(own, bearing, side) {
                Some(bow) => (Some(bow), Some(-astern), false),
                None => (Some(bearing), Some(0), false),
            }
        };
        if let Some(reapproach) = &self.reapproach {
            let plot = reapproach.plot_m;
            let flown = match reapproach.stage {
                ReapproachStage::Open { .. } => {
                    let bow = self
                        .young_pos(picture, doctrine, &reapproach.d, now_ms)
                        .unwrap_or(plot);
                    ("reapproach", back_off(bow))
                }
                ReapproachStage::Wait { .. } => (
                    "approach_wait",
                    (Some(bearing_deg(own, plot)), Some(0), false),
                ),
            };
            self.orbit_since_ms = None;
            self.rule = Some(flown.0);
            return flown.1;
        }
        // 3a. The return (cut AN.3; never in force with a re-approach):
        // backing out of the broken machine's sight bow on its last plot,
        // through the shared reverse path (held ground behind turns it into a
        // stop); then stopped, facing the plot, for its one return.
        if let Some(ret) = &self.ret {
            let flown = match ret.stage {
                // Cut AO.1: the bow on the machine while it is held young, as
                // the re-approach's, else on its last plot.
                ReturnStage::Back { .. } => {
                    let bow = self
                        .young_pos(picture, doctrine, &ret.d, now_ms)
                        .unwrap_or(ret.plot_m);
                    ("return_back", back_off(bow))
                }
                ReturnStage::Wait { .. } => (
                    "return_wait",
                    (Some(bearing_deg(own, ret.plot_m)), Some(0), false),
                ),
            };
            self.orbit_since_ms = None;
            self.rule = Some(flown.0);
            return flown.1;
        }
        // 3b. The turn into a side or rear hit, stopped (cut AL.2).
        if let Some(threat) = self.threat_turn(doctrine, contact, aim, own, now_ms) {
            self.orbit_since_ms = None;
            // Said now that the turn is flown, not when the hit armed it.
            if let Some(note) = self.threat_note.take() {
                self.note(note);
            }
            return (Some(threat), Some(0), false);
        }
        // 3c. Fire from the halt (cut AN.2): stopped, bow on its hostile while
        // that one is held young; the guns fire by rule 5 as always.
        if let Some(halt) = &self.halt
            && let Some(pos) = self.young_pos(picture, doctrine, &halt.d, now_ms)
        {
            self.orbit_since_ms = None;
            self.rule = Some("halt");
            return (Some(bearing_deg(own, pos)), Some(0), false);
        }
        // 4. A hostile held young: the doctrine's stance (a `hunt` doctrine's
        // only while its plot is fresh) — or, for the broken machine followed
        // home (cut AN.3), in its place a hold bow on at the follow's band,
        // the guns firing by rule 5.
        // Cut AO.1: a followed row is flown to `HUNT_STALE_PLOT_MS` under every
        // doctrine — the follow's own lost test reads that limit, so one limit
        // governs a followed row; every other row keeps `held_limit_ms`.
        let followed = |c: &Contact| {
            self.follow
                .as_ref()
                .is_some_and(|follow| follow.d == c.designator)
        };
        if let (Some(contact), Some(aim)) = (contact, aim)
            && contact.age_ms
                <= if followed(contact) {
                    HUNT_STALE_PLOT_MS
                } else {
                    held_limit_ms(doctrine)
                }
        {
            if followed(contact) {
                self.search = SearchLeg::ToB;
                self.rule = Some("follow");
                return self.hold(
                    bearing_deg(own, aim),
                    distance_m(own, aim),
                    FOLLOW_BAND_M,
                    full,
                    astern,
                );
            }
            return self.engage(doctrine, contact, aim, own, full, astern, live, now_ms);
        }
        // 4a and 4b. Seen second (cut AL.3), only while no hostile is held
        // young: stopped, facing a return not yet told apart, then facing a
        // hunted hostile expected back inside the sight.
        let held_young = picture.side.map(Side::other).is_some_and(|other| {
            picture
                .contacts
                .values()
                .any(|c| self.live_hostile(c, other) && c.age_ms <= held_limit_ms(doctrine))
        });
        if !held_young {
            if let Some(bearing) = self.unidentified_stop(picture, now_ms) {
                self.orbit_since_ms = None;
                return (Some(bearing), Some(0), false);
            }
            if let Some(bearing) = self.ambush_wait(own, now_ms) {
                self.orbit_since_ms = None;
                return (Some(bearing), Some(0), false);
            }
        }
        // 5. The hunt, before any stance round a stale row.
        if self.hunt.is_some()
            && let Some(heading) = self.hunt_heading(own, sign, now_ms)
        {
            self.orbit_since_ms = None;
            return (Some(heading), Some(full), false);
        }
        // 4, for an older row the pilot never held fresh (nothing to hunt).
        if let (Some(contact), Some(aim)) = (contact, aim)
            && !self.trailed.contains(&contact.designator)
        {
            return self.engage(doctrine, contact, aim, own, full, astern, live, now_ms);
        }
        // 6. Nothing to fly at: the search, after five quiet seconds. A stale
        // row whose hunt is spent is nothing to fly at.
        self.orbit_since_ms = None;
        let spent = contact.is_some_and(|c| self.trailed.contains(&c.designator));
        if target.is_none() || spent {
            // 6a. The opening while it is not over (cut AN.2), where the
            // search would fly and with no five-second wait.
            if let Some(opening) = self.opening_motion(picture, own, full) {
                return opening;
            }
            let quiet_since = self.last_hostile_ms.unwrap_or(now_ms);
            if now_ms >= quiet_since.saturating_add(SEARCH_AFTER_MS) {
                return self.search(picture, own, full);
            }
        }
        (None, None, false)
    }

    /// **Step 6a: the opening** (cut AN.2), while it is not over: nothing
    /// before its brief is read (the tank stands at the muster), (cut AO.1) a
    /// move in for room at `full` ahead and a stop while the brief after it is
    /// read, its step off the road at [`OPENING_STEP_KPH`], then stopped with
    /// the bow toward the other side's entry. `None` once it is over.
    fn opening_motion(
        &mut self,
        picture: &Picture,
        own: (f64, f64),
        full: i32,
    ) -> Option<(Option<f64>, Option<i32>, bool)> {
        let flown = match &self.opening {
            OpeningStage::Done => return None,
            // Cut AO.1: stopped while a brief after a move in is asked and
            // read; the muster's first brief is waited for as AN.2 waited.
            OpeningStage::Brief { .. } if self.opening_legs > 0 => (None, Some(0), false),
            OpeningStage::Fresh | OpeningStage::Brief { .. } => (None, None, false),
            // Cut AO.1: the move in, at full ahead through the shared forward
            // path (steered round held ground as every forward heading is).
            OpeningStage::Inward { aim_m, .. } => {
                (Some(bearing_deg(own, *aim_m)), Some(full), false)
            }
            OpeningStage::Step { aim_m, .. } => (
                Some(bearing_deg(own, *aim_m)),
                Some(OPENING_STEP_KPH),
                false,
            ),
            OpeningStage::Hold { facing_deg, .. } => {
                let facing = self
                    .other_entry_now(picture)
                    .map_or(f64::from(*facing_deg), |entry| bearing_deg(own, entry));
                (Some(facing), Some(0), false)
            }
        };
        self.rule = Some("opening");
        Some(flown)
    }

    /// Where `d` is now when it is a live hostile held young
    /// ([`held_limit_ms`]), as [`target_pos`] says (cut AN.2).
    fn young_pos(
        &self,
        picture: &Picture,
        doctrine: &Doctrine,
        d: &str,
        now_ms: u64,
    ) -> Option<(f64, f64)> {
        let other = picture.side.map(Side::other)?;
        let contact = picture
            .contacts
            .get(d)
            .filter(|c| self.live_hostile(c, other) && c.age_ms <= held_limit_ms(doctrine))?;
        target_pos(picture, contact, now_ms)
    }

    /// **Step 3b** (cut AL.2): the threat's bearing while its turn lasts and
    /// the doctrine is not an orbit — but nothing when a young target is held
    /// within [`HOLD_BOW_DEG`] of it (the hold already faces it).
    fn threat_turn(
        &mut self,
        doctrine: &Doctrine,
        contact: Option<&Contact>,
        aim: Option<(f64, f64)>,
        own: (f64, f64),
        now_ms: u64,
    ) -> Option<f64> {
        let (bearing, until) = self.threat?;
        if now_ms >= until || doctrine.stance == Stance::Orbit {
            self.threat = None;
            self.threat_note = None;
            return None;
        }
        if let (Some(contact), Some(aim)) = (contact, aim)
            && contact.age_ms <= held_limit_ms(doctrine)
            && angle_between(whole(bearing), whole(bearing_deg(own, aim))) <= HOLD_BOW_DEG
        {
            return None;
        }
        Some(bearing)
    }

    /// **Step 4a: a return not yet told apart** (cut AL.3; "Stop before you are
    /// found", `help cockpit`): the bearing its own row read, to face stopped
    /// while it is told apart. A return qualifies when it has no side, **a row
    /// read** (a notice alone may carry no bearing), a plot no older than
    /// [`FRESH_PLOT_MS`], and was first seen at most [`UNIDENTIFIED_WAIT_MS`]
    /// ago. The stop in force goes on while its return qualifies and no hit
    /// has been taken since it began; otherwise a new one begins for the
    /// nearest that qualifies (the first by designator among equals) — but
    /// not inside the rest after the last ([`Pilot::unknown_rest_until_ms`])
    /// and not past [`UNIDENTIFIED_STOPS_MAX`] a match. Said once per
    /// designator while it stays an unknown return (again when it is found
    /// again).
    fn unidentified_stop(&mut self, picture: &Picture, now_ms: u64) -> Option<f64> {
        let qualifies = |c: &Contact| {
            c.side.is_none()
                && c.range_m.is_some()
                && c.age_ms <= FRESH_PLOT_MS
                && self
                    .unknown_since
                    .get(&c.designator)
                    .is_some_and(|since| now_ms <= since.saturating_add(UNIDENTIFIED_WAIT_MS))
        };
        if let Some((designator, since)) = &self.unknown_stop {
            // In force: on while it qualifies and nothing has hit the tank.
            let going_on = self.hit_taken_ms <= *since;
            let contact = picture.contacts.get(designator).filter(|c| qualifies(c));
            let (Some(contact), true) = (contact, going_on) else {
                return None;
            };
            let bearing = f64::from(contact.bearing_deg);
            let until = self
                .unknown_since
                .get(designator)
                .map_or(now_ms, |first| first.saturating_add(UNIDENTIFIED_WAIT_MS));
            self.wait = Some((designator.clone(), until));
            self.unknown_flown = true;
            return Some(bearing);
        }
        if now_ms < self.unknown_rest_until_ms || self.unknown_stops >= UNIDENTIFIED_STOPS_MAX {
            return None;
        }
        let contact = picture
            .contacts
            .values()
            .filter(|c| qualifies(c))
            .min_by_key(|c| c.range_m)?;
        let designator = contact.designator.clone();
        let bearing = f64::from(contact.bearing_deg);
        let until = self
            .unknown_since
            .get(&designator)
            .map_or(now_ms, |first| first.saturating_add(UNIDENTIFIED_WAIT_MS));
        self.unknown_stops = self.unknown_stops.saturating_add(1);
        self.unknown_stop = Some((designator.clone(), now_ms));
        self.unknown_flown = true;
        self.wait = Some((designator.clone(), until));
        if self.unknown_said.insert(designator.clone()) {
            self.note(format!(
                "Stopping for {designator}: facing {:03}° while it is told apart.",
                whole(bearing)
            ));
        }
        Some(bearing)
    }

    /// **Step 4b: the hunted hostile expected back inside the sight** (cut
    /// AL.3): during a hunt whose wait is not spent, the bearing to its
    /// estimate ([`wait_estimate`]), to face stopped for [`AMBUSH_WAIT_MS`]
    /// from the first decision it is at most [`SEEN_SECOND_M`] off — "A tank
    /// on the move is seen from further away than a tank standing still" and
    /// "let the ground hide you and it drives at where it last saw you". Never
    /// for a hostile the stream said lost a section ([`Pilot::hurt`]: it goes
    /// home, and waiting lets it), nor for one whose trail's track was going
    /// away or across when the hunt began ([`Hunt::going_away`]: its course
    /// within 90° of the bearing from the own tank to its plot then; the
    /// picture publishes a track only for a mover). **The legs are frozen
    /// through the wait** (it returns before the hunt's heading is worked out,
    /// so no arrival or leg change is judged in it); a hit taken after it
    /// began ends it for good, as does its time.
    fn ambush_wait(&mut self, own: (f64, f64), now_ms: u64) -> Option<f64> {
        let hit_taken_ms = self.hit_taken_ms;
        let exempt = {
            let hunt = self.hunt.as_ref()?;
            if hunt.wait_spent {
                return None;
            }
            hunt.going_away || self.hurt.contains(&hunt.trail.designator)
        };
        let hunt = self.hunt.as_mut()?;
        let d = hunt.trail.designator.clone();
        if let Some(since) = hunt.wait_since_ms {
            if exempt || hit_taken_ms > since || now_ms >= since.saturating_add(AMBUSH_WAIT_MS) {
                hunt.wait_spent = true;
                self.note(format!("The wait for {d} is over; hunting on."));
                return None;
            }
            let bearing = bearing_deg(own, wait_estimate(hunt, now_ms));
            self.wait = Some((d, since.saturating_add(AMBUSH_WAIT_MS)));
            return Some(bearing);
        }
        if exempt {
            return None;
        }
        let estimate = wait_estimate(hunt, now_ms);
        let off = distance_m(own, estimate);
        if off > SEEN_SECOND_M {
            return None;
        }
        hunt.wait_since_ms = Some(now_ms);
        let bearing = bearing_deg(own, estimate);
        self.wait = Some((d.clone(), now_ms.saturating_add(AMBUSH_WAIT_MS)));
        self.note(format!(
            "Stopping before {d} finds us: its estimate is {:.0} m off at {:03}°.",
            off.round(),
            whole(bearing)
        ));
        Some(bearing)
    }

    /// The doctrine's stance round a held hostile (cut AL.2): `orbit` as cut AK
    /// built it; `hold` and `hunt` a hold at the band; `close` a hold at
    /// [`CLOSE_M`]; `withdraw` a reverse with the bow on. After the fall back,
    /// at the widened band, and `close` flown as `hold` there.
    #[allow(clippy::too_many_arguments)]
    fn engage(
        &mut self,
        doctrine: &Doctrine,
        contact: &Contact,
        aim: (f64, f64),
        own: (f64, f64),
        full: i32,
        astern: i32,
        live: Option<Turn>,
        now_ms: u64,
    ) -> (Option<f64>, Option<i32>, bool) {
        // A hostile is held: the next search starts over.
        self.search = SearchLeg::ToB;
        let (stance, band) = if self.fallen_back {
            let stance = match doctrine.stance {
                Stance::Close => Stance::Hold,
                stance => stance,
            };
            (stance, f64::from(widened_band(doctrine.band_m)))
        } else {
            (doctrine.stance, f64::from(doctrine.band_m))
        };
        let d = distance_m(own, aim);
        let beta = bearing_deg(own, aim);
        match stance {
            Stance::Orbit => (
                Some(self.orbit(doctrine, contact, aim, own, d, beta, band, live, now_ms)),
                Some(full),
                false,
            ),
            Stance::Hold | Stance::Hunt => self.hold(beta, d, band, full, astern),
            Stance::Close => self.hold(beta, d, CLOSE_M, full, astern),
            Stance::Withdraw => {
                self.orbit_since_ms = None;
                (Some(beta), Some(-astern), false)
            }
        }
    }

    /// **The hold** (cut AL.2): the bow on the hostile at bearing `beta`
    /// always, and the range changed only along that line. From `Stopped` it
    /// closes (`full` ahead) once `d` is past `band` by [`HOLD_SLACK_M`] and
    /// [`HOLD_HYSTERESIS_M`], and opens (`astern`, in reverse) once short of
    /// it by as much; closing stops once within `band + HOLD_SLACK_M`, opening
    /// once out at `band − HOLD_SLACK_M`. At the default 450 m band the tank
    /// reverses only under 375 m and stops reversing at 400 m, so a machine at
    /// the help's 420 m, read at the scope's `400 m`, is a stop.
    pub fn hold(
        &mut self,
        beta: f64,
        d: f64,
        band: f64,
        full: i32,
        astern: i32,
    ) -> (Option<f64>, Option<i32>, bool) {
        self.orbit_since_ms = None;
        self.hold_flown = true;
        let leave = HOLD_SLACK_M + HOLD_HYSTERESIS_M;
        // Closing or opening stops at the slack; a stop then moves again only
        // past the hysteresis (both in one decision when the range jumped).
        if (self.hold_motion == HoldMotion::Ahead && d <= band + HOLD_SLACK_M)
            || (self.hold_motion == HoldMotion::Astern && d >= band - HOLD_SLACK_M)
        {
            self.hold_motion = HoldMotion::Stopped;
        }
        if self.hold_motion == HoldMotion::Stopped {
            if d > band + leave {
                self.hold_motion = HoldMotion::Ahead;
            } else if d < band - leave {
                self.hold_motion = HoldMotion::Astern;
            }
        }
        let kph = match self.hold_motion {
            HoldMotion::Stopped => 0,
            HoldMotion::Ahead => full,
            HoldMotion::Astern => -astern,
        };
        (Some(beta), Some(kph), false)
    }

    /// The orbit's heading: the tangent at the band, bent up to 60° toward the
    /// target when outside it and away when inside, reached 150 m off the band.
    /// With one track gone the circling sense, after the orbit's own choice, is
    /// the one that turns the live way (cut AL.2: counter-clockwise circling is
    /// a counter-clockwise turn), and the pilot's sense is set to it.
    #[allow(clippy::too_many_arguments)]
    fn orbit(
        &mut self,
        doctrine: &Doctrine,
        contact: &Contact,
        aim: (f64, f64),
        own: (f64, f64),
        d: f64,
        beta: f64,
        band: f64,
        live: Option<Turn>,
        now_ms: u64,
    ) -> f64 {
        if self.orbit_since_ms.is_none() {
            self.orbit_since_ms = Some(now_ms);
            self.last_flip_ms = now_ms;
        }
        let s = match doctrine.sense {
            Sense::Cw => {
                self.sense = -1;
                -1.0
            }
            Sense::Ccw => {
                self.sense = 1;
                1.0
            }
            Sense::Auto => match &contact.track {
                // Toward the rear: clockwise when the rear lies clockwise of the
                // pilot as the target sees it. No timed flip while a track holds.
                Some(track) => {
                    let theta = norm(track.course_deg + 180.0 - bearing_deg(aim, own));
                    self.sense = if theta > 0.0 && theta < 180.0 { -1 } else { 1 };
                    self.last_flip_ms = now_ms;
                    f64::from(self.sense)
                }
                None => {
                    if now_ms >= self.last_flip_ms.saturating_add(FLIP_EVERY_MS) {
                        self.sense = -self.sense;
                        self.last_flip_ms = now_ms;
                    }
                    f64::from(self.sense)
                }
            },
        };
        let s = match live {
            Some(turn) => {
                self.sense = match turn {
                    Turn::CounterClockwise => 1,
                    Turn::Clockwise => -1,
                };
                f64::from(self.sense)
            }
            None => s,
        };
        let offset = ((d - band) / ORBIT_CORRECTION_SPAN_M).clamp(-1.0, 1.0) * ORBIT_CORRECTION_DEG;
        beta + s * (90.0 - offset)
    }

    /// The search: for The Crossroads, past it 800 m along the start's line
    /// through it, then back.
    fn search(
        &mut self,
        picture: &Picture,
        own: (f64, f64),
        full: i32,
    ) -> (Option<f64>, Option<i32>, bool) {
        if let SearchLeg::Beyond {
            origin,
            heading_deg,
        } = self.search
        {
            if distance_m(origin, own) < SEARCH_BEYOND_M {
                return (Some(heading_deg), Some(full), true);
            }
            self.search = SearchLeg::ToB;
        }
        if let Some(spot) = self.spot_b_pos
            && distance_m(own, spot) <= SEARCH_ARRIVE_M
        {
            let heading = match self.start_pos {
                Some(start) if distance_m(start, spot) > 1.0 => bearing_deg(start, spot),
                _ => f64::from(picture.own.heading_deg),
            };
            self.search = SearchLeg::Beyond {
                origin: own,
                heading_deg: heading,
            };
            return (Some(heading), Some(full), true);
        }
        let heading = match self.spot_b_pos {
            Some(spot) => Some(bearing_deg(own, spot)),
            None => picture
                .spots
                .get(&SEARCH_SPOT)
                .and_then(|spot| spot.bearing_deg)
                .map(f64::from),
        };
        (heading, Some(full), true)
    }
}

/// A contact's last plot: its last fix, else its row's range and bearing from
/// where the pilot was when the row was read.
fn last_plot(picture: &Picture, contact: &Contact) -> Option<(f64, f64)> {
    match contact.fixes.back() {
        Some(fix) => Some(fix.pos_m),
        None => Some(along(
            picture.own_pos_at(contact.read_ms)?,
            f64::from(contact.bearing_deg),
            f64::from(contact.range_m?),
        )),
    }
}

/// Where the target is now, as best the picture says: its last plot (a fix, or
/// the row's range and bearing from where the pilot was), advanced along a
/// published track by the plot's age, at most 3 s.
pub fn target_pos(picture: &Picture, contact: &Contact, now_ms: u64) -> Option<(f64, f64)> {
    let plot = last_plot(picture, contact)?;
    Some(match &contact.track {
        Some(track) => {
            let age_ms = contact
                .age_ms
                .saturating_add(now_ms.saturating_sub(contact.read_ms))
                .min(TRACK_LEAD_MAX_MS);
            along(
                plot,
                track.course_deg,
                track.speed_mps * age_ms as f64 / 1_000.0,
            )
        }
        None => plot,
    })
}

/// **A track the broken machine's test reads** (cut AO.1): the contact's
/// published track when its speed is at most [`BROKEN_TRACK_MAX_MPS`] (a
/// stock tank cannot drive faster); a faster one is no evidence, read as none.
fn plausible_track(contact: &Contact) -> Option<&Track> {
    contact
        .track
        .as_ref()
        .filter(|track| track.speed_mps <= BROKEN_TRACK_MAX_MPS)
}

/// Rule 5: the guns to fire at `target`, main gun first. None while the
/// picture holds every gun in doubt (a lost section's `its weapons are gone`,
/// cut AL.1), and never a gun whose counted rounds are gone or one the tank
/// said it does not carry.
fn fireable(picture: &Picture, target: &str, other: Side, now_ms: u64) -> Vec<String> {
    if picture.lock.as_deref() != Some(target)
        || picture.lock_friendly
        || picture.weapons_doubt.is_some()
    {
        return Vec::new();
    }
    let Some(contact) = picture.contacts.get(target) else {
        return Vec::new();
    };
    if contact.side != Some(other)
        || contact.age_ms > FRESH_PLOT_MS
        || now_ms.saturating_sub(contact.read_ms) > FRESH_READ_MS
    {
        return Vec::new();
    }
    let main = picture
        .weapons
        .iter()
        .position(|w| w.id == "cannon")
        .unwrap_or(0);
    let order = std::iter::once(main).chain((0..picture.weapons.len()).filter(|i| *i != main));
    order
        .filter_map(|index| picture.weapons.get(index))
        .filter(|w| {
            !picture.struck.contains(&w.id)
                && w.rounds_left() > 0
                && w.percent != Some(0)
                && believed_ready(w, now_ms)
        })
        .map(|w| w.id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;
    use crate::arena::{
        classify,
        picture::{Event, Fix, Section, Track, Weapon},
        settings::{Engagement, Settings},
    };

    const NOW: u64 = 100_000;

    /// A red tank at the origin, powered and aboard in a running match, heading
    /// north, its ceilings read (54 ahead), `SPEED 999` already out.
    fn flying() -> (Picture, Pilot) {
        let mut picture = Picture {
            phase: Phase::Running { match_id: 1 },
            following: Some(1),
            side: Some(Side::Red),
            aboard: true,
            ..Picture::default()
        };
        picture.own.power = Power::Online;
        picture.own.pos_m = Some((0.0, 0.0));
        picture.own.pos_read_ms = NOW;
        picture.own.max_ahead_kph = Some(54);
        picture.own.max_astern_kph = Some(27);
        let mut pilot = Pilot::new();
        pilot.power_ms = Some(NOW - 10_000);
        pilot.ceilings_asked = true;
        (picture, pilot)
    }

    /// A contact at `bearing`/`range` from the origin, read now, fresh.
    fn contact(designator: &str, side: Option<Side>, bearing: u32, range: u32) -> Contact {
        contact_from((0.0, 0.0), designator, side, bearing, range)
    }

    /// A contact at `bearing`/`range` from `from`, read now, fresh.
    fn contact_from(
        from: (f64, f64),
        designator: &str,
        side: Option<Side>,
        bearing: u32,
        range: u32,
    ) -> Contact {
        let pos = along(from, f64::from(bearing), f64::from(range));
        Contact {
            designator: designator.into(),
            was: None,
            side,
            bearing_deg: bearing,
            rel_deg: bearing,
            band: "far".into(),
            range_m: Some(range),
            age_ms: 0,
            read_ms: NOW,
            fixes: VecDeque::from([Fix {
                at_ms: NOW - 500,
                pos_m: pos,
                sigma_m: 60.0,
                from_lock: false,
            }]),
            track: None,
        }
    }

    fn hold(picture: &mut Picture, contact: Contact) {
        picture.contacts.insert(contact.designator.clone(), contact);
    }

    fn weapon(id: &str, ready: bool) -> Weapon {
        Weapon {
            id: id.into(),
            ready,
            rounds: 10,
            rounds_max: 40,
            read_ms: NOW - 100,
            ..Weapon::default()
        }
    }

    /// **The orbit these tests were written for**: cut AK's default made
    /// explicit (cut AL.2 made the hold the default).
    fn doctrine() -> Doctrine {
        Doctrine::pilot_default(&Settings {
            engagement: Engagement::Orbit,
            ..Settings::default()
        })
    }

    /// The pilot's default doctrine now: the hold (cut AL.2).
    fn hold_doctrine() -> Doctrine {
        Doctrine::pilot_default(&Settings::default())
    }

    fn decide(picture: &Picture, pilot: &mut Pilot) -> Orders {
        pilot.decide(picture, &doctrine(), NOW)
    }

    fn heading(orders: &Orders) -> u32 {
        orders.heading.expect("a heading")
    }

    #[test]
    fn orbit_takes_the_tangent_at_the_band_and_bends_outside_and_inside_it() {
        for (sense, tangent) in [(Sense::Ccw, 90), (Sense::Cw, 270)] {
            let (mut picture, mut pilot) = flying();
            hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
            let doctrine = Doctrine {
                sense,
                ..doctrine()
            };
            let orders = pilot.decide(&picture, &doctrine, NOW);
            assert_eq!(heading(&orders), tangent, "{sense:?} at the band");
            assert_eq!(orders.speed, Some(54), "full ahead");
            // Far: 30° off the bearing, toward it.
            let (mut picture, mut pilot) = flying();
            hold(&mut picture, contact("BA", Some(Side::Blue), 0, 750));
            let far = heading(&pilot.decide(&picture, &doctrine, NOW));
            assert_eq!(far, if sense == Sense::Ccw { 30 } else { 330 });
            // Near: 150° off it, opening.
            let (mut picture, mut pilot) = flying();
            hold(&mut picture, contact("BA", Some(Side::Blue), 0, 150));
            let near = heading(&pilot.decide(&picture, &doctrine, NOW));
            assert_eq!(near, if sense == Sense::Ccw { 150 } else { 210 });
            // Half the span off the band (R + 75): half the correction, 30°.
            let (mut picture, mut pilot) = flying();
            hold(&mut picture, contact("BA", Some(Side::Blue), 0, 525));
            let half = heading(&pilot.decide(&picture, &doctrine, NOW));
            assert_eq!(half, if sense == Sense::Ccw { 60 } else { 300 });
        }
    }

    #[test]
    fn close_hold_and_withdraw_headings() {
        let run = |stance: Stance, range: u32| {
            let (mut picture, mut pilot) = flying();
            hold(&mut picture, contact("BA", Some(Side::Blue), 90, range));
            let doctrine = Doctrine {
                stance,
                sense: Sense::Ccw,
                ..doctrine()
            };
            pilot.decide(&picture, &doctrine, NOW)
        };
        assert_eq!(run(Stance::Close, 600).heading, Some(90));
        assert_eq!(run(Stance::Close, 600).speed, Some(54));
        // At 200 m it holds there (cut AL.2: it used to circle there): bow on,
        // stopped.
        let close = run(Stance::Close, 200);
        assert_eq!((close.heading, close.speed), (Some(90), Some(0)));
        assert_eq!(run(Stance::Hold, 700).heading, Some(90));
        // Inside the band past the hysteresis it opens the range in reverse,
        // bow on (cut AL.2: it used to turn away, 270).
        let near = run(Stance::Hold, 200);
        assert_eq!((near.heading, near.speed), (Some(90), Some(-27)));
        let inside = run(Stance::Hold, 460);
        assert_eq!((inside.heading, inside.speed), (Some(90), Some(0)));
        // A withdraw backs away bow on (cut AL.2: it used to drive away, 270).
        let away = run(Stance::Withdraw, 400);
        assert_eq!((away.heading, away.speed), (Some(90), Some(-27)));
    }

    #[test]
    fn auto_sense_circles_toward_a_tracked_targets_rear_with_no_timed_flip() {
        // The target is north of the pilot, coursing east (090): its rear points
        // west. Seen from the target, the pilot is due south (180); the rear
        // (270) lies clockwise of it, so the pilot circles clockwise.
        let (mut picture, mut pilot) = flying();
        let mut ba = contact("BA", Some(Side::Blue), 0, 450);
        ba.track = Some(Track {
            course_deg: 90.0,
            speed_mps: 8.0,
        });
        hold(&mut picture, ba);
        let orders = decide(&picture, &mut pilot);
        assert_eq!(pilot.sense, -1, "clockwise, toward the rear");
        // The track is advanced by the plot's age: with age 0 read now the aim is
        // the plot, so the heading is the tangent clockwise, 270 give or take the
        // radial correction.
        let h = heading(&orders);
        assert!(angle_between(h, 270) <= 5, "heading {h}");
        // Coursing west instead: the rear is east, counter-clockwise of south.
        let (mut picture, mut pilot) = flying();
        let mut ba = contact("BA", Some(Side::Blue), 0, 450);
        ba.track = Some(Track {
            course_deg: 270.0,
            speed_mps: 8.0,
        });
        hold(&mut picture, ba);
        decide(&picture, &mut pilot);
        assert_eq!(pilot.sense, 1, "counter-clockwise, toward the rear");
        // No timed flip while the track holds, however long.
        for step in 1..=5u64 {
            pilot.decide(&picture, &doctrine(), NOW + step * FLIP_EVERY_MS);
            assert_eq!(pilot.sense, 1);
        }
    }

    #[test]
    fn with_no_track_the_orbit_reverses_every_twenty_seconds() {
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
        let mut senses = Vec::new();
        for at in [0, 19_999, 20_000, 39_999, 40_000] {
            let mut picture = picture.clone();
            if let Some(c) = picture.contacts.get_mut("BA") {
                c.read_ms = NOW + at;
            }
            picture.own.pos_read_ms = NOW + at;
            pilot.decide(&picture, &doctrine(), NOW + at);
            senses.push(pilot.sense);
        }
        assert_eq!(senses, [1, 1, -1, -1, 1]);
    }

    /// The orbit's heading at `d` from a target at `beta`, circling by `s` at
    /// `band`, computed from the named constants.
    fn orbit_heading(beta: f64, d: f64, band: u32, s: f64) -> u32 {
        let offset = ((d - f64::from(band)) / ORBIT_CORRECTION_SPAN_M).clamp(-1.0, 1.0)
            * ORBIT_CORRECTION_DEG;
        whole(beta + s * (90.0 - offset))
    }

    /// Half the hull's internals gone.
    fn half_hull(picture: &mut Picture) {
        picture.own.sections.insert(
            "hull".into(),
            Section {
                condition: "damaged".into(),
                internals: Some((30, 60)),
                facets: Default::default(),
            },
        );
    }

    #[test]
    fn falling_back_on_half_the_hull_withdraws_then_widens_the_band() {
        // A band of 300 widens toward the sight's ceiling.
        let band = 300;
        let widened = widened_band(band);
        assert_eq!(
            widened,
            band.saturating_add(FALL_BACK_WIDEN_M).min(FALL_BACK_MAX_M)
        );
        let (mut picture, mut pilot) = flying();
        // Inside the band, and so inside the widened band.
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 250));
        half_hull(&mut picture);
        let doctrine = Doctrine {
            sense: Sense::Ccw,
            band_m: band,
            ..doctrine()
        };
        let orders = pilot.decide(&picture, &doctrine, NOW);
        assert!(pilot.fallen_back);
        // Cut AL.2: in reverse, bow on (it used to turn away, 180).
        assert_eq!(
            (orders.heading, orders.speed),
            (Some(0), Some(-27)),
            "withdrawing: backing away bow on"
        );
        // `WITHDRAW_MS` on it orbits at the widened band: at 250 m that is
        // inside it, so the heading opens past the tangent.
        let mut later = picture.clone();
        let at = NOW + WITHDRAW_MS;
        later.own.pos_read_ms = at;
        if let Some(c) = later.contacts.get_mut("BA") {
            c.read_ms = at;
        }
        let orders = pilot.decide(&later, &doctrine, at);
        assert_eq!(pilot.withdraw_until_ms, None);
        assert_eq!(heading(&orders), orbit_heading(0.0, 250.0, widened, 1.0));
        // A band of 450 stays 450, the sight's ceiling: with the target at
        // 450 m that is the tangent (090), not a wider band's closing turn.
        assert_eq!(widened_band(450), 450);
        let mut at_450 = later.clone();
        hold(&mut at_450, contact("BA", Some(Side::Blue), 0, 450));
        if let Some(c) = at_450.contacts.get_mut("BA") {
            c.read_ms = at;
        }
        let wide = Doctrine {
            band_m: 450,
            ..doctrine.clone()
        };
        let orders = pilot.decide(&at_450, &wide, at);
        assert_eq!(heading(&orders), 90, "the band stays 450");
    }

    #[test]
    fn falling_back_on_a_quarter_facet_from_a_hit_line() {
        let quarter = 400 / FALL_BACK_FACET_DIVISOR;
        let (mut picture, mut pilot) = flying();
        // Inside the widened band, so the withdrawal runs its time.
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 300));
        picture.apply(
            NOW,
            &classify(&format!(
                "Hit on the hull's front facet: {quarter} of 400 armour left."
            )),
        );
        let orders = decide(&picture, &mut pilot);
        assert!(pilot.fallen_back);
        assert_eq!(pilot.withdraw_until_ms, Some(NOW + WITHDRAW_MS));
        // Cut AL.2: in reverse, bow on (it used to turn away, 180).
        assert_eq!((orders.heading, orders.speed), (Some(0), Some(-27)));
        // One more is not a quarter.
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 300));
        picture.apply(
            NOW,
            &classify(&format!(
                "Hit on the hull's front facet: {} of 400 armour left.",
                quarter + 1
            )),
        );
        decide(&picture, &mut pilot);
        assert!(!pilot.fallen_back);
    }

    #[test]
    fn a_held_notice_turns_forty_five_degrees_by_sense_for_three_seconds() {
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
        picture.own.heading_deg = 100;
        picture.events.push_back(Event {
            at_ms: NOW - 10,
            kind: EventKind::Held,
            text: "Held: something is in the way.".into(),
        });
        let doctrine = Doctrine {
            sense: Sense::Cw,
            ..doctrine()
        };
        // A clockwise doctrine: s = -1, so the turn is -45, whatever the
        // pilot's running sense was (+1 until an orbit sets it).
        assert_eq!(pilot.sense, 1);
        let orders = pilot.decide(&picture, &doctrine, NOW);
        assert_eq!(heading(&orders), 55);
        pilot.sent_heading(55, NOW);
        let orders = pilot.decide(&picture, &doctrine, NOW + HELD_FOR_MS - 1);
        assert_eq!(orders.heading, None, "held on 055");
        let orders = pilot.decide(&picture, &doctrine, NOW + HELD_FOR_MS);
        assert_eq!(heading(&orders), 270, "back on the clockwise tangent");
        // Counter-clockwise: +45.
        let held = |sense: Sense, running: i8| {
            let (mut picture, mut pilot) = flying();
            hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
            picture.own.heading_deg = 100;
            pilot.sense = running;
            picture.events.push_back(Event {
                at_ms: NOW - 10,
                kind: EventKind::Held,
                text: "Held: something is in the way.".into(),
            });
            let doctrine = Doctrine {
                sense,
                ..Doctrine::pilot_default(&Settings::default())
            };
            heading(&pilot.decide(&picture, &doctrine, NOW))
        };
        assert_eq!(held(Sense::Ccw, -1), 145);
        assert_eq!(held(Sense::Cw, 1), 55);
        // `Auto`: the pilot's running sense.
        assert_eq!(held(Sense::Auto, 1), 145);
        assert_eq!(held(Sense::Auto, -1), 55);
    }

    #[test]
    fn an_immobile_tank_orders_no_motion_and_a_destroyed_one_nothing() {
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
        picture.weapons = vec![weapon("cannon", true)];
        picture.own.immobile = true;
        let orders = decide(&picture, &mut pilot);
        assert_eq!((orders.heading, orders.speed), (None, None));
        assert_eq!(orders.lock.as_deref(), Some("BA"), "it still locks");
        picture.own.immobile = false;
        picture.own.disabled = true;
        let orders = decide(&picture, &mut pilot);
        assert_eq!((orders.heading, orders.speed), (None, None));
        picture.own.destroyed = true;
        assert_eq!(decide(&picture, &mut pilot), Orders::default());
    }

    #[test]
    fn nothing_unless_aboard_powered_and_running() {
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
        let mut off = picture.clone();
        off.aboard = false;
        assert_eq!(decide(&off, &mut pilot), Orders::default());
        let mut dark = picture.clone();
        dark.own.power = Power::Starting;
        assert_eq!(decide(&dark, &mut pilot), Orders::default());
        let mut seated = picture.clone();
        seated.phase = Phase::Seated { match_id: 1 };
        assert_eq!(decide(&seated, &mut pilot), Orders::default());
    }

    #[test]
    fn never_a_friend_or_an_unknown_locked_or_fired_on() {
        let (mut picture, mut pilot) = flying();
        picture.weapons = vec![weapon("cannon", true)];
        hold(&mut picture, contact("X1", None, 0, 300));
        hold(&mut picture, contact("RB", Some(Side::Red), 90, 200));
        let orders = decide(&picture, &mut pilot);
        assert_eq!(orders.lock, None);
        assert!(orders.fire.is_empty());
        // Even locked, a friend or an unknown is never fired on.
        for (lock, friendly) in [("X1", false), ("RB", true)] {
            picture.lock = Some(lock.into());
            picture.lock_friendly = friendly;
            let orders = decide(&picture, &mut pilot);
            assert!(orders.fire.is_empty(), "{lock}");
            assert_eq!(orders.lock, None);
        }
    }

    #[test]
    fn the_lock_keeps_its_target_until_another_is_a_hundred_metres_nearer() {
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
        hold(&mut picture, contact("BB", Some(Side::Blue), 90, 360));
        picture.lock = Some("BA".into());
        assert_eq!(decide(&picture, &mut pilot).lock, None, "90 m nearer: kept");
        hold(&mut picture, contact("BB", Some(Side::Blue), 90, 350));
        assert_eq!(
            decide(&picture, &mut pilot).lock.as_deref(),
            Some("BB"),
            "100 m nearer"
        );
        // With no lock the nearest fresh plot wins, a stale one last.
        let (mut picture, mut pilot) = flying();
        let mut stale = contact("BA", Some(Side::Blue), 0, 100);
        stale.age_ms = 4_000;
        hold(&mut picture, stale);
        hold(&mut picture, contact("BB", Some(Side::Blue), 90, 500));
        assert_eq!(decide(&picture, &mut pilot).lock.as_deref(), Some("BB"));
        // A lock order goes at most once every 1.5 s.
        pilot.sent_lock(NOW);
        assert_eq!(pilot.decide(&picture, &doctrine(), NOW + 1_499).lock, None);
        assert_eq!(
            pilot
                .decide(&picture, &doctrine(), NOW + 1_500)
                .lock
                .as_deref(),
            Some("BB")
        );
    }

    #[test]
    fn fire_is_gated_on_readiness_rounds_freshness_and_chance() {
        let armed = || {
            let (mut picture, pilot) = flying();
            hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
            picture.lock = Some("BA".into());
            picture.weapons = vec![weapon("machine_gun", true), weapon("cannon", true)];
            (picture, pilot)
        };
        let (picture, mut pilot) = armed();
        assert_eq!(
            decide(&picture, &mut pilot).fire,
            ["cannon", "machine_gun"],
            "the main gun first"
        );
        // Not ready.
        let (mut picture, mut pilot) = armed();
        picture.weapons[1].ready = false;
        assert_eq!(decide(&picture, &mut pilot).fire, ["machine_gun"]);
        // A `ready` read inside 1.5 s of the ack is not believed.
        let (mut picture, mut pilot) = armed();
        picture.weapons[1].fired_ms = Some(NOW - 1_000);
        picture.weapons[1].read_ms = NOW - 100;
        assert_eq!(decide(&picture, &mut pilot).fire, ["machine_gun"]);
        picture.weapons[1].fired_ms = Some(NOW - 2_000);
        assert_eq!(decide(&picture, &mut pilot).fire, ["cannon", "machine_gun"]);
        // A cycle not yet elapsed; then elapsed — on the one clock, which the
        // ack sets from the cycle. A shot and a cycle with no clock set are
        // no clock: the row is read, and it says not ready.
        let (mut picture, mut pilot) = armed();
        picture.weapons[1].ready = false;
        picture.weapons[1].fired_ms = Some(NOW - 4_000);
        picture.weapons[1].cycle_ms = Some(4_000);
        assert_eq!(decide(&picture, &mut pilot).fire, ["machine_gun"]);
        picture.weapons[1].next_ready_ms = Some(NOW - 4_000 + 6_000);
        assert_eq!(decide(&picture, &mut pilot).fire, ["machine_gun"]);
        picture.weapons[1].next_ready_ms = Some(NOW - 4_000 + 4_000);
        assert_eq!(decide(&picture, &mut pilot).fire, ["cannon", "machine_gun"]);
        // `next_ready_ms` from a cycling row.
        let (mut picture, mut pilot) = armed();
        picture.weapons[1].ready = false;
        picture.weapons[1].next_ready_ms = Some(NOW + 1);
        assert_eq!(decide(&picture, &mut pilot).fire, ["machine_gun"]);
        picture.weapons[1].next_ready_ms = Some(NOW);
        assert_eq!(decide(&picture, &mut pilot).fire, ["cannon", "machine_gun"]);
        // No rounds; 0%.
        let (mut picture, mut pilot) = armed();
        picture.weapons[1].rounds = 0;
        picture.weapons[0].percent = Some(0);
        assert!(decide(&picture, &mut pilot).fire.is_empty());
        // A stale lock: the plot too old, or the row read too long ago.
        let (mut picture, mut pilot) = armed();
        if let Some(c) = picture.contacts.get_mut("BA") {
            c.age_ms = 3_001;
        }
        assert!(decide(&picture, &mut pilot).fire.is_empty());
        let (mut picture, mut pilot) = armed();
        if let Some(c) = picture.contacts.get_mut("BA") {
            c.read_ms = NOW - 2_001;
        }
        assert!(decide(&picture, &mut pilot).fire.is_empty());
        // Locked on something else: nothing.
        let (mut picture, mut pilot) = armed();
        hold(&mut picture, contact("BB", Some(Side::Blue), 90, 460));
        picture.lock = Some("BB".into());
        assert_eq!(decide(&picture, &mut pilot).fire, ["cannon", "machine_gun"]);
        picture.lock = None;
        assert!(decide(&picture, &mut pilot).fire.is_empty());
        // No cannon: the first weapon is the main gun.
        let (mut picture, mut pilot) = armed();
        picture.weapons = vec![weapon("laser", true), weapon("machine_gun", true)];
        assert_eq!(decide(&picture, &mut pilot).fire, ["laser", "machine_gun"]);
    }

    #[test]
    fn the_search_drives_for_b_past_it_eight_hundred_metres_and_back() {
        let (mut picture, mut pilot) = flying();
        picture.spots.insert(
            'B',
            crate::arena::picture::Spot {
                name: "The Crossroads".into(),
                range_m: Some(1_000),
                bearing_deg: Some(90),
                read_ms: NOW,
                plotted_m: Some((1_000, 0)),
            },
        );
        // Nothing held since power came up ten seconds ago: searching.
        let orders = decide(&picture, &mut pilot);
        assert!(orders.ask_spot_b);
        assert_eq!(heading(&orders), 90);
        assert_eq!(pilot.spot_b_pos.map(|p| p.0.round()), Some(1_000.0));
        assert_eq!(pilot.start_pos, Some((0.0, 0.0)));
        // Within 100 m of B: on along start→B (090) for 800 m.
        let mut at_b = picture.clone();
        at_b.own.pos_m = Some((950.0, 0.0));
        at_b.own.heading_deg = 90;
        // A `BEARING B` answer read in the same tick as the position changes the
        // spot's bearing, not where the spot lies.
        if let Some(spot) = at_b.spots.get_mut(&'B') {
            spot.bearing_deg = Some(95);
            spot.read_ms = NOW + 1;
        }
        at_b.own.pos_read_ms = NOW + 1;
        pilot.sent_heading(90, NOW);
        let orders = pilot.decide(&at_b, &doctrine(), NOW + 1);
        assert!(matches!(pilot.search, SearchLeg::Beyond { .. }));
        assert!(orders.ask_spot_b);
        assert_eq!(orders.heading, None, "already on 090");
        let mut on = at_b.clone();
        on.own.pos_m = Some((1_700.0, 0.0));
        let orders = pilot.decide(&on, &doctrine(), NOW + 2);
        assert_eq!(orders.heading, None, "still going: 750 m out");
        let mut out = at_b.clone();
        out.own.pos_m = Some((1_760.0, 0.0));
        let orders = pilot.decide(&out, &doctrine(), NOW + 3);
        assert_eq!(pilot.search, SearchLeg::ToB);
        assert_eq!(heading(&orders), 270, "back for B");
        // A hostile held: no search, and not before five quiet seconds.
        let (mut picture, mut pilot) = flying();
        pilot.power_ms = Some(NOW - 4_999);
        assert!(!decide(&picture, &mut pilot).ask_spot_b);
        hold(&mut picture, contact("X1", None, 0, 500));
        let (_, mut fresh) = flying();
        // Cut AL.3: a return not yet told apart, its row read, is faced
        // stopped first…
        let orders = fresh.decide(&picture, &doctrine(), NOW);
        assert!(!orders.ask_spot_b);
        assert_eq!((orders.heading, orders.speed), (Some(0), Some(0)));
        // …and once its time is up, the search as ever.
        assert!(
            fresh
                .decide(&picture, &doctrine(), NOW + UNIDENTIFIED_WAIT_MS + 1)
                .ask_spot_b,
            "an unknown is not a hostile"
        );
    }

    #[test]
    fn the_crossroads_is_plotted_from_a_status_block_never_from_a_bearing_answer() {
        let (mut picture, mut pilot) = flying();
        let read = |picture: &mut Picture, at: u64, lines: &[&str]| {
            for line in lines {
                picture.apply(at, &classify(line));
            }
        };
        // At (1005, 975) The Crossroads is nearest, 290 m at 090: (1295, 975).
        read(
            &mut picture,
            NOW,
            &[
                "Callsign RA.",
                "Red Tank 1: power online, 0 kph, heading 090°",
                "  at 1005 m east, 975 m north; scope clear",
                "  nearest spot: The Crossroads (B), 290 m at 090°",
                "No contacts.",
            ],
        );
        decide(&picture, &mut pilot);
        assert_eq!(pilot.spot_b_pos, Some((1_295.0, 975.0)));
        // Twenty seconds on, at (828, 839) with The Ford nearest, a `STATUS` and
        // a `BEARING B` answer land together: the bearing is fresh, the spot's
        // range is the old block's. B stays where it is.
        let later = NOW + 20_000;
        read(
            &mut picture,
            later,
            &[
                "Callsign RA.",
                "Red Tank 1: power online, 0 kph, heading 090°",
                "  at 828 m east, 839 m north; scope clear",
                "  nearest spot: The Ford (A), 300 m at 200°",
                "Bearing to The Crossroads (B): 070° (rel 340°).",
            ],
        );
        assert_eq!(picture.spots[&'B'].bearing_deg, Some(70));
        assert_eq!(picture.spots[&'B'].range_m, Some(290));
        let orders = pilot.decide(&picture, &doctrine(), later);
        assert_eq!(pilot.spot_b_pos, Some((1_295.0, 975.0)));
        // bearing (828, 839) -> (1295, 975) is 073.8.
        assert_eq!(orders.heading, Some(74));
        // A `RANGE B` answer does not move it either.
        read(
            &mut picture,
            later + 10,
            &["Range to The Crossroads (B): 480 m."],
        );
        pilot.decide(&picture, &doctrine(), later + 10);
        assert_eq!(pilot.spot_b_pos, Some((1_295.0, 975.0)));
    }

    #[test]
    fn a_heading_goes_again_only_eight_degrees_off_or_after_four_seconds() {
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
        let doctrine = Doctrine {
            sense: Sense::Ccw,
            ..doctrine()
        };
        assert_eq!(pilot.decide(&picture, &doctrine, NOW).heading, Some(90));
        pilot.sent_heading(90, NOW);
        pilot.sent_speed(54);
        // 7° off (the target 7° further round): not yet.
        hold(&mut picture, contact("BA", Some(Side::Blue), 7, 450));
        let orders = pilot.decide(&picture, &doctrine, NOW + 100);
        assert_eq!((orders.heading, orders.speed), (None, None));
        assert_eq!(
            pilot.decide(&picture, &doctrine, NOW + 4_000).heading,
            Some(97),
            "any amount after 4 s"
        );
        hold(&mut picture, contact("BA", Some(Side::Blue), 8, 450));
        assert_eq!(
            pilot.decide(&picture, &doctrine, NOW + 100).heading,
            Some(98)
        );
        // An unanswered order is forgotten and goes again.
        pilot.forget_heading();
        assert_eq!(
            pilot.decide(&picture, &doctrine, NOW + 101).heading,
            Some(98)
        );
    }

    #[test]
    fn the_first_speed_order_is_nine_nine_nine() {
        let (mut picture, mut pilot) = flying();
        picture.own.max_ahead_kph = None;
        pilot.ceilings_asked = false;
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
        let orders = decide(&picture, &mut pilot);
        assert_eq!(orders.speed, Some(CEILING_ASK_KPH));
        pilot.sent_speed(CEILING_ASK_KPH);
        // Asked, not yet read: full ahead is 40.
        assert_eq!(decide(&picture, &mut pilot).speed, Some(UNKNOWN_FULL_KPH));
        // Unanswered: asked again.
        pilot.forget_speed();
        assert_eq!(decide(&picture, &mut pilot).speed, Some(CEILING_ASK_KPH));
        // Read: 54.
        picture.apply(
            NOW,
            &classify("That unit makes 54 kph ahead and 27 astern; SPEED -27 to 54."),
        );
        assert_eq!(decide(&picture, &mut pilot).speed, Some(54));
    }

    // -- cut AK.1: the hunt ------------------------------------------------------

    /// Mid-field, where no hunt aim of these tests meets the edge.
    const MID: (f64, f64) = (1_280.0, 1_000.0);

    /// `flying()`, the tank at `pos`.
    fn flying_at(pos: (f64, f64)) -> (Picture, Pilot) {
        let (mut picture, pilot) = flying();
        picture.own.pos_m = Some(pos);
        (picture, pilot)
    }

    /// The own tank read at `pos` at `at`.
    fn read_at(picture: &mut Picture, pos: (f64, f64), at: u64) {
        picture.own.pos_m = Some(pos);
        picture.own.pos_read_ms = at;
    }

    /// A fresh `BA` at `bearing`/`range` from `from`, decided on at `NOW`, then
    /// gone from the scope a second later: the pilot's orders then.
    fn lost(
        from: (f64, f64),
        bearing: u32,
        range: u32,
        track: Option<Track>,
    ) -> (Picture, Pilot, Orders) {
        lost_as(from, bearing, range, track, false)
    }

    /// **`lost`, BA hurt first** (cut AL.3): the stream said `BA's turret is
    /// destroyed.` before the loss, so the hunt drives at once — never waits —
    /// as every hunt in AK.3's runs was of a machine the bot had hurt. The
    /// hunt tests about its legs use it.
    fn lost_hurt(
        from: (f64, f64),
        bearing: u32,
        range: u32,
        track: Option<Track>,
    ) -> (Picture, Pilot, Orders) {
        lost_as(from, bearing, range, track, true)
    }

    fn lost_as(
        from: (f64, f64),
        bearing: u32,
        range: u32,
        track: Option<Track>,
        hurt: bool,
    ) -> (Picture, Pilot, Orders) {
        let (mut picture, mut pilot) = flying_at(from);
        let mut ba = contact_from(from, "BA", Some(Side::Blue), bearing, range);
        ba.track = track;
        hold(&mut picture, ba);
        if hurt {
            hurt_event(&mut picture, "BA", NOW - 100);
        }
        decide(&picture, &mut pilot);
        picture.contacts.clear();
        let orders = pilot.decide(&picture, &doctrine(), NOW + 1_000);
        (picture, pilot, orders)
    }

    /// The stream says `{designator}'s turret is destroyed.` at `at_ms`.
    fn hurt_event(picture: &mut Picture, designator: &str, at_ms: u64) {
        picture.events.push_back(Event {
            at_ms,
            kind: EventKind::TargetSectionLost,
            text: format!("{designator}'s turret is destroyed."),
        });
    }

    fn hunt_of(pilot: &Pilot) -> Hunt {
        pilot.hunt.clone().expect("hunting")
    }

    fn held_event(picture: &mut Picture, at_ms: u64) {
        picture.events.push_back(Event {
            at_ms,
            kind: EventKind::Held,
            text: "Held: the ground ahead will not take the tank.".into(),
        });
    }

    #[test]
    fn a_lost_hostile_is_hunted_to_its_last_plot() {
        let plot = along(MID, 0.0, 450.0);
        for track in [
            None,
            Some(Track {
                course_deg: 90.0,
                speed_mps: 8.0,
            }),
            // Coursing back at the pilot: the first leg is still the plot.
            Some(Track {
                course_deg: 180.0,
                speed_mps: 8.0,
            }),
        ] {
            // Cut AL.3: BA hurt, so hunted at once (the untracked and the
            // closing cases would otherwise wait first; the hunt's legs are
            // what this tests).
            let (_, mut pilot, orders) = lost_hurt(MID, 0, 450, track.clone());
            let hunt = hunt_of(&pilot);
            assert_eq!(hunt.leg, HuntLeg::ToPlot, "{track:?}");
            assert_eq!(hunt.trail.designator, "BA");
            assert!(distance_m(hunt.trail.plot_m, plot) < 1e-6);
            assert_eq!(hunt.trail.track, track);
            assert_eq!(heading(&orders), whole(bearing_deg(MID, plot)));
            assert_eq!(orders.speed, Some(54), "full ahead");
            assert_eq!(
                pilot.take_notes(),
                ["Hunting BA: its last plot, 450 m away."]
            );
        }
    }

    #[test]
    fn a_tracked_hostile_is_hunted_along_its_course() {
        let track = Track {
            course_deg: 90.0,
            speed_mps: 8.0,
        };
        // West of mid-field, so the capped aim stays inside the ground.
        let from = (300.0, 1_000.0);
        let (mut picture, mut pilot, _) = lost(from, 0, 450, Some(track.clone()));
        pilot.take_notes();
        let hunt = hunt_of(&pilot);
        let plot = hunt.trail.plot_m;
        assert_eq!((hunt.course_deg, hunt.speed_mps), (90.0, 8.0));
        // At the plot, ten seconds after it was taken: 80 m on along 090.
        let t = hunt.trail.plot_ms + 10_000;
        read_at(&mut picture, plot, t);
        let orders = pilot.decide(&picture, &doctrine(), t);
        let hunt = hunt_of(&pilot);
        assert_eq!(hunt.leg, HuntLeg::Along);
        let (aim, clamped) = leg_aim(&hunt, t);
        assert!(!clamped);
        assert!(distance_m(aim, (plot.0 + 80.0, plot.1)) < 1e-6, "{aim:?}");
        assert_eq!(heading(&orders), 90);
        assert_eq!(
            pilot.take_notes(),
            ["Hunting BA: along 090° from its last plot."]
        );
        // Far enough on, the aim stops `HUNT_ALONG_MAX_M` past the plot.
        let far =
            hunt.trail.plot_ms + (HUNT_ALONG_MAX_M / track.speed_mps * 1_000.0) as u64 + 5_000;
        let (aim, clamped) = leg_aim(&hunt, far);
        assert!(!clamped);
        assert!(
            distance_m(aim, (plot.0 + HUNT_ALONG_MAX_M, plot.1)) < 1e-6,
            "{aim:?}"
        );
    }

    #[test]
    fn the_hunt_never_aims_past_the_arena_edge() {
        let edge = ARENA_SIDE_M - EDGE_MARGIN_M;
        let track = Track {
            course_deg: 90.0,
            speed_mps: 8.0,
        };
        // A plot 200 m east of a tank at 2 300 m east: at the edge's margin,
        // coursing east.
        let (mut picture, mut pilot, _) = lost((2_300.0, 1_000.0), 90, 200, Some(track));
        let hunt = hunt_of(&pilot);
        let plot = hunt.trail.plot_m;
        let t = hunt.trail.plot_ms + 10_000;
        let course_leg = Hunt {
            leg: HuntLeg::Along,
            ..hunt.clone()
        };
        let (aim, clamped) = leg_aim(&course_leg, t);
        assert!(clamped, "the course runs past the edge");
        assert!(aim.0 <= edge, "{aim:?}");
        // Arriving there ends the course leg: the sweep, inside the ground.
        read_at(&mut picture, plot, t);
        pilot.decide(&picture, &doctrine(), t);
        let hunt = hunt_of(&pilot);
        assert!(
            matches!(hunt.leg, HuntLeg::Sweep { leg: 0, .. }),
            "{hunt:?}"
        );
        let (aim, _) = leg_aim(&hunt, t);
        assert!((EDGE_MARGIN_M..=edge).contains(&aim.0) && (EDGE_MARGIN_M..=edge).contains(&aim.1));
        // A plot outside the square: the leg to it aims inside.
        let (_, pilot, orders) = lost((2_000.0, 1_000.0), 90, 900, None);
        let hunt = hunt_of(&pilot);
        assert!(hunt.trail.plot_m.0 > ARENA_SIDE_M);
        let (aim, clamped) = leg_aim(&hunt, NOW + 1_000);
        assert!(clamped);
        assert!(aim.0 <= edge && aim.0 >= EDGE_MARGIN_M, "{aim:?}");
        assert_eq!(heading(&orders), 90);
    }

    /// The course leg, driven on along the course once level with its moving
    /// aim, never runs the tank past the edge (its aim still short of the
    /// clamp), nor on far past the aim: either ends the leg in the sweep.
    #[test]
    fn the_course_leg_ends_at_the_edge_or_past_its_aim() {
        let edge = ARENA_SIDE_M - EDGE_MARGIN_M;
        // A plot 250 m short of the edge's margin, coursing east at the speed
        // that puts the aim 50 m behind a tank 50 m short of it at half the
        // trail's life.
        let (plot, own_x) = (edge - 250.0, edge - 50.0);
        let half = HUNT_TRAIL_MS / 2;
        let speed = (own_x - 50.0 - plot) / (half as f64 / 1_000.0);
        let course = |speed_mps| Track {
            course_deg: 90.0,
            speed_mps,
        };
        // BA unhurt, going away when it was lost (cut AL.3): never waited
        // for, even level with or past the plot, where the bearing back to
        // the plot is BA's course turned round.
        let on_the_course = |from_x: f64, speed_mps: f64| {
            let (mut picture, mut pilot, _) =
                lost((from_x - 200.0, 1_000.0), 90, 200, Some(course(speed_mps)));
            let hunt = hunt_of(&pilot);
            let t = NOW + 2_000;
            read_at(&mut picture, hunt.trail.plot_m, t);
            pilot.decide(&picture, &doctrine(), t);
            assert_eq!(hunt_of(&pilot).leg, HuntLeg::Along);
            pilot.take_notes();
            (picture, pilot, hunt.trail.plot_ms)
        };
        let (mut picture, mut pilot, plot_ms) = on_the_course(plot, speed);
        let t = plot_ms + half;
        let (aim, clamped) = leg_aim(&hunt_of(&pilot), t);
        assert!(!clamped && (aim.0 - (own_x - 50.0)).abs() < 1e-6, "{aim:?}");
        // Level with the aim, 50 m ahead of it: the course's next 100 m
        // crosses the edge, so the leg is done there.
        read_at(&mut picture, (own_x, 1_000.0), t);
        let orders = pilot.decide(&picture, &doctrine(), t);
        let hunt = hunt_of(&pilot);
        let HuntLeg::Sweep {
            leg: 0,
            heading_deg,
            origin,
        } = hunt.leg
        else {
            panic!("the sweep at the edge: {hunt:?}");
        };
        assert_ne!(heading(&orders), 90, "not on toward the edge");
        assert_eq!(heading(&orders), whole(heading_deg));
        assert!(origin.0 <= edge);
        assert_eq!(pilot.take_notes(), ["Sweeping for BA."]);
        // Mid-field: level with the aim and at most `HUNT_ARRIVE_M` ahead, it
        // drives on along the course; further ahead, the estimate is behind
        // it and the leg is done.
        let track = course(8.0);
        let (mut picture, mut pilot, plot_ms) = on_the_course(MID.0, track.speed_mps);
        let t = plot_ms + 10_000;
        let (aim, clamped) = leg_aim(&hunt_of(&pilot), t);
        assert!(!clamped);
        read_at(&mut picture, (aim.0 + HUNT_ARRIVE_M - 10.0, aim.1), t);
        let orders = pilot.decide(&picture, &doctrine(), t);
        assert_eq!(hunt_of(&pilot).leg, HuntLeg::Along);
        assert_eq!(heading(&orders), 90, "on along the course");
        let t = t + 1_000;
        let (aim, _) = leg_aim(&hunt_of(&pilot), t);
        read_at(&mut picture, (aim.0 + HUNT_ARRIVE_M + 50.0, aim.1), t);
        pilot.decide(&picture, &doctrine(), t);
        assert!(
            matches!(hunt_of(&pilot).leg, HuntLeg::Sweep { leg: 0, .. }),
            "past the aim: the sweep"
        );
    }

    #[test]
    fn a_held_line_ends_a_hunt_leg() {
        let (mut picture, mut pilot, _) = lost(MID, 0, 450, None);
        let t = NOW + 2_000;
        held_event(&mut picture, t - 10);
        let orders = pilot.decide(&picture, &doctrine(), t);
        assert_eq!(hunt_of(&pilot).leg, HuntLeg::Along);
        assert_eq!(
            orders.heading, pilot.held_heading,
            "the held turn still happens"
        );
        let t = t + 1_000;
        held_event(&mut picture, t - 10);
        pilot.decide(&picture, &doctrine(), t);
        assert!(matches!(hunt_of(&pilot).leg, HuntLeg::Sweep { leg: 0, .. }));
        assert!(pilot.take_notes().contains(&"Sweeping for BA.".to_owned()));
    }

    /// Drive a hunt on `track` from mid-field to its sweep; the time then.
    fn to_the_sweep(picture: &mut Picture, pilot: &mut Pilot) -> u64 {
        let hunt = hunt_of(pilot);
        let plot = hunt.trail.plot_m;
        read_at(picture, plot, NOW + 5_000);
        pilot.decide(picture, &doctrine(), NOW + 5_000);
        assert_eq!(hunt_of(pilot).leg, HuntLeg::Along);
        let t = hunt.trail.plot_ms + HUNT_TRAIL_MS + 1;
        read_at(picture, plot, t);
        pilot.decide(picture, &doctrine(), t);
        assert!(matches!(hunt_of(pilot).leg, HuntLeg::Sweep { leg: 0, .. }));
        t
    }

    #[test]
    fn the_hunt_sweeps_then_hands_over_to_the_search() {
        let track = Track {
            course_deg: 90.0,
            speed_mps: 8.0,
        };
        let (mut picture, mut pilot, _) = lost(MID, 0, 450, Some(track.clone()));
        let mut now = to_the_sweep(&mut picture, &mut pilot);
        pilot.take_notes();
        // Each leg a quarter turn on by the pilot's running sense, from the
        // course.
        let sense = f64::from(pilot.sense);
        for k in 0..HUNT_SWEEP_LEGS {
            let orders = pilot.decide(&picture, &doctrine(), now);
            let hunt = hunt_of(&pilot);
            let HuntLeg::Sweep {
                leg, heading_deg, ..
            } = hunt.leg
            else {
                panic!("sweeping: {hunt:?}");
            };
            assert_eq!(leg, k);
            let expected = whole(track.course_deg + 90.0 * sense * f64::from(k + 1));
            assert_eq!(whole(heading_deg), expected);
            assert_eq!(heading(&orders), expected, "leg {k}");
            let (aim, _) = leg_aim(&hunt, now);
            now += 1_000;
            read_at(&mut picture, aim, now);
        }
        let orders = pilot.decide(&picture, &doctrine(), now);
        assert!(pilot.hunt.is_none() && pilot.trail.is_none());
        assert_eq!(pilot.take_notes(), ["The hunt for BA ends; searching."]);
        assert!(orders.ask_spot_b, "the search asks for B");
        // Separately: the sweep ends `HUNT_SWEEP_MS` after it began.
        let (mut picture, mut pilot, _) = lost(MID, 0, 450, Some(track));
        let began = to_the_sweep(&mut picture, &mut pilot);
        pilot.decide(&picture, &doctrine(), began + HUNT_SWEEP_MS - 1);
        assert!(pilot.hunt.is_some());
        let orders = pilot.decide(&picture, &doctrine(), began + HUNT_SWEEP_MS);
        assert!(pilot.hunt.is_none());
        assert!(
            pilot
                .take_notes()
                .contains(&"The hunt for BA ends; searching.".to_owned())
        );
        assert!(orders.ask_spot_b);
    }

    #[test]
    fn a_fresh_hostile_ends_the_hunt() {
        let (mut picture, mut pilot, _) = lost(MID, 0, 450, None);
        pilot.take_notes();
        let t = NOW + 2_000;
        let mut bb = contact_from(MID, "BB", Some(Side::Blue), 90, 450);
        bb.read_ms = t;
        hold(&mut picture, bb);
        let doctrine = Doctrine {
            sense: Sense::Ccw,
            ..doctrine()
        };
        let orders = pilot.decide(&picture, &doctrine, t);
        assert!(pilot.hunt.is_none());
        assert_eq!(
            pilot.take_notes(),
            ["The hunt for BA ends: BB is in sight."]
        );
        assert_eq!(heading(&orders), 180, "the tangent round BB, at 090");
    }

    /// A stale `BA` row, the plot `HUNT_STALE_PLOT_MS` and more old, read at `t`.
    fn stale_ba(picture: &mut Picture, t: u64) {
        let mut stale = contact_from(MID, "BA", Some(Side::Blue), 0, 450);
        stale.age_ms = HUNT_STALE_PLOT_MS + 1;
        stale.read_ms = t;
        hold(picture, stale);
    }

    #[test]
    fn a_stale_hostile_is_hunted_not_orbited() {
        let (mut picture, mut pilot) = flying_at(MID);
        hold(
            &mut picture,
            contact_from(MID, "BA", Some(Side::Blue), 0, 450),
        );
        decide(&picture, &mut pilot);
        let t = NOW + HUNT_STALE_PLOT_MS + 2_000;
        stale_ba(&mut picture, t);
        let orders = pilot.decide(&picture, &doctrine(), t);
        assert_eq!(hunt_of(&pilot).leg, HuntLeg::ToPlot);
        assert_eq!(
            heading(&orders),
            0,
            "the bearing to its plot, not a tangent"
        );
        // A row that is already that old at its first sight has no trail and
        // is nothing to hunt: it is flown round like a young one.
        let doctrine = Doctrine {
            sense: Sense::Ccw,
            ..doctrine()
        };
        let (mut picture, mut pilot) = flying_at(MID);
        stale_ba(&mut picture, NOW);
        let orders = pilot.decide(&picture, &doctrine, NOW);
        assert!(pilot.trail.is_none() && pilot.hunt.is_none());
        assert_eq!(
            heading(&orders),
            orbit_heading(0.0, 450.0, doctrine.band_m, 1.0),
            "the orbit's tangent"
        );
        assert!(!orders.ask_spot_b);
    }

    #[test]
    fn a_stale_row_does_not_block_the_search_after_a_hunt() {
        let (mut picture, mut pilot) = flying_at(MID);
        hold(
            &mut picture,
            contact_from(MID, "BA", Some(Side::Blue), 0, 450),
        );
        decide(&picture, &mut pilot);
        let plot = pilot.trail.clone().expect("a trail").plot_m;
        // Hunted to the plot, along, into the sweep and out of it, the stale
        // row listed all the while.
        let mut t = NOW + HUNT_STALE_PLOT_MS + 2_000;
        for (pos, at) in [(MID, t), (plot, t + 1_000), (plot, NOW + HUNT_TRAIL_MS + 1)] {
            t = at;
            stale_ba(&mut picture, t);
            read_at(&mut picture, pos, t);
            pilot.decide(&picture, &doctrine(), t);
        }
        assert!(matches!(hunt_of(&pilot).leg, HuntLeg::Sweep { .. }));
        t += HUNT_SWEEP_MS;
        stale_ba(&mut picture, t);
        let orders = pilot.decide(&picture, &doctrine(), t);
        assert!(pilot.hunt.is_none());
        assert_eq!(
            pilot.last_hostile_ms,
            Some(NOW),
            "the stale row does not count as held"
        );
        assert!(t >= NOW + SEARCH_AFTER_MS);
        assert!(orders.ask_spot_b, "searching, not orbiting the stale plot");
        assert_eq!(pilot.orbit_since_ms, None);
    }

    #[test]
    fn a_kill_drops_the_trail() {
        let (mut picture, mut pilot) = flying_at(MID);
        hold(
            &mut picture,
            contact_from(MID, "BA", Some(Side::Blue), 0, 450),
        );
        decide(&picture, &mut pilot);
        assert!(pilot.trail.is_some());
        picture.events.push_back(Event {
            at_ms: NOW + 100,
            kind: EventKind::Kill,
            text: "BA destroyed.".into(),
        });
        // The wreck is still listed, fresh: it is not trailed again.
        pilot.decide(&picture, &doctrine(), NOW + 200);
        assert!(pilot.trail.is_none());
        assert!(pilot.wrecks.contains("BA"));
        picture.contacts.clear();
        picture.events.push_back(Event {
            at_ms: NOW + 900,
            kind: EventKind::Lost,
            text: "Contact BA lost.".into(),
        });
        pilot.decide(&picture, &doctrine(), NOW + 1_000);
        assert!(pilot.hunt.is_none());
        assert!(pilot.take_notes().is_empty(), "no hunt started");
        // An unnamed kill of the lock.
        let (mut picture, mut pilot) = flying_at(MID);
        hold(
            &mut picture,
            contact_from(MID, "BA", Some(Side::Blue), 0, 450),
        );
        picture.lock = Some("BA".into());
        decide(&picture, &mut pilot);
        picture.lock = None;
        picture.events.push_back(Event {
            at_ms: NOW + 100,
            kind: EventKind::Kill,
            text: "Target destroyed.".into(),
        });
        picture.contacts.clear();
        pilot.decide(&picture, &doctrine(), NOW + 1_000);
        assert!(pilot.trail.is_none() && pilot.hunt.is_none());
    }

    /// A wreck's row, still listed fresh, is no hostile: it neither holds off
    /// nor ends the hunt for another, is never targeted, locked or shot at,
    /// and does not keep the search away.
    #[test]
    fn a_wreck_neither_holds_nor_ends_a_hunt() {
        let (mut picture, mut pilot) = flying_at(MID);
        hold(
            &mut picture,
            contact_from(MID, "BB", Some(Side::Blue), 0, 300),
        );
        hold(
            &mut picture,
            contact_from(MID, "BA", Some(Side::Blue), 90, 450),
        );
        decide(&picture, &mut pilot);
        assert_eq!(
            pilot.trail.as_ref().map(|t| t.designator.as_str()),
            Some("BB")
        );
        // BA is destroyed (its row stays, fresh), and BB drops out of sight.
        picture.lock = Some("BA".into());
        picture.events.push_back(Event {
            at_ms: NOW + 100,
            kind: EventKind::Kill,
            text: "BA destroyed.".into(),
        });
        // Cut AL.3: BB hurt before it went, so it is hunted at once.
        hurt_event(&mut picture, "BB", NOW + 100);
        picture.contacts.remove("BB");
        let wreck = |picture: &mut Picture, t: u64| {
            if let Some(ba) = picture.contacts.get_mut("BA") {
                ba.read_ms = t;
            }
        };
        let t = NOW + 1_000;
        wreck(&mut picture, t);
        let orders = pilot.decide(&picture, &doctrine(), t);
        assert_eq!(hunt_of(&pilot).trail.designator, "BB", "the hunt starts");
        assert_eq!(
            pilot.take_notes(),
            [format!(
                "Hunting BB: its last plot, {:.0} m away.",
                distance_m(MID, along(MID, 0.0, 300.0))
            )]
        );
        assert!(
            orders.fire.is_empty() && orders.lock.is_none(),
            "{orders:?}"
        );
        assert_eq!(heading(&orders), 0, "to BB's plot, not round the wreck");
        // Still listed, still fresh: the hunt goes on, nothing is said.
        let t = t + 1_000;
        wreck(&mut picture, t);
        pilot.decide(&picture, &doctrine(), t);
        assert!(pilot.hunt.is_some());
        assert!(pilot.take_notes().is_empty());
        assert_eq!(pilot.last_hostile_ms, Some(NOW), "the wreck is not held");
    }

    #[test]
    fn an_unidentified_contact_neither_starts_nor_ends_a_hunt() {
        // An unknown alone is never trailed, so it is never hunted.
        let (mut picture, mut pilot) = flying_at(MID);
        hold(&mut picture, contact_from(MID, "X1", None, 0, 450));
        decide(&picture, &mut pilot);
        assert!(pilot.trail.is_none());
        picture.contacts.clear();
        pilot.decide(&picture, &doctrine(), NOW + 1_000);
        assert!(pilot.hunt.is_none());
        // A hunt starts with a fresh unknown held, and one does not end it.
        let (mut picture, mut pilot) = flying_at(MID);
        hold(
            &mut picture,
            contact_from(MID, "BA", Some(Side::Blue), 0, 450),
        );
        decide(&picture, &mut pilot);
        picture.contacts.clear();
        for t in [NOW + 1_000, NOW + 2_000] {
            let mut x2 = contact_from(MID, "X2", None, 90, 300);
            x2.read_ms = t;
            hold(&mut picture, x2);
            pilot.decide(&picture, &doctrine(), t);
            assert_eq!(hunt_of(&pilot).trail.designator, "BA");
        }
    }

    /// **The bot-side observer rule for the new state**: fed through
    /// [`Picture::apply`], the trail and the hunt name only a designator a
    /// contact row or a contact notice named.
    #[test]
    fn the_trail_names_only_a_designator_the_stream_named() {
        let (mut picture, mut pilot) = flying();
        let lines = [
            (NOW, "Callsign RA."),
            (NOW, "Red Tank 1: power online, 0 kph, heading 000°"),
            (NOW, "  at 1280 m east, 1000 m north; scope holds a hostile"),
            (NOW + 10, "New contact X1: bearing 000°, far."),
            (NOW + 20, "Contact X1 identified: BA, blue."),
            (
                NOW + 30,
                "BA  blue  bearing 000° (rel 000°)  far  450 m  0 s",
            ),
            (NOW + 40, "No such contact."),
        ];
        let mut named = BTreeSet::new();
        let mut read = |picture: &mut Picture, lines: &[(u64, &str)]| {
            for (at, text) in lines {
                let line = classify(text);
                match &line {
                    crate::arena::Line::ContactRow(row) => {
                        named.insert(row.designator.clone());
                    }
                    crate::arena::Line::NewContact { designator, .. }
                    | crate::arena::Line::ContactLost { designator } => {
                        named.insert(designator.clone());
                    }
                    crate::arena::Line::Identified {
                        was, designator, ..
                    } => {
                        named.insert(was.clone());
                        named.insert(designator.clone());
                    }
                    _ => {}
                }
                picture.apply(*at, &line);
            }
        };
        read(&mut picture, &lines);
        pilot.decide(&picture, &doctrine(), NOW + 50);
        let trail = pilot.trail.clone().expect("a trail");
        // Cut AL.3: BA hurt before it went, so it is hunted at once (the
        // waits' sentences have their own test).
        read(&mut picture, &[(NOW + 900, "BA's turret is destroyed.")]);
        read(&mut picture, &[(NOW + 1_000, "Contact BA lost.")]);
        pilot.decide(&picture, &doctrine(), NOW + 1_010);
        let hunt = hunt_of(&pilot);
        assert!(named.contains(&trail.designator), "{named:?}");
        assert!(named.contains(&hunt.trail.designator));
        for note in pilot.take_notes() {
            let who = note
                .strip_prefix("Hunting ")
                .and_then(|rest| rest.split(':').next())
                .expect("a hunt sentence");
            assert!(named.contains(who), "{note}");
        }
    }

    #[test]
    fn only_the_hulls_own_plates_fall_back() {
        let quarter = 400 / FALL_BACK_FACET_DIVISOR;
        let plate = |section: &str, left: i64, max: i64| {
            let (mut picture, _) = flying();
            picture.own.sections.insert(
                section.into(),
                Section {
                    condition: "damaged".into(),
                    internals: None,
                    facets: [("front".to_owned(), (left, max))].into_iter().collect(),
                },
            );
            fall_back_due(&picture)
        };
        assert_eq!(plate("turret", quarter, 400), None, "a turret plate");
        assert_eq!(plate("left_track", 0, 100), None, "a track plate");
        // Nor does the pilot, deciding: it flies exactly as an unhurt tank
        // would, no withdrawal and nothing said.
        let flown = |plate: Option<(&str, i64, i64)>| {
            let (mut picture, mut pilot) = flying();
            hold(&mut picture, contact("BA", Some(Side::Blue), 0, 300));
            if let Some((section, left, max)) = plate {
                picture.own.sections.insert(
                    section.into(),
                    Section {
                        condition: "damaged".into(),
                        internals: None,
                        facets: [("front".to_owned(), (left, max))].into_iter().collect(),
                    },
                );
            }
            let orders = decide(&picture, &mut pilot);
            (orders.heading, pilot.fallen_back, pilot.take_notes())
        };
        let unhurt = flown(None);
        assert_ne!(unhurt.0, Some(180), "an orbit, not a withdrawal");
        assert!(!unhurt.1 && unhurt.2.is_empty(), "{unhurt:?}");
        for hurt in [("turret", quarter, 400), ("left_track", 0, 100)] {
            assert_eq!(flown(Some(hurt)), unhurt, "{hurt:?}");
        }
        assert_eq!(
            plate("hull", quarter, 400),
            Some(FallBackWhy::Plate {
                facet: "front".into(),
                gone: false
            })
        );
        // Each that does falls back once, with its sentence.
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 300));
        picture.apply(
            NOW,
            &classify(&format!(
                "Hit on the hull's front facet: {quarter} of 400 armour left."
            )),
        );
        decide(&picture, &mut pilot);
        let band = widened_band(doctrine().band_m);
        assert_eq!(
            pilot.take_notes(),
            [format!(
                "Falling back to {band} m: the hull's front plate is at a quarter."
            )]
        );
        // A plate the `STATUS` never read, gone: stored as (0, 0).
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 300));
        picture.apply(
            NOW,
            &classify("Hit on the hull's rear facet: armour gone, 140 of 200 inside."),
        );
        assert_eq!(picture.own.sections["hull"].facets["rear"], (0, 0));
        assert!(!hull_failing(&picture));
        decide(&picture, &mut pilot);
        assert!(pilot.fallen_back);
        assert_eq!(
            pilot.take_notes(),
            [format!(
                "Falling back to {band} m: the hull's rear plate is gone."
            )]
        );
        let (mut picture, mut pilot) = flying();
        half_hull(&mut picture);
        assert!(hull_failing(&picture));
        assert_eq!(fall_back_due(&picture), Some(FallBackWhy::HullHalf));
        decide(&picture, &mut pilot);
        assert_eq!(
            pilot.take_notes(),
            [format!("Falling back to {band} m: the hull is at half.")]
        );
        decide(&picture, &mut pilot);
        assert!(pilot.take_notes().is_empty(), "once a match");
    }

    #[test]
    fn the_fall_back_band_never_narrows_and_stays_inside_the_sight() {
        for band in (150..=SIGHT_BAND_MAX_M).step_by(10) {
            let widened = widened_band(band);
            assert!(band <= widened && widened <= SIGHT_BAND_MAX_M, "{band}");
        }
        assert_eq!(FALL_BACK_MAX_M, SIGHT_BAND_MAX_M);
        // The withdrawal ends once the target is out at the widened band,
        // before its time.
        let band = 300;
        let widened = widened_band(band);
        let doctrine = Doctrine {
            sense: Sense::Ccw,
            band_m: band,
            ..doctrine()
        };
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 100));
        half_hull(&mut picture);
        // Cut AL.2: in reverse, bow on (it used to turn away, 180).
        let orders = pilot.decide(&picture, &doctrine, NOW);
        assert_eq!((orders.heading, orders.speed), (Some(0), Some(-27)));
        let t = NOW + 1_000;
        assert!(t < NOW + WITHDRAW_MS);
        let mut out = contact("BA", Some(Side::Blue), 0, widened);
        out.read_ms = t;
        hold(&mut picture, out);
        let orders = pilot.decide(&picture, &doctrine, t);
        assert_eq!(pilot.withdraw_until_ms, None);
        assert_eq!(heading(&orders), 90, "the tangent at the widened band");
    }

    #[test]
    fn after_the_fall_back_the_doctrine_still_steers() {
        let band = 300;
        let widened = widened_band(band);
        let fly = |stance: Stance| {
            let doctrine = Doctrine {
                stance: Stance::Orbit,
                sense: Sense::Ccw,
                band_m: band,
                ..doctrine()
            };
            let (mut picture, mut pilot) = flying();
            hold(&mut picture, contact("BA", Some(Side::Blue), 0, 100));
            half_hull(&mut picture);
            pilot.decide(&picture, &doctrine, NOW);
            assert!(pilot.fallen_back);
            let t = NOW + WITHDRAW_MS;
            let mut at = contact("BA", Some(Side::Blue), 0, widened);
            at.read_ms = t;
            hold(&mut picture, at);
            let orders = pilot.decide(&picture, &Doctrine { stance, ..doctrine }, t);
            (orders.heading, orders.speed)
        };
        // Hold at the widened band: at it, stopped facing the target.
        assert_eq!(fly(Stance::Hold), (Some(0), Some(0)));
        // Close is flown as a hold there (not straight in to 200 m; cut AL.2:
        // it used to circle there, 090 at 54).
        assert_eq!(fly(Stance::Close), (Some(0), Some(0)));
        // A doctrine withdraw backs away bow on, ahead of the widened band
        // (cut AL.2: it used to turn away, 180 at 54).
        assert_eq!(fly(Stance::Withdraw), (Some(0), Some(-27)));
        // A hunt flies at the widened band too: at it, it holds (cut AL.2: it
        // used to circle, 090 at 54).
        assert_eq!(fly(Stance::Hunt), (Some(0), Some(0)));
        // The orbit is still the orbit there: the tangent.
        assert_eq!(fly(Stance::Orbit), (Some(90), Some(54)));
    }

    // -- cut AK.2: the hunt stance ---------------------------------------------------

    fn hunting(band_m: u32) -> Doctrine {
        Doctrine {
            stance: Stance::Hunt,
            sense: Sense::Ccw,
            band_m,
            ..doctrine()
        }
    }

    #[test]
    fn the_hunt_stance_drives_in_to_the_band_then_holds() {
        let band = 300;
        let fly = |range: u32| {
            let (mut picture, mut pilot) = flying();
            hold(&mut picture, contact("BA", Some(Side::Blue), 0, range));
            let orders = pilot.decide(&picture, &hunting(band), NOW);
            (orders.heading, orders.speed)
        };
        // Outside the band, its slack and the hysteresis: straight at it,
        // full ahead.
        assert_eq!(fly(900), (Some(0), Some(54)));
        let slack = HOLD_SLACK_M as u32;
        let leave = (HOLD_SLACK_M + HOLD_HYSTERESIS_M) as u32;
        assert_eq!(fly(band + leave + 10), (Some(0), Some(54)));
        // At the band, and inside the slack and the hysteresis: the hold, bow
        // on and stopped (cut AL.2: it used to circle there).
        assert_eq!(fly(band), (Some(0), Some(0)));
        assert_eq!(fly(band + slack), (Some(0), Some(0)));
        assert_eq!(fly(band + slack + 10), (Some(0), Some(0)));
        // Well inside the band (past the slack and the hysteresis): in
        // reverse, bow on.
        assert_eq!(fly(band - leave - 10), (Some(0), Some(-27)));
        // A fresh friend is never hunted: nothing to fly at yet.
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact("RB", Some(Side::Red), 0, 900));
        assert_eq!(pilot.decide(&picture, &hunting(band), NOW).heading, None);
    }

    #[test]
    fn the_hunt_stance_with_nothing_held_hunts_the_trail_at_once() {
        // BA held fresh, then its row four seconds old: under an orbit
        // doctrine it is still circled; under a hunt it is hunted at once.
        let stale = |doctrine: &Doctrine| {
            let (mut picture, mut pilot) = flying_at(MID);
            hold(
                &mut picture,
                contact_from(MID, "BA", Some(Side::Blue), 0, 450),
            );
            // Cut AL.3: BA hurt, so a hunt drives at once.
            hurt_event(&mut picture, "BA", NOW - 100);
            pilot.decide(&picture, doctrine, NOW);
            assert!(pilot.hunt.is_none());
            let t = NOW + 1_000;
            if let Some(ba) = picture.contacts.get_mut("BA") {
                ba.age_ms = FRESH_PLOT_MS + 1_000;
                ba.read_ms = t;
            }
            const { assert!(FRESH_PLOT_MS + 1_000 <= HUNT_STALE_PLOT_MS) };
            read_at(&mut picture, MID, t);
            let orders = pilot.decide(&picture, doctrine, t);
            (pilot, orders)
        };
        // What is flown tells the two limits apart: the orbit's tangent round
        // the four-second row (BA due north at the band), not its last plot.
        let (pilot, orders) = stale(&doctrine());
        assert!(pilot.hunt.is_none(), "an orbit circles it");
        assert!(
            [90, 270].contains(&heading(&orders)),
            "the tangent: {orders:?}"
        );
        for target in [None, Some("BA".to_owned())] {
            let doctrine = Doctrine {
                target,
                ..hunting(450)
            };
            let (mut pilot, orders) = stale(&doctrine);
            let hunt = hunt_of(&pilot);
            assert_eq!(hunt.leg, HuntLeg::ToPlot);
            assert_eq!(hunt.trail.designator, "BA");
            // Toward its last plot, due north, full ahead — not round the row.
            assert_eq!((orders.heading, orders.speed), (Some(0), Some(54)));
            assert!(
                pilot
                    .take_notes()
                    .iter()
                    .any(|n| n.starts_with("Hunting BA: its last plot, "))
            );
        }
        // The row gone altogether: the same hunt, at once.
        let (mut picture, mut pilot) = flying_at(MID);
        hold(
            &mut picture,
            contact_from(MID, "BA", Some(Side::Blue), 0, 450),
        );
        hurt_event(&mut picture, "BA", NOW - 100);
        pilot.decide(&picture, &hunting(450), NOW);
        picture.contacts.clear();
        let orders = pilot.decide(&picture, &hunting(450), NOW + 1_000);
        assert_eq!(hunt_of(&pilot).leg, HuntLeg::ToPlot);
        assert_eq!(heading(&orders), 0, "toward its last plot, due north");
        // With no trail, the search as ever: the power came up ten seconds
        // ago with nothing held since, more than five quiet seconds.
        let (picture, mut pilot) = flying_at(MID);
        assert!(NOW >= pilot.power_ms.expect("power") + SEARCH_AFTER_MS);
        let orders = pilot.decide(&picture, &hunting(450), NOW);
        assert!(pilot.hunt.is_none());
        assert!(orders.ask_spot_b, "the search");
    }

    #[test]
    fn a_doctrine_naming_a_row_too_old_to_fight_gives_way_to_a_fresh_hostile() {
        // BA held fresh (so trailed) due north, then its row four seconds old,
        // while BB is fresh 500 m due south: the doctrine still names BA.
        let fly = |doctrine: &Doctrine| {
            let (mut picture, mut pilot) = flying_at(MID);
            hold(
                &mut picture,
                contact_from(MID, "BA", Some(Side::Blue), 0, 450),
            );
            pilot.decide(&picture, doctrine, NOW);
            assert!(pilot.trailed.contains("BA"));
            let t = NOW + 1_000;
            if let Some(ba) = picture.contacts.get_mut("BA") {
                ba.age_ms = FRESH_PLOT_MS + 1_000;
                ba.read_ms = t;
            }
            let mut bb = contact_from(MID, "BB", Some(Side::Blue), 180, 500);
            bb.read_ms = t;
            hold(&mut picture, bb);
            read_at(&mut picture, MID, t);
            let orders = pilot.decide(&picture, doctrine, t);
            (pilot, orders)
        };
        // A hunt fights only a fresh plot: BA is too old, so the pilot flies
        // at BB (outside the band) rather than at nothing.
        let hunt = Doctrine {
            target: Some("BA".to_owned()),
            ..hunting(300)
        };
        let (pilot, orders) = fly(&hunt);
        assert!(pilot.hunt.is_none(), "BB is in sight");
        assert_eq!((orders.heading, orders.speed), (Some(180), Some(54)));
        // An orbit still fights a four-second row: BA is circled as named.
        let orbit = Doctrine {
            target: Some("BA".to_owned()),
            ..doctrine()
        };
        let (_, orders) = fly(&orbit);
        assert!(
            [90, 270].contains(&heading(&orders)),
            "round BA: {orders:?}"
        );
    }

    // -- cut AK.1: the held tank ---------------------------------------------------

    #[test]
    fn a_held_stop_orders_speed_again() {
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
        picture.own.heading_deg = 100;
        pilot.sent_speed(54);
        assert_eq!(decide(&picture, &mut pilot).speed, None, "already ordered");
        held_event(&mut picture, NOW);
        let orders = pilot.decide(&picture, &doctrine(), NOW + 10);
        assert_eq!(orders.heading, Some(whole(100.0 + HELD_TURN_DEG)));
        assert_eq!(orders.speed, Some(54), "the speed goes again");
        assert_eq!(pilot.held_from_deg, Some(100));
    }

    /// A `STATUS` read at `t`: held at 0 kph on `heading`, the contact read
    /// with it.
    fn held_read(picture: &mut Picture, t: u64, heading: u32) {
        picture.own.held = Some(super::super::picture::HeldCause::Impassable);
        picture.own.speed_kph = 0;
        picture.own.heading_deg = heading;
        picture.own.pos_read_ms = t;
        for contact in picture.contacts.values_mut() {
            contact.read_ms = t;
        }
    }

    #[test]
    fn a_tank_still_held_turns_further_from_where_it_stopped_then_backs_off() {
        // Orbiting BA counter-clockwise: the doctrine's heading is 090 or so,
        // never the stall's.
        let doctrine = Doctrine {
            sense: Sense::Ccw,
            ..doctrine()
        };
        let stopped = 90;
        let start = |picture: &mut Picture, pilot: &mut Pilot| {
            hold(picture, contact_from(MID, "BA", Some(Side::Blue), 0, 450));
            picture.own.heading_deg = stopped;
            held_event(picture, NOW - 10);
            let orders = pilot.decide(picture, &doctrine, NOW);
            assert_eq!(
                orders.heading,
                Some(whole(f64::from(stopped) + HELD_TURN_DEG))
            );
            // The held turn is over; the tank still reads held: a stall opens.
            let t0 = NOW + HELD_FOR_MS;
            held_read(picture, t0, stopped + 45);
            pilot.decide(picture, &doctrine, t0);
            assert_eq!((pilot.stall_since_ms, pilot.stall_level), (Some(t0), 0));
            t0
        };
        let (mut picture, mut pilot) = flying_at(MID);
        let t0 = start(&mut picture, &mut pilot);
        // A read inside `STALL_MS` does not escalate.
        held_read(&mut picture, t0 + STALL_MS - 1, 200);
        let orders = pilot.decide(&picture, &doctrine, t0 + STALL_MS - 1);
        assert_eq!(pilot.stall_level, 0);
        assert_eq!(orders.heading, Some(90), "the doctrine still steers");
        pilot.take_notes();
        // Each read `STALL_MS` on turns further from 090, whatever it read.
        let mut t = t0;
        for level in 1..=3_u8 {
            t += STALL_MS;
            held_read(&mut picture, t, 17 * u32::from(level));
            let orders = pilot.decide(&picture, &doctrine, t);
            let expected = whole(f64::from(stopped) + HELD_TURN_DEG * f64::from(level + 1));
            assert_eq!(pilot.stall_level, level);
            assert_eq!(orders.heading, Some(expected), "level {level}");
            assert_eq!(orders.speed, Some(54));
            assert_eq!(
                pilot.take_notes(),
                [format!("Held at 0 kph: turning to {expected:03}°.")]
            );
            // Kept against the doctrine between reads.
            let orders = pilot.decide(&picture, &doctrine, t + 10);
            assert_eq!(orders.heading, Some(expected));
        }
        // The fourth: backing off, astern, for `HELD_REVERSE_MS`.
        t += STALL_MS;
        held_read(&mut picture, t, 270);
        let orders = pilot.decide(&picture, &doctrine, t);
        assert_eq!((orders.heading, orders.speed), (None, Some(-27)));
        assert_eq!(pilot.take_notes(), ["Held at 0 kph: backing off."]);
        assert_eq!(pilot.stall_level, 4);
        let orders = pilot.decide(&picture, &doctrine, t + HELD_REVERSE_MS - 1);
        assert_eq!(orders.speed, Some(-27));
        let orders = pilot.decide(&picture, &doctrine, t + HELD_REVERSE_MS);
        assert_eq!(pilot.stall_level, 0, "closed when the back-off ends");
        assert_eq!(orders.speed, Some(54), "forward again");
        assert_eq!(orders.heading, Some(90), "the doctrine again");
        // A `STATUS` at speed closes the stall; its heading is kept
        // `HELD_FOR_MS` more to clear the ground.
        let (mut picture, mut pilot) = flying_at(MID);
        let t0 = start(&mut picture, &mut pilot);
        let t = t0 + STALL_MS;
        held_read(&mut picture, t, 135);
        let turned = pilot.decide(&picture, &doctrine, t).heading;
        assert_eq!(
            turned,
            Some(whole(f64::from(stopped) + 2.0 * HELD_TURN_DEG))
        );
        let freed = t + 2_000;
        picture.own.held = None;
        picture.own.speed_kph = 54;
        picture.own.pos_read_ms = freed;
        pilot.decide(&picture, &doctrine, freed);
        assert_eq!(pilot.stall_level, 0);
        let orders = pilot.decide(&picture, &doctrine, freed + HELD_FOR_MS - 1);
        assert_eq!(orders.heading, turned, "still clearing the ground");
        let orders = pilot.decide(&picture, &doctrine, freed + HELD_FOR_MS);
        assert_ne!(orders.heading, turned, "the doctrine again");
        // A new stop is a new episode: its own held turn, the stall from 0.
        let (mut picture, mut pilot) = flying_at(MID);
        let t0 = start(&mut picture, &mut pilot);
        let t = t0 + STALL_MS;
        held_read(&mut picture, t, 135);
        pilot.decide(&picture, &doctrine, t);
        assert_eq!(pilot.stall_level, 1);
        picture.own.heading_deg = 200;
        held_event(&mut picture, t + 500);
        let orders = pilot.decide(&picture, &doctrine, t + 600);
        assert_eq!(pilot.stall_level, 0);
        assert_eq!(pilot.held_from_deg, Some(200));
        assert_eq!(orders.heading, Some(whole(200.0 + HELD_TURN_DEG)));
        // Freed, then held again with no notice read (a re-board, say): the
        // new stall turns from the heading it is held on now, not from the
        // last episode's stop.
        let (mut picture, mut pilot) = flying_at(MID);
        let t0 = start(&mut picture, &mut pilot);
        let freed = t0 + 1_000;
        picture.own.held = None;
        picture.own.speed_kph = 54;
        picture.own.pos_read_ms = freed;
        pilot.decide(&picture, &doctrine, freed);
        assert_eq!(pilot.held_from_deg, None, "the episode is over");
        let again = freed + 20_000;
        held_read(&mut picture, again, 220);
        pilot.decide(&picture, &doctrine, again);
        assert_eq!(pilot.held_from_deg, Some(220));
        held_read(&mut picture, again + STALL_MS, 220);
        let orders = pilot.decide(&picture, &doctrine, again + STALL_MS);
        assert_eq!(pilot.stall_level, 1);
        assert_eq!(orders.heading, Some(whole(220.0 + 2.0 * HELD_TURN_DEG)));
    }

    // -- cut AK.1: the fire clock --------------------------------------------------

    #[test]
    // The slack's floor is nothing (cut AL.1), which the lint reads as a
    // no-op; the rule stays written through its names.
    #[allow(clippy::unnecessary_min_or_max)]
    fn after_a_shot_the_weapon_waits_for_its_own_clock() {
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
        picture.lock = Some("BA".into());
        // An earlier row left the clock in the past (the smoke's case).
        picture.weapons = vec![Weapon {
            id: "cannon".into(),
            ready: false,
            rounds: 10,
            rounds_max: 40,
            read_ms: NOW - 5_000,
            cycle_ms: Some(6_000),
            next_ready_ms: Some(NOW - 1_000),
            slack_ms: crate::arena::picture::FIRE_SLACK_START_MS,
            ..Weapon::default()
        }];
        assert_eq!(decide(&picture, &mut pilot).fire, ["cannon"]);
        let a = NOW;
        picture.apply(a, &classify("Firing cannon."));
        let slack = picture.weapons[0].slack_ms;
        assert_eq!(
            slack,
            crate::arena::picture::FIRE_SLACK_START_MS
                .saturating_sub(crate::arena::picture::FIRE_SLACK_DOWN_MS)
                .max(crate::arena::picture::FIRE_SLACK_MIN_MS)
        );
        let gap = a + crate::arena::lane::FIRE_GAP_MS;
        assert!(!believed_ready(&picture.weapons[0], gap));
        assert!(pilot.decide(&picture, &doctrine(), gap).fire.is_empty());
        let due = a + 6_000 + slack;
        assert!(!believed_ready(&picture.weapons[0], due - 1));
        assert!(believed_ready(&picture.weapons[0], due));
        if let Some(c) = picture.contacts.get_mut("BA") {
            c.read_ms = due;
        }
        assert_eq!(pilot.decide(&picture, &doctrine(), due).fire, ["cannon"]);
    }

    // -- cut AL.1: a lost gun, an empty gun, every gun in doubt -----------------

    /// **`fireable` skips a struck, an empty and a doubted gun** (cut AL.1):
    /// a gun the tank said it does not carry, a gun whose rounds the acks have
    /// counted down to none, and every gun while a lost section's `its weapons
    /// are gone` waits for `WEAPONSTATUS`.
    #[test]
    fn fireable_skips_a_struck_an_empty_and_a_doubted_gun() {
        let armed = || {
            let (mut picture, _) = flying();
            hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
            picture.lock = Some("BA".into());
            picture.weapons = vec![weapon("cannon", true), weapon("machine_gun", true)];
            picture
        };
        let picture = armed();
        assert_eq!(
            fireable(&picture, "BA", Side::Blue, NOW),
            ["cannon", "machine_gun"]
        );
        // Struck: the no-such-weapon answer takes it out of the list.
        let mut picture = armed();
        picture.apply(
            NOW,
            &classify("That unit carries no weapon called `cannon`."),
        );
        assert_eq!(fireable(&picture, "BA", Side::Blue, NOW), ["machine_gun"]);
        // …and a gun the struck set names is never fired, whatever the list says.
        let mut picture = armed();
        picture.struck.insert("machine_gun".into());
        assert_eq!(fireable(&picture, "BA", Side::Blue, NOW), ["cannon"]);
        // Empty: every round the last row counted has been acked.
        let mut picture = armed();
        if let Some(mg) = picture.weapons.iter_mut().find(|w| w.id == "machine_gun") {
            mg.shots_since_read = mg.rounds;
        }
        assert_eq!(fireable(&picture, "BA", Side::Blue, NOW), ["cannon"]);
        // Doubted: nothing at all until `WEAPONSTATUS` answers.
        let mut picture = armed();
        picture.apply(
            NOW,
            &classify("The turret is destroyed: its weapons are gone."),
        );
        assert!(fireable(&picture, "BA", Side::Blue, NOW).is_empty());
        picture.apply(NOW + 100, &classify("Locked on BA, 62% to hit."));
        picture.apply(
            NOW + 100,
            &classify("machine_gun (mg)  ready  400 of 500 round(s)"),
        );
        picture.apply(NOW + 101, &classify("Power online."));
        assert_eq!(picture.weapons_doubt, None);
        assert_eq!(
            fireable(&picture, "BA", Side::Blue, NOW + 101),
            ["machine_gun"]
        );
    }

    // -- cut AL.2: nose-on ------------------------------------------------------

    /// One decision of `pilot` on `doctrine` at `at`, with BA at `bearing` and
    /// `range` from the origin, its orders not held back by an earlier one:
    /// `(heading, speed)`.
    fn fly_on(
        picture: &mut Picture,
        pilot: &mut Pilot,
        doctrine: &Doctrine,
        bearing: u32,
        range: u32,
        at: u64,
    ) -> (Option<u32>, Option<i32>) {
        let mut ba = contact("BA", Some(Side::Blue), bearing, range);
        ba.read_ms = at;
        hold(picture, ba);
        picture.own.pos_read_ms = at;
        pilot.forget_heading();
        pilot.forget_speed();
        let orders = pilot.decide(picture, doctrine, at);
        (orders.heading, orders.speed)
    }

    /// **The default engagement holds bow on at the band** (cut AL.2): for
    /// bearings round the clock, a fresh pilot on the default doctrine is
    /// stopped inside the slack and the hysteresis, closes full ahead beyond
    /// them until back at the slack, opens in reverse short of them until out
    /// at the slack, never reverses for a machine at the help's 420 m or one
    /// read at exactly 400 m, and orders its heading on the hostile throughout.
    #[test]
    fn the_default_engagement_holds_bow_on_at_the_band() {
        let doctrine = hold_doctrine();
        assert_eq!(doctrine.stance, Stance::Hold);
        let band = f64::from(doctrine.band_m);
        let at = |m: f64| m.round() as u32;
        let slack = HOLD_SLACK_M;
        let leave = HOLD_SLACK_M + HOLD_HYSTERESIS_M;
        // Due north the plot lies at exactly its range, so each threshold is
        // pinned on its own metre there; round the clock (where a bearing's
        // sine leaves a hair of float) a metre inside it.
        for bearing in (0..360).step_by(30) {
            let edge = if bearing == 0 { 0.0 } else { 1.0 };
            let check = |(heading, _): (Option<u32>, Option<i32>)| {
                let h = heading.expect("a heading");
                assert!(
                    angle_between(h, bearing) <= HOLD_BOW_DEG,
                    "{h} for {bearing}"
                );
            };
            // A fresh pilot inside the slack and the hysteresis: stopped.
            for range in [
                band - leave + edge,
                band - slack,
                band,
                band + slack,
                band + leave - edge,
            ] {
                let (mut picture, mut pilot) = flying();
                let flown = fly_on(&mut picture, &mut pilot, &doctrine, bearing, at(range), NOW);
                check(flown);
                assert_eq!(flown.1, Some(0), "{range} at {bearing}");
                assert_eq!(pilot.hold_motion, HoldMotion::Stopped);
            }
            // Beyond them: full ahead, kept until back at the slack.
            let (mut picture, mut pilot) = flying();
            let mut t = NOW;
            for (range, kph) in [
                (band + leave + 1.0, 54),
                (band + slack + 1.0, 54),
                (band + slack - edge, 0),
                (band + slack + 1.0, 0),
                (band + leave - edge, 0),
            ] {
                let flown = fly_on(&mut picture, &mut pilot, &doctrine, bearing, at(range), t);
                check(flown);
                assert_eq!(flown.1, Some(kph), "closing: {range} at {bearing}");
                t += 1_000;
            }
            // Short of them: in reverse, kept until out at the slack.
            let (mut picture, mut pilot) = flying();
            let mut t = NOW;
            for (range, kph) in [
                (band - leave - 1.0, -27),
                (band - slack - 1.0, -27),
                (band - slack + edge, 0),
                (band - slack - 1.0, 0),
                (band - leave + edge, 0),
            ] {
                let flown = fly_on(&mut picture, &mut pilot, &doctrine, bearing, at(range), t);
                check(flown);
                assert_eq!(flown.1, Some(kph), "opening: {range} at {bearing}");
                t += 1_000;
            }
            // The machines' 420 m, and a row read at exactly 400 m: never a
            // reverse, fresh or arriving from a close.
            for range in [420, 400] {
                let (mut picture, mut pilot) = flying();
                assert_eq!(
                    fly_on(&mut picture, &mut pilot, &doctrine, bearing, range, NOW).1,
                    Some(0)
                );
                let (mut picture, mut pilot) = flying();
                fly_on(&mut picture, &mut pilot, &doctrine, bearing, 700, NOW);
                assert_eq!(pilot.hold_motion, HoldMotion::Ahead);
                let flown = fly_on(&mut picture, &mut pilot, &doctrine, bearing, range, NOW + 1);
                check(flown);
                assert_eq!(flown.1, Some(0), "{range} after a close");
                let flown = fly_on(&mut picture, &mut pilot, &doctrine, bearing, range, NOW + 2);
                assert_eq!(flown.1, Some(0), "{range} held");
            }
        }
        // A tank whose astern ceiling is unknown reverses at the slow default.
        let (mut picture, mut pilot) = flying();
        picture.own.max_astern_kph = None;
        let flown = fly_on(&mut picture, &mut pilot, &doctrine, 0, 100, NOW);
        assert_eq!(flown, (Some(0), Some(-UNKNOWN_ASTERN_KPH)));
    }

    /// **The orbit is a stance, no longer the default** (cut AL.2): the default
    /// doctrine holds; with `[arena] engagement = "orbit"` it is the orbit,
    /// flown as cut AK built it.
    #[test]
    fn orbit_is_a_stance_and_no_longer_the_default() {
        assert_eq!(
            Doctrine::pilot_default(&Settings::default()).stance,
            Stance::Hold
        );
        let orbit = Doctrine::pilot_default(&Settings {
            engagement: Engagement::Orbit,
            ..Settings::default()
        });
        assert_eq!(orbit.stance, Stance::Orbit);
        assert_eq!(orbit, doctrine(), "the orbit the older tests fly");
        // At the band, BA due north: the hold stops facing it; the orbit takes
        // its tangent (the pilot's running sense, counter-clockwise) full ahead.
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
        let held = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!((held.heading, held.speed), (Some(0), Some(0)));
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
        let circled = pilot.decide(&picture, &orbit, NOW);
        assert_eq!(
            (circled.heading, circled.speed),
            (Some(orbit_heading(0.0, 450.0, orbit.band_m, 1.0)), Some(54))
        );
        assert_eq!(pilot.hold_motion, HoldMotion::Stopped);
    }

    /// **`close` and `hunt` end in a hold** (cut AL.2): `close` drives in bow
    /// on and holds at [`CLOSE_M`] (reversing only well inside it); `hunt`
    /// drives at a fresh hostile and holds at the band.
    #[test]
    fn close_and_hunt_end_in_a_hold() {
        let close = Doctrine {
            stance: Stance::Close,
            ..hold_doctrine()
        };
        let (mut picture, mut pilot) = flying();
        let slack = HOLD_SLACK_M;
        let leave = HOLD_SLACK_M + HOLD_HYSTERESIS_M;
        let mut t = NOW;
        for (range, kph) in [
            (600.0, 54),
            (CLOSE_M + slack + 1.0, 54),
            (CLOSE_M + slack, 0),
            (CLOSE_M, 0),
            (CLOSE_M - leave, 0),
            (CLOSE_M - leave - 1.0, -27),
        ] {
            let flown = fly_on(&mut picture, &mut pilot, &close, 0, range as u32, t);
            assert_eq!(flown, (Some(0), Some(kph)), "close at {range}");
            t += 1_000;
        }
        let hunt = Doctrine {
            stance: Stance::Hunt,
            ..hold_doctrine()
        };
        let band = f64::from(hunt.band_m);
        let (mut picture, mut pilot) = flying();
        let mut t = NOW;
        for (range, kph) in [
            (900.0, 54),
            (band + slack + 1.0, 54),
            (band + slack, 0),
            (band, 0),
        ] {
            let flown = fly_on(&mut picture, &mut pilot, &hunt, 180, range as u32, t);
            assert_eq!(flown, (Some(180), Some(kph)), "hunt at {range}");
            t += 1_000;
        }
    }

    /// **The fall back and a kept withdraw reverse bow on** (cut AL.2): with a
    /// target, the heading on it and a negative speed; the fall back with no
    /// target to face keeps its heading at full ahead.
    #[test]
    fn the_fall_back_and_a_kept_withdraw_reverse_bow_on() {
        for bearing in (0..360).step_by(45) {
            // The pilot's own fall back, on half the hull.
            let (mut picture, mut pilot) = flying();
            half_hull(&mut picture);
            hold(&mut picture, contact("BA", Some(Side::Blue), bearing, 250));
            let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
            assert!(pilot.withdraw_until_ms.is_some());
            assert_eq!((orders.heading, orders.speed), (Some(bearing), Some(-27)));
            // A kept doctrine withdraw.
            let (mut picture, mut pilot) = flying();
            hold(&mut picture, contact("BA", Some(Side::Blue), bearing, 450));
            let withdraw = Doctrine {
                stance: Stance::Withdraw,
                hold_s: Some(20),
                ..hold_doctrine()
            };
            let orders = pilot.decide(&picture, &withdraw, NOW);
            assert_eq!((orders.heading, orders.speed), (Some(bearing), Some(-27)));
        }
        // Nothing to face: the fall back keeps the heading, full ahead.
        let (mut picture, mut pilot) = flying();
        half_hull(&mut picture);
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert!(pilot.withdraw_until_ms.is_some());
        assert_eq!((orders.heading, orders.speed), (None, Some(54)));
    }

    /// A contact at `bearing`/`range` from the origin whose own row reads
    /// `rel` relative to the hull.
    fn contact_rel(designator: &str, bearing: u32, range: u32, rel: u32) -> Contact {
        Contact {
            rel_deg: rel,
            ..contact(designator, Some(Side::Blue), bearing, range)
        }
    }

    fn hit(picture: &mut Picture, at: u64, line: &str) {
        picture.apply(at, &classify(line));
    }

    const LEFT_HIT: &str = "Hit on the hull's left facet: 200 of 250 armour left.";

    /// **A side or rear hit turns the bow to the shooter** (cut AL.2).
    #[test]
    fn a_side_or_rear_hit_turns_the_bow_to_the_shooter() {
        // A hostile in the struck arc by its own row's relative bearing: the
        // `STATUS` heading (090, seconds old) would put BB astern, but its row
        // reads it at 270, on the left. BA is the held target, due north.
        let (mut picture, mut pilot) = flying();
        picture.own.heading_deg = 90;
        hold(&mut picture, contact_rel("BA", 0, 450, 0));
        hold(&mut picture, contact_rel("BB", 270, 500, 270));
        picture.lock = Some("BA".into());
        hit(&mut picture, NOW, LEFT_HIT);
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!((orders.heading, orders.speed), (Some(270), Some(0)));
        assert_eq!(
            pilot.take_notes(),
            ["Turning into the shot: hit on the hull's left facet; facing BB."]
        );
        let (bearing, until) = pilot.threat.expect("a threat");
        assert_eq!((whole(bearing), until), (270, NOW + THREAT_TURN_MS));
        // A newer hit on the same threat does not re-arm it; one on a threat
        // more than `HOLD_BOW_DEG` away does (BC on the right, by its row).
        hit(&mut picture, NOW + 500, LEFT_HIT);
        pilot.decide(&picture, &hold_doctrine(), NOW + 500);
        assert_eq!(pilot.threat, Some((bearing, until)), "the same threat");
        assert!(pilot.take_notes().is_empty());
        hold(&mut picture, contact_rel("BC", 90, 600, 90));
        hit(
            &mut picture,
            NOW + 1_000,
            "Hit on the turret's right facet: 200 of 220 armour left.",
        );
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW + 1_000);
        assert_eq!(
            pilot.threat.map(|(b, u)| (whole(b), u)),
            Some((90, NOW + 1_000 + THREAT_TURN_MS))
        );
        assert_eq!(orders.heading, Some(90));
        assert_eq!(
            pilot.take_notes(),
            ["Turning into the shot: hit on the turret's right facet; facing BC."]
        );
        // The turn lasts `THREAT_TURN_MS`; then the hold faces BA again.
        let end = NOW + 1_000 + THREAT_TURN_MS;
        let mut later = picture.clone();
        for c in later.contacts.values_mut() {
            c.read_ms = end;
        }
        later.own.pos_read_ms = end;
        pilot.decide(&later, &hold_doctrine(), end - 1);
        assert!(pilot.threat.is_some());
        let orders = pilot.decide(&later, &hold_doctrine(), end);
        assert_eq!(pilot.threat, None);
        assert_eq!((orders.heading, orders.speed), (Some(0), Some(0)));

        // None in the arc and none held: the arc's centre from the reference
        // heading — the `STATUS` heading with no order since…
        let (mut picture, mut pilot) = flying();
        picture.own.heading_deg = 40;
        hit(
            &mut picture,
            NOW,
            "Hit on the hull's rear facet: 100 of 150 armour left.",
        );
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!((orders.heading, orders.speed), (Some(220), Some(0)));
        assert_eq!(
            pilot.take_notes(),
            ["Turning into the shot: hit on the hull's rear facet; facing 220°."]
        );
        // …or the heading ordered after it.
        let (mut picture, mut pilot) = flying();
        picture.own.heading_deg = 40;
        pilot.sent_heading(100, NOW + 1);
        hit(&mut picture, NOW + 2, LEFT_HIT);
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW + 2);
        assert_eq!(orders.heading, Some(10));
        assert_eq!(
            pilot.take_notes(),
            ["Turning into the shot: hit on the hull's left facet; facing 010°."]
        );

        // A young target held and no other hostile in the arc: the target is
        // the shooter, and the hold already faces it — whether it is in the
        // arc itself or not.
        for (bearing, rel) in [(0, 0), (270, 270)] {
            let (mut picture, mut pilot) = flying();
            hold(&mut picture, contact_rel("BA", bearing, 450, rel));
            hit(&mut picture, NOW, LEFT_HIT);
            let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
            assert_eq!(pilot.threat, None);
            assert_eq!((orders.heading, orders.speed), (Some(bearing), Some(0)));
            assert!(pilot.take_notes().is_empty());
        }
        // A wreck, an unidentified contact and a notice-only contact in the
        // arc are no shooter.
        let (mut picture, mut pilot) = flying();
        let mut x1 = contact_rel("X1", 270, 300, 270);
        x1.side = None;
        hold(&mut picture, x1);
        // (BD, known only from its notice, has no row: no range, and its plot
        // too old to be held, so it is neither the shooter nor a young target.)
        let mut bd = contact_rel("BD", 260, 300, 260);
        bd.range_m = None;
        bd.age_ms = HUNT_STALE_PLOT_MS + 1;
        hold(&mut picture, bd);
        hold(&mut picture, contact_rel("BE", 280, 300, 280));
        pilot.wrecks.insert("BE".into());
        hit(&mut picture, NOW, LEFT_HIT);
        pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!(
            pilot.take_notes(),
            ["Turning into the shot: hit on the hull's left facet; facing 270°."]
        );

        // A front or top hit does nothing.
        for line in [
            "Hit on the hull's front facet: 300 of 400 armour left.",
            "Hit on the turret's top facet: 80 of 100 armour left.",
        ] {
            let (mut picture, mut pilot) = flying();
            hit(&mut picture, NOW, line);
            pilot.decide(&picture, &hold_doctrine(), NOW);
            assert_eq!(pilot.threat, None, "{line}");
            assert_eq!(pilot.threat_seen_ms, NOW, "read once");
        }
        // An orbit doctrine does nothing.
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact_rel("BB", 270, 500, 270));
        hit(&mut picture, NOW, LEFT_HIT);
        pilot.decide(&picture, &doctrine(), NOW);
        assert_eq!(pilot.threat, None);
        assert!(pilot.take_notes().is_empty());
    }

    /// The compass direction of the turn from `from` to `to`: `+1` clockwise,
    /// `-1` counter-clockwise, `0` none.
    fn turn_way(from: u32, to: u32) -> i64 {
        signed_turn(from, to).signum()
    }

    /// `flying()` with `track` destroyed.
    fn one_track(track: &str) -> (Picture, Pilot) {
        let (mut picture, pilot) = flying();
        lose_track(&mut picture, track);
        (picture, pilot)
    }

    fn lose_track(picture: &mut Picture, track: &str) {
        picture.own.sections.insert(
            track.into(),
            Section {
                condition: "destroyed".into(),
                internals: Some((0, 24)),
                facets: Default::default(),
            },
        );
    }

    /// **A one-track tank turns only the live way** (cut AL.2), asserted on
    /// the compass direction of each ordered heading from the reference
    /// heading, never on the orbit's sense.
    #[test]
    fn a_one_track_tank_turns_only_the_live_way() {
        assert_eq!(live_turn(&flying().0), None);
        let (mut both, _) = one_track("left_track");
        lose_track(&mut both, "right_track");
        assert_eq!(live_turn(&both), None, "both gone: no live turn");
        for (track, live) in [
            ("left_track", Turn::Clockwise),
            ("right_track", Turn::CounterClockwise),
        ] {
            let way = live.compass_sign() as i64;
            let words = if way > 0 {
                "clockwise"
            } else {
                "counter-clockwise"
            };
            let (picture, _) = one_track(track);
            assert_eq!(live_turn(&picture), Some(live));

            // The held turn, the pilot's sense set against the live way.
            let (mut picture, mut pilot) = one_track(track);
            pilot.sense = -(way as i8);
            picture.own.heading_deg = 100;
            held_event(&mut picture, NOW - 10);
            let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
            let h = orders.heading.expect("the held turn");
            assert_eq!(turn_way(100, h), way, "{track}: the held turn {h}");
            assert_eq!(angle_between(100, h), HELD_TURN_DEG as u32);
            // The sentence, once a match.
            let notes = pilot.take_notes();
            assert_eq!(
                notes.first().map(String::as_str),
                Some(format!("The {track} is gone: turning {words} only.").as_str())
            );
            pilot.decide(&picture, &hold_doctrine(), NOW + 10);
            assert!(
                !pilot.take_notes().iter().any(|n| n.contains(" is gone: ")),
                "once"
            );

            // A stall's level: 90° from the stop, the live way.
            let t0 = NOW + HELD_FOR_MS;
            held_read(&mut picture, t0, 145);
            pilot.decide(&picture, &hold_doctrine(), t0);
            assert!(pilot.stall_since_ms.is_some(), "{track}: a stall opens");
            let t1 = t0 + STALL_MS;
            held_read(&mut picture, t1, 145);
            let orders = pilot.decide(&picture, &hold_doctrine(), t1);
            assert_eq!(pilot.stall_level, 1);
            let h = orders.heading.expect("the stall's turn");
            assert_eq!(turn_way(100, h), way, "{track}: the stall {h}");
            assert_eq!(angle_between(100, h), 2 * HELD_TURN_DEG as u32);

            // The sweep's next leg, a quarter turn from the course the live
            // way, the pilot's sense set against it.
            let track_of = Track {
                course_deg: 90.0,
                speed_mps: 8.0,
            };
            let (mut picture, mut pilot, _) = lost(MID, 0, 450, Some(track_of));
            lose_track(&mut picture, track);
            pilot.sense = -(way as i8);
            to_the_sweep(&mut picture, &mut pilot);
            let HuntLeg::Sweep { heading_deg, .. } = hunt_of(&pilot).leg else {
                panic!("sweeping");
            };
            assert_eq!(turn_way(90, whole(heading_deg)), way, "{track}: the sweep");
            assert_eq!(angle_between(90, whole(heading_deg)), 90);

            // A detour round held ground: the search on 090 meets a mark
            // ahead (half way between the clear distance and the reach); it
            // turns aside the live way.
            let (mut picture, mut pilot) = one_track(track);
            picture.own.heading_deg = 90;
            picture.spots.insert(
                'B',
                crate::arena::picture::Spot {
                    name: "The Crossroads".into(),
                    range_m: None,
                    bearing_deg: Some(90),
                    read_ms: NOW,
                    plotted_m: None,
                },
            );
            let ahead = mark_ahead_m();
            pilot.held_marks.push(HeldMark {
                at_m: (ahead - HELD_MARK_AHEAD_M, 0.0),
                heading_deg: 90,
                astern: false,
                blocked_m: (ahead, 0.0),
            });
            let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
            let h = orders.heading.expect("the detour");
            assert_eq!(turn_way(90, h), way, "{track}: the detour {h}");
            assert_eq!(f64::from(angle_between(90, h)), HELD_MARK_DETOUR_DEG);

            // An orbit with `Sense::Auto` and no track, across a timed flip:
            // it circles the way that turns the live way. The tank drives
            // 100 m along the first heading the orbit orders; seen from there
            // the orbit's next heading has turned the live way.
            // (The tank already lies on the orbit's tangent: from a bow on BA
            // the tangent is a quarter turn, the dead way for one of the two
            // tracks, which a deadband tuned to 90° would not order.)
            let (mut picture, mut pilot) = one_track(track);
            let ba_at = (0.0, 450.0);
            hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
            let circling = match live {
                Turn::CounterClockwise => 1.0,
                Turn::Clockwise => -1.0,
            };
            picture.own.heading_deg = orbit_heading(0.0, 450.0, doctrine().band_m, circling);
            let first = heading(&pilot.decide(&picture, &doctrine(), NOW));
            assert_eq!(first, picture.own.heading_deg, "{track}: on the tangent");
            pilot.sent_heading(first, NOW);
            let t = NOW + FLIP_EVERY_MS;
            let own = along((0.0, 0.0), f64::from(first), 100.0);
            let mut ba = contact_from(
                own,
                "BA",
                Some(Side::Blue),
                whole(bearing_deg(own, ba_at)),
                distance_m(own, ba_at).round() as u32,
            );
            ba.read_ms = t;
            hold(&mut picture, ba);
            read_at(&mut picture, own, t);
            picture.own.heading_deg = first;
            let second = heading(&pilot.decide(&picture, &doctrine(), t));
            assert_eq!(
                turn_way(first, second),
                way,
                "{track}: {first} then {second}"
            );

            // The deadband: a small dead-side correction, and one of exactly
            // `ONE_TRACK_DEADBAND_DEG`, are not ordered; a live-side one is.
            let dead_by =
                |deg: u32| -> u32 { ((100 - way * i64::from(deg)).rem_euclid(360)) as u32 };
            let (mut picture, mut pilot) = one_track(track);
            picture.own.heading_deg = 100;
            for off in [10, ONE_TRACK_DEADBAND_DEG] {
                let flown = fly_on(
                    &mut picture,
                    &mut pilot,
                    &hold_doctrine(),
                    dead_by(off),
                    450,
                    NOW,
                );
                assert_eq!(flown, (None, Some(0)), "{track}: {off}° dead side");
            }
            let live_side = ((100 + way * 40).rem_euclid(360)) as u32;
            let flown = fly_on(
                &mut picture,
                &mut pilot,
                &hold_doctrine(),
                live_side,
                450,
                NOW,
            );
            assert_eq!(flown.0, Some(live_side), "{track}: live side at once");
            // A large one is ordered once, then nothing until `STATUS` reads
            // it…
            let (mut picture, mut pilot) = one_track(track);
            picture.own.heading_deg = 100;
            let far = dead_by(ONE_TRACK_DEADBAND_DEG + 10);
            let flown = fly_on(&mut picture, &mut pilot, &hold_doctrine(), far, 450, NOW);
            assert_eq!(flown.0, Some(far));
            pilot.sent_heading(far, NOW);
            assert_eq!(pilot.turn_commit, Some((far, NOW)));
            let elsewhere = ((100 + way * 60).rem_euclid(360)) as u32;
            let flown = fly_on(
                &mut picture,
                &mut pilot,
                &hold_doctrine(),
                elsewhere,
                450,
                NOW + 500,
            );
            assert_eq!(flown.0, None, "{track}: committed");
            // (a `STATUS` short of it does not end the commit)
            picture.own.heading_deg = (far + 20) % 360;
            let flown = fly_on(
                &mut picture,
                &mut pilot,
                &hold_doctrine(),
                elsewhere,
                450,
                NOW + 1_000,
            );
            assert_eq!(flown.0, None, "{track}: not read yet");
            picture.own.heading_deg = ((i64::from(far) - 3 * way).rem_euclid(360)) as u32;
            let flown = fly_on(
                &mut picture,
                &mut pilot,
                &hold_doctrine(),
                far,
                450,
                NOW + 1_500,
            );
            assert_eq!(pilot.turn_commit, None, "{track}: read");
            assert_eq!(flown.0, Some(far));
            // …or the commit lapses.
            let (mut picture, mut pilot) = one_track(track);
            picture.own.heading_deg = 100;
            fly_on(&mut picture, &mut pilot, &hold_doctrine(), far, 450, NOW);
            pilot.sent_heading(far, NOW);
            let lapse = NOW + ONE_TRACK_COMMIT_MS;
            let flown = fly_on(
                &mut picture,
                &mut pilot,
                &hold_doctrine(),
                far,
                450,
                lapse - 1,
            );
            assert_eq!(flown.0, None);
            let flown = fly_on(&mut picture, &mut pilot, &hold_doctrine(), far, 450, lapse);
            assert_eq!(flown.0, Some(far), "{track}: the commit lapsed");
            // The re-board clears it.
            let (mut picture, mut pilot) = one_track(track);
            picture.own.heading_deg = 100;
            fly_on(&mut picture, &mut pilot, &hold_doctrine(), far, 450, NOW);
            pilot.sent_heading(far, NOW);
            pilot.forget_turns(&picture);
            assert_eq!(pilot.turn_commit, None);
            let flown = fly_on(
                &mut picture,
                &mut pilot,
                &hold_doctrine(),
                far,
                450,
                NOW + 500,
            );
            assert_eq!(flown.0, Some(far), "{track}: re-boarded");
        }
    }

    /// How far ahead a test puts a mark that must block: half way between the
    /// clear distance and the reach, so it blocks wherever a tuning round
    /// moves either within its bounds.
    fn mark_ahead_m() -> f64 {
        (HELD_MARK_CLEAR_M + HELD_MARK_NEAR_M) / 2.0
    }

    /// The tank read at `pos` on `heading` at `at`, free and stopped.
    fn status_at(picture: &mut Picture, pos: (f64, f64), heading: u32, at: u64) {
        picture.own.pos_m = Some(pos);
        picture.own.heading_deg = heading;
        picture.own.pos_read_ms = at;
        picture.own.held = None;
        picture.own.speed_kph = 0;
    }

    /// **A held stop marks its ground and the search steers round it** (cut
    /// AL.2): the bot's own run-2 loop, and the mark's geometry.
    #[test]
    fn a_held_stop_marks_its_ground_and_the_search_steers_round_it() {
        // Stopped at (1 420, 1 049) on 000 by the slope.
        let stop = (1_420.0, 1_049.0);
        let (mut picture, mut pilot) = flying_at(stop);
        picture.events.push_back(Event {
            at_ms: NOW - 10,
            kind: EventKind::Held,
            text: "Held: the slope ahead is too steep for the tank.".into(),
        });
        pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!(
            pilot.held_marks,
            [HeldMark {
                at_m: stop,
                heading_deg: 0,
                astern: false,
                blocked_m: (1_420.0, 1_049.0 + HELD_MARK_AHEAD_M),
            }]
        );
        assert!(pilot.take_notes().contains(
            &"Held ground marked at 1420 m east, 1049 m north, heading 000°.".to_owned()
        ));
        // Later, searching from (1 330, 1 052) for B on 091: blocked, and the
        // search's heading is turned aside.
        let from = (1_330.0, 1_052.0);
        assert!(pilot.blocked(from, 91.0));
        picture.spots.insert(
            'B',
            crate::arena::picture::Spot {
                name: "The Crossroads".into(),
                range_m: None,
                bearing_deg: Some(91),
                read_ms: NOW,
                plotted_m: None,
            },
        );
        let t = NOW + 20_000;
        status_at(&mut picture, from, 91, t);
        let orders = pilot.decide(&picture, &hold_doctrine(), t);
        let aside = whole(91.0 + HELD_MARK_DETOUR_DEG * pilot.turn_sign(&picture));
        assert!(orders.ask_spot_b, "searching");
        assert_eq!(orders.heading, Some(aside));
        assert!(!pilot.blocked(from, f64::from(aside)));
        assert_eq!(
            pilot.take_notes(),
            [format!("Steering round held ground: {aside:03}° for 091°.")]
        );
        assert!(pilot.detour);
        // Said once while the detour lasts.
        pilot.decide(&picture, &hold_doctrine(), t + 50);
        assert!(pilot.take_notes().is_empty());
        // A heading clear of the mark is untouched.
        assert!(!pilot.blocked(from, 270.0));
        assert!(!pilot.blocked(from, 0.0));
        assert_eq!(pilot.steer_round(from, 270.0, 1.0), 270.0);
        assert!(!pilot.detour);
        // A mark beyond `HELD_MARK_NEAR_M` is ignored.
        let far = (stop.0 - HELD_MARK_NEAR_M - 20.0, 1_069.0);
        assert!(!pilot.blocked(far, 90.0));
        // From the stop point itself every heading is free (the clear
        // distance), and from anywhere within it of the mark.
        for h in (0..360).step_by(5) {
            assert!(!pilot.blocked(stop, f64::from(h)), "{h}");
            assert!(!pilot.blocked((1_420.0, 1_040.0), f64::from(h)), "{h}");
        }
        // Three blocked detours leave the heading as it was.
        let mut boxed = pilot.clone();
        for k in 1..=3_u8 {
            let detour = 91.0 + f64::from(k) * HELD_MARK_DETOUR_DEG;
            boxed.held_marks.push(HeldMark {
                at_m: from,
                heading_deg: 0,
                astern: false,
                blocked_m: along(from, detour, mark_ahead_m()),
            });
        }
        assert_eq!(boxed.steer_round(from, 91.0, 1.0), 91.0);
        // A stopped or reversing tank's heading is untouched: a hold at the
        // band on 091, and a hold reversing on 091 (its ray behind it clear),
        // with only the run-2 mark — which a forward heading on 091 would be
        // steered round (fixes after review: the boxed marks left 091 either
        // way).
        assert_ne!(
            pilot.clone().steer_round(from, 91.0, 1.0),
            91.0,
            "steered, were it ahead"
        );
        let (mut picture, mut pilot) = flying_at(from);
        pilot.held_marks = pilot_marks(stop);
        let ba = contact_from(from, "BA", Some(Side::Blue), 91, 450);
        hold(&mut picture, ba);
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!((orders.heading, orders.speed), (Some(91), Some(0)));
        let (mut picture, mut pilot) = flying_at(from);
        pilot.held_marks = pilot_marks(stop);
        hold(
            &mut picture,
            contact_from(from, "BA", Some(Side::Blue), 91, 200),
        );
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!((orders.heading, orders.speed), (Some(91), Some(-27)));
        // The same ahead: steered round.
        let (mut picture, mut pilot) = flying_at(from);
        pilot.held_marks = pilot_marks(stop);
        hold(
            &mut picture,
            contact_from(from, "BA", Some(Side::Blue), 91, 900),
        );
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!(orders.speed, Some(54));
        assert_ne!(orders.heading, Some(91), "closing: steered round");
        // `Held: something is in the way.` (a unit) and the unstated hold mark
        // nothing.
        for text in ["Held: something is in the way.", "Held: the tank is held."] {
            let (mut picture, mut pilot) = flying_at(stop);
            picture.events.push_back(Event {
                at_ms: NOW - 10,
                kind: EventKind::Held,
                text: text.into(),
            });
            pilot.decide(&picture, &hold_doctrine(), NOW);
            assert!(pilot.held_marks.is_empty(), "{text}");
            assert!(pilot.held_heading.is_some(), "{text}: still turned");
        }
        // The edge is ground; at most `HELD_MARKS_MAX` are kept, the oldest
        // dropped.
        let (mut picture, mut pilot) = flying_at(stop);
        for i in 0..=HELD_MARKS_MAX as u64 {
            picture.events.push_back(Event {
                at_ms: NOW + i,
                kind: EventKind::Held,
                text: "Held: this is the edge of the arena.".into(),
            });
            picture.own.pos_m = Some((stop.0 + i as f64, stop.1));
            pilot.decide(&picture, &hold_doctrine(), NOW + i + 1);
        }
        assert_eq!(pilot.held_marks.len(), HELD_MARKS_MAX);
        assert_eq!(pilot.held_marks[0].at_m, (stop.0 + 1.0, stop.1));
    }

    /// The run-2 stop's one mark.
    fn pilot_marks(stop: (f64, f64)) -> Vec<HeldMark> {
        vec![HeldMark {
            at_m: stop,
            heading_deg: 0,
            astern: false,
            blocked_m: along(stop, 0.0, HELD_MARK_AHEAD_M),
        }]
    }

    /// **A stop while reversing marks behind and stops the reverse** (cut
    /// AL.2; a guard — no transcript holds one): a fall back backing into a
    /// steep bank runs no full-ahead held turn, marks the ground behind, turns
    /// the next reverse into a stop with the bow kept, and opens no stall.
    #[test]
    fn a_stop_while_reversing_marks_behind_and_stops_the_reverse() {
        let (mut picture, mut pilot) = flying_at(MID);
        half_hull(&mut picture);
        hold(
            &mut picture,
            contact_from(MID, "BA", Some(Side::Blue), 0, 250),
        );
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!((orders.heading, orders.speed), (Some(0), Some(-27)));
        pilot.sent_heading(0, NOW);
        pilot.sent_speed(-27);
        pilot.take_notes();
        // Held while reversing.
        picture.events.push_back(Event {
            at_ms: NOW + 500,
            kind: EventKind::Held,
            text: "Held: the slope ahead is too steep for the tank.".into(),
        });
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW + 510);
        assert_eq!(
            (orders.heading, orders.speed),
            (None, Some(0)),
            "no full-ahead held turn: stopped, the bow kept"
        );
        assert!(pilot.held_astern);
        assert_eq!(pilot.held_heading, None);
        assert_eq!(
            pilot.held_marks,
            [HeldMark {
                at_m: MID,
                heading_deg: 0,
                astern: true,
                blocked_m: along(MID, 180.0, HELD_MARK_AHEAD_M),
            }]
        );
        assert_eq!(
            pilot.take_notes(),
            ["Held ground marked at 1280 m east, 1000 m north, heading 000°."]
        );
        assert!(pilot.blocked_behind(MID, 0.0), "the ray behind meets it");
        assert!(
            !pilot.blocked(MID, 180.0),
            "a forward ray keeps its clear distance"
        );
        // After the held stop the fall back still runs: its reverse is a stop
        // with the bow kept.
        let t = NOW + 510 + HELD_FOR_MS;
        held_read(&mut picture, t, 0);
        let orders = pilot.decide(&picture, &hold_doctrine(), t);
        assert!(pilot.withdraw_until_ms.is_some());
        assert_eq!(orders.speed, Some(0));
        assert!(
            orders
                .heading
                .is_none_or(|h| angle_between(h, 0) <= HOLD_BOW_DEG),
            "the bow kept: {orders:?}"
        );
        // Read held at 0 kph a whole stall later: no stall, the pilot chose
        // the stop.
        let t = t + STALL_MS;
        held_read(&mut picture, t, 0);
        pilot.decide(&picture, &hold_doctrine(), t);
        assert_eq!(pilot.stall_since_ms, None);
        assert_eq!(pilot.stall_level, 0);
        // A hold that would open the range here stops instead, bow on.
        let (mut picture, mut pilot) = flying_at(MID);
        pilot.held_marks = vec![HeldMark {
            at_m: MID,
            heading_deg: 0,
            astern: true,
            blocked_m: along(MID, 180.0, HELD_MARK_AHEAD_M),
        }];
        hold(
            &mut picture,
            contact_from(MID, "BA", Some(Side::Blue), 0, 200),
        );
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!((orders.heading, orders.speed), (Some(0), Some(0)));
        assert_eq!(
            pilot.hold_motion,
            HoldMotion::Astern,
            "still wanting to open"
        );
        // Facing the other way, the mark is ahead, and the reverse goes.
        let (mut picture, mut pilot) = flying_at(MID);
        pilot.held_marks = vec![HeldMark {
            at_m: MID,
            heading_deg: 0,
            astern: true,
            blocked_m: along(MID, 180.0, HELD_MARK_AHEAD_M),
        }];
        hold(
            &mut picture,
            contact_from(MID, "BA", Some(Side::Blue), 180, 200),
        );
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!((orders.heading, orders.speed), (Some(180), Some(-27)));
    }

    /// **The back-off stops short of held ground behind** (fixes after
    /// review): step 1's back-off is checked against the ray behind the tank
    /// like every other reverse — a mark taken reversing is met however near,
    /// one taken driving ahead only past the clear distance (so the back-off
    /// from a forward stop is not stopped by that stop's own mark), and a mark
    /// off the ray stops nothing.
    #[test]
    fn the_back_off_stops_short_of_held_ground_behind() {
        let mark = |bearing: f64, metres: f64, astern: bool| HeldMark {
            at_m: MID,
            heading_deg: 0,
            astern,
            blocked_m: along(MID, bearing, metres),
        };
        for (marks, kph, why) in [
            (vec![], -27, "nothing marked"),
            (
                vec![mark(180.0, HELD_MARK_AHEAD_M, true)],
                0,
                "a mark taken reversing, just behind",
            ),
            (
                vec![mark(180.0, mark_ahead_m(), false)],
                0,
                "a mark taken ahead, past the clear distance behind",
            ),
            (
                vec![mark(180.0, HELD_MARK_AHEAD_M, false)],
                -27,
                "a mark taken ahead, inside the clear distance",
            ),
            (
                vec![mark(90.0, mark_ahead_m(), true)],
                -27,
                "a mark off the ray behind",
            ),
        ] {
            let (picture, mut pilot) = flying_at(MID);
            pilot.held_marks = marks;
            pilot.stall_level = 4;
            pilot.reverse_until_ms = NOW + HELD_REVERSE_MS;
            let orders = pilot.decide(&picture, &doctrine(), NOW);
            assert_eq!((orders.heading, orders.speed), (None, Some(kph)), "{why}");
        }
    }

    /// **A hold goes back to `Stopped` once anything else is flown** (fixes
    /// after review): a hold opening the range in reverse, then a decision
    /// under a withdraw, then the hold again inside its hysteresis is a stop,
    /// not the old reverse.
    #[test]
    fn a_hold_stops_again_after_flying_anything_else() {
        let band = f64::from(hold_doctrine().band_m);
        let (mut picture, mut pilot) = flying();
        let opening = (band - HOLD_SLACK_M - HOLD_HYSTERESIS_M - 10.0) as u32;
        let flown = fly_on(&mut picture, &mut pilot, &hold_doctrine(), 0, opening, NOW);
        assert_eq!(flown, (Some(0), Some(-27)));
        assert_eq!(pilot.hold_motion, HoldMotion::Astern);
        let withdraw = Doctrine {
            stance: Stance::Withdraw,
            hold_s: Some(20),
            ..hold_doctrine()
        };
        fly_on(&mut picture, &mut pilot, &withdraw, 0, opening, NOW + 1_000);
        assert_eq!(pilot.hold_motion, HoldMotion::Stopped);
        // Inside the hysteresis, short of the slack: a fresh hold stands.
        let inside = (band - HOLD_SLACK_M - 10.0) as u32;
        let flown = fly_on(
            &mut picture,
            &mut pilot,
            &hold_doctrine(),
            0,
            inside,
            NOW + 2_000,
        );
        assert_eq!(flown, (Some(0), Some(0)));
        assert_eq!(pilot.hold_motion, HoldMotion::Stopped);
    }

    /// **The turn into a hit in a crossfire** (fixes after review): the
    /// nearest hostile in the struck arc is the shooter; a threat within
    /// [`HOLD_BOW_DEG`] of the held target is not turned to (the hold faces
    /// it) and its sentence is not said; and a current threat whose shooter
    /// is gone, with only the held target left to blame, is dropped.
    #[test]
    fn a_crossfire_turns_to_the_nearest_shooter_and_never_off_the_target() {
        // Two in the left arc: the nearer, BB, is faced (BA, nearer still,
        // is the target, due north).
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact_rel("BA", 0, 200, 0));
        hold(&mut picture, contact_rel("BB", 250, 300, 250));
        hold(&mut picture, contact_rel("BC", 290, 600, 290));
        hit(&mut picture, NOW, LEFT_HIT);
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!((orders.heading, orders.speed), (Some(250), Some(0)));
        assert_eq!(
            pilot.take_notes(),
            ["Turning into the shot: hit on the hull's left facet; facing BB."]
        );
        // The shooter (BB, left arc) within `HOLD_BOW_DEG` of the held target
        // (BA, astern-left): the hold already faces it, nothing is said.
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact_rel("BA", 220, 450, 220));
        hold(&mut picture, contact_rel("BB", 240, 500, 240));
        hit(&mut picture, NOW, LEFT_HIT);
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!(
            pilot.threat.map(|(b, _)| whole(b)),
            Some(240),
            "armed on BB"
        );
        assert_eq!((orders.heading, orders.speed), (Some(220), Some(0)));
        assert!(pilot.take_notes().is_empty());
        // A threat armed on BB; BB gone, a second left hit with only BA (well
        // off it) to blame: the threat is dropped, and the hold faces BA.
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact_rel("BA", 0, 450, 0));
        hold(&mut picture, contact_rel("BB", 270, 500, 270));
        hit(&mut picture, NOW, LEFT_HIT);
        pilot.decide(&picture, &hold_doctrine(), NOW);
        assert!(pilot.threat.is_some());
        picture.contacts.remove("BB");
        hit(&mut picture, NOW + 500, LEFT_HIT);
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW + 500);
        assert_eq!(pilot.threat, None);
        assert_eq!((orders.heading, orders.speed), (Some(0), Some(0)));
    }

    /// **A turn into a hit that is not flown is not said** (fixes after
    /// review): a withdrawal outranks step 3b for the whole turn, and the
    /// sentence never goes; a hit read after its turn would be over arms
    /// nothing; and a hit taken while the machine flew the tank is read at the
    /// re-board ([`Pilot::forget_turns`]) and turns nothing.
    #[test]
    fn a_turn_into_a_hit_is_said_only_when_flown() {
        // Falling back on half the hull, BB in the left arc: the reverse bow
        // on BA goes on, and nothing turns or is said.
        let (mut picture, mut pilot) = flying();
        half_hull(&mut picture);
        hold(&mut picture, contact_rel("BA", 0, 250, 0));
        hold(&mut picture, contact_rel("BB", 270, 500, 270));
        hit(&mut picture, NOW, LEFT_HIT);
        for t in [NOW, NOW + THREAT_TURN_MS - 1, NOW + THREAT_TURN_MS] {
            let orders = pilot.decide(&picture, &hold_doctrine(), t);
            assert!(pilot.withdraw_until_ms.is_some(), "still falling back");
            assert_eq!((orders.heading, orders.speed), (Some(0), Some(-27)), "{t}");
            pilot.forget_heading();
            pilot.forget_speed();
        }
        // The fall back over, the turn's time long past: dropped, unsaid.
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW + WITHDRAW_MS);
        assert_eq!(pilot.withdraw_until_ms, None);
        assert_eq!(pilot.threat, None, "lapsed unflown");
        assert_eq!(orders.heading, Some(0), "the hold on BA");
        let notes = pilot.take_notes();
        assert!(
            !notes.iter().any(|n| n.starts_with("Turning into the shot")),
            "{notes:?}"
        );
        // A kept doctrine withdraw outranks it the same way.
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact_rel("BA", 0, 450, 0));
        hold(&mut picture, contact_rel("BB", 270, 500, 270));
        hit(&mut picture, NOW, LEFT_HIT);
        let withdraw = Doctrine {
            stance: Stance::Withdraw,
            hold_s: Some(20),
            ..hold_doctrine()
        };
        let orders = pilot.decide(&picture, &withdraw, NOW);
        assert_eq!((orders.heading, orders.speed), (Some(0), Some(-27)));
        assert!(pilot.take_notes().is_empty());
        // A hit read a whole turn late (after a stretch immobile, say), with
        // nothing to blame: it would have turned to 270. Not even armed (the
        // fall back running here would leave an armed one standing, spent,
        // in the `arena-picture` record).
        let (mut picture, mut pilot) = flying();
        half_hull(&mut picture);
        hit(&mut picture, NOW - THREAT_TURN_MS, LEFT_HIT);
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!(pilot.threat, None);
        assert_eq!(pilot.threat_seen_ms, NOW - THREAT_TURN_MS, "read once");
        assert_ne!(orders.heading, Some(270));
        assert_eq!(
            pilot.take_notes(),
            ["Falling back to 450 m: the hull is at half."]
        );
        // A hit taken while away, read at the re-board.
        let (mut picture, mut pilot) = flying();
        hit(&mut picture, NOW - 1_000, LEFT_HIT);
        pilot.forget_turns(&picture);
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!(pilot.threat, None);
        assert_ne!(orders.heading, Some(270));
        assert!(pilot.take_notes().is_empty());
    }

    /// **A second hit with nothing to blame, mid-turn, does not move the
    /// turn** (fixes after review): the arc's centre is measured from the
    /// reference heading, and while the turn's own heading is ordered and not
    /// yet read by `STATUS` that is where the hull is going, not where it is —
    /// so the turn stands until a `STATUS` reads the hull.
    #[test]
    fn an_unblamed_hit_mid_turn_does_not_send_the_bow_round_again() {
        let (mut picture, mut pilot) = flying();
        hit(&mut picture, NOW, LEFT_HIT);
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!(orders.heading, Some(270));
        pilot.sent_heading(270, NOW);
        pilot.take_notes();
        let armed = pilot.threat;
        // Again on the left, 1.2 s on, no `STATUS` between: the same turn.
        hit(&mut picture, NOW + 1_200, LEFT_HIT);
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW + 1_200);
        assert_eq!(pilot.threat, armed);
        assert_eq!(orders.heading, None, "270 already ordered");
        assert!(pilot.take_notes().is_empty());
        // A `STATUS` reads the hull part way round, at 300: a left hit now
        // is placed from it.
        picture.own.heading_deg = 300;
        picture.own.pos_read_ms = NOW + 2_000;
        hit(&mut picture, NOW + 2_100, LEFT_HIT);
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW + 2_100);
        assert_eq!(
            pilot.threat.map(|(b, u)| (whole(b), u)),
            Some((210, NOW + 2_100 + THREAT_TURN_MS))
        );
        assert_eq!(orders.heading, Some(210));
        assert_eq!(
            pilot.take_notes(),
            ["Turning into the shot: hit on the hull's left facet; facing 210°."]
        );
    }

    /// **One track gone, a hit on the dead side turns nothing** (fixes after
    /// review): the server would turn the bow the long, live way round —
    /// longer than the turn lasts, showing a side and the stern to the held
    /// target on the way — so no turn is armed, ordered or committed, and the
    /// hold keeps its bow; a hit on the live side turns at once; a hit dead
    /// astern (as long either way) turns.
    #[test]
    fn a_one_track_tank_never_turns_the_dead_way_into_a_hit() {
        for (track, dead, live) in [
            ("right_track", ("right", 90), ("left", 270)),
            ("left_track", ("left", 270), ("right", 90)),
        ] {
            let line =
                |facet: &str| format!("Hit on the hull's {facet} facet: 200 of 250 armour left.");
            // A shooter in the dead-side arc, BA held due north.
            let (mut picture, mut pilot) = one_track(track);
            hold(&mut picture, contact_rel("BA", 0, 450, 0));
            hold(&mut picture, contact_rel("BB", dead.1, 500, dead.1));
            hit(&mut picture, NOW, &line(dead.0));
            let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
            assert_eq!(pilot.threat, None, "{track}: {} hit", dead.0);
            assert_eq!(
                (orders.heading, orders.speed),
                (Some(0), Some(0)),
                "{track}"
            );
            if let Some(deg) = orders.heading {
                pilot.sent_heading(deg, NOW);
            }
            assert_eq!(pilot.turn_commit, None, "{track}: nothing committed");
            assert!(
                !pilot
                    .take_notes()
                    .iter()
                    .any(|n| n.starts_with("Turning into the shot")),
                "{track}"
            );
            // Nothing held, nothing in the arc: the dead side's centre, the
            // same.
            let (mut picture, mut pilot) = one_track(track);
            hit(&mut picture, NOW, &line(dead.0));
            let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
            assert_eq!(pilot.threat, None, "{track}: unblamed {} hit", dead.0);
            assert_ne!(orders.heading, Some(dead.1));
            // The live side: turned at once.
            let (mut picture, mut pilot) = one_track(track);
            hold(&mut picture, contact_rel("BA", 0, 450, 0));
            hold(&mut picture, contact_rel("BC", live.1, 500, live.1));
            hit(&mut picture, NOW, &line(live.0));
            let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
            assert_eq!(
                (orders.heading, orders.speed),
                (Some(live.1), Some(0)),
                "{track}: {} hit",
                live.0
            );
            // Dead astern, nothing to blame: as long either way, so turned.
            let (mut picture, mut pilot) = one_track(track);
            hit(
                &mut picture,
                NOW,
                "Hit on the hull's rear facet: 100 of 150 armour left.",
            );
            pilot.decide(&picture, &hold_doctrine(), NOW);
            assert_eq!(pilot.threat.map(|(b, _)| whole(b)), Some(180), "{track}");
        }
    }

    /// **The bot-side observer rule for AL.2's notes**: fed through
    /// [`Picture::apply`], every designator a new note names is one a contact
    /// row or notice named; a mark holds positions, a heading and a direction.
    #[test]
    fn the_new_notes_name_only_designators_the_stream_named() {
        let (mut picture, mut pilot) = flying();
        let lines = [
            (NOW, "Callsign RA."),
            (NOW, "Red Tank 1: power online, 0 kph, heading 000°"),
            (NOW, "  at 1280 m east, 1000 m north; scope holds a hostile"),
            (NOW, "  right_track  destroyed  internals 0/24"),
            (NOW + 10, "New contact X1: bearing 270°, far."),
            (NOW + 20, "Contact X1 identified: BB, blue."),
            (
                NOW + 30,
                "BB  blue  bearing 270° (rel 270°)  far  500 m  0 s",
            ),
            (
                NOW + 31,
                "BA  blue  bearing 000° (rel 000°)  far  300 m  0 s",
            ),
            (NOW + 40, "No such contact."),
            (
                NOW + 50,
                "Hit on the hull's left facet: 200 of 250 armour left.",
            ),
            // (Read after the turn into the hit is flown: a held stop
            // outranks the turn, and its sentence is said only when flown.)
            (
                NOW + 110,
                "Held: the slope ahead is too steep for the tank.",
            ),
        ];
        let mut named = BTreeSet::new();
        let mut notes = Vec::new();
        for (at, text) in lines {
            if at == NOW + 110 {
                assert_eq!(live_turn(&picture), Some(Turn::CounterClockwise));
                pilot.decide(&picture, &hold_doctrine(), NOW + 100);
                notes.extend(pilot.take_notes());
            }
            let line = classify(text);
            match &line {
                crate::arena::Line::ContactRow(row) => {
                    named.insert(row.designator.clone());
                }
                crate::arena::Line::NewContact { designator, .. }
                | crate::arena::Line::ContactLost { designator } => {
                    named.insert(designator.clone());
                }
                crate::arena::Line::Identified {
                    was, designator, ..
                } => {
                    named.insert(was.clone());
                    named.insert(designator.clone());
                }
                _ => {}
            }
            picture.apply(at, &line);
        }
        pilot.decide(&picture, &hold_doctrine(), NOW + 120);
        notes.extend(pilot.take_notes());
        // The search later meets the mark: the detour's sentence too.
        pilot.held_marks.push(HeldMark {
            at_m: (1_280.0, 900.0),
            heading_deg: 90,
            astern: false,
            blocked_m: (1_280.0 + mark_ahead_m(), 1_000.0),
        });
        assert!(pilot.steer_round((1_280.0, 1_000.0), 90.0, -1.0) != 90.0);
        notes.extend(pilot.take_notes());
        for start in [
            "The right_track is gone",
            "Turning into the shot",
            "Held ground marked",
            "Steering round held ground",
        ] {
            assert!(
                notes.iter().any(|n| n.starts_with(start)),
                "{start}: {notes:?}"
            );
        }
        for note in &notes {
            for word in note
                .split(|c: char| !c.is_ascii_alphanumeric())
                .filter(|w| {
                    w.len() > 1
                        && w.chars()
                            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
                        && w.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                })
            {
                assert!(named.contains(word), "{note}: {word}");
            }
        }
        assert!(notes.iter().any(|n| n.ends_with("facing BB.")), "{notes:?}");
        // The threat names nothing but a bearing.
        let value = serde_json::to_value(&pilot).expect("json");
        assert!(value["threat"].is_array());
        for mark in value["held_marks"].as_array().expect("marks") {
            let keys: BTreeSet<&str> = mark
                .as_object()
                .expect("a mark")
                .keys()
                .map(String::as_str)
                .collect();
            assert_eq!(
                keys,
                BTreeSet::from(["astern", "at_m", "blocked_m", "heading_deg"])
            );
        }
    }

    // -- cut AL.3: seen second ---------------------------------------------------------

    /// A return not yet told apart at `bearing`/`range` from mid-field, its row
    /// read at `t`.
    fn unknown_row(picture: &mut Picture, designator: &str, bearing: u32, range: u32, t: u64) {
        let mut row = contact_from(MID, designator, None, bearing, range);
        row.read_ms = t;
        hold(picture, row);
    }

    /// A hit taken at `at_ms` that names no facet (no turn into it).
    fn hit_taken(picture: &mut Picture, at_ms: u64) {
        picture.events.push_back(Event {
            at_ms,
            kind: EventKind::HitTaken,
            text: "Hit on the hull.".into(),
        });
    }

    /// **A return not yet told apart is faced, stopped** (cut AL.3, step 4a):
    /// a row at bearing 087° stops the tank facing it, said once; past
    /// `UNIDENTIFIED_WAIT_MS` the pilot moves on; told apart as a hostile it is
    /// held; a return known only from its notice does not stop the tank, nor
    /// does a second inside the rest after a stop, nor any past
    /// `UNIDENTIFIED_STOPS_MAX` a match; and a hit taken ends a stop for good.
    #[test]
    fn an_unidentified_return_stops_the_tank_facing_it() {
        let (mut picture, mut pilot) = flying_at(MID);
        unknown_row(&mut picture, "X1", 87, 450, NOW);
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!((orders.heading, orders.speed), (Some(87), Some(0)));
        assert!(!orders.ask_spot_b, "not searching: {orders:?}");
        assert_eq!(
            pilot.take_notes(),
            ["Stopping for X1: facing 087° while it is told apart."]
        );
        assert_eq!(
            pilot.wait,
            Some(("X1".to_owned(), NOW + UNIDENTIFIED_WAIT_MS))
        );
        assert_eq!(pilot.unknown_stops, 1);
        // Still not told apart at its time's end: still faced, not said again.
        unknown_row(&mut picture, "X1", 87, 450, NOW + UNIDENTIFIED_WAIT_MS);
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW + UNIDENTIFIED_WAIT_MS);
        assert_eq!((orders.heading, orders.speed), (Some(87), Some(0)));
        // Past it, the pilot moves on: the search, full ahead.
        let t = NOW + UNIDENTIFIED_WAIT_MS + 50;
        unknown_row(&mut picture, "X1", 87, 450, t);
        let orders = pilot.decide(&picture, &hold_doctrine(), t);
        assert!(orders.ask_spot_b, "{orders:?}");
        assert_eq!(orders.speed, Some(54));
        assert_eq!(
            (pilot.wait.clone(), pilot.unknown_stop.clone()),
            (None, None)
        );
        assert!(pilot.take_notes().is_empty(), "said once");

        // Told apart as a hostile inside its time: the hold (BA 700 m off at
        // 087°, so the hold closes, full ahead at it).
        let (mut picture, mut pilot) = flying_at(MID);
        unknown_row(&mut picture, "X1", 87, 700, NOW);
        assert_eq!(pilot.decide(&picture, &hold_doctrine(), NOW).speed, Some(0));
        picture.contacts.clear();
        let mut ba = contact_from(MID, "BA", Some(Side::Blue), 87, 700);
        ba.read_ms = NOW + 3_000;
        hold(&mut picture, ba);
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW + 3_000);
        assert_eq!((orders.heading, orders.speed), (Some(87), Some(54)));
        assert_eq!(pilot.hold_motion, HoldMotion::Ahead);
        assert_eq!(pilot.unknown_stop, None);

        // Known only from its notice (no row, so no bearing it read): no stop.
        let (mut picture, mut pilot) = flying_at(MID);
        picture.apply(NOW, &classify("New contact X1: bearing 087°, far."));
        assert!(
            picture
                .contacts
                .get("X1")
                .is_some_and(|c| c.range_m.is_none())
        );
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert!(orders.ask_spot_b, "searching: {orders:?}");
        assert_eq!(pilot.unknown_stops, 0);

        // A second return inside the rest after a stop does not stop the tank;
        // one after the rest does.
        let (mut picture, mut pilot) = flying_at(MID);
        unknown_row(&mut picture, "X1", 87, 450, NOW);
        pilot.decide(&picture, &hold_doctrine(), NOW);
        picture.contacts.clear();
        let ended = NOW + 1_000;
        assert_eq!(
            pilot.decide(&picture, &hold_doctrine(), ended).speed,
            Some(54)
        );
        assert_eq!(pilot.unknown_rest_until_ms, ended + UNIDENTIFIED_WAIT_MS);
        let t = ended + 1_000;
        unknown_row(&mut picture, "X2", 200, 450, t);
        let orders = pilot.decide(&picture, &hold_doctrine(), t);
        assert_eq!(orders.speed, Some(54), "inside the rest");
        let t = ended + UNIDENTIFIED_WAIT_MS;
        unknown_row(&mut picture, "X2", 200, 450, t);
        let orders = pilot.decide(&picture, &hold_doctrine(), t);
        assert_eq!(
            (orders.heading, orders.speed),
            (Some(200), Some(0)),
            "after the rest"
        );

        // At most `UNIDENTIFIED_STOPS_MAX` a match.
        let (mut picture, mut pilot) = flying_at(MID);
        let mut t = NOW;
        for k in 0..=UNIDENTIFIED_STOPS_MAX {
            picture.contacts.clear();
            unknown_row(&mut picture, &format!("X{}", k + 1), 87, 450, t);
            let orders = pilot.decide(&picture, &hold_doctrine(), t);
            let want = if k < UNIDENTIFIED_STOPS_MAX { 0 } else { 54 };
            assert_eq!(orders.speed, Some(want), "return {k}");
            // It goes, and the rest runs out before the next.
            picture.contacts.clear();
            pilot.decide(&picture, &hold_doctrine(), t + 100);
            t += 100 + UNIDENTIFIED_WAIT_MS;
        }
        assert_eq!(pilot.unknown_stops, UNIDENTIFIED_STOPS_MAX);

        // A hit taken ends the stop for good.
        let (mut picture, mut pilot) = flying_at(MID);
        unknown_row(&mut picture, "X1", 87, 450, NOW);
        pilot.decide(&picture, &hold_doctrine(), NOW);
        hit_taken(&mut picture, NOW + 500);
        for t in [NOW + 600, NOW + 2_000] {
            unknown_row(&mut picture, "X1", 87, 450, t);
            let orders = pilot.decide(&picture, &hold_doctrine(), t);
            assert_eq!(orders.speed, Some(54), "at {t}");
        }
        assert_eq!(pilot.unknown_stop, None);

        // Lost and found again, a return is an unknown one once more and
        // reads `X1` again (`help cockpit`): first seen afresh, so stopped
        // for — and said — again, long after the first X1 was first seen.
        let (mut picture, mut pilot) = flying_at(MID);
        unknown_row(&mut picture, "X1", 87, 450, NOW);
        pilot.decide(&picture, &hold_doctrine(), NOW);
        pilot.take_notes();
        picture.contacts.clear();
        let mut ba = contact_from(MID, "BA", Some(Side::Blue), 87, 450);
        ba.read_ms = NOW + 3_000;
        hold(&mut picture, ba);
        pilot.decide(&picture, &hold_doctrine(), NOW + 3_000);
        assert!(!pilot.unknown_since.contains_key("X1"), "told apart");
        // BA hurt, so its hunt drives rather than waits; then it is lost.
        hurt_event(&mut picture, "BA", NOW + 3_500);
        picture.contacts.clear();
        pilot.decide(&picture, &hold_doctrine(), NOW + 4_000);
        assert!(pilot.hunt.is_some());
        let t = NOW + 3_000 + UNIDENTIFIED_WAIT_MS + 1_000;
        unknown_row(&mut picture, "X1", 87, 450, t);
        let orders = pilot.decide(&picture, &hold_doctrine(), t);
        assert_eq!(
            (orders.heading, orders.speed),
            (Some(87), Some(0)),
            "found again"
        );
        assert_eq!(pilot.unknown_since.get("X1"), Some(&t));
        assert_eq!(pilot.unknown_stops, 2);
        let notes = pilot.take_notes();
        assert!(
            notes
                .iter()
                .any(|n| n == "Stopping for X1: facing 087° while it is told apart."),
            "{notes:?}"
        );

        // The turn into a side hit outranks the stop: a return first seen
        // as the hit lands is not stopped for while the turn lasts; after it,
        // a new return is.
        let (mut picture, mut pilot) = flying_at(MID);
        unknown_row(&mut picture, "X1", 87, 450, NOW + 600);
        hit(&mut picture, NOW + 600, LEFT_HIT);
        for t in [NOW + 600, NOW + 2_000] {
            unknown_row(&mut picture, "X1", 87, 450, t);
            let orders = pilot.decide(&picture, &hold_doctrine(), t);
            assert_eq!(
                (orders.heading, orders.speed),
                (Some(270), Some(0)),
                "at {t}"
            );
            assert_eq!((pilot.unknown_stop.clone(), pilot.unknown_stops), (None, 0));
        }
        let t = NOW + 600 + THREAT_TURN_MS;
        picture.contacts.clear();
        unknown_row(&mut picture, "X2", 200, 450, t);
        let orders = pilot.decide(&picture, &hold_doctrine(), t);
        assert_eq!((orders.heading, orders.speed), (Some(200), Some(0)));
        assert_eq!(pilot.unknown_stops, 1);
    }

    /// **The hunt stops short of its estimate and waits, once** (cut AL.3,
    /// step 4b): an unhurt, untracked BA lost inside `SEEN_SECOND_M` is waited
    /// for, stopped and facing its plot, its legs frozen through the wait;
    /// after `AMBUSH_WAIT_MS` the hunt goes on and never waits again for that
    /// trail; outside `SEEN_SECOND_M` there is no stop until the estimate is
    /// inside it; a hurt BA and a track going away or across are hunted at
    /// once, a closing track waits; a hit taken ends the wait; a fresh hostile
    /// ends it into the hold; and the turn into a side hit outranks it.
    #[test]
    fn the_hunt_stops_short_of_its_estimate_and_waits_once() {
        let (mut picture, mut pilot, orders) = lost(MID, 0, 450, None);
        assert_eq!((orders.heading, orders.speed), (Some(0), Some(0)));
        let began = NOW + 1_000;
        let hunt = hunt_of(&pilot);
        assert_eq!(hunt.leg, HuntLeg::ToPlot);
        assert_eq!((hunt.wait_since_ms, hunt.wait_spent), (Some(began), false));
        assert_eq!(
            pilot.take_notes(),
            [
                "Hunting BA: its last plot, 450 m away.",
                "Stopping before BA finds us: its estimate is 450 m off at 000°."
            ]
        );
        assert_eq!(pilot.wait, Some(("BA".to_owned(), began + AMBUSH_WAIT_MS)));
        // The legs are frozen through the wait: at the plot, nothing arrives.
        let plot = hunt.trail.plot_m;
        read_at(&mut picture, plot, began + 5_000);
        let orders = pilot.decide(&picture, &doctrine(), began + 5_000);
        assert_eq!(orders.speed, Some(0));
        assert_eq!(hunt_of(&pilot).leg, HuntLeg::ToPlot, "no arrival judged");
        assert!(pilot.take_notes().is_empty());
        // Its time up: the hunt goes on (arrived: the course leg), and says so.
        let over = began + AMBUSH_WAIT_MS;
        read_at(&mut picture, plot, over);
        let orders = pilot.decide(&picture, &doctrine(), over);
        assert_eq!(orders.speed, Some(54));
        let hunt = hunt_of(&pilot);
        assert_eq!(hunt.leg, HuntLeg::Along);
        assert!(hunt.wait_spent);
        assert_eq!(
            pilot.take_notes(),
            [
                "The wait for BA is over; hunting on.",
                "Hunting BA: along 000° from its last plot."
            ]
        );
        // Never again for that trail: 450 m off again, it drives.
        read_at(&mut picture, MID, over + 1_000);
        let orders = pilot.decide(&picture, &doctrine(), over + 1_000);
        assert_eq!(orders.speed, Some(54));
        assert_eq!(pilot.wait, None);

        // Outside `SEEN_SECOND_M`: no stop, until the estimate comes inside it.
        let far = (SEEN_SECOND_M + 100.0) as u32;
        let (mut picture, mut pilot, orders) = lost(MID, 0, far, None);
        assert_eq!(orders.speed, Some(54));
        assert_eq!(hunt_of(&pilot).wait_since_ms, None);
        read_at(&mut picture, along(MID, 0.0, 150.0), NOW + 10_000);
        let orders = pilot.decide(&picture, &doctrine(), NOW + 10_000);
        assert_eq!(orders.speed, Some(0));
        assert_eq!(hunt_of(&pilot).wait_since_ms, Some(NOW + 10_000));

        // Hurt: hunted at once. A track going away or across: at once; a
        // closing one waits.
        let (_, pilot, orders) = lost_hurt(MID, 0, 450, None);
        assert_eq!(orders.speed, Some(54), "hurt");
        assert_eq!(hunt_of(&pilot).wait_since_ms, None);
        for (course, waits) in [
            (0.0, false),
            (90.0, false),
            (270.0, false),
            (150.0, true),
            (180.0, true),
        ] {
            let track = Track {
                course_deg: course,
                speed_mps: 8.0,
            };
            let (_, pilot, orders) = lost(MID, 0, 450, Some(track));
            let want = if waits { 0 } else { 54 };
            assert_eq!(orders.speed, Some(want), "course {course}");
            assert_eq!(
                hunt_of(&pilot).wait_since_ms.is_some(),
                waits,
                "course {course}"
            );
        }
        // Going across when the hunt began stays so: past the plot on the
        // course leg, where the bearing back to the plot is the course turned
        // round and the estimate a few tens of metres off, it still drives.
        let track = Track {
            course_deg: 90.0,
            speed_mps: 8.0,
        };
        let (mut picture, mut pilot, _) = lost(MID, 0, 450, Some(track));
        assert!(hunt_of(&pilot).going_away);
        let plot = hunt_of(&pilot).trail.plot_m;
        for (pos, t) in [
            (plot, NOW + 10_000),
            ((plot.0 + 10.0, plot.1), NOW + 11_000),
            ((plot.0 + 40.0, plot.1), NOW + 13_000),
        ] {
            read_at(&mut picture, pos, t);
            let orders = pilot.decide(&picture, &doctrine(), t);
            assert_eq!(orders.speed, Some(54), "at {pos:?}");
            assert_eq!(hunt_of(&pilot).leg, HuntLeg::Along, "at {pos:?}");
            assert_eq!(hunt_of(&pilot).wait_since_ms, None, "at {pos:?}");
        }
        assert!(
            !pilot
                .take_notes()
                .iter()
                .any(|n| n.starts_with("Stopping before")),
            "never waited"
        );

        // A hit taken after the wait began ends it, for good.
        let (mut picture, mut pilot, _) = lost(MID, 0, 450, None);
        pilot.take_notes();
        hit_taken(&mut picture, NOW + 2_000);
        let orders = pilot.decide(&picture, &doctrine(), NOW + 2_100);
        assert_eq!(orders.speed, Some(54));
        assert!(hunt_of(&pilot).wait_spent);
        assert_eq!(pilot.take_notes(), ["The wait for BA is over; hunting on."]);
        let orders = pilot.decide(&picture, &doctrine(), NOW + 3_000);
        assert_eq!(orders.speed, Some(54), "for good");

        // A fresh hostile ends it into the hold (BB 700 m off at 090°: the
        // hold closes, full ahead at it).
        let (mut picture, mut pilot, _) = lost(MID, 0, 450, None);
        pilot.take_notes();
        let t = NOW + 3_000;
        let mut bb = contact_from(MID, "BB", Some(Side::Blue), 90, 700);
        bb.read_ms = t;
        hold(&mut picture, bb);
        let orders = pilot.decide(&picture, &hold_doctrine(), t);
        assert!(pilot.hunt.is_none());
        assert_eq!((orders.heading, orders.speed), (Some(90), Some(54)));
        assert_eq!(pilot.hold_motion, HoldMotion::Ahead);
        assert_eq!(pilot.wait, None);
        assert_eq!(
            pilot.take_notes(),
            ["The hunt for BA ends: BB is in sight."]
        );

        // The turn into a side hit outranks it; the hit has ended it, so after
        // the turn the hunt drives.
        let (mut picture, mut pilot, _) = lost(MID, 0, 450, None);
        let t = NOW + 2_000;
        hit(&mut picture, t, LEFT_HIT);
        let orders = pilot.decide(&picture, &hold_doctrine(), t);
        assert_eq!(
            (orders.heading, orders.speed),
            (Some(270), Some(0)),
            "the turn into the shot: {:?}",
            pilot.take_notes()
        );
        assert!(pilot.threat.is_some());
        let t = t + THREAT_TURN_MS;
        let orders = pilot.decide(&picture, &hold_doctrine(), t);
        assert_eq!(orders.speed, Some(54));
        assert!(hunt_of(&pilot).wait_spent);

        // A side hit before the wait begins: the turn is flown and no wait
        // begins while it lasts; after it, the wait does (a hit before it
        // began does not end it).
        let (mut picture, mut pilot) = flying_at(MID);
        hold(
            &mut picture,
            contact_from(MID, "BA", Some(Side::Blue), 0, 450),
        );
        decide(&picture, &mut pilot);
        picture.contacts.clear();
        hit(&mut picture, NOW + 600, LEFT_HIT);
        for t in [NOW + 1_000, NOW + 3_000] {
            let orders = pilot.decide(&picture, &hold_doctrine(), t);
            assert_eq!(
                (orders.heading, orders.speed),
                (Some(270), Some(0)),
                "at {t}"
            );
            assert_eq!(hunt_of(&pilot).wait_since_ms, None, "at {t}");
            assert_eq!(pilot.wait, None, "at {t}");
        }
        let t = NOW + 600 + THREAT_TURN_MS;
        let orders = pilot.decide(&picture, &hold_doctrine(), t);
        assert_eq!((orders.heading, orders.speed), (Some(0), Some(0)));
        assert_eq!(hunt_of(&pilot).wait_since_ms, Some(t));
    }

    /// **The bot-side observer rule for AL.3's notes**: fed through
    /// [`Picture::apply`], every designator a wait's sentence names is one a
    /// contact row or notice named.
    #[test]
    fn the_waits_name_only_designators_the_stream_named() {
        fn names(line: &crate::arena::Line) -> Vec<String> {
            match line {
                crate::arena::Line::ContactRow(row) => vec![row.designator.clone()],
                crate::arena::Line::NewContact { designator, .. }
                | crate::arena::Line::ContactLost { designator } => vec![designator.clone()],
                crate::arena::Line::Identified {
                    was, designator, ..
                } => vec![was.clone(), designator.clone()],
                _ => Vec::new(),
            }
        }
        let (mut picture, mut pilot) = flying();
        let mut named = BTreeSet::new();
        let mut notes = Vec::new();
        let script: [(u64, &str); 8] = [
            (NOW, "Callsign RA."),
            (NOW, "Red Tank 1: power online, 0 kph, heading 000°"),
            (NOW, "  at 1280 m east, 1000 m north; scope holds a hostile"),
            (NOW + 10, "New contact X1: bearing 087°, far."),
            (
                NOW + 20,
                "X1  unknown  bearing 087° (rel 087°)  far  450 m  0 s",
            ),
            (NOW + 3_000, "Contact X1 identified: BA, blue."),
            (
                NOW + 3_010,
                "BA  blue  bearing 087° (rel 087°)  far  450 m  0 s",
            ),
            (NOW + 4_000, "Contact BA lost."),
        ];
        for (at, text) in script {
            let line = classify(text);
            named.extend(names(&line));
            picture.apply(at, &line);
            // A row is read once its reply has settled.
            picture.settle(at + 200);
            pilot.decide(&picture, &hold_doctrine(), at + 200);
            notes.extend(pilot.take_notes());
        }
        let over = NOW + 4_200 + AMBUSH_WAIT_MS;
        pilot.decide(&picture, &hold_doctrine(), over);
        notes.extend(pilot.take_notes());
        for start in [
            "Stopping for X1: ",
            "Stopping before BA finds us: ",
            "The wait for BA is over",
        ] {
            assert!(
                notes.iter().any(|n| n.starts_with(start)),
                "{start}: {notes:?}"
            );
        }
        for note in &notes {
            for word in note
                .split(|c: char| !c.is_ascii_alphanumeric())
                .filter(|w| {
                    w.len() > 1
                        && w.chars()
                            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
                        && w.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                })
            {
                assert!(named.contains(word), "{note}: {word}");
            }
        }
        // The wait in the pilot's state names a designator the stream named.
        let value = serde_json::to_value(&pilot).expect("json");
        assert!(value["wait"].is_null(), "{}", value["wait"]);
        for designator in value["unknown_since"].as_object().expect("a map").keys() {
            assert!(named.contains(designator), "{designator}");
        }
        for designator in value["hurt"].as_array().expect("a set") {
            assert!(named.contains(designator.as_str().expect("a word")));
        }
    }

    /// **The waits end while immobile** (cut AM.2): an immobile or disabled
    /// tank flies no motion step, so an unidentified stop in force ends (its
    /// rest begins), no wait stands, a hold goes back to stopped, and nothing
    /// of the heading or the speed is ordered.
    #[test]
    fn the_waits_end_while_immobile() {
        for disabled in [false, true] {
            let (mut picture, mut pilot) = flying_at(MID);
            unknown_row(&mut picture, "X1", 87, 450, NOW);
            let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
            assert_eq!((orders.heading, orders.speed), (Some(87), Some(0)));
            assert!(pilot.unknown_stop.is_some() && pilot.wait.is_some());
            // A hold left moving, to see it go back to stopped.
            pilot.hold_motion = HoldMotion::Ahead;
            if disabled {
                picture.own.disabled = true;
            } else {
                picture.own.immobile = true;
            }
            let t = NOW + 1_000;
            unknown_row(&mut picture, "X1", 87, 450, t);
            let orders = pilot.decide(&picture, &hold_doctrine(), t);
            assert_eq!(pilot.wait, None, "disabled {disabled}");
            assert_eq!(pilot.unknown_stop, None, "disabled {disabled}");
            assert_eq!(
                pilot.unknown_rest_until_ms,
                t + UNIDENTIFIED_WAIT_MS,
                "disabled {disabled}"
            );
            assert_eq!(
                pilot.hold_motion,
                HoldMotion::Stopped,
                "disabled {disabled}"
            );
            assert_eq!(
                (orders.heading, orders.speed),
                (None, None),
                "disabled {disabled}: {orders:?}"
            );
        }
    }

    // -- cut AN.2: the opening, the halt and the re-approach --------------------

    /// When the own `Power online.` is read in these tests.
    const POWER: u64 = 10_000;
    /// The shipped muster of red's slot 1, on the east-west road.
    const MUSTER: (f64, f64) = (60.0, 970.0);
    /// **A muster with room behind it** (cut AO.1): 400 m from the west edge,
    /// past [`OPENING_ROOM_M`], so the opening steps off the road at once.
    /// The other side's entry is then (2 160, 970). Where the opening's tests
    /// whose subject is not the room are flown.
    const ROOMY_MUSTER: (f64, f64) = (400.0, 970.0);

    /// A running red tank's picture, aboard, in a skirmish, its ceilings read.
    fn running() -> Picture {
        let mut picture = Picture {
            phase: Phase::Running { match_id: 1 },
            following: Some(1),
            side: Some(Side::Red),
            aboard: true,
            ..Picture::default()
        };
        picture.own.max_ahead_kph = Some(54);
        picture.own.max_astern_kph = Some(27);
        picture
    }

    /// **At the muster** (cut AN.2): this match's own `Power online.` read at
    /// [`POWER`], the pilot's first decision 50 ms on (nothing read yet), then
    /// the first `STATUS` at `start` and the first `CONTACTS` reply (empty)
    /// 100 ms after power. The pilot flies [`Knobs::on`].
    fn at_muster(start: (f64, f64)) -> (Picture, Pilot) {
        let mut picture = running();
        picture.apply(POWER, &classify("Power online."));
        let mut pilot = Pilot::with_knobs(Knobs::on());
        pilot.ceilings_asked = true;
        let first = pilot.decide(&picture, &hold_doctrine(), POWER + 50);
        assert!(!first.ask_view_brief, "nothing read yet");
        status_at(&mut picture, start, 90, POWER + 100);
        picture.apply(POWER + 100, &classify("No contacts."));
        (picture, pilot)
    }

    fn decide_at(picture: &Picture, pilot: &mut Pilot, at: u64) -> Orders {
        pilot.decide(picture, &hold_doctrine(), at)
    }

    /// The brief's header (the shipped 256 by 256 cells of 10 m), then `rays`
    /// (`{Bearing}: …` lines) and `No contacts.`, read at `at`.
    fn read_brief(picture: &mut Picture, at: u64, rays: &[String]) {
        picture.apply(
            at,
            &classify(
                "Window around RA: cells 0 to 38 east and 88 to 107 north of 256 by 256; you at \
                 (6, 97), heading 090°; 10 m a cell.",
            ),
        );
        for ray in rays {
            picture.apply(at, &classify(ray));
        }
        picture.apply(at, &classify("No contacts."));
    }

    /// Eight rays, the one for each bearing `(word, deg)` given by `ray`.
    fn rays(ray: impl Fn(&str, u32) -> String) -> Vec<String> {
        crate::arena::lines::BRIEF_BEARINGS
            .iter()
            .map(|(word, deg)| ray(word, *deg))
            .collect()
    }

    /// **The shipped muster's brief**: north and south 10 m of road then
    /// grass, east and west all road, the diagonals 10 m of road then grass.
    fn road_brief() -> Vec<String> {
        rays(|word, deg| match deg {
            90 | 270 => format!("{word}: 190 m of road, then beyond the window."),
            _ => format!("{word}: 10 m of road, then grass to 90 m, then beyond the window."),
        })
    }

    /// **The shipped muster's brief as the window clamps it** (cut AO.1):
    /// [`road_brief`] with the west ray `60 m of road` to the arena's edge and
    /// the east ray `320 m of road` to the window's reach.
    fn muster_brief() -> Vec<String> {
        rays(|word, deg| match deg {
            270 => format!("{word}: 60 m of road, then the edge of the arena."),
            90 => format!("{word}: 320 m of road, then beyond the window."),
            _ => format!("{word}: 10 m of road, then grass to 90 m, then beyond the window."),
        })
    }

    /// The muster, its brief read, the step chosen: the pilot and the picture
    /// at the step's first decision, and that decision's orders.
    fn stepping(start: (f64, f64), brief: &[String]) -> (Picture, Pilot, Orders) {
        let (mut picture, mut pilot) = at_muster(start);
        let asked = decide_at(&picture, &mut pilot, POWER + 150);
        assert!(asked.ask_view_brief, "the brief is asked for");
        assert!(matches!(pilot.opening, OpeningStage::Brief { .. }));
        read_brief(&mut picture, POWER + 1_000, brief);
        let orders = decide_at(&picture, &mut pilot, POWER + 1_050);
        (picture, pilot, orders)
    }

    /// The muster in a field of grass: the opening holds at once.
    fn holding(start: (f64, f64)) -> (Picture, Pilot) {
        let (picture, mut pilot, _) = stepping(
            start,
            &rays(|word, _| format!("{word}: 90 m of grass, then beyond the window.")),
        );
        assert!(matches!(pilot.opening, OpeningStage::Hold { .. }));
        pilot.take_notes();
        (picture, pilot)
    }

    /// A contact `range` m from `from` on `bearing`, read at `t`, its plot
    /// `age_ms` old.
    fn row_at(
        from: (f64, f64),
        designator: &str,
        side: Option<Side>,
        bearing: u32,
        range: u32,
        t: u64,
        age_ms: u64,
    ) -> Contact {
        let mut row = contact_from(from, designator, side, bearing, range);
        row.read_ms = t;
        row.age_ms = age_ms;
        for fix in &mut row.fixes {
            fix.at_ms = t.saturating_sub(500 + age_ms);
        }
        row
    }

    #[test]
    fn the_other_entry_is_the_muster_mirrored() {
        let side = (ARENA_SIDE_M, ARENA_SIDE_M);
        assert_eq!(other_entry((65.0, 975.0), side), (2_495.0, 975.0));
        assert_eq!(other_entry((1_280.0, 40.0), side), (1_280.0, 2_520.0));
        // A ground the brief's header sized: 100 by 50 cells of 10 m.
        let header = (1_000.0, 500.0);
        assert_eq!(other_entry((30.0, 240.0), header), (970.0, 240.0));
        assert_eq!(other_entry((500.0, 20.0), header), (500.0, 480.0));
        // The pilot reads the side off the header when it has one, else the
        // help's 2.56 km.
        let (mut picture, mut pilot) = at_muster((65.0, 975.0));
        decide_at(&picture, &mut pilot, POWER + 150);
        assert_eq!(pilot.muster_m, Some((65.0, 975.0)));
        assert_eq!(pilot.other_entry_now(&picture), Some((2_495.0, 975.0)));
        picture.apply(
            POWER + 200,
            &classify(
                "Window around RA: cells 0 to 38 east and 88 to 107 north of 200 by 256; you at \
                 (6, 97), heading 090°; 10 m a cell.",
            ),
        );
        assert_eq!(pilot.other_entry_now(&picture), Some((1_935.0, 975.0)));
    }

    #[test]
    fn the_opening_steps_off_the_road_and_faces_the_other_entry() {
        let (mut picture, mut pilot, orders) = stepping(ROOMY_MUSTER, &road_brief());
        // s 10 (the road's reach), a 5 (half a cell), and the cover (35 m
        // north at the shipped 20).
        let step = 10.0 + 5.0 + OPENING_COVER_M;
        assert_eq!(
            pilot.take_notes(),
            [format!(
                "Opening: stepping off the road to the north ({step:.0} m)."
            )]
        );
        let OpeningStage::Step { aim_m, from_m, .. } = pilot.opening.clone() else {
            panic!("stepping: {:?}", pilot.opening);
        };
        assert_eq!(from_m, ROOMY_MUSTER);
        assert!(distance_m(aim_m, (400.0, 970.0 + step)) < 1e-6, "{aim_m:?}");
        assert_eq!(
            (orders.heading, orders.speed),
            (Some(0), Some(OPENING_STEP_KPH))
        );
        assert!(!orders.ask_view_brief, "the brief is read");
        assert!(!orders.ask_spot_b);
        assert_eq!(pilot.rule(), Some("opening"));
        pilot.sent_heading(0, POWER + 1_050);
        pilot.sent_speed(OPENING_STEP_KPH);
        // One metre short of arriving: still stepping.
        let short = (400.0, 970.0 + step - OPENING_ARRIVE_M - 1.0);
        status_at(&mut picture, short, 0, POWER + 3_000);
        let orders = decide_at(&picture, &mut pilot, POWER + 3_000);
        assert!(matches!(pilot.opening, OpeningStage::Step { .. }));
        assert_eq!((orders.heading, orders.speed), (None, None));
        assert!(pilot.take_notes().is_empty());
        // Arrived, `OPENING_ARRIVE_M` short of the aim: the hold, stopped, the
        // bow toward the other side's entry (2 160, 970).
        let arrived = (400.0, 970.0 + step - OPENING_ARRIVE_M);
        status_at(&mut picture, arrived, 0, POWER + 4_000);
        let orders = decide_at(&picture, &mut pilot, POWER + 4_000);
        let facing = whole(bearing_deg(arrived, (2_160.0, 970.0)));
        assert_eq!(facing, 91);
        assert_eq!((orders.heading, orders.speed), (Some(facing), Some(0)));
        assert!(matches!(
            pilot.opening,
            OpeningStage::Hold { facing_deg: 91, .. }
        ));
        assert_eq!(
            pilot.take_notes(),
            [format!(
                "Opening: holding near our entry, facing {facing:03}° toward the other side's entry."
            )]
        );
        assert!(!orders.ask_spot_b);
        assert_eq!(pilot.rule(), Some("opening"));
        // Held there: nothing more is ordered, and no search.
        pilot.sent_heading(facing, POWER + 4_000);
        pilot.sent_speed(0);
        let orders = decide_at(&picture, &mut pilot, POWER + 60_000);
        assert_eq!(orders, Orders::default());
        assert!(pilot.take_notes().is_empty());

        // A diagonal step's aim carries a whole diagonal cell and a half (15
        // m; 45 m at the shipped cover): only the north-west offers grass,
        // from (300, 970).
        let diagonal = 10.0 + 15.0 + OPENING_COVER_M;
        let start = (300.0, 970.0);
        let brief = rays(|word, deg| match deg {
            315 => format!("{word}: 10 m of road, then grass to 90 m, then beyond the window."),
            _ => format!("{word}: 190 m of road, then beyond the window."),
        });
        let (_, mut pilot, orders) = stepping(start, &brief);
        assert_eq!(
            pilot.take_notes(),
            [format!(
                "Opening: stepping off the road to the north-west ({diagonal:.0} m)."
            )]
        );
        let OpeningStage::Step { aim_m, .. } = pilot.opening.clone() else {
            panic!("stepping: {:?}", pilot.opening);
        };
        assert!(distance_m(aim_m, along(start, 315.0, diagonal)) < 1e-6);
        assert_eq!(
            (orders.heading, orders.speed),
            (Some(315), Some(OPENING_STEP_KPH))
        );
        assert!(!orders.ask_spot_b);
    }

    #[test]
    fn on_cover_the_opening_holds_where_it_stands() {
        let (_, mut pilot, orders) = stepping(
            ROOMY_MUSTER,
            &rays(|word, _| format!("{word}: 90 m of grass, then beyond the window.")),
        );
        assert_eq!(
            pilot.take_notes(),
            ["Opening: holding near our entry, facing 090° toward the other side's entry."]
        );
        assert_eq!((orders.heading, orders.speed), (Some(90), Some(0)));
        // Six of eight on cover is enough: north and south road.
        let brief = rays(|word, deg| match deg {
            0 | 180 => format!("{word}: 10 m of road, then grass to 90 m, then beyond the window."),
            _ => format!("{word}: 30 m of light woods, then beyond the window."),
        });
        let (_, mut pilot, _) = stepping(ROOMY_MUSTER, &brief);
        assert!(matches!(pilot.opening, OpeningStage::Hold { .. }));
        assert!(
            pilot
                .take_notes()
                .iter()
                .all(|note| !note.contains("stepping"))
        );
    }

    #[test]
    fn with_no_brief_the_opening_holds_after_its_wait() {
        let (picture, mut pilot) = at_muster(ROOMY_MUSTER);
        let orders = decide_at(&picture, &mut pilot, POWER + 150);
        assert!(orders.ask_view_brief);
        assert_eq!((orders.heading, orders.speed), (None, None));
        let orders = decide_at(
            &picture,
            &mut pilot,
            POWER + 150 + OPENING_BRIEF_WAIT_MS - 1,
        );
        assert!(orders.ask_view_brief, "still asking");
        assert!(matches!(pilot.opening, OpeningStage::Brief { .. }));
        let orders = decide_at(&picture, &mut pilot, POWER + 150 + OPENING_BRIEF_WAIT_MS);
        assert!(!orders.ask_view_brief);
        assert!(matches!(pilot.opening, OpeningStage::Hold { .. }));
        assert_eq!(
            pilot.take_notes(),
            ["Opening: holding near our entry, facing 090° toward the other side's entry."]
        );
        assert_eq!((orders.heading, orders.speed), (Some(90), Some(0)));
        // A dark window is no ground either: a whole brief that offers the
        // north step, then the dark answer before the next decision, leaves
        // no ground, and the opening holds where it stands once its wait is
        // up, with no step.
        let (mut picture, mut pilot) = at_muster(ROOMY_MUSTER);
        decide_at(&picture, &mut pilot, POWER + 150);
        read_brief(&mut picture, POWER + 800, &road_brief());
        assert!(
            picture
                .ground
                .as_ref()
                .is_some_and(|ground| ground.complete)
        );
        picture.apply(
            POWER + 900,
            &classify("The window is dark: the panel has told you nothing yet."),
        );
        assert_eq!(picture.ground, None);
        decide_at(&picture, &mut pilot, POWER + 1_000);
        assert!(
            matches!(pilot.opening, OpeningStage::Brief { .. }),
            "{:?}",
            pilot.opening
        );
        let orders = decide_at(&picture, &mut pilot, POWER + 150 + OPENING_BRIEF_WAIT_MS);
        assert!(matches!(pilot.opening, OpeningStage::Hold { .. }));
        let notes = pilot.take_notes();
        assert!(
            notes.iter().all(|note| !note.contains("stepping")),
            "{notes:?}"
        );
        assert_eq!(orders.speed, Some(0));
    }

    #[test]
    fn the_opening_never_steps_toward_the_other_entry() {
        let brief = rays(|word, deg| match deg {
            45 | 90 | 135 => {
                format!("{word}: 10 m of road, then grass to 90 m, then beyond the window.")
            }
            _ => format!("{word}: 190 m of road, then beyond the window."),
        });
        let (_, mut pilot, orders) = stepping(ROOMY_MUSTER, &brief);
        assert!(matches!(pilot.opening, OpeningStage::Hold { .. }));
        let notes = pilot.take_notes();
        assert!(
            notes.iter().all(|note| !note.contains("stepping")),
            "{notes:?}"
        );
        assert_eq!(orders.speed, Some(0));
    }

    #[test]
    fn the_opening_ends_on_a_hostile_a_hit_or_its_time() {
        // (a) A hostile in sight: the opening is over and the halt armed.
        let (mut picture, mut pilot) = holding(ROOMY_MUSTER);
        let t = POWER + 42_300;
        hold(
            &mut picture,
            row_at(ROOMY_MUSTER, "BA", Some(Side::Blue), 90, 550, t, 0),
        );
        let orders = decide_at(&picture, &mut pilot, t);
        assert_eq!(
            pilot.take_notes(),
            [
                "Opening over: BA is in sight after 42 s.",
                "Firing from the halt at BA, 550 m off."
            ]
        );
        assert_eq!(pilot.opening, OpeningStage::Done);
        assert_eq!(
            pilot.halt,
            Some(Halt {
                d: "BA".into(),
                since_ms: t,
                nearest_m: 550,
                nearest_ms: t,
            })
        );
        assert_eq!((orders.heading, orders.speed), (Some(90), Some(0)));
        assert_eq!(pilot.rule(), Some("halt"));
        // (b) A hit taken: over, no halt.
        let (mut picture, mut pilot) = holding(ROOMY_MUSTER);
        hit_taken(&mut picture, POWER + 20_000);
        decide_at(&picture, &mut pilot, POWER + 20_100);
        assert_eq!(pilot.take_notes(), ["Opening over: hit after 20 s."]);
        assert_eq!(pilot.opening, OpeningStage::Done);
        assert_eq!(pilot.halt, None);
        // (c) Its time: over, and the search.
        let (picture, mut pilot) = holding(ROOMY_MUSTER);
        let orders = decide_at(&picture, &mut pilot, POWER + OPENING_MAX_MS - 1);
        assert!(!orders.ask_spot_b, "still holding");
        let orders = decide_at(&picture, &mut pilot, POWER + OPENING_MAX_MS);
        assert_eq!(
            pilot.take_notes(),
            [format!(
                "Opening over: nothing came in {} s; searching.",
                OPENING_MAX_MS / 1_000
            )]
        );
        assert_eq!(pilot.halt, None);
        assert!(orders.ask_spot_b, "the search");
        assert_eq!(pilot.rule(), None);
        // (d) A hostile row past the stock sight is a teammate's report: the
        // opening is over with its note, and no halt.
        let (mut picture, mut pilot) = holding(ROOMY_MUSTER);
        let t = POWER + 30_000;
        hold(
            &mut picture,
            row_at(ROOMY_MUSTER, "BA", Some(Side::Blue), 90, 1_400, t, 0),
        );
        decide_at(&picture, &mut pilot, t);
        assert_eq!(
            pilot.take_notes(),
            ["Opening over: BA is in sight after 30 s."]
        );
        assert_eq!(pilot.opening, OpeningStage::Done);
        assert_eq!(pilot.halt, None);
        assert_eq!(pilot.rule(), None, "the stance flies as before");
    }

    #[test]
    fn no_opening_without_a_muster_a_skirmish_or_a_clear_start() {
        let none_said = |pilot: &mut Pilot, picture: &Picture, from: u64| {
            for t in [from, from + 1_000, from + 6_000, from + 20_000] {
                let orders = decide_at(picture, pilot, t);
                assert!(!orders.ask_view_brief, "{t}");
            }
            assert_eq!(pilot.opening, OpeningStage::Done);
            let notes = pilot.take_notes();
            assert!(
                notes.iter().all(|note| !note.starts_with("Opening")),
                "{notes:?}"
            );
        };
        // Power from `Power is already online.`: a running match joined.
        let mut picture = running();
        picture.apply(POWER, &classify("Power is already online."));
        status_at(&mut picture, MUSTER, 90, POWER + 100);
        picture.apply(POWER + 100, &classify("No contacts."));
        let mut pilot = Pilot::with_knobs(Knobs::on());
        pilot.ceilings_asked = true;
        none_said(&mut pilot, &picture, POWER + 150);
        assert_eq!(pilot.muster_m, None);
        // The start read 6 s after power: no muster.
        let mut picture = running();
        picture.apply(POWER, &classify("Power online."));
        let mut pilot = Pilot::with_knobs(Knobs::on());
        pilot.ceilings_asked = true;
        decide_at(&picture, &mut pilot, POWER + 50);
        picture.apply(POWER + 100, &classify("No contacts."));
        let orders = decide_at(&picture, &mut pilot, POWER + 3_000);
        assert!(!orders.ask_view_brief, "the muster is not known yet");
        status_at(&mut picture, MUSTER, 90, POWER + 6_000);
        none_said(&mut pilot, &picture, POWER + 6_050);
        assert_eq!(pilot.start_pos, Some(MUSTER));
        assert_eq!(pilot.muster_m, None);
        // A hold match and a range.
        for mode in [
            "  mode: hold — keep The Crossroads (B) uncontested for 120 s",
            "  mode: range — 2 targets on the blue side",
        ] {
            let mut picture = running();
            picture.apply(POWER - 5_000, &classify("Simulator centre: selection"));
            picture.apply(POWER - 5_000, &classify(mode));
            assert_ne!(picture.match_mode, MatchMode::Skirmish);
            picture.apply(POWER, &classify("Power online."));
            status_at(&mut picture, MUSTER, 90, POWER + 100);
            picture.apply(POWER + 100, &classify("No contacts."));
            let mut pilot = Pilot::with_knobs(Knobs::on());
            pilot.ceilings_asked = true;
            none_said(&mut pilot, &picture, POWER + 150);
        }
        // The posed start: the first `CONTACTS` after power holds an unknown
        // X1 at 500 m, identified as BA a second later.
        let mut picture = running();
        picture.apply(POWER, &classify("Power online."));
        let mut pilot = Pilot::with_knobs(Knobs::on());
        pilot.ceilings_asked = true;
        decide_at(&picture, &mut pilot, POWER + 50);
        status_at(&mut picture, MUSTER, 90, POWER + 100);
        picture.apply(
            POWER + 100,
            &classify("X1  unknown  bearing 090° (rel 000°)  far  500 m  0 s"),
        );
        picture.settle(POWER + 300);
        decide_at(&picture, &mut pilot, POWER + 300);
        assert_eq!(pilot.opening, OpeningStage::Done);
        picture.apply(POWER + 1_300, &classify("Contact X1 identified: BA, blue."));
        picture.apply(
            POWER + 1_400,
            &classify("BA  blue  bearing 090° (rel 000°)  far  500 m  0 s"),
        );
        picture.settle(POWER + 1_600);
        let orders = decide_at(&picture, &mut pilot, POWER + 1_600);
        assert!(!orders.ask_view_brief);
        assert_eq!(pilot.halt, None);
        let notes = pilot.take_notes();
        assert!(
            notes
                .iter()
                .all(|note| !note.starts_with("Opening") && !note.starts_with("Firing from")),
            "{notes:?}"
        );
    }

    #[test]
    fn an_untold_return_does_not_end_the_opening() {
        let (mut picture, mut pilot) = holding(ROOMY_MUSTER);
        let t = POWER + 20_000;
        hold(
            &mut picture,
            row_at(ROOMY_MUSTER, "X1", None, 45, 500, t, 0),
        );
        let orders = decide_at(&picture, &mut pilot, t);
        // Step 4a faces it, stopped.
        assert_eq!((orders.heading, orders.speed), (Some(45), Some(0)));
        assert_eq!(
            pilot.take_notes(),
            ["Stopping for X1: facing 045° while it is told apart."]
        );
        assert!(matches!(pilot.opening, OpeningStage::Hold { .. }));
        // Lost untold: the opening holds on, the bow back toward the other
        // entry.
        picture.contacts.clear();
        let orders = decide_at(&picture, &mut pilot, t + 2_000);
        assert!(matches!(pilot.opening, OpeningStage::Hold { .. }));
        assert_eq!(orders.heading, Some(90));
        assert_eq!(pilot.rule(), Some("opening"));
        assert!(pilot.take_notes().is_empty());
    }

    /// The opening held, then BA sighted `range` m east on a closing track,
    /// locked, with a ready cannon.
    fn sighted_closing(range: u32, t: u64) -> (Picture, Pilot) {
        let (mut picture, pilot) = holding(ROOMY_MUSTER);
        let mut ba = row_at(ROOMY_MUSTER, "BA", Some(Side::Blue), 90, range, t, 0);
        ba.track = Some(Track {
            course_deg: 270.0,
            speed_mps: 8.0,
        });
        hold(&mut picture, ba);
        picture.lock = Some("BA".into());
        let mut cannon = weapon("cannon", true);
        cannon.read_ms = t - 100;
        picture.weapons = vec![cannon];
        (picture, pilot)
    }

    #[test]
    fn the_halt_holds_still_beyond_the_band() {
        let t = POWER + 50_000;
        let (picture, mut pilot) = sighted_closing(590, t);
        let orders = decide_at(&picture, &mut pilot, t);
        assert_eq!(orders.speed, Some(0), "stopped");
        assert_eq!(orders.heading, Some(90), "the bow on it");
        assert_eq!(orders.fire, ["cannon"]);
        assert_eq!(pilot.rule(), Some("halt"));
        // The same picture flown by the pilot as it was: the hold closes on
        // it (590 m is past the band, its slack and its hysteresis).
        let mut old = Pilot::new();
        old.power_ms = Some(POWER);
        old.ceilings_asked = true;
        let orders = old.decide(&picture, &hold_doctrine(), t);
        assert!(orders.speed.is_some_and(|kph| kph > 0), "{orders:?}");
        assert_eq!(orders.fire, ["cannon"]);
    }

    #[test]
    fn the_halt_ends_when_it_stops_finds_us_or_reaches_the_band() {
        let t = POWER + 50_000;
        // Found: a hit taken.
        let (mut picture, mut pilot) = sighted_closing(590, t);
        decide_at(&picture, &mut pilot, t);
        pilot.take_notes();
        hit_taken(&mut picture, t + 1_000);
        decide_at(&picture, &mut pilot, t + 1_100);
        assert_eq!(pilot.take_notes(), ["The halt is over: we are found."]);
        assert_eq!(pilot.halt, None);
        // At the band: inside the band and its slack (500 m).
        let (mut picture, mut pilot) = sighted_closing(590, t);
        decide_at(&picture, &mut pilot, t);
        pilot.take_notes();
        let mut ba = row_at(ROOMY_MUSTER, "BA", Some(Side::Blue), 90, 500, t + 2_000, 0);
        ba.track = Some(Track {
            course_deg: 270.0,
            speed_mps: 8.0,
        });
        hold(&mut picture, ba);
        let orders = decide_at(&picture, &mut pilot, t + 2_000);
        assert_eq!(pilot.take_notes(), ["The halt is over: BA is at the band."]);
        assert_eq!(pilot.halt, None);
        assert_eq!(pilot.rule(), None, "the stance flies");
        assert_eq!(orders.speed, Some(0), "the hold, at the band");
        // Stopped: no shorter range and no track for the halt's still time.
        let (mut picture, mut pilot) = sighted_closing(590, t);
        decide_at(&picture, &mut pilot, t);
        pilot.take_notes();
        let still = row_at(
            ROOMY_MUSTER,
            "BA",
            Some(Side::Blue),
            90,
            590,
            t + HALT_STILL_MS,
            0,
        );
        hold(&mut picture, still.clone());
        decide_at(&picture, &mut pilot, t + HALT_STILL_MS - 1);
        assert!(pilot.halt.is_some(), "not yet");
        decide_at(&picture, &mut pilot, t + HALT_STILL_MS);
        assert_eq!(pilot.take_notes(), ["The halt is over: BA has stopped."]);
        assert_eq!(pilot.halt, None);
        // Coming on with no track published (a mover the scope's 50 m rows
        // cannot fit): each nearer row restarts the still time, so the halt
        // holds while it closes, and ends once a whole still time passes with
        // no nearer row.
        let (mut picture, mut pilot) = sighted_closing(590, t);
        decide_at(&picture, &mut pilot, t);
        pilot.take_notes();
        let nearer = t + HALT_STILL_MS / 2;
        hold(
            &mut picture,
            row_at(ROOMY_MUSTER, "BA", Some(Side::Blue), 90, 550, nearer, 0),
        );
        for at in [nearer, t + HALT_STILL_MS, nearer + HALT_STILL_MS - 1] {
            let orders = decide_at(&picture, &mut pilot, at);
            assert!(pilot.halt.is_some(), "closing at {at}");
            assert!(pilot.take_notes().is_empty(), "at {at}");
            assert!(orders.speed.is_none_or(|kph| kph <= 0), "at {at}");
        }
        assert_eq!(pilot.halt.as_ref().map(|h| h.nearest_m), Some(550));
        decide_at(&picture, &mut pilot, nearer + HALT_STILL_MS);
        assert_eq!(pilot.take_notes(), ["The halt is over: BA has stopped."]);
        assert_eq!(pilot.halt, None);
        // Its time: still coming on (a track) after a minute.
        let (mut picture, mut pilot) = sighted_closing(590, t);
        decide_at(&picture, &mut pilot, t);
        pilot.take_notes();
        let mut slow = row_at(
            ROOMY_MUSTER,
            "BA",
            Some(Side::Blue),
            90,
            560,
            t + HALT_MAX_MS,
            0,
        );
        slow.track = Some(Track {
            course_deg: 270.0,
            speed_mps: 3.0,
        });
        hold(&mut picture, slow);
        decide_at(&picture, &mut pilot, t + HALT_MAX_MS);
        assert_eq!(pilot.take_notes(), ["The halt is over: its time is up."]);
        // Lost: over, nothing said.
        let (mut picture, mut pilot) = sighted_closing(590, t);
        decide_at(&picture, &mut pilot, t);
        pilot.take_notes();
        picture.contacts.clear();
        decide_at(&picture, &mut pilot, t + 1_000);
        assert_eq!(pilot.halt, None);
        assert!(
            pilot
                .take_notes()
                .iter()
                .all(|note| !note.starts_with("The halt"))
        );
    }

    /// A tank at `MID` heading east, with the re-approach on and the opening
    /// over (no `Power online.` of its own read).
    fn reapproaching() -> (Picture, Pilot) {
        let (mut picture, _) = flying_at(MID);
        picture.own.heading_deg = 90;
        let mut pilot = Pilot::with_knobs(Knobs::on());
        pilot.power_ms = Some(NOW - 30_000);
        pilot.ceilings_asked = true;
        (picture, pilot)
    }

    /// `taken` hits taken and `scored` hits scored on BA, all in the second
    /// before `t`.
    fn exchange(picture: &mut Picture, taken: usize, scored: usize, t: u64) {
        for i in 0..taken {
            hit_taken(picture, t - 1_000 + i as u64);
        }
        for i in 0..scored {
            picture.events.push_back(Event {
                at_ms: t - 900 + i as u64,
                kind: EventKind::HitScored,
                text: "Hit on BA: turret.".into(),
            });
        }
    }

    /// An exchange that out-hits the pilot, read through the tunable names
    /// (two over the floor taken — at least the margin — and the margin fewer
    /// scored)…
    const OUT_TAKEN: usize = max_usize(EXCHANGE_MIN_TAKEN, EXCHANGE_MARGIN) + 2;
    const OUT_SCORED: usize = OUT_TAKEN - EXCHANGE_MARGIN;
    /// …and one that does not: the floor taken, but one short of the margin.
    const EVEN_TAKEN: usize = max_usize(EXCHANGE_MIN_TAKEN, EXCHANGE_MARGIN);
    const EVEN_SCORED: usize = EVEN_TAKEN + 1 - EXCHANGE_MARGIN;

    const fn max_usize(a: usize, b: usize) -> usize {
        if a > b { a } else { b }
    }

    /// The re-approach's opening note for the out-hitting exchange.
    fn opening_the_range_note() -> String {
        format!(
            "Opening the range on BA: {OUT_TAKEN} hits taken to {OUT_SCORED} scored in {} s; \
             backing off bow on.",
            EXCHANGE_WINDOW_MS / 1_000
        )
    }

    /// The re-approach wait's end note on BA.
    fn waited_note() -> String {
        format!(
            "No approach from BA in {} s; moving on.",
            REAPPROACH_WAIT_MS / 1_000
        )
    }

    /// BA, a stopped machine at the world point 420 m east of `MID`, as a row
    /// read from `own` at `t`, its plot `age_ms` old.
    fn ba_from(own: (f64, f64), t: u64, age_ms: u64) -> Contact {
        let ba = along(MID, 90.0, 420.0);
        let range = distance_m(own, ba).round() as u32;
        let bearing = whole(bearing_deg(own, ba));
        row_at(own, "BA", Some(Side::Blue), bearing, range, t, age_ms)
    }

    #[test]
    fn the_reapproach_backs_off_bow_on_past_the_sight_and_waits() {
        let (mut picture, mut pilot) = reapproaching();
        let t = NOW;
        hold(&mut picture, ba_from(MID, t, 0));
        exchange(&mut picture, OUT_TAKEN, OUT_SCORED, t);
        let orders = pilot.decide(&picture, &hold_doctrine(), t);
        assert_eq!(pilot.take_notes(), [opening_the_range_note()]);
        assert_eq!((orders.heading, orders.speed), (Some(90), Some(-27)));
        assert_eq!(pilot.rule(), Some("reapproach"));
        pilot.sent_heading(90, t);
        pilot.sent_speed(-27);
        let plot = pilot.reapproach.as_ref().expect("in force").plot_m;
        assert!(distance_m(plot, along(MID, 90.0, 420.0)) < 1.0);
        // Out of its sight at 480 m: its row stale, still reversing, on the
        // plot.
        let own = along(MID, 270.0, 60.0);
        status_at(&mut picture, own, 90, t + 8_000);
        hold(
            &mut picture,
            ba_from(own, t + 1_000, HUNT_STALE_PLOT_MS + 1),
        );
        let orders = pilot.decide(&picture, &hold_doctrine(), t + 8_000);
        assert_eq!(
            (orders.heading, orders.speed),
            (None, None),
            "still reversing, on 090"
        );
        assert_eq!(pilot.last_chosen_speed, Some(-27));
        assert!(matches!(
            pilot.reapproach.as_ref().map(|r| r.stage),
            Some(ReapproachStage::Open { .. })
        ));
        assert_eq!(pilot.rule(), Some("reapproach"));
        // `REAPPROACH_OPEN_M` from the plot: stopped, facing it, waiting.
        let own = along(MID, 270.0, REAPPROACH_OPEN_M - 420.0);
        status_at(&mut picture, own, 90, t + 30_000);
        hold(&mut picture, ba_from(own, t + 1_000, 30_000));
        let orders = pilot.decide(&picture, &hold_doctrine(), t + 30_000);
        assert_eq!((orders.heading, orders.speed), (None, Some(0)));
        assert_eq!(
            pilot.take_notes(),
            [format!(
                "Out of BA's sight at {REAPPROACH_OPEN_M:.0} m from its last plot; waiting for it to come on."
            )]
        );
        assert_eq!(pilot.rule(), Some("approach_wait"));
        // Waiting: no hunt.
        for dt in [31_000, 40_000, 60_000] {
            pilot.decide(&picture, &hold_doctrine(), t + dt);
            assert_eq!(pilot.hunt, None);
            assert!(pilot.take_notes().is_empty());
        }
        // A teammate's report from past the stock sight (BB at 1 400 m, held
        // young) is no machine coming on: no halt, still waiting.
        hold(
            &mut picture,
            row_at(own, "BB", Some(Side::Blue), 90, 1_400, t + 60_500, 0),
        );
        let orders = pilot.decide(&picture, &hold_doctrine(), t + 60_500);
        assert_eq!(pilot.halt, None);
        assert!(pilot.take_notes().is_empty());
        assert!(matches!(
            pilot.reapproach.as_ref().map(|r| r.stage),
            Some(ReapproachStage::Wait { .. })
        ));
        assert_eq!(pilot.rule(), Some("approach_wait"));
        assert!(orders.speed.is_none_or(|kph| kph == 0), "{orders:?}");
        // A fresh BA: the halt armed for it, the wait over.
        let ba = row_at(own, "BA", Some(Side::Blue), 90, 600, t + 61_000, 0);
        hold(&mut picture, ba);
        let orders = pilot.decide(&picture, &hold_doctrine(), t + 61_000);
        assert_eq!(
            pilot.take_notes(),
            ["Firing from the halt at BA, 600 m off."]
        );
        assert_eq!(pilot.reapproach, None);
        assert_eq!(
            pilot.reapproach_rest_until_ms,
            t + 61_000 + REAPPROACH_REST_MS
        );
        assert_eq!(pilot.halt.as_ref().map(|h| h.d.as_str()), Some("BA"));
        assert_eq!((orders.heading, orders.speed), (None, Some(0)));
        assert_eq!(pilot.rule(), Some("halt"));
    }

    #[test]
    fn no_reapproach_on_an_even_exchange_a_mover_or_an_orbit() {
        // The floor taken, but one short of the margin over what was scored.
        let (mut picture, mut pilot) = reapproaching();
        hold(&mut picture, ba_from(MID, NOW, 0));
        exchange(&mut picture, EVEN_TAKEN, EVEN_SCORED, NOW);
        pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!(pilot.reapproach, None);
        // Out-hit, but by a mover.
        let (mut picture, mut pilot) = reapproaching();
        let mut ba = ba_from(MID, NOW, 0);
        ba.track = Some(Track {
            course_deg: 270.0,
            speed_mps: 6.0,
        });
        hold(&mut picture, ba);
        exchange(&mut picture, OUT_TAKEN, OUT_SCORED, NOW);
        pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!(pilot.reapproach, None);
        // Out-hit by a stopped machine, under an orbit.
        let (mut picture, mut pilot) = reapproaching();
        hold(&mut picture, ba_from(MID, NOW, 0));
        exchange(&mut picture, OUT_TAKEN, OUT_SCORED, NOW);
        pilot.decide(&picture, &doctrine(), NOW);
        assert_eq!(pilot.reapproach, None);
        // …and with the knob off.
        let (mut picture, _) = reapproaching();
        let mut pilot = Pilot::with_knobs(Knobs {
            reapproach: false,
            ..Knobs::on()
        });
        pilot.power_ms = Some(NOW - 30_000);
        hold(&mut picture, ba_from(MID, NOW, 0));
        exchange(&mut picture, OUT_TAKEN, OUT_SCORED, NOW);
        pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!(pilot.reapproach, None);
        // The same, out-hit by a stopped machine under the hold: it begins.
        let (mut picture, mut pilot) = reapproaching();
        hold(&mut picture, ba_from(MID, NOW, 0));
        exchange(&mut picture, OUT_TAKEN, OUT_SCORED, NOW);
        pilot.decide(&picture, &hold_doctrine(), NOW);
        assert!(pilot.reapproach.is_some());
    }

    #[test]
    fn the_reapproach_gives_up_rests_and_counts_per_match() {
        let (mut picture, mut pilot) = reapproaching();
        let t = NOW;
        let begin = |picture: &mut Picture, pilot: &mut Pilot, at: u64| {
            hold(picture, ba_from(MID, at, 0));
            exchange(picture, OUT_TAKEN, OUT_SCORED, at);
            pilot.decide(picture, &hold_doctrine(), at);
        };
        begin(&mut picture, &mut pilot, t);
        assert!(pilot.reapproach.is_some());
        assert_eq!(pilot.reapproaches, 1);
        pilot.take_notes();
        // It never gets out (held where it is): given up at its bound.
        pilot.decide(&picture, &hold_doctrine(), t + REAPPROACH_MAX_MS - 1);
        assert!(pilot.reapproach.is_some());
        pilot.decide(&picture, &hold_doctrine(), t + REAPPROACH_MAX_MS);
        assert_eq!(
            pilot.take_notes(),
            [format!(
                "Could not open the range on BA in {} s; holding.",
                REAPPROACH_MAX_MS / 1_000
            )]
        );
        assert_eq!(pilot.reapproach, None);
        let rested = t + REAPPROACH_MAX_MS + REAPPROACH_REST_MS;
        assert_eq!(pilot.reapproach_rest_until_ms, rested);
        // None inside the rest.
        begin(&mut picture, &mut pilot, rested - 1);
        assert_eq!(pilot.reapproach, None);
        assert_eq!(pilot.reapproaches, 1);
        // After it, the second (on the same exchange, a millisecond old and
        // still inside its window: a second batch a millisecond later would
        // sit among the first, older than the events already taken).
        pilot.decide(&picture, &hold_doctrine(), rested);
        assert!(pilot.reapproach.is_some());
        assert_eq!(pilot.reapproaches, REAPPROACH_PER_MATCH);
        pilot.decide(&picture, &hold_doctrine(), rested + REAPPROACH_MAX_MS);
        assert_eq!(pilot.reapproach, None);
        // A third is refused at the match's cap.
        let later = rested + REAPPROACH_MAX_MS + REAPPROACH_REST_MS + 1;
        begin(&mut picture, &mut pilot, later);
        assert_eq!(pilot.reapproach, None);
        assert_eq!(pilot.reapproaches, REAPPROACH_PER_MATCH);
    }

    #[test]
    fn the_reapproach_wait_ends_into_what_the_precedence_flies() {
        // `out_ms` after it began the tank is `REAPPROACH_OPEN_M` from the plot, BA lost
        // then; its plot was `age_ms` old when it began.
        let waited = |age_ms: u64, out_ms: u64| {
            let (mut picture, mut pilot) = reapproaching();
            let t = NOW;
            hold(&mut picture, ba_from(MID, t, age_ms));
            exchange(&mut picture, OUT_TAKEN, OUT_SCORED, t);
            pilot.decide(&picture, &hold_doctrine(), t);
            assert!(pilot.reapproach.is_some());
            picture.apply(t + 500, &classify("Contact BA lost."));
            let own = along(MID, 270.0, REAPPROACH_OPEN_M - 420.0);
            status_at(&mut picture, own, 90, t + out_ms);
            pilot.decide(&picture, &hold_doctrine(), t + out_ms);
            assert!(matches!(
                pilot.reapproach.as_ref().map(|r| r.stage),
                Some(ReapproachStage::Wait { .. })
            ));
            pilot.take_notes();
            let end = t + out_ms + REAPPROACH_WAIT_MS;
            pilot.decide(&picture, &hold_doctrine(), end - 1);
            assert!(pilot.reapproach.is_some());
            let orders = pilot.decide(&picture, &hold_doctrine(), end);
            (pilot, orders, end)
        };
        // The trail's plot past `HUNT_TRAIL_MS` at the wait's end: no hunt,
        // the search.
        let (mut pilot, orders, end) = waited(FRESH_PLOT_MS, REAPPROACH_MAX_MS - 2_000);
        let trail = pilot.trail.as_ref().expect("a trail").plot_ms;
        assert!(end - trail > HUNT_TRAIL_MS, "{}", end - trail);
        let notes = pilot.take_notes();
        assert_eq!(notes, [waited_note()]);
        assert_eq!(pilot.hunt, None);
        assert!(orders.ask_spot_b, "the search");
        // Younger: the hunt.
        let (mut pilot, _, end) = waited(0, 1_000);
        let trail = pilot.trail.as_ref().expect("a trail").plot_ms;
        assert!(end - trail <= HUNT_TRAIL_MS);
        let notes = pilot.take_notes();
        assert_eq!(
            notes.first().map(String::as_str),
            Some(waited_note().as_str())
        );
        assert!(
            notes
                .iter()
                .any(|note| note.starts_with("Hunting BA: its last plot")),
            "{notes:?}"
        );
        assert!(pilot.hunt.is_some());
    }

    #[test]
    fn the_exchange_counts_with_the_reapproach_off() {
        let (mut picture, _) = reapproaching();
        let mut pilot = Pilot::with_knobs(Knobs {
            reapproach: false,
            ..Knobs::on()
        });
        pilot.power_ms = Some(NOW - 30_000);
        hold(&mut picture, ba_from(MID, NOW, 0));
        exchange(&mut picture, OUT_TAKEN, OUT_SCORED, NOW);
        // A friendly hit and an unnamed one count for nothing; an event from
        // before the power is not this match's.
        for (at, text) in [
            (NOW - 500, "Hit on RB (FRIENDLY): turret."),
            (NOW - 400, "Hit."),
            (NOW - 40_000, "Hit on BA: hull."),
        ] {
            picture.events.push_back(Event {
                at_ms: at,
                kind: EventKind::HitScored,
                text: text.into(),
            });
        }
        pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!(pilot.exchange_counts(NOW), (OUT_TAKEN, OUT_SCORED));
        assert_eq!(pilot.reapproach, None);
        // Past the window, nothing.
        let inside = NOW + EXCHANGE_WINDOW_MS - 1_000;
        pilot.decide(&picture, &hold_doctrine(), inside);
        assert_eq!(pilot.exchange_counts(inside), (OUT_TAKEN, OUT_SCORED));
        pilot.decide(&picture, &hold_doctrine(), NOW + EXCHANGE_WINDOW_MS + 1_001);
        assert_eq!(
            pilot.exchange_counts(NOW + EXCHANGE_WINDOW_MS + 1_001),
            (0, 0)
        );
        assert!(pilot.exchange.is_empty(), "pruned");
    }

    #[test]
    fn with_the_knobs_off_the_pilot_is_as_it_was() {
        let mut picture = running();
        picture.apply(POWER, &classify("Power online."));
        let mut old = Pilot::new();
        let mut off = Pilot::with_knobs(Knobs::off());
        fn both(old: &mut Pilot, off: &mut Pilot, picture: &Picture, t: u64) -> Orders {
            let a = old.decide(picture, &hold_doctrine(), t);
            let b = off.decide(picture, &hold_doctrine(), t);
            assert_eq!(a, b, "at {t}");
            assert!(!a.ask_view_brief);
            assert_eq!(old.take_notes(), off.take_notes(), "at {t}");
            assert_eq!(old.rule(), None);
            assert_eq!(off.rule(), None);
            a
        }
        both(&mut old, &mut off, &picture, POWER + 50);
        status_at(&mut picture, MUSTER, 90, POWER + 100);
        picture.apply(POWER + 100, &classify("No contacts."));
        both(&mut old, &mut off, &picture, POWER + 150);
        // Five quiet seconds: the search.
        picture.spots.insert(
            'B',
            crate::arena::picture::Spot {
                name: "The Crossroads".into(),
                range_m: Some(1_000),
                bearing_deg: Some(90),
                read_ms: POWER + 100,
                plotted_m: Some((1_060, 970)),
            },
        );
        let searching = both(&mut old, &mut off, &picture, POWER + 5_050);
        assert!(searching.ask_spot_b);
        assert_eq!(searching.heading, Some(90));
        // A young hostile, then lost and hunted.
        let t = POWER + 20_000;
        hold(
            &mut picture,
            row_at(MUSTER, "BA", Some(Side::Blue), 90, 550, t, 0),
        );
        both(&mut old, &mut off, &picture, t);
        picture.contacts.clear();
        both(&mut old, &mut off, &picture, t + 1_000);
        assert!(old.hunt.is_some() && off.hunt.is_some());
        for dt in [2_000, 10_000, 30_000] {
            both(&mut old, &mut off, &picture, t + dt);
        }
    }

    #[test]
    fn the_new_rules_name_only_held_designators() {
        // Every designator a note of the opening or the halt names is one the
        // picture held when it was said; every one a note of the re-approach
        // names, one the stream named this match.
        let check = |pilot: &mut Pilot, picture: &Picture| {
            for note in pilot.take_notes() {
                for word in note.split(|c: char| !c.is_ascii_alphanumeric()) {
                    if crate::arena::lines::is_designator(word) {
                        assert!(picture.contacts.contains_key(word), "{word} in {note}");
                    }
                }
            }
            assert!(pilot.rule().is_none_or(|rule| {
                ["opening", "halt", "reapproach", "approach_wait"].contains(&rule)
            }));
        };
        // The opening, its end on a hostile and the halt.
        let (mut picture, mut pilot) = holding(ROOMY_MUSTER);
        let t = POWER + 40_000;
        hold(
            &mut picture,
            row_at(ROOMY_MUSTER, "BA", Some(Side::Blue), 90, 580, t, 0),
        );
        decide_at(&picture, &mut pilot, t);
        check(&mut pilot, &picture);
        hit_taken(&mut picture, t + 500);
        decide_at(&picture, &mut pilot, t + 600);
        check(&mut pilot, &picture);
        // The re-approach through the stream, the path the lane flies: BA's
        // row, the exchange, `Contact BA lost.` out of its sight, the wait
        // and its end. BA is no longer held when the later notes are said, so
        // each designator is checked against what the stream named this
        // match (the waits' rule above).
        fn names(line: &crate::arena::Line) -> Vec<String> {
            match line {
                crate::arena::Line::ContactRow(row) => vec![row.designator.clone()],
                crate::arena::Line::NewContact { designator, .. }
                | crate::arena::Line::ContactLost { designator } => vec![designator.clone()],
                crate::arena::Line::Identified {
                    was, designator, ..
                } => vec![was.clone(), designator.clone()],
                _ => Vec::new(),
            }
        }
        let (mut picture, mut pilot) = reapproaching();
        let mut named = BTreeSet::new();
        let mut notes = Vec::new();
        let mut read = |picture: &mut Picture, at: u64, text: &str| {
            let line = classify(text);
            named.extend(names(&line));
            picture.apply(at, &line);
            picture.settle(at + 200);
        };
        read(
            &mut picture,
            NOW - 400,
            "BA  blue  bearing 090° (rel 000°)  far  400 m  0 s",
        );
        exchange(&mut picture, OUT_TAKEN, OUT_SCORED, NOW);
        pilot.decide(&picture, &hold_doctrine(), NOW);
        notes.extend(pilot.take_notes());
        read(&mut picture, NOW + 500, "Contact BA lost.");
        pilot.decide(&picture, &hold_doctrine(), NOW + 1_000);
        notes.extend(pilot.take_notes());
        let own = along(MID, 270.0, REAPPROACH_OPEN_M - 390.0);
        status_at(&mut picture, own, 90, NOW + 30_000);
        pilot.decide(&picture, &hold_doctrine(), NOW + 30_000);
        notes.extend(pilot.take_notes());
        assert!(!picture.contacts.contains_key("BA"), "BA is not held");
        pilot.decide(
            &picture,
            &hold_doctrine(),
            NOW + 30_000 + REAPPROACH_WAIT_MS,
        );
        notes.extend(pilot.take_notes());
        for start in [
            "Opening the range on BA: ",
            "Out of BA's sight at ",
            "No approach from BA in ",
        ] {
            assert!(
                notes.iter().any(|note| note.starts_with(start)),
                "{start}: {notes:?}"
            );
        }
        for note in &notes {
            for word in note.split(|c: char| !c.is_ascii_alphanumeric()) {
                if crate::arena::lines::is_designator(word) {
                    assert!(named.contains(word), "{word} in {note}");
                }
            }
        }
        // The exchange holds times and sides of a hit, never a name; the
        // rule is a word.
        let json = serde_json::to_value(&pilot.exchange).expect("json");
        assert!(!json.to_string().contains("BA"));
    }

    // -- cut AN.3: the broken machine ---------------------------------------------

    /// **A red tank at `own` heading east, flying [`Knobs::on`], from the
    /// shipped muster** ([`MUSTER`], so the other side's entry is (2 500,
    /// 970) and its entry line `x = 2 500`), the opening over, BA locked and
    /// a ready cannon.
    fn following_at(own: (f64, f64)) -> (Picture, Pilot) {
        let (mut picture, _) = flying_at(own);
        picture.own.heading_deg = 90;
        picture.lock = Some("BA".into());
        picture.weapons = vec![weapon("cannon", true)];
        let mut pilot = Pilot::with_knobs(Knobs::on());
        pilot.power_ms = Some(NOW - 60_000);
        pilot.ceilings_asked = true;
        pilot.start_pos = Some(MUSTER);
        pilot.muster_m = Some(MUSTER);
        pilot.opening = OpeningStage::Done;
        (picture, pilot)
    }

    /// The other side's entry line from [`MUSTER`] on the help's ground.
    const ENTRY_LINE_X: f64 = ARENA_SIDE_M - 60.0;

    /// BA `range` m due east of `own`, read at `t`, driving `course_deg` at
    /// 8 m/s (none: no track published).
    fn ba_east(own: (f64, f64), range: u32, t: u64, course_deg: Option<f64>) -> Contact {
        let mut ba = row_at(own, "BA", Some(Side::Blue), 90, range, t, 0);
        ba.track = course_deg.map(|course_deg| Track {
            course_deg,
            speed_mps: 8.0,
        });
        ba
    }

    /// `n` hits scored on `d` in the second before `t`.
    fn scored(picture: &mut Picture, d: &str, n: u32, t: u64) {
        for i in 0..u64::from(n) {
            picture.events.push_back(Event {
                at_ms: t - 1_000 + i,
                kind: EventKind::HitScored,
                text: format!("Hit on {d}: turret."),
            });
        }
    }

    /// The follow's note on `d`.
    fn following_note(d: &str) -> String {
        format!(
            "Following {d} home: it is hurt and driving away; holding {:.0} m behind it.",
            FOLLOW_BAND_M
        )
    }

    /// The keep-out's note on `d`.
    fn not_following_note(d: &str) -> String {
        format!("Not following {d} into its entry zone; backing off out of its sight.")
    }

    /// The wait's note on `d`, `m` metres from its last plot.
    fn waiting_note(d: &str, m: f64) -> String {
        format!("Waiting for {d} to come back, {m:.0} m from its last plot.")
    }

    /// No note so far says `Hunting `.
    fn no_hunt(notes: &[String]) {
        assert!(
            notes.iter().all(|note| !note.starts_with("Hunting ")),
            "{notes:?}"
        );
    }

    /// **A broken machine is followed from behind at the band** (cut AN.3): a
    /// hurt BA driving away (its course 20° off the bearing to it) is
    /// followed, said verbatim; the pilot closes ahead past the band's slack
    /// and hysteresis, stops inside its slack, keeps the bow on and fires —
    /// where the stance would stand at its own band.
    #[test]
    fn a_broken_machine_is_followed_from_behind_at_the_band() {
        let own = (1_000.0, 970.0);
        let far = (FOLLOW_BAND_M + HOLD_SLACK_M + HOLD_HYSTERESIS_M + 25.0) as u32;
        let (mut picture, mut pilot) = following_at(own);
        hold(&mut picture, ba_east(own, far, NOW, Some(110.0)));
        hurt_event(&mut picture, "BA", NOW - 100);
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!(pilot.take_notes(), [following_note("BA")]);
        assert_eq!(
            pilot.follow,
            Some(Follow {
                d: "BA".into(),
                since_ms: NOW,
                tracked_ms: NOW,
            })
        );
        assert_eq!(pilot.rule(), Some("follow"));
        assert_eq!(orders.heading, Some(90), "the bow on it");
        assert_eq!(orders.speed, Some(54), "closing ahead");
        assert_eq!(orders.fire, ["cannon"]);
        pilot.sent_heading(90, NOW);
        pilot.sent_speed(54);
        // The stance would stand: the band is the doctrine's, and `far` lies
        // inside its slack and hysteresis.
        let (mut stance_picture, _) = following_at(own);
        hold(&mut stance_picture, ba_east(own, far, NOW, Some(110.0)));
        let mut stance = Pilot::with_knobs(Knobs {
            follow_broken: false,
            ..Knobs::on()
        });
        stance.power_ms = Some(NOW - 60_000);
        stance.ceilings_asked = true;
        stance.opening = OpeningStage::Done;
        let held = stance.decide(&stance_picture, &hold_doctrine(), NOW);
        assert_eq!(held.speed, Some(0), "{held:?}");
        // Inside the band's slack: stopped, bow on, firing.
        let near = (FOLLOW_BAND_M + HOLD_SLACK_M - 10.0) as u32;
        let t = NOW + 2_000;
        hold(&mut picture, ba_east(own, near, t, Some(110.0)));
        let orders = pilot.decide(&picture, &hold_doctrine(), t);
        assert_eq!(orders.speed, Some(0), "{orders:?}");
        assert_eq!(orders.fire, ["cannon"]);
        assert_eq!(pilot.rule(), Some("follow"));
        assert!(pilot.take_notes().is_empty());
        assert_eq!(pilot.follow.as_ref().map(|f| f.tracked_ms), Some(t));
        // Off to one side, the bow comes round onto it.
        let mut ba = row_at(own, "BA", Some(Side::Blue), 60, near, t + 1_000, 0);
        ba.track = Some(Track {
            course_deg: 70.0,
            speed_mps: 8.0,
        });
        hold(&mut picture, ba);
        let orders = pilot.decide(&picture, &hold_doctrine(), t + 1_000);
        assert_eq!(orders.heading, Some(60));
        assert_eq!(pilot.rule(), Some("follow"));
    }

    /// **An unhurt machine driving off is not broken** (cut AN.3): one hit
    /// short of [`BROKEN_HITS_MIN`] and never hurt, the hold flies as it
    /// would with the follow off.
    #[test]
    fn an_unhurt_machine_driving_off_is_not_broken() {
        let own = (1_000.0, 970.0);
        let range = (FOLLOW_BAND_M + HOLD_SLACK_M + HOLD_HYSTERESIS_M + 25.0) as u32;
        let fly = |knobs: Knobs| {
            let (mut picture, mut pilot) = following_at(own);
            pilot.knobs = knobs;
            hold(&mut picture, ba_east(own, range, NOW, Some(90.0)));
            scored(&mut picture, "BA", BROKEN_HITS_MIN - 1, NOW);
            let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
            (pilot, orders)
        };
        let (mut on, orders_on) = fly(Knobs::on());
        let (mut off, orders_off) = fly(Knobs {
            follow_broken: false,
            ..Knobs::on()
        });
        assert_eq!(on.scored_on.get("BA"), Some(&(BROKEN_HITS_MIN - 1)));
        assert_eq!(on.follow, None);
        assert_eq!(on.rule(), None);
        assert_eq!(orders_on, orders_off);
        assert_eq!(on.take_notes(), off.take_notes());
        // A machine hurt but coming on (its course back at the tank) is not
        // broken either.
        let (mut picture, mut pilot) = following_at(own);
        hold(&mut picture, ba_east(own, range, NOW, Some(270.0)));
        hurt_event(&mut picture, "BA", NOW - 100);
        pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!(pilot.follow, None);
        // …nor one that publishes no track.
        let (mut picture, mut pilot) = following_at(own);
        hold(&mut picture, ba_east(own, range, NOW, None));
        hurt_event(&mut picture, "BA", NOW - 100);
        pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!(pilot.follow, None);
        // Within `BROKEN_AWAY_DEG` of the bearing it is; past it, not.
        for (course, broken) in [
            (90.0 + f64::from(BROKEN_AWAY_DEG), true),
            (90.0 + f64::from(BROKEN_AWAY_DEG) + 2.0, false),
        ] {
            let (mut picture, mut pilot) = following_at(own);
            hold(&mut picture, ba_east(own, range, NOW, Some(course)));
            hurt_event(&mut picture, "BA", NOW - 100);
            pilot.decide(&picture, &hold_doctrine(), NOW);
            assert_eq!(pilot.follow.is_some(), broken, "course {course}");
        }
    }

    /// **The follow stops short of the entry line and backs out of sight**
    /// (cut AN.3): by the machine within [`ENTRY_KEEP_OUT_M`] of the line, and
    /// by the own tank within that and [`FOLLOW_BAND_M`]; each says so, backs
    /// off in reverse bow on the plot, and stops at [`RETURN_STANDOFF_M`] from
    /// it with the wait's note — and no hunt starts at any point.
    #[test]
    fn the_follow_stops_short_of_the_entry_line_and_backs_out_of_sight() {
        // (own x, BA's range before, own x and BA's range at the keep-out).
        let by_target = {
            let own_x = ENTRY_LINE_X - ENTRY_KEEP_OUT_M - FOLLOW_BAND_M - 100.0;
            let before = (ENTRY_LINE_X - ENTRY_KEEP_OUT_M - own_x - 50.0) as u32;
            let after = (ENTRY_LINE_X - ENTRY_KEEP_OUT_M - own_x + 20.0) as u32;
            (own_x, before, own_x, after)
        };
        let by_own = {
            let own_x = ENTRY_LINE_X - ENTRY_KEEP_OUT_M - FOLLOW_BAND_M - 100.0;
            let later_x = ENTRY_LINE_X - ENTRY_KEEP_OUT_M - FOLLOW_BAND_M + 10.0;
            (own_x, 300, later_x, 300)
        };
        for (case, (own_x, before, later_x, after)) in
            [("the machine", by_target), ("the own tank", by_own)]
        {
            let own = (own_x, 970.0);
            let (mut picture, mut pilot) = following_at(own);
            hold(&mut picture, ba_east(own, before, NOW, Some(90.0)));
            hurt_event(&mut picture, "BA", NOW - 100);
            let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
            let mut notes = pilot.take_notes();
            assert_eq!(notes, [following_note("BA")], "{case}");
            assert_eq!(
                (orders.heading, orders.speed),
                (Some(90), Some(0)),
                "{case}"
            );
            pilot.sent_heading(90, NOW);
            pilot.sent_speed(0);
            // At the keep-out.
            let t = NOW + 5_000;
            let later = (later_x, 970.0);
            status_at(&mut picture, later, 90, t);
            let ba = ba_east(later, after, t, Some(90.0));
            let plot = ba.fixes.back().expect("a fix").pos_m;
            hold(&mut picture, ba);
            let orders = pilot.decide(&picture, &hold_doctrine(), t);
            let said = pilot.take_notes();
            assert_eq!(said, [not_following_note("BA")], "{case}");
            notes.extend(said);
            assert_eq!(pilot.follow, None);
            assert_eq!(
                pilot.ret,
                Some(Return {
                    d: "BA".into(),
                    plot_m: plot,
                    stage: ReturnStage::Back { since_ms: t },
                }),
                "{case}"
            );
            assert_eq!((orders.heading, orders.speed), (None, Some(-27)), "{case}");
            assert_eq!(pilot.rule(), Some("return_back"));
            pilot.sent_speed(-27);
            // BA lost; backing off on its plot, short of the standoff.
            picture.contacts.clear();
            picture.apply(t + 500, &classify("Contact BA lost."));
            let short = (plot.0 - RETURN_STANDOFF_M + 1.0, 970.0);
            status_at(&mut picture, short, 90, t + 20_000);
            let orders = pilot.decide(&picture, &hold_doctrine(), t + 20_000);
            assert_eq!(orders.speed, None, "still reversing: {orders:?}");
            assert_eq!(pilot.rule(), Some("return_back"));
            assert_eq!(pilot.hunt, None, "{case}");
            notes.extend(pilot.take_notes());
            // At the standoff: stopped, facing the plot, waiting.
            let out = (plot.0 - RETURN_STANDOFF_M, 970.0);
            status_at(&mut picture, out, 90, t + 22_000);
            let orders = pilot.decide(&picture, &hold_doctrine(), t + 22_000);
            let said = pilot.take_notes();
            assert_eq!(said, [waiting_note("BA", RETURN_STANDOFF_M)], "{case}");
            notes.extend(said);
            assert_eq!(orders.speed, Some(0));
            assert_eq!(orders.heading, None, "already on 090, facing the plot");
            assert_eq!(pilot.rule(), Some("return_wait"));
            assert!(matches!(
                pilot.ret.as_ref().map(|r| r.stage),
                Some(ReturnStage::Wait { since_ms }) if since_ms == t + 22_000
            ));
            for dt in [30_000, 60_000] {
                pilot.decide(&picture, &hold_doctrine(), t + dt);
                assert_eq!(pilot.hunt, None, "{case} at {dt}");
                notes.extend(pilot.take_notes());
            }
            no_hunt(&notes);
        }
        // The backing off gives up at its bound, and waits where it is.
        let own = (1_000.0, 970.0);
        let (mut picture, mut pilot) = following_at(own);
        pilot.ret = Some(Return {
            d: "BA".into(),
            plot_m: (1_300.0, 970.0),
            stage: ReturnStage::Back { since_ms: NOW },
        });
        pilot.decide(&picture, &hold_doctrine(), NOW + RETURN_BACK_MS - 1);
        assert!(pilot.take_notes().is_empty());
        status_at(&mut picture, own, 90, NOW + RETURN_BACK_MS);
        pilot.decide(&picture, &hold_doctrine(), NOW + RETURN_BACK_MS);
        assert_eq!(pilot.take_notes(), [waiting_note("BA", 300.0)]);
        assert_eq!(pilot.rule(), Some("return_wait"));
    }

    /// The return waiting for BA from [`RETURN_STANDOFF_M`] west of its last
    /// plot, (2 100, 970), since `NOW`, BA lost and its trail kept.
    fn waiting_for_ba() -> (Picture, Pilot, (f64, f64)) {
        let plot = (2_100.0, 970.0);
        let own = (plot.0 - RETURN_STANDOFF_M, 970.0);
        let (picture, mut pilot) = following_at(own);
        pilot.trail = Some(Trail {
            designator: "BA".into(),
            plot_m: plot,
            plot_ms: NOW - 20_000,
            track: Some(Track {
                course_deg: 90.0,
                speed_mps: 8.0,
            }),
            seen_from_m: Some((plot.0 - 350.0, 970.0)),
        });
        pilot.trailed.insert("BA".into());
        pilot.hurt.insert("BA".into());
        pilot.ret = Some(Return {
            d: "BA".into(),
            plot_m: plot,
            stage: ReturnStage::Wait { since_ms: NOW },
        });
        (picture, pilot, own)
    }

    /// **The return is fired on from the halt** (cut AN.3): a fresh BA inside
    /// the stock sight during the wait arms the halt and ends the wait; a hit
    /// taken ends it with nothing said, and the precedence flies what it
    /// would.
    #[test]
    fn the_return_is_fired_on_from_the_halt() {
        let (mut picture, mut pilot, own) = waiting_for_ba();
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW + 10_000);
        assert_eq!(orders.speed, Some(0));
        assert_eq!(pilot.rule(), Some("return_wait"));
        let t = NOW + 30_000;
        hold(&mut picture, ba_east(own, 550, t, None));
        let orders = pilot.decide(&picture, &hold_doctrine(), t);
        assert_eq!(
            pilot.take_notes(),
            ["Firing from the halt at BA, 550 m off."]
        );
        assert_eq!(pilot.ret, None);
        assert_eq!(pilot.halt.as_ref().map(|h| h.d.as_str()), Some("BA"));
        assert_eq!(pilot.rule(), Some("halt"));
        assert!(orders.speed.is_none_or(|kph| kph == 0), "{orders:?}");
        assert_eq!(orders.fire, ["cannon"]);
        // A hit taken ends the wait, nothing said, and the hunt may start.
        let (mut picture, mut pilot, _) = waiting_for_ba();
        hit_taken(&mut picture, NOW + 10_000);
        pilot.decide(&picture, &hold_doctrine(), NOW + 10_100);
        assert_eq!(pilot.ret, None);
        let notes = pilot.take_notes();
        assert!(
            notes
                .iter()
                .all(|note| !note.starts_with("No return") && !note.starts_with("Waiting")),
            "{notes:?}"
        );
        assert!(pilot.hunt.is_some(), "the hunt, its trail still young");
    }

    /// **The return's wait ends into the search** (cut AN.3): at
    /// [`RETURN_WAIT_MS`] it says so, drops the trail of the machine it
    /// waited for, starts no hunt, and the search flies.
    #[test]
    fn the_return_wait_ends_into_the_search() {
        let (picture, mut pilot, _) = waiting_for_ba();
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW + RETURN_WAIT_MS - 1);
        assert!(pilot.ret.is_some());
        assert!(!orders.ask_spot_b);
        assert!(pilot.take_notes().is_empty());
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW + RETURN_WAIT_MS);
        let notes = pilot.take_notes();
        assert_eq!(
            notes,
            [format!(
                "No return from BA in {} s; searching.",
                RETURN_WAIT_MS / 1_000
            )]
        );
        no_hunt(&notes);
        assert_eq!(pilot.ret, None);
        assert_eq!(pilot.trail, None, "the trail dropped");
        assert_eq!(pilot.hunt, None);
        assert!(orders.ask_spot_b, "the search: {orders:?}");
        assert_eq!(pilot.rule(), None);
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW + RETURN_WAIT_MS + 5_000);
        assert!(orders.ask_spot_b || orders == Orders::default());
        assert_eq!(pilot.hunt, None);
        no_hunt(&pilot.take_notes());
    }

    /// **A stopped broken machine ends the follow** (cut AN.3): no track for
    /// [`FOLLOW_STOPPED_MS`] outside the keep-out ends it silently, and the
    /// stance flies.
    #[test]
    fn a_stopped_broken_machine_ends_the_follow() {
        let own = (1_000.0, 970.0);
        let (mut picture, mut pilot) = following_at(own);
        hold(&mut picture, ba_east(own, 420, NOW, Some(90.0)));
        hurt_event(&mut picture, "BA", NOW - 100);
        pilot.decide(&picture, &hold_doctrine(), NOW);
        assert!(pilot.follow.is_some());
        pilot.take_notes();
        hold(&mut picture, ba_east(own, 420, NOW + 1_000, None));
        for t in [NOW + 1_000, NOW + FOLLOW_STOPPED_MS - 1] {
            pilot.decide(&picture, &hold_doctrine(), t);
            assert!(pilot.follow.is_some(), "at {t}");
            assert_eq!(pilot.rule(), Some("follow"));
        }
        hold(
            &mut picture,
            ba_east(own, 420, NOW + FOLLOW_STOPPED_MS, None),
        );
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW + FOLLOW_STOPPED_MS);
        assert_eq!(pilot.follow, None);
        assert_eq!(pilot.ret, None);
        assert!(pilot.take_notes().is_empty(), "silently");
        assert_eq!(pilot.rule(), None, "the stance flies");
        assert!(orders.speed.is_none_or(|kph| kph == 0), "{orders:?}");
        // A kill ends a follow too, and the wait for a return.
        let (mut picture, mut pilot) = following_at(own);
        hold(&mut picture, ba_east(own, 420, NOW, Some(90.0)));
        hurt_event(&mut picture, "BA", NOW - 100);
        pilot.decide(&picture, &hold_doctrine(), NOW);
        assert!(pilot.follow.is_some());
        picture.events.push_back(Event {
            at_ms: NOW + 500,
            kind: EventKind::Kill,
            text: "BA destroyed.".into(),
        });
        pilot.decide(&picture, &hold_doctrine(), NOW + 600);
        assert_eq!(pilot.follow, None);
        let (mut picture, mut pilot, _) = waiting_for_ba();
        picture.events.push_back(Event {
            at_ms: NOW + 500,
            kind: EventKind::Kill,
            text: "BA destroyed.".into(),
        });
        pilot.decide(&picture, &hold_doctrine(), NOW + 600);
        assert_eq!(pilot.ret, None);
        // A doctrine `withdraw` ends it, and none begins under one.
        let (mut picture, mut pilot) = following_at(own);
        hold(&mut picture, ba_east(own, 420, NOW, Some(90.0)));
        hurt_event(&mut picture, "BA", NOW - 100);
        pilot.decide(&picture, &hold_doctrine(), NOW);
        assert!(pilot.follow.is_some());
        let withdraw = Doctrine {
            stance: Stance::Withdraw,
            ..hold_doctrine()
        };
        pilot.decide(&picture, &withdraw, NOW + 500);
        assert_eq!(pilot.follow, None);
        pilot.decide(&picture, &withdraw, NOW + 1_000);
        assert_eq!(pilot.follow, None);
    }

    /// **With the follow off a hurt machine is held and hunted as before**
    /// (cut AN.3); and with the re-approach off, eight hits scored on an
    /// unhurt machine driving away still make it broken (the count is fed
    /// whatever the knobs say).
    #[test]
    fn with_follow_off_a_hurt_machine_is_held_and_hunted_as_before() {
        let own = (1_000.0, 970.0);
        let (mut picture, mut pilot) = following_at(own);
        pilot.knobs = Knobs {
            follow_broken: false,
            ..Knobs::on()
        };
        hold(&mut picture, ba_east(own, 450, NOW, Some(90.0)));
        hurt_event(&mut picture, "BA", NOW - 100);
        scored(&mut picture, "BA", BROKEN_HITS_MIN, NOW);
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!(pilot.follow, None);
        assert_eq!(pilot.rule(), None);
        assert_eq!(orders.speed, Some(0), "held at the doctrine's band");
        assert_eq!(pilot.scored_on.get("BA"), Some(&BROKEN_HITS_MIN));
        picture.contacts.clear();
        pilot.decide(&picture, &hold_doctrine(), NOW + 1_000);
        let notes = pilot.take_notes();
        assert!(
            notes
                .iter()
                .any(|note| note.starts_with("Hunting BA: its last plot")),
            "{notes:?}"
        );
        assert!(notes.iter().all(|note| !note.starts_with("Following")));
        assert_eq!(pilot.ret, None);
        // The re-approach off, eight hits on an unhurt BA driving away.
        let (mut picture, mut pilot) = following_at(own);
        pilot.knobs = Knobs {
            reapproach: false,
            ..Knobs::on()
        };
        hold(&mut picture, ba_east(own, 450, NOW, Some(90.0)));
        scored(&mut picture, "BA", BROKEN_HITS_MIN, NOW);
        assert!(!pilot.hurt.contains("BA"));
        pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!(pilot.scored_on.get("BA"), Some(&BROKEN_HITS_MIN));
        assert_eq!(pilot.take_notes(), [following_note("BA")]);
        // A friendly hit and an unnamed one count for nobody.
        let (mut picture, mut pilot) = following_at(own);
        for text in ["Hit on RB (FRIENDLY): turret.", "Hit.", "Hit on BB."] {
            picture.events.push_back(Event {
                at_ms: NOW - 50,
                kind: EventKind::HitScored,
                text: text.into(),
            });
        }
        pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!(
            pilot.scored_on,
            BTreeMap::from([("BB".to_owned(), 1)]),
            "only a named hostile"
        );
    }

    /// **No follow without a muster** (cut AN.3): power from `Power is
    /// already online.` (a match joined running) leaves no entry line, so a
    /// hurt machine driving away is held as before.
    #[test]
    fn no_follow_without_a_muster() {
        let own = (1_000.0, 970.0);
        let mut picture = running();
        picture.apply(NOW - 5_000, &classify("Power is already online."));
        status_at(&mut picture, own, 90, NOW - 4_900);
        let mut pilot = Pilot::with_knobs(Knobs::on());
        pilot.ceilings_asked = true;
        pilot.decide(&picture, &hold_doctrine(), NOW - 4_800);
        assert_eq!(pilot.muster_m, None);
        assert_eq!(pilot.other_entry_now(&picture), None);
        hold(&mut picture, ba_east(own, 450, NOW, Some(90.0)));
        hurt_event(&mut picture, "BA", NOW - 100);
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!(pilot.follow, None);
        assert_eq!(pilot.rule(), None);
        assert_eq!(orders.speed, Some(0), "held at the band: {orders:?}");
        let notes = pilot.take_notes();
        assert!(
            notes.iter().all(|note| !note.starts_with("Following")),
            "{notes:?}"
        );
    }

    /// **The follow names only held designators** (cut AN.3; the bot's own
    /// observer rule): the follow's and the keep-out's notes name a contact
    /// held when they are said, the wait's notes one the stream named this
    /// match, and the rule is a word.
    #[test]
    fn the_follow_names_only_held_designators() {
        let mut named: BTreeSet<String> = BTreeSet::new();
        let check = |pilot: &mut Pilot, picture: &Picture, named: &mut BTreeSet<String>| {
            named.extend(picture.contacts.keys().cloned());
            for note in pilot.take_notes() {
                for word in note.split(|c: char| !c.is_ascii_alphanumeric()) {
                    if crate::arena::lines::is_designator(word) {
                        let held = picture.contacts.contains_key(word);
                        let waits =
                            note.starts_with("Waiting for ") || note.starts_with("No return");
                        assert!(held || (waits && named.contains(word)), "{word} in {note}");
                    }
                }
            }
            assert!(pilot.rule().is_none_or(|rule| {
                [
                    "opening",
                    "halt",
                    "reapproach",
                    "approach_wait",
                    "follow",
                    "return_back",
                    "return_wait",
                ]
                .contains(&rule)
            }));
        };
        let own = (1_400.0, 970.0);
        let (mut picture, mut pilot) = following_at(own);
        hold(&mut picture, ba_east(own, 300, NOW, Some(90.0)));
        hurt_event(&mut picture, "BA", NOW - 100);
        // Cut AO.1: hits scored on BA, so the counts' check below reads a
        // count, not an empty map.
        scored(&mut picture, "BA", 2, NOW);
        pilot.decide(&picture, &hold_doctrine(), NOW);
        assert!(pilot.follow.is_some());
        check(&mut pilot, &picture, &mut named);
        let later = (1_800.0, 970.0);
        status_at(&mut picture, later, 90, NOW + 40_000);
        hold(&mut picture, ba_east(later, 300, NOW + 40_000, Some(90.0)));
        pilot.decide(&picture, &hold_doctrine(), NOW + 40_000);
        assert!(pilot.ret.is_some());
        check(&mut pilot, &picture, &mut named);
        picture.contacts.clear();
        let out = (2_100.0 - RETURN_STANDOFF_M, 970.0);
        status_at(&mut picture, out, 90, NOW + 70_000);
        pilot.decide(&picture, &hold_doctrine(), NOW + 70_000);
        check(&mut pilot, &picture, &mut named);
        pilot.decide(&picture, &hold_doctrine(), NOW + 70_000 + RETURN_WAIT_MS);
        check(&mut pilot, &picture, &mut named);
        // The follow, the return and the counts hold names the stream gave
        // and nothing else (cut AO.1: over inputs that hold some — the
        // count, and every designator anywhere in the pilot's state).
        assert!(!pilot.scored_on.is_empty());
        assert!(pilot.scored_on.keys().all(|d| d == "BA"));
        let json = serde_json::to_value(&pilot).expect("json");
        let mut held = Vec::new();
        designators_in(&json, &mut held);
        assert!(!held.is_empty(), "the pilot's state names BA");
        assert!(held.iter().all(|d| named.contains(d)), "{held:?}");
        for d in ["BB", "BC", "X1"] {
            assert!(!json.to_string().contains(d), "{d}");
        }
    }

    /// **A broken machine ends a re-approach or a halt and is followed** (cut
    /// AN.3): opening the range on a stopped BA that out-hits the tank, BA
    /// loses its turret and drives away: the follow begins at that decision,
    /// said, and the re-approach ends, rested — so no hunt drives at the
    /// machine's last plot once it is lost. The same from a halt in force.
    #[test]
    fn a_broken_machine_ends_a_reapproach_or_a_halt_and_is_followed() {
        let own = (1_000.0, 970.0);
        let far = (FOLLOW_BAND_M + HOLD_SLACK_M + HOLD_HYSTERESIS_M + 25.0) as u32;
        let (mut picture, mut pilot) = following_at(own);
        hold(&mut picture, ba_east(own, 420, NOW, None));
        exchange(&mut picture, OUT_TAKEN, OUT_SCORED, NOW);
        pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!(pilot.take_notes(), [opening_the_range_note()]);
        assert!(pilot.reapproach.is_some());
        assert_eq!(pilot.rule(), Some("reapproach"));
        // Hurt, and driving away.
        let t = NOW + 2_000;
        hurt_event(&mut picture, "BA", t - 100);
        hold(&mut picture, ba_east(own, far, t, Some(90.0)));
        let orders = pilot.decide(&picture, &hold_doctrine(), t);
        assert_eq!(pilot.take_notes(), [following_note("BA")]);
        assert_eq!(pilot.reapproach, None);
        assert_eq!(pilot.reapproach_rest_until_ms, t + REAPPROACH_REST_MS);
        assert_eq!(pilot.follow.as_ref().map(|f| f.d.as_str()), Some("BA"));
        assert_eq!(pilot.rule(), Some("follow"));
        assert_eq!(orders.speed, Some(54), "closing ahead: {orders:?}");
        // From a halt in force on it.
        let (mut picture, mut pilot) = following_at(own);
        pilot.halt = Some(Halt {
            d: "BA".into(),
            since_ms: NOW - 1_000,
            nearest_m: far,
            nearest_ms: NOW - 1_000,
        });
        hold(&mut picture, ba_east(own, far, NOW, Some(90.0)));
        hurt_event(&mut picture, "BA", NOW - 100);
        pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!(pilot.take_notes(), [following_note("BA")]);
        assert_eq!(pilot.halt, None);
        assert!(pilot.follow.is_some());
        assert_eq!(pilot.rule(), Some("follow"));
        // A halt on another hostile, BB coming on from the north, holds the
        // follow off: step 3c outranks step 4.
        let (mut picture, mut pilot) = following_at(own);
        pilot.halt = Some(Halt {
            d: "BB".into(),
            since_ms: NOW - 1_000,
            nearest_m: 600,
            nearest_ms: NOW - 1_000,
        });
        hold(&mut picture, ba_east(own, far, NOW, Some(90.0)));
        hurt_event(&mut picture, "BA", NOW - 100);
        let mut bb = row_at(own, "BB", Some(Side::Blue), 0, 560, NOW, 0);
        bb.track = Some(Track {
            course_deg: 180.0,
            speed_mps: 8.0,
        });
        hold(&mut picture, bb);
        pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!(pilot.follow, None);
        assert_eq!(pilot.halt.as_ref().map(|h| h.d.as_str()), Some("BB"));
        assert_eq!(pilot.rule(), Some("halt"));
        assert!(
            pilot
                .take_notes()
                .iter()
                .all(|note| !note.starts_with("Following"))
        );
    }

    /// **No re-approach while a follow or a return is in force** (cut AN.3;
    /// the two rules of step 3a are never both in force): a followed machine
    /// stopped in sight that out-hits the tank begins no re-approach, nor does
    /// one while the return backs off.
    #[test]
    fn no_reapproach_while_a_follow_or_a_return_is_in_force() {
        let own = (1_000.0, 970.0);
        let (mut picture, mut pilot) = following_at(own);
        hold(&mut picture, ba_east(own, 420, NOW, Some(90.0)));
        hurt_event(&mut picture, "BA", NOW - 100);
        pilot.decide(&picture, &hold_doctrine(), NOW);
        assert!(pilot.follow.is_some());
        pilot.take_notes();
        // Stopped (no track, inside FOLLOW_STOPPED_MS) and out-hitting.
        let t = NOW + 1_000;
        hold(&mut picture, ba_east(own, 420, t, None));
        exchange(&mut picture, OUT_TAKEN, OUT_SCORED, t);
        pilot.decide(&picture, &hold_doctrine(), t);
        assert_eq!(pilot.reapproach, None);
        assert!(pilot.follow.is_some());
        assert_eq!(pilot.rule(), Some("follow"));
        assert!(
            pilot
                .take_notes()
                .iter()
                .all(|note| !note.starts_with("Opening the range")),
        );
        // The return backing off, BA stopped in sight and out-hitting.
        let (mut picture, mut pilot) = following_at(own);
        pilot.ret = Some(Return {
            d: "BA".into(),
            plot_m: (own.0 + 300.0, 970.0),
            stage: ReturnStage::Back { since_ms: NOW },
        });
        hold(&mut picture, ba_east(own, 420, NOW, None));
        exchange(&mut picture, OUT_TAKEN, OUT_SCORED, NOW);
        pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!(pilot.reapproach, None);
        assert_eq!(pilot.follow, None, "no follow while its return is in force");
        assert_eq!(pilot.rule(), Some("return_back"));
        assert!(
            pilot
                .take_notes()
                .iter()
                .all(|note| !note.starts_with("Opening the range")),
        );
    }

    /// **The follow ends when the pilot turns to another machine** (cut
    /// AN.3): followed BA still held, BB 140 m nearer becomes the target; the
    /// follow ends, silently and with no return, so it neither holds off the
    /// re-approach on BB nor backs the tank off bow on BA's plot.
    #[test]
    fn the_follow_ends_when_the_pilot_turns_to_another_machine() {
        let own = (1_000.0, 970.0);
        let (mut picture, mut pilot) = following_at(own);
        hold(&mut picture, ba_east(own, 420, NOW, Some(90.0)));
        hurt_event(&mut picture, "BA", NOW - 100);
        pilot.decide(&picture, &hold_doctrine(), NOW);
        assert!(pilot.follow.is_some());
        pilot.take_notes();
        let t = NOW + 1_000;
        hold(&mut picture, ba_east(own, 440, t, Some(90.0)));
        hold(
            &mut picture,
            row_at(own, "BB", Some(Side::Blue), 45, 300, t, 0),
        );
        pilot.decide(&picture, &hold_doctrine(), t);
        assert_eq!(pilot.follow, None);
        assert_eq!(pilot.ret, None);
        assert_ne!(pilot.rule(), Some("follow"));
        let notes = pilot.take_notes();
        assert!(
            notes
                .iter()
                .all(|note| !note.starts_with("Not following") && !note.starts_with("Waiting")),
            "{notes:?}"
        );
        // BB stopped and out-hitting the tank: the re-approach may begin.
        let t = NOW + 2_000;
        hold(&mut picture, ba_east(own, 460, t, Some(90.0)));
        hold(
            &mut picture,
            row_at(own, "BB", Some(Side::Blue), 45, 300, t, 0),
        );
        exchange(&mut picture, OUT_TAKEN, OUT_SCORED, t);
        pilot.decide(&picture, &hold_doctrine(), t);
        assert_eq!(pilot.reapproach.as_ref().map(|r| r.d.as_str()), Some("BB"));
        assert_eq!(pilot.follow, None);
    }

    /// **A followed machine lost backs off from its trail** (cut AN.3): BA
    /// gone from the scope while followed ends the follow into the return from
    /// the trail's plot of it, nothing more said and no hunt; with the tank
    /// already [`RETURN_STANDOFF_M`] or more from that plot, the return waits
    /// at once, said.
    #[test]
    fn a_followed_machine_lost_backs_off_from_its_trail() {
        let own = (1_000.0, 970.0);
        let lose = |moved_to: (f64, f64)| {
            let (mut picture, mut pilot) = following_at(own);
            hold(&mut picture, ba_east(own, 420, NOW, Some(90.0)));
            hurt_event(&mut picture, "BA", NOW - 100);
            let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
            assert!(pilot.follow.is_some());
            assert_eq!(pilot.take_notes(), [following_note("BA")]);
            if let Some(kph) = orders.speed {
                pilot.sent_speed(kph);
            }
            let plot = pilot.trail.as_ref().expect("the trail of BA").plot_m;
            let t = NOW + 1_000;
            status_at(&mut picture, moved_to, 90, t);
            picture.contacts.clear();
            let orders = pilot.decide(&picture, &hold_doctrine(), t);
            (picture, pilot, orders, plot, t)
        };
        // Short of the standoff: backing off.
        let (picture, mut pilot, orders, plot, t) = lose(own);
        assert_eq!(pilot.follow, None);
        assert_eq!(
            pilot.ret,
            Some(Return {
                d: "BA".into(),
                plot_m: plot,
                stage: ReturnStage::Back { since_ms: t },
            })
        );
        assert!(pilot.take_notes().is_empty(), "nothing more said");
        assert_eq!(pilot.hunt, None);
        assert_eq!(pilot.rule(), Some("return_back"));
        assert_eq!(orders.speed, Some(-27), "{orders:?}");
        pilot.decide(&picture, &hold_doctrine(), t + 5_000);
        assert_eq!(pilot.hunt, None);
        no_hunt(&pilot.take_notes());
        // Already past the standoff: waiting at once.
        let away = (own.0 - 300.0, 970.0);
        let (_, mut pilot, orders, plot, t) = lose(away);
        let off = distance_m(away, plot);
        assert!(off >= RETURN_STANDOFF_M, "{off}");
        assert_eq!(pilot.take_notes(), [waiting_note("BA", off.round())]);
        assert!(matches!(
            pilot.ret.as_ref().map(|r| r.stage),
            Some(ReturnStage::Wait { since_ms }) if since_ms == t
        ));
        assert_eq!(pilot.rule(), Some("return_wait"));
        assert!(orders.speed.is_none_or(|kph| kph == 0), "{orders:?}");
        assert_eq!(pilot.hunt, None);
    }

    // -- cut AO.1: off the wall ---------------------------------------------------

    /// The move in's note: `leg` metres in, `room` metres behind.
    fn moving_in_note(leg: f64, room: f64) -> String {
        format!(
            "Opening: moving in {:.0} m toward the other side's entry: {:.0} m of ground behind \
             us, {:.0} m wanted.",
            leg.round(),
            room.round(),
            OPENING_ROOM_M.round()
        )
    }

    /// AN.2's step note, north, on a ray of 10 m of road then grass.
    fn stepping_north_note() -> String {
        let step = 10.0 + 5.0 + OPENING_COVER_M;
        format!("Opening: stepping off the road to the north ({step:.0} m).")
    }

    /// The notes in `notes` that start `head`.
    fn starting<'a>(notes: &'a [String], head: &str) -> Vec<&'a String> {
        notes.iter().filter(|note| note.starts_with(head)).collect()
    }

    /// **The opening moves in for room behind** (cut AO.1): from the shipped
    /// muster, 60 m off the west edge, the brief's room is 60 m; the pilot
    /// drives in on 090 at full ahead the shortfall and the arrival, says so,
    /// stops at the aim and asks the brief again; the muster's brief is never
    /// read twice; the second, from the aim, has room, and AN.2's step north
    /// and hold follow, facing 091.
    #[test]
    fn the_opening_moves_in_for_room_behind() {
        let (mut picture, mut pilot, orders) = stepping(MUSTER, &muster_brief());
        let room = 60.0;
        let leg = OPENING_ROOM_M - room + OPENING_ARRIVE_M;
        assert!(leg <= 320.0 - OPENING_COVER_M, "inside the east ray");
        assert_eq!(pilot.take_notes(), [moving_in_note(leg, room)]);
        assert_eq!((orders.heading, orders.speed), (Some(90), Some(54)));
        assert!(!orders.ask_view_brief);
        assert_eq!(pilot.opening.word(), "inward");
        assert_eq!(pilot.rule(), Some("opening"));
        assert_eq!(pilot.opening_legs, 1);
        let OpeningStage::Inward { aim_m, from_m, .. } = pilot.opening.clone() else {
            panic!("moving in: {:?}", pilot.opening);
        };
        assert_eq!(from_m, MUSTER);
        assert!(
            distance_m(aim_m, (MUSTER.0 + leg, MUSTER.1)) < 1e-6,
            "{aim_m:?}"
        );
        pilot.sent_heading(90, POWER + 1_050);
        pilot.sent_speed(54);
        // Short of the aim: driving on.
        status_at(
            &mut picture,
            (aim_m.0 - OPENING_ARRIVE_M - 1.0, aim_m.1),
            90,
            POWER + 12_000,
        );
        let orders = decide_at(&picture, &mut pilot, POWER + 12_000);
        assert_eq!(pilot.opening.word(), "inward");
        assert_eq!((orders.speed, orders.ask_view_brief), (None, false));
        // At the aim: stopped, and the next brief asked.
        status_at(&mut picture, aim_m, 90, POWER + 17_000);
        let orders = decide_at(&picture, &mut pilot, POWER + 17_000);
        assert!(
            matches!(pilot.opening, OpeningStage::Brief { since_ms } if since_ms == POWER + 17_000),
            "{:?}",
            pilot.opening
        );
        assert_eq!(orders.speed, Some(0));
        assert!(orders.ask_view_brief);
        assert_eq!(pilot.rule(), Some("opening"));
        assert!(pilot.take_notes().is_empty());
        pilot.sent_speed(0);
        // The muster's brief is not read again: still asking, stopped.
        let orders = decide_at(&picture, &mut pilot, POWER + 18_000);
        assert!(orders.ask_view_brief);
        assert!(matches!(pilot.opening, OpeningStage::Brief { .. }));
        assert_eq!(orders.speed, None, "the stop stands");
        // The second brief, from the aim: room enough, AN.2's step north.
        read_brief(&mut picture, POWER + 19_000, &road_brief());
        let orders = decide_at(&picture, &mut pilot, POWER + 19_050);
        assert_eq!(pilot.take_notes(), [stepping_north_note()]);
        assert_eq!(
            (orders.heading, orders.speed),
            (Some(0), Some(OPENING_STEP_KPH))
        );
        assert!(!orders.ask_view_brief);
        pilot.sent_heading(0, POWER + 19_050);
        pilot.sent_speed(OPENING_STEP_KPH);
        let step = 10.0 + 5.0 + OPENING_COVER_M;
        let arrived = (aim_m.0, aim_m.1 + step - OPENING_ARRIVE_M);
        status_at(&mut picture, arrived, 0, POWER + 22_000);
        let orders = decide_at(&picture, &mut pilot, POWER + 22_000);
        let facing = whole(bearing_deg(arrived, (2_500.0, 970.0)));
        assert_eq!(facing, 91);
        assert_eq!((orders.heading, orders.speed), (Some(facing), Some(0)));
        assert_eq!(
            pilot.take_notes(),
            [format!(
                "Opening: holding near our entry, facing {facing:03}° toward the other side's entry."
            )]
        );
        assert_eq!(pilot.opening_legs, 1);
        assert!(!pilot.opening_no_way_said);
    }

    /// **With room behind, the opening steps off the road at once** (cut
    /// AO.1): from [`ROOMY_MUSTER`] there is no move in; AN.2's step and hold
    /// follow, and the brief is asked once.
    #[test]
    fn with_room_behind_the_opening_steps_off_the_road_at_once() {
        let (mut picture, mut pilot, orders) = stepping(ROOMY_MUSTER, &road_brief());
        let notes = pilot.take_notes();
        assert_eq!(notes, [stepping_north_note()]);
        assert!(starting(&notes, "Opening: moving in").is_empty());
        assert_eq!(
            (orders.heading, orders.speed),
            (Some(0), Some(OPENING_STEP_KPH))
        );
        assert!(!orders.ask_view_brief);
        pilot.sent_heading(0, POWER + 1_050);
        pilot.sent_speed(OPENING_STEP_KPH);
        let step = 10.0 + 5.0 + OPENING_COVER_M;
        let arrived = (ROOMY_MUSTER.0, ROOMY_MUSTER.1 + step - OPENING_ARRIVE_M);
        status_at(&mut picture, arrived, 0, POWER + 4_000);
        for t in [POWER + 4_000, POWER + 30_000, POWER + 60_000] {
            let orders = decide_at(&picture, &mut pilot, t);
            assert!(!orders.ask_view_brief, "{t}");
        }
        assert!(matches!(pilot.opening, OpeningStage::Hold { .. }));
        assert_eq!(pilot.opening_legs, 0);
    }

    /// **Blocking ground ahead cuts the move in** (cut AO.1): deep water
    /// 100 m east cuts the leg to 100 m less the cover; with heavy woods 60 m
    /// behind, every brief has the same short room, and once the legs are used
    /// up the next brief holds where the tank stands. A forward ray that
    /// clears less than the cover and the least leg says `no way in`, once;
    /// a shortfall of the slack or less is room enough, and one metre more is
    /// a leg.
    #[test]
    fn blocking_ground_ahead_cuts_the_move_in() {
        let woods_behind = |word: &str| {
            format!("{word}: 60 m of grass, then heavy woods to 120 m, then beyond the window.")
        };
        let brief = rays(|word, deg| match deg {
            90 => format!(
                "{word}: 100 m of road, then deep water to 150 m, then grass to 190 m, then \
                 beyond the window."
            ),
            270 => woods_behind(word),
            _ => format!("{word}: 90 m of grass, then beyond the window."),
        });
        let (mut picture, mut pilot, mut orders) = stepping(MUSTER, &brief);
        let leg = 100.0 - OPENING_COVER_M;
        let mut t = POWER + 1_050;
        for n in 1..=OPENING_INWARD_LEGS_MAX {
            assert_eq!(pilot.take_notes(), [moving_in_note(leg, 60.0)], "leg {n}");
            assert_eq!(pilot.opening_legs, n);
            assert_eq!(orders.speed, Some(54), "leg {n}");
            let OpeningStage::Inward { aim_m, from_m, .. } = pilot.opening.clone() else {
                panic!("leg {n}: {:?}", pilot.opening);
            };
            assert!((distance_m(from_m, aim_m) - leg).abs() < 1e-6);
            assert_eq!(whole(bearing_deg(from_m, aim_m)), 90);
            pilot.sent_speed(54);
            if let Some(deg) = orders.heading {
                pilot.sent_heading(deg, t);
            }
            t += 10_000;
            status_at(&mut picture, aim_m, 90, t);
            let stopped = decide_at(&picture, &mut pilot, t);
            assert_eq!(stopped.speed, Some(0), "leg {n}");
            assert!(stopped.ask_view_brief, "leg {n}");
            pilot.sent_speed(0);
            t += 1_000;
            read_brief(&mut picture, t, &brief);
            t += 50;
            orders = decide_at(&picture, &mut pilot, t);
        }
        // The legs used up: the brief holds where the tank stands (seven of
        // the eight rays begin on grass), with no leg and no note of one.
        let notes = pilot.take_notes();
        assert!(
            starting(&notes, "Opening: moving in").is_empty(),
            "{notes:?}"
        );
        assert!(
            matches!(pilot.opening, OpeningStage::Hold { .. }),
            "{:?}",
            pilot.opening
        );
        assert_eq!(pilot.opening_legs, OPENING_INWARD_LEGS_MAX);
        assert!(
            orders.speed.is_none_or(|kph| kph == 0),
            "stopped: {orders:?}"
        );

        // A forward ray that clears less than the cover and the least leg.
        let clear = (OPENING_COVER_M + OPENING_INWARD_MIN_M - 10.0).round();
        let brief = rays(|word, deg| match deg {
            90 => format!(
                "{word}: {clear:.0} m of road, then deep water to {:.0} m, then beyond the window.",
                clear + 100.0
            ),
            270 => woods_behind(word),
            _ => format!("{word}: 90 m of grass, then beyond the window."),
        });
        let (picture, mut pilot, orders) = stepping(MUSTER, &brief);
        assert_eq!(
            pilot.take_notes(),
            [
                "Opening: no way in toward the other side's entry; 60 m of ground behind us."
                    .to_owned(),
                "Opening: holding near our entry, facing 090° toward the other side's entry."
                    .to_owned(),
            ]
        );
        assert_eq!(orders.speed, Some(0));
        assert_eq!(pilot.opening_legs, 0);
        assert!(pilot.opening_no_way_said);
        for t in [POWER + 2_000, POWER + 20_000] {
            decide_at(&picture, &mut pilot, t);
            assert!(pilot.take_notes().is_empty(), "said once");
        }

        // A shortfall of the slack is room enough; one metre more is a leg.
        let enough = (OPENING_ROOM_M - OPENING_ROOM_SLACK_M, 970.0);
        let (_, mut pilot, _) = stepping(enough, &road_brief());
        assert_eq!(pilot.take_notes(), [stepping_north_note()]);
        assert_eq!(pilot.opening_legs, 0);
        let short = (enough.0 - 1.0, 970.0);
        let (_, mut pilot, _) = stepping(short, &road_brief());
        assert_eq!(
            pilot.take_notes(),
            [moving_in_note(
                OPENING_ROOM_SLACK_M + 1.0 + OPENING_ARRIVE_M,
                short.0
            )]
        );
    }

    /// **Blocking ground behind is no room** (cut AO.1): at [`ROOMY_MUSTER`],
    /// heavy woods 50 m behind leave 50 m of room, whatever the edge, and the
    /// pilot moves in.
    #[test]
    fn blocking_ground_behind_is_no_room() {
        let brief = rays(|word, deg| match deg {
            270 => {
                format!("{word}: 50 m of grass, then heavy woods to 120 m, then beyond the window.")
            }
            90 => format!("{word}: 190 m of road, then beyond the window."),
            _ => format!("{word}: 10 m of road, then grass to 90 m, then beyond the window."),
        });
        let (picture, mut pilot, orders) = stepping(ROOMY_MUSTER, &brief);
        let ground = picture.ground.as_ref().expect("the brief's ground");
        assert_eq!(opening_room(ground, 90), Some(50.0));
        let leg = (OPENING_ROOM_M - 50.0 + OPENING_ARRIVE_M).min(190.0 - OPENING_COVER_M);
        assert_eq!(pilot.take_notes(), [moving_in_note(leg, 50.0)]);
        assert_eq!(pilot.opening.word(), "inward");
        assert_eq!(orders.heading, Some(90));
    }

    /// **The room rule needs a back-off to serve** (cut AO.1): with neither
    /// the re-approach nor the follow on, the shipped muster steps off the
    /// road at once as AN.2 did; with either on, it moves in.
    #[test]
    fn the_room_rule_needs_a_back_off_to_serve() {
        let fly = |knobs: Knobs| {
            let (mut picture, mut pilot) = at_muster(MUSTER);
            pilot.knobs = knobs;
            decide_at(&picture, &mut pilot, POWER + 150);
            read_brief(&mut picture, POWER + 1_000, &muster_brief());
            let orders = decide_at(&picture, &mut pilot, POWER + 1_050);
            (pilot.take_notes(), orders, pilot.opening.word())
        };
        let (notes, orders, word) = fly(Knobs {
            reapproach: false,
            follow_broken: false,
            ..Knobs::on()
        });
        assert_eq!(notes, [stepping_north_note()]);
        assert_eq!(word, "step");
        assert_eq!(orders.heading, Some(0));
        let leg = OPENING_ROOM_M - 60.0 + OPENING_ARRIVE_M;
        for knobs in [
            Knobs {
                follow_broken: false,
                ..Knobs::on()
            },
            Knobs {
                reapproach: false,
                ..Knobs::on()
            },
        ] {
            let (notes, orders, word) = fly(knobs);
            assert_eq!(notes, [moving_in_note(leg, 60.0)], "{knobs:?}");
            assert_eq!(word, "inward");
            assert_eq!(orders.heading, Some(90));
        }
    }

    /// A red tank at `own`, flying [`Knobs::on`], with BA stopped 420 m off on
    /// `bearing` out-hitting it at `NOW`; and BA's world point.
    fn out_hit_at(own: (f64, f64), bearing: u32) -> (Picture, Pilot, (f64, f64)) {
        let (mut picture, _) = flying_at(own);
        picture.own.heading_deg = bearing;
        let mut pilot = Pilot::with_knobs(Knobs::on());
        pilot.power_ms = Some(NOW - 30_000);
        pilot.ceilings_asked = true;
        let ba = row_at(own, "BA", Some(Side::Blue), bearing, 420, NOW, 0);
        let at = ba.fixes.back().expect("a fix").pos_m;
        hold(&mut picture, ba);
        exchange(&mut picture, OUT_TAKEN, OUT_SCORED, NOW);
        (picture, pilot, at)
    }

    /// The help's square, as the pilot reads it with no header.
    const HELP_SIDE: (f64, f64) = (ARENA_SIDE_M, ARENA_SIDE_M);

    /// **A re-approach at the edge slants, then holds** (cut AO.1): a machine
    /// stopped off the axis (060°) out-hits the tank near the west edge.
    /// While a slant leaves `EDGE_BACK_MARGIN_M` astern, the reverse's bow is
    /// the smallest such slant (10°, then 20°, minus first); once none does,
    /// the back-off holds — stopped, its note said, its own wait, and no give-up
    /// after `REAPPROACH_MAX_MS`. Square to the edge the straight bow is kept
    /// until its room runs out; and a back-off begun with no room stops at once
    /// and holds at the next decision.
    #[test]
    fn a_reapproach_at_the_edge_slants_then_holds() {
        let margin = EDGE_BACK_MARGIN_M;
        let step = f64::from(BACK_OFF_SLANT_STEP_DEG);
        let roomy = |own: (f64, f64), bow: f64| room_astern(own, bow, HELP_SIDE) >= margin;
        let own = (margin * 0.8, 1_000.0);
        let (mut picture, mut pilot, ba) = out_hit_at(own, 60);
        let aim = bearing_deg(own, ba);
        assert!(!roomy(own, aim) && roomy(own, aim - step));
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!(pilot.take_notes(), [opening_the_range_note()]);
        assert_eq!(pilot.rule(), Some("reapproach"));
        let bow = whole(aim - step);
        assert_eq!((orders.heading, orders.speed), (Some(bow), Some(-27)));
        pilot.sent_heading(bow, NOW);
        pilot.sent_speed(-27);
        // Nearer the edge: the 10° slants leave too little, the 20° enough.
        let own = (margin * 0.66, 1_000.0);
        status_at(&mut picture, own, bow, NOW + 2_000);
        let aim = bearing_deg(own, ba);
        assert!(!roomy(own, aim - step) && !roomy(own, aim + step));
        assert!(roomy(own, aim - 2.0 * step));
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW + 2_000);
        let bow = whole(aim - 2.0 * step);
        assert_eq!(orders.heading, Some(bow));
        assert_eq!(pilot.rule(), Some("reapproach"));
        assert!(pilot.take_notes().is_empty());
        pilot.sent_heading(bow, NOW + 2_000);
        // At the edge: no slant inside the front arc leaves room. Held.
        let own = (margin * 0.4, 1_000.0);
        status_at(&mut picture, own, bow, NOW + 4_000);
        let aim = bearing_deg(own, ba);
        let slants = (1..=HOLD_BOW_DEG / BACK_OFF_SLANT_STEP_DEG).map(|k| f64::from(k) * step);
        assert!(
            std::iter::once(0.0)
                .chain(slants)
                .all(|s| !roomy(own, aim - s) && !roomy(own, aim + s))
        );
        let plot = pilot.reapproach.as_ref().expect("in force").plot_m;
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW + 4_000);
        assert_eq!(
            pilot.take_notes(),
            [format!(
                "No room behind: waiting {:.0} m from BA's last plot.",
                distance_m(own, plot).round()
            )]
        );
        assert_eq!(orders.speed, Some(0));
        assert_eq!(pilot.rule(), Some("approach_wait"));
        assert!(matches!(
            pilot.reapproach.as_ref().map(|r| r.stage),
            Some(ReapproachStage::Wait { since_ms }) if since_ms == NOW + 4_000
        ));
        // BA lost; no give-up, ever.
        picture.contacts.clear();
        let mut notes = Vec::new();
        for t in [
            NOW + 5_000,
            NOW + REAPPROACH_MAX_MS,
            NOW + REAPPROACH_MAX_MS + 1_000,
        ] {
            pilot.decide(&picture, &hold_doctrine(), t);
            notes.extend(pilot.take_notes());
        }
        assert!(
            starting(&notes, "Could not open the range").is_empty(),
            "{notes:?}"
        );

        // Square to the edge: the straight bow while the room lasts, then the
        // hold.
        let own = (margin + 20.0, 1_000.0);
        let (mut picture, mut pilot, ba) = out_hit_at(own, 90);
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!((orders.heading, orders.speed), (Some(90), Some(-27)));
        assert_eq!(pilot.take_notes(), [opening_the_range_note()]);
        pilot.sent_heading(90, NOW);
        pilot.sent_speed(-27);
        let own = (margin + 1.0, 1_000.0);
        status_at(&mut picture, own, 90, NOW + 2_000);
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW + 2_000);
        assert_eq!(orders.heading, None, "still the straight bow");
        assert_eq!(pilot.rule(), Some("reapproach"));
        let own = (margin * 0.8, 1_000.0);
        status_at(&mut picture, own, 90, NOW + 4_000);
        pilot.decide(&picture, &hold_doctrine(), NOW + 4_000);
        assert_eq!(
            pilot.take_notes(),
            [format!(
                "No room behind: waiting {:.0} m from BA's last plot.",
                distance_m(own, ba).round()
            )]
        );
        assert_eq!(pilot.rule(), Some("approach_wait"));

        // Begun with no room: stopped at once, the bow on; held next.
        let own = (margin * 0.4, 1_000.0);
        let (picture, mut pilot, _) = out_hit_at(own, 90);
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!(pilot.take_notes(), [opening_the_range_note()]);
        assert_eq!((orders.heading, orders.speed), (Some(90), Some(0)));
        assert_eq!(pilot.rule(), Some("reapproach"));
        pilot.decide(&picture, &hold_doctrine(), NOW + 500);
        assert_eq!(
            starting(&pilot.take_notes(), "No room behind: ").len(),
            1,
            "held"
        );
        assert_eq!(pilot.rule(), Some("approach_wait"));
    }

    /// `Held: this is the edge of the arena.` at `at`.
    fn edge_event(picture: &mut Picture, at_ms: u64) {
        picture.events.push_back(Event {
            at_ms,
            kind: EventKind::Held,
            text: "Held: this is the edge of the arena.".into(),
        });
    }

    /// **A held stop while backing off ends the back-off** (cut AO.1): a
    /// `Held:` read while the return's `Back` reverses ends it in its wait at
    /// once, said; the same for the re-approach's `Open`; and a held mark
    /// behind that the shared reverse path stops on ends the back-off at the
    /// next decision.
    #[test]
    fn a_held_stop_while_backing_off_ends_the_back_off() {
        // The return.
        let own = (1_000.0, 970.0);
        let (mut picture, mut pilot) = following_at(own);
        pilot.ret = Some(Return {
            d: "BA".into(),
            plot_m: (1_300.0, 970.0),
            stage: ReturnStage::Back { since_ms: NOW },
        });
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!(orders.speed, Some(-27));
        assert_eq!(pilot.rule(), Some("return_back"));
        pilot.sent_speed(-27);
        edge_event(&mut picture, NOW + 1_000);
        pilot.decide(&picture, &hold_doctrine(), NOW + 1_100);
        let notes = pilot.take_notes();
        assert_eq!(
            starting(&notes, "No room behind for the return: "),
            ["No room behind for the return: waiting 300 m from BA's last plot."],
            "{notes:?}"
        );
        assert!(starting(&notes, "Waiting for ").is_empty(), "{notes:?}");
        assert!(matches!(
            pilot.ret.as_ref().map(|r| r.stage),
            Some(ReturnStage::Wait { since_ms }) if since_ms == NOW + 1_100
        ));
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW + 1_100 + HELD_FOR_MS);
        assert_eq!(pilot.rule(), Some("return_wait"));
        assert!(orders.speed.is_none_or(|kph| kph == 0), "{orders:?}");
        assert!(pilot.take_notes().is_empty());

        // The re-approach.
        let (mut picture, mut pilot) = reapproaching();
        hold(&mut picture, ba_from(MID, NOW, 0));
        exchange(&mut picture, OUT_TAKEN, OUT_SCORED, NOW);
        pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!(pilot.take_notes(), [opening_the_range_note()]);
        pilot.sent_speed(-27);
        edge_event(&mut picture, NOW + 1_000);
        pilot.decide(&picture, &hold_doctrine(), NOW + 1_100);
        let notes = pilot.take_notes();
        assert_eq!(
            starting(&notes, "No room behind: "),
            ["No room behind: waiting 420 m from BA's last plot."],
            "{notes:?}"
        );
        assert!(starting(&notes, "Out of ").is_empty(), "{notes:?}");
        assert!(matches!(
            pilot.reapproach.as_ref().map(|r| r.stage),
            Some(ReapproachStage::Wait { since_ms }) if since_ms == NOW + 1_100
        ));

        // Case (c): held ground behind stops the reverse; the back-off ends
        // at the next decision.
        let (mut picture, mut pilot) = reapproaching();
        hold(&mut picture, ba_from(MID, NOW, 0));
        exchange(&mut picture, OUT_TAKEN, OUT_SCORED, NOW);
        pilot.decide(&picture, &hold_doctrine(), NOW);
        pilot.take_notes();
        pilot.sent_heading(90, NOW);
        pilot.sent_speed(-27);
        pilot.held_marks.push(HeldMark {
            at_m: MID,
            heading_deg: 90,
            astern: true,
            blocked_m: along(MID, 270.0, HELD_MARK_AHEAD_M),
        });
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW + 1_000);
        assert_eq!(orders.speed, Some(0), "stopped by the mark");
        assert_eq!(pilot.rule(), Some("reapproach"));
        assert_eq!(pilot.back_off_blocked_ms, NOW + 1_000);
        assert!(pilot.take_notes().is_empty());
        pilot.sent_speed(0);
        pilot.decide(&picture, &hold_doctrine(), NOW + 2_000);
        assert_eq!(
            pilot.take_notes(),
            ["No room behind: waiting 420 m from BA's last plot."]
        );
        assert_eq!(pilot.rule(), Some("approach_wait"));
    }

    /// **The return's back keeps its bow and its plot** (cut AO.1): BA seen
    /// again during `Back` nearer than the stored plot steers the bow onto it
    /// and leaves the plot, and `Back` still ends at the standoff from that
    /// plot; seen farther, the plot follows it; with no own position the wait
    /// names no distance.
    #[test]
    fn the_returns_back_keeps_its_bow_and_its_plot() {
        let own = (1_000.0, 970.0);
        let stored = (1_300.0, 970.0);
        let backing = || {
            let (picture, mut pilot) = following_at(own);
            pilot.ret = Some(Return {
                d: "BA".into(),
                plot_m: stored,
                stage: ReturnStage::Back { since_ms: NOW },
            });
            (picture, pilot)
        };
        // Nearer: BA 200 m off at 070°.
        let (mut picture, mut pilot) = backing();
        let ba = row_at(own, "BA", Some(Side::Blue), 70, 200, NOW, 0);
        let at = ba.fixes.back().expect("a fix").pos_m;
        hold(&mut picture, ba);
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!((orders.heading, orders.speed), (Some(70), Some(-27)));
        assert_eq!(pilot.rule(), Some("return_back"));
        assert_eq!(pilot.ret.as_ref().map(|r| r.plot_m), Some(stored));
        pilot.sent_heading(70, NOW);
        pilot.sent_speed(-27);
        // At the standoff from the stored plot, BA still nearer: the wait.
        let out = (stored.0 - RETURN_STANDOFF_M, 970.0);
        status_at(&mut picture, out, 70, NOW + 30_000);
        let range = distance_m(out, at).round() as u32;
        let bearing = whole(bearing_deg(out, at));
        hold(
            &mut picture,
            row_at(out, "BA", Some(Side::Blue), bearing, range, NOW + 30_000, 0),
        );
        assert!(f64::from(range) < RETURN_STANDOFF_M);
        pilot.decide(&picture, &hold_doctrine(), NOW + 30_000);
        assert_eq!(pilot.ret.as_ref().map(|r| r.plot_m), Some(stored));
        assert_eq!(pilot.take_notes(), [waiting_note("BA", RETURN_STANDOFF_M)]);
        // Farther: the plot follows it.
        let (mut picture, mut pilot) = backing();
        let ba = row_at(own, "BA", Some(Side::Blue), 90, 400, NOW, 0);
        let at = ba.fixes.back().expect("a fix").pos_m;
        hold(&mut picture, ba);
        pilot.decide(&picture, &hold_doctrine(), NOW);
        let plot = pilot.ret.as_ref().expect("in force").plot_m;
        assert!(distance_m(plot, at) < 1e-6, "{plot:?}");
        // No own position at the wait.
        let (mut picture, mut pilot) = backing();
        picture.own.pos_m = None;
        pilot.decide(&picture, &hold_doctrine(), NOW + RETURN_BACK_MS);
        assert_eq!(pilot.take_notes(), ["Waiting for BA to come back."]);
        assert!(matches!(
            pilot.ret.as_ref().map(|r| r.stage),
            Some(ReturnStage::Wait { .. })
        ));
    }

    /// BA with no track, `range` m due east of `own`, its fixes `fixes`.
    fn trackless_ba(own: (f64, f64), range: u32, fixes: &[(u64, (f64, f64))]) -> Contact {
        let mut ba = row_at(own, "BA", Some(Side::Blue), 90, range, NOW, 0);
        ba.fixes = fixes
            .iter()
            .map(|(at_ms, pos_m)| Fix {
                at_ms: *at_ms,
                pos_m: *pos_m,
                sigma_m: 55.0,
                from_lock: false,
            })
            .collect();
        ba
    }

    /// **A hurt machine leaving with no track is broken** (cut AO.1): BA hit
    /// `BROKEN_HITS_MIN` times, no track, its fixes 450 m and then 450 m and
    /// `BROKEN_OPEN_M` from the still tank over 8 s, is broken and followed;
    /// the same fixes of a still machine while the tank itself reversed
    /// `BROKEN_OPEN_M` are not.
    #[test]
    fn a_hurt_machine_leaving_with_no_track_is_broken() {
        let own = (1_000.0, 970.0);
        let near = along(own, 90.0, 450.0);
        let far = along(own, 90.0, 450.0 + BROKEN_OPEN_M);
        let (mut picture, mut pilot) = following_at(own);
        let range = (450.0 + BROKEN_OPEN_M) as u32;
        let ba = trackless_ba(own, range, &[(NOW - 8_500, near), (NOW - 500, far)]);
        assert!(pilot.leaving(&ba, own, NOW));
        hold(&mut picture, ba.clone());
        scored(&mut picture, "BA", BROKEN_HITS_MIN, NOW);
        pilot.decide(&picture, &hold_doctrine(), NOW);
        assert!(pilot.broken(&picture, &ba, own, NOW));
        assert_eq!(pilot.take_notes(), [following_note("BA")]);
        assert!(pilot.follow.is_some());
        // A still machine, the tank having reversed: the same world point.
        let reversed = along(own, 270.0, BROKEN_OPEN_M);
        let (mut picture, mut pilot) = following_at(reversed);
        let ba = trackless_ba(reversed, range, &[(NOW - 8_500, near), (NOW - 500, near)]);
        assert!(!pilot.leaving(&ba, reversed, NOW));
        hold(&mut picture, ba.clone());
        scored(&mut picture, "BA", BROKEN_HITS_MIN, NOW);
        pilot.decide(&picture, &hold_doctrine(), NOW);
        assert!(!pilot.broken(&picture, &ba, reversed, NOW));
        assert_eq!(pilot.follow, None);
        assert!(starting(&pilot.take_notes(), "Following").is_empty());
        // Opened by less than `BROKEN_OPEN_M`, or inside the span: not leaving.
        let short = along(own, 90.0, 450.0 + BROKEN_OPEN_M - 1.0);
        let ba = trackless_ba(own, range, &[(NOW - 8_500, near), (NOW - 500, short)]);
        assert!(!pilot.leaving(&ba, own, NOW));
        let ba = trackless_ba(
            own,
            range,
            &[
                (NOW - 500 - BROKEN_OPEN_SPAN_MS + 1, near),
                (NOW - 500, far),
            ],
        );
        assert!(!pilot.leaving(&ba, own, NOW));
        // A fix older than the window counts for nothing.
        let ba = trackless_ba(
            own,
            range,
            &[(NOW - BROKEN_OPEN_WINDOW_MS - 1, near), (NOW - 500, far)],
        );
        assert!(!pilot.leaving(&ba, own, NOW));
    }

    /// **An implausible track is no evidence** (cut AO.1): a hurt BA 50 m off
    /// with a 181 m/s track going away and its fixes still is not broken; the
    /// same track at a stock tank's speed is.
    #[test]
    fn an_implausible_track_is_no_evidence() {
        let own = (1_000.0, 970.0);
        let still = along(own, 90.0, 50.0);
        let fly = |speed_mps: f64| {
            let (mut picture, mut pilot) = following_at(own);
            let mut ba = trackless_ba(own, 50, &[(NOW - 8_500, still), (NOW - 500, still)]);
            ba.track = Some(Track {
                course_deg: 90.0,
                speed_mps,
            });
            hold(&mut picture, ba.clone());
            hurt_event(&mut picture, "BA", NOW - 100);
            pilot.decide(&picture, &hold_doctrine(), NOW);
            let broken = pilot.broken(&picture, &ba, own, NOW);
            (broken, pilot.follow.is_some(), pilot.take_notes())
        };
        let (broken, followed, notes) = fly(181.0);
        assert!(!broken && !followed, "{notes:?}");
        assert!(starting(&notes, "Following").is_empty());
        const { assert!(BROKEN_TRACK_MAX_MPS < 181.0) };
        let (broken, _, _) = fly(BROKEN_TRACK_MAX_MPS);
        assert!(broken, "a stock tank's speed is evidence");
    }

    /// **A broken machine inside the keep-out is not followed** (cut AO.1):
    /// only the keep-out's sentence, the return begun, and no `Following`.
    #[test]
    fn a_broken_machine_inside_the_keep_out_is_not_followed() {
        let own = (ENTRY_LINE_X - ENTRY_KEEP_OUT_M - 300.0, 970.0);
        let (mut picture, mut pilot) = following_at(own);
        let ba = ba_east(own, 300, NOW, Some(90.0));
        let plot = ba.fixes.back().expect("a fix").pos_m;
        hold(&mut picture, ba);
        hurt_event(&mut picture, "BA", NOW - 100);
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!(pilot.take_notes(), [not_following_note("BA")]);
        assert_eq!(pilot.follow, None);
        assert_eq!(
            pilot.ret,
            Some(Return {
                d: "BA".into(),
                plot_m: plot,
                stage: ReturnStage::Back { since_ms: NOW },
            })
        );
        assert_eq!(pilot.rule(), Some("return_back"));
        assert_eq!(orders.speed, Some(-27));
    }

    /// **A return held short waits out a machine still leaving** (cut AO.1):
    /// the keep-out's return stopped at once by a `Held:` waits with BA still
    /// in sight, broken and driving home; BA's receding rows neither arm the
    /// halt nor let the follow's begin back off again (one room hold, one
    /// keep-out note, for the whole wait), and BA's row read nearer than the
    /// plot the wait faces is fired on from the halt.
    #[test]
    fn a_return_held_short_waits_out_a_machine_still_leaving() {
        let own = (ENTRY_LINE_X - ENTRY_KEEP_OUT_M - 300.0, 970.0);
        let (mut picture, mut pilot) = following_at(own);
        hold(&mut picture, ba_east(own, 300, NOW, Some(90.0)));
        hurt_event(&mut picture, "BA", NOW - 100);
        pilot.decide(&picture, &hold_doctrine(), NOW);
        assert_eq!(pilot.take_notes(), [not_following_note("BA")]);
        pilot.sent_speed(-27);
        edge_event(&mut picture, NOW + 1_000);
        let t = NOW + 1_100;
        hold(&mut picture, ba_east(own, 350, t, Some(90.0)));
        pilot.decide(&picture, &hold_doctrine(), t);
        let notes = pilot.take_notes();
        assert_eq!(
            starting(&notes, "No room behind for the return: "),
            ["No room behind for the return: waiting 350 m from BA's last plot."],
            "{notes:?}"
        );
        pilot.sent_speed(0);
        // BA still in sight, still driving home, for the next ten decisions
        // (from the end of the `Held:`'s own stop, step 1).
        let mut notes = Vec::new();
        for k in 1..=10_u32 {
            let t = t + HELD_FOR_MS + u64::from(k) * 1_000;
            hold(&mut picture, ba_east(own, 350 + 8 * k, t, Some(90.0)));
            let orders = pilot.decide(&picture, &hold_doctrine(), t);
            assert!(orders.speed.is_none_or(|kph| kph == 0), "{k}: {orders:?}");
            let said = pilot.take_notes();
            assert_eq!(
                pilot.rule(),
                Some("return_wait"),
                "{k}: {:?} {:?} {:?} {said:?}",
                pilot.ret,
                pilot.halt,
                pilot.follow,
            );
            notes.extend(said);
        }
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(pilot.halt, None);
        assert_eq!(pilot.follow, None);
        assert!(matches!(
            pilot.ret.as_ref().map(|r| r.stage),
            Some(ReturnStage::Wait { since_ms }) if since_ms == t
        ));
        // BA comes back past the plot the wait faces: fired on from the halt.
        let back = t + HELD_FOR_MS + 11_000;
        hold(&mut picture, ba_east(own, 300, back, Some(270.0)));
        pilot.decide(&picture, &hold_doctrine(), back);
        let notes = pilot.take_notes();
        assert_eq!(
            notes.first().map(String::as_str),
            Some("Firing from the halt at BA, 300 m off."),
            "{notes:?}"
        );
        assert_eq!(pilot.ret, None);
        assert!(starting(&notes, "Not following").is_empty(), "{notes:?}");
    }

    /// **A followed row is flown to the stale limit under `hunt`** (cut AO.1):
    /// the followed BA's row 4 s old — past a `hunt` doctrine's fresh limit,
    /// inside `HUNT_STALE_PLOT_MS` — is still flown: the follow's hold, its
    /// rule word.
    #[test]
    fn a_followed_row_is_flown_to_the_stale_limit_under_hunt() {
        let own = (1_000.0, 970.0);
        let far = (FOLLOW_BAND_M + HOLD_SLACK_M + HOLD_HYSTERESIS_M + 25.0) as u32;
        let doctrine = hunting(450);
        let (mut picture, mut pilot) = following_at(own);
        hold(&mut picture, ba_east(own, far, NOW, Some(90.0)));
        hurt_event(&mut picture, "BA", NOW - 100);
        let orders = pilot.decide(&picture, &doctrine, NOW);
        assert_eq!(pilot.take_notes(), [following_note("BA")]);
        assert_eq!(pilot.rule(), Some("follow"));
        assert_eq!(orders.speed, Some(54));
        pilot.sent_speed(54);
        let t = NOW + 2_000;
        let age = 4_000;
        assert!(age > FRESH_PLOT_MS && age <= HUNT_STALE_PLOT_MS);
        let mut ba = ba_east(own, far, t, Some(90.0));
        ba.age_ms = age;
        hold(&mut picture, ba);
        let orders = pilot.decide(&picture, &doctrine, t);
        assert_eq!(pilot.rule(), Some("follow"));
        assert_eq!(pilot.hold_motion, HoldMotion::Ahead, "the follow's hold");
        assert!(orders.speed.is_none_or(|kph| kph > 0), "{orders:?}");
        assert!(pilot.follow.is_some());
        assert_eq!(pilot.hunt, None);
    }

    /// The pilot's orders after the first `CONTACTS` reply (its `lines`,
    /// read 100 ms after power at [`ROOMY_MUSTER`]).
    fn first_reply(lines: &[&str]) -> (Pilot, Orders) {
        let mut picture = running();
        picture.apply(POWER, &classify("Power online."));
        let mut pilot = Pilot::with_knobs(Knobs::on());
        pilot.ceilings_asked = true;
        decide_at(&picture, &mut pilot, POWER + 50);
        status_at(&mut picture, ROOMY_MUSTER, 90, POWER + 100);
        for line in lines {
            picture.apply(POWER + 100, &classify(line));
        }
        picture.settle(POWER + 300);
        let orders = decide_at(&picture, &mut pilot, POWER + 300);
        (pilot, orders)
    }

    /// **A wingman keeps its opening beside a teammate** (cut AO.1): an
    /// unknown 20 m off, an own-side row at 40 m or 300 m, a notice for an
    /// unknown then held 20 m off, and a notice for one identified as the own
    /// side before the first reply (held under its new designator) leave the
    /// opening on (the brief asked); an
    /// unknown 500 m off, and a notice whose contact the first reply did not
    /// hold, end it.
    #[test]
    fn a_wingman_keeps_its_opening_beside_a_teammate() {
        for lines in [
            &["X1  unknown  bearing 090° (rel 000°)  close  20 m  0 s"][..],
            &["RB  red  bearing 000° (rel 270°)  close  40 m  0 s"][..],
            &["RB  red  bearing 000° (rel 270°)  effective  300 m  0 s"][..],
            &[
                "New contact X1: bearing 045°, close.",
                "X1  unknown  bearing 045° (rel 315°)  close  20 m  0 s",
            ][..],
            &[
                "New contact X1: bearing 045°, close.",
                "Contact X1 identified: RB, red.",
                "RB  red  bearing 045° (rel 315°)  close  30 m  0 s",
            ][..],
        ] {
            let (pilot, orders) = first_reply(lines);
            assert!(orders.ask_view_brief, "{lines:?}");
            assert!(
                matches!(pilot.opening, OpeningStage::Brief { .. }),
                "{lines:?}: {:?}",
                pilot.opening
            );
        }
        for lines in [
            &["X1  unknown  bearing 090° (rel 000°)  far  500 m  0 s"][..],
            &["New contact X1: bearing 090°, far.", "No contacts."][..],
        ] {
            let (pilot, orders) = first_reply(lines);
            assert!(!orders.ask_view_brief, "{lines:?}");
            assert_eq!(pilot.opening, OpeningStage::Done, "{lines:?}");
        }
    }

    /// Every designator in `value` (keys and strings), depth first.
    fn designators_in(value: &serde_json::Value, out: &mut Vec<String>) {
        use crate::arena::lines::is_designator;
        match value {
            serde_json::Value::String(text) if is_designator(text) => out.push(text.clone()),
            serde_json::Value::Array(items) => {
                for item in items {
                    designators_in(item, out);
                }
            }
            serde_json::Value::Object(fields) => {
                for (key, item) in fields {
                    if is_designator(key) {
                        out.push(key.clone());
                    }
                    designators_in(item, out);
                }
            }
            _ => {}
        }
    }

    /// **The new notes name only held designators** (cut AO.1; the bot's own
    /// observer rule): the opening's room notes name none; the back-offs'
    /// holds, the keep-out's begin and the wait with no own position name only
    /// a designator the stream named this match; the rule is a word.
    #[test]
    fn the_new_notes_name_only_held_designators() {
        let mut named: BTreeSet<String> = BTreeSet::new();
        let mut said: Vec<String> = Vec::new();
        let mut check = |pilot: &mut Pilot, picture: &Picture| {
            named.extend(picture.contacts.keys().cloned());
            for note in pilot.take_notes() {
                for word in note.split(|c: char| !c.is_ascii_alphanumeric()) {
                    if crate::arena::lines::is_designator(word) {
                        assert!(named.contains(word), "{word} in {note}");
                    }
                }
                said.push(note);
            }
            assert!(pilot.rule().is_none_or(|rule| {
                [
                    "opening",
                    "halt",
                    "reapproach",
                    "approach_wait",
                    "follow",
                    "return_back",
                    "return_wait",
                ]
                .contains(&rule)
            }));
        };
        // The opening's room notes.
        let (picture, mut pilot, _) = stepping(MUSTER, &muster_brief());
        check(&mut pilot, &picture);
        let brief = rays(|word, deg| match deg {
            90 => {
                format!("{word}: 30 m of road, then deep water to 130 m, then beyond the window.")
            }
            _ => format!("{word}: 90 m of grass, then beyond the window."),
        });
        let (picture, mut pilot, _) = stepping(MUSTER, &brief);
        check(&mut pilot, &picture);
        // The re-approach held at the edge.
        let own = (EDGE_BACK_MARGIN_M * 0.4, 1_000.0);
        let (picture, mut pilot, _) = out_hit_at(own, 90);
        pilot.decide(&picture, &hold_doctrine(), NOW);
        check(&mut pilot, &picture);
        pilot.decide(&picture, &hold_doctrine(), NOW + 500);
        check(&mut pilot, &picture);
        // The keep-out at the follow's begin, then the return held by the
        // edge with BA no longer held.
        let own = (ENTRY_LINE_X - ENTRY_KEEP_OUT_M - 300.0, 970.0);
        let (mut picture, mut pilot) = following_at(own);
        hold(&mut picture, ba_east(own, 300, NOW, Some(90.0)));
        hurt_event(&mut picture, "BA", NOW - 100);
        let orders = pilot.decide(&picture, &hold_doctrine(), NOW);
        check(&mut pilot, &picture);
        if let Some(kph) = orders.speed {
            pilot.sent_speed(kph);
        }
        picture.contacts.clear();
        edge_event(&mut picture, NOW + 1_000);
        pilot.decide(&picture, &hold_doctrine(), NOW + 1_100);
        assert!(!picture.contacts.contains_key("BA"), "BA is not held");
        check(&mut pilot, &picture);
        // The wait with no own position.
        let (mut picture, mut pilot) = following_at((1_000.0, 970.0));
        hold(&mut picture, ba_east((1_000.0, 970.0), 300, NOW, None));
        pilot.ret = Some(Return {
            d: "BA".into(),
            plot_m: (1_300.0, 970.0),
            stage: ReturnStage::Back { since_ms: NOW },
        });
        pilot.decide(&picture, &hold_doctrine(), NOW);
        check(&mut pilot, &picture);
        picture.contacts.clear();
        picture.own.pos_m = None;
        pilot.decide(&picture, &hold_doctrine(), NOW + RETURN_BACK_MS);
        check(&mut pilot, &picture);
        for head in [
            "Opening: moving in ",
            "Opening: no way in ",
            "No room behind: ",
            "Not following BA",
            "No room behind for the return: ",
            "Waiting for BA to come back.",
        ] {
            assert!(starting(&said, head).len() == 1, "{head}: {said:?}");
        }
    }

    /// **The time bounds cover the drive** (cut AO.1), the arithmetic from
    /// the named figures: `help cockpit`'s 27 kph astern, scaled as grass caps
    /// 54 to about 41, reached about 7 s after `SPEED -27` (the bot's own
    /// transcript; counted here as standing still); the re-approach's back-off
    /// from a row reading 400 m (the help's 420 m on the scope) to
    /// `REAPPROACH_OPEN_M`, and the return's from `FOLLOW_BAND_M` to
    /// `RETURN_STANDOFF_M`, each inside its bound.
    #[test]
    fn the_time_bounds_cover_the_drive() {
        const ASTERN_KPH: f64 = 27.0;
        const AHEAD_KPH: f64 = 54.0;
        const GRASS_KPH: f64 = 41.0;
        const RAMP_S: f64 = 7.0;
        const ROW_READS_M: f64 = 400.0;
        let mps = ASTERN_KPH * GRASS_KPH / AHEAD_KPH / 3.6;
        let reapproach_s = RAMP_S + (REAPPROACH_OPEN_M - ROW_READS_M) / mps;
        assert!(
            reapproach_s * 1_000.0 <= REAPPROACH_MAX_MS as f64,
            "{reapproach_s:.1} s"
        );
        let return_s = RAMP_S + (RETURN_STANDOFF_M - FOLLOW_BAND_M) / mps;
        assert!(
            return_s * 1_000.0 <= RETURN_BACK_MS as f64,
            "{return_s:.1} s"
        );
        // And the opening's room is the longest back-off the pilot orders,
        // from the help's stand-off, with its margin.
        const { assert!(OPENING_ROOM_M >= REAPPROACH_OPEN_M - MACHINE_STANDOFF_M) };
    }
}
