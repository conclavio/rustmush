# 46. Commerce twins and the test rig — a plan to remove the clones

Draft 0.2, 2026-11-13. Surveyed against `origin/main` at `e4e2f6e8e`. Anchors are file
and directory names; line numbers drift. §3's clone figures were measured for Draft 0.1 on that
commit. §4's similarity figures and file counts come from three read-only surveys of a checkout a
few merges behind that commit, and were not each re-read; the two renewal twins were diffed by hand.
Draft 0.1, the same day, was the plan. **Draft 0.2 records the owner's ruling of 2026-11-13: every
proposed default, Q-46-1 to Q-46-8, is accepted as written** (§6), and that unit CT.0 is built
([cut note](46-cut-notes.md#ct0)); the other units wait as Q-46-8 says.

This is a plan in the form [doc 44](44-minds-wired-through-the-game.md) and
[doc 45](45-the-absent-player.md) used: what exists, what is proposed, owner questions with a
default each, and units of work. CT.0 is built; nothing else here is.

## 1. Why this document exists

The owner read the mind-commerce and refit crates and found heavy clone-and-modify duplication:
sibling modules that are near-verbatim copies, such as `managed_house_renewal/receipt.rs` and
`managed_renewal/receipt.rs`, with a hand-written wire struct beside every envelope. The question
put was what it would look like to make the closed-protocol state machines one generic multi-party
K3 workflow engine, and to consolidate the story tests' scaffolding.

The answer this document gives: **collapse the twins and share the scaffolding; do not build one
engine for every protocol.** §4 is why, §5 is the shape, §7 is the work.

## 2. What is in scope

- The four crates a protocol family lives in: `mush-core` (`action_state/terminal_capacity/`),
  `mush-economy` (`managed_*`, `bay_refit`, `core_refit`, `core_integration`), `mush-roll`
  (`commerce/`), `mush-market` (`mind_*`).
- Their tests, and the composed stories in `crates/tacsim/src/minds/bulk/` and
  `crates/tacsim/src/composition/mind_commerce/`.

Out of scope: any change to what a protocol does, to a persisted byte, to a player-facing sentence,
or to the K3 kernel's contracts. This is a refactor that must be provably behaviour-preserving.

## 3. The baseline

Method, so it can be re-run: for each crate, every `.rs` file is split into lines, each line
trimmed, blank lines dropped; a line counts as cloned when it sits inside an 8-line window that
also occurs, exactly, in another file of the same crate. Both sides of a clone count. The measure
is a script since CT.0, `node scripts/clone-yardstick.mjs` (the four roots below; `--stems` folds
`renewal.rs` into `renewal/` for the kernel's rows of the second table). It reproduces both tables
to the line on main at `2022eb974`. It is run by hand at each unit and recorded in the
[cut notes](46-cut-notes.md); it is not a gate (Q-46-7).

| Where | Cloned lines | Of | Rate |
|---|---|---|---|
| `mush-market/src` | 10,407 | 43,659 | 23.8% |
| `mush-economy/src` | 21,762 | 99,404 | 21.9% |
| `mush-roll/src/commerce` | 16,691 | 89,366 | 18.7% |
| `mush-core/src/action_state/terminal_capacity` | 11,529 | 37,936 | 30.4% |

The owner's own figures (30.4% market, 27.8% economy, 19.8% roll) were taken with a different tool
and are of the same order and in the same order. `mush-core`'s per-family layer is a fourth copy the
owner's scan did not name.

Where the cloned lines sit:

| Crate | Family directories | Cloned lines | Share of the directory |
|---|---|---|---|
| market | `mind_renewal`, `mind_house_renewal` | 3,217 and 3,171 | 60% and 90% |
| market | `mind_entitlement`, `mind_house` | 1,774 and 1,772 | 34% and 53% |
| economy | `managed_renewal`, `managed_house_renewal` | 3,716 and 3,757 | 48% and 94% |
| economy | `bay_refit`, `core_refit`, `core_integration` | 1,769, 1,794 and 1,230 | 65%, 62% and 30% |
| economy | `managed_paid_resale`, `managed_coverage`, `managed_charter_transfer` | 1,602, 1,471 and 1,432 | about a third each, mostly `owner_tests.rs` |
| economy | `registry_filing` | 2,498 | 24%, mostly its sub-protocols' tests |
| roll | `renewal`, `house_renewal` | 3,493 and 3,498 | 80% and 83% |
| roll | `entitlement`, `house_purchase` | 1,290 and 1,080 | 31% and 44% |
| roll | `registry_recall`, `registry_revocation` | 1,159 and 1,144 | 63% and 44% |
| core | `terminal_capacity/renewal*`, `house_renewal*` | about 2,630 each | 89% to 94% |
| core | `terminal_capacity/entitlement*`, `house*` | about 1,440 each | about half |

## 4. What the survey found

**4.1 Only the House and player twins are clones.** Between the personal and the House renewal
families most files are 91% to 100% the same: in economy `receipt.rs` differs by 2 lines of 981,
`service.rs` by 4, `money_install.rs` by 21; in roll `chain.rs`, `roots.rs`, `capacity.rs` and
`records.rs` are 96% to 99%; in market 11 of 14 files are 93% to 99%. The purchase twins
(`mind_entitlement` and `mind_house`; roll's `entitlement` and `house_purchase`) are looser, mostly
70% to 88%, with some files under 40%.

**4.2 Different protocols are not clones.** Purchase against renewal against resale against
entitlement against refit overlap by 7% to 45%, file for file, and replacing the family words with
one token before the comparison barely moves that. They share a discipline (stages, receipts, roots,
a three-owner audit, a boot recovery bridge), not text.

**4.3 The twins differ by more than a field.** About nine differences are real logic and would be
the parameters of a shared implementation:

1. the terms: a seller, a lineage and up to three royalty parents, against an organisation and a
   scope, each with its own payout computation (`subject.rs`);
2. the current-authority check: a player's titles against the organisation store;
3. the payee: the seller's personal account against the issuer's scoped treasury;
4. the birth variant and its economics family, with one audit check inverted;
5. a buyer-admission policy and a purchase-template acceptance that only the House has;
6. a stop rule (`OwnCharter`) that only the player has;
7. the account pairing: a distinct pair against one personal account;
8. the prerequisite audits the kernel asks for;
9. every identity string (§4.4).

**4.4 Identity is in string literals.** Digest domains (`economy.[house_]mind_renewal_offer.v1`
and its kin), family ids, action ids (`draft_/fund_/settle_/refund_/close_[house_]mind_renewal`),
key prefixes and the protocol's JSON name are literals in each file, not shared constants. Digests
are taken over canonical JSON with sorted keys, so a struct's field order does not move a digest;
it does decide the wire's byte order where the market checks that a re-encoding equals the stored
bytes. **No test pins a digest as a literal**: tests recompute digests from the same strings, so
nothing today would catch a refactor that changed a persisted byte. *As found by CT.0:* the
first half holds for `DefinitionDigest::for_canonical_value` only. Economy's consent hash and plan
hash are taken over the typed value's own encoding, so there a field's order does move a persisted
hash ([cut note](46-cut-notes.md#ct0)).

**4.5 The wire structs are one macro, copied.** The same `macro_rules!` (named `checked!`,
`record!` or `data!`) is pasted into at least 16 files across the three crates. It generates the
domain struct, a private `deny_unknown_fields` wire struct, and a `Deserialize` that builds the
domain struct and then validates. A few pairs are written by hand (`FundingWire`, `TerminalWire`,
a parents list capped at three). *As found by CT.0:* twenty files define it, under five names
(`record!`, `closed!`, `checked!`, `strict!`, `data!`), and the copies differ in field visibility
and in whether a field's attributes land on the wire struct, on both structs or nowhere; the cut
note lists them.

**4.6 The kernel's lists are closed by design.** `terminal_capacity.rs` says each family is a
"closed, composition-checked protocol; modules cannot provide arbitrary target vectors, byte
ceilings or verifier callbacks". Each family is added by hand to the registry's fields and builder
methods, to a validator of about 1,500 lines in the kernel, and to enums in economy and roll
(`EconomyRequest`, `EconomyOutput`, `RootRequest`, `RootAnchor`, `PlanGuard`).

**4.7 Test scaffolding is the other large block.**

- Three economy `owner_tests.rs` files repeat a mock-module skeleton of about 1,350 lines nearly
  line for line.
- A cold-restart block is hand-written in 13 files, and 15 test files define their own `fn cold`.
  `mush-testkit` already has `AcknowledgedWorld::reboot_into`; the commerce tests do not use it.
- Money setup, a `FixtureReceipt` state family, and small helpers (`token`, `digest`, `stamp`) are
  redefined privately in between 5 and 33 files each. `test_checks.rs` is copied into each crate.
- The twins' tests are the same scenarios with the types renamed, and the House twin has fewer:
  `managed_house_renewal` has no in-crate tests, and `core_refit` has four.
- The composed stories grow one rig, tacsim's `PlayerWorld`, by a boot variant a family (26 of
  them).

**4.8 What pins the tests in place.** `mush-contract-tests/test-tiers.toml` names gated tests by
file path and function (two under `tacsim/src/minds/bulk/`); CI parses the `M7_STORY` phase lines
those stories print; 22 files wire test modules with `#[path]`; one market test pins the personal
and the House command output as byte-identical.

## 5. The shape proposed

**5.1 A twin is one implementation over a sealed issuer.** In each crate the renewal family becomes
one module generic over a trait with exactly two implementations, sealed so that nothing outside
the crate can add a third:

```rust
pub trait RenewalIssuer: sealed::Sealed {
    const FAMILY: &'static str;        // personal_mind_renewal_v1 | house_mind_renewal_v1
    const OFFER_DOMAIN: &'static str;  // economy.[house_]mind_renewal_offer.v1
    const ACTIONS: RenewalActionIds;   // draft_/fund_/settle_/refund_/close_…
    type Terms: RenewalTerms;          // seller + lineage + parents | organization + scope
    fn authority(ctx: &Ctx<'_>, terms: &Self::Terms) -> Result<Authority, Refusal>;
    fn payees(terms: &Self::Terms) -> Payees;
    fn stop(ctx: &Ctx<'_>) -> Option<RenewalStop>;   // OwnCharter: the player only
}
```

The names are a sketch. The files that are pure renames today (`receipt`, `service`,
`money_install`, `audit`, `bridge`, `chain`, `roots`, `capacity`, `protocol`, `records`,
`registration`, `publication` and the rest) become generic. The files that carry real logic
(`subject.rs`, roll's `reader.rs`, market's `offer.rs` and `config.rs`) stay as the two
implementations. Each identity string of §4.4 becomes an associated constant with today's value.

This keeps §4.6's rule: the generic is resolved at compile time over a closed set, and no module
hands the kernel a callback. The kernel's own lists stay closed and hand-edited; only the twin
validators (`terminal_capacity/renewal.rs` and `house_renewal.rs`, 95% alike) are merged.

**5.2 One wire macro.** `checked!` moves to `mush-core`, which all three crates depend on, and the
copies are deleted. It keeps field attributes, field visibility, `deny_unknown_fields` and
validation on decode exactly. The hand-written pairs stay hand-written.

**5.3 One test kit.** `mush-testkit` gains what the commerce tests each rebuild: the small
helpers; the cold restart, on `AcknowledgedWorld::reboot_into`; the funded-accounts block; the
fixture receipt family; the mock-module skeleton as a builder. The twins' tests become one body
run for both issuers, which gives the House twin every scenario the personal one has. Tests that
poke one party's internals (strict codec and raw decoder tests, capacity bounds, checkpoint and
sparse-fold tests) keep their own setup.

**5.4 Proof before the refactor.** Since no digest is pinned (§4.4), the first unit records one: a
snapshot, for each twin family walked to each of its terminal states, of every persisted key and
value and every digest, committed as a file and compared on every run, on Windows and on Linux.
Every later unit must leave it byte-identical.

**5.5 Not proposed: one engine for every protocol.** With 7% to 45% overlap (§4.2) it would be a
new abstraction over money and custody machines, not a removal of copies, and it would have to be
proven again by the tests that prove today's. The time to look again is when a new protocol of an
existing shape is wanted; the three contract-shaped economy families (`managed_coverage`,
`managed_charter_transfer`, `managed_paid_resale`) are the nearest to a shared stage engine.

## 6. Owner survey

**Ruled 2026-11-13: all eight proposed defaults accepted as written.** Each question below is
ruled at its proposed default; the alternatives were not taken. Q-46-2 is the one flagged
expensive to reverse, and CT.0's snapshot is its proof. Q-46-8 let CT.0 start at once; nothing
else starts until doc 44's MW.13 has landed.

**Q-46-1 — The scope.** *Ruled: the default.*
- **Proposed default:** twins, the wire macro and the test kit, as §5.1 to §5.4. No generic engine.
- **Alternative:** design the one engine first.
- **Why:** §4.1 and §4.2: the copies are in the twins.

**Q-46-2 — Bytes do not move. (expensive to reverse)** *Ruled: the default.*
- **Proposed default:** every identity string, key, persisted encoding and player-facing sentence
  is unchanged, and §5.4's snapshot proves it.
- **Alternative:** rename while we are here (there are no players and no data).
- **Why:** an unchanged snapshot is the only cheap proof a refactor of this size changed nothing.

**Q-46-3 — Sealed and compile-time.** *Ruled: the default.*
- **Proposed default:** a sealed trait with two implementations a family; no registration, no
  callback into the kernel.
- **Alternative:** a runtime description of a protocol that an engine interprets.
- **Why:** §4.6's rule stands as written.

**Q-46-4 — The purchase twins.** *Ruled: the default.*
- **Proposed default:** after the renewal twins, take only the files that are 90% alike or more;
  leave the rest as they are and say which.
- **Alternative:** collapse them fully, or leave them alone.
- **Why:** at 70% to 88% the shared part is smaller and the hooks are more.

**Q-46-5 — The refit pair.** *Ruled: the default.*
- **Proposed default:** `bay_refit` and `core_refit` share `fold`, `install`, `money`, `receipt`
  and `plans` (89% to 96% alike); `core_integration` is left alone. `core_refit` gains the tests
  `bay_refit` has.
- **Alternative:** leave the refit machines out.
- **Why:** it is the one clone outside mind commerce, and `core_refit` is the least tested.

**Q-46-6 — The pinned stories stay where they are.** *Ruled: the default.*
- **Proposed default:** no test named in `test-tiers.toml` moves or is renamed, and the `M7_STORY`
  lines are unchanged.
- **Alternative:** move them and re-point the pins.
- **Why:** the pins are the contract; nothing here needs them moved.

**Q-46-7 — A yardstick in CI.** *Ruled: the default.*
- **Proposed default:** §3's measure is a script in the repository, run by hand and recorded in
  the cut notes at each unit; not a gate.
- **Alternative:** a ratchet that fails a pull request which raises a crate's rate.
- **Why:** a ratchet on a crude measure invites gaming it; revisit once the rate is low.

**Q-46-8 — When.** *Ruled: the default.*
- **Proposed default:** CT.0 may start at once (it only adds a test). Nothing else starts until
  doc 44's MW.13 has landed, since it is writing in the registry and the claim path.
- **Alternative:** wait for the minds and absent-player programmes to close.
- **Why:** one writer a crate.

## 7. Units of work

### 7.1 The units

| Unit | What it is | Mostly in | Waits on | Done when |
|---|---|---|---|---|
| **CT.0** | **The snapshot and the yardstick.** For each twin family, a test that walks the protocol to each terminal state and compares every persisted key, value and digest with a committed file; §3's measure as a script, with this baseline recorded. **Built** ([cut note](46-cut-notes.md#ct0)): the snapshots are a crate's own, under `crates/<crate>/tests/ct0/`, compared by `mush_testkit::snapshot`; the refit pair's real path is walked in `mush-industry` and the composed registration in `tacsim`; the Roll's writers and the market's providers are pinned there by their rows and checks, not on their real path. **The second pull request the cut note owed, CT.0b, is built** ([cut note](46-cut-notes.md#ct0b)): the pinned M7 story records every preparation of the renewal and purchase twins' market actions on the composed path (the provider's outcome or its refusal as printed, the transition's replay writes, the store's documents at an operation's end) and compares four committed files under `crates/tacsim/src/minds/bulk/ct0b/`, which hold no digest and no object number; a unit proves digests by comparing the unprojected record of its base with its tip's | the four crates' tests; `scripts/` | — | the snapshot passes on Windows and Linux; changing one digest domain, one key prefix or one field's order fails it; the script reproduces §3 |
| **CT.1** | **One wire macro.** `checked!` in `mush-core`; the copies deleted. **Built** ([cut note](46-cut-notes.md#ct1)): `mush_core::checked!` (`crates/mush-core/src/checked_wire.rs`); the twenty copies are gone and each of their 93 calls says in its head who sees the struct, who sees its fields, and, where a field has an attribute, whether it lands on the wire struct or on both; no snapshot file moved | `mush-core`; the twenty files that defined the macro (`subject.rs`, `records.rs`, `economics.rs`, `purchase.rs`, `rating.rs`) | CT.0 | no copy of the macro is left; CT.0's snapshot is unchanged; a decode with an unknown field or a failed validation is refused as before |
| **CT.2** | **The test kit.** The helpers, the cold restart, the funded accounts, the fixture receipt family and the mock-module builder in `mush-testkit`; one `test_checks`. **Built** ([cut note](46-cut-notes.md#ct2)): `mush_testkit::{checks, fixtures, cold, funding, owner_rig}`, naming no commerce type so that a crate's own unit tests can use them; 72 test files of the three crates moved onto them with every test name unchanged; the five economy owner rigs share one skeleton; the cold restart is the kit's, over a boot map and not over `AcknowledgedWorld`'s Store, and each crate's `test_checks.rs` is left as three forwarding macros (both in the cut note's divergences) | `mush-testkit`; the test files of the three crates | CT.0 | the three `owner_tests.rs` share one skeleton; no test file defines its own cold restart; every test that passed still passes under its old name |
| **CT.3** | **The renewal twin in the kernel.** One validator over a sealed issuer. **Built** ([cut note](46-cut-notes.md#ct3)): `renewal_twin.rs` is the validator, generic over a sealed `Issuer` of fourteen identity constants and nine functions; `renewal.rs` and `house_renewal.rs` are kept as the two implementations and their entry points, so no caller and no test path moved. The kernel holds four of §4.3's nine differences (the birth variant and its economics family, the prerequisite audits, the identity strings, and its share of the terms); the payee, the current-authority check, the admission policy, the `OwnCharter` stop and the account pairing are the owners' and wait for CT.4 to CT.6. The kernel's layer is at 17.1% on the yardstick, from 28.8% | `mush-core` `action_state/terminal_capacity/` | CT.1 | one implementation, two instantiations; the registry's family version strings unchanged; the twin's tests are one body run twice |
| **CT.4** | **The renewal twin in economy** | `mush-economy` `managed_renewal`, `managed_house_renewal` | CT.2, CT.3 | as CT.3; the House issuer runs every scenario the personal one does |
| **CT.5** | **The renewal twin in the Roll** | `mush-roll` `commerce/renewal`, `house_renewal` | CT.4 | as CT.3 |
| **CT.6** | **The renewal twin in the market** | `mush-market` `mind_renewal`, `mind_house_renewal` | CT.5 | as CT.3; the command output both issuers print is still byte-identical |
| **CT.7** | **The refit pair** (Q-46-5) | `mush-economy` `bay_refit`, `core_refit`; the kernel's two | CT.4 | the five shared files are one; `core_refit` has `bay_refit`'s tests |
| **CT.8** | **The composed stories' rig.** `PlayerWorld`'s boot variants become one builder with a family switch; the House renewal story runs the personal story's scenarios | `crates/tacsim/src/minds/bulk/`, `composition/mind_commerce/` | CT.6 | no boot variant a family; the pinned stories unmoved and green; the `M7_STORY` lines unchanged |
| **CT.9** | **The purchase twins and the close-out** (Q-46-4): the files 90% alike or more; roll's `registry_recall` and `registry_revocation` policy and authority; the yardstick re-run | the four crates | CT.6 | each crate's rate recorded beside §3's; what was left alone is named with its reason |

### 7.2 Order

`CT.0`, then `CT.1` and `CT.2` side by side (source against tests), then the twin bottom-up in
dependency order, `CT.3` → `CT.4` → `CT.5` → `CT.6`, one writer a crate. `CT.7` may run beside
`CT.5`. `CT.8` and `CT.9` close.

### 7.3 What it should buy

An estimate, from §3's table: the renewal twins hold about 6,400 cloned lines in the market, 7,500
in economy, 7,000 in the Roll and 5,300 in the kernel; the refit pair about 3,600; the repeated
test skeletons about 1,700 in economy alone. With those gone the rates would be near 10% in the
market, economy and the Roll, and near 17% in the kernel's layer, before CT.9. Roughly 12,000 to
14,000 lines of source and several thousand of test scaffolding are deleted.

### 7.4 Risks

- **A silent byte change.** CT.0 is the answer and must land first.
- **A lost test.** A generic test body can drop a twin's one odd case. Each twin unit lists the
  tests before and after by name.
- **Compile time.** Generics over large modules can cost build time; each unit records the crate's
  check time before and after.
- **The kernel's closed lists** (§4.6) are not made generic, so adding a family stays a hand edit
  in several places. That is deliberate.
