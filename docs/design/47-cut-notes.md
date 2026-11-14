# 47. The close-out — cut notes

One section a unit of [doc 47](47-close-out.md) §6. A unit's session replaces its own "Not built."
line with what it built, under these headings: **Survey**, **Built**, **Tests**, **Divergences**,
**Not built**. Each session edits only its own section, so two units' notes never touch the same
lines.

## CO.1

Not built.

## CO.2

Not built.

## CO.3

Not built.

## CO.4

Not built.

## CO.5

Hired crew in armed rigs (R-47-6). Built 2026-11-14 on `claude/doc47-close-out` (`6cdc49a55`).

**Survey.**

- **What the hand-off is for.** `mush_sim::handoff` is the K6 control-lease machine: one lease
  per `(unit, channel)`, a request staged as a pending handoff with one durable schedule and one
  job, applied by the scheduler at its boundary. A physically seated NPC (`physical_npc_station`:
  an NPC in the console's room, holding the billet) asks through `station::request_npc_control`,
  which `+sortie/seat` calls after it has placed the NPC and written the billet. Launch and recover
  (`detachment/sortie/runtime.rs`) witness the **helm** lease held by the slot's operator, so a
  hire with no helm lease cannot launch.
- **Why one channel.** Cut 5 leased per channel, and the offer (`project_station_offer`) pins one
  `channel` and one `expected_revision`; `station_channel` answered only a console of exactly one
  channel, so a console of several had no offer at all. Cut 16b K.2 made the proxy seat skip the
  request for the same reason. Nothing deeper: the lease, schedule and job are each per channel
  already, so several channels are several leases.
- **The escort tank's console.** Doc 32 cut CF.6c gave an armed rig one pilot console carrying
  every cockpit channel (`port/vehicle.rs` `console_channels`: helm, sensors, fire); an unarmed
  rig's carries the helm alone. A player flies an armed rig without any K6 lease on sensors or
  fire: the cockpit's gate (`arena_bridge` `authorise_seat`) refuses only a lease **another**
  principal holds, so a vacant lease passes.
- **How a hired NPC fires in a field.** Not through the cockpit. Its hull is checked out as
  `Controller::Mind` (`field/admit.rs`), and the field host offers each delivered order to the
  kernel (`field/orders.rs`, `offer_field_detachment`); the mind fires unless the order's
  `hold_fire` binds it (`Arena::holds_fire`). Nothing on that path read a channel or a lease.
- **The existing rule for a hire whose contract does not cover combat.** An activation has one
  duty (`+sortie/activate …=mining|combat|recovery`) and every hired NPC's contract must carry it
  (`activation/preflight.rs`; checked again at the seat, at launch and at the field's
  reconcile). A combat contract never mines (`sortie/npc.rs`, doc 27b FC.1). There was **no rule
  about fire**: no hire could sit at a console that fires at all, because the hand-off refused
  every console of more than one channel. AP.8's story stood a combat hire in a refitted
  prospector by a staff hand.
- **Absent-player rules.** None apply: the NPC is the operator, never absent; the leader's
  absence is AP.8's lapse, unchanged.
- **Persisted records.** Nothing versioned or schema-bearing changes. A seated NPC in an armed rig
  now holds two more rows of the existing control-lease family (sensors, fire), with their
  schedules and jobs, each in the family's existing format. The request's parameters gain an
  optional `also` list (`#[serde(default)]`); a one-channel offer carries no `also` and is the
  offer it was. **Cheap to reverse.**

**Built.**

- **`mush_sim::handoff`.** `npc_channels` is the set a physically seated NPC leases: the
  station's channels as its billet grants them (for a one-channel console exactly
  `station_channel`'s answer). `physical_npc_station` admits any console whose billet grants a
  non-empty set. A physically seated NPC at a console of several channels gets its own offer
  (`project_npc_console_offer`): every channel not yet its own, in one invocation (`channel` the
  first, `also` the rest), vacant leases only; a lease another principal holds or any pending
  handoff on the set projects no offer. The release gives back every lease of the set the NPC
  holds. `prepare_request` stages each lease with its own schedule and job (`request_lease`, the
  old body); `validate` refuses `also` on anything but an NPC console's request and on a repeated
  channel. Every other station keeps cut 5's one-channel offer, untouched.
- **`mush_sim::station::request_npc_control`** answers "already observed" only when the NPC holds
  every lease of its set.
- **`+sortie/seat`** (`detachment/sortie/seating.rs`): a console that carries the fire channel
  takes a hire only under a combat activation (`facts::console_fires`, `facts::fires_by_duty`);
  otherwise it refuses with `NPC_SEAT_NO_FIRE`. Launch checks the same rule again
  (`facts::launch_person`), so a console that came to fire after the seat does not launch.
- **The field's fire gate** (`field/orders.rs`, `may_fire`, `offer`): every order the host offers
  for a hired NPC goes through one door, which sets `hold_fire` when the NPC sits at a console
  that carries the fire channel and does not hold that channel's lease. A console with no fire
  channel is not the gate's.
- **Help**: `docs/help/detachments.md` says an armed rig's console takes control of all three
  channels and that only a combat duty sits at a console that fires.
- **Test rig**: `roster_rig::Billet::HiredArmed` leases the slot's rig `+tv/lease escort go`
  (the treasury pays the captain the difference by a staff mint, since an escort beside a
  prospector is more than a new First Captain holds); `roster_rig::staged` runs `fly`'s steps 1
  to 5 and stops before the seat; `seat` is `pub(crate)`.

**Tests.** `crates/tacsim/tests/domain_field/field_co5.rs`, on AP.8's field scaffold:

1. `co5_a_combat_hire_is_seated_in_the_escort_tank_and_fires_under_its_contract`: Rook Adeyemi
   (combat contract) in the leased escort tank under a combat roster, seated by `+sortie/seat`,
   holds the helm, sensors and fire leases; the roster launches into a field of three parties;
   on `+field/mark` and `2 ENGAGE` the tank presses the order and its rounds go at the intruder.
   No refit, no staff seat.
2. `co5_a_hire_on_a_mining_duty_is_refused_the_escort_tanks_console`: Ilya Mendes (mining
   contract) planned into the escort tank of a mining roster is refused (`NPC_SEAT_NO_FIRE`) and
   holds no billet and no lease.
3. `co5_a_prospector_seat_leases_the_helm_alone`: Mendes in a prospector is seated and holds the
   helm lease and no other.

`mush-sim` `handoff::tests::co5_one_npc_request_stages_every_console_channel`: one request stages
nine operations (three leases, schedules and jobs) and three schedules come due; `also` is refused
on a non-NPC reason and on a repeated channel.

On conclav.dev at the branch head: the three stories, all of `domain_field` (305 passed, 16
ignored), and the `mush-sim` library (86). Mutations, each alone, against the three stories:

| Mutation | Caught by |
|---|---|
| the hand-off drops the fire channel (the NPC console offer skips `fire`) | story 1 |
| a non-combat hire gets it (`+sortie/seat`'s refusal removed) | story 2 |
| the one-channel console path breaks (`npc_channels` answers nothing for one channel) | story 3 |

Locally: clippy on every target of `mush-sim` and `tacsim` under `-D warnings`, CI's M7
memory-size pass on `tacsim`, rustdoc for both, `mush-contract-tests`.

**Divergences.**

- **A hire not on a combat duty is refused the console, not seated on a narrowed one.** The
  billet's narrowing (`StationAssignment::granted`) could have withheld the fire channel and
  seated them, but a boot clears every narrowing (`boot.rs`, E7 cut 10's A6), and the mind of a
  hull checked out to a hire is bound by no `hold_fire` until an order stands, so a narrowed seat
  would not be sure to hold its fire through a restart or before its first order. Refusing the seat is the
  rule that cannot leak. The owner may object: R-47-6 reads "hired crew may be seated in armed
  rigs", and this seats only crew on a combat duty in them.
- **The fire channel follows the activation's duty, not the contract alone.** A contract carrying
  both mining and combat, activated for mining, is refused too: the duty a plan is activated for
  is what the hire was engaged to do on it.

**Not built.**

- **A mind with no order standing.** The field gate binds orders the host offers. A hired NPC's
  hull with no order yet is the mind's, as before CO.5; with the seat's rule only a combat hire
  can be in an armed rig, so no hire not on a combat duty is exposed to it.
- **A staff refit of a hired prospector** (AP.8's stand-in, still used by its stories) leaves a
  one-channel console on an armed hull: the gate does not apply and the hull fires as it did.
  `port/vehicle.rs`'s console boot warns of such a console as changed and leaves it; nothing in
  play makes one.
- **A cancel at a console of several channels** is not offered: the NPC's request applies at the
  instant it is made, so there is nothing to withdraw.

## CO.6

Not built.

## CO.7

Not built.

## CO.8

Not built.

## CO.9

Not built.

## CO.10

Not built.

## CO.11

Not built.

## CO.12

Not built.
