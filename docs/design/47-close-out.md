# 47. The close-out: the owner's last rulings, the build gaps and the known defects

Draft 0.1, 2026-11-14, at `origin/main` `114c03694`. A plan, not a survey: each unit surveys its own
ground before it cuts and records it in [the cut notes](47-cut-notes.md). Anchors are file and
directory names; line numbers drift.

## 1. Why this document exists

Docs 44 (minds wired through the game) and 45 (the absent player) are built except doc 44's MW.3
(the go-live, with the owner) and the deferred MW.14b (Q-44-49), and doc 45's AP.11 ship arm. Doc
46's CT train runs on a standing go and changes no behaviour. What is left is a list of owner calls
the units raised, a handful of build gaps that stories had to stand in for with staff fiat, and the
defects found and not fixed. On 2026-11-14 the owner ruled the calls and asked for one cut plan to
close the rest out. This is it.

## 2. The owner's rulings (2026-11-14)

| # | Call | Ruling | Raised by |
|---|---|---|---|
| R-47-1 | The focus-fire order word | **`FOCUS`**, not `CONCENTRATE` ("a lot to type in a battle"). The stored variant `concentrate` stays | MW.14c |
| R-47-2 | Return fire for a ship with a mind sealed aboard | **The house default answers fire under attack**, beside the sealed mind, which keeps the helm and sensors | AP.9 default 1 |
| R-47-3 | `RETURN` and fire | **An absent unit on `RETURN` answers fire under attack and does not stop its return.** Reading taken here (§3): one rule on the ground and in space | AP.9, AP.6 |
| R-47-4 | A leader's order to a member who has gone | The owner asked "wouldn't the leader's order go to the mind if the player is gone?" **Reading taken here: yes.** A leader may order an absent member's unit; the order goes to its mind; the absent-player limits still bind | AP.4a |
| R-47-5 | A part displaced or removed by a refit | **Non-destroyed parts removed go to storage, to be fitted again or sold.** Reverses doc 17f §9's "a second install cannot reuse it" | MW.5 |
| R-47-6 | Hired crew in armed rigs | **Hired crew may be seated in armed rigs** | AP.8 |

