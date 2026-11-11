# 44. Minds, wired through the game — cut notes

One section a unit of [doc 44](44-minds-wired-through-the-game.md) §7. A unit's session replaces its
own "Not built." line with what it built, under these headings: **Built**, **Tests**,
**Divergences**, **Not built**, **Not pinned by a story**, and the open Q-44 defaults it was built
on. Each session edits only its own section, so two units' notes never touch the same lines.

## MW.0

Licensed minds composed on a live server (doc 44 §7.1, Q-44-39; second review, §4c row 1). Built
with MW.22 below as one change: the owner ruled that neither reaches main without the other.

**Built.**

- **Every Active launch composes the claim path.** `tacsim::launch::LaunchComposition::prepare`
  (`crates/tacsim/src/launch.rs`) now ends its Active branch with
  `with_ic_minds_in_bays(Vec::new())`, after the House chain and the player market, for a
  format-1 and a format-2 deployment alike. An `Initialize` launch composes none of it, as
  before. So on a production boot `claim_path_composed` is true once the launch is Active: a
  licensed `+tv/<n> launch mind=filed:<n> go` opens its claim in the bay before the ramp, the
  field's `claim_seal` finds the claim and admits the rig under its own seal, a licence's seats
  are counted at `go`, and the reader refuses an Open while a resale is pending.
- **One function, shared by construction.** `mush_server::boot` already called `prepare` and
  nothing after it. The switch the rigs alone could reach,
  `LaunchComposition::with_ic_minds_in_bays`, is deleted, and the M8 world's `factory`
  (`crates/tacsim/tests/domain_field/m8_world.rs`) calls `prepare` and nothing after it. No
  world can now compose a launch without the claim path or with something production lacks, so
  every M8 story runs on the production composition. `m8_world_with_claims` and `factory_for`
  are gone; `m8_world_field` remains as the M8 world on a chosen site.
- **Nothing else had to be composed with it.** Doc 17g §4 deferred "IC claims and hull cover in
  live play" for want of an IC region host. The host exists since doc 23's S.4b (the surface
  tick's claim arm and the IC field). `with_ic_minds_in_bays` asks only for an Active commerce.
  Hull cover asks for the claim path, not the other way about, and stays uncomposed.
- **The shared story helpers follow the claim.** `m8_world::launch` and `recover` pump the
  claim arm where a launch opened a claim, so doc 17h's stories read as before.

**Tests.** Fast, on the production composition (`domain_field::m8_world`):
`m8g_the_combined_world_boots_active` now asserts the claim path is composed, and every M8
world asserts, at its own `Initialize` boot, that none is composed there. The acceptance stories are `#[ignore]`d (each takes a genuine Registry rating) and run on
the CI dispatch: `s4b3_a_licensed_mind_flies_a_field_at_256` and `_at_512` (a filed mind on a
rig its pilot holds title to opens a claim, is admitted to a field under its own seal and card,
and the one seat refuses the second rig), and
`s4b2_a_licensed_rig_opens_its_claim_in_the_bay_and_closes_it_home`, rewritten for MW.22 (its
step 3 is the launch refused while the licence's resale is pending). Results are under MW.22.

**Divergences.**

- **A leased rig and a House hull are not refused.** The row's last clause ("a leased rig and a
  House-titled hull are refused by name") describes MW.0 alone. Under Q-44-39 it never reaches
  main alone, and MW.22 admits both. What is refused by name is a hull the pilot has no
  authority over (MW.22).
- **No change to `mush-server`'s code**, only a comment at the call: the function it calls
  changed.

**Not built.** Hull cover and paid Follow stay uncomposed (17g §4). The DT launch
(`+sortie/launch`) still takes no `mind=` word, so a detachment's flight opens no claim (doc 23
§17.9, 23-10). A cored ship opens no claim (D-23-12).

**Not pinned by a story.**

- **A changed mind changing what the rig does.** The field stories pin that the rig flies the
  claim's own seal, mind and card and not the ROM's, with no drop and no fault. No story flies
  two different filed minds on one rig and compares what it did; MW.7's filed-mind story is the
  first that flies a mind written to behave differently.
- **A boot of `mushd` itself with an Active `launch_path`.** `check_config` prepares the launch
  composition from a document (`split_boundary::boot_parity`), and the M8 world boots what
  `prepare` returns, but no test boots the server binary's own module list over an Active
  launch and launches a rig. MW.1's bring-up is where that is first done.
- **A definition-set change at deploy.** The claim path adds its owners' definitions to an
  Active launch's manifest, so a store written by an earlier build does not boot under this
  one. The dev boxes are wiped at every step; nothing migrates.

