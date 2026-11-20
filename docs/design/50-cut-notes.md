# 50. Law and the corridor — cut notes

One section a unit of [doc 50](50-law-and-the-corridor.md) §7. A unit's session replaces its own "Not built." line with
what it built, under these headings: **Survey**, **Built**, **Tests**, **Divergences**, **Not built**.
Each session edits only its own section, so two units' notes never touch the same lines.

The order of work, and which unit waits on which, is the lead's map of 2026-11-19 (in brief):
- **Wave 1, no unit waits:** CR, KC.0a, OP.0.
- **KC:** KC.0a, then KC.0, KC.1, KC.2 (which also waits on CR), then KC.3 and KC.4.
- **LW:** LW.1 waits on KC.2's terms schedule; LW.2 on KC.0; LW.3 on LW.1, LW.2 and CR; LW.4 on
  LW.3; LW.5 on CR and LW.2.
- **OP:** OP.1 on KC.0; OP.2 on OP.1 and KC.0a; OP.3 on OP.2 and CR; OP.4 on OP.3 and KC.2; OP.5 on
  OP.0, OP.4 and LW.1; OP.6 and OP.7 on OP.5; OP.8 on everything.
- **Lane bumps go one at a time:** KC.0, KC.2, KC.4 and OP.1 each change the lane pack.

## CR

The cargo reading, and two live defects beside it from doc 50 §4.3: the picket and the hold's
fill check count a bulk good held as a lot; a seizure can take cargo under a haul's hold and fails
the haul; the 35 km breach records the pilot, not the hull, and writes a ruling.

**Survey** (at `ecb96979d`):
- **The reading.** `read_pool` (`tacsim/src/port_tick.rs:2626-2646`) adds `lot_tally` only for a
  part. Doc 48e had already added `read_held` beside it (`:2658-2675`), which counts lots for every
  good, and the planner, the gauges and the hull sale read through it. The picket's manifest
  (`space/picket.rs:688-704`) and the hold's fill check (`space/mining.rs:1301-1320`, read by the
  mining commands and the mining tick) still read `read_pool`. So a 51 t lot of strategic ore read
  as 0 kg and cleared the picket.
- **The seizure.** The ruling's prepare (`space/customs.rs:451-490`) took
  `min(seize_t, pool.available)` and debited it. `seize_t` is all the unlicensed ore aboard
  (`space/commands.rs:5001`). A haul's cargo is a pool hold in the hull's hold under the job's
  holder (`services/haul.rs:413-463`, `mush-inventory/src/pool/carriage.rs`), so it is not
  available. Ore held as a lot is not in the pool at all.
- **One write a key.** An `ActionTxn` writes a key once (`mush-core/src/action_state.rs`,
  `require_fresh_mutation`), so a `release` then a `debit` of one pool cannot share a bundle.
  Inventory had no call that releases a hold and debits in one write (`carry` is the precedent
  for one-write compound moves).
- **The breach.** `space/tick.rs:4639-4650` called `record_ran_the_picket_incident(ctx, hull,
  hull)` and set the outcome itself. `+helm/run` files a `Ran` ruling (`file_ruling`), and the
  incident follows the landed row, with `ruling.principal` as the pilot (`customs.rs:877-892`).

**Built:**
1. **The cargo reading: `crate::port::ship::hold_reading(ctx, hull, good)`.** It is
   `read_held` on the hull's hold vault, `Ok(0)` for a hull with no hold. The picket's manifest and
   `hull_hold_used_tonnes` read through it. KC.2's duty, LW.3's audit and OP.3's haul handling
   should call it too. `Lot` is unchanged.
2. **`InventoryFamilies::seize(txn, endpoint, commodity, amount, release)`**
   (`mush-inventory/src/pool/seizure.rs`). In one write of the pool's pair it releases the named
   live holds, in order and only while what is available falls short, then debits. It answers
   `Seized { taken, released }`. It credits nothing.
3. **The seizure takes lots and held cargo, and fails the haul.**
   - At the filing, `seizable` lists the live hauls whose strategic ore is held in that hold and
     the exact lots of it there, at most 16 of each. They ride on `PendingCustomsRuling` as
     `hauls` and `lots`, which are serde-defaulted, so the landed finding's shape is unchanged.
   - The prepare (`CustomsProvider::seize`) takes the unheld lots whole while they fit
     (`consume_products`). It then takes the pool through `seize`, naming each haul's
     reservation (`haul_reservation`).
   - For each released hold it writes `HaulLedger { acts: 2, last: LegKind::Seized }` in the same
     transaction. A later delivery or return then fails the ledger's replay guard. The customs
     provider gets the ledger handle from `haul::register`, which now returns it.
   - The haul arm sees `Seized` on the ledger. It calls `mush_economy::lapse_job`, which forfeits
     the bond to the shipper and refunds the fee, as a deadline does. Then it closes the record
     and tells the shipper and the hauler.
   - The seized ore still goes to nobody (LW.5).