Every other default the units took stands as written, among them: a rig nobody sits in is its
mind's in a field whether or not its player is connected (AP.7 default 1); a sealed script that
ignores the order home is not driven home (Q-35-B4); `+sortie/orders` is read by the organiser and
the players flying a slot; a ship's posture is set from aboard her and by no relay; a carrier on
her pad is told `HOLD`; an unarmed rig on `GUARD` is flown as `HOLD`; what a mind launches flies in
the step it lands in (MW.11 default 13); the mind counter's door at the Registry & Co-op Desk;
`[tacsim.minds].workers` two and the simulator's fill native; the laser against the drones is
provisional tuning (MW.12). **FB.4, the retrieval job, is not built**: nobody pays and a unit with
no ramp in reach holds (MW.7's default stands).

## 3. What R-47-2 to R-47-4 mean, read together

- **Fire.** An absent player's unit answers a unit whose fire has arrived on it, under every
  posture. It never fires first and never at a party that is only marked hostile. `GUARD` alone
  also answers for the player's other units (unchanged). The rule is one gate,
  `arena::absent::permits`, read by the field and by space (AP.9's applier), so a rig and a ship
  obey the same words (doc 45).
- **Movement.** `RETURN` keeps going home while it answers; `HOLD` and `GUARD` stay. So the three
  words now differ in where the unit goes and, for `GUARD`, whose quarrel it takes; not in whether
  it defends itself.
- **Customs and pickets.** A ship still heaves to for the customs or picket ship she complies with
  and never answers that ship (AP.9 default 3 stands).
- **A sealed mind in space.** The house default answers the fire; the sealed mind keeps the helm.
  Today the think host answers one think a hull, so this is two sources on one hull: the sealed
  mind for helm and sensors, the house default for the fire channel only.
- **A leader's order.** Today a posture set while the member holds the seat (including the
  five-minute grace) is kept and flown; a new order after that is refused ("command authority does
  not cover the target unit"). Under R-47-4 it is accepted and handed to the unit's mind, bound by
  everything above.

## 4. The build gaps

- **No route for a player to get a cored ship.** `+ship/mind` (MW.13) seals only on a hull with a
  governed core, and no route in play fits one; MW.21's story binds the core by staff fiat, and
  MW.5's stories 2 and 7 were not built for want of it.
- **No hulls for Houses** (MW.1).
- **No shipped tug can tow a raider's line tank** (doc 42b's surrender chain).
- **The late-launch interdiction defect** (found by MW.10; a session was started for it and is idle
  with nothing landed).
- **AP.11's ship arm** (doc 45): AP.9 left `space_ap9::left_on_posture_to_the_picket`,
  `ship_at_the_picket` and `until_ship` for it.

## 5. The known defects

| | Defect | Found by |
|---|---|---|
| D1 | A 3 kg burn in a bubble takes a whole tonne off the hold's propellant pool | MW.21 |
| D2 | Lines over the 120-column floor: `+ship/mind` on a sealed hull (154 columns); the absent-unit notices (120 to 196) | MW.21, AP.11 |
| D3 | The reconnect queue reads each rig's line before the field's opening line | AP.11 |
| D4 | A returning player is not told a `RETURN` rig nobody sat in is already aboard until the close | AP.11 |
| D5 | The recover owed across a restart is lost: the retry list is held in memory only | MW.7 |
| D6 | `mush-server` `rate_queues_and_cancels_a_genuine_registry_preparation` fails now and then | MW.13, MW.19, MW.21 |
| D7 | `docs/design/45-cut-notes.md` has the `## AP.11` section twice | this survey |
| D8 | MW.19's known defect (its item 3, which the nightly passes) and its item 4's missing sentence | MW.19 |

**For the owner, not a defect until ruled:** no carrier can fire on another carrier in a bubble.
Carriers are marked Friendly to each other when they join and no command marks a party (AP.9, by
reading). Whether one carrier should be able to fight another is a design call.

## 6. Units

| Unit | What | Area | Waits on | Done when |
|---|---|---|---|---|
| **CO.1** | The `FOCUS` rename (R-47-1) | `field_commands.rs`, `directive_text`, help | — | `ORDER … ALL FOCUS <t>` does what `CONCENTRATE` did; the stored variant reads `concentrate`; MW.14c's story passes with the new word |
| **CO.2** | Absent units answer fire under attack (R-47-2, R-47-3) | `arena::absent`, `field/`, `space/` (applier, backup seat, think host), the house default script | — | under each posture an absent unit answers a unit whose fire arrived on it and never fires first, on the ground and in space; `RETURN` keeps its course while it answers; a ship with a sealed mind answers by the house default while the sealed mind keeps the helm; AP.6's and AP.9's stories rewritten where they held "`RETURN` never fires" |
| **CO.5** | Hired crew in armed rigs (R-47-6) | `mush_sim::handoff`, `+sortie/seat` | — | a hired NPC under a combat contract is seated in the escort tank and fires under its contract's rules; a prospector seat unchanged |
| **CO.8** | The late-launch interdiction defect | space routes | — | the story that shows it fails on main and passes |
| **CO.9** | Hulls for Houses | content, launch bring-up | — | a House can own and field a hull through the routes a House uses |
| **CO.10** | A tug that can tow a raider's line tank | content, tow | — | the surrender chain's tow lands with a shipped tug |
| **CO.11** | The defect sweep, D1 to D8 | several, small | — | each defect has a test that failed before its fix, or is named with the reason it was left |
| **CO.3** | A leader's order to an absent member's unit (R-47-4) | detachment authority, order book | CO.2 | the order is accepted and flown by the mind under §3's limits; a present member's unit is unchanged |
| **CO.7** | AP.11's ship arm | `tests/domain_field/field_ap11.rs` | CO.2 | the closing story's ship flies her posture under the new `RETURN` rule |
| **CO.4** | Removed parts to storage (R-47-5) | the refit admission (`bay_refit`, `core_refit`, the kernel's two), the `detached` head, inventory | CT.7 (same files) | a displaced or removed part that is not destroyed is stored, can be fitted again, and can be sold |
| **CO.6** | A route to a cored ship | refit, yard, content | CO.4 | a player can have a governed core fitted to a ship by registered commands; MW.21's staff fiat removed; MW.5's stories 2 and 7 built |

**Order.** Wave 1: CO.1, CO.2, CO.5, CO.8, CO.9, CO.10, CO.11, side by side, one writer a file.
Wave 2: CO.3 and CO.7 after CO.2. Wave 3: CO.4 after doc 46's CT.7, then CO.6.

**Collisions to watch.** CO.1 and CO.2 both touch the field; CO.1 is a rename and lands first if
they meet. CO.2 and CO.11's D3/D4 both touch the absent-unit notices: D3 and D4 wait for CO.2. CO.2
may move the mind API (a new house-default ask) or the gate harness's baselines: if so it says so
before it cuts, follows AP.3's path and runs the gate harness. Doc 46's CT.5 (`mush-roll`) and CT.7
(the refit pair) are running: no wave 1 unit edits their files.

## 7. Not in this plan

MW.3, the go-live, with the owner. MW.14b, the Rhai commander, deferred (Q-44-49). Doc 46's
remaining CT units. FB.4 (§2). The dev1 and dev2 redeploy, the ufw closure behind Caddy and backup
and restore, which are operations and the owner's.