## MW.1

Not built.

## MW.2

Not built.

## MW.3

Not built.

## MW.4

Not built.

## MW.5

Not built.

## MW.6

The Registry's pace as a file key, and the two notice drains that stopped on one bad holding
(doc 44 §7.1, Q-44-41; second review, §4c rows 4 and 10). Built on Q-44-41's default, which is
open: the key exists with today's value as its default, and a dev box sets it low.

**Built.**

- **`[tacsim.minds].registry_pulse_ms`.** `mush_config::TacsimMindsCfg.bulk_maintenance_ms` was
  `serde(skip)`, so no file could set the pace of a Registry lease; only the acceptance stories'
  harness could, in code. It is now a file key under the name `registry_pulse_ms` (the Rust
  field keeps its name and is not a key). Absent, it is 1,000 ms, production's second, as doc
  43c records was decided. `MindsConfig::validate` (`crates/tacsim/src/config.rs`) refuses a
  value outside 10 to 60,000 by the key's name, ahead of the bulk model's own check. The key is
  documented, commented, in `mushd.toml.example` and `deploy/instance/mushd.toml.tmpl`, and in
  the key table at the head of `crates/mush-config/src/config.rs`. `tacsim::MindsConfig` itself
  still skips the field in its own serde; the host's mapping (`mush_server::config_ext`) is the
  path a file takes.
- **Both notice drains skip one bad holding.** In `crates/mush-roll/src/commerce/registry_recall/notice.rs`
  and `registry_revocation/notice.rs`, `tick` returned the first error from `own` before the
  cursors were written back, so the next tick read the same page, met the same holding and
  stopped again: every notice behind it waited for ever. Now an error that is one holding's own
  (`one_holding`: `Missing`, `Mismatch`, `Invalid`) skips that holding, emits nothing for it,
  logs it once (`Runtime::note_skipped`, bounded like the remembered notices) and lets the
  cursor pass. An error that is the inventory's (`Unavailable`, `Stale`, `Capacity`) still stops
  the drain where it is, before anything is emitted, to be tried again. `page` had the same
  fault one level up: a row that failed `validate` failed the whole page. Such a row is now
  carried with no recipient, so the cursor passes it.

**Tests.** `mush-config`: `m1b1_the_minds_block_parses_absent_written_and_refuses_a_typo` gains
the key (absent is 1,000; `registry_pulse_ms = 50` parses; `bulk_maintenance_ms` is an unknown
field). `mush-server`: `config_ext`'s refusal table gains 5 and 60,001, each refused by the key's
name, and the mapper carries 50 through to the layer's config. `mush-roll`: one test in each
notice module, `a_bad_holding_is_skipped_and_an_unavailable_inventory_is_not`. Local: `cargo test
-p mush-config` 141 + 14 passed; `cargo test -p mush-roll notice` 3 passed; `cargo test -p
mush-server --lib config_ext` 18 passed; `cargo test -p mush-contract-tests` green; clippy with
warnings denied on mush-roll, mush-config, tacsim and mush-server.

**Divergences.** The unit's row asks for "the measured time a rating takes at both paces". It
was not measured in this cut. The figures on file are `ci.yml`'s (about 1,070 leases a
certificate, about three hours a story at 1,000 ms) and the acceptance stories' (minutes at
their 50 ms). MW.1's bring-up is where a dev box's rating is first timed.

**Not built.** Registry pacing beyond the pulse: a lease still takes 3 to 7 s of work at the
stock pace, and nothing here shortens the work a lease does. The key is not set in
`deploy/dev1` or `deploy/dev2`; MW.1 sets it when it renders the launch.

**Not pinned by a story.** The drains end to end: no rig corrupts one holding and shows the
holder behind it getting their notice. The tests pin which errors skip and which stop, and that
the skipped keys stay bounded. A row that fails `validate` in `page` is not pinned at all.

## MW.7

Not built.

## MW.8

Not built.

## MW.9

Not built.

## MW.10

Not built.

## MW.11

Not built.

## MW.12

Not built.

## MW.13

Not built.

## MW.14a

Not built.

## MW.14b

Not built.

## MW.15

Not built.

## MW.16

Not built.

## MW.17

The last decision in the staff view of an NPC (doc 44 §7.1, Q-44-30; first review, finding 8;
second review, §4c row 19).

**Built.** In `crates/mush-npc`:

