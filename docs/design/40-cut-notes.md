# 40. Cut notes — the field's second round

One `## FR.n` heading per cut of [40-field-second-round.md](40-field-second-round.md), in the
order they land: what landed, what it deferred and why, how it was verified.

## FR.1 (2026-11-03) — stances (§4.2; Q-38-23, Q-40-11..14, round 2's 40-3)

Built against doc 40 Draft 0.4 (§4.2, §5.2 and the FR.1 row of §6), one PR.

- **The table** (`crates/mush-faction/src/stance.rs`): `StanceSide::{Org, Faction}` (the field's
  `Affiliation`s), `Stance::{Allied, Neutral, Hostile}`, and `StanceRow { from, to, stance,
  declared_by, at }` (`deny_unknown_fields`), one row a direction, in
  `FactionState.stances` beside the edges — the "future relationship matrix" `data.rs` reserved.
  Persisted whole under `mod:faction:stance:<from>:<to>` (`f<id>` / `o<id>`), write-through, no
  receipt; Neutral deletes the row. `FactionState::stance`, `::mutually_allied`,
  `::stances_naming`. The boot loads the rows as leaves (a bad or key-disagreeing row is skipped
  loudly) and drops a row naming a missing faction; an org side stays (org ids are never reused).
  `@faction/destroy` deletes the destroyed faction's rows.
- **The verbs** (`stance_commands.rs`): `+faction/stance <yours>=<other>/<allied|neutral|hostile>`
  for a faction's principals and members at `officer_level` and an organization's holders of
  `org.stance.declare` on its root scope; `@faction/stance <from>=<to>/<stance>` for staff. A side
  is a faction first, else an organization; a hidden one the viewer may not know reads as missing.
  A declaration is a line to the other side's officers. `+faction <name>` shows the rows
  ("Stance toward X: Hostile (mutual).", "X stands Allied toward it (unreciprocated)."), each naming
  a side the viewer may know; `+faction <org>` shows an organization's rows. `+faction` moves from
  the read-only registration group to legacy-post-image with its new switch (allowlist stanza
  refreshed, family `faction.stance`; `PINNED_BASELINE` unmoved); `@faction`'s stanza notes the
  switch.
- **The capability**: `mush_org::STANCE_DECLARE_CAPABILITY` = `org.stance.declare`, in tacsim's
  built-in vocabulary beside `org.role.manage` and in the server's vocabulary pin.
- **The field** (`crates/tacsim/src/field/`): `parties::opening_stances` reads the table over the
  parties' open affiliations — `(by, of)` Hostile where an organization of `by` declared Hostile on
  one of `of`'s, and every mutually Allied pair (Q-40-12); a fixture raider's standing marks ride
  the same rows. `FieldParty.stance_hostile` (`#[serde(default)]`, skipped when empty, so no record
  format moves) holds them on the live record: written at admission (`parties::admit_stances`),
  extended in `carrier::joining` for the pairs a new party opens only, and `FieldHost.marks` is
  built from those rows (`parties::standing_marks`) at both. Allied pairs join `FieldHost.friends`.
  `mark_in_field`'s clear of a party on its row sends `Some(Mark::Hostile)` and says "cleared to
  the standing Hostile" (`PARTY_CLEARED_TO_STANDING`) — so a raider's clear now behaves the same.
  `friends_of` (the void's friendly-fire test) counts a mutual Allied pair (Q-40-14).
  `friends_on_ground` returns the Allied pairs and each party's `stance_hostile` as admission
  would write them. `field/carrier.rs` is touched in `joining` only. No kernel change.
- **The story** (`tests/shard_2/field_fr1.rs`, `fr1_declared_stances_at_256` / `_at_512`, Ahuna
  Flank and Urvara Reach): three co-ops as factions (Concord ↔ Basin Hostile both ways, Concord ↔
  Halo Allied both ways, Halo → Basin Allied one-sided; Concord's leader also an open Basin
  member); a stance opens nothing (Q-40-13); Concord's `+field/mark` promotes; each picture holds
  the other Hostile from the first classification; Halo is Concord's Friend both ways, Basin is
  not Halo's; the clear of Basin returns to Hostile (not the shared member's Friend), a Friendly
  mark replaces it, the next clear returns to it; Halo's mid-field Neutral changes nothing; a
  Basin captain landing mid-field joins with Concord's Hostile on it in Concord's book and its own
  on Concord; everyone stands down and the field disengages; the next field reads Halo's Neutral
  and admits the arrival under the Hostile. The org declare path (`@org/create`, `@org/grant …
  org.stance.declare`) is exercised by the same story; mush-org has no membership verb, so an
  organization's stance has no field effect through a command path yet — the factions carry the
  field half. `mush-faction/tests/faction.rs` covers the refusals, the row round-trip through the
  store and the destroy cascade.
- **The raider's clear changed, as §4.2 rules**: CF.7's field story (`shard_9/field_story.rs`) had
  the raider's leader `=clear` every party but the bystander's to aim the raid; a clear now returns
  to the raider's standing Hostile, so the story marks them `=friendly` instead (two lines, the
  same intent). No other test clears a standing mark.
- **Grounds**: FR.0's hoist of `field_story.rs`'s grounds into `tests/support/field` had not
  landed, so the story carries its own two-row table (cells from `field_gc10` and `field_story`).
  `driving_rig::en_route_at` is the one support addition (`en_route` for any site).
- **Shard** (doc 43 rule #3): shard_2, the cell with the room (11 summed minutes in doc 43 §10).
- **Summed-minutes delta** (rule #14): **+2.9 min** on `tacsim-integration-2` (the two stories 84.8 s + 86.5 s on the PR run 41730885044 JUnit); the cell now sums 23.0 min (shard_2 14.2 + cost_lane), its wall 17 m 0 s, under the twenty-minute line.
