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
