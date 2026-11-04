# 23. Cut notes — IC sorties

One `## <cut>` heading per cut of [23-ic-sorties.md](23-ic-sorties.md) from §17 on, in the order
they land: what landed, what it deferred and why, how it was verified. S.0–S.4a are recorded in
the doc's own §14.

## S.4b-T (2026-11-03) — the testimony producer (§17.8, §17.10; D-23-19, Q-23-16, Q-23-17)

Built on `claude/compassionate-neumann-add73f` off `main` `8d983ce2`, one PR, wave 0 of doc 38
§9. Touches no kernel file and no hash pin: the producer is a world-side reader of the chunk the
bridge already parses.

- **The producer** (`crates/tacsim/src/field/testimony.rs`): `on_chunk` (`arena_bridge.rs`) hands
  every chunk's parsed events to `field::testimony::note_chunk` beside `standing::note_chunk`. A
  chunk of any task the `FieldBook` does not host (a SIM match) is ignored, so every arena mode
  still earns nothing. For a field's chunk — the log `field.<site>.<ordinal>.<segment>` — each
  `Classified { viewer, target }` line whose draws carry its own `classify:<viewer>:<target>:`
  label (`own_sensors`) is admitted for its **author**: the viewer's `FieldUnit.pilot` (the
  admission pilot), else its party's `FieldParty.leader`, which is the entrant's leader that
  `admit::custody_unit` seals under (the warrant principal, D-23-5). A mesh classification (no
  draw) and a `Detected` line earn nothing; a unit of a party nobody leads, with no pilot, earns
  for nobody.
- **The index** (`crates/tacsim/src/minds/earned_contacts.rs`): the module head's "No producer
  yet" is replaced by the producer's description. `EarnedContacts` is now registered by the field
  host (`field::register`), not by the quirk claim's composition: the producer needs no Registry
  and no licensed mind (§17.8), and `has_earned` keeps its owner check. `admit` now says whether
  the ring changed (a repeat of the newest hull does not).
- **Persistence**: one `EarnedRecord { format: 1, author, hulls }` per author at
  `tacsim:earned:<author>` (`deny_unknown_fields`, the identity rule), the whole ring of 256
  written in the chunk's bundle (`push_op`, the module lane the chunk's own ledger append rides)
  and only when the ring changed. `earned_contacts::read` restores every ring at
  `on_world_loaded`, after the crew records; a document that does not decode or names another
  character than its key is skipped with a warning and costs only that author's ring.
- **Wording**: no new player line. `@mind/claim`'s refusal is still `+hull/box`'s sentence
  (`NO_BOX`) and the charter's is the driver's one sentence. The `@mind` row of
  `legacy-handlers.toml` and the claim's comments say where the index is fed from now.

### Tests

- `tacsim` lib (`field::testimony::tests`, `minds::earned_contacts::tests`): the provenance rule
  (own draw, mesh share, another pair's draw, a detection); the author rule (pilot, leader,
  nobody); **the producer over a field's chunk on a composed world** — the rig that told the
  raider apart earns it for its pilot in one ring document, its party-mate that held it only over
  the mesh earns nothing for its leader (the blind radio recipient), a rig that only detected it
  earns nothing, a bystander nothing, another task's chunk nothing, the same chunk again writes
  nothing, and the document reads back at boot (one naming another author is skipped); the
  ring's document round trip.
- `tacsim --test shard_2` (`testimony::`), new module, both sizes:
  `s4bt_a_rig_earns_what_it_told_apart_in_a_field_at_256` (Ahuna Flank) and `…_at_512` (Urvara
  Reach). The keyed field rig rebooted under the mind commerce with the Registry inspection and
  the quirk claim composed (`support/field/claim_rig.rs`, the ported twin of shard_9's
  `reboot_with_inspection`; `driving_rig::Rig` gains an optional `commerce` every reboot
  composes); the content's raider seeded and launched beside the prospector's rig, its leader's
  Hostile mark through `+field/mark`; the field runs until the rig's own pass classifies a raider
  hull. Then, and again after a crash and reboot mid-field: the index is exactly the log's
  own-sensor lines by author (every earned pair backed by one; every own pair earned; the
  bystander nothing), and through `@mind/claim`'s command path the operator naming the raider
  hull its rig told apart passes the observer rule (the reply is the charter's refusal, not
  `NO_BOX`), naming a hull its units never told apart is `NO_BOX`, and the bystander naming
  either raider hull is `NO_BOX`; nothing is queued.
- The commerce's preparation validates `[tacsim]` as production does, so the story's tune is
  `field_raiders`' fast field with `substep_max_ms` at the engaged pulse and a 1 s snapshot (the
  lab tune's 250 ms substep and 500 ms snapshot are outside the checked bounds); and the Roll's
  `initialize_commerce` lands on the composed path (`land_composed`), whose context initializes
  the commerce shells. The bystander and the Registry operator are players on gateway accounts
  of their own (`driving_rig::pilot`), since `@mind` resolves its principal through the account.
