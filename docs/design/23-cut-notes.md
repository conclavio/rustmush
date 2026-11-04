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

From the PR's last run's JUnit (`41773658403`, at `6dc72e09`): **+0.07 summed minutes on
`tacsim-integration-9`** — the one new story, `s4b1_a_live_claim_cursor_holds_the_rig_in_its_bay`,
4.0 s; the cell sums 30.9 min. The lib, `rest-libs` and `bot` additions are under a second in all
(`tacsim-lib` 0.77 s over six tests, `rest-libs` 0.03 s, `bot` 0.01 s).

## S.4b-2 (2026-11-04): the claim arm, the claim through the surface, the field's claim seal (§17.3–§17.6, §17.10; D-23-13, D-23-14, D-23-15, D-23-21, D-23-23)

Built on `claude/s4b2-claim-arm` off `main` `c0801a8a` (S.4b-1, #390), one PR. It runs beside doc
40's FR.3 (#400), which builds steps 2–5 of the pending-launch protocol on S.4b-1's seams.

**Three slices, not two.** Building S.4b-2 turned up three gaps, each larger than its line in §17:

1. **The tick cannot use `IcHost::submit`.** `submit` uses `submit_authoritative_observed`, which needs a
   dispatch caller: a player's own command. The surface tick's swapped call has none.
2. **The store's catalog has no rig roots.** The tacsim store holds the plain fielded catalog, so
   `Execution::capture` refused every governed or fitted rig (`unit schema`).
3. **The field half reaches into mush-tactics' four-owner custody.** It needs the check-out under a
   field mark, the field's own write to a held hull, and a Close over the live hull.

This PR closes all three gaps and runs the claim's whole lifecycle through the surface. The field
stories at 256 and 512, the join's install, the seats, the drop and the carried memory are
**S.4b-3**, listed under "Deferred".

### What landed

- **The composition.**
  - `MindCommerce::with_ic_minds_in_bays` and `LaunchComposition::with_ic_minds_in_bays` compose
    the claim path with the exclusion admitting a bay as the hangar
    (`ic_sortie::registration::Composition::with_hangars`).
  - The surface tick gets the composed `IcHost`: `port_tick::register_with_coverage` now registers
    the surface tick itself, after the providers.
  - The IC seal owner reads **the composition's own unit catalog** (`Config.catalog`, set from the
    layer's `definition_catalog` at registration), so a rig root's armament is read, never guessed.
- **`go` (D-23-21, step 1).** Where the claim path is composed, a filed mind that will fly makes
  `dispatch` return `Dispatched::Claim { licence, state }` and takes no seal (the claim's capture is
  the one seal, D-23-14). The start writes `RigRecord.claim = Opening { operator, licence, org,
  launch_ic: now }` and `Queued { kind: Launch, launch: QueuedLaunch { launch_ic, mind, org,
  licence_state } }` in one bundle, then answers `Queued.`. No act starts, and the rig stays in its
  bay.
- **The production `HostTransition`** (`minds/ic_sortie/surface.rs`). `surface_start` mints the
  `StartPermit` from the record under `SortieId::ic(tv, launch_ic)`; `surface_close` mints the
  `CloseEvent` over the claim's Open with an empty plan fenced on the live revision.
  `claim_of`, `claim_closed`, `claim_seal` and `retained_source` read the book; the cursor moves
  only on what the book shows.
- **`IcHost::submit_as_host`.** The claim arm's one closed call, as the operator, through
  `Fire::as_principal` (S.2's epoch-arm precedent). It is the plain authoritative submission of the
  same signed selector under the ticket's own key. The arm resubmits that key on later fires until
  the book shows the transition, so it never reads an attempt state.
- **The claim arm** (`surface/mind_claim.rs`; the tick's `claim` candidate reason, `CLAIM_CYCLE`).
  It takes, as the cursor's operator:
  - **`Opening` with no Open in the book:** mint, enqueue (once a boot) and submit the Open.
  - **`Opening` with the operator's active Open in the book:** acknowledge the Open, then submit
    the queued launch's own start. That start's prepare (`queued_claim`) sets the cursor to
    `Open { open }`, opens the excursion at `go`'s instant, and carries the claim's seal on the act
    (`DispatchSeal.open`), so the open line prints the claim's seal.
  - **`Closing` with the book's row not closed:** mint, enqueue and submit the Close. Once the book
    holds it, nothing more is owed: the relaunch guard reads a `Closing` cursor whose Close the book
    holds as spent (`closed_in_book`).
  - **The tries spent with no Open (D-23-21):** submit the queued launch's start with
    `claim: refused`, whose prepare writes `Refused { reason: CLAIM_REFUSED }` and clears the queue.
    The operator hears `CLAIM_REFUSED` once.
  - The claim's attempt key is kept in the tick's `still_due` set, so its tries survive the prune.
- **Every epoch close owes the claim's Close** (`RigRecord::owe_claim_close`, `Open` → `Closing`),
  with the close line's own kind:
  - `ReturnedToBase`: the recover's `Done` and the staff home into its own hull;
  - `RecoveredToBase`: the home into a second carrier, the parked or abandoned home, and the
    salvage load (Q-23-11);
  - `Destroyed`: `close_destroyed`.
- **The field takes the claim's seal** (D-23-17 (a)).
  - `Entrant.claim` carries the rig record's `Open` token. `custody_unit` seals that entrant under
    `claim_seal(world, open)`; everyone else gets the ROM, as before.
  - `arena_bridge::sealed_sources` also reads the claim's retained source, so the arena installs
    the claim's Rhai runner at admission (the `sources` seam, §17.10).
  - `held_out` no longer walls a hull whose hold is in a hangar (`mush_sim::ic_exclusion::held_in_hangar`).
- **Custody coexistence and the live-hull Close (D-23-15, mush-tactics).**
  - `custody::check_out` admits an IC-held hull under a field mark; a match's mark is still
    refused.
  - The boot check skips a held hull deployed to a field (the field's resume owns it) and otherwise
    accepts a held hull of the Open's object and definition at a revision no older than the Open's:
    its bytes are held only at the Open's own revision (`live_matches`).
  - The Close compares the live hull the same way, and `project_live_close_unit_digest` projects
    over it. The seal participant's Close identity compares object, definition and revision, and
    projects over the live hull too.
- **Refusal reasons for the operator.** The IC owners' error helpers (seal, executable, tactics
  custody, sim exclusion, the Roll's writer and its reader's refusal branches) trace their reason at
  `debug` under the `ic_claim` target. The player's sentence stays generic.

### The exclusion-rule change (D-23-15; for the reviewer)

The freeze on a hold in a hangar is **re-anchoring only** (`check_plan`, `hangar_move_admitted`).
A hold in a fixed base is frozen exactly as before. For a hull held in a hangar:

| Move | Rule |
|---|---|
| A pose on the domain the hull stands on | admitted only on that same domain |
| The ramp from its hangar | admitted only onto the domain its carrier is landed on: the carrier's own placement is `Direct` on that domain and not unplaced, the carrier is under no journey (`JourneyBook::under_way`), and the domain is `Active` |
| A move from one domain to another (a teleport) | refused |
| A re-parent `Interior` into its own hangar, or into any carrier's bay (`MovingObject`-anchored: a recoverer's, S.3; a salvage load's, Q-23-11) | admitted |
| A mount, an unplacing, a fixed room, a projection with no admitted placement, a rewrite of the bay's interior | refused |

Also changed:

- **The boot check:** a hangar hold's hull must stand on a domain or in a bay, never at the Open's
  image, and the hangar's carrier must stand.
- **The journey boot check:** a hangar hold is barred only by a journey rooted at the hull itself;
  the carrier may leave, per F-6.

`check_plan` now reads the **planner's own spatial state**. Inside a plan, the world's slot is not
readable (the first version read `crate::state(world)` and refused every ramp in production while
passing the unit test, which plans with the state left in the world).

Negative tests, in the mush-sim hangar test: a ramp onto a second, unrelated active domain is
refused, and a ramp onto the carrier's own domain is refused once the carrier is not landed.

### Tests

- **`mush-sim` lib:** `ic_exclusion::tests::s4b1_a_carrier_bay_is_a_hangar_where_hangars_are_admitted`
  is extended:
  - `held_in_hangar` names the carrier;
  - the ramp onto the carrier's domain is admitted;
  - the unrelated domain and the unlanded carrier are refused;
  - a fixed room is refused.
- **`mush-tactics` lib:** `ic_custody::tests::s4b2_a_field_mark_coexists_with_the_claim_and_the_close_takes_the_live_hull`:
  - a match's check-out is refused and a field's admitted;
  - the field's check-in writes the hull, and the claim stands through it;
  - `live_matches` accepts the Open's image and the later revision;
  - the Close prepares and installs over the live hull.
- **`tacsim --test shard_9`:**
  - `ic_sortie_surface::s4b2_an_open_refused_for_good_cancels_the_queued_launch`. In the commerce
    harness with the claims composed in the bays (`reboot_with_claims`), every Open the arm submits
    is refused by the Roll's reader (no licence 7 is held). Past the tries the cursor reads
    `Refused`, the queue is gone, the rig never left its bay, the pilot hears `CLAIM_REFUSED` once,
    and a plain launch afterwards is not held.
  - `m8_world::s4b2_a_licensed_rig_opens_its_claim_in_the_bay_and_closes_it_home`. Ignored; the CI
    dispatch runs it, as M8g's stories run. In the combined M8 world with the claims composed in the
    bays, with one genuine rating (Coprates) and the paid Pin over the governed Class II rig:
    1. the leased rig is refused with D-23-22's sentence;
    2. once titled to the pilot, `go` files `Opening` with the launch queued and no act;
    3. the arm opens the claim in the bay (the pilot is its dispatcher) and starts the queued
       launch (the cursor reads `Open`);
    4. the open line's `seal_digest` is the claim's, the licence is the Pin's, and the claim's
       seal sorties under the epoch's IC id;
    5. the recover owes the Close, the arm lands it, the book holds no active claim, and a plain
       launch afterwards is not held.

### Deferred, and why (S.4b-3)

- **The field stories at 256 and 512** (§17.10's acceptance): a licensed rig admitted under its
  claim's seal that thinks in a field on Ahuna Flank and on Urvara Reach. The only world holding a
  filed licence is the M8 world (genuine ratings, Ahuna only). The field-story harness
  (`tests/support/field`) composes no mind commerce. S.4b-3's first task is to compose the two (the
  M8 world's launch over `driving_rig`'s grounds), then run the field half (admission under the seal,
  the field's check-in, the live-hull Close) at both sizes. Its in-source halves are tested above.
- **The join's install** (D-23-23 step 3 for a reinforcement). `arena/join.rs` refuses a scripted
  seal. FR.3's `join_launches` keeps skipping a hull whose claim is `Open` until the arena's join
  installs the claim's runner. That is a kernel file (doc 38 §9.2: `arena/*` is FR.5's), so it is
  coordinated there.
- **The seats at `go` (D-23-16).** Today a second Open under a held licence seat is refused by the
  Roll at the Open and ends `Refused` past the tries. The refusal at `go` with the arithmetic, and
  the compute sum, are S.4b-3's.
- **The drop and the wall (D-23-17 (b)), the carried memory (D-23-18), the install at a field's
  resume, M8g's "To S.4b" stories, and `PoweredDownInBase` for a launch cancelled after its Open.**
  S.4b-2 leaves the cursor no path from `Open` to a cancel; the epoch's close is the only Close.

### Hash pins (rule #13)

None of doc 40 §4a's pinned files is touched. `arena/*` is untouched: the field's install rides
`install_sealed_runners`' existing `sources` seam, and no OOC arena installs a retained execution.
The harness kind is dispatched on the PR's branch before it is marked ready.

### Cost (rule #14)

From the PR run's JUnit (`41815089235`, at `a31f9f49`, green):

- **`tacsim-integration-9`: +0.41 summed minutes.** The one new shard_9 story,
  `s4b2_an_open_refused_for_good_cancels_the_queued_launch`, takes 24.5 s. The cell now sums
  53.3 min.
- **`rest-libs`: +0.03 s,** for the tactics test.
- **The mush-sim test's additions** are inside an existing test.
- **The licensed-rig story is dispatch-only** (the `m8-gate-stories` matrix), so it adds nothing to
  a PR cell. It passed on conclav.dev in 895 s (gate run `41815082693`) and locally in 1,194 s.

Rule #13's harness kind is green at `41808113918`.

## S.4b-3 (2026-11-04): licensed minds in the field (§17.5–§17.6, §17.10; D-23-16, D-23-17, D-23-23 step 5)

Built on `claude/s4b3-field-minds` off `main` `4390f04c` (S.4b-2 = #403 merged), one PR. It runs beside FR.5's #410, which builds the join's kernel half (`arena/join.rs` installing a scripted seal handed its source); the host half rides this PR.

### What landed

- **The licence seat at `go` (D-23-16).**
  - A filed mind's `go` is refused before any rig leaves a bay once its licence's seats are held: the book's active claims under the licence (`ic_sortie::active_under_licence`) plus every other rig's claim still opening (the `Opening` cursors the launch caps' loop reads), against the paid licence's seats.
  - The seats come from the paid terms through `PaidHoldingView::seats` (mush-roll, never encoded). An owner licence dispatches with no seat cap.
  - The sentence is `MIND_SEATS_HELD` ("Licence {licence} already flies {held} of its {seats} seats; bring a hull home first.").
- **A launch cancelled after its Open (D-23-23 step 5).**
  - When the claim arm's tries are spent with the claim open but its queued launch never started (the charge, the caps), the `claim: refused` start writes `Closing { PoweredDownInBase { base: <the hangar's hull> } }` and clears the queue. The arm then closes the claim with the rig in its bay.
  - The operator hears `CLAIM_CANCELLED` once. A claim never opened still ends `Refused` (S.4b-2).
- **The field installs a claim's retained mind (D-23-17).**
  - A field's admission and its resume install each claimed hull through `IcHost::resume(open)?.install` (`arena_bridge::ic_retained`), which drops a seal of another ABI to the ROM, journalled once. The strict rule then covers the rest (`install_sealed_runners_except`); OOC installs are unchanged.
  - The field's resume, which passes no `sources`, no longer fails on a claimed hull.
  - `Execution::install` drops a retained source that no longer compiles at this build to the ROM, with `mind_dropped { cause: abi, fault: compile }` and the compiler's words traced. The boot audit records such a row instead of refusing the whole book.
- **The field stories' harness.**
  - `excursions::on_pad_as_at` (any site).
  - `m8_world::m8_world_field` (the claims world on any site, two rigs).
  - `m8_world::into_field_rig`, which hands the M8 world's live world to `driving_rig` after its rating. The field half does not reboot, because a `driving_rig` reboot composes no launch.
  - **The field runs at the stock tempo.** A tuned layer also runs the Registry's rating arenas at its 100 ms tempo, which multiplied their pulses past the twenty-minute cap. A reboot to retune after the purchase lost the pilot's licence read.

### Tests

- **`tacsim` lib:** `minds::ic_sortie::execution::tests::s4b3_a_retained_source_that_no_longer_compiles_drops_to_the_rom`.
- **`tacsim --test shard_9`, the field stories** (ignored; the M8 gate stories run them). In the claims world on Ahuna Flank (`…_at_256`) and on Urvara Reach (`…_at_512`), with one genuine rating and the paid one-seat Pin over two rigs bound to the governed Class II root and titled to the pilot:
  1. rig 1's claim opens in the bay and it launches under the claim's seal;
  2. rig 2's `go` under the same Pin is refused with `MIND_SEATS_HELD`;
  3. the raider marks rig 1 Hostile and the field opens. Rig 1 is admitted under its claim's own seal: the field's `mind_sealed` line names the filed mind, the Pin's licence, `active` and the claim's card digest. It flies twenty seconds with no `mind_dropped` and no `mind_fault`;
  4. the raiders withdraw (`@field/raiders/withdraw`, the field story's own ending) and the field demotes. Clearing a mark returns a party to its standing Hostile, so it does not end a field;
  5. `+tv/1 recover go` is landed on the composed path, and the composed ticks land the recover's finish and the claim's Close over the field-written hull. Rig 2's `go` then files.

  **Both passed on conclav.dev** (gate run `41854426069`): 512 in 1,170 s and 256 in 1,197 s. The 256 story finished just inside the gate profile's twenty-minute cap; the CI M8 job (`cargo test`) has no such cap.

### Deferred, and why

- **The join's host half:** waits for FR.5's #410 (the kernel half, `arena/join.rs`), to be folded in once #410 is on `main`.
- **The carried memory (D-23-18):** `UnitReturn` carries no mind memory, and the arena side of the end-of-match return is FR.5's in this wave.
- **M8g's "To S.4b" stories.**

### Hash pins (rule #13)

No pinned file is touched. `install_sealed_runners` keeps its exact rule for every OOC arena; only an IC field's spawn and resume take the retained path. The harness kind is dispatched before ready.

### Cost (rule #14)

From the PR run's JUnit once it lands.
