# 40. Cut notes — the field's second round

One `## FR.n` heading per cut of [40-field-second-round.md](40-field-second-round.md) §6, in the order they
landed: what was built, the story that proves it, where the PR departed from §6's row, and the cell's
summed-minutes delta (doc 43 rule 2.3 #14).

## FR.2 — durable standing (Q-38-24, Q-40-15..18)

**Built.**

- **The tally on the live record** (Q-40-15). `FieldLive.tally: Vec<FireRow { shooter, victim, rounds }>`
  (`#[serde(default)]`, `FIELD_LIVE_FORMAT` 1 kept). `standing::note_chunk` counts into the host's RAM book
  (`FieldBook.tally`) as before; `field::on_snapshot` lays it on the live record in the snapshot's own batch, and
  the boot's resume seeds the book from it, so a resumed field's tally is the snapshot's plus the pulses it
  re-fights. `FireTally` is gone. Admission clears a site's tally; a failed resume forgets it.
- **The window on the grant** (Q-40-16). mush-faction's `Grant` carries `window: Option<VoidWindow { opened_at,
  until_ic }>` (serde default; the store's schema 1 kept), written once by `record_voidable_standing`, which takes
  it as a new last argument; an identical retry keeps the first window (it is not in the conflict compare).
  `void_standing` refuses a windowed grant past `until_ic`. `StandingReceipt.window` reports it, and
  `voidable_causes(w, prefix, by)` lists the open grants a voider may still void. `VoidOffers`, the
  `tacsim.field.standing` tick and `STANDING_TICK` are gone.
- **The synchronous close** (Q-40-17). mush-core's `SessionStat.prev_input: Option<i64>`: `None` from
  `session_start` (a fresh session or a reconnect), `Some(0)` after the first input (nothing before it), then the
  second of the input before the latest. `+field/void` voids every `field:` grant naming the actor whose window
  is open: inside `until_ic`, and `prev_input <= opened_at`. A void typed after another command is refused at
  once; a void typed first after a reconnect is forgiven (named in doc 40 §4.3). The window stays 600 IC s.
- **The director's auto-void** (Q-40-18). `FieldParty.director: bool` (serde default), set at admission or the
  join from `Entrant.director`: a fixture raider's hull whose carrier carries the director's `raid` key then. It is
  fixed at admission because the director clears the key when the carrier lifts, which may be before the field
  demotes. At demotion, a director party's friendly-fire grants are written and then voided in the same batch by
  its leader, with `DIRECTOR_VOID_REASON`; nobody is offered them. A staff raider's leader is offered as before.

**Stories** (`crates/tacsim/tests/shard_2/field_fr2.rs`, shard_2 per doc 43 §10 amendment 2):

- `fr2_ahuna_the_tally_survives_a_restart_and_the_void_is_accepted` and
  `fr2_urvara_the_tally_survives_a_restart_and_the_void_is_accepted`: GC.10's void gate at 256 and 512, with a
  `crash` once a snapshot holds the round; the resumed live record's tally is at least the pre-crash one, the
  demotion writes the one change with its window, and A's `+field/void` with no command between is accepted.
- `fr2_ahuna_a_void_after_another_command_is_refused` and `fr2_urvara_a_void_after_another_command_is_refused`:
  the same field; A types `+field`, then `+field/void`, which is refused and the change stands.
- `fr2_a_director_raids_friendly_fire_is_voided_with_no_hand`: GC.11's tempting site with the prospector and the
  hired guard in the Skulk Syndicate; the director lands, declares and fights; at demotion every change of the
  raid leader's is voided by that leader, with no command typed.
- Unit: `standing::tests` (the window's close on the next input, the reconnect's forgiveness, the tally's
  round trip), `world.rs` `session_stats_track_connect_and_input_times` (prev_input), and the mush-faction
  reputation suite on the new signature.

**Divergences from §6's row.**

- The row names `FieldLive.fire`; that field already holds the pending opening shot (`PendingFire`, doc 32
  §15), so the tally is `FieldLive.tally`.
- The director flag rides the party (`FieldParty.director`), not a read of the carrier's `raid` key at
  demotion: the key goes when the raid is over (`surface::raid::over`), which can precede the demotion.

- The void gates run `[tacsim.arena.ledger].summary_ms = 100`: the black box's 16 KiB chunks close only when full
  (or at the end), and at the default 5 s summary a four-unit field never closes one mid-field, so the round
  would reach the host only after the restart and the gate would not exercise the durable tally. That is also a
  production fact worth naming: the tally on the live record holds what reached the host in closed chunks; rounds
  in the chunk still open at a snapshot are counted when that chunk closes after the resume.

**Cost** (rule 14): five new stories in shard_2, summed locally (Windows, nextest, `-j 8`) at 42 + 42 + 39 + 44 + 67 s
= **+3.9 summed minutes** on shard_2's cell; the CI figure is the PR run's JUnit for `shard_2`.