- Placement (doc 43 rule #3 and §10.2): shard_2, the cell with the most room (11 summed minutes
  at `f146734e`); it already declares the field rigs, and `claim_rig` is declared in every root
  like the other `*_rig` modules so the story can move without an import pinning it.

### Deferred, and why

- **A filed claim** ("Your claim is filed." and the operator's `@registry/claims` read of it):
  the author must hold a mind charter title, which only the Registry's filing of a rated mind
  mints (the M8 world's `certify`); no field world composes that, and no story seeds a title by
  hand. The stories stop at the observer rule, the half this cut owns; the filed positive control
  rides S.4b's harness (the field world over M8e's fitted roster and the filed commerce, which
  §17.9 already names as S.4b's first task).
- **M8g's "To S.4b" stories** (17h M8g "Still owed": the pending-transfer refusal and the IC
  audit over a draft claim) need S.4b, not this cut: the refusal is the seal's at `go`
  (`surface/dispatch.rs`'s sentence) and the audit reads the sortie claim's row
  (`minds::ic_sortie`), and both run over the claim Open S.4b's production `HostTransition`
  mints (D-23-13, D-23-21). §17.8's "the two stories run over a real claim" reads that claim, so
  none of them can run without S.4b; they stay owed there.
- **A live blind radio recipient of a different author, and a live detect-only rig**: a field's
  comms mesh shares only within a party, and the only multi-unit party any field story admits is
  the raider (no roster party is staged). The live log does show the mesh at work — at both sizes
  the raider's carrier held every contact it had only over the mesh (classifications with no
  draw), and the rig detected before it classified — but the carrier's author is the raider's
  leader, who earns the same hulls through the raider's own hulls. The negative cases with a
  distinct author are therefore the composed-world producer test above, over the same
  `note_chunk`; the live stories assert the log/index identity for whatever the field produced.

### Cost

Local (Windows, `-j 8`, a sibling build sharing the box): the two shard_2 stories ran in 60.7 s
of wall, both together, each about 60 s; the five lib tests 0.5 s. So shard_2's summed minutes
rise by about 2 (11 → ~13 at `f146734e`'s figure), still the cell with the most room; the PR
run's JUnit figure is the one rule #14 asks for, recorded in the PR body once its run lands.

## S.4b-1 (2026-11-03) — the protocol's shape, the IC id, the hangar, the title rule (§17.3–§17.5, §17.10; D-23-11, D-23-14, D-23-15, D-23-21..23; Q-23-21, Q-23-22)

Built on `claude/s4b-moving-anchors-c79caa` off `main` `a2ef4075`, one PR, wave 1 of doc 38 §9.
S.4b is built in two slices (§17.10's "Built in two slices"). At `a2ef4075`:

- (a) No world but the claim path's own test worlds composes the IC claim path. `with_ic_minds`
  has no non-test caller, production boot composes no mind commerce, and neither the surface
  harness nor the field harness composes it.
- (b) The exclusion admitted an Open only into a `ConfiguredIcBase` on a `FixedDomain`.
- (c) The book refused `SortieId::Ic` and read its ordinal off the seal.

This slice builds what stands on its own. **Nothing in it is reachable by a player**: no world
composes the claim path, so no `go` mints a permit and no cursor is written outside a test.

- **D-23-22, the title rule** (`surface/dispatch.rs`, `untitled_refusal`, `MIND_UNTITLED`). A
  filed mind that will fly (`Resolved::Held` with a licence) is refused at `go` with the one
  sentence when the hull's economy title is not the operator's own and unscoped (`titled_to`,
  the fields the Roll's `TitleFence` checks). The refusal comes after the word's own refusals
  and before any seal.
  - It applies **only where the claim path is composed**
    (`minds::ic_sortie::claim_path_composed`: the seal owner's book is registered). The refusal
    guards the claim's Open. Without the claim path the held mind is S.4a's dormant seal, which
    opens nothing.
  - Applied unconditionally, it would break doc 17h M8g's shipped stories, none of which opens a
    claim: `m8_world.rs`'s tier-2 story flies `mind=filed:` over a leased rig, and the zero-PC
    story flies over organisation-titled hulls.
  - A draft opens no claim and a licence fallen to the ROM flies no mind, so neither is read.
    Leases are S.4c's (D-23-22).
- **The seals' armament** (`match_::seal_armament`; `seal_held`, `seal_rom`, the DT launch's
  `epoch_facts`). Each now passes the hull's armament summaries, the same list the field's
  `custody_unit` and `Execution::capture` pass, so a dispatch seal of an armed rig digests the
  same as the field's (D-23-14). Draft 0.11 found every dispatch seal taken over an empty list.
- **The book's ordinal and the IC id** (`minds/ic_sortie/records.rs`, `state.rs`, `audit.rs`,
  `participant.rs`, `execution.rs`; round 2's 23-7).
  - `Opening.transition: u64` is the book's ordinal, set by the participant's Open from
    `next_transition`. The book's load orders by it and its `opening` checks it.
  - `IcSortieOutput::sortie()` returns it. So the Roll's writer check `sortie() == transition()`,
    the Roll's `SortieNumber`, and the `sortie` that tactics custody and the exclusion keep are
    all unchanged.
  - The Close's `transition > open` compares against it, and the audit compares it with the
    Roll's `claim.sortie()`.
  - The record format is 2 (`records::FORMAT`). Format 1 is refused, never migrated: this is
    pre-launch and no claim exists.
  - `Execution::validate` and the book accept `SortieId::Ic { hull, at_ic }` over the sealed
    hull. They also accept the fixture's packed ordinal, which must equal `transition`.
  - `StartPermit.sortie: Option<SortieId>` names the epoch the Open seals under (`None` is the
    fixture's packed ordinal), and `Execution::capture` takes the id.
- **The hangar** (`mush-sim` `ic_exclusion.rs`; D-23-11, D-23-15).
  - `IcExclusionConfig::with_hangars(manifest, participants, bases)` admits an Open for a hull
    `Interior` to the bay: a live interior anchored `MovingObject { host }` on a live carrier.
    `bases` may be empty.
  - The Open records `OpenImage.hangar: HangarImage { space, carrier }` beside `base`, with
    exactly one of the two present. A row held in a base serializes as before, so its digest and
    the claim path's own gate are unmoved.
  - The ancestors' journey refusal covers the carrier. The boot checks require the hangar's
    interior and carrier to stand.
  - **The freeze is unchanged**: no placement write of the hull and no rewrite of its interior.
    D-23-15's re-scope to re-anchoring isn't needed to admit an Open in a bay. It lands in S.4b-2
    with the claim arm that moves a held hull down the ramp.
  - `IcExclusionConfig::new`, the base-only composition, still refuses a bay.
- **The protocol's types** (`surface/excursion.rs`; D-23-13, D-23-21, D-23-23).
  - `RigRecord.claim: Option<Box<ClaimCursor>>`, serde default and skipped when `None`, so every
    S.1–S.3 record decodes byte for byte. The states are `ClaimCursor::{Opening { operator,
    licence, org, launch_ic, tries }, Open { open, operator }, Closing { open, kind, operator },
    Closed { open, close }, Refused { reason, at_ic }}`. `ClaimCloseKind` carries the four close
    kinds.
  - `Queued.launch: Option<QueuedLaunch { launch_ic, mind, org }>`. `RigRecord::check` refuses it
    until S.4b-2 queues one.
  - The relaunch guard at `go`: a launch over a live cursor (`Opening`, `Open` or `Closing`) is
    refused with `CLAIM_STILL_OPEN` ("that rig's mind claim is still open").
- **The bot's vocabulary** (`mush-bot/src/mine/lines.rs`). `launch(n, word)` builds `+tv/<n>
  launch mind=<word> go`, or the stock line with no word, and `MIND_UNTITLED` mirrors the game's
  sentence. S.4b-2's licensed stories send it.
- **The review-lane branch** `claude/youthful-kowalevski-168ddb` (`50cdbe03`) is already an
  ancestor of `origin/main`. Its `prepare.rs` cap is on main, so nothing was folded.

### The seams FR.3 lands on (doc 40 §4.4; D-23-23)

Of doc 38 §9.2's one-writer files (`detachment/sortie/prepare.rs`, `field/carrier.rs`,
`field/admit.rs`), this slice touches only `prepare.rs`, at the DT seal (one argument). The
protocol's shape:

1. **Step 1 (S.4b-2's).** `go` with a `mind=` word, where the claim path is composed, writes two
   things in one bundle and answers `Queued.`; no act starts:
   - `RigRecord.claim = Opening { operator, licence, org, launch_ic: <go's IC instant>,
     tries: 0 }`;
   - `RigRecord.queued = Queued { kind: Launch, launch: Some(QueuedLaunch { launch_ic, mind,
     org }), .. }`.

   The epoch's id `SortieId::ic(tv, launch_ic)` and the excursion's `ExcursionId { tv,
   launch_ic }` are both fixed at `go`. The fire that observes the book's active row writes
   `Open { open, operator }` and starts the queued launch. A refusal for good writes `Refused`
   and clears the queue.
2. **Step 2.** The queued launch starts as today's launch act (`LAUNCH_S`, the ramp).
3. **Step 3 (FR.3's).** At the landing the field reads `RigRecord.claim`:
   - `Some(Open { open, .. })`: S.4b-2's `custody_unit` takes the claim's seal from
     `IcHost::resume(world, open)?.seal()`, and the join installs it with
     `IcRetainedExecution::install`.
   - Anything else (`None`, `Opening`, `Refused`, …): the ROM, as for every entrant today.

   FR.3 can build its unlicensed half reading only `None`.
4. **Step 4 (FR.3's).** A refused join goes back aboard with the cursor untouched. The bay is the
   claim's hangar, and S.4b-2's re-scoped freeze admits that re-parent.
5. **Step 5 (S.4b-2's).** A launch cancelled after `Open`, or a ramp that lands nobody, writes
   `Closing { kind: PoweredDownInBase { base: <the hangar's hull> } }`. The next fire closes it.

`ClaimCursor::is_live()` is the one predicate for "a claim is owed or held": the relaunch guard
uses it now, and the seats' count will in S.4b-2. FR.3's `SiteRecord.pending` counts the queued
launch from step 1 on, so a pending licensed launch is one pending row like any other.

### Tests

- **`tacsim` lib:**
  - `surface::dispatch::tests::s4b1_the_dispatch_seals_an_armed_rig_over_its_armament`: the
    escort's dispatch ROM seal equals `seal_rom_for` over its armament, and the empty list
    digests differently.
  - `surface::dispatch::tests::s4b1_a_licensed_mind_needs_the_hull_titled_where_claims_open`:
    with no claim path nothing is read; with it composed the launch is refused with the sentence.
  - `surface::dispatch::tests::s4b1_the_title_rule`.
  - `minds::ic_sortie::execution::tests::s4b1_an_ic_epoch_id_seals_a_retained_execution`: an IC
    id over its own hull validates and installs; another hull's id, or none, is refused.
  - `surface::excursion::tests::s4b1_the_claim_cursor_and_the_queued_launch`: a record with no
    claim has no key; each state round-trips strictly under its `state` word; `is_live` and
    `open` read as specified; the check refuses a queued launch.
- **`mush-sim` lib:**
  - `ic_exclusion::tests::s4b1_a_carrier_bay_is_a_hangar_where_hangars_are_admitted`. The
    base-only composition refuses the bay. `with_hangars` admits it: the row names the bay and
    the carrier and no base, the hull alone is excluded, the freeze still refuses a placement
    write, and a reboot keeps the hold. A base row has no `hangar` key.
  - The fixture gains a carrier on the field's domain and its bay, beside the base (`setup_in`,
    `Admission`).
- **`tacsim --test shard_9`:**
  - `ic_sortie_surface::s4b1_a_live_claim_cursor_holds_the_rig_in_its_bay`: through the composed
    launch path, each live cursor refuses `+tv/1 launch go` with the rig left in its bay; a
    terminal cursor doesn't.
  - `excursions::put_record` is now `pub(crate)` for this test.
- **The M7 gate** (`genuine_registry_original_purchase_grant_refund_and_paired_recovery`,
  ignored; the CI dispatch runs it). The `OwnedDispatch` branch now opens its claim under
  `SortieId::Ic` through the real four owners:
  - the book's row is format 2 and carries its `transition`;
  - the Roll's claim `sortie` equals it;
  - the resumed seal carries the IC id;
  - a reboot reconstructs it.

  `chartered_owner`'s Close then closes that Open.
- **`mush-bot` lib:** `mine::lines::tests::s4b1_the_launch_line_names_the_mind`.
- The existing claim-path, exclusion, dispatch, excursion and shard_9 suites run unchanged.

### Hash pins (rule #13)

None of doc 40 §4a's pinned files is touched:

- the book and the exclusion aren't pinned;
- the dispatch seals are IC seals (`mind_sealed` lines, no OOC practice hash);
- `match_/seal.rs` is unchanged (`seal_armament` is a new function in `arena_def.rs`);
- no OOC arena installs a retained execution.

The brief's rule for S.4b asks for the harness kind regardless, so it is dispatched on the PR's
branch before the PR is marked ready. The run is linked in the PR.

### Deferred, and why

Everything §17.10 lists beyond the bullets above belongs to S.4b-2:

- the composition and the story harness (fact a);
- the production `HostTransition` and its `go`;
- the surface claim arm and the Close producers;
- the re-scoped freeze (D-23-15's `check_plan` and boot checks);
- custody coexistence and the live-hull Close;
- `custody_unit` from `IcHost::resume`, and the join's install;
- the licence and compute seats (D-23-16);
- the per-row compile drop and the wall (D-23-17);
- the carried memory (D-23-18);
- M8g's "To S.4b" stories;
- the acceptance stories at 256 and 512.

### Cost (rule #14)

The PR run's JUnit figure is recorded here once its run lands (see the PR body).