4. **The breach is a run ruled as `+helm/run`'s.** The tick calls `customs::file_breach`, which
   files a `Ran` ruling with the hull's pilot as its officer. The pilot is
   `commands::helm_pilot_aboard`: the crew member aboard on a movement station, else the first of
   her crew aboard. The principal is resolved from that pilot. The incident, the corvette's turn
   and the outcome follow the landed row through `apply_landed`, as for `+helm/run`. The crew hear
   that the corvette is logging the run. A ruling already pending waits. A lapsed ruling is filed
   again on the next step, because she is still past the boundary.

**Tests:**
- `mush-inventory` `pool::seizure::tests::cr_a_seizure_releases_the_holds_it_needs_and_takes_held_cargo`:
  a debit cannot reach held cargo. `seize` releases only the named hold it needs and takes the
  rest. An unnamed hold stands, and no pool takes nothing.
- `tacsim` `domain_world` `voyage::cr_the_picket_reads_ore_held_as_a_lot_and_the_seizure_takes_the_lot`:
  51 t of strategic ore held as one lot, none pooled. The picket finds 51,000 kg, the ruling
  seizes 51 t and the lot is gone. Before the unit she was cleared with 0 kg.
- `voyage::cr_a_seizure_takes_cargo_under_a_hauls_hold_and_fails_the_haul`: a real `+haul/post`
  and `+haul/accept` of 51 t, all of it held. The ruling lands, the hold is (0, 0) and the
  ledger reads `(2, Seized)`. The arm closes the job, and the contract is `Expired`. Before the
  unit the ruling did not land.
- `voyage::cr_burning_past_the_picket_is_a_run_ruled_with_the_pilot_as_its_officer`: she burns
  flat out away from the corvette under her own drive (tests may not write the space book). Past
  35 km the tick submits one customs ruling, as the pilot. It lands
  as `Ran` with the pilot as actor and principal, the corvette is hostile and the front advances
  once. Before the unit no ruling was filed.
- Each tacsim story was mutation-checked (the fix taken out, the story fails): `read_held` back
  to `read_pool` in `hold_reading`; `seize` handed no holds; the tick's old direct record.
- Pinned sites moved with the change: the matter census (`domain_economy` `mill::sm12a_only_the_clerk_…`)
  now finds the seizure's `seize` and `consume_products` in place of its `.debit(`, and the bot's
  haul table has rows for `TOLD_SEIZED` and `TOLD_HAULER_SEIZED`.
- Re-run beside them: `voyage::sfr26c_…`, `voyage::sfr12_…`, `voyage::sfr6_…`,
  `domain_economy` `ex6_haul::`, and the tacsim lib's haul, customs and picket tests.

**Divergences:**
- **A new inventory call.** The brief kept CR to tacsim's cargo, customs and picket code. Taking
  held cargo in the ruling's one transaction needs a release and a debit in one write of the
  pair, and only `mush-inventory` can make that write. `seize` is additive, beside `carry`.
- **"Fail the job in the same transaction"** is the job's ledger row, written in the ruling's
  transaction. The contract's lapse follows on the haul arm's next fire. The economy's contracts
  are not action-state families, so no action can write them; the customs fine already follows
  this pattern. A contract that will not lapse then (a frozen economy) lapses at its own deadline.
- **Lots are taken whole.** A lot larger than what is left to take is passed over. Today
  `seize_t` is all the strategic ore aboard, so every unheld lot fits.
- **A breach with nobody of her crew aboard** has no officer to submit a ruling as. `filed_by`
  projects only to an actor aboard. The incident is then recorded at once, as before, but it
  names the hull's principal (`resolve_hull_customs_principal`) as its pilot, not the hull.

**Not built:**
- Where seized goods go (to the enforcing authority's vault, Q-50-44): LW.5.
- The full loss handling for wrecks and captures, and the canon record that advances the other
  side's clock: OP.3.
- **A held lot is not seized.** A lot under a lot hold (a listing, a dispute's evidence) is
  counted by the reading but skipped by the seizure. When all the unlicensed ore is in such a
  lot, the ruling still fails to land, as A-5 did. Releasing a lot hold in the ruling (ending
  the listing that placed it) is left to LW.3 or LW.5.
- **A haul whose hold has lapsed** but whose record is not closed is not failed by a seizure.
  Its ore is free and taken, so the return leg finds nothing and gives up after its tries.
- The fill check's switch to the reading has no story of its own. It calls the same function the
  picket's story mutation-checks.

## KC.0a

Not built.

## KC.0

Not built.

## KC.1

Not built.

## KC.2

Not built.

## KC.3

Not built.

## KC.4

Not built.

## LW.1

Not built.

## LW.2

Not built.

## LW.3

Not built.

## LW.4

Not built.

## LW.5

Not built.

## OP.0

Not built.

## OP.1

Not built.

## OP.2

Not built.

## OP.3

Not built.

## OP.4

Not built.

## OP.5

Not built.

## OP.6

Not built.

## OP.7

Not built.

## OP.8

Not built.