- **A turn records what woke it.** `NpcHostState::begin_turn` (`runtime.rs`) drains the mailbox,
  so it is the one place that knows; it now writes a `LastDecision` on the NPC's runtime: the
  correlation, when it began, and for the first `LAST_DECISION_WOKE_MAX` (8) stimuli of the batch
  their kind (`Stimulus::kind_name`) and whom each was about. `fire_dialogue` (`module.rs`) adds
  how many public facts and recalled memories the snapshot showed the model
  (`note_decision_context`); guarded and secret facts never cross the async boundary, so the
  count is of public ones.
- **A completion records what the turn came to.** `NpcCompletion::on_complete` writes a
  `DecisionResult`: the model answered with these actions (their tool names), the model did
  nothing, the profile's fallback line with the staff-facing reason (capped at 160 characters),
  or the payload did not decode. A completion for another turn changes nothing
  (`finish_decision`).
- **`@agent/show` prints it**, in two lines after the runtime line (`LastDecision::lines`), and
  `docs/help/npc.md` describes them. The verb is Builder and up, as before.

The record holds kinds, ids and counts. It holds no prompt, no reply and no stimulus text, in
keeping with the audit's rule (`docs/help/llm.md`).

**Tests.** `runtime::tests::mw17_the_last_decision_records_what_woke_a_turn_and_how_it_ended`
(the record, the cap, another turn's completion, the two lines carrying no text) and
`mw17_staff_read_what_woke_a_turn_and_whether_the_model_or_the_fallback_spoke`
(`crates/mush-npc/tests/agents.rs`): through `@agent/show`, a turn in flight, the same turn
answered by the model, the next turn starved of budget and answered by the fallback, and a
player refused the verb. No test makes a provider call. Local: `cargo test -p mush-npc` 150 +
72 + 34 passed; clippy clean with warnings denied.

**Divergences.**

- **The NPC decision call still carries no NPC.** The row asks for `decision_llm.rs` to pass its
  NPC so the per-NPC budget and `@llm/kill <npc>` reach a decision. It cannot be done at that
  call site: the worker lease it is handed (`mush_core::decision::DecisionWorkerLease`) is
  public-safe by design, "no world or private witness reference can cross the async boundary",
  and names a controller and a decision, not an object. Carrying an audit attribution on the
  lease is a change to that contract in `mush-core` and belongs to its own cut.
- **"The knowledge tiers the NPC was allowed"** is reported as a count of public facts and of
  recalled memories. The snapshot carries no other tier, so there is nothing else to report.

**Not built.** The record does not survive a restart (it is runtime state, like the mailbox).
It keeps one turn, not a history. It does not cover a summarise turn or a scene-turn decision
(`npc_decision`), only dialogue turns.

**Not pinned by a story.** A completion whose payload does not decode (the `Unreadable`
result) through `@agent/show`.

## MW.18

Not built.

## MW.19

Not built.

## MW.20

Not built.

## MW.21

Not built.

## MW.22

Doc 23's cut S.4c: lease and organisation authority in the title fence, in the Roll and at the
dispatch (doc 44 §7.1, Q-44-39; second review, §4c row 2). The design note is doc 23 §18
(D-23-24), written first.

**Built.**

- **One rule, in the Roll.** `mush_roll::commerce::ic_claims::hull_authority(world, hull, actor)`
  (`crates/mush-roll/src/commerce/ic_claims/hull.rs`) answers by which of three arms a character
  may open a claim on a hull, or why not. The title must be live. Then: the character's own
  unscoped **title**; a **lease**, read from the economy alone (the title carries its holder's
  `Lease` claim, and that contract is Active, lease-shaped, issued by the holder, pledges the
  hull and names the character a party); or an **office** (the hull is an organisation's and
  the character holds a live `roll.holder.org` grant on the title's scope, asked as a read).
- **The Roll's reader captures it.** `TitleFence` (`ic_claims/reader.rs`) keeps the arm with the
  title's revision and re-reads both at the fresh check, so a lease that ended or a grant taken
  back between the read and the commit is stale. The reader's definition says
  `"hull":"current_title_lease_or_office"`.
- **The dispatch calls the same function.** `untitled_refusal` (`surface/dispatch.rs`) refuses at
  `go` exactly what the Open would refuse for want of authority, and names the hull and its
  title's holder: "{mark} is {owner}'s hull; a licensed mind flies a hull titled to you, leased
  to you, or held by an organisation you act for." The dispatch's own copy of the rule
  (`titled_to`) is deleted. The bot's pinned phrase is still inside the sentence.
- **An organisation's owner licence opens a claim** (doc 23 Q-23-19's path, which a House's
  licence on any hull needs). The reader's fence over an owner licence's charter accepted only
  the dispatcher's personal title; it now accepts the arm the owner projection proved, a House's
  charter by the acting character's office. The claim history's audit (`ic_claims/audit.rs`)
  accepted only the charter's historical owner as dispatcher; for a House birth it accepts any
  dispatcher, as the paid arm already does for an organisation's licence.

**Tests.**

- `mush-roll`, six tests in `ic_claims::hull` (`s4c_*`): each arm, the order they are tried in,
  the refusal naming the holder, a title that is not live admitting nobody, a claim that is not
  the titled lessor's, a grant on another scope.
- `tacsim`, in source: `s4b1_a_licensed_mind_needs_the_hull_titled_where_claims_open`
  (`surface/dispatch/tests.rs`), rewritten: nothing is read without the claim path.
- `tacsim`, fast, on the production composition:
  `mw22_the_hull_rule_reads_a_real_lease_a_house_title_and_a_personal_title`
  (`domain_field::m8_world`) asks the rule of the M8 world's own rig as `+tv/lease go` left it
  (the lessee admitted by the lease, a stranger refused naming Occator), then titled to a House
  (refused naming the House; admitted by the office once granted), then titled to the pilot.
- The acceptance story, `#[ignore]`d, on the CI dispatch:
  `s4b2_a_licensed_rig_opens_its_claim_in_the_bay_and_closes_it_home`, rewritten. Through
  registered commands, with a non-GOD pilot: (1) a chartered pilot launches her paid Pin on her
  leased rig, the claim opening in the bay and the open line carrying the claim's seal; (2) the
  one seat refuses the second leased rig; (3) with the Pin's resale accepted while that claim
  is open, the second rig is refused by the pending sentence; (4) the recover lands the Close;
  (5) on a rig titled to a House she holds no office with, she is refused by name; (6) on a rig
  titled to the Coprates House she flies the House's own owner licence, the House the holder
  and she the acting principal, and it closes at home.
- Doc 17h's stories with the claim path composed, on the CI dispatch:
  `m8g_story_tier2_held_minds_seal_aboard`,
  `m8g_story_zero_pc_procurement_buys_the_best_two_under_the_cap` and
  `m8g_story_the_combined_world_rates_files_lists_and_sells`.

RESULTS_PLACEHOLDER

**Divergences.**

- **The lease is read from the economy's contract, not from `TvLease`.** Doc 23 Draft 0.10
  sketched a `LeaseFence { lease, lessor, lessee, term_ends_ic }` captured from TacSim's rig
  lease record. The Roll cannot read a TacSim record, and the economy already holds the lease as
  a contract and a claim on the title, so the rule reads those. Doc 23 §18.2 records it.
- **The claim row does not record the arm.** Draft 0.10 has the lease "audited like the title's
  revision". The row keeps the title's revision, as before, and nothing else new: the row's wire
  is mirrored field for field in `mush-core`'s terminal-capacity protocol, and M8c S2 already
  set the precedent that the row records the dispatcher and not the grant.
- **The fence is asked as a read.** `roll.holder.org` is amount-bearing; flying a hull moves no
  money, so a grant with a spending limit admits (M8f's P2a ruling). An organisation's licence
  itself is still asked the way M8c built it.

**Defaults that would be a significant change to undo.**

- **Recording the arm on the claim.** If an inspection or a dispute ever has to say on what
  authority a hull was flown, the claim row, the admission digest and `mush-core`'s mirror of
  the row all change, and every stored claim with them.
- **A forfeit rig lease still admits.** The Roll reads the economy's contract, which stays
  Active after Occator captures the deposit; the launch verb does not refuse a forfeit rig
  either. Refusing a licensed mind there means a TacSim fact in the Roll's rule, or a second
  rule at the dispatch that the Open does not share.

**Not built.** Hull cover still sells only over a hull titled to its dispatcher
(`coverage/reader.rs`); no production launch composes it. The Trust's charter of a ship has the
lease arm's shape but no ship opens a claim (D-23-12). No verb lets a House name which officers
may fly which hull: the grant is the organisation-wide one its licences use.

**Not pinned by a story.**

- A lease that ends, or a grant taken back, **while the claim is open**: the design says the
  claim stays open until the epoch closes; no story ends a lease mid-claim. No in-game verb
  takes a `roll.holder.org` grant back.
- A rig repossessed where it lies on a site with its claim open: not traced (doc 23 §18.4).
- The reader's stale fence (authority lost between the read and the commit) end to end; the
  unit tests pin the rule, not the race.
- A character-held owner licence flown on a leased rig, and an organisation's paid Pin on an
  organisation's hull: the story flies a personal paid Pin on a lease and a House owner licence
  on a House hull; the zero-PC story flies a House's paid Pin on a leased rig.

## MW.23

Not built.

## MW.24

Not built.

## MW.17b

Not built.
