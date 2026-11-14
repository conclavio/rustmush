# 46. Commerce twins and the test rig — cut notes

One section a unit of [doc 46](46-commerce-twins-and-test-rig.md) §7. A unit's session replaces its
own "Not built." line with what it built, under these headings: **Built**, **Tests**,
**Divergences**, **Not built**, and the yardstick's figures after it. Each session edits only its
own section, so two units' notes never touch the same lines.

**The rule every unit works under (Q-46-2).** No unit of doc 46 may re-bless a snapshot file
(`crates/*/tests/ct0/*.snap`, and tacsim's `ct0_registration.snap`). A diff in one of those files
is a stopped unit: find the change that moved
the byte and undo it. A unit that adds a walk blesses the new file once, in the pull request that
adds it, and names it here.

## CT.0

The snapshot and the yardstick. Built 2026-11-13 on main `2022eb974`, then rebased onto
`4efa11521` (doc 44's MW.13 and MW.14c in) with no snapshot file changed.

**Built.**

- **The yardstick**, `scripts/clone-yardstick.mjs` (Node, no dependency). It is §3's method as
  written: every `.rs` file under a root split into lines, each trimmed, blanks dropped; a line is
  cloned when it sits in an 8-line window that also occurs, exactly, in another file under the same
  root; both sides count; a window repeated only inside one file does not. The windows are compared
  as text, not by a hash. It prints the four totals and a table a root (§3's second table);
  `--stems` folds `renewal.rs` into `renewal/`, which is how §3's kernel rows were added up;
  `--top N` lists the files; `--json` is for a diff between units. Run by hand, not a gate (Q-46-7).
- **The snapshot helper**, `mush_testkit::snapshot` (`crates/mush-testkit/src/snapshot.rs`). A walk
  writes `[section]` lines and `label = value` lines; `check` compares the whole with a committed
  file and, on a difference, says which line moved and shows both sides. What it offers:
  - `line`, `hex`, `bytes` (text where the bytes are UTF-8, hex where not), `text` (printed output,
    a line each);
  - `wire`, a typed record as its `Serialize` writes it, in the struct's own field order;
  - `replay_writes`, every `ReplayWrite` of a transition as the tagged JSON a bundle carries;
  - `codec`, the pin for one strict wire type: its bytes in field order, and the refusal a decode
    answers to an unknown field and to a failed validation (`codec_shape` where no JSON edit can
    fail the type's validation).
  It is not the shared test kit (that is CT.2): it knows nothing of a family.
- **The snapshot tests**: 34 tests named `ct0_*` in six crates, and 34 committed files: 31 tests
  over the four crates' own code with their files under `crates/<crate>/tests/ct0/`, two composed
  walks of the refit pair in `mush-industry`, and one composed registration snapshot in `tacsim`.
  `cargo test -p <crate> --lib ct0_` runs a crate's.

**Where the snapshot lives, and why.** A crate over its own stores first; a composed walk only
where the crate cannot reach its own real path.

- A failure names the crate, the family and the line, and each of the four crates' tests runs in
  under a second (the figures are below), on every pull request, in the cells that already run
  those crates' unit tests. Nothing was added to a slow cell.
- Two things need the composed owners and have a fast rig that composes them, so they are walked
  there: the refit owners (`mush-industry`'s rigs) and the registrations
  (`tacsim/src/composition/mind_commerce/tests.rs`).
- The stories under `crates/tacsim/src/minds/bulk/` are the only rigs that drive the Roll's
  writers and the market's providers, and they are the M7 gate: ignored on pull requests, half an
  hour at the fast pulse, pinned by path in `test-tiers.toml` (Q-46-6). CT.0 did not touch them.
  What that costs is under **Not covered**.
- The tests are unit tests under `src/` because the rigs they reuse are private to each family's
  test module. Each is a child module of the test module whose rig it uses.
- The files are text, LF on every platform (`.gitattributes` has `* text=auto eol=lf`, and the
  helper reads a CRLF checkout as LF: no produced line holds a raw carriage return).

**Re-blessing.** `COMMERCE_SNAPSHOT_BLESS=1 cargo test -p <crate> --lib ct0_` rewrites the files
from the run, the same idiom as `TACSIM_BLESS` and `PIXEL_BLESS`. Each file's header says what
this note says: no unit of doc 46 may re-bless; a diff is a stopped unit.

The files are a tripwire for everybody, not only for doc 46. A change outside doc 46 that means
to move a line (a new field on an economy record, a reworded sentence, a new registration)
re-blesses in its own pull request, reads the file's diff line by line, and names it in its cut
note; the failing test's report says so. So that this is rare, the two composed snapshots
(`mush-industry` and `tacsim`, below) hold only the families' own bytes and names: nothing owned
by Tactics, Inventory or the object store, no hash over a whole bundle, no digest that inherits
TacSim's registry manifest.

**What is pinned, by crate.** "Walk" means the real code path to the state, with fixed tokens,
clocks and ids; "codec pin" means bytes in field order plus the two refusals.

### The kernel, `mush-core` (`src/action_state/terminal_capacity/`): 7 tests, 7 files

The walks drive the validators through the tests' own mock modules: `draft` / `advance` / `close`,
then `prepare`, then `install`. Every step writes its replay writes, the stored capacity
reservation and its canonical bytes, the capacity fingerprint, and every digest the validator
checks; each end is followed by a cold rebuild that must equal the live store byte for byte.

| File | Family | Ends reached |
|---|---|---|
| `renewal_personal.snap`, `renewal_house.snap` | `renewal*`, `house_renewal*` (one walk, `renewal_walk!`, run for both) | granted (then the next term's automatic draft); refunded (then a buyer retry to a grant); unfunded closed, `declined` and `expired` (each then a retry and a declined close); closed by the service, `deadline_reached` and `clock_invalidated` (each then a holder retry); a funded offer the service cannot close; a changed manifest that only allows a refund; a Follow birth to a grant |
| `refit_bay.snap` | `bay_refit*` | Installed, for three part shapes (in and out, out only, in only); Cancelled straight after Accept; Cancelled after Advance |
| `refit_core.snap` | `core_refit*` (it had no rig; this one mirrors the bay's) | Installed into a coreless hull; a replacing Install with a displaced core; Cancelled after Accept; a replacement Cancelled after Advance |
| `purchase_personal.snap`, `purchase_house.snap` | `entitlement*`, `house*` (one walk, `purchase_walk!`) | granted and refunded, for Original, Derived, OriginalFollow and DerivedFollow; and for House, TwoSeatPin and TwoSeatFollow; four funding faults and every finishing fault the rigs define |
| `wire_capacity.snap` | the capacity reservation | codec pins of `Key`, `Target` (create, update), `Reservation` (with and without `protocol_fingerprint`), `DefinitionRef`, `ClockStamp`; the key prefix and path; `encode()`'s bytes |

The identity sections hold the action ids, the transitions, the family names and byte ceilings,
the protocol JSON with and without the service, and the family descriptors. The kernel has no
copy of the wire macro; `bay_refit/wire.rs` and `core_refit/wire.rs` parse JSON by hand and are
pinned through the walks' writes.

Not reached: the transferred-holder renewal attempts (`holder_automatic`, `holder_retry`, binding
formats 2 and 4: they need a paid-resale seed); a purchase's changed-manifest historical refund;
the core refit policies `ungovern`, `coreless` and `families`; a `restart` drive cause in either
refit. One seed: the core refit rig sets `registry.core_refit` on the frozen registry, because the
family's audit wants a core-integration declaration whose descriptors are private to that family;
the audit's refusal is pinned instead. One test helper changed: `advance_full` (`renewal/tests.rs`)
and `advance_with_service` (`house_renewal/tests.rs`) take the caller's closure at the unfunded
close, so that a walk can reach `expired`; no existing caller passes one.

### Economy, `mush-economy`: 7 tests, 7 files

| File | Family | Ends reached |
|---|---|---|
| `renewal_personal.snap`, `renewal_house.snap` | `managed_renewal`, `managed_house_renewal` (one walk, `managed_renewal/tests/ct0_walk.rs`, included by both; the House twin had no test and no rig, and has both now) | closed unfunded, Declined (then a buyer retry, format 2, declined again) and Expired at the deadline; closed by the service, DeadlineReached and ClockInvalidated, each after a cold boot; funded then Granted, with every payee role; funded then Refunded, then a buyer retry funded and Granted; a cold rebuild through the production boot bridge (`bridge::recover`) after every end, after "funded" and after "offered", which must equal the live book, service records, contracts and receipt audit |
| `refit_bay.snap`, `refit_core.snap` | `bay_refit`, `core_refit` | no walk in this crate (below); the policy, the action references, the three fixed plans with their hashes, and the funded, installed and cancelled payment histories with their receipts, each accepted by the history's own `validate()`; the core refit with and without a displaced core |
| `wire_charter_transfer.snap`, `wire_coverage.snap`, `wire_paid_resale.snap` | `managed_charter_transfer`, `managed_coverage`, `managed_paid_resale` | codec pins only |

Every step of the renewal walk writes its replay writes, the output's fields, the offer record,
the offer, the marker, the contract and the service record in field order, the offer digest, the
consent hashes, the plan hash, and the transaction, hold, receipt and receipt-audit wire; the
refusals a repeated transition, an unknown offer and a disabled boot answer are written as text.
Codec pins: `RenewalBasisV1`, `RenewalOfferTermsV1` and `RenewalOffer` of each twin (venue and
parents absent and present), and the hand-written pairs `RenewalMarkerV1`,
`RenewalServiceClosureV1`, `RenewalFundingReceiptV1`, `RenewalTerminalReceiptV1` and
`RenewalHolderBasisV1`; the four charter-transfer, four coverage and seven paid-resale types;
`BaySubject` and `RefitSubject`. `codec_shape` (bytes and the unknown-field refusal only) where a
type derives a plain `Deserialize` with no validation: the offer record, the closures, the
service record, the history and receipt types, the attempt enum, the refit policies, and
`PaidBirthRefV1`, whose one rule no JSON edit can break.

Not reached: the holder renewal attempts on the real path (formats 3 and 4 need a settled paid
resale in the same store); their record bytes are pinned from the walked row with the attempt
changed and the consent hash recomputed. The refit owners in this crate: `money::prepare` and
`install::prepare` take an `OwnerPrepareContext`, which only the Engine builds. Their real path
is walked in `mush-industry` (below).

### The Roll, `mush-roll` (`src/commerce/`): 12 tests, 12 files

| File | Family | Ends reached |
|---|---|---|
| `renewal_personal.snap`, `renewal_house.snap` | `renewal`, `house_renewal` (one walk, `ct0_renewal_walk!`) | drafted, funded, granted (with the granted licence, term and economics rows), refunded, unfunded closed (declined, expired), closed by the service (deadline reached, clock invalidated); every attempt kind (automatic, buyer retry after a refund, holder automatic after a declined close, holder retry); a second-generation automatic draft after a grant; `chain::verify` cold, over the decoded stored rows, for the buyer-retry and the holder chains |
| `purchase_personal.snap`, `purchase_house.snap` | `entitlement`, `house_purchase` (one walk, `purchase_walk!`) | funded, granted, refunded, for OriginalV3 and DerivedV1, HouseV1 and HouseV2; on a grant the licence, term and paid-holding rows |
| `registry_recall.snap` | `registry_recall` | its one end, a permanent cause, without and with a prior author withdrawal; the notice and the refusal as printed |
| `registry_revocation.snap` | `registry_revocation` | its one end, for an owner Follow term and a paid Pin term; the notice and the refusal as printed |
| `wire_charter_pending.snap`, `wire_conduct.snap`, `wire_entitlement_economics.snap`, `wire_house_purchase_economics.snap`, `wire_paid_resale.snap`, `wire_rating.snap` | the other macro files | codec pins only |

**These walks are structural, and this is the largest limit of the unit.** The Roll's renewal
writer and its purchase `fund` and `terminal` take the Economy owner's opaque outputs
(`RenewalOfferOutput`, `EntitlementTerminalOutput`), which have no constructor outside the composed
owners, and the fresh transitions' roots take the reader's facts, which have none outside the
reader. So each walk builds the rows the writer writes and runs every check the writer calls on
them (`follows`, `validate`, `chain::check_draft`, `chain::verify`, `history::check_offer` against
a real Economy offer, `capacity::check`, `audit::required`, `RenewalHeadV1::for_operation`,
`RenewalRoots::prepare` for refund, close and service close), and writes each row's full key
(`owner/family/key`), the row in field order, and its stored canonical bytes. It does not run
`writer.rs`, and the snapshot holds no replay write. The family descriptors, all five transition
descriptors and the service descriptor (registered, with their digests), both reader descriptors
and the capacity declarations are written.

Codec pins: every type of the eight macro files (the renewal records in the walk files), with
`RatingJob` in six states, `PendingHead` in five, `PaidResaleHead` in six; the hand-written pairs
`RenewalHeadV1`, `EntitlementAllocationV1`, `HouseAllocationV1`, `RecallOperation`,
`RevocationOperation` and `RevokedTerm`. `codec_shape` for `RatingSubject`, `RatingCompatibility`,
`CaseBinding`, `HolderTerm` and `PendingOwnerTerm`, whose validation no well-typed edit fails.
`RatingId` and `ResultId` (`digest_id!`) are bare strings, pinned as lines with a round trip.

### The market, `mush-market`: 5 tests, 5 files

| File | Family | Ends reached |
|---|---|---|
| `renewal_personal.snap`, `renewal_house.snap` | `mind_renewal`, `mind_house_renewal` (one walk, `renewal_walk!`) | drafted, funded, granted, refunded, unfunded closed (declined, expired), closed by the service (deadline reached, clock invalidated). Each transition is one real typed transaction over the families `families::family` registers in a bare engine: a draft inserts, a fund replaces, an end replaces and inserts the receipt |
| `purchase_personal.snap` | `mind_entitlement` | listed, quoted, funded, granted, refunded, for an original sealed and a derived open-resale product, through the real `index` writers |
| `purchase_house.snap` | `mind_house` | the same states for House V1, rows only (below) |
| `wire_paid_resale.snap` | `mind_paid_resale` | codec pins only, and the four families' descriptors |

A state of the renewal walk writes the replay writes, the stored canonical documents, the row and
the receipt in field order, the Roll projection, the keys, the digests, and **what a player
reads**: the status line, the front's words at three points of the term, the queued and
unresolved lines, the three `renew_stopped` sentences, the service's command, refusal and stopped
line. Also the family descriptors, the five `expected_action` references, the action parameters,
and, for each configuration (the House twin with a one-seat and a two-seat Pin), the actions, the
policy, the declarations and `capacity::check`. Codec pins: all seventeen `data!` types, each
with a validation that really fails.

Not reached: a cold boot from the documents (the kernel's boot audit refuses renewal records
without the registered renewal protocol, which needs the composed Roll and purchase audits; the
test asserts that refusal, and that the written documents decode back to the live rows); the
providers' prepare arms; the House purchase's replay writes and family descriptors
(`Families::register` asks for the Roll's House sale reader, which a bare engine refuses, and the
House family has no test helper like the personal twin's).

### Industry, `mush-industry`: the refit pair on its real path

Economy's refit owners cannot be driven inside economy, so the refit pair is walked where the
real Engine composes it: `mush-industry`'s own rigs (`bay_refit/rig_tests.rs`,
`core_production/refit_integration_tests.rs`), through the real Org, Economy, Inventory,
Industry, Sim and Tactics owners and the kernel's protocols. 2 tests, 2 files
(`crates/mush-industry/tests/ct0/refit_bay.snap`, `refit_core.snap`).

| File | Ends reached |
|---|---|
| `refit_bay.snap` | offer, accept, Install refused before the work, the work, **Install**, cold boot; accept then **Cancel**, cold boot; accept, some work, **Cancel**, cold boot; the whole work, Install refused at the reservation's deadline, Cancel, cold boot; an accept after the offer's 300 s, refused; the carry refused while the hull is held, as printed |
| `refit_core.snap` | **Install** over the stock core; a second certified core **installed, displacing the first** (detached, its condition kept, returned to the requester), cold boot; some work then **Cancel** (fee, slot and core released), cold boot; the whole work, Install refused at the deadline, Cancel, cold boot; an expired offer, refused |

A transition writes the invocation's parameters, the action and its action, outcome and
participant definitions, the refit's own outcome record, and its replay writes by one rule
(`ct0_snapshot::Scope::pinned`): **a write is pinned with its value when the refit family owns
it**, and by kind, namespace and key alone when it does not.

- With values: every write in the `economy` namespace and every ledger chunk (Economy's journal,
  accounts, holds, idempotency and meta keys); the kernel's capacity reservation; Industry's
  refit records (orders, offers, index, hull and asset heads, schedule rows, and for the core the
  component exchanges the kernel's validator joins).
- By key only: every Tactics and Inventory write, every object write, and Industry's facility
  slots, book and capacity index.
- Not in the file: the unit image, the lots, object locations, the engine's semantic events,
  witnesses and outbox, and every hash over the whole bundle. The tests still assert on them (the
  unit after an install, the lot consumed and the part returned, the displaced core back with the
  buyer, the cold boot's records equal to the live ones); another programme's change to a unit
  or a lot record does not move these files.

After each step the records are read back in field order: the Industry order, offer, index and
heads; Economy's subject, policy and evidence digests, payment history and hold; the kernel's
reservation document; the two accounts. A cold boot writes what the store took at boot. The
identity section holds each registered action reference with its descriptor, the participants,
the policies, the kernel's family names with their descriptors, and every registered refit
definition of Economy, Industry, Inventory and Tactics (that list is what catches Economy's
`bay_refit_policy` and `core_refit_policy`, which no bundle names). Codec pins of this crate's
own strict records (`BayOffer`, `BayOrder`, `RefitOffer`, `RefitOrder`, `AssetPosition` in its
four variants and the rest) follow the walks. What still depends on another crate: the refit
policy, subject, offer and order name the rig's fixture unit, component and recipe definitions by
digest, and the evidence records carry the Tactics component projection.

Not reached: an install into a coreless hull and an ungovern refit (the rig's payment policy has
neither; both are codec fixtures only); the Initialize bundle (the rigs run it themselves; its
census row is written as the first records).

### TacSim, `tacsim` (`src/composition/mind_commerce/`): what the composition registers

The registrations need the composed owners, and the one fast rig that composes them is
`composition/mind_commerce/tests.rs` (not ignored, on every pull request). One test,
`ct0_composed_registration_snapshot`, and one file beside it,
`src/composition/mind_commerce/ct0_registration.snap` (a contract test refuses a non-domain
directory under `crates/tacsim/tests/`).

It boots the widest chain the launch order allows (House owner terms, filings, reconformance,
minds facts, House minds, two-seat, House renewals and their service, owner attestations; then
source purchases, forks, the marked Registry, dependent owners, entitlements, compatible and
derived updates, charter transfers, revocation, pending charters, paid resale, recall, personal
renewals and their service, withdrawal and re-qualification) and 21 narrower configurations, and
writes each as **its difference from the same modules composed without commerce**:

- every definition commerce registers, by name (`owner/kind/id@vN`), with its current flag, its
  payload, its references by name and the owner of its action provider: the binding descriptors
  with their protocol names, the policy kinds, the actions, outcomes, owner reads, participants;
- the state families (owner and id), delegations and read grants; bridges and migrations;
- what a player with no holding is offered: 22 actions with verb, label, help and parameter
  schema (every `mind-renewal-*`, `house-renewal-*` and `paid-mind-resale-*` verb among them);
- the builder's order; the commands commerce adds (`@mindrenewal`, `@housemindrenewal`,
  `@mindpurchase`, `@paidresale`, `@mindrecall`, `@mindrevoke`, `@registry`) and `+minds`, with
  switches, gates and help as printed; the ticks, in registration order; panel feeds, job
  queues, script APIs, persist delegations, namespace owners and continuation delegations.

**It holds names and shapes, and no digest.** Every commerce action's descriptor refers to
TacSim's registry manifest, whose references name the unit catalogue, so a digest here would
move with any catalogue edit by another programme. A definition reference is written without its
digest, a `{key, version, digest}` object inside a payload loses its digest, the manifest's own
payload is not written, and the test fails if a line holds 32 hex digits in a row. The digests
are pinned by the crates' own snapshots, over fixture manifests. The payloads' numbers (prices,
terms) are the composition tests' fixtures, not the launch configuration.

Not enumerable from a composed engine, so not here: a state family's descriptor (codec, schema
ids, bounds) and the kernel's terminal-capacity protocols (both in the kernel's and the crates'
snapshots); the participants registry and validator bindings (seen through the definitions they
bind); the service-close presentations (`mind-renewal-service`, `house-renewal-service`), which
project only with a retained record; the recall and revocation offer verbs
(`roll-registry-recall`, `roll-registry-revocation`), offered to an operator only, and their
capability ids. Conduct, the Registry inspection family, IC claims and House admission are not in
the widest chain (they need the hardware catalogue).

**The wire macro (CT.1's ground).** Twenty files define it (`record!`, `closed!`, `checked!`,
`strict!`, `data!`), not sixteen:

| Crate | Files | Macro |
|---|---|---|
| economy | `managed_renewal/subject.rs`, `managed_house_renewal/subject.rs`, `managed_coverage/subject.rs` | `record!` |
| economy | `managed_charter_transfer/subject.rs`, `managed_paid_resale/subject.rs` | `closed!` |
| roll | `renewal/records.rs`, `house_renewal/records.rs`, `paid_resale/records.rs` | `checked!` |
| roll | `charter_pending/records.rs` | `closed!` |
| roll | `conduct/records.rs` | `strict!` |
| roll | `entitlement/economics.rs`, `rating.rs` | `record!` |
| roll | `house_purchase/economics.rs` | `data!` |
| market | `mind_renewal/records.rs`, `mind_house_renewal/records.rs`, `mind_entitlement/records.rs`, `mind_entitlement/purchase.rs`, `mind_house/records.rs`, `mind_house/purchase.rs`, `mind_paid_resale/records.rs` | `data!` |

Every type those files build has a codec pin. The copies are not identical, and the one macro
CT.1 writes must keep each difference, since two of the three change bytes or what compiles:

- **Where a field's attributes land.** Nine copies take none (both `closed!` in economy, the
  Roll's `closed!`, `paid_resale`'s `checked!`, `entitlement/economics.rs`'s `record!`, the Roll's
  `data!`, the market's `data!` under `mind_renewal`, `mind_house_renewal` and
  `mind_paid_resale`). Nine put them on the wire struct only, so
  they shape the decode and never the encode (economy's three `record!`, the market's four
  `data!` under `mind_entitlement` and `mind_house`, `strict!`, `rating.rs`'s `record!`). Only the
  Roll's two renewal `checked!` put them on both structs. An attribute moved onto a domain struct
  (a `skip_serializing_if`, say) would change stored bytes.
- **Field visibility.** `pub` in economy; `pub(crate)` in the Roll's `checked!`, `closed!`,
  `data!` and `entitlement/economics.rs`; `pub(super)` in `strict!` and the market; private in
  `rating.rs`.
- **Struct visibility.** `pub` everywhere but the Roll's `closed!`, which is `pub(crate)`.

Where a type has a `skip_serializing_if`, a `default` or a `deserialize_with` field, the pin holds
one fixture with the field present and one with it absent. The other macros in
those crates (`row!`, `digest_id!`, `identity!`, `int_id!`, `numeric_id!`, `put_record!`, the
`check!` family) are not the wire macro; `row!` and `digest_id!`'s outputs are pinned where they
name a family or a key.

**The mutations.** Each was made alone, on a throwaway clone on conclav.dev (so that the worktree
was never edited and nothing had to be reverted in it), the crate's `ct0_` tests run, and the file
put back. Each line is the mutation and the first line of the report it produced.

| # | Where | The mutation | What the tests said |
|---|---|---|---|
| E1 | economy | digest domain (personal renewal offer): `"economy.mind_renewal_offer.v2",` | 1 test(s) failed. `SNAPSHOT MOVED: …/renewal_personal.snap` (232 of 729 lines), first difference line 58: `write = {"kind":"module_json_set","namespace":"economy","key":"contrac…` |
| E2 | economy | digest domain (House renewal offer): `"economy.house_mind_renewal_offer.v2",` | 1 test(s) failed. `SNAPSHOT MOVED: …/renewal_house.snap` (232 of 725 lines), first difference line 58: `write = {"kind":"module_json_set","namespace":"economy","key":"contrac…` |
| E3 | economy | key prefix (personal offer documents): `pub(super) const PREFIX: &str = "renewal-offr:";` | 1 test(s) failed. `SNAPSHOT MOVED: …/renewal_personal.snap` (44 of 729 lines), first difference line 19: `offer key prefix = renewal-offer:` |
| E4 | economy | key prefix (House service closures): `pub(crate) const PREFIX: &str = "house-renewal-svc-close:";` | 1 test(s) failed. `SNAPSHOT MOVED: …/renewal_house.snap` (8 of 725 lines), first difference line 20: `service key prefix = house-renewal-service-close:` |
| E5 | economy | field order (record! RenewalBasisV1, personal): `product_digest:DefinitionDigest, prior_quote_digest:DefinitionDigest,` | 1 test(s) failed. `SNAPSHOT MOVED: …/renewal_personal.snap` (225 of 729 lines), first difference line 58: `write = {"kind":"module_json_set","namespace":"economy","key":"contrac…` |
| E6 | economy | field order (hand-written House funding receipt): `contract: ContractId,` | 1 test(s) failed. `SNAPSHOT MOVED: …/renewal_house.snap` (17 of 725 lines), first difference line 407: `funding receipt = {"format":1,"commit":"000000000000000400000000000000…` |
| E7 | economy | action id (personal draft): `pub(crate) const DRAFT_ACTION: &str = "draft_mind_renewal2";` | 1 test(s) failed. `SNAPSHOT MOVED: …/renewal_personal.snap` (210 of 729 lines), first difference line 12: `draft action = draft_mind_renewal` |
| E8 | economy | action id (House refund): `pub(crate) const REFUND_ACTION: &str = "refund_house_mind_renewal2";` | 1 test(s) failed. `SNAPSHOT MOVED: …/renewal_house.snap` (15 of 725 lines), first difference line 16: `refund action = refund_house_mind_renewal` |
| E9 | economy | sentence (personal refusal): `ParticipantError::refused("The renewal offer is not available or has changed.")` | 1 test(s) failed. `SNAPSHOT MOVED: …/renewal_personal.snap` (19 of 729 lines), first difference line 55: `unknown offer refused = owner participant refused preparation: The ren…` |
| E10 | economy | field order (closed! CharterTermBasisV1): `generation: u64, licence: u64, operation: CommitToken,` | 1 test(s) failed. `SNAPSHOT MOVED: …/wire_charter_transfer.snap` (4 of 49 lines), first difference line 36: `wire = {"licence":1,"generation":1,"operation":"0101010101010101010101…` |
| E11 | economy | field order (record! CoverageMoneyTermsV1): `base_premium:Amount, insured_value:Amount, discount:Amount, premium:Amount,` | 1 test(s) failed. `SNAPSHOT MOVED: …/wire_coverage.snap` (6 of 70 lines), first difference line 41: `wire = {"format":1,"buyer":4,"insurer":0,"scope":0,"treasury":3,"payer…` |
| E12 | economy | field order (closed! PaidResaleWindowV1): `format:u16,issued_at:i64,eligibility_start:i64,term_end:i64,grace_end:i64,` | 1 test(s) failed. `SNAPSHOT MOVED: …/wire_paid_resale.snap` (8 of 85 lines), first difference line 46: `wire = {"format":1,"eligibility_start":0,"issued_at":0,"term_end":1000…` |
| E13 | economy | field order (BaySubject): `pub order: u64,` | 1 test(s) failed. `SNAPSHOT MOVED: …/refit_bay.snap` (11 of 144 lines), first difference line 41: `subject = {"order":1,"intent":"010101010101010101010101010101010101010…` |
| E14 | economy | identity (bay refit policy id): `mush_core::DefinitionId::new("bay_refits").map_err(\|e\| err(&e))?,` | **not caught**: the tests passed |
| C1 | the kernel | digest domain (personal renewal capacity fingerprint): `digest("core.mind_renewal.capacity.v2", &json!(values))` | 2 test(s) failed. `SNAPSHOT MOVED: …/wire_capacity.snap` (3 of 59 lines), first difference line 35: `wire = {"format":1,"funding":"1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f","effec…` |
| C2 | the kernel | digest domain (House renewal template, compared): `!= &json!(digest("roll.mind.house_renewal_template.v2", template)?)` | 1 test(s) failed, in the walk itself: `TerminalCapacity("renewal predecessor or disclosed terms differ")` |
| C3 | the kernel | key prefix (capacity reservations): `const PREFIX: &str = "action-state/capacity/v2/";` | 7 test(s) failed. `SNAPSHOT MOVED: …/wire_capacity.snap` (2 of 59 lines), first difference line 12: `metadata key prefix = action-state/capacity/v1/` |
| C4 | the kernel | field order (Reservation): `funding: CommitToken,` | 7 test(s) failed. `SNAPSHOT MOVED: …/wire_capacity.snap` (4 of 59 lines), first difference line 35: `wire = {"format":1,"funding":"1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f","effec…` |
| C5 | the kernel | action id (House fund): `"fund_house_mind_renewal2",` | 1 test(s) failed. `SNAPSHOT MOVED: …/renewal_house.snap` (83 of 1552 lines), first difference line 13: `action id 1 = fund_house_mind_renewal` |
| C6 | the kernel | family id (personal renewal head): `const HEAD: &str = "roll_mind_renewal_head2";` | 2 test(s) failed. `SNAPSHOT MOVED: …/wire_capacity.snap` (4 of 59 lines), first difference line 35: `wire = {"format":1,"funding":"1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f","effec…` |
| C7 | the kernel | protocol JSON name (personal renewal): `json!({"format":1,"protocol":"personal_mind_renewal_v2","transition":TRANSITIONS[index]})` | 2 test(s) failed. `SNAPSHOT MOVED: …/wire_capacity.snap` (5 of 59 lines), first difference line 35: `wire = {"format":1,"funding":"1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f","effec…` |
| C8 | the kernel | sentence (personal renewal refusal): `return Err(refused("renewal needs both closed purchase audits"));` | **not caught**: the tests passed |
| C9 | the kernel | digest domain (bay refit capacity fingerprint): `"industry.bay_refit.capacity.v2",` | 1 test(s) failed. `SNAPSHOT MOVED: …/refit_bay.snap` (15 of 493 lines), first difference line 98: `capacity fingerprint = hex:9758d9447a135f8b532f8cf3a20cec4642a1c16c256…` |
| C10 | the kernel | digest domain (core refit holder): `"industry.wo_holder.v2",` | 1 test(s) failed. `SNAPSHOT MOVED: …/refit_core.snap` (49 of 420 lines), first difference line 52: `capacity fingerprint = hex:92a5402ba5e2ca27975dff9c7f4aa05ae3e7267a7dc…` |
| C11 | the kernel | digest domain (personal purchase grant economics): `"roll.mind.grant_economics.v2"` | 3 test(s) failed. `SNAPSHOT MOVED: …/wire_capacity.snap` (3 of 59 lines), first difference line 35: `wire = {"format":1,"funding":"1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f","effec…` |
| C12 | the kernel | digest domain (House purchase capacity fingerprint): `digest("core.house_mind_purchase.capacity.v2", &json!(values))` | 1 test(s) failed. `SNAPSHOT MOVED: …/purchase_house.snap` (30 of 594 lines), first difference line 101: `write = {"kind":"module_json_set","namespace":"market","key":"action-s…` |
| R1 | the Roll | digest domain (personal renewal predecessor): `digest("roll.mind.renewal_predecessor.v2", term)` | 1 test(s) failed. `SNAPSHOT MOVED: …/renewal_personal.snap` (44 of 628 lines), first difference line 182: `operation record = {"format":1,"operation":"0a0a0a0a0a0a0a0a0a0a0a0a0a…` |
| R2 | the Roll | digest domain (House renewal template): `digest("roll.mind.house_renewal_template.v2", self.grant.renewal())` | 1 test(s) failed. `SNAPSHOT MOVED: …/renewal_house.snap` (87 of 624 lines), first difference line 178: `operation record = {"format":1,"operation":"0a0a0a0a0a0a0a0a0a0a0a0a0a…` |
| R3 | the Roll | family id / key (personal renewal operation): `const FAMILY: &'static str = "roll_mind_renewal_operations";` | 1 test(s) failed, in the walk itself: `operation: 0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a is not a valid roll_mind_renewal_operations key` |
| R4 | the Roll | family id / key (House renewal head): `const FAMILY: &'static str = "roll_house_mind_renewal_heads";` | 1 test(s) failed, in the walk itself: `head: licence/0000000000000007 is not a valid roll_house_mind_renewal_heads key` |
| R5 | the Roll | field order (checked! RenewalOperationV1, personal): `format:u16,action:DefinitionRef,operation:CommitToken,attempt:RenewalAttemptV1,` | 1 test(s) failed. `SNAPSHOT MOVED: …/renewal_personal.snap` (21 of 628 lines), first difference line 182: `operation record = {"format":1,"operation":"0a0a0a0a0a0a0a0a0a0a0a0a0a…` |
| R6 | the Roll | action id (House settle): `Self::Grant => "settle_house_mind_renewal2",` | 1 test(s) failed, in the walk itself: `Error("renewal evidence differs from authoritative history", line: 0, column: 0)` |
| R7 | the Roll | sentence (recall notice): `"Registry recall: your mind licence {} selects filed revision {}, recalled by operation {}…` | 1 test(s) failed. `SNAPSHOT MOVED: …/registry_recall.snap` (2 of 104 lines), first difference line 70: `notice \| 01 = Registry recall: your mind licence 9 selects filed revis…` |
| R8 | the Roll | digest domain (recall cause): `"roll.registry_revision_recall.cause.v2",` | 1 test(s) failed. `SNAPSHOT MOVED: …/registry_recall.snap` (4 of 104 lines), first difference line 64: `cause digest = "8fc9a424b673adb49b0cad1ee0c01aee5a0ccc16daf7a0d993fe9f…` |
| R9 | the Roll | digest domain (grant economics): `digest("roll.mind.grant_economics.v2", self)` | 3 test(s) failed. `SNAPSHOT MOVED: …/wire_entitlement_economics.snap` (2 of 85 lines), first difference line 44: `sealed grant digest = "2a34471d8b60c797d8e173aa6b1cddc9e61a233fdc1a74a…` |
| R10 | the Roll | field order (closed! PendingHead): `closed!(PendingHead,HeadWire,{format:u16,current:PendingRef,charter:CharterId,state:Pendin…` | 1 test(s) failed. `SNAPSHOT MOVED: …/wire_charter_pending.snap` (15 of 202 lines), first difference line 139: `head 0 record = {"format":1,"charter":"100","current":{"sequence":1,"o…` |
| R11 | the Roll | field order (strict! ConductJob): `admission:ConductAdmission,subject:ConductSubject,at_utc:i64` | 1 test(s) failed. `SNAPSHOT MOVED: …/wire_conduct.snap` (3 of 175 lines), first difference line 93: `job record = {"format":1,"operation":"0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a…` |
| R12 | the Roll | field order (record! Peak): `record!(Peak, PeakWire, { fees: u64, ops: u64, unit: u16, think_seq: u64 });` | 1 test(s) failed. `SNAPSHOT MOVED: …/wire_rating.snap` (11 of 184 lines), first difference line 57: `wire = {"ops":100,"fees":20,"unit":0,"think_seq":1}` |
| R13 | the Roll | field order (data! HouseTermEconomicsV1): `format:u16,generation:TermGeneration,licence:LicenceId,operation:CommitToken,` | 3 test(s) failed. `SNAPSHOT MOVED: …/wire_house_purchase_economics.snap` (6 of 74 lines), first difference line 30: `term economics record = {"format":1,"licence":"18446744073709551615","…` |
| R14 | the Roll | field order (checked! PaidResaleRef): `checked!(PaidResaleRef,ResaleRefWire,{operation:CommitToken,sequence:u64,digest:Definition…` | 1 test(s) failed. `SNAPSHOT MOVED: …/wire_paid_resale.snap` (14 of 204 lines), first difference line 133: `head 0 record = {"format":1,"licence":"1","current":{"sequence":1,"ope…` |
| M1 | the market | family id / key (personal renewal operation): `pub const FAMILY: &'static str = "market_mind_renewal_operations";` | 1 test(s) failed, in the walk itself: `renewal documents boot only under the kernel's renewal protocol` |
| M2 | the market | family id / key (House renewal receipt): `pub const FAMILY: &'static str = "market_house_mind_renewal_receipts";` | 1 test(s) failed. `SNAPSHOT MOVED: …/renewal_house.snap` (19 of 301 lines), first difference line 14: `receipt family = market_house_mind_renewal_receipt` |
| M3 | the market | field order (data! MindRenewalReceiptV1, House): `format:u16,licence:LicenceId,operation:CommitToken,predecessor:TermGeneration,` | 1 test(s) failed. `SNAPSHOT MOVED: …/renewal_house.snap` (8 of 301 lines), first difference line 149: `receipt = {"format":1,"operation":"0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a","…` |
| M4 | the market | sentence (personal renewal status): `pub const STATUS_FUNDED: &str = "Payment is held; completion is still pending.";` | 2 test(s) failed. `SNAPSHOT MOVED: …/renewal_personal.snap` (1 of 275 lines), first difference line 105: `status line = Payment is held; completion is pending. Price: 500 minor…` |
| M5 | the market | sentence (renewal offer line): `pub const RENEW_OFFER: &str = "Licence {licence} renews for {price} ({term}+{grace} IC day…` | 2 test(s) failed. `SNAPSHOT MOVED: …/renewal_house.snap` (3 of 301 lines), first difference line 116: `front request at world-IC 100 = say: Licence 7 renews for 5cr (1+1 IC…` |
| M6 | the market | digest domain (derived product): `(EntitlementProtocol::DerivedV1, "product") => Ok("market.derived_product.v2"),` | 1 test(s) failed. `SNAPSHOT MOVED: …/purchase_personal.snap` (25 of 335 lines), first difference line 82: `derived_mind_purchase_v1 product domain = market.derived_product.v1` |
| M7 | the market | digest domain (House quote): `(HouseProtocol::HouseV1, "quote") => Ok("market.house_quote.v2"),` | 1 test(s) failed. `SNAPSHOT MOVED: …/purchase_house.snap` (18 of 189 lines), first difference line 16: `house_mind_purchase_v1 quote domain = market.house_quote.v1` |
| M8 | the market | field order (data! HouseReceiptV1): `hold:HoldId,contract:ContractId,transaction:TransactionId,journal_seq:u64,` | 1 test(s) failed. `SNAPSHOT MOVED: …/purchase_house.snap` (6 of 189 lines), first difference line 134: `receipt = {"format":1,"operation":"04040404040404040404040404040404","…` |
| M9 | the market | digest domain (personal service intent): `let d=DefinitionDigest::for_canonical_value("market.renewal_service_intent.v2",&serde_json…` | 1 test(s) failed. `SNAPSHOT MOVED: …/renewal_personal.snap` (2 of 275 lines), first difference line 232: `deadline_reached guard = provider market actor 1 capability 4f2a373631…` |
| M10 | the market | digest domain (House submission, named uncovered): `"market.house_renewal_submission.v2",` | **not caught**: the tests passed |
| M11 | the market | identity (renewal policy kind, named uncovered): `DefinitionKind::new("renewal_policies").map_err(error)?,` | **not caught**: the tests passed |
| M12 | the market | family id (personal purchase receipt): `const FAMILY: &'static str = "market_mind_entitlement_receipts";` | 1 test(s) failed. `SNAPSHOT MOVED: …/purchase_personal.snap` (18 of 335 lines), first difference line 43: `receipt family id = market_mind_entitlement_receipt` |
| I1 | industry (refit) | key prefix (economy bay refit namespace): `pub(crate) const NAMESPACE: &str = "industry.bay_refit.v2";` | 1 test(s) failed. `SNAPSHOT MOVED: …/refit_bay.snap` (107 of 730 lines), first difference line 152: `write = {"kind":"module_json_set","namespace":"economy","key":"hold:0"…` |
| I2 | industry (refit) | identity (economy bay refit policy id, E14 again): `mush_core::DefinitionId::new("bay_refits").map_err(\|e\| err(&e))?,` | 1 test(s) failed. `SNAPSHOT MOVED: …/refit_bay.snap` (1 of 730 lines), first difference line 91: `registered = {"key":{"owner":"economy","kind":"bay_refit_policy","id":…` |
| I3 | industry (refit) | field order (economy core refit terminal receipt): `pub(crate) commit: CommitToken,` | 1 test(s) failed. `SNAPSHOT MOVED: …/refit_core.snap` (21 of 906 lines), first difference line 266: `record RefitOrder wo#b1b0ff91cf3c0c14 = {"format":1,"sequence":1,"acce…` |
| I4 | industry (refit) | printed text (economy core refit plan description): `p.description = "Certified core refit".into();` | 1 test(s) failed. `SNAPSHOT MOVED: …/refit_core.snap` (115 of 906 lines), first difference line 139: `write = {"kind":"module_json_set","namespace":"economy","key":"idem:7b…` |
| I5 | industry (refit) | identity (economy core refit hold purpose): `pub(crate) const HOLD_PURPOSE: &str = "industry.core_refits";` | 1 test(s) failed. `SNAPSHOT MOVED: …/refit_core.snap` (138 of 906 lines), first difference line 138: `write = {"kind":"module_json_set","namespace":"economy","key":"hold:1"…` |
| I6 | industry (refit) | digest domain (kernel core refit replacement): `!= &json!(digest("industry.core_refit.replacement.v2", replacement)?)` | 1 test(s) failed, in the walk itself: `Provider { action: DefinitionRef { key: DefinitionKey { owner: OwnerId("industry"), kind: DefinitionKind("acti…` |
| I7 | industry (refit) | digest domain (kernel bay refit capacity fingerprint): `"industry.bay_refit.capacity.v2",` | 1 test(s) failed. `SNAPSHOT MOVED: …/refit_bay.snap` (11 of 730 lines), first difference line 156: `write = {"kind":"module_json_set","namespace":"industry","key":"action…` |
| T1 | tacsim (registration) | identity (market renewal policy kind, M11 again): `DefinitionKind::new("renewal_policies").map_err(error)?,` | 1 test(s) failed, in the walk itself: `Msg("definition registry: owner market did not register a semantic validator for definition kind renewal_polic…` |
| T2 | tacsim (registration) | identity (market House renewal binding kind): `DefinitionKind::new("house_mind_renewal_bindings").map_err(error)?,` | 1 test(s) failed, in the walk itself: `Msg("definition registry: owner market did not register a semantic validator for definition kind house_mind_re…` |
| T3 | tacsim (registration) | printed verb (personal renewal fund): `Transition::Fund => "mind-renewal-pay",` | 1 test(s) failed. `SNAPSHOT MOVED: …/ct0_registration.snap` (1 of 5551 lines), first difference line 1088: `presentation = {"help":"Use the exact acknowledged offer. Requesting a…` |
| T4 | tacsim (registration) | command name (House renewal service): `const COMMAND: &str = "@houserenewal";` | 1 test(s) failed. `SNAPSHOT MOVED: …/ct0_registration.snap` (4 of 5551 lines), first difference line 892: `+ command = @housemindrenewal` |
| T5 | tacsim (registration) | tick name (personal renewal service): `"personal_mind_renewal_svc",` | 1 test(s) failed. `SNAPSHOT MOVED: …/ct0_registration.snap` (4 of 5551 lines), first difference line 971: `+ tick = market:personal_mind_renewal_service Coarse(1s)` |
| T6 | tacsim (registration) | action id (Roll personal renewal close): `Self::Close => "close_mind_renewal2",` | 1 test(s) failed, in the walk itself: `Msg("terminal capacity refused: renewal complete action identity or order")` |

Sixty-five mutations; sixty-one failed the tests of the crate they were run against, and two of
the four that did not are caught by a composed walk. What the brief asked for is covered several
times over: a digest domain (E1, E2, C1, C2, C9 to C12, R1, R2, R8, R9, M6, M7, M9, I6, I7), a
key prefix (E3, E4, C3, I1; and a family name, which is the key's middle, R3, R4, M1, M2, M12),
a field's order in a wire struct (E5, E6, E10 to E13, C4, R5, R10 to R14, M3, M8, I3), an action
id (E7, E8, C5, R6, T6) and a printed sentence (E9, R7, M4, M5, I4, T3).

- **E14** (Economy's bay refit policy id) passed economy's own tests, whose boot bridge is not
  driven there, and is caught by the composed refit walk (I2).
- **M11** (the market's renewal policy kind) passed the market's own tests, which do not compose
  the registration, and is caught by the registration snapshot (T1).
- **C8** (a kernel refusal reason) and **M10** (the House renewal submission's digest domain) are
  caught by nothing. Both are named under "Not covered" (items 7 and 2).

Where a row says "in the walk itself", no line moved: the walk's own check refused the mutated
value before the compare (the kernel recomputing a digest it was handed, a key no longer valid
for its family, a registration the kernel refuses). That is a failed test all the same.

The procedure differed from the brief's in one way. Each mutation was made on a throwaway clone
on conclav.dev (`perl` over one file, the crate's `ct0_` tests, the file copied back), not in the
worktree, so no uncommitted work was ever at risk and nothing had to be reverted. The tests ran
on Linux for the mutations; the unmutated tests pass on Windows and on Linux.

**The yardstick, before and after.** Before is main `2022eb974` with no file added, and is §3 to
the line. After is this unit: the tests are under `src/`, so they count.

| Root | Before: cloned of lines | Rate | After: cloned of lines | Rate |
|---|---|---|---|---|
| `crates/mush-market/src` | 10,407 of 43,659 | 23.8% | 10,698 of 45,375 | 23.6% |
| `crates/mush-economy/src` | 21,762 of 99,404 | 21.9% | 22,237 of 101,401 | 21.9% |
| `crates/mush-roll/src/commerce` | 16,691 of 89,366 | 18.7% | 16,942 of 91,868 | 18.4% |
| `crates/mush-core/src/action_state/terminal_capacity` | 11,529 of 37,936 | 30.4% | 11,605 of 40,352 | 28.8% |

The "before" run printed §3's second table as well, row for row (with `--stems` for the kernel's
rows: `house_renewal` 2,632 of 2,912 and `renewal` 2,630 of 2,763, §3's "about 2,630 each").

CT.0 added 8,631 lines of test under the four roots and 1,093 cloned lines with them: 291 in the
market, 475 in economy, 251 in the Roll, 76 in the kernel. A twin's walk is written once and run
for both (a macro in the kernel, the Roll and the market; one included file in economy), so the
new clones are the two thin adapters a twin and the fixtures that mirror an existing test's. The
largest are the market's two purchase snapshots (108 and 74 lines, which could not share a walk:
the rigs differ) and economy's walk (88 of 886). A later unit's "after" is compared with this
"after", not with §3.

**Time.** On the owner's Windows box, `-j 2`, the crate alone with its dependencies checked:

| Crate | `cargo check -p` (the library) | `cargo check -p --lib --tests` | The `ct0_` tests | The files |
|---|---|---|---|---|
| `mush-core` | 2.8 s | 30.6 s | 7 tests, 0.68 s | 7 files, 1.56 MB |
| `mush-economy` | 2.6 s | 17.0 s | 7 tests, 0.12 s | 7 files, 0.92 MB |
| `mush-roll` | 2.6 s | 15.3 s | 12 tests, 0.11 s | 12 files, 0.69 MB |
| `mush-market` | 1.3 s | 57.3 s | 5 tests, 0.22 s | 5 files, 0.62 MB |
| `mush-industry` | not measured | not measured | 2 tests, 1.86 s | 2 files, 0.98 MB |
| `tacsim` | not measured | not measured | 1 test, 11.3 s | 1 file, 0.27 MB |

The check times are each crate re-checked alone after its root file is touched, once, on a warm
box; they are this unit's record for §7.4's compile-time risk, and the library figure is the one
a twin unit compares against (no test is compiled into it, so CT.0 did not move it). The market's
second figure is its seven integration test binaries. The test times are the `finished in` line of
`cargo test -p <crate> --lib ct0_` on Windows; on the Linux box (conclav.dev) 0.66, 0.11,
0.10 and 0.21 s for the four crates, 1.6 s for the refit walks and 11 s for the registration
snapshot, which is 25 engine boots (the widest chain, two baselines, 21 configurations). Nothing
was added to a pull-request cell that did not already build and run these crates' unit tests.
In branch CI (run 43504818193, the hosted Linux runners, one process a test under nextest) the
33 tests of the five crates sum to 8.9 s in the `rest-libs` cell and the registration snapshot
takes 30 s in `tacsim-lib`, from the cells' JUnit reports. The run was dispatched, so the
nightly-tier jobs ran on the branch too (doc 43 rule 2.3): the M7 and M8 stories, the bot's minds
lane, the scale harness and the release-profile build all finished green.

The files are 5.0 MB of text together, which is large (the kernel's two renewal files are 0.45 MB
each), because a line holds a whole stored record and a walk repeats the record at every step.
That is deliberate: a line a record, nothing summarised. They are written once and never change.

**Not covered.** A later unit must not read this snapshot as proof of what is listed here.

1. **The Roll's writers, on their real path.** `renewal/writer.rs`, `house_renewal/writer.rs` and
   the purchase `fund` and `terminal` are not run by any CT.0 test (see the Roll, above): the rows
   they write, the keys and the checks they call are pinned, the code that assembles the writes
   is not. The same holds for the readers (`reader.rs`), whose facts have no constructor outside
   them. Only the M7 stories run those paths, and they recompute what they compare. **CT.5 must
   not treat a green snapshot as proof for `writer.rs` or `reader.rs`.** What would close it is
   under "Owed".
2. **The market's providers and dispatched service.** The providers' prepare arms and the
   service's dispatch need the composed Economy and Roll. Unpinned there: the submission digest
   domains `market.renewal_submission.v1` and `market.house_renewal_submission.v1` (mutation M10
   passed unseen); the idempotency key formats `renewal.close.{d|c}.…` and `renewal.<digest>`,
   which are persisted; the provider outcome kinds (`drafted`, `funded`, `granted`, `refunded`,
   `closed`); the provider refusals "This renewal is unavailable or has changed." and "This
   retained renewal cleanup is unavailable."; the delegation reason.
3. **Registration, where a composed engine cannot be enumerated.** The market's and the Roll's
   registration literals (binding descriptors, policy kinds, action ids, commands, ticks, verbs)
   are pinned by name in tacsim's file. Not pinned anywhere: the service-close presentation
   verbs `mind-renewal-service` and `house-renewal-service`; the operator-only offer verbs
   `roll-registry-recall` and `roll-registry-revocation` and the capability ids
   `roll.registry.recall` and `roll.registry.revoke`; the `presentation_probe` parameter. A state
   family's descriptor (codec, schema ids, bounds) is pinned only where a crate's own walk
   registers the family: the market's renewal and personal purchase walks and its paid-resale
   pin do; the market's House purchase does not (item 4).
4. **The House purchase in the market, at the store.** `purchase_house.snap` holds rows, keys,
   canonical bytes and digests, and no replay write or family descriptor (the Roll's House sale
   reader is not registered in a bare engine). Its `families.rs` descriptor format, the
   `"registry"` counts key (the personal twin's too) and the key closure are unpinned.
5. **The transferred-holder renewal attempts on the real path** (`holder_automatic`,
   `holder_retry`), in economy and the kernel: they need a settled paid resale in the same store.
   The Roll walks them (structurally); economy pins their record bytes from a walked row.
6. **Economy's purchase twins** (`managed_entitlement`, `managed_house_purchase`, and what of
   `managed_purchase` they share): no snapshot. They define no copy of the wire macro, so CT.1
   does not touch them; CT.9 does.
7. **The kernel's refusal reasons.** A kernel refusal is `ActionStateError::TerminalCapacity(&str)`,
   a diagnostic reason that is not persisted and not what a player reads. There are about 130 in
   each renewal validator; the walks pin the ones they provoke (27 lines in the personal renewal
   file). A reworded reason the walks do not reach passes unseen (mutation C8 did).
8. **Hand-written pairs with no fixture.** The Roll's `PendingRecord` and `PaidResaleRecord` need a
   validated native Economy history (`CharterPendingHistory`, `PaidResaleHistoryV1`) and have no
   codec pin; nor have `ExecutionObservation` with a `flight`, or a `Nonconforming` rating job.
   These are not macro-built, so CT.1 does not touch them.
9. **Digest domains and names outside the twin families**, in files CT.1 edits only for the macro:
   `roll.charter_pending.{funding,record,intent_guard,intent_key}.v1`;
   `roll.paid_resale.{record,acceptance,house_royalty_basis,legacy_economics,legacy_royalty_basis,intent_guard,intent_key}.v1`
   and `roll.paid_holder.birth.v1`; `roll.conduct.receipt.v1` and the conduct registration names;
   the legacy purchase names `original_mind_purchase_v1` and `_v2` in `entitlement/history.rs`.
   The types' bytes are pinned; these strings sit in history and provider paths no CT.0 test runs.
10. **Two sentences in command handlers**: "No Registry recall is recorded for your selected
    filed revision." and its revocation twin are inline in handlers that need a composed engine.
11. **Families doc 46 leaves alone** have no snapshot and are not owed one: `core_integration`
    (Q-46-5) in economy and the kernel, and the kernel's `coverage`, `ic_claim`, `registry`,
    `paid`, `paid_resale`, `charter_pending` and `conduct` validators.
12. **The crates together.** No CT.0 test sees one store written by all the owners; CT.8's stories
    do, and they stay as they are (Q-46-6).

**Owed, for a second pull request** (none of it holds CT.1, CT.2, CT.3 or CT.4, whose ground is
the kernel's and economy's walks and the codec pins; CT.5 and CT.6 should not start their writer
and provider files without the first item):

- **A capture on the real composed path** for the Roll's writers and the market's providers and
  service: the store's documents and each transition's replay writes for the renewal and purchase
  twins, compared with a committed file. The only rig that drives them is the M7 story
  (`minds/bulk/original_purchase_integration_tests.rs`, nightly tier, about half an hour at the
  fast pulse), whose branches call `grant_chain`, `retries` and the House twin's steps with
  explicit commit numbers. Either the story's branches write their `Disk` to a snapshot at each
  end (a compare added to a pinned story, which neither moves nor renames it; whether its stamps
  are fixed from run to run was not checked), or CT.2's test kit gains a pre-rated store so that
  a fast composed walk can exist. Not designed here.
- The market's House purchase at the store (a test helper beside the personal twin's
  `token_family`, or the composed reader).
- Economy's purchase twins; the holder attempts on the real path; the two record pins of item 8.

**Found on the way.**

- **Field order is identity in economy.** §4.4 says a struct's field order moves no digest. That
  holds for `DefinitionDigest::for_canonical_value` (sorted keys). It does not hold for economy's
  consent hash and plan hash, which are taken over `serde_json::to_vec` of the typed value
  (`contract.rs` `consent_hash`, `hash.rs`): swapping two fields of `RenewalBasisV1` moved the
  consent hash, and with it 225 lines of the personal renewal snapshot, the stored contract among
  them. CT.1 and CT.4 must keep every field where it is; the snapshot holds them to it.
- **The snapshot assumes `serde_json` without `preserve_order`.** A stored `Value` is written with
  sorted keys because no crate in the workspace enables that feature. If one did, stored values
  and these files would both move.
- **The refusal lines quote `serde_json`'s error text**, column number included. It is the same on
  every platform, and a `serde_json` upgrade may move it. That is not a persisted byte: such a
  diff is the upgrade's, and is the one case where the file is re-blessed, by the pull request
  that upgrades, with the diff read line by line.
- No defect was found: nothing non-deterministic, nothing that differs between Windows and Linux,
  no terminal state that cannot be reached by the code that owns it.

### CT.0b

The second pull request CT.0 owed: the capture on the real composed path. Built 2026-11-14 on
main `26d30dd4b` (CT.1, CT.2 and CT.3 in). It adds four snapshot files and moves none:
`git diff --stat origin/main -- '*.snap'` shows four additions. The rule at the top of this note
holds for the four new files as it does for CT.0's.

**Built.**

- **The record**, `crates/tacsim/src/minds/bulk/ct0b_capture.rs`, a test module of the pinned M7
  story (`original_purchase_integration_tests.rs`,
  `genuine_registry_original_purchase_grant_refund_and_paired_recovery`). The story calls
  `ct0b::begin()` first and `ct0b::finish()` last. Between them, every preparation by one of the
  story's rigs of a renewal twin's or a purchase twin's market action is recorded: the actor and
  the parameters; then the provider's outcome and the transition's replay writes, or the
  provider's refusal as printed. When the rig installs that transition the record says so, and
  when the transition ended an operation (it wrote a receipt) the store's documents under every
  key the twin wrote since its last end are read back from the rig's store. A submission the
  market's dispatched service queued is recorded when the rig takes it from the engine: its actor,
  action, parameters and idempotency key.
- **The hooks**, one line each where a rig prepares or installs: the personal rig
  (`entitlement_integration_tests.rs` `prepare` and `prepare_public`,
  `dependent_owner_integration_tests.rs` `apply`), the House rig
  (`house_purchase_integration_tests.rs` `signed`, `direct`, `persist`), the two service rigs
  (`renewal_service_integration_tests.rs` and `house_renewal_service_integration_tests.rs`, `next`
  and `land`), the IC rig that runs the two-seat House purchase
  (`paid_resale_integration_tests.rs` `prepare` and `take_install`,
  `house_two_seat_integration_tests.rs` `act`), the procurement tick's `land`, and the paid-resale
  route's rig (`paid_resale_route_integration_tests.rs` `prepare` and `land`), which is where a
  transferred holder renews. A hook passes its argument through: outside the story (no `begin`)
  it records nothing, and for any action that is not a twin's it returns at once.
- **Four committed files**, `crates/tacsim/src/minds/bulk/ct0b/renewal_personal.snap`,
  `renewal_house.snap`, `purchase_personal.snap` and `purchase_house.snap`, compared by
  `mush_testkit::snapshot` at the story's end. A difference fails the story with the kit's
  report (the file, the line, both sides).
- **A story that stops early still reports.** `begin` returns a guard the story holds. If the
  story fails before its end (a step refused, a check failed), dropping the guard compares the
  record so far with the committed files as a prefix and prints for each file the first line
  that differs, or that the record agrees as far as it goes
  (`CT0B_CAPTURE file=… preparations=N: first difference, line L`). A change that makes a
  provider refuse a step therefore names the preparation that is now refused, not only the
  story's own error.
- **In the kit**, `mush_testkit::snapshot::refuses_left_out`: the pin for a decode-only
  attribute that requires a key (below, "The smaller pins").

No production source was edited and no visibility was widened. The story's name, tier, path and
`M7_STORY` lines are as they were; every line the record prints of its own starts
`CT0B_CAPTURE`.

**The route, and the measurement behind it.** CT.0 named two routes. This is the first: the
story compares. The second (a fast composed walk over a pre-rated store) needs a generator and a
staleness check that CT.2's note describes and nobody has built, and the measurement did not
call for it:

| Question | Measured | Answer |
|---|---|---|
| Are the story's stamps, tokens and ids the same from run to run? | Three runs of one binary at once on conclav.dev, the unprojected record of each (7.3 MB, every digest and number as it is) compared by `cmp` | Byte-identical, all four files: three runs of the first recorder, two of the second, and with the final recorder three more (two at once and the blessing run), whose unprojected records have the same SHA-256 as each other |
| And on Windows? | One run on the owner's Windows box against the three Linux runs | Byte-identical, unprojected and projected: the final recorder's four records on Windows have the SHA-256 of the Linux ones, and the story passed there against the files blessed on Linux. The genuine rating gives the same certificate on both. The hosted Linux runner of branch CI passed against them too |
| What does the compare add to a pinned story? | The recorder times itself (`CT0B_CAPTURE recorder_ms=… compare_ms=…`) | 0.16 s in the hooks and 0.4 s comparing, of a 970 to 1,000 s story on conclav.dev; 0.9 s of 1,057 s on Windows. Noise |
| Does the record see every twin transition the story runs? | An audit build (never committed) in which the engine printed every composed preparation and the recorder every one it saw | The first audit found 30 renewal preparations the hooks missed, all in the paid-resale route's rig, which was then hooked. The second: 353 of 353. The story's other 119 market preparations are the legacy original purchase (versions 1 and 2) and the paid resale, which are not twins |
| Would another programme's edit move the committed files? | Two variants (never committed), each compared with the committed files: a comment added to `crates/tacsim/src/minds/fees.rs`, which the registry manifest pins by its source text; and every object made after the shipped content renumbered by seven | The comment moves 2,348 lines of the unprojected record and none of the projected one (measured with the first and the second recorder; the final one renames a superset) The renumbering moves 2,006 lines of the unprojected record (the House purchase's none: its charter is made before the content loads) and none of the committed files: the story passes. A first projection that renamed only `charter`, `acting`, `object` and `Object` failed this check on `holder` and `buyer`, which were added |

The story is 16 to 17 minutes on conclav.dev, not the half hour CT.0's note gives; on the hosted
runner the whole CI cell, build included, took 20 minutes (run 43613654718).

**What a file holds.** Only what the twins' families own, so that a change elsewhere does not
re-bless it. The projection is in `ct0b_capture.rs` and is five rules:

- A write in the `market`, `roll` or `economy` namespace is written as the tagged JSON a bundle
  carries. Any other write (an object, another module's document) is written by its kind,
  namespace and key alone.
- Of a journal chunk, only the entries the transition appended to what the rig's store held. A
  chunk's earlier entries are other transactions' (a mint, a filing, a legacy purchase).
- No digest. Every digest on this path inherits TacSim's registry manifest: each action's
  descriptor refers to it, and the manifest pins the unit catalogue and the source text of more
  than twenty files of the minds' own code. A 64-digit hex string, and a hash written as an array of
  32 bytes (economy's consent hash and plan hash), is written `<dN>`, numbered by its first
  appearance in its section. What the digests join is kept; their values are not.
- No object number. The story's worlds load the shipped out-of-character content (about a
  hundred objects), so a charter or a character made afterwards has a number that moves with
  every room a content pack adds. The numeric value of a field that holds a charter, a character
  or a principal (`charter`, `acting`, `object`, `Object`, `holder`, `buyer`, `seller`, `actor`,
  `character`, `publisher`) is written `<oN>`, numbered the same way, as is the actor of a
  request.
- The dispatched service's submission key is a digest, so the line under it says what it is a
  digest of. The recorder makes the digest again from the literal domain
  (`market.renewal_submission.v1`, `market.house_renewal_submission.v1`) over the actor, the
  action, the parameters and the service's selection, and fails the story if the key is not it.
  This is the one place a digest's domain is pinned in a committed file.

What is left in a file does depend on the story: licence, contract, hold and journal numbers
count what the story did before, and economy's `meta` document carries its counters. A change to
the story's own drive of the twins, or to an earlier phase of the same store, moves the files and
is re-blessed by whoever makes it, as the top rule says for a change outside doc 46. No doc 46
unit edits the story (Q-46-6).

**The digests are not lost.** With `CT0B_RAW_DIR=<directory>` the story also writes each record
there unprojected (`<file>.raw`, every digest and number as it is) and this run's projected one
beside it (`<file>.snap`). Two unprojected records of one tree are byte-identical (the table
above), so a unit compares its base's with its tip's: see "For CT.5 and CT.6".

**The files blessed**, once, by this pull request, on conclav.dev
(`COMMERCE_SNAPSHOT_BLESS=1` on the story):

| File | Preparations | Of them refused | Installed | Service submissions | Lines | Size |
|---|---|---|---|---|---|---|
| `renewal_personal.snap` | 113 | 31 | 70 | 15 | 1,839 | 1.25 MB |
| `renewal_house.snap` | 50 | 14 | 26 | 10 | 880 | 0.59 MB |
| `purchase_personal.snap` | 161 | 30 | 125 | none | 2,770 | 1.69 MB |
| `purchase_house.snap` | 29 | 5 | 16 | none | 524 | 0.30 MB |

353 preparations, the audit's count, and 3.83 MB of text. A line is a whole stored document, as
in CT.0's files.

**What the composed path reaches**, by file:

- `renewal_personal.snap` (`market/{draft,fund,settle,refund,close}_mind_renewal@1` and
  `close_mind_renewal_service@1`; the Roll's `renewal/writer.rs` `draft`, `fund`, `terminal`,
  `close` and `service_close`): drafted, funded, granted, refunded, closed unfunded (declined and
  expired), closed by the service (deadline reached and clock invalidated); all four attempts,
  `automatic`, `buyer_retry` and, after a paid resale in the same store, `holder_automatic` and
  `holder_retry`; a character holder and an organisation's acting character; the service's
  submissions of a draft, a fund, a grant, a refund, a close and its own close, with both close
  keys (`renewal.close.d.…` and `renewal.close.c.…`).
- `renewal_house.snap` (the House twin's six actions; `house_renewal/writer.rs`): the same
  states and attempts but the expired close, which the story does not drive for a House.
- `purchase_personal.snap` (`market/{list,quote,purchase,settle,refund}_original_mind@3` and
  `…_derived_mind@1`; the Roll's `entitlement/writer.rs` `fund` and `terminal`): listed, quoted,
  funded, granted and refunded, for `original_v3` and `derived_v1`, open and open-resale forms,
  a character buyer and an organisation's.
- `purchase_house.snap` (`market/{list,quote,purchase,settle,refund}_house_mind@1` and `@2`;
  `house_purchase/writer.rs` `fund` and `terminal`): the same states for `house_v1` and for the
  two-seat `house_v2`, sealed form.

This closes, on the composed path, four items of CT.0's "Not covered": item 1 (the Roll's
writers; their readers run too, and what they read is in the rows the writers write), item 2
(the outcome kinds `drafted`, `funded`, `granted`, `refunded`, `closed` and `service_closed`; both
provider refusals as printed; both close key formats; the submission key's domain), item 4 (the
House purchase's replay writes, with each row's family and schema envelope and the `registry`
counts document, `market_mind_house_counts`) and item 5 (the transferred-holder attempts).

**The smaller pins.**

- **The market's House renewal submission domain** (CT.0's mutation M10, caught by nothing): the
  fifth rule above. Mutations 7 and 8 below.
- **The market's House purchase at the store**: by the composed capture (`purchase_house.snap`
  holds its replay writes), not by a test helper beside the personal twin's `token_family`,
  which would have been an edit to `mind_house/families.rs`.
- **The three decode-only attributes CT.1 found unpinned.** `refuses_left_out` takes a fixture
  whose optional field is absent and checks that it says so with an explicit `null`, that its
  bytes decode back, and that the same object with the key left out is refused. Called from the
  three `ct0_` tests that already build the fixtures, with no line added to a committed file:
  the market's `HouseReceiptV1` (`mind_house/ct0_snapshot.rs`, a refunded receipt's
  `issued_at_ic`), the Roll's `ConductResult` (`commerce/tests/ct0_conduct.rs`, a cancelled
  result's `artifact` and `measurement`) and `MeterSummary` (`rating/tests/ct0_wire.rs`, the
  `peak` of a meter that never thought). Each attribute was removed alone on a throwaway clone
  and its test failed: `HouseReceiptV1 (refunded, not issued): a record with `issued_at_ic` left
  out decoded`, `ConductResult (cancelled): a record with `artifact` left out decoded`,
  `MeterSummary (no think, no peak): a record with `peak` left out decoded`.
- **Not done:** economy's purchase twins as a crate's own walk, and the two record pins of
  "Not covered" item 8 (`PendingRecord`, `PaidResaleRecord`). Economy's side of a purchase is in
  the composed files (every `economy` write of a purchase transition: the contract, the hold, the
  receipts, the appended journal entry), without its digests.

**The mutations.** Each was made alone on a throwaway clone on conclav.dev (`perl` over one
file, the story's test binary built, the file put back, the story run), never in the worktree.
"Committed files" is what the story said; "Unprojected record" is the mutant's `CT0B_RAW_DIR`
against the unmutated run's.

| # | Where | The mutation | What the story said | Unprojected record |
|---|---|---|---|---|
| 1 | the Roll, renewal records (a digest domain the writer stores) | `roll.mind.renewal_predecessor.v1` to `.v2` | failed in the walk, at the first renewal draft (340 s): `Provider { action: … draft_mind_renewal … message: "This renewal is unavailable or has changed." }`. Re-run on Windows with the guard (474 s): `CT0B_CAPTURE file=renewal_personal preparations=1: first difference, line 24`, committed `[0001 market/draft_mind_renewal@1]`, produced `[0001 market/draft_mind_renewal@1, refused]` | the story stopped |
| 2 | the Roll, term economics (the digest the renewal writer's grant stores) | `roll.mind.paid_term_economics.v1` to `.v2` | failed in the walk, at the same draft (337 s), with the same refusal | the story stopped |
| 3 | the Roll, commercial roots (the key prefix of a renewal operation's root) | `v1/renewal_operation/{token}` to `v1/renewal_op/{token}` | failed in the walk, at the same draft (339 s), with the same refusal | the story stopped |
| 4 | the Roll, `renewal/writer.rs` (the order of two writes in the grant's bundle) | the `paid_terms` and `term_economics` inserts swapped | **not caught**: the story passed (971 s) | **not caught**: no line differs |
| 5 | the market, `mind_renewal/publication.rs` (a field of the receipt a provider assembles) | `predecessor: self.binding.predecessor()` to `self.binding.next()` | failed in the walk (340 s): the service's close, `close_mind_renewal_service … "This retained renewal cleanup is unavailable."` | the story stopped |
| 6 | the market, `mind_renewal/provider.rs` (an outcome kind) | `"drafted"` to `"draft"` | `SNAPSHOT MOVED: …/ct0b/renewal_personal.snap (33 of 1840 lines)`, first difference line 27: `outcome = {…"kind":"drafted",…}` | 66 lines, all in `renewal_personal` |
| 7 | the market, `mind_house_renewal/service.rs` (the House submission domain, CT.0's M10) | `market.house_renewal_submission.v1` to `.v2` | `SNAPSHOT MOVED: …/ct0b/renewal_house.snap (6 of 881 lines)`, first difference line 251: committed `key is = renewal.<the digest of market.house_renewal_submission.v1 over the actor, the action, the parameters and the selection d/2/1>`, produced `key is = not the service's submission digest` | 24 lines, all in `renewal_house` |
| 8 | the market, `mind_renewal/service.rs` (the personal submission domain) | `market.renewal_submission.v1` to `.v2` | `SNAPSHOT MOVED: …/ct0b/renewal_personal.snap (11 of 1840 lines)`, first difference line 22, the same `key is` line | 44 lines |
| 9 | the Roll, `renewal/protocol.rs` (an action id the market's dispatched service submits: `service.rs` `action` selects by it) | `Self::Fund => "fund_mind_renewal2"` | failed at the first renewal boot (339 s): `economy: renewal composition or complete recovery refused` | the story stopped |
| 10 | the market, `mind_renewal/service.rs` (the service-close key, which is persisted) | `"renewal.close.{}.{}.{}"` to `"renewal.closed.{}.{}.{}"` | `SNAPSHOT MOVED: …/ct0b/renewal_personal.snap (1803 of 1844 lines)`, first difference line 42: `key = renewal.close.d.000000000000000000000000008adb08.<d1>` | 12 lines |
| 11 | the market, `mind_renewal/registration.rs` (a printed sentence of a provider) | `"This renewal is unavailable or has changed."` to `"This renewal is unavailable or changed."` | `SNAPSHOT MOVED: …/ct0b/renewal_personal.snap (28 of 1840 lines)`, first difference line 159: `refused = action provider rejected market:action:draft_mind_renewal@1#<d1>: ProviderRejected: This renewal is unavailable or has changed.` (on Windows, 1,308 s) | 56 lines, all in `renewal_personal` |
| 12 | the market, `mind_entitlement/provider.rs` (a purchase twin's outcome kind) | `"funded"` to `"fund"` | `SNAPSHOT MOVED: …/ct0b/purchase_personal.snap (35 of 2771 lines)`, first difference line 60: `outcome = {…"id":"mind_entitlement_recorded"…"kind":"funded",…}` (on Windows, 1,292 s) | 70 lines, all in `purchase_personal` |
| 13 | the Roll, commercial roots (the House renewal operation's key prefix) | `v1/house_renewal_operation/{token}` to `v1/house_renewal_op/{token}` | failed in the walk, in the House phase (39 s): `draft_house_mind_renewal … "This renewal is unavailable or has changed."` | the story stopped |

Thirteen mutations; twelve fail the story. Every kind the brief named is there: a digest domain
in what a Roll writer stores (1, 2), a key prefix (3, 13), the order of two writes in a writer's
bundle (4), a field of a record a market provider assembles (5), the House renewal submission
domain (7, with its personal twin 8), an action id the market's dispatched service submits (9)
and a printed sentence of a provider (11); and the outcome kinds (6, 12) and the service-close
key (10) besides.

- **Two ways to fail.** Where the mutated byte is one a later step reads back (a stored digest,
  a root's key, a receipt the pair checks), the walk refuses before any compare: the Roll's
  writers and the kernel recompute what they were handed over the stored rows, and the story's
  error names the refused action (1, 2, 3, 5, 9, 13). Where nothing reads it back (an outcome, a
  key the service submits, a sentence), the story reaches its end and the compare names the file
  and the line (6, 7, 8, 10, 11, 12). These runs were made before the guard above existed; with
  it, a walk that refuses also prints where the record left the committed files. Mutation 1 was re-run with it, on Windows: the personal renewal file's first
  difference is line 24, `[0001 market/draft_mind_renewal@1]` produced as
  `[0001 market/draft_mind_renewal@1, refused]`, and the other three files agree as far as they
  go.
- **Mutation 4 is not caught, and nothing needs to catch it.** The kernel sorts a bundle's
  writes by their target before they are a bundle (`canonicalize_replay_writes`,
  `crates/mush-core/src/authoritative.rs`), so the order a writer emits them in is not persisted:
  the mutant's unprojected record is the unmutated one, byte for byte. Only a test of the
  writer's emitted order before canonicalisation could see it, and it would pin a detail no store
  and no reader sees. Not built.
- **The procedure.** Mutations 1 to 10 and 13 ran on conclav.dev, each alone on a throwaway
  clone: `perl` over one file, the story's test binary built, the file put back, the story run
  with its own `CT0B_RAW_DIR`. conclav.dev stopped answering (ssh on 22 and 2222 timed out) at
  about 16:17 UTC with 11 and 12 running, so those two were run on the Windows box: each built
  alone with the file put back after the build (`git checkout` of a file with no uncommitted
  work), then the two stories side by side.

**Not covered.** A later unit must not read these files as proof of what is listed here.

1. **A digest's value, in the committed files.** They hold no digest, so a digest domain that
   nobody recomputes moves no committed line. On this path most are recomputed (by the kernel's
   validators, or by the paired owner), and such a change fails the story itself (mutations 1
   and 2); the two submission domains are pinned by the fifth rule. For every other digest the
   proof is the unprojected comparison below, which a unit must run: a green story is not it.
2. **The order of a writer's writes.** The kernel sorts a bundle's replay writes by their target
   (`canonicalize_replay_writes`), so the order a writer emits them in is not persisted and no
   file can show it (mutation 4).
3. **What the story does not drive.** An expired close of a House renewal; a renewal or a
   purchase under a changed manifest; a Follow birth. The kernel's and the crates' own walks
   reach some of these (CT.0's tables); the composed path does not.
4. **What a prepared action carries beside its outcome and its writes**: its definitions, its
   eligible witnesses, its durable outbox and its semantic events are not recorded. Nor is an
   offer's presentation (verb, label, help): the caller transitions are selected by verb, so a
   changed verb fails the story, but the service-close offers (`mind-renewal-service`,
   `house-renewal-service`) are selected by action and their presentation is still unpinned
   (CT.0's "Not covered" item 3).
5. **The readers' facts as such.** `reader.rs` runs on this path and what it reads is in the
   rows the writers write; a fact a writer checks and does not store is seen only as a refusal.
6. **Why a provider refused.** Every refusal of a twin's provider is one sentence
   ("This renewal is unavailable or has changed.", "This retained renewal cleanup is
   unavailable.", "No personal mind entitlement is available under these terms.", "No house mind
   licence is available under these terms."), whatever the writer's `PublicationError` or the
   owner's reason was. The
   file pins the sentence and that the story's refusals are refusals; a reworded internal reason
   passes unseen, as CT.0's item 7 says of the kernel's.
7. **The other families on the path**: the legacy original purchase (versions 1 and 2), the paid
   resale, the charter transfer, the filings, recall and revocation. Their transitions run in
   the story and are not recorded.
8. **An object number in a field the projection does not name.** The list of fields is by
   type, and the renumbering variant found two (`holder`, `buyer`) that the first list lacked. A
   new field that carries a character made after the content would make a file move with the
   shipped content; the variant's patch is kept with this unit's hand-over note.
9. **Economy's purchase twins by a crate's own walk**, the two record pins of item 8, the
   kernel's refusal reasons (item 7) and the registrations a composed engine cannot enumerate
   (item 3): as CT.0 left them.

**For CT.5 and CT.6.** The story is the proof for the Roll's writers and readers and the
market's providers and service. Two commands, from the repository root:

```sh
STORY=minds::bulk::registry_integration_tests::original_purchase::genuine_registry_original_purchase_grant_refund_and_paired_recovery

# 1. The committed files. A moved line fails the story with SNAPSHOT MOVED, the file, the
#    line and both sides.
cargo test --locked -p tacsim --lib $STORY -- --ignored --exact --test-threads=1 --nocapture

# 2. The digests. The unprojected record at the unit's base, then at its tip, compared.
#    (Two clones side by side, or one checkout twice.) The base must have CT.0b.
CT0B_RAW_DIR=$HOME/ct0b-raw/base cargo test --locked -p tacsim --lib $STORY -- --ignored --exact --test-threads=1 --nocapture
CT0B_RAW_DIR=$HOME/ct0b-raw/tip  cargo test --locked -p tacsim --lib $STORY -- --ignored --exact --test-threads=1 --nocapture
diff -r $HOME/ct0b-raw/base $HOME/ct0b-raw/tip && echo "no byte moved on the composed path"
```

- **Run time.** On conclav.dev the story is 16 to 17 minutes (970 to 1,000 s measured, up to
  six at once), and its test binary rebuilds in three to four minutes after a change to `mush-roll`, `mush-market` or the
  story's files, at sixteen jobs (a cold build was not timed): so command 1 is about twenty
  minutes, and command 2 about twenty-five with the two runs side by side. On the owner's
  Windows box the story is 18 to 22 minutes (1,057 s alone, 1,307 s beside a build) after a
  ten-minute build at two jobs. On a hosted runner the whole CI cell, build included, took 20 minutes (run 43613654718).
- **In branch CI.** `gh workflow run ci.yml --ref <branch>` runs command 1 as the cell
  "M7 author and buyer lifecycle" of `mind-commerce-acceptance` (the tacsim stories do not read
  the pulse). Pull-request CI does not run it: the story is nightly tier, as before.
- **Command 2 is the unit's own.** No job runs it. Both records come from the same tree but
  for the unit's change, so a difference is the unit's: `diff` names the file and the line, and
  the line's section says which action of which preparation. If the unit edits a file the
  registry manifest pins (none of `mush-roll`, `mush-market`, `mush-economy` or `mush-core`
  is), every digest moves and command 2 proves nothing: command 1 still does.
- A unit that moves no persisted byte, key, digest or sentence leaves both commands green. A unit may
  not re-bless: `COMMERCE_SNAPSHOT_BLESS=1` on the story is for a change outside doc 46 that
  means to move a line, with the diff named in its cut note.
- What these commands do not prove is the list above. For CT.5 that is chiefly item 3 (the
  ends the story does not reach, which the Roll's own `ct0_` walks hold structurally) and
  item 6; for CT.6, item 4 (the service-close presentation) and the commands' printed text,
  which the market's own snapshot holds.

**Found on the way.**

- **The registry manifest pins source text**, more than twenty files of `crates/tacsim/src/minds`
  and `crates/mush-mind/src` (`implementation()` in `minds/registry/manifest.rs`). A comment
  added to one of them moves the manifest's digest and with it every commerce digest on the
  composed path: 2,348 lines of the unprojected record. That, more than the unit catalogue, is
  why the committed files can hold no digest.
- **The composed path is deterministic to the byte**, from run to run and between Windows and
  Linux: commit tokens are the story's own numbers, every stamp comes from a clock the story
  sets, and the genuine rating gives the same certificate on both platforms.
- **A bundle's writes are sorted by target** before they are persisted (the third item above).
- **A twin's provider has one refusal sentence** (the renewal service's close has its own). The
  80 refusals the story provokes have many causes and print four lines between them.
- **conclav.dev stopped answering** (ssh on 22 and 2222 timing out) at about 16:17 UTC on
  2026-11-14, while this unit had about eight story runs and two 14-job builds on it. Whether
  the load did it is not known. The owner treats it as a one-off (the box has run as hard
  before; it may have been the provider's), so it sets no limit. It was back half an hour later.
- No defect was found.

## CT.1

One wire macro. Built 2026-11-13 on main `2b1f5aa3b`. No snapshot file moved.

**Built.**

- **`mush_core::checked!`**, in `crates/mush-core/src/checked_wire.rs` (a private module that
  holds the macro, its documentation and its tests; `#[macro_export]` puts the macro at the
  crate's root). It builds what each copy built: the domain struct, deriving `Clone`, `Debug`,
  `PartialEq`, `Eq` and `Serialize`; a private wire struct with the same fields in the same
  order, deriving `Deserialize` with `deny_unknown_fields`; and a `Deserialize` for the domain
  struct that decodes the wire struct, moves the fields across and calls the domain struct's
  own `validate`, whose text is the refusal.
- **The twenty copies are deleted** (`record!`, `closed!`, `checked!`, `strict!`, `data!`), and
  their 93 calls call the one macro. A call's braces, the fields, are not touched: in the diff a
  call is its head line and nothing else. Each of the twenty files gains
  `use mush_core::checked;`, and eight lose a `serde` name they imported only for their copy
  (the expansion names `::serde` itself).
- **The head carries what the copies differed in**, so nothing is a default:

  `checked!(VIS struct Name, NameWire, FIELDS fields[, attrs on wire | attrs on both], { … })`

  - `VIS` is the domain struct's visibility: `pub` in nineteen files, `pub(crate)` in the
    Roll's `charter_pending/records.rs`. The macro takes any visibility and widens none.
  - `FIELDS` is `pub`, `pub(crate)`, `pub(super)` (any `pub(…)`) or `private`, for every field
    of the domain struct. Three arms of the macro read it and hand it on as one group.
  - Where a field's attributes land is three more arms. With no clause the fields take no
    attribute, and one does not compile. `attrs on wire` puts them on the wire struct only.
    `attrs on both` puts them on both. Each arm writes a field as
    `[visibility] [the domain struct's attributes] [the wire struct's attributes] name: type`,
    and one last arm builds the two structs and the decode from that.
  - The clause is on a call only where the call has an attribute: eleven calls of the 93
    (nine `attrs on wire`, two `attrs on both`). A record with no attribute says nothing, and
    whoever gives it one must say where it lands.

  What each file's calls say:

  | Files | Was | `VIS struct` | `FIELDS fields` | Calls | With an attribute |
  |---|---|---|---|---|---|
  | economy `managed_renewal/subject.rs`, `managed_house_renewal/subject.rs`, `managed_coverage/subject.rs` | `record!` | `pub` | `pub` | 3, 3, 4 | 1, 1, 1, `attrs on wire` |
  | economy `managed_charter_transfer/subject.rs`, `managed_paid_resale/subject.rs` | `closed!` | `pub` | `pub` | 4, 7 | none |
  | roll `renewal/records.rs`, `house_renewal/records.rs` | `checked!` | `pub` | `pub(crate)` | 3, 3 | 1, 1, `attrs on both` |
  | roll `paid_resale/records.rs` | `checked!` | `pub` | `pub(crate)` | 6 | none |
  | roll `charter_pending/records.rs` | `closed!` | `pub(crate)` | `pub(crate)` | 5 | none |
  | roll `conduct/records.rs` | `strict!` | `pub` | `pub(super)` | 6 | 1, `attrs on wire` |
  | roll `entitlement/economics.rs` | `record!` | `pub` | `pub(crate)` | 5 | none |
  | roll `rating.rs` | `record!` | `pub` | `private` | 22 | 2, `attrs on wire` |
  | roll `house_purchase/economics.rs` | `data!` | `pub` | `pub(crate)` | 3 | none |
  | market `mind_renewal/records.rs`, `mind_house_renewal/records.rs`, `mind_paid_resale/records.rs` | `data!` | `pub` | `pub(super)` | 2, 2, 3 | none |
  | market `mind_entitlement/records.rs`, `mind_entitlement/purchase.rs`, `mind_house/purchase.rs` | `data!` | `pub` | `pub(super)` | 4, 2, 2 | 1, 1, 1, `attrs on wire` |
  | market `mind_house/records.rs` | `data!` | `pub` | `pub(super)` | 4 | none |

  The market's `mind_house/records.rs` copy placed attributes on the wire struct and no call of
  it ever had one, so its four calls take the plain head.

**Tests.**

- **The snapshot.** The 31 `ct0_` tests of the four crates pass on Windows and on Linux, and the
  three of `mush-industry` and `tacsim` on Linux; `git diff --stat origin/main -- '*.snap'` is
  empty. No `ct0_` test and no test file of the three crates was edited.
- **The crates' own tests**, on Linux (conclav.dev), `cargo test -p <crate> --lib`: `mush-core`
  836 (829 on main and the macro's seven), `mush-economy` 279, `mush-roll` 367, `mush-market`
  136, the same counts as main, all passing.
- **The macro's own**, in `checked_wire.rs`: seven unit tests and two doc tests. They pin what
  no record that exists today can: that an attribute under `attrs on wire` never reaches the
  encode, that one under `attrs on both` does, that the plain head refuses an attribute (a
  `compile_fail` doc test beside the same call compiling under `attrs on wire`), each field
  visibility, the encode's order, and the two refusals with `serde`'s own text.
- **The guards.** `cargo clippy --all-targets -- -D warnings` over the four crates, CI's M7
  memory-size pass, `cargo doc --document-private-items` with warnings denied, and
  `cargo test -p mush-contract-tests` (35, 80 and 84 tests) pass with nothing adjusted: the
  census and the restriction lints count no macro definition and no file this unit moved.

**The mutations.** Each is one change to `checked!`, made alone on a throwaway clone on
conclav.dev (so the worktree was never edited), with `cargo test --lib` over the four crates and
`mush-core`'s doc tests for the macro, and the file put back.

| # | The mutation | Tests that failed | `ct0_` among them | The others |
|---|---|---|---|---|
| 1 | the wire struct loses `deny_unknown_fields` | 34 | 18: every codec pin and renewal snapshot of the three crates (5 economy, 8 roll, 5 market) | 15 existing strict-wire tests (`…_reject_duplicates_unknown_null_missing…`, `new_wire_is_strict…` and kin); the macro's `an_unknown_field_is_refused`; its doc test |
| 2 | the decode does not call `validate` | 36 | the same 18 | 17 existing tests; the macro's `a_failed_validation_is_refused_with_its_own_text`; its doc test |
| 3a | `attrs on wire` puts the attributes on the domain struct only | 9 | 3: economy's `ct0_wire_coverage` and its two renewal snapshots | 4 existing (`rating`'s `missing_duplicate_unknown_and_nullable_fields_are_rejected`, economy's `new_wire_is_strict…`, two of the market's `mind_entitlement`); two of the macro's |
| 3b | `attrs on wire` puts them on both structs | 1 | none | the macro's `attributes_on_the_wire_shape_the_decode_and_never_the_encode` only |
| 3c | `attrs on both` puts them on the wire struct only | 38 | 4: the Roll's and the market's two renewal snapshots each | 33 existing renewal tests; the macro's `attributes_on_both_shape_the_decode_and_the_encode` |
| 3d | `attrs on both` puts them on the domain struct only | 3 | none | the Roll's `holder_binding_preserves_old_bytes_and_requires_exact_new_attempt_evidence`, personal and House; the macro's |
| 3e | the plain head takes attributes (and drops them) | the `compile_fail` doc test | none | none |
| 4a | the domain struct lists the first field last | 24 | 21 | three of the macro's |
| 4b | the wire struct lists the first field last | 19 | 18 (the refusal lines: `serde` lists the expected fields in the wire struct's order) | the macro's `an_unknown_field_is_refused` |

Every mutation fails a test. What the table says about CT.0's pin:

- **3b and 3e fail only the macro's own tests**, and no test of the three crates could fail on
  them: no record today has an attribute that shapes an encode under `attrs on wire`, and none
  has an attribute under the plain head, so those mutations change no byte and no refusal of a
  record that exists. They would change the first record that gained one. The macro's tests are
  the pin, in a file of this unit's own.
- **3d fails no `ct0_` test.** With the attribute off the wire struct, the Roll's
  `RenewalBindingV1` accepts an explicit `null` for `holder_basis`. The codec pin holds a
  fixture with the field and one without it, not one with a null; the existing `holder` tests
  hold that.

**Found on the way: three decode-only attributes that no unit test of their crate pins.**
Mutation 3a was caught unevenly, so each of the eleven calls that has an attribute was mutated
by itself (its attributes removed) and its crate's `cargo test --lib` run:

| Record | Attribute | What failed |
|---|---|---|
| economy `CoverageBasisV1` | `required_option` | `ct0_wire_coverage` |
| economy `RenewalOfferTermsV1`, personal | `gross_cut`, `venue_cut`, `parent_cuts` | `ct0_renewal_personal_snapshot`, `new_wire_is_strict…` |
| economy `RenewalOfferTermsV1`, House | `gross_cut`, `venue_cut` | `ct0_renewal_house_snapshot` |
| roll `RenewalBindingV1`, personal and House | `default`, `skip_serializing_if`, `present_holder` | the renewal snapshot and the `holder` test, each |
| roll `CaseResult` | `required_option` | `missing_duplicate_unknown_and_nullable_fields_are_rejected` |
| market `EntitlementSubjectV1` | `codec::required` | `required_nulls_unknowns_duplicates_and_bounded_arrays_refuse` |
| market `EntitlementReceiptV1` | `codec::required` | `terminal_receipt_requires_exact_phase_stamp_and_grant_digest` |
| **market `HouseReceiptV1`** (`mind_house/purchase.rs`) | `codec::required` | **nothing** |
| **roll `ConductResult`** (`conduct/records.rs`) | `required`, twice | **nothing** |
| **roll `MeterSummary`** (`rating.rs`) | `required_option` | **nothing** |

Each of the three attributes makes a decode refuse a record whose key is left out, where
`serde` would otherwise read a missing `Option` as `None`. Nothing in the record's own crate
fails if the attribute goes (the market's integration tests and `tacsim` were not run for
this). CT.1 did not move them: the macro puts them where the copies did, and mutation 3a
proves the macro's side. CT.0's codec pin records two refusals, an unknown field and a failed
validation, and not a key left out. **Owed, not built here:** a refusal line for a left-out key
on those three records. It belongs in the Roll's and the market's test modules, which are
CT.2's while it runs, or in the codec pin itself, which would add lines to committed snapshot
files.

**Divergences.**

- **The mutations were run on a clone, not by a WIP commit and a revert.** The work was
  committed first; each mutated file was written beside the worktree, copied over the clone's
  file on conclav.dev and copied back, as CT.0 did. The worktree's files were never mutated.
- **The clause is a call's, not a file's.** A copy placed attributes for its whole file. The
  one macro asks only the calls that have an attribute, which changes no expansion.
- **A test module was added**, in `mush-core`, for the macro. It is not a test file of the
  three crates.
- **"Three placements" are two that can differ and one that refuses.** For a call with no
  attribute the three arms expand to the same tokens.

**Not built.**

- **Generic records.** `checked!` takes no type parameter, as no copy did. A twin record that
  is generic over its issuer needs a new arm, and `serde`'s derive then adds its own bounds;
  that is for the unit that first wants one.
- The hand-written pairs (`FundingWire`, `TerminalWire`, the parents list, `GrossWire` and
  kin) and the other macros (`row!`, `digest_id!`, `identity!`, `int_id!`, `numeric_id!`,
  `put_record!`, the `check!` family) are as they were.
- The pin owed above, and everything CT.0 lists as owed.

**For CT.3 and the twin units.**

- Inside `mush-core` the macro is `crate::checked!` (or `use crate::checked;`). The kernel has
  no call of it today: no file of `action_state/terminal_capacity/` defined a copy.
- **A wire struct's name is part of a refusal** (`invalid type: …, expected struct
  BindingWire`), and no snapshot line holds that text. The two twins of every family already
  give their wire structs the same names, so a merged twin keeps them.
- The personal and the House file of each renewal family have the same head on every call
  (the same visibilities, the same clause), so merging them changes no head.
- A field added to a record goes where it should sit in the stored bytes, and an attribute
  that must shape the encode needs `attrs on both`: under `attrs on wire` a
  `skip_serializing_if` compiles and does nothing.

**The yardstick, before and after.** Before is main `2b1f5aa3b`, which is CT.0's "after" to the
line.

| Root | Before: cloned of lines | Rate | After: cloned of lines | Rate |
|---|---|---|---|---|
| `crates/mush-market/src` | 10,698 of 45,375 | 23.6% | 10,588 of 45,277 | 23.4% |
| `crates/mush-economy/src` | 22,237 of 101,401 | 21.9% | 22,181 of 101,321 | 21.9% |
| `crates/mush-roll/src/commerce` | 16,942 of 91,868 | 18.4% | 16,924 of 91,757 | 18.4% |
| `crates/mush-core/src/action_state/terminal_capacity` | 11,605 of 40,352 | 28.8% | 11,605 of 40,352 | 28.8% |

289 lines left the three crates (the twenty definitions, less twenty imports) and 184 cloned
lines with them. The macro was small beside the twins; what this unit buys is one place to
read and to change, not the rate.

CT.2 landed first (main `003de2253`), and this unit was merged with it. With both in, the four
totals are 10,522 of 45,090 (market, 23.3%), 20,913 of 99,897 (economy, 20.9%), 16,802 of
91,607 (the Roll's commerce, 18.3%) and 11,605 of 40,352 (the kernel, 28.8%). CT.3 compares
with these.

**Check time** (§7.4), `cargo check -p <crate>` for the library, the crate's root file touched
and every dependency already checked, in milliseconds, the median of the runs:

| Crate | conclav.dev before | after | Windows before | after |
|---|---|---|---|---|
| `mush-core` | 8,642 | 8,403 | 3,068 | 3,017 |
| `mush-economy` | 8,532 | 8,370 | 3,106 | 2,856 |
| `mush-roll` | 8,239 | 8,059 | 2,927 | 2,820 |
| `mush-market` | 3,595 | 3,422 | 1,404 | 1,471 |

conclav.dev is five runs with `CARGO_INCREMENTAL=0` and six jobs, so each is the whole crate
checked again; the box was busy (a load of 15 to 23) and the runs of one crate spread by up to
a second. Windows is three runs at `-j 2` with incremental compilation on, the figure CT.0's
table holds. Neither moved outside its own spread: a call now expands three times (the head,
the placement, the build) where it expanded once, and 93 calls of that do not show.

## CT.2

The test kit. Built 2026-11-13 on main `2b1f5aa3b`, beside CT.1 (source against tests: this unit
edits `mush-testkit` and test files only, and none of CT.1's twenty files).

**Built.** Five modules in `mush-testkit` and the three crates' tests moved onto them. The kit
names no commerce type. That is forced, not a choice: `mush-economy` dev-depends on
`mush-testkit`, so a kit function that named `EconomyStore` would name the library build's type,
which is not the type economy's own unit tests see. Everything is the kernel's vocabulary
(`mush-core`) and command text.

| Module | What it is | Files that use it (economy, the Roll, the market) |
|---|---|---|
| `checks` | `check!`, `check_eq!`, `check_ne!`, `refused`, `CheckFailed`: the one implementation of what each crate kept as `src/test_checks.rs` (about 180 lines a copy) | every test of the three crates, through a forwarding shim a crate (below) |
| `fixtures` | `token(n)`, `tokens(ids)`, `digest(n)` (the files' `hash` or `digest`), `stamp(at)`, `boxed_engine` (the files' `boxed_test_engine`), and the one-line drivers `texts`, `has_error`, `ok`, `err` | 63 files: 26, 25, 12 |
| `cold` | the cold restart: `Boot`, `take_typed_rows`, `load` (typed rows hydrated before any module's boot, then the modules), `load_typed`, `fold_replay_writes_into`, and on `TestWorld` `try_cold_loaded`, `cold_reboot_into`, `cold_restart` | 27 files: 19, 5, 3 |
| `funding` | `Funding`: the setup lines that open and fund fixture accounts, run as God, the first refusal an error that names the line | 9 files, all economy |
| `owner_rig` | the mock-owner skeleton: `MockOwner` (a rig's names), `declare()` (the builder for its definitions) and `register_declarations`, the fixture receipt family (`FixtureReceipt`, `receipts`, `prepared_receipt`), `InertOwner` and `Unavailable`, `prepare`, `stage`, `install_staged` | 5 files, all economy |

- **The skeleton.** The three `owner_tests.rs` (`managed_coverage`, `managed_charter_transfer`,
  `managed_paid_resale`) and the two `registry_filing` rigs (`charter_transfer/tests.rs`,
  `update/tests.rs`) wrote the same skeleton out in full. Each now keeps what is its family's (the
  provider's request, the receipt's subject and rule, the fixture objects, the setup lines) and
  calls the kit for the rest. The five files went from 1,361, 1,350, 1,365, 996 and 1,333 lines to
  1,113, 1,121, 1,112, 787 and 1,128. What a rig still spells out: its `MockOwner` constant, its
  receipt struct, its provider, its `Fixture`, and four thin methods (`prepare`, `stage`,
  `take_install`, `publish`) of three to six lines each that keep the tests' call sites as they
  were.
- **The cold restart.** No test file of the three crates partitions typed rows, hydrates or
  loads by hand any more (one `hydrate_action_state` call is left, named below). A rig still says
  which world and which modules. Three shapes are in use, and each rig took the one that keeps
  its tests' meaning:
  - `cold_restart`, a restart over a copy of the running world's objects, clock and timeline into
    a freshly composed engine: the three owner rigs. They used to rebuild a fresh world, re-spawn
    the same players and boot it twice;
  - a re-seeded world, then `try_cold_loaded`: economy's boot tests and integration tests, which
    check the fresh ids against the old ones or boot documents into a world without the players;
  - a bare `Engine`, then `cold::load` or `cold::load_typed`: the Roll's and the market's family
    rigs, and `registry_filing/update/tests/house.rs`.
- **The small helpers.** A private copy became a `use` of the kit's function, aliased where the
  file's name differs (`use mush_testkit::fixtures::digest as hash;`), so no call site and no
  test body changed for them.
- `TestWorld::reboot_into` and `AcknowledgedWorld::reboot_into` compose their engine through the
  same function as the kit's restart (`recompose`: sessions dropped, a fresh registry, the
  modules over the world's objects). Their behaviour is unchanged.

**Tests.** No test was lost or renamed. The lists are `cargo test -p <crate> -- --list`, ignored
tests included, sorted, compared name for name (Linux, conclav.dev):

| Crate | Tests before | Tests after | Ignored before and after |
|---|---|---|---|
| `mush-economy` | 809 | 809, the same names | 0 |
| `mush-roll` | 369 | 369, the same names | 0 |
| `mush-market` | 234 | 234, the same names | 1 |
| `mush-testkit` | 37 | 64: the 37 and 27 of the kit's own (one doc test's listed name carries its line number, `boot_world (line 86)`, now `(line 95)`) | 1 |

Every `ct0_*` test passes and `git diff --stat origin/main -- '*.snap'` is empty: nothing was
re-blessed. The kit's own tests cover each module: a failed check reports what the assert
reported; a restart holds what the documents hold and not what only the running world's memory
held; a typed row goes to the kernel and never to a module's boot; a module's refusal is the
restart's error in the module's words; a receipt that names another action, or that the rig's
rule refuses, does not stage; a definition that drifted from its declaration does not compose; a
refused setup line stops the funding and names itself.

Test bodies changed in three places, each asserting what it asserted: four hydrate-then-boot
pairs in the Roll's `bootstrap_tests.rs` are one `restart` call each; the market's
`mind_house_renewal/ct0_snapshot.rs` probe asks `load_typed` for the kernel's refusal
(`ColdError::Typed`) where it asked `hydrate_action_state(..).is_err()`; and the positive boots of
economy's boot tests call `try_cold_loaded` where they called `try_modules_loaded`.

**The mutations.** The kit must not make a test pass that should fail. Each line is one break of
production behaviour, made alone, and the test whose setup moved onto the kit run against it.
| # | Crate | The break | The test, and what of the kit it stands on | What the test said |
|---|---|---|---|---|
| E1 | economy | `managed_coverage/install.rs`: the bind no longer writes the hold it places, so the running store has it and the documents do not | `managed_coverage::owner_tests::native_bind_charges_once_reserves_full_liability_and_replays_exact_collateral` (the owner rig, `fold_replay_writes_into`, `cold_restart`) | failed at the restart: `Module(Msg("economy: hull coverage requires exact paired recovery"))` |
| E2 | economy | `commands.rs`: `@econ/mint` credits the system source back, not the account | `managed_paid_resale::owner_tests::resale_immediate_debits_gross_and_credits_exact_split_without_title_or_hold` (`Funding`) | failed: the acceptance is refused, the payer holds nothing |
| E5 | economy | the same break | `managed_charter_transfer::owner_tests`, the module (`Funding`) | 11 tests failed, at the initial filing the actor can no longer pay |
| E3b | economy | `managed_coverage/decision.rs`: a clean expiry before the term's end is accepted | `managed_coverage::owner_tests::expiry_clock_loss_interval_and_claim_delay_refuse_false_terminal_windows` (the owner rig's `prepare`) | `assertion failed: f.terminal(3, expired.clone()).is_err()` |
| E4 | economy | `commands.rs`: `@econ/history <account> 0` is clamped again, not refused | `tests/e4c.rs`, `vf_d8_history_validates_n_and_the_help_says_ids_are_masked` (`fixtures::err`) | panicked: `` `@econ/history 1 0` should have been refused, got: tx #0 … `` |
| R1 | the Roll | `commerce/bootstrap.rs`: a setup-mode boot skips its emptiness checks | `commerce::bootstrap_tests::setup_uses_actual_empty_economy_replay_and_does_not_open_until_active_restart` (`fold_replay_writes_into`, `cold::load`) | panicked: `Initialize refuses retained prior deployment` |
| R2b | the Roll | `commerce/renewal/records.rs`: `follows` no longer asks for the same origin | `commerce::renewal::tests::exact_transition_and_issued_clock_refuse_crossed_receipts` (`fixtures::{token, digest, stamp}`) | `assertion failed: crossed.follows(&funded).is_err()` |
| R3b | the Roll | `commerce/paid_holdings.rs`: the boot audit reconciles nothing | `commerce::paid_holdings::tests::paid_holdings_audit_requires_exactly_one_row_per_pointer` (`cold::load_typed`) | `Error: missing` |
| M1 | the market | `mind_trade/mod.rs`: the families' boot audit answers `Ok` without looking | `mind_trade::tests::lifecycle::restart_audit_refuses_missing_counter_actor_or_expiry_and_orphan_index` (`cold::load_typed`) | panicked: `missing market_mind_offer_counts must not look healthy` |
| M2 | the market | `trade.rs`: a venue gate's refusal of a quote is ignored | `tests/ic7.rs`, `ic7_a_gate_refusal_escrows_nothing` (`fixtures::err`) | panicked: `` `+market/quote 0=10` should have been refused, got: Quote #0 … `` |

Ten pairs, ten failures. E1 is the cold restart (a write dropped across it is caught, and by the
owner's own recovery, not by the fixture); E2 and E5 are the funded accounts.

Three other breaks were tried first and **passed**: a coverage loss dated at the term's end
accepted (`decision.rs`, the `occurred_at.at >= ends_at.at` condition removed), a Roll renewal
draft accepted over a head with funding pending (`renewal/chain.rs`, `head.pending.is_some()`
removed), and the Roll's boot no longer calling the paid-holdings audit (`commerce/audit.rs`).
Each was then run against main `2b1f5aa3b`, before any of this unit, and passes there too. The kit
hides nothing: in the first two another condition of the same check refuses the same input (for
the loss, the claim delay; for the draft it was not traced), and in the third the rig's own
module calls the family's audit, so the line removed is one no test of that crate reaches. They
are listed under "Found on the way".

The procedure differed from the brief's in the way CT.0's did: each mutation was made on a
throwaway clone on conclav.dev (`perl` over one file, the named test, the file put back), not in
the worktree, so no uncommitted work was at risk and nothing had to be reverted in it.

**Divergences.**

- **The cold restart is not built on `AcknowledgedWorld`.** §5.3 says "on
  `AcknowledgedWorld::reboot_into`". It is built on the same restart sequence (and shares the
  composition step with it), over a boot map with no Store behind it. `AcknowledgedWorld` drives a
  real in-memory Store and refuses a boot whose typed rows are not what that Store acknowledged
  ("typed boot must use the actual acknowledged fixture Store"). The commerce owner tests stage
  and install prepared actions directly, and a large share of them hand the restart a document
  they damaged on purpose, to see the owner under test refuse it. On `AcknowledgedWorld` the
  fixture would refuse first and the owner would never be asked. Moving them would also have
  meant rewriting what those tests do, not their setup.
- **`test_checks.rs` is still a file in each crate, 40 lines or so.** The implementation is one
  (`mush_testkit::checks`). What is left in `mush-economy`, `mush-roll` and `mush-market` is three
  forwarding macros (and, in the Roll, the conversion into `RollError`, which only the Roll can
  write). A `macro_rules!` is textually scoped and each crate's tests name the macros bare, so
  deleting the file needs `#[macro_use] extern crate mush_testkit;` in place of
  `#[macro_use] mod test_checks;` in each `lib.rs`. That is a one-line edit of a file this unit
  did not touch (production source, and CT.1 was writing in those crates); a twin unit that owns
  the crate's `lib.rs` can make it. The seven other copies (`mush-industry`, `mush-inventory`,
  `mush-proto`, `mush-server`, `mush-sim`, `mush-tactics`, `tacsim`) are outside the three crates
  and unchanged.
- **Five rigs share the skeleton, not three.** The two `registry_filing` rigs carried the same
  code and moved with the three `owner_tests.rs`.
- **In economy's boot tests a typed row now reaches the kernel.** The positive boots of
  `registry_filing/boot_tests.rs`, `registry_filing/charter_transfer/tests.rs`,
  `registry_filing/update/tests.rs` and `managed_purchase/boot_tests.rs` used
  `try_modules_loaded`, which handed the rig's typed fixture receipts to the mock module's boot as
  plain documents and left the engine's typed state empty. Through `try_cold_loaded` they are
  hydrated and validated first, as a real boot does. The tests pass unchanged.
- **The definition-drift message is the kit's.** Two rigs said "fixture drift" and three "fixture
  definition drift"; no test reads it. A refused setup line now names the line and what it
  printed, where each rig had its own fixed sentence.
- `cold_restart` copies every object of the running world, so a flag set after setup (the buyer's
  GOING flag in the paid-resale refund test) is in the restarted world, where the old fresh
  re-seed did not have it. That is what a restart over a kept store holds.

**Left where it is, by name.** A helper of the same name that means something else stays:

- `token`: big-endian over a `u64` or `u128` in economy's `managed_renewal/tests.rs`,
  `managed_renewal/tests/ct0_walk.rs`, `managed_house_renewal/ct0_snapshot.rs`, the market's
  `mind_renewal/service/tests.rs` and `mind_trade/lifecycle_tests.rs` (`id`); from a hex string in
  the Roll's `publication_tests.rs` and `ic_claims/tests.rs`; with a four-byte prefix in the
  market's `mind_commands/tests.rs`.
- `digest` / `hash`: a fixed `[255; 32]` with no argument in economy's
  `managed_paid_resale/tests.rs`; from a hex string in the Roll's `publication_tests.rs`; a
  canonical-value digest in the market's `mind_trade/tests.rs`.
- Restarts that are one party's internals (§5.3): the three `cold` helpers under economy's
  `managed_renewal/tests/` rebuild an `OfferBook` or run `bridge::recover` from documents, with no
  engine; the Roll's `src/tests.rs` `load` and `reboot` reload a `RollStore` from a disk; the one
  `hydrate_action_state` left, in the Roll's `publication_tests.rs`, hands the kernel a single
  hand-built row to see it refused.
- Boots that are not restarts: `live_boot` (the Roll's `paid_holdings`) and the market's
  `fn boot(&mut Engine)` re-run a module's audit over its own running engine; the market's
  `tests/ic7.rs` and `ic8.rs` call `TestWorld::modules_loaded` to reload edited documents into the
  running engine, whose rigs hold handles bound to the live composition.
- Boots inside test bodies that assert a module's own refusal (`RetryFullFetch`, a damaged or
  disabled family) stay `try_modules_loaded`: hydrating first would change which side refuses.
- The `fold_kv` family of economy's integration tests (`e2b`, `e3b`, `e3b2`, `e3_verify`, `e4b1`,
  `e4b2`): they return the folded rows for the tests to search and patch by key, `log:` chunk keys
  included; `fold_module_ops_into` moves those keys into `ledger`, so it is not the same thing.
- The Roll's `filing_tests.rs` `retain` folds replay writes with deletes into its own disk;
  `fold_replay_writes_into` refuses a delete, as the economy rigs did.
- Setup written as one `ok(&mut tw, GOD, …)` or `exec_ok` a line (the market's `ic*` tests,
  economy's `e*` tests and `core_integration/end_user/tests.rs`) is already the kit's driver and
  was not turned into a `Funding`.

**Not built.**

- No visibility in production code was widened and no production file was edited; no helper was
  left for want of one.
- The twins' tests as one body run for both issuers (§5.3's second sentence) are the twin units'
  (CT.3 to CT.6), not this one.
- The composed stories' rig (`PlayerWorld`, its `Durable`, `Disk` and `retain_writes`) is CT.8's
  and was not touched (Q-46-6).
- **A pre-rated store for a fast composed walk** (CT.0's "Owed") is not in this unit. The kit
  makes one half of it cheaper. The stories keep a `Durable` (a disk of documents, the objects,
  the next id, the clock), fold replay writes into it with their own `retain_writes` and boot a
  composition from it; `cold::Boot`, `fold_replay_writes_into` and `cold::load` are that
  machinery in one place, so a composed walk outside the stories does not have to copy it. What
  it would still take: (1) the kit's `Boot` holds documents only, so a kept store needs the
  objects, the next id and the clock beside it, and a file form; (2) the half hour is
  `complete_registry`, the real rating schedule, which the stories run on purpose ("no synthetic
  rating rows"); skipping it means a store captured after it, and such a store is tied to the
  tree's store formats and to the registry manifest, which names the unit catalogue, so it cannot
  be a committed blob: it needs a generator (the first act of the M7 story, at the fast pulse)
  and a check that refuses a stale one; (3) whether the story's stamps are the same from run to
  run is still not checked, and a compared file needs them to be.

**For the twin units** (CT.4 first).

- A helper that names a crate's own type cannot live in the kit: the crate's unit tests would
  get the library build's copy of the type. A rig shared by a twin's two issuers belongs in a
  `#[cfg(test)]` module of the crate, over the kit's kernel-level pieces.
- Economy's renewal tests do not stand on `owner_rig`. `managed_renewal/tests.rs` has its own
  participant-level `Rig` (no mock provider, no engine boot), a big-endian `token(u64)`, and
  `cold` helpers that rebuild an `OfferBook` or run `bridge::recover`. This unit left all of it
  alone, and `managed_house_renewal` still has only CT.0's walk. "One body run for both issuers"
  is that `Rig` made generic, which is CT.4's to do; from the kit it has the checks, `stamp`,
  `boxed_engine`, `cold::load` and `fold_replay_writes_into`.
- `cold_restart` keeps the running world's objects. A test that needs a world without them, or
  that checks fresh ids against old ones, re-seeds and calls `try_cold_loaded`.
- The name-list check is a script kept with this unit's hand-over note (`ct2-names.sh`): a
  crate's `cargo test -- --list`, sorted, against the list recorded before. Each twin unit owes
  the same comparison (§7.4).

**The yardstick, before and after** (`node scripts/clone-yardstick.mjs`; before is CT.0's
"after", reproduced to the line on main `2b1f5aa3b`):

| Root | Before: cloned of lines | Rate | After: cloned of lines | Rate |
|---|---|---|---|---|
| `crates/mush-market/src` | 10,698 of 45,375 | 23.6% | 10,632 of 45,188 | 23.5% |
| `crates/mush-economy/src` | 22,237 of 101,401 | 21.9% | 20,969 of 99,977 | 21.0% |
| `crates/mush-roll/src/commerce` | 16,942 of 91,868 | 18.4% | 16,820 of 91,718 | 18.3% |
| `crates/mush-core/src/action_state/terminal_capacity` | 11,605 of 40,352 | 28.8% | 11,605 of 40,352 | 28.8% |

The yardstick counts a clone inside one root, and economy's rigs are where the skeleton was:
economy's root is 1,424 lines shorter and holds 1,268 fewer cloned lines, the Roll's 150 and 122, the market's 187 and 66. The small helpers barely move it: a three-line `fn token` is under the eight-line
window.

Test lines (a file under a `tests/` directory, named `*tests.rs`, `ct0_*.rs` or `test_checks.rs`,
or starting `#![cfg(test)]`; an inline `#[cfg(test)] mod` counts from its first line):

| Crate | Test lines before | After | Other lines before and after |
|---|---|---|---|
| `mush-economy` | 55,111 | 53,501 | 79,457 |
| `mush-roll` | 25,701 | 25,422 | 73,881 |
| `mush-market` | 20,462 | 20,128 | 35,109 |
| the three | 101,274 | 99,051 | 188,447 |
| `mush-testkit` (its own tests) | 2,410 | 3,092 | 4,740 before, 6,044 after: the kit |

By `git diff --shortstat` against `2b1f5aa3b`: economy 30 files, 540 added and 2,150 removed; the
Roll 27 files, 114 and 393; the market 15 files, 67 and 401: 72 test files, 721 lines added and
2,944 removed. The kit is 1,976 lines in five new files, of which about a third are its
own tests (682 lines) and about a fifth its documentation (412).

**Time.** `cargo test -p <crate> --lib`, the `finished in` line, three runs each on conclav.dev
(Linux, 32 cores, other work on the box):

| Crate | Before | After |
|---|---|---|
| `mush-economy` (279 tests) | 8.74, 8.62, 9.19 s | 7.29, 7.28, 7.29 s |
| `mush-roll` (367) | 18.78, 19.32, 19.45 s | 18.12, 18.03, 18.18 s |
| `mush-market` (136) | 2.40, 2.28, 2.50 s | 1.97, 1.89, 1.91 s |
| `mush-testkit` (13, then 37) | 0.01 s | 0.01 s |

Nothing is slower. The "after" runs came out about a second faster in economy, which is more
likely the box's other load at the time than the kit: a restart over a copied world composes one
engine where the old rigs composed one and booted it twice, and that is all that changed in what
a test executes. On the owner's Windows box the same three finish in 12.50, 25.55 and 2.56 s
(one run, `-j 2`, after only).

**Checked.** On Linux (conclav.dev) and on Windows (the owner's box), the full suites of the four
crates, all features: 30 test binaries and the doc tests, 1,488 tests passed, none failed, 2
ignored, the same on both. On Linux also: `cargo clippy --all-targets --all-features -D warnings`
on the four crates; CI's M7 memory-size clippy pass on the three; `cargo doc --no-deps
--document-private-items` with `-D warnings`; `cargo test -p mush-contract-tests` (census, panic
budget, restriction lints, fiat budget: no count moved, since no worker added or removed a slot
accessor in a `tests/` file). `cargo tree -i mush-testkit -e normal` shows one dependent,
`mush-contract-tests`, as before: no `Cargo.toml` changed, and the kit took no new dependency.
The rustdoc run caught one thing, fixed before the pull request: a module documented both by an
outer comment in `lib.rs` and by its own `//!` leaves the links of the second unresolved. The
kit's modules document themselves; `mush-testkit`'s older `toy_ruleset` module has the same
shape and the same broken links under its feature, which CI's default-feature run does not build
(listed below).

**Found on the way.**

- The integration tests' in-place reboots (`e2b`'s `try_boot`, `e3b`, `e3b2`, `e3_verify`,
  `e4b1`, `e4b2`) boot the same engine a second time. `try_cold_loaded` hydrates before it loads,
  and the kernel refuses to hydrate over typed state it already holds. None of these worlds holds
  any today. A rig there that staged a typed action and then rebooted in place would now be
  refused, where before it was silently booted over.
- Three production conditions that no test of their crate guards alone (the three mutations that
  passed, above): the coverage decision's `occurred_at.at >= ends_at.at`, the renewal draft's
  `head.pending.is_some()`, and the paid-holdings audit's call from the Roll's commerce boot
  (`commerce/audit.rs`), which only the composed stories reach. A test that feeds each its one
  input would close them. None was added: a new test is not this unit's, and the list of names
  had to stay as it was.
- `mush-testkit`'s `toy_ruleset` module does not document under `--features toy-ruleset` with
  `-D warnings` (seven unresolved links, `ToyRuleset::named` and others), on main as here: its
  outer doc comment in `lib.rs` sits beside the module's own. CI documents default features only.
  Not fixed (not this unit's module).
- No defect in production code was found.

## CT.3

The renewal twin in the kernel. Built 2026-11-13 on main `003de2253` (CT.0 and CT.2 in). CT.1 was
open as a pull request and is not stood on: the kernel has no copy of the wire macro, and this
unit touches neither `checked_wire.rs` nor its export.

**The survey, before the cut.** `terminal_capacity/renewal.rs` (1,483 lines) against
`house_renewal.rs` (1,556): `diff` gives 27 hunks and 149 changed lines, and everything else is
the same text. Sorted:

| Kind | Hunks | Lines | What they are |
|---|---|---|---|
| (a) a rename | 8 | 16 | the registry's field (`mind_renewal` against `house_mind_renewal`) at seven sites, and the file's first comment line |
| (b) an identity string | 11 | 38 | 19 strings: five action ids, the service action id, the four operation, head and receipt families, the term-economics family, the two `protocol` names of the action payloads, the other issuer's two operation families in the disjointness list, four digest domains (predecessor, offer, template, capacity fingerprint) |
| (c) real logic | 8 | 95 | eight differences, below; 68 of the lines are one function, `require_house_birth` |

The eight differences that are real logic, and where each sits in doc 46 §4.3's nine:

| # | The difference | §4.3 |
|---|---|---|
| 1 | the closed purchase audits a declaration needs: both personal audits, against the House's one | 8 |
| 2 | the personal Follow birth check (`entitlement::require_follow_birth`), made before the predecessor term is read | 4 |
| 3 | the digest domain of a term's economics row (`economics_domain` of the personal or the House purchase) | 4 |
| 4 | the digest domain of the grant (`grant_domain`) | 4 |
| 5 | the royalty chain: the personal offer's must equal the grant's; the House refuses an offer that has one (the one check inverted) | 4 |
| 6 | the House birth route (`require_house_birth`: the exact allocation, the issuer's organisation and scope against the terms, no seller, lineage or parents), made after the disclosed terms agree | 4, and the kernel's share of 1 |
| 7 | the format of the economics row a Grant writes (`grant_version`) | 4 |
| 8 | at a Draft, Fund or Grant the House birth's purchase recipe must still be declared (`require_product`) | 4 |

So the kernel's surface is narrower than §4.3's nine, not wider, and the cut went ahead as one
trait. Five of the nine are not in the kernel at all: the current-authority check (2), the payee
(3), the buyer-admission policy and template acceptance (5), the `OwnCharter` stop (6) and the
account pairing (7). The validator's first line says why: money and the issuer's current
authority are the owners' obligations; core binds the paired images and the room for them. Those
five are CT.4's, CT.5's and CT.6's. The two files answered the same 124 refusal sentences
("renewal …"); one more is the personal file's alone and ten are the House's, and each of those
eleven stays in its issuer's file. The set of sentences is the same 135 before and after.

**Built.** One validator, two instantiations, in `crates/mush-core/src/action_state/terminal_capacity/`:

| File | Lines | What it is |
|---|---|---|
| `renewal_twin.rs` | 1,580 | the validator, generic over the issuer: the old personal file with 221 of its lines changed, plus the entry points |
| `renewal_twin/issuer.rs` | 118 | the sealed trait `Issuer` and the two issuer types, `Personal` and `House` |
| `renewal.rs` | 81 | `impl Issuer for Personal`, and the personal entry points |
| `house_renewal.rs` | 157 | `impl Issuer for House`, `require_house_birth` as it was, and the House entry points |

The two validators were 3,039 lines; the four files are 1,936.

*The trait as built.* `Issuer: sealed::Sealed + Sized + 'static`, visible in `terminal_capacity`
and nowhere else. Fourteen associated constants and nine functions:

| Item | What it carries | §4.3 |
|---|---|---|
| `ACTIONS`, `SERVICE_ACTION` | the six action ids | 9 |
| `PROTOCOL`, `SERVICE_PROTOCOL` | the `protocol` name in the action payloads (`personal_mind_renewal_v1`, `house_mind_renewal_v1` and their `_service_` forms) | 9 |
| `MARKET`, `ROLL`, `HEAD`, `RECEIPT`, `ECONOMICS` | the five family ids that differ. The licence and term families are the same for both issuers and are plain constants of the module | 9 |
| `OTHER_ISSUER` | the other issuer's Roll and market operation families, for the disjointness check. Written as the other implementation's own constants (`[House::ROLL, House::MARKET]`), not as a second copy of the strings | 9 |
| `PREDECESSOR_DOMAIN`, `OFFER_DOMAIN`, `TEMPLATE_DOMAIN`, `CAPACITY_DOMAIN` | the four digest domains | 9 |
| `protocol(registry)` | which of the registry's two fields is this issuer's. The fields stay two hand-written fields; this is the rename (a) | (a) |
| `birth_audits(registry)` | difference 1, with each issuer's own refusal sentence | 8 |
| `economics_domain`, `grant_domain`, `grant_version` | differences 3, 4 and 7: the economics family of the birth. Each forwards to the purchase twin's function (`entitlement::` or `house::`), which this unit does not touch | 4 |
| `follow_birth(view, binding)` | difference 2. The House's is empty | 4 |
| `royalty_chain_differs(basis, grant)` | difference 5, the personal half. The House's answers `false`; its inverse is a line of `require_house_birth` | 4 |
| `birth_route(view, binding, basis, terms)` | difference 6. The personal one is empty | 4, 1 |
| `birth_product(after, binding)` | difference 8. The personal one is empty | 4 |

Every constant holds the value its file held, byte for byte; the two tests named below hold each
to a literal, and the snapshot holds the bytes they produce.

*What stayed as two implementations, and why.* The two `impl Issuer` blocks, and
`require_house_birth` whole. Four of the nine functions are one-sided (one issuer's body is
`Ok(())` or `Ok(false)`), and they were kept as four rather than folded into fewer because each
is called at the place its check was made in the old file. The personal Follow check runs before
the predecessor term is read and the House birth route after the disclosed terms agree; folding
them into one call would change which refusal a record with two faults answers. The same holds
for the royalty comparison, which sits inside the long disjunction it always sat in. A trait with
fewer functions would have been tidier and would have moved a diagnostic.

*The entry points.* The rest of the kernel names the two families by module, by hand, as before:
`renewal::verify` and `house_renewal::verify` in the verifier rows, `audit_boot` and
`validate_changes` in the two lists, and the purchase files' calls
(`renewal::validate_current_birth`, `house_renewal::shared_output_is_renewal` and the rest). One
macro in `renewal_twin.rs`, `entry_points!`, expands in each issuer's file to the `Protocol`
alias and eight forwarding functions, so `terminal_capacity.rs` changed by one line (`mod
renewal_twin;`) and `entitlement.rs`, `house.rs`, `paid.rs`, `paid_resale/` and `action_state.rs`
by none. `RenewalProtocol` and `HouseRenewalProtocol` are `Protocol<Personal>` and
`Protocol<House>`: two types, so neither can be put in the other's registry field. The type
parameter is a `PhantomData` that serde skips; the protocol's JSON is unchanged (the snapshot's
`protocol` and `service protocol` lines).

*What did not move.* No file outside `mush-core` is edited, and in `mush-core` nothing outside
`terminal_capacity/` but the one `mod` line. The registry's family version strings
(`state_family_registry.v6` and `.v7` for the personal protocol without and with its service
action, `.v12` and `.v13` for the House's) are in `action_state.rs`, which has no diff. The
registry's fields, its four builder methods, `module.rs`'s and `resolution.rs`'s per-issuer
methods and the error enum's two variants are as they were. No `dyn`, no function pointer and no
callback was added; the one function-pointer table the kernel already had (the verifier rows)
holds the two instantiations where it held the two functions.

**Tests.** `cargo test -p mush-core -- --list`, ignored tests included, before and after:

| | Before | After |
|---|---|---|
| The library's tests | 829 | 839 |
| All targets (library, integration tests, doc-tests) | 1,084 | 1,095 |
| Under `terminal_capacity::renewal::` | 18 | 22 |
| Under `terminal_capacity::house_renewal::` | 20 | 25 |
| Under `terminal_capacity::renewal_twin::` | 0 | 1 |

No name disappeared and none changed: the diff of the two sorted lists is eleven additions,
the ten tests named below and the doc-test on `Issuer`.

- **One body, run twice.** The eight tests of `renewal/tests.rs` and the eight of
  `renewal/tests/service_tests.rs` are generic functions over the issuer in
  `renewal_twin/tests.rs` and `renewal_twin/tests/service.rs`. `instantiate!` (one in each of
  those files) names each as a `#[test]` in the issuer's own module, so
  `renewal::tests::<name>` and `house_renewal::tests::<name>` are the paths they always were. A
  body left out of the list is dead code, which `-D warnings` refuses.
- **The rig.** `Rig: Issuer` is the test-side trait, in `renewal_twin/tests.rs`: the birth seed
  and the Follow birth seed, the registry a birth leaves (`inherit`), the declaration
  (`declare`), the issuer's own fields of a Draft (`disclose`: seller, lineage, parents and
  royalty chain, against issuer, organisation, scope and the birth's quote), and the two
  `PrepareCtx` calls. `Fixture<I>` carries the issuer, so a call on a fixture needs no
  annotation. The seeds the paid-resale tests take (`resale_term_seed`, `closed_holder_seed`,
  `continue_holder_seed`, `refuse_holder_seed`) are forwarded in each issuer's module under their
  old paths; `paid_resale/tests.rs` and `tests/index_rows.rs` are unedited.
- **The CT.0 walk** was a macro in `tests/ct0_kit.rs` expanded in each twin's module. It is now
  one generic function, `renewal_twin/tests/ct0_walk.rs`, called by the two tests under their old
  names; the macro is gone from the kit. Both files are byte-identical (below).
- **Added, ten names:**
  - `renewal::tests::personal_identity_strings_hold_todays_bytes` and
    `house_renewal::tests::house_identity_strings_hold_todays_bytes`: every associated constant
    against a literal. The old rigs spelled the digest domains and the birth family out
    themselves, which made each test a second, independent copy of the strings; the shared rig
    reads the constants, so these two tests are where that second copy now lives.
  - `renewal_twin::tests::the_issuer_seal_admits_exactly_two` (below).
  - Three bodies run for both issuers, six names, each asked for by a mutation that passed
    (below): `a_declaration_without_a_closed_birth_audit_is_refused`,
    `a_granted_offer_is_held_to_its_issuers_royalty_rule` and
    `a_follow_binding_is_held_to_the_acquisition_of_its_birth`, under `renewal::tests::` and
    under `house_renewal::tests::`.
  - `house_renewal::tests::house_birth_product_requires_the_allocation_and_its_declared_recipe`,
    House only, for the same reason.
- **The House gained no scenario the personal issuer had**: in the kernel the two test modules
  already held the same sixteen. One assertion the House's first test had (the personal protocol
  is declared in the House fixture) is now made for both issuers, where it is trivially true of
  the personal one.
- **Single-issuer tests, and why each is single:**
  - `house_renewal::tests::house_birth_and_scope_are_required_through_later_term_recovery`: it
    corrupts the House allocation's organisation, scope and protocol; a personal licence has none.
  - `house_renewal::tests::house_binding_requires_exact_org_scope_quote_and_no_personal_subject`:
    it calls `require_house_birth`, the House's birth route, directly.
  - `house_renewal::tests::personal_profile_rejects_house_aliases_and_resolves_each_real_owner`:
    only the House fixture declares both protocols in one registry.
  - `house_renewal::tests::house_birth_product_requires_the_allocation_and_its_declared_recipe`:
    `birth_product` is the House's hook; the personal issuer's is empty.
  - `renewal::tests::service_tests::ct0_snapshot::ct0_capacity_wire`: CT.0's pin of the capacity
    metadata, taken from a personal Draft; it is not a twin test.
  - The owner-index rig `renewal::tests::renewal_grant` and `coexistence_registry_seed` are
    helpers other families call for the personal issuer only, and stay in `renewal/tests.rs`.

The test files went from 2,808 lines and the walk's 470 to 2,800, the ten new tests in them.

**The snapshots.** `git diff --stat 003de2253 -- '*.snap'` is empty. The kernel's seven `ct0_*`
tests pass on Windows and on Linux and the other crates' 27 on Linux, and `renewal_personal.snap`
and `renewal_house.snap` were never re-blessed.

**The seal.** `Issuer`'s supertrait is `sealed::Sealed`, a public trait in a module private to
`renewal_twin/issuer.rs`, implemented there for `Personal` and `House`.

- *From outside the crate* the trait cannot be named. The `compile_fail` example on `Issuer`'s
  documentation is that attempt, and runs with the crate's doc-tests.
- *From inside the crate* a doc-test cannot reach a private trait, so the three attempts were
  made by hand, each alone, in `house.rs` on the throwaway clone, with `cargo check -p mush-core`:

  | Attempt | The compiler's answer |
  |---|---|
  | `struct Guild; impl renewal_twin::Issuer for Guild {}` | `error[E0277]: the trait bound `Guild: sealed::Sealed` is not satisfied` (beside E0046 for the items the attempt leaves out) |
  | `impl renewal_twin::issuer::sealed::Sealed for Guild {}` | `error[E0603]: module `issuer` is private` |
  | a second `impl renewal_twin::Issuer for renewal_twin::Personal {}` | `error[E0119]: conflicting implementations of trait `Issuer` for type `Personal`` |

- `the_issuer_seal_admits_exactly_two` is the tripwire for a later edit that widens the seal
  instead of going round it: over the layer's source it finds exactly two `impl Sealed for`, both
  in `issuer.rs`; exactly two `impl Issuer for`, one in each issuer's file; one `mod sealed {`
  with no `pub`; and no `dyn` beside the trait's name. It is a census of the source, not a proof;
  the compiler's answers above are the proof.

**The mutations.** Each was made alone on a throwaway clone on conclav.dev (CT.0's procedure:
the worktree is never edited, so nothing is reverted in it), the layer's tests run
(`cargo test -p mush-core --lib terminal_capacity`: 235 at the first pass, 242 once the tests
below were added), the file put back.

| # | The mutation | What failed | A `ct0_` test among them |
|---|---|---|---|
| I1 | a family id, personal: `Personal::ROLL` given another letter | 7 tests: both identity tests (the House reads it as its `OTHER_ISSUER`), the personal walk, `ct0_capacity_wire`, three paid-resale tests | yes, two |
| I2 | a family id, House: `House::HEAD` given another letter | 5 tests: the House identity test, the House walk, three paid-resale tests | yes |
| I3 | an action id, House: `fund_house_mind_renewal2` | the House identity test and the House walk | yes |
| I4 | an action id, personal: the service close | the personal identity test and the personal walk | yes |
| I5 | a digest domain, personal: the offer's, `.v2` | the personal identity test, the personal walk, `ct0_capacity_wire` | yes, two |
| I6 | a digest domain, House: the capacity fingerprint's, `.v2` | the House identity test and the House walk | yes |
| I7 | the `protocol` name of the House's action payloads, `_v2` | the same two | yes |
| I8 | personal's `OTHER_ISSUER` no longer names the House's two families | the personal identity test and `personal_profile_rejects_house_aliases_and_resolves_each_real_owner` | no |
| B1 | a prerequisite audit skipped: personal no longer asks for the entitlement purchase audit | **nothing, at first.** Now `renewal::tests::a_declaration_without_a_closed_birth_audit_is_refused` | no |
| B2 | the same for the House: its one audit is not asked for | **nothing, at first.** Now the same test under `house_renewal::tests` | no |
| B3 | the inverted check uninverted: the House accepts only an offer that has a royalty chain | 27 tests: every House test that drafts, the House walk among them | yes |
| B4 | the inverted check dropped: the House no longer refuses a royalty chain | `house_binding_requires_exact_org_scope_quote_and_no_personal_subject` | no |
| B5 | the personal half dropped: `royalty_chain_differs` answers `false` | **nothing, at first.** Now `renewal::tests::a_granted_offer_is_held_to_its_issuers_royalty_rule` | no |
| B6 | the House birth route no longer run by the validator: `birth_route` answers `Ok` | **nothing, at first**: its two House tests call `require_house_birth` directly, and the allocations they corrupt are refused by the purchase's own audit. Now the House's `a_granted_offer_is_held_to_its_issuers_royalty_rule` and `a_follow_binding_is_held_to_the_acquisition_of_its_birth` | no |
| B7 | the House recipe check: `require_product`'s refusal ignored | **nothing, at first.** Now `house_birth_product_requires_the_allocation_and_its_declared_recipe` | no |
| B7b | the House recipe hook emptied: `birth_product` answers `Ok` | the same test | no |
| B8 | the personal Follow birth check dropped: `follow_birth` answers `Ok` | **nothing, at first.** Now `renewal::tests::a_follow_binding_is_held_to_the_acquisition_of_its_birth` | no |
| B9 | the economics family swapped: personal digests a term's economics in the House's domains | 26 tests, both personal `ct0_` tests among them | yes, two |
| B10 | the same the other way: the House digests its grant in the personal domains | 3 tests: the House walk, the House's Follow test and a paid-resale Follow test | yes |
| B11 | the House's `grant_version` off by one | 5 tests, the House walk among them | yes |
| B12 | the validator no longer calls `birth_product` at a Draft, Fund or Grant | **nothing, and no test was added** (below) | no |
| P1 | serde no longer skips the type parameter: `"issuer":null` in the protocol's JSON | 4 tests: both walks and both `old_protocol_encoding_and_five_actions_keep_their_exact_identity` | yes, two |

Against the brief's eight: a family id (I1, I2), an action id (I3, I4) and a digest domain (I5, I6)
each fail an identity test and a `ct0_` test; the inverted check uninverted (B3) fails 27 tests;
a prerequisite audit skipped (B1, B2) failed nothing and has a test now. The payee rule, the
buyer-admission check and the `OwnCharter` stop are not in the kernel; B6 and B7 are the nearest
the kernel has to a House admission and to its template acceptance.

- **Seven mutations passed at first** (B1, B2, B5, B6, B7, B8, B12): each drops a birth check
  that no test of the layer missed. They are older than this unit. The six that can be made in
  the old files were made on main's own two (a third clone at `003de2253`, never synced), and
  each passes main's 232 tests of the layer. The twin did not open these gaps; it made them
  visible, because every birth check is now a named function of a trait and "what if this one
  answers `Ok`" is one line to try.
- **Six are closed** by the seven tests named under Tests, and each of those tests was then
  shown to fail under its mutation (the table's "Now"). The two cold-boot tests tamper a granted
  operation's two paired images consistently (the offer rewritten and its digest recomputed in
  both; the binding's `filed` changed in both), so that the only rule left to refuse the record
  is the issuer's own. Each has a control: the royalty test first rewrites a field the kernel
  does not read and boots.
- **B12 is not closed.** For every birth the two seeds can make, the validator's call to
  `birth_product` refuses nothing that `birth_route`, which runs just before it, has not already
  refused: a first-recipe House birth always has its recipe declared, and a Follow binding's
  recipe is checked by the birth route itself. The call only decides something for a two-seat
  **Pin** birth under a registry that no longer declares the two-seat recipe, and the one rig
  that can make a two-seat birth is private to `house/tests.rs`, which this unit may not edit.
  The hook's own body is pinned by a direct test (B7, B7b). Owed to CT.9, which owns that file:
  a `two_seat_renewal_birth_seed`, and the test that a Pin renewal of it is refused at Draft,
  Fund and Grant, and still refunded, once the recipe is gone.
- **What the shared rig costs.** The rig now reads the trait, so a swapped digest domain is
  consistent between the rig and the validator and an ordinary test does not see it. B10 shows
  it: three tests fail, where B9 (whose swap also breaks a format check) fails 26. The CT.0
  snapshot is what catches a domain that moved, and the two identity tests are the second catch
  for the constants. CT.4 inherits this.

**Divergences.**

- **The trait is narrower than §5.1's sketch.** No `Terms` type, no `authority`, no `payees`, no
  `stop`: none of those is in the kernel (the survey). The sketch is the shape of economy's twin.
- **Three modules, not one.** §5.1 says the family "becomes one module". Here it is one
  implementation module and two issuer modules, because the purchase files (CT.9's, not to be
  edited) and every test path name `renewal::` and `house_renewal::`. The issuer modules hold only
  an `impl` and the entry points.
- **Three of the brief's eight mutations have nothing to mutate in the kernel**: the payee rule,
  the House's buyer-admission check and the player's `OwnCharter` stop. The nearest things the
  kernel has were mutated in their place (B6, B7) and are named as such in the table.
- **`match` became `contains`.** An associated constant of a type parameter cannot be a pattern,
  so the six places that matched on a family name (`HEAD | LICENCE => …`) test membership
  (`[I::HEAD, LICENCE].contains(&family)`). The arms and their order are the same.
- **One `#[expect(private_bounds)]`**, on `impl<I: Issuer> Protocol<I>`. `action_state.rs` holds
  the two protocols and calls their constructors, so the type is visible there; the trait is
  visible one level further in, on purpose, so that only this layer can write code generic over
  the issuer. The alternative was to widen `View`, a private type of the coordinator, for a lint.
- **The entry points are a macro**, not sixteen hand-written forwarders: the two expansions are
  the two instantiations, and the forwarders would have been a new clone.
- **The seal's in-crate proof is by hand** (above): a doc-test is a separate crate.

**Not built.**

- Anything outside the kernel. `mush-economy`, `mush-roll`, `mush-market`, `mush-industry` and
  `tacsim` compile and pass unedited.
- The kernel's per-issuer lists (§4.6): the registry's two fields and four builder methods,
  `module.rs`'s four and `resolution.rs`'s four public methods, and the pairs of calls in
  `terminal_capacity.rs`, `entitlement.rs`, `house.rs`, `paid.rs` and `paid_resale/`. Deliberate.
- The purchase twins (`entitlement`, `house`: CT.9) and the refit pair (CT.7).
- The mirror of the alias test: that the House recipe refuses an operation the personal issuer
  owns is checked by the House birth route's own test and by every House walk, not by a test of
  its own.
- What CT.0's walks do not reach is still not reached (the transferred-holder attempts on the
  real path; the refusal sentences no walk provokes).

**Compile time** (§7.4). The owner's Windows box, `-j 2`, `CARGO_INCREMENTAL=0`, `lib.rs`
touched before each run, cargo's own `Finished` figure; five runs of the library and three of the
test targets, each after one run that is not counted.

| | Before | After |
|---|---|---|
| `cargo check -p mush-core` | 8.29, 8.35, 8.38, 8.41, 8.47 s: median 8.38 s | 7.96, 8.09, 8.17, 8.17, 8.19 s: median 8.17 s |
| `cargo check -p mush-core --lib --tests` | 21.30, 21.53, 21.77 s: median 21.53 s | 21.04, 21.45, 21.82 s: median 21.45 s |

The library's check is 0.2 s shorter (one validator to type-check, not two) and the test targets'
is flat, with seven tests more in them. The dependants' check time was not measured: no generic
item crosses the crate boundary (the trait is private to the layer and the entry points are
ordinary functions), so a dependant reads what it read before.

**On Linux** (conclav.dev, a throwaway clone of `003de2253` with this branch's files over it):
`cargo test -p mush-economy -p mush-roll -p mush-market -p mush-industry --no-fail-fast` passes
1,667 tests with none failed and one ignored (the four libraries' 279, 367, 136 and 73 among
them), no file of the four crates edited; `cargo test -p tacsim --lib ct0_` passes the
registration snapshot; `cargo test -p mush-core` passes 1,095, the library's 839 among them. No
`.snap` file differs after the runs. On Windows the kernel's tests pass, and
`cargo test -p mush-contract-tests` (the census, the panic budget, the restriction lints), clippy
with `-D warnings` over all targets, CI's M7 memory-size pass and the docs check with
`-D warnings` are clean.

**The yardstick, before and after.** Before is main `003de2253`.

| Root | Before: cloned of lines | Rate | After: cloned of lines | Rate |
|---|---|---|---|---|
| `crates/mush-market/src` | 10,632 of 45,188 | 23.5% | 10,632 of 45,188 | 23.5% |
| `crates/mush-economy/src` | 20,969 of 99,977 | 21.0% | 20,969 of 99,977 | 21.0% |
| `crates/mush-roll/src/commerce` | 16,820 of 91,718 | 18.3% | 16,820 of 91,718 | 18.3% |
| `crates/mush-core/src/action_state/terminal_capacity` | 11,605 of 40,352 | 28.8% | 6,610 of 38,725 | 17.1% |

The kernel layer lost 4,995 cloned lines and is at §7.3's estimate ("near 17%"). With `--stems`,
`renewal` and `house_renewal` were 2,633 of 2,864 and 2,635 of 2,926; `renewal_twin` is 252 of
3,820, and the two issuer stems are below the table's last row. By `wc -l` the layer (its files
and `terminal_capacity.rs`) went from 41,751 lines to 40,171. What is left at the top of the
kernel's table is the purchase twins (`entitlement` 1,436, `house` 1,415), which is CT.9.

**Found on the way.**

- **Six birth checks had no test** (the mutations, above). Closed here; a seventh (B12) is owed
  to CT.9 with its recipe.
- **The two files differed by nothing the survey did not list.** Every line outside the 27
  hunks was the same text, refusals included, so the House validator had been answering
  "renewal …" sentences all along and still does.
- **The licence and term families are shared.** A House licence renews into
  `roll_mind_personal_paid_licence` and `roll_mind_paid_term`, the personal names; only the
  term-economics family differs. They are plain constants of the module, not the trait's.
- **A second read, not anchored on this one.** A reviewer given the two old files and the four
  new ones, and told to substitute each issuer into the twin and explain every hunk that is left,
  found no behavioural difference, no change in the order of evaluation, and the same reachable
  refusal sentences for each issuer (125 personal, 134 House). One side effect it named:
  `Protocol`'s derived `Debug` output now shows the type parameter. The registry's own `Debug` is
  hand-written and prints no protocol, so nothing reads it.
- No defect in production code was found, and no production behaviour was changed.

**For CT.4.**

- *Copy:* the survey first (the diff sorted into rename, identity string and real logic, with
  counts) and the trait written from it, not from §5.1's sketch; the issuer types and the seal in
  one file and each `impl` in its issuer's own file; the old modules kept as the two
  instantiation points, so that no caller and no test path moves; the entry points by one macro;
  `Rig: Issuer` and `Fixture<I>` for the tests, with `instantiate!` listing each body once; an
  identity test an issuer that holds every constant to a literal; the mutations on a throwaway
  clone, one a trait function and one a constant kind.
- *Keep each hook where its check was.* Where the two files made the "same" check at different
  places, two hooks keep both orders; one hook moves a refusal.
- *Patterns.* A `match` on a family constant will not compile once the constant is the trait's;
  test membership and keep the arms' order.
- *Do differently:* economy's twin is where §4.3's other five differences live (the terms and
  their payout, the current-authority check, the payee, the admission policy, the `OwnCharter`
  stop, the account pairing), so its trait will be wider than this one and §5.1's `Terms`
  associated type will be needed there. Economy's consent hash and plan hash are taken over the
  typed value in field order (CT.0's finding), so a record that becomes generic must keep every
  field where it is, and a `PhantomData` field must be skipped by serde as it is here (P1 is
  what happens when it is not).
- *Run the hook mutations early.* "This function of the trait answers `Ok`" is the cheapest
  mutation there is once the twin is cut, and here it found six untested checks. Economy's House
  twin had no test at all before CT.0, so expect more.
- Economy cannot name the kernel's `Issuer` (it is private to `terminal_capacity`, by design), so
  economy's two implementations each call their own `mind_renewal_*` or `house_mind_renewal_*`
  method of the kernel's builder and of `PrepareCtx`, as the kernel's test rig does in `Rig`.

## CT.4

The renewal twin in economy. Built 2026-11-14 on main `26d30dd4b` (CT.0 to CT.3 in).

**The survey, before the cut.** `managed_renewal.rs` and its nine files (4,480 lines) against
`managed_house_renewal.rs` and its nine (4,471): `diff` gives 96 hunks, 257 changed lines on the
personal side and 248 on the House's, and everything else is the same text. File by file, and
sorted:

| File | Lines, personal and House | Hunks | (a) a rename | (b) an identity string | (c) real logic | (d) tests only |
|---|---|---|---|---|---|---|
| `managed_renewal.rs` | 459, 461 | 10 | 4 | 2 | 2 | 2 |
| `audit.rs` | 147, 147 | 9 | 9 | | | |
| `bridge.rs` | 253, 257 | 12 | 10 | 2 | | |
| `funding.rs` | 434, 434 | 13 | 13 | | | |
| `history.rs` | 630, 611 | 5 | 1 | 1 | 2 | 1 |
| `install.rs` | 526, 535 | 18 | 17 | | 1 | |
| `money_install.rs` | 557, 550 | 6 | 5 | | 1 | |
| `receipt.rs` | 981, 981 | 1 | | 1 | | |
| `service.rs` | 195, 195 | 1 | | 1 | | |
| `subject.rs` | 298, 300 | 21 | 3 | 1 | 17 | |
| **All** | 4,480, 4,471 | 96 | 62 | 8 | 23 | 3 |

- *(a) A rename, 62 hunks.* Which of the store's two fields (`renewal_offers` against
  `house_renewal_offers`, on 27 lines), which of the fold's (`renewal_receipts`, on 22), the boot latch,
  the managed consent's variant read and built, the engine's plan guard and the closed plan's
  name, and in the House file a local `issuer` that is the buyer under another name.
- *(b) An identity string, 8 hunks, 14 strings:* five action ids and the service's, the journal
  namespace, the two key prefixes, the two `protocol` names of the action payloads, the offer's
  digest domain, the stored contract's name and the funding plan's description.
- *(c) Real logic, 23 hunks.* Outside `subject.rs` there are six, and they are four differences:
  the scope a contract is filed under (none, or the organisation's), the titles an offer and a
  sale fence (the charter and each paid royalty parent's, or the charter alone; two sites), what
  a retry must keep of the offer it follows (the lineage, or the organisation and the scope),
  and the purchase family a transferred holder's licence was born in; the sixth hunk is the
  personal file's export of `RenewalLineageV1`. The other seventeen are `subject.rs`: the
  basis's one extra field, the terms' fields and the wire of the royalty parents, the payout
  computation, the payees, and the check of the issuer's current authority.
- *(d) Tests only, 3 hunks:* two `mod` lines, and two `#[cfg(test)]` constructors of the offer
  book the House file did not have.

Where each of doc 46 §4.3's nine sits in economy:

| §4.3 | In economy | Carried by |
|---|---|---|
| 1, the terms and their payout | `subject.rs`, and three of the four differences above | each family's own records; `Terms::payouts` and `recipients`; `titles`, `same_line` |
| 2, the current-authority check | `subject.rs` `check_current` | `Terms::check_current` |
| 3, the payee | `subject.rs` (the remainder's owner and, for the House, its treasury's scope) and the contract's scope | `Terms::payouts`, `Terms::check_current`, `contract_scope` |
| 4, the birth variant | one constant; the inverted audit check is the kernel's | `BIRTH_ROUTE` |
| 5, buyer admission and template acceptance | not in economy (the market's `config.rs` and `offer.rs`) | — |
| 6, the `OwnCharter` stop | not in economy (the Roll's `reader.rs`, the market's `player.rs`) | — |
| 7, the account pairing | the payout computation: no payee is the payer's account, and one owner an account | `Terms::payouts` |
| 8, the prerequisite audits | the kernel's. Economy's own prerequisite, that a family's renewals need its birth's purchases, is two hand-written checks in `module.rs` and stays so | — |
| 9, the identity strings | the 14 strings of (b) | 14 associated constants |

So economy's surface is, as the kernel's was, narrower than §4.3's nine: six of them are in the
twin, a seventh (the prerequisites) is two hand-written checks outside it, and two of the
brief's mutations (the House's buyer admission, the `OwnCharter` stop) have nothing here to
mutate. Both files hold the same ten sentences: one refusal ("The renewal offer is
unavailable or has changed."), the composition's refusal, two audit errors, four `serde`
messages and the two memo lines of a grant's journal entry. The personal file alone has the two
messages about royalty parents. Every string
literal of the twenty production files was listed before and after: the same 80, but for one
format string (`"{PREFIX}{}"` is `"{}{}"` with the prefix as an argument) and the empty string
of `#[serde(bound = "")]`.

**Built.** One owner, two instantiations, in `crates/mush-economy/src/`:

| File | Lines | What it is |
|---|---|---|
| `renewal_twin.rs` | 790 | the marker, the attempt, the output, the seven `prepare_*` functions and the contract, generic over the issuer; `entry_points!` and `subject_records!` |
| `renewal_twin/issuer.rs` | 158 | the sealed trait `Issuer`, the issuer types `Personal` and `House`, and the three record traits `Basis`, `Terms` and `Offer` |
| `renewal_twin/{audit, bridge, funding, history, install, money_install, receipt, service}.rs` | 3,751 | the eight files of the old personal family, generic over the issuer |
| `renewal_twin/wire.rs` | 39 | the decode helpers both `subject.rs` files had word for word |
| `managed_renewal.rs`, `managed_house_renewal.rs` | 115, 109 | `impl Issuer for Personal` and `for House`, and the family's face (`entry_points!`) |
| `managed_renewal/subject.rs`, `managed_house_renewal/subject.rs` | 204, 205 | each family's own three `checked!` records, its payout computation and its authority check |

The twenty production files were 8,951 lines; the fifteen are 5,371.

*The trait as built.* `Issuer: sealed::Sealed + Copy + Debug + Default + Eq + 'static`, in a
module private to the crate. Fifteen associated constants, three associated types and eleven
functions:

| Item | What it carries | §4.3 |
|---|---|---|
| `DRAFT_ACTION`, `CLOSE_ACTION`, `FUND_ACTION`, `GRANT_ACTION`, `REFUND_ACTION`, `SERVICE_CLOSE_ACTION` | the six action ids | 9 |
| `NAMESPACE` | the idempotency and cause namespace of the family's journal entries, and its holds' | 9 |
| `OFFER_PREFIX`, `SERVICE_PREFIX` | the key prefixes of the offer record and the service closure | 9 |
| `PROTOCOL`, `SERVICE_PROTOCOL` | the `protocol` name the composition checks in the action payloads | 9 |
| `OFFER_DOMAIN` | the digest domain of an offer | 9 |
| `CONTRACT_NAME`, `FUNDING_DESCRIPTION` | the stored contract's name; the funding plan's description, which is under the plan hash | 9 |
| `BIRTH_ROUTE` | the purchase family a renewed licence was born in | 4 |
| `Basis`, `Terms`, `Offer` | the family's own three records (below) | 1 |
| `book`, `bind`, `receipts`, `receipts_mut`, `inventory_checked` | which of the store's and the fold's fields are this issuer's. The fields stay two hand-written fields each; this is the rename (a) | (a) |
| `marker`, `consent` | this issuer's variant of the managed consent, read and built | (a) |
| `validate` | the engine's validation under this issuer's plan guard | (a) |
| `contract_scope(terms)` | the scope the contract is filed under | 3 |
| `titles(terms)` | the titles an offer and a sale stand on | 1 |
| `same_line(row, before)` | what a retry keeps of the offer it follows | 1 |

The record traits are what the owner reads of an issuer's records: `Basis` (its shape check and
four fields), `Offer` (its basis, its terms, `validate` and `digest`) and `Terms` (eight fields,
and the three functions that are the issuer's own rules: `payouts`, `recipients` and
`check_current`, §4.3's 1, 2, 3 and 7). Every constant holds the value its file held, byte for
byte; two tests hold each to a literal and the snapshot holds the bytes they produce.

*What stayed as two implementations, and why.*

- **The two `impl Issuer` blocks.** Eight of the eleven functions are one line each: a field,
  a variant or a guard of a closed list that stays two hand-written entries (§4.6).
- **`subject.rs`, as doc 46 §5.1 expected.** The three records are different shapes (the
  personal basis has the royalty chain's digest; the terms share twelve fields of fifteen and
  fourteen), the personal payout is the entitlement purchase's `EconomicRecipe` and the
  House's is its own, and the authority check reads a player's titles in one and the
  organisation store in the other. What the two files had word for word (the basis's shape
  check, the terms' window check, the offer's `decode`, `validate` and `digest`, the wire
  helpers and the player check) is written once: `subject_records!` and `wire.rs`.
- **No record went through a new arm of `checked!`.** The owner reads an offer through
  `Issuer::Offer`, an associated type, and each family keeps its own `RenewalBasisV1`,
  `RenewalOfferTermsV1` and `RenewalOffer` under the one macro, with the calls as CT.1 left
  them. The alternative was one `RenewalOffer<I>`, which needs a generic arm in `mush-core`
  for a record of three fields whose other two are different shapes anyway. `mush-core` is not
  edited and CT.1's mutations did not need re-running. What it costs: the owner reads
  `offer.terms().buyer()` where it read `offer.terms.buyer`, and a call whose only
  issuer-typed argument is an offer cannot infer the issuer and names it
  (`install::offer::<I>(…)`).
- **The other records are generic**, with every field where it was. Six hold an issuer-typed
  field and need no marker (`RenewalMarkerV1<I>` holds `I::Basis`; `OfferRecord<I>` holds
  `I::Offer`; `Proof`, the two histories and the receipt audit hold those). Three hold nothing
  of the issuer's but are validated against its action ids when decoded, so they carry a
  `PhantomData<I>`: the funding receipt, the terminal receipt and the service closure. The
  marker is `#[serde(skip)]` on the encode and their decode is the hand-written pair it always
  was, through the same `FundingWire`, `TerminalWire` and `ClosureWire`, which are not generic.
  Mutations F4 and F5 are what happens when the skip is not there.
- **Seven plain names are one type under both paths**: `RenewalAttemptV1`,
  `UnfundedCloseReason`, `RenewalOutcome`, `RenewalServiceCloseReason`, `RenewalAuditError`,
  `RenewalMoneyRequest` and `RenewalClosureView`. They are vocabulary with public fields or
  variants, nothing in them depends on the issuer, and the money request is built by struct
  literal in the market, so it could not take a marker. Everything that is a proof is still a
  type an issuer: the outputs, the receipts, the histories, the marker, the book, the receipt
  audit, the patches, the closed plan, the composition's configuration and the audit's views.
  Mutations X4 to X7 are the compiler refusing one issuer the other's.

*The entry points.* The rest of Economy, and the Roll and the market, name the two families by
module and by hand, as before. `entry_points!` expands in each family's file to what its callers
name: the aliases of the generic types at the family's issuer, the shared names, the identity
constants under their old names, seven `prepare_*` forwarders, and the seven small modules
(`audit`, `bridge`, `history`, `install`, `money_install`, `receipt`, `service`) with the
aliases and the five forwarders `boot.rs`, `module.rs`, `fold.rs` and `registry_filing.rs` read.
The engine's two names for the closed plan, `ClosedRenewalPlan` and `ClosedHouseRenewalPlan`,
are `ClosedPlan<Personal>` and `ClosedPlan<House>`.

*What did not move.* In `mush-economy`, outside the two families' files and the new module,
two files are edited: `lib.rs` (one `mod` line) and `module.rs` (two lines: the composition's
`Configuration` literal names its marker). `registry_filing.rs`, `registry_filing/prepare.rs`,
`boot.rs`, `engine.rs`, `fold.rs`, `sweep.rs` and `contract/managed.rs` have no diff: the
closed lists (`EconomyRequest`, `EconomyOutput`, the plan guard, the managed-consent wire) are
as they were. No file outside `mush-economy` is edited. No `dyn`, no function pointer and no
callback was added.

**Tests.** `cargo test -p mush-economy -- --list`, ignored tests included (there are none),
before and after:

| | Before | After |
|---|---|---|
| All targets (library, integration tests, doc-tests) | 809 | 881 |
| The library's tests | 279 | 350 |
| Under `managed_renewal::` | 52 | 61 |
| Under `managed_house_renewal::` | 1 | 62 |
| Under `renewal_twin::` | 0 | 1 |

No name disappeared and none changed: the diff of the two sorted lists is 72 additions.

- **One body, run twice.** The personal family's 51 scenarios are functions generic over the
  issuer under `renewal_twin/`: nine in `tests.rs`, fifteen in `tests/funded.rs`, six, seven and
  six in `tests/funded/{bridge, reoffer, service}.rs`, one in `tests/output.rs` and seven in
  `bridge_registration_tests.rs`. `instantiate!` (one in `tests.rs`, one in
  `bridge_registration_tests.rs`) names each as a `#[test]` in the family's own module and
  rebuilds the personal family's module tree, so `managed_renewal::tests::funded::reoffer::<name>`
  and the rest are the paths they always were, and `managed_house_renewal::tests::…` and
  `managed_house_renewal::bridge_registration_tests::…` are the same 51 names, new. A body left
  out of the list is dead code, which `-D warnings` refuses.
- **The rig.** `Rig: Issuer` is the test-side trait (`renewal_twin/tests.rs`): the seed (a
  world at 1000 with a buyer, the issuer's side and a charter, three zero-balance accounts and
  an offer of 100 over 100 IC seconds), `widen` (every payee role the issuer has: six for the
  personal issuer, three for the House), a mutable view of the fields both issuers' offers
  have (`terms`, `basis`: the owner only reads an offer, a test rewrites one), `lapse` (the
  seller leaves, or the organisation's charter does), `stranger`, `other_line`, `widest` and
  `own_wire_is_strict`. `Fixture<I>` carries the issuer. `Composition: Issuer`
  (`bridge_registration_tests.rs`) is the birth a family's registration stands on: six
  entitlement actions, or the House filing and three House purchase actions. Each family's
  `impl` is in its own test module; the House's is CT.0's walk fixture, moved.
- **The CT.0 walk** was one file `include!`d by both families. It is one generic function,
  `renewal_twin/tests/ct0_walk.rs`, called by the two snapshot tests under their old names.
- **What the bodies no longer assume.** Where a scenario counted the personal issuer's six
  payees or its seven journal lines it asks the rig (`ROLES`); where it reached for the seller
  it asks for a `stranger` or for the issuer's `lapse`; the strict-wire scenario's royalty-parent
  cases are the personal rig's `own_wire_is_strict`, and the House's refuses an offer without
  its organisation or its scope.
- **Added, 21 names.**
  - `managed_renewal::tests::personal_identity_strings_hold_todays_bytes` and
    `managed_house_renewal::tests::house_identity_strings_hold_todays_bytes`: every associated
    constant against a literal, and the family module's old names against the constants.
  - `renewal_twin::tests::the_issuer_seal_admits_exactly_two`, and the `compile_fail` doc-test
    on `Issuer` (below).
  - Six bodies run for both issuers, twelve names, each asked for by a mutation that passed
    (below): `every_title_an_offer_stands_on_is_current_and_fenced`,
    `an_offer_is_refused_once_the_issuers_side_lapses` and
    `a_payee_cannot_be_the_payers_own_account` under `tests::`;
    `every_title_a_sale_stands_on_is_current_and_fenced` and
    `a_sale_is_refused_once_the_issuers_side_lapses_and_a_refund_is_not` under `tests::funded::`;
    `a_retry_on_another_line_of_the_issuer_is_refused` under `tests::funded::reoffer::`.
  - Five single-issuer tests, for the same reason (next).
- **Single-issuer tests, and why each is single:**
  - `managed_house_renewal::tests::a_house_offer_needs_its_organisation_scope_and_title_current`:
    a suspended organisation, a retired scope, another organisation's scope, a title on
    another scope and a title somebody else holds. The personal issuer has no organisation;
    its counterpart is the titles scenario above, which runs for both.
  - `managed_house_renewal::tests::a_house_offer_pays_its_remainder_to_the_scoped_treasury`:
    the House's payee. The personal remainder is held to the seller by the entitlement
    purchase's recipe, which is not this unit's.
  - `managed_house_renewal::tests::a_house_contract_is_filed_under_the_issuers_scope` and
    `managed_renewal::tests::a_personal_contract_is_filed_under_no_scope`: one hook, two
    answers, each against a literal. A shared body would read the hook it is testing.
  - `managed_renewal::tests::a_personal_offer_refuses_a_title_on_a_scope`: a personal title
    carries no scope; a House title must.
  - The two identity tests and the two snapshot tests are an issuer's by construction.
  - The brief's two examples of a single-issuer scenario, the `OwnCharter` stop and the
    House's buyer admission, are not in economy (the survey).

The test files went from 4,933 lines to 5,849, the eleven new scenarios, the rig and the two
identity tests in them.

**What the House runs found.** `managed_house_renewal` had no test before CT.0's walk.

- **Every one of the 51 scenarios passes for the House on the first run**, once three
  assertions that counted the personal issuer's payees were asked of the rig. No scenario fails
  for the House on main's own code, and nothing was fixed in production.
- **What they did not find is the finding.** The 51 are the personal family's scenarios, so
  they exercise what the two families share. The House's own rules (its organisation, its
  scope, its scoped title, its treasury) had no test in the crate at all: ten mutations of
  them passed (S7 to S14, H5, H5b, below), and on main's own files the three of those that
  were tried pass as well. They are covered now by the House-only tests and by the scenarios
  that run for both.

**The snapshots.** `git diff --stat 26d30dd4b -- '*.snap'` is empty. Economy's seven `ct0_`
tests pass on Windows and on Linux, the other crates' on Linux, and `renewal_personal.snap` and
`renewal_house.snap` were never re-blessed. The snapshot stopped the unit twice on the way, both
times in the tests and neither in the owner: the scripted pass over the walk had rewritten the
label `"RenewalMarkerV1"` as `"RenewalMarkerV1<I>"` (sixteen lines of each file), and then
`"RenewalOffer"` as `"I::Offer"`. Both were undone.

**The seal.** `Issuer`'s supertrait is `sealed::Sealed`, a public trait in a module private to
`renewal_twin/issuer.rs`, implemented there for `Personal` and `House`.

- *From outside the crate* the trait cannot be named: the module is private. The `compile_fail`
  example on `Issuer`'s documentation is that attempt, and runs with the crate's doc-tests.
- *From inside the crate* the attempts were made by hand on the clone, each alone, in
  `managed_house_renewal.rs`, with `cargo check -p mush-economy` (X1 to X3 below).
- `the_issuer_seal_admits_exactly_two` is the census for a later edit that widens the seal
  instead of going round it: exactly two `impl Sealed for`, both in `issuer.rs`; exactly two
  `impl Issuer for`, one in each family's file; one `mod sealed {` with no `pub`; no `dyn`
  beside the trait's name. It is a census of the source, not a proof.

**The mutations.** Each was made alone on a throwaway clone on conclav.dev (the worktree is
never edited, so nothing is reverted in it), economy's library tests run
(`cargo test -p mush-economy --lib`: 333 at the first pass, 350 once the tests below were
added), the file put back. "P" is a test under `managed_renewal::`, "H" one under
`managed_house_renewal::`. The first pass is what the tests caught before any was added; the
second is the same list against the tree as built.

*The trait's functions, one issuer at a time, answering their neutral value.*

| # | The mutation | First pass | As built | A `ct0_` test |
|---|---|---|---|---|
| H1 | `House::contract_scope` answers `None` | the House snapshot alone | 2 tests (H): `a_house_contract_is_filed_under_the_issuers_scope` | yes |
| H2 | `Personal::titles` answers the charter alone | **nothing** | 2 tests (P): `every_title_an_offer_stands_on_is_current_and_fenced`, `every_title_a_sale_stands_on_is_current_and_fenced` | no |
| H3 | `House::titles` answers nothing | H `actual_before_images_and_identity_refuse_stale_bind…` | 3 tests (H): `every_title_an_offer_stands_on_is_current_and_fenced`, `every_title_a_sale_stands_on_is_current_and_fenced` | no |
| H4 | `Personal::same_line` answers `true` | **nothing** | 1 test (P): `a_retry_on_another_line_of_the_issuer_is_refused` | no |
| H5 | `House::same_line` answers `true` | **nothing** | 1 test (H): `a_retry_on_another_line_of_the_issuer_is_refused` | no |
| H5b | `House::same_line` no longer compares the scope | **nothing** | 1 test (H): `a_retry_on_another_line_of_the_issuer_is_refused` | no |
| H5c | `House::same_line` no longer compares the organisation | **nothing** | **nothing** | no |
| H6 | `Personal::inventory_checked` answers `true` | 3 tests: both personal absence tests, **and the House's default-boot test** (below) | 2 tests (P): `renewal_bridge_resolves_all_actions_to_the_one_actual_economy_slot_in_either_order`, `actual_default_boot_proves_absence_without_new_serialized_history` | no |
| H7 | `House::inventory_checked` answers `true` | H `renewal_bridge_resolves_all_actions…` | 2 tests (H): `renewal_bridge_resolves_all_actions_to_the_one_actual_economy_slot_in_either_order`, `actual_default_boot_proves_absence_without_new_serialized_history` | no |
| H8 | `Personal::marker` answers `None` | 23 personal tests, the snapshot among them | 25 tests (P): `actual_proof_and_contract_reconstruction_reject_each_missing_crossed_or_funded_shape`, `a_sale_is_refused_once_the_issuers_side_lapses_and_a_refund_is_not`, … | yes |
| H9 | `House::marker` answers `None` | 23 House tests, the snapshot among them | 25 tests (H): `actual_proof_and_contract_reconstruction_reject_each_missing_crossed_or_funded_shape`, `a_sale_is_refused_once_the_issuers_side_lapses_and_a_refund_is_not`, … | yes |
| X4 to X7 | the personal issuer handed the House's book, receipts, consent or plan guard (`book`, `receipts`, `consent`, `validate`) | does not compile: `error[E0308]: mismatched types`, four times | the same | — |

`bind` and `receipts_mut` have no neutral value and return the same fields as `book` and
`receipts`; the compiler refuses the swap as it does X4 and X6.

*The terms' own rules (`subject.rs`), §4.3's 1, 2, 3 and 7.*

| # | The mutation | First pass | As built |
|---|---|---|---|
| S1 | personal `check_current` answers `Ok` after the shape check | 2 personal tests | 7 tests (P): `a_personal_offer_refuses_a_title_on_a_scope`, `an_offer_is_refused_once_the_issuers_side_lapses`, … |
| S2 | House `check_current` answers `Ok` after the shape check | 2 House tests | 8 tests (H): `a_house_offer_needs_its_organisation_scope_and_title_current`, `a_house_offer_pays_its_remainder_to_the_scoped_treasury`, … |
| S3 | personal: the seller need not be a present player | **nothing** | **nothing** |
| S4 | personal: the seller need not hold the charter's title | 2 personal tests | 5 tests (P): `a_personal_offer_refuses_a_title_on_a_scope`, `every_title_an_offer_stands_on_is_current_and_fenced`, … |
| S5 | personal: a royalty parent's title is not checked | **nothing** | 2 tests (P): `every_title_an_offer_stands_on_is_current_and_fenced`, `every_title_a_sale_stands_on_is_current_and_fenced` |
| S6 | personal: a title on a scope is accepted as the seller's | **nothing** | 1 test (P): `a_personal_offer_refuses_a_title_on_a_scope` |
| S7 | House: the organisation need not be active | **nothing** | 1 test (H): `a_house_offer_needs_its_organisation_scope_and_title_current` |
| S8 | House: the scope need not be active | **nothing** | 1 test (H): `a_house_offer_needs_its_organisation_scope_and_title_current` |
| S8b | House: the scope need not be the organisation's | **nothing** | 1 test (H): `a_house_offer_needs_its_organisation_scope_and_title_current` |
| S9 | House: the organisation need not hold the charter's title | **nothing** | 1 test (H): `a_house_offer_needs_its_organisation_scope_and_title_current` |
| S10 | House: the title need not be on the offer's scope | **nothing** | 1 test (H): `a_house_offer_needs_its_organisation_scope_and_title_current` |
| S11 | the payee: the House remainder need not be the organisation's | **nothing** | 1 test (H): `a_house_offer_pays_its_remainder_to_the_scoped_treasury` |
| S12 | the payee: the House remainder account need not be the scoped treasury | **nothing** | 1 test (H): `a_house_offer_pays_its_remainder_to_the_scoped_treasury` |
| S13 | the account pairing: a House payee may be the payer's own account | **nothing** | 1 test (H): `a_payee_cannot_be_the_payers_own_account` |
| S14 | House: the charter may be leaving the game | **nothing** | 2 tests (H): `an_offer_is_refused_once_the_issuers_side_lapses`, `a_sale_is_refused_once_the_issuers_side_lapses_and_a_refund_is_not` |

*The identity constants, one of each kind.* Each fails its issuer's identity test and its
issuer's snapshot, and nothing else unless said: the shared rig reads the constants, so an
ordinary scenario moves with them (CT.3's warning).

| # | The mutation | What failed | A `ct0_` test |
|---|---|---|---|
| C1 | the namespace, personal: `.v2` | 2 tests (P): `ct0_renewal_personal_snapshot`, `personal_identity_strings_hold_todays_bytes` | yes |
| C2 | the namespace, House, given the personal value | 6 tests (2 P, 4 H): `renewal_six_payee_capture_and_refund_have_exact_real_journal_provenance`, `actual_refund_then_retry_grant_retains_both_journal_histories_and_old_closed_proof` | yes |
| C3 | an action id, House: `fund_house_mind_renewal2` | 2 tests (H): `ct0_renewal_house_snapshot`, `house_identity_strings_hold_todays_bytes` | yes |
| C4 | an action id, personal: the service close | 2 tests (P): `ct0_renewal_personal_snapshot`, `personal_identity_strings_hold_todays_bytes` | yes |
| C5 | a digest domain, personal: the offer's, `.v2` | 2 tests (P): `ct0_renewal_personal_snapshot`, `personal_identity_strings_hold_todays_bytes` | yes |
| C6 | a digest domain, House: the offer's, `.v2` | 2 tests (H): `ct0_renewal_house_snapshot`, `house_identity_strings_hold_todays_bytes` | yes |
| C7 | a key prefix, House: the offer record's | 2 tests (H): `ct0_renewal_house_snapshot`, `house_identity_strings_hold_todays_bytes` | yes |
| C8 | a key prefix, personal: the service closure's | 2 tests (P): `ct0_renewal_personal_snapshot`, `personal_identity_strings_hold_todays_bytes` | yes |
| C9 | the `protocol` name, House actions | 2 tests (H): `ct0_renewal_house_snapshot`, `house_identity_strings_hold_todays_bytes` | yes |
| C10 | the `protocol` name, personal service action | 2 tests (P): `ct0_renewal_personal_snapshot`, `personal_identity_strings_hold_todays_bytes` | yes |
| C11 | the contract's name, House | 2 tests (H): `ct0_renewal_house_snapshot`, `house_identity_strings_hold_todays_bytes` | yes |
| C12 | the funding plan's description, personal | 2 tests (P): `ct0_renewal_personal_snapshot`, `personal_identity_strings_hold_todays_bytes` | yes |
| C13 | the birth route, personal given the House's | 1 test (P): `personal_identity_strings_hold_todays_bytes` | no |
| C14 | the birth route, House given the personal one | 1 test (H): `house_identity_strings_hold_todays_bytes` | no |

*The hooks' call sites in the one implementation, and the records.*

| # | The mutation | First pass | As built | A `ct0_` test |
|---|---|---|---|---|
| G1 | `require_link` no longer asks the issuer's line | **nothing** | 2 tests (1 P, 1 H): `a_retry_on_another_line_of_the_issuer_is_refused` | no |
| G2 | an offer fences the charter's title alone | **nothing** | 1 test (P): `every_title_an_offer_stands_on_is_current_and_fenced` | no |
| G3 | a sale fences the charter's title alone | **nothing** | 1 test (P): `every_title_a_sale_stands_on_is_current_and_fenced` | no |
| G4 | the contract is filed under no scope | the House snapshot alone | 2 tests (H): `a_house_contract_is_filed_under_the_issuers_scope` | yes |
| G5 | the holder basis always asks for an entitlement birth | **nothing** | **nothing** | no |
| G6 | the absence proof no longer needs the boot latch | 4 tests, two an issuer | 4 tests (2 P, 2 H): `renewal_bridge_resolves_all_actions_to_the_one_actual_economy_slot_in_either_order`, `actual_default_boot_proves_absence_without_new_serialized_history` | no |
| G7 | an offer no longer checks the issuer's current authority | 2 tests, one an issuer | 9 tests (4 P, 5 H): `a_house_offer_needs_its_organisation_scope_and_title_current`, `a_house_offer_pays_its_remainder_to_the_scoped_treasury`, … | no |
| G8 | a sale no longer checks the issuer's current authority | 2 tests, one an issuer | 6 tests (3 P, 3 H): `a_sale_is_refused_once_the_issuers_side_lapses_and_a_refund_is_not`, `every_title_a_sale_stands_on_is_current_and_fenced`, … | no |
| F1 | a field moved: `OfferRecord`'s `contract` and `consent_hash` change places | 4 tests: both snapshots, both `legacy_wire_is_exact…` | 4 tests (2 P, 2 H): `legacy_wire_is_exact_and_retry_shape_forbids_missing_null_or_automatic_attempts` | yes, two |
| F2 | a field moved: `RenewalMarkerV1`'s `operation` and `effect` | both snapshots | 2 tests (1 P, 1 H): `ct0_renewal_house_snapshot`, `ct0_renewal_personal_snapshot` | yes, two |
| F3 | a field moved: `RenewalFundingReceiptV1`'s `commit` and `action` | both snapshots | 2 tests (1 P, 1 H): `ct0_renewal_house_snapshot`, `ct0_renewal_personal_snapshot` | yes, two |
| F4 | the funding receipt's marker is no longer skipped by serde (`"issuer":null`) | 10 tests, five an issuer, both snapshots among them | 10 tests (5 P, 5 H): `maximum_primitive_receipts_fit_their_real_strict_codecs`, `renewal_checkpoint_join_requires_both_actual_money_rows_even_after_hot_eviction`, … | yes, two |
| F5 | the service closure's marker is no longer skipped | 6 tests, three an issuer, both snapshots among them | 6 tests (3 P, 3 H): `service_both_causes_preserve_origin_bytes_and_all_money_through_cold_replay`, `service_closed_offer_allows_only_fresh_explicit_retry_and_retains_old_proof` | yes, two |

*The seal, with `cargo check -p mush-economy`.*

| # | The attempt, in `managed_house_renewal.rs` | The compiler's answer |
|---|---|---|
| X1 | `struct Guild; impl Issuer for Guild {}` | `error[E0277]: the trait bound `Guild: Sealed` is not satisfied`, beside E0046 for the items the attempt leaves out |
| X2 | `impl crate::renewal_twin::issuer::sealed::Sealed for Guild {}` | `error[E0603]: module `issuer` is private` |
| X3 | a second `impl Issuer for crate::renewal_twin::Personal {}` | `error[E0119]: conflicting implementations of trait `Issuer` for type `Personal`` |

Against the brief's list: every function of the trait (H1 to H9, X4 to X7, and S1, S2 for the
terms'); a family id, an action id, a digest domain and a key prefix (C1 to C8), each failing
an identity test and a `ct0_` test; the payee (S11, S12, and H1 and G4 for the contract's
scope); the account pairing (S13); a field moved in a generic record (F1 to F3). The House's
buyer admission and the `OwnCharter` stop are not in economy; the nearest economy has to an
admission is the House's authority check (S7 to S10, S14) and to a stop the issuer's lapse.

- **Twenty-one mutations passed at first:** H2, H4, H5, H5b, H5c; S3, S5 to S14 and S8b;
  G1, G2, G3, G5. Every one is a check of what an issuer's side must be (its titles, its line,
  its organisation, scope, treasury and payees), and none is a shared check: the 51 scenarios
  the House now runs were written for what the families share, and pass for both.
- **They are older than this unit.** Thirteen of them were made on main's own files (a second
  clone at `26d30dd4b`, never synced). The six personal ones that can be made there (H2 and G2,
  G3, H4 and G1, S3, S5, S6) pass main's 279 tests, and so do the five House ones tried (H5,
  S7, S12, S13 and G7's House half), which on main have only CT.0's walk to fail. G7 and G8
  for the personal family fail one test each on main, as they did here at first. The twin did not open the
  gaps: it made them cheap to find, since every difference is now a named function or a line
  of an issuer's file.
- **What closed them** is under Tests. Each new test was then shown to fail under its
  mutation (the "As built" column).
- **Not closed: three.** H5c and S3 are checks held twice ("Found on the way"), which
  no test can catch alone while the other copy stands. G5, the birth route of a transferred
  holder's basis, is reached only by record formats 3 and 4, which need a settled paid resale
  in the same store; it is owed with CT.0's holder attempts on the real path, and the two
  identity tests hold the constant meanwhile (C13, C14).
- **A House test failed under a personal mutation (H6).** One shared scenario,
  `actual_default_boot_proves_absence_without_new_serialized_history`, still called the
  personal family's public absence method and read the personal fold key when run for the
  House: a line the generic pass over the tests had left alone, in a test that passed for
  both. It asks the rig now (`Rig::absent`, `Rig::FOLD_KEY`), and the shared bodies were
  searched for any other name that is one family's. A hook mutation that fails the *other*
  issuer's test is the sign to look for.

**Divergences.**

- **The trait is §5.1's sketch with the store added and the stop taken out.** `FAMILY`,
  `OFFER_DOMAIN`, the action ids and `Terms` are there. `authority` and `payees` are functions
  of the terms (`Terms::check_current`, `Terms::payouts`), because that is where both files had
  them. `stop` is not there: economy has no `OwnCharter`. Eight functions the sketch did not
  have say which entry of a closed list is the issuer's.
- **Three modules, not one**, as in the kernel: one implementation module and two family
  modules, because every caller and every test path names `managed_renewal::` and
  `managed_house_renewal::`.
- **The records are an associated type, not a generic record** (above), so `mush-core` and the
  wire macro are untouched.
- **Seven plain names are shared by both families** (above). A request or a close reason built
  for one family is, to the compiler, the other's too. Nothing new is accepted at run time: the
  operation is looked up in the issuer's own book and the action is matched against the
  issuer's own id.
- **Two macros.** `entry_points!` is the two instantiations, as in the kernel. `subject_records!`
  is not part of the owner: it gives two concrete records one text for the inherent methods
  their callers reach by path (`RenewalOffer::decode`, `digest`, `validate`), and implements the
  record traits over them.
- **`match` became `if`.** `Configuration::require` matched on the six action ids; an associated
  constant of a type parameter cannot be a pattern. The arms are in their old order.
- **Three `#[expect(clippy::boxed_local)]`**, on the two `bind`s and on `finish_images`. The
  patches and the money preparation are boxed on purpose (they are large, and M7's lints hold
  stack frames to 64 KiB); behind a type parameter clippy cannot size them and says the box is
  not needed. The return types name two aliases (`OfferPrepared<I>`, `MoneyPrepared<I, O>`),
  which `type_complexity` asked for once the tuples had a parameter.
- **Derived `Debug` shows the marker** on the two receipts, the service closure and the
  composition's configuration, and so on whatever holds them. No production path formats them.
- **The mutations ran on a clone**, as in CT.0 to CT.3; the worktree was never mutated.
- **The House rig has a second scope.** CT.0's House fixture made one scope; the rig now makes
  a second (`other`) for the retry-on-another-line scenario. The House snapshot did not move.

**Not built.**

- Anything outside `mush-economy`. `mush-core`, `mush-roll`, `mush-market`, `mush-industry` and
  `tacsim` are unedited and pass.
- The closed lists (§4.6): the store's two books and two latches, the fold's two audits, the
  two variants of the managed consent, the two plan guards, the eighteen renewal variants of
  `EconomyRequest` and the six of `EconomyOutput`, `prepare.rs`'s eighteen request methods, and
  the two registration prerequisites in `module.rs`. Deliberate.
- The refit pair (CT.7), the purchase twins and `managed_purchase` (CT.9), `managed_coverage`,
  `managed_charter_transfer`, `managed_paid_resale`.
- A public face for the twin. `renewal_twin` is private to the crate (below, for CT.5).
- The holder attempts on the real path (formats 3 and 4 need a settled paid resale in the same
  store), as CT.0 left them: `BIRTH_ROUTE` is held by the identity tests and by no walk
  (mutations C13, C14 and G5).

**Compile time** (§7.4). conclav.dev, six jobs, `CARGO_INCREMENTAL=0`, the crate's `lib.rs`
touched before each run and every dependency already checked, wall time of
`cargo check -p <crate>`. One run that is not counted, then five; two rounds, before and after
alternating (main's own clone, then this branch's), because the box is shared and its load
moved between 6 and 12 while they ran. Medians of five, in seconds:

| | Before, rounds 1 and 2 | After, rounds 1 and 2 |
|---|---|---|
| `cargo check -p mush-economy` | 8.33, 8.46 | 8.01, 8.00 |
| `cargo check -p mush-roll` | 8.13, 8.17 | 8.14, 8.28 |
| `cargo check -p mush-market` | 3.39, 3.52 | 3.61, 3.46 |

Economy's check is about 0.4 s shorter, 4%: one owner to type-check, not two. The Roll's and
the market's moved by less than the two rounds differ from each other. They read the family
modules' aliases, and no generic function of economy's is instantiated in them: the entry
points are ordinary functions.

**On Linux** (conclav.dev, a throwaway clone of `26d30dd4b` with this branch's files over it):
`cargo test -p mush-economy` passes 881 tests over its 13 targets, none failed and none ignored (the
library's 350 among them); `cargo test -p mush-roll -p mush-market
-p mush-industry --no-fail-fast` passes 858 tests with none failed and one ignored, no file of
the three crates edited; `cargo test -p tacsim --lib ct0_` passes the registration snapshot. No
`.snap` file differs after the runs. On Windows economy's full suite passes (the same counts),
and on both `cargo test -p mush-contract-tests` (the census, the panic budget, the restriction
lints), clippy with `-D warnings` over all targets of the four crates, CI's M7 memory-size pass
and the docs check with `-D warnings` are clean. The branch's dispatched CI run, which is the
only place the M7 and M8 stories drive the renewal families end to end, [run 43610436921](https://github.com/conclavio/rustmush/actions/runs/43610436921)
at `b2186f598`: 54 of 55 jobs green when this was written, the M7 player-market story still
running. Since that commit the code has changed by one comment in `renewal_twin.rs` and the
default-boot test's fix; the pull request's own run covers the head.

**The yardstick, before and after.** Before is main `26d30dd4b`.

| Root | Before: cloned of lines | Rate | After: cloned of lines | Rate |
|---|---|---|---|---|
| `crates/mush-market/src` | 10,522 of 45,090 | 23.3% | 10,522 of 45,090 | 23.3% |
| `crates/mush-economy/src` | 20,913 of 99,897 | 20.9% | 13,466 of 97,211 | 13.9% |
| `crates/mush-roll/src/commerce` | 16,802 of 91,607 | 18.3% | 16,802 of 91,607 | 18.3% |
| `crates/mush-core/src/action_state/terminal_capacity` | 6,610 of 38,725 | 17.1% | 6,610 of 38,725 | 17.1% |

Economy lost 7,447 cloned lines, which is §7.3's estimate for its renewal twin (7,500). The two
family directories were 3,918 of 8,604 and 3,800 of 4,196; they are 123 of 577 and 210 of 691,
and `renewal_twin/` is 727 of 8,756. By `wc -l` economy's `src` went from 102,292 lines to
99,631, with eleven scenarios and a second issuer's rig more in it. What the family
directories still hold in common is the two `impl Issuer` blocks and the two rigs' seeds. What
is left at the top of economy's table is the refit pair (CT.7) and the repeated owner-test
skeletons.

**Found on the way.**

- **The House's own rules had no test** (the mutations, above), and five of the personal
  family's checks had none either: a royalty parent's title, checked and fenced; the retry's
  lineage; a title on a scope. Closed here.
- **One check has no test and cannot have one from here:** the birth route of a transferred
  holder's basis (G5). It is owed with CT.0's holder attempts on the real path.
- **Two checks are held twice.** The personal seller's presence is asked for by
  `check_current` and again by the title check that follows it (S3), and the House's
  organisation comparison in `same_line` is implied by the scope's, since a scope has one
  organisation and the authority check holds the offer to it (H5c). Both are kept as written;
  neither can be caught alone.
- **The two files differed by nothing the survey did not list.** Every line outside the 96
  hunks was the same text.
- **A second read, not anchored on this one.** A reviewer given the twenty old files and the
  new ones, and told to substitute each issuer into the twin and explain every hunk that is
  left, found no behavioural difference, no change in the order of evaluation and no
  persisted byte that could differ for either issuer, and each of the fifteen constants equal
  to its old literal character for character. It named two things that are not behaviour:
  the derived `Debug` of the two receipts, the service closure and the configuration now
  prints the marker (no production path formats them), and seven plain types are shared by
  both families (above), where a request built for one family is still refused by the other's
  owner at run time (the operation is looked up in the issuer's own book).
- No defect in production code was found, and no production behaviour was changed.

**For CT.5.**

- *Copy:* the survey first, sorted into rename, identity string and real logic, and the trait
  from it; the issuer types and the seal in one file and each `impl` in its family's file; the
  old modules as the two instantiation points; `Rig: Issuer`, `Fixture<I>` and `instantiate!`;
  an identity test an issuer; the mutations on a clone, one a function and one a constant kind,
  and a surviving mutation run against main's own files before anything is concluded from it.
- *Move the tests under the twin.* A generic scenario reads items that are `pub(super)` in the
  twin's files. With the bodies in `renewal_twin/tests/` they see them; from the family's test
  module they would not, and every visibility would have to be widened for the tests alone.
  The family's test module holds its `impl Rig` and one `instantiate!`.
- *Two commits.* The first here made the owner generic and left the old tests untouched,
  compiling against the family modules' aliases: 279 tests passing over the new owner before
  one test line changed. It costs a throwaway set of test-only forwarders in `entry_points!`,
  and it separates "the owner moved" from "the tests moved".
- *The nested test modules.* `instantiate!` rebuilds the personal family's module tree
  (`tests::funded::reoffer::…`). A macro cannot repeat a path prefix inside a repetition of
  names, so it takes the prefix as one token tree (`(funded::reoffer::)`) and a second macro
  splices it.
- *An associated type cannot be inferred from.* `fn offer<I: Issuer>(request: I::Offer)` needs
  `offer::<I>(…)` at every call. Functions that take `&OfferRecord<I>` infer. There are 26
  turbofish in the owner here and about two hundred in the tests.
- *A derive asks the issuer's records for more than they have.* `#[derive(Default)]` on a
  struct holding `Vec<I::Offer>` wants `I::Offer: Default`; write the `impl`. Serde wants
  `#[serde(bound = "")]` on every generic record, with the bounds on the trait's associated
  types.
- *The scripted pass rewrote names inside string literals.* `"RenewalMarkerV1"`, a snapshot
  label, became `"RenewalMarkerV1<I>"`, and the snapshot caught it. Exclude literals.
- *Do differently: decide what is shared.* Seven plain types here are one type under both
  paths. The Roll's `from_economy` conversions of `RenewalAttemptV1`, one a family, now take the
  same type; that compiles unedited and can be one function.
- *What the Roll will meet that economy did not.*
  - **Economy's twin is not nameable from the Roll.** Each Roll implementation can name its
    family's aliases (`mush_economy::managed_renewal::RenewalOfferView<'_>`), as economy's two
    name the kernel's builder methods. To write the Roll's body once over economy's types
    (`RenewalOfferView<'_, R::Economy>`) the module has to be public, and before that the
    eight store functions (`book`, `bind`, `receipts`, …) must leave the public trait for a
    supertrait in the private `sealed` module: `bind` hands out the snapshot mutably. That is
    an edit to economy, small, and CT.5's to propose.
  - **The `OwnCharter` stop and the authority reader** (`reader.rs`), which economy does not
    have, and a record with `attrs on both` (`RenewalBindingV1`'s `holder_basis`): if that
    record has to be generic, the macro arm CT.4 avoided is needed there.
  - **The Roll's walks are structural** (CT.0), so its snapshot proves less than economy's
    did; CT.0b's composed capture is what stands in.
  - **Rows and roots**, which have key shapes and capacity fingerprints the kernel checks:
    more identity constants than economy's fourteen, and the kernel's `Issuer` holds the other
    half of some of them.

## CT.5

The renewal twin in the Roll. Built 2026-11-14 on main `23bd1caa6` (CT.0 to CT.4 and CT.0b in;
the branch was cut from CT.0b's and merged with main `114c03694` first, whose tree is main's merge
of #596).

**The survey, before the cut.** `commerce/renewal/` and its ten production files (3,360 lines)
against `commerce/house_renewal/` and its ten (3,271): `diff` gives 153 hunks, 37 of them in
`reader.rs`. Outside the reader, file by file:

| File | Lines, personal and House | Hunks | (a) a rename | (b) an identity string | (c) real logic |
|---|---|---|---|---|---|
| `mod.rs` | 24, 19 | 5 | 5 (the order of the `mod` lines, the docs) | | |
| `audit.rs` | 299, 307 | 20 | 14 | | 6 |
| `capacity.rs` | 83, 83 | 3 | 3 | | |
| `chain.rs` | 227, 230 | 4 | 2 | | 2 |
| `history.rs` | 102, 147 | 6 | 2 | 1 | 3 |
| `protocol.rs` | 117, 117 | 3 | | 3 | |
| `records.rs` | 777, 788 | 28 | 15 | 6 | 7 |
| `roots.rs` | 236, 245 | 6 | 1 | | 5 |
| `writer.rs` | 737, 766 | 41 | 28 | | 13 |
| **All but the reader** | 2,602, 2,702 | 116 | 70 | 10 | 36 |

- *(a) A rename, 70 hunks.* Which of the commerce families is the issuer's (`renewal_operations`
  against `house_renewal_operations`, the heads, `term_economics` against `house_economics`),
  which of Economy's audits (`renewal_audit`, `house_renewal_audit`, `require_no_…`), which
  entry of the closed lists (`RootAnchor::RenewalOperation`, `RootRequest::Renewal` and their
  House twins), which family's economy types are named (`managed_renewal::` against
  `managed_house_renewal::`, `PaidTermEconomicsV1` against `HouseTermEconomicsV1`, the two
  grant types, the two admission-capacity modules), one `Box` the House did not have, and the
  order of the `mod` lines.
- *(b) An identity string, 10 hunks, 19 strings:* the five action ids (twice: `protocol.rs` and
  the closure's and operation's validation), the two `protocol` names of the action
  descriptors, the operation and head state families, the predecessor and template digest
  domains, and the offer's digest domain.
- *(c) Real logic, 36 hunks.* Outside the writer: the House binding's issuer field, its getter
  and its capture argument (5 hunks of `records.rs`), the seat count (the personal licence has
  one seat, the House template's protocol says how many), the chain held to the House that
  issued the licence (`chain.rs`, and its two `audit.rs` hunks), the birth variant and the
  issuer in the audit's join, the one audit check inverted (a later generation with no renewal
  of this issuer must be the other's), the issuer's side of an offer against the binding (the
  royalty chain's digest, or the organisation and scope), the House-only
  `granted_generation`, and the House's buyer admission carried in the roots' fence (5 hunks).
  In the writer, 13: the admission it carries and passes on, `with_purchase_admission`, the
  read grant of the entitlement economics, the accounts it pairs, the admission check at
  `observe`, the House's own economics in `absent_next`, and the issuer's side of the offer
  against the sale (lineage, seller and royalty parents, or organisation and scope).
- *`reader.rs`, 37 hunks* (758 and 569 lines). The sale reader is a different type
  (`EntitlementSaleReader`, `HouseSaleReader`), the facts it answers differ, the personal reader
  defines `RenewalStop` and computes `OwnCharter`, and a House licence is filtered by its birth.
  As doc 46 §5.1 expected, it stays as two implementations.

Where each of doc 46 §4.3's nine sits in the Roll:

| §4.3 | In the Roll | Carried by |
|---|---|---|
| 1, the terms | the writer's check of an offer against the sale, and history's check against the binding | `Rules::offer_differs_from_sale`, `Rules::offer_differs_from_binding` |
| 2, the current-authority check | each family's reader (the sale reader asks the title or the organisation store) | the two readers |
| 3, the payee | not in the Roll: Economy's payout. The Roll pairs the accounts (7) | — |
| 4, the birth variant, one audit check inverted | the audit's join, the inverted check, the chain's line | `Rules::check_birth`, `Rules::other_issuers_birth`, `Rules::line`, `Rules::off_line`, and `Issuer::Line` |
| 5, buyer admission | the House writer's admission at `observe` and the roots' fence at every fresh step | `Issuer::Admission`, `Rules::observe_accounts`, `Rules::admit_fresh`, `with_purchase_admission` |
| 6, the `OwnCharter` stop | the personal reader | the personal reader |
| 7, the account pairing | the writer's `observe`: a distinct requester and seller pair, or the requester's own account | `Rules::observe_accounts` |
| 8, the prerequisite audits | the kernel's; the Roll's is that a disabled composition has no renewal history in Economy | `Rules::require_no_history` |
| 9, the identity strings | the 19 strings of (b) | 4 Roll constants and 9 of Economy's (below) |

Both families' files held the same nine `ModError` sentences ("renewal writer requires one Roll
registration", "renewal action differs from the fixed protocol" and the rest); the House's alone
held "renewal buyer policy already configured". Every refusal of the generic code is a
`PublicationError` variant, which is not a sentence. Every string literal of the families'
production files and the twin was listed before and after (`literals.py`): 85 before, 76 after.
Gone are only the five action ids of each family and the two offer domains, which the twin reads
from Economy's constants of the same value; added are `""` (`serde(bound = "")`) and the two
`reason`s of the lint expectations below.

**Built.** One owner, two instantiations, in `crates/mush-roll/src/commerce/`:

| File | Lines | What it is |
|---|---|---|
| `renewal_twin.rs` | 33 | the module: the issuer types, the traits, the submodules |
| `renewal_twin/issuer.rs` | 293 | the sealed trait `Issuer`, the record traits `Binding`, `Operation`, `Closure`, `TermEconomics` and `Facts`, and the crate-private `Rules` |
| `renewal_twin/{audit, capacity, chain, history, protocol, roots, writer}.rs` | 1,885 | the seven files of the old personal family, generic over the issuer |
| `renewal_twin/records.rs` | 1,087 | the shared helpers, the attempt (one type), the state and the head (generic), and `binding_records!` and `operation_records!` |
| `renewal/mod.rs`, `house_renewal/mod.rs` | 235, 258 | `impl Issuer` and `impl Rules` for the family's issuer, `impl Facts` for its reader's facts, `impl TermEconomics` for its economics row; the House's `with_purchase_admission` |
| `renewal/records.rs`, `house_renewal/records.rs` | 51, 57 | each family's `checked!` binding and the two macros expanded for it |
| the families' `audit`, `capacity`, `chain`, `history`, `protocol`, `roots`, `writer` | 147, 189 | aliases of the twin's types at the issuer and forwarders, so that every path a caller names is still there; the House's `history.rs` keeps `granted_generation` |
| `renewal/reader.rs`, `house_renewal/reader.rs` | 655, 575 | as they were, but for the service tests (below) |

The twenty production files were 6,631 lines; the twenty-four are 5,465, of which the two
readers are 1,230.

*The issuer types are Economy's.* `Personal` and `House` are `mush_economy::renewal_twin`'s, and
the Roll's `Issuer` is a subtrait of Economy's. The Roll's half of a renewal is paired with
Economy's owner, so a family has one issuer type across the two crates: the twin names
`RenewalOfferOutput<I>`, `RenewalFundingReceiptV1<I>`, `RenewalOfferView<'_, I>` and the rest at
its own `I`, and reads the action ids, the protocol names and the offer's digest domain from
Economy's constants. That needed Economy's twin to be public (below).

*The trait as built.* Two traits, split by what they can name.

`Issuer: sealed::Sealed + mush_economy::renewal_twin::Issuer + Send + Sync`, public, in a sealed
module: four associated constants and nine associated types.

| Item | What it carries | §4.3 |
|---|---|---|
| `OPERATION_FAMILY`, `HEAD_FAMILY` | the two state families' ids (`roll_[house_]mind_renewal_operation`, `…_head`) | 9 |
| `PREDECESSOR_DOMAIN`, `TEMPLATE_DOMAIN` | the binding's two digest domains | 9 |
| `Binding`, `Operation`, `Closure` | the family's own three `checked!` records | 1 |
| `Economics`, `Grant` | the term economics row a generation is captured in, and its grant | 4 |
| `Reader`, `Facts` | the family's reader and the facts it observes | 2 |
| `Admission` | `()`, or the House purchase composition's `Option<Arc<dyn HouseBuyerAdmission>>` (the `dyn` was the House file's before; none was added) | 5 |
| `Line` | `()`, or the `HouseIssuer` a chain is held to | 4 |

`Rules: Issuer`, private to the crate, with two associated types (`StoredOperation`,
`StoredEconomics`: the same records, with the crate-private `Record` bound the stores need) and
twenty-one functions, each called where its check was:

| Function | What it says | §4.3 |
|---|---|---|
| `operations`, `heads`, `economics` | which of `CommerceFamilies`' fields are the issuer's. The fields stay hand-written; this is the rename (a) | (a) |
| `admission_capacity` | the entitlement or the House purchase's headroom check | (a) |
| `audit`, `require_no_history` | which of Economy's audits | (a), 8 |
| `anchor`, `root_request` | which entry of `RootAnchor` and `RootRequest` | (a) |
| `own`, `into_facts` | the reader's request and answer | (a) |
| `install_extra` | the House's read grant of the entitlement economics | 5 |
| `observe_accounts` | the pairing, and the House's admission at `observe` | 7, 5 |
| `next_present` | the House's own economics row in `absent_next` | 4 |
| `offer_differs_from_sale`, `offer_differs_from_binding` | the issuer's side of an offer | 1 |
| `admit_fresh` | the House's admission at a fresh step's fence | 5 |
| `line`, `off_line` | the chain's line | 4 |
| `check_birth`, `other_issuers_birth` | the audit's join and its inverted check | 4 |
| `seats_differ` | the seat count | 1 |

Why two traits: the hooks name crate-private types (`Birth`, `RootRequest`, `RootAnchor`,
`CommerceFamilies`' handles, `Record`), so a public trait could not carry them
(`private_interfaces`), and the twin's types must be public because the family modules' public
names are aliases of them. So `Issuer` is public and holds only public types, and the public
types' methods that need the hooks are in impl blocks bound on `Rules`, which carry
`#[expect(private_bounds)]` (the writer's two, the head's `for_operation`). From another crate
the families' names work as before; generic code over `Issuer` cannot call the writer
("For CT.6").

*The records.* Each family keeps its three `checked!` records, as CT.4 did, behind associated
types: the binding is a different shape (the House's has an `issuer` field, between `holder`
and `charter`), and `checked!` takes no type parameter. What the two files had word for word is
written once, as two macros expanded beside the records: `binding_records!` (the binding's
methods; its `extra` is the House's issuer, taken by `capture_holder` after the economics) and
`operation_records!` (the closure and the operation, their methods and the record traits). The
records that held no family field became one: `RenewalAttemptV1` is one type for both paths,
`RenewalStateV1<I>` and `RenewalHeadV1<I>` are generic (the head carries a
`#[serde(skip)] PhantomData`, and its decode is the hand-written pair it was, through the same
`HeadWire`). Every field is where it was. The House operation's binding is now boxed, as the
personal one was: `Box` is transparent to serde, so no byte moves.

*The transitions.* The generic code names a transition by `Step`, which has no issuer. Each
family keeps its own `Transition` (the market and tacsim name `house_renewal::protocol::Transition`,
and its `id` answers the family's action), expanded from `protocol::entry_points!` with the four
forwarders `descriptor`, `validate_refs`, `service_descriptor` and `service_reference`.

*What stayed as two implementations, and why.*

- **The readers**, as doc 46 §5.1 said. A different sale reader, different facts, the
  `OwnCharter` computation and the House birth filter; and two thirds of the file is the
  reader's request and inventory wire, which is the same text in both and stays so (a shared
  inventory type would be a third copy of the reader's wire to keep aligned, for 100 lines).
- **The two `impl Rules` blocks.** Eleven of the twenty-one functions are one line: a field or
  an entry of a closed list. Five are one-sided (the personal `next_present`, `admit_fresh`,
  `line`, `off_line` and `install_extra` answer nothing), kept as hooks at their places rather
  than folded, because each is called where its check was.
- **The binding**, and the House's `granted_generation`.

*What did not move.* In `mush-roll`, outside the two families and the new module: `commerce/mod.rs`
(one `mod` line) and, in tests, `commerce/tests.rs` (one `pub(crate)`) and
`commerce/tests/ct0_purchase.rs` (two `pub(crate)`s, so the renewal rigs can build CT.0's
purchase rows). `participant.rs`, `roots.rs`, `families.rs`, `module.rs`, `audit.rs` and the
paid, entitlement and House purchase files are unedited: the closed lists are as they were.

*Outside `mush-roll`: Economy, and why it is needed.* Six files of `mush-economy`, 49 lines:

- `lib.rs`: `pub mod renewal_twin` (was `pub(crate)`). Without it the Roll cannot name
  `RenewalOfferOutput<I>` and the rest at a generic `I`; the alternative was a Roll trait with an
  associated type and an accessor for each of the eight Economy types the writer reads, written
  twice, which is the clone this unit removes.
- `renewal_twin/issuer.rs`: `bind` takes a `BindKey`, which nothing outside Economy can make.
  `bind` hands out the snapshot mutably, and once the trait is public any crate could otherwise
  call it through a generic bound (a sealed supertrait would not stop that: a supertrait's
  methods are callable through the subtrait). CT.4's note proposed moving the eight store
  functions to a supertrait; the key is smaller and closes the one that matters (the other
  seven read, and `receipts_mut` needs the fold, which is `pub(crate)`). The compile-fail
  example's sentence now says why a third issuer is refused from outside.
- `renewal_twin/issuer.rs` and `renewal_twin.rs`: `Basis` gains `next_generation`,
  `prior_quote_digest`, `product_digest`, `template_digest` and `grant_economics_digest`, and
  `Terms` gains `title_revision`, implemented once in `subject_records!`: what the Roll's history
  compares.
- `managed_renewal.rs`, `managed_house_renewal.rs`, `renewal_twin/install.rs`: the key.

No persisted byte, identity string or behaviour of Economy changed; its full suite passes
unedited, CT.4's identity tests among them.

**Tests.** `cargo test -p mush-roll -- --list`, ignored tests included (there are none), before
and after:

| | Before | After |
|---|---|---|
| All targets (library, integration tests, doc-tests) | 369 | 381 |
| Under `commerce::renewal::` | 16 | 20 |
| Under `commerce::house_renewal::` | 12 | 19 |
| Doc-tests | 2 | 3 |

No name disappeared and none changed: the diff of the two sorted lists is 12 additions.

- **One body, run for both.** The families' scenarios are generic functions under
  `renewal_twin::tests` (seven in `tests.rs`, three in `tests/holder.rs`, one in
  `tests/capacity.rs`), over a test-side `Rig: Rules` (the birth's terms and grant, the
  economics row, the binding's capture, the line, the field edits a scenario makes, the offer,
  the purchase row a licence is born in). `instantiate!` names each as a `#[test]` in the
  family's test module and rebuilds the `holder` and `capacity_tests` modules, so every path is
  the one it was. The House family had ten of the personal family's eleven; it ran all of them
  before this unit (they were copies), and runs them now from the same text.
- **The CT.0 walk** was already one text (`ct0_renewal_walk!`); it is untouched, and each
  family's `tests.rs` keeps the helpers it reads (`birth`, `draft`, `granted_birth`, as
  forwarders).
- **The readers' service tests** (three, the personal reader's alone) are one text,
  `reader_service_tests!`, expanded in both readers' test modules: the House reader runs them,
  new, and passes.
- **Added**, 12 names:
  - `personal_identity_strings_hold_todays_bytes`, `house_identity_strings_hold_todays_bytes`:
    every constant the twin reads (the Roll's four and Economy's nine) against a literal, the
    records' `FAMILY`, the five `Transition` ids and the anchor's key.
  - `the_issuer_seal_admits_exactly_two`, for both, and the `compile_fail` doc-test on `Issuer`.
  - `an_offer_off_the_issuers_side_of_the_binding_is_refused` and
    `the_audit_joins_a_renewal_to_its_own_issuers_birth`, for both: asked for by mutations that
    passed (below).
  - The three reader service tests under `house_renewal::reader::service_tests::`.
- **Single-issuer, and why.** `the_marked_licence_cannot_be_offered_for_resale` stays the
  personal family's: the transfer mark is the entitlement purchase's, and a House licence is
  born sealed. The two identity tests are an issuer's by construction.

**The snapshots.** `git diff --stat origin/main -- '*.snap'` is empty. The Roll's twelve `ct0_`
tests pass on Windows and on Linux and were never re-blessed; the snapshot was never the thing
that stopped this unit, because the macros were written over the old text and the struct
literals kept their field order. The constant, field and closed-list mutations below are what it
caught.

**The proof on the composed path (CT.0b's two commands).** On conclav.dev, base the slot at
`6165e7f96` (the tree main's `23bd1caa6` has), tip this branch with the production code as built
(commit `68febabd0`; what followed it is tests and docs), both binaries built at eight jobs, the
two stories side by side:

| | Base | Tip |
|---|---|---|
| Command 1, the story against the committed files | passed, 1,114.7 s | passed, 1,112.8 s |
| The recorder (`CT0B_CAPTURE recorder_ms=… compare_ms=…`) | 195, 466 | 203, 471 |
| Command 2, `diff -r` of the unprojected records | | empty: no byte moved on the composed path |

The eight files (`{renewal,purchase}_{personal,house}.{raw,snap}`, 8.6 MB) have the same SHA-256 at
base and tip, e.g. `renewal_personal.raw` `d0376cd8…`, `renewal_house.raw` `3690ec43…`. The
branch's dispatched CI run, which runs command 1 as "M7 author and buyer lifecycle" and every M7
and M8 story, is [run 43628278565](https://github.com/conclavio/rustmush/actions/runs/43628278565):
all 55 jobs green, at `085d1b02b`; since then only tests and docs changed, and the pull request's own run covers the head.

**The seal.** `Issuer`'s supertrait is `sealed::Sealed`, a public trait in a module private to
`renewal_twin/issuer.rs`, implemented there for Economy's two issuer types.

- *From outside the crate* the trait can be named but not implemented: the `compile_fail` example on
  `Issuer` is that attempt, and runs with the crate's doc-tests. `Rules` cannot be named at all.
- *From inside the crate* the attempts were made on the clone, each alone (S1 to S3 below).
- `the_issuer_seal_admits_exactly_two`, run for both issuers, is the census: two
  `impl Sealed for mush_economy::renewal_twin::…` in `issuer.rs`, one `impl Issuer for` and one
  `impl Rules for` in each family's `mod.rs`, one `mod sealed {` in the twin, no `dyn` beside
  either trait's name. A census of the source, not a proof.

**The mutations.** Each was made alone on a throwaway clone on conclav.dev (the worktree was never
edited), the Roll's library tests run (`cargo test -p mush-roll --lib`, 373 tests at the first
pass, 378 as built) or, for a check the library cannot reach, the pinned M7 story run with its
own binary (`ct5-story-muts.py`: build, copy the binary, put the file back, run four at a time).
"P" is a test under `renewal::`, "H" one under `house_renewal::`.

*`Rules`, one issuer at a time, answering its neutral value.*

| # | The mutation | First pass | As built | The M7 story |
|---|---|---|---|---|
| R1 | personal `observe_accounts` answers `Ok` | nothing | nothing | **fails** (630 s): `retained actual login account pair must be rechecked` |
| R2 | House `observe_accounts` answers `Ok` | nothing | nothing | **fails** (75 s): the story's House admission refusal (`stage(…).is_err()`) no longer refuses |
| R3 | House `next_present` answers `false` | nothing | nothing | passes |
| R4 | personal `offer_differs_from_sale` answers `false` | nothing | nothing | passes |
| R5 | House `offer_differs_from_sale` answers `false` | nothing | nothing | passes |
| R6 | personal `offer_differs_from_binding` answers `false` | 1 test (P): `an_offer_off_the_issuers_side_of_the_binding_is_refused` | the same | — |
| R7 | House `offer_differs_from_binding` answers `false` | 1 test (H): the same name | the same | — |
| R8 | House `admit_fresh` answers `Ok` | nothing | nothing | passes |
| R9 | House `off_line` answers `false` | 1 test (H): `closed_attempt_can_precede_exact_retry_grant_without_eternal_next_absence` | the same | — |
| R10 | personal `check_birth` answers `Ok` | nothing | 1 test (P): `the_audit_joins_a_renewal_to_its_own_issuers_birth` | passes |
| R11 | House `check_birth` answers `Ok` | nothing | 1 test (H): the same name | passes |
| R12 | personal `other_issuers_birth` answers `true` | nothing | 1 test (P): the same name | passes |
| R13 | House `other_issuers_birth` answers `true` | nothing | 1 test (H): the same name | passes |
| R14 | personal `seats_differ` answers `false` | nothing | nothing | — |
| R15 | House `seats_differ` answers `false` | nothing | nothing | — |
| R16 | House `install_extra` grants nothing | nothing | nothing | **fails** (75 s): the first House draft, `This renewal is unavailable or has changed.` |
| R17 | House `admission_capacity` answers `Ok` | 1 test (H): `admitted_widest_economics_keeps_all_actual_roll_terminal_envelopes_bounded` | the same | — |
| R18 | personal `admission_capacity` answers `Ok` | 1 test (P): the same name | the same | — |
| R19 | personal `require_no_history` answers `Ok` | nothing | nothing | — |
| R20 | House `line` answers the structural House without reading the birth | nothing | nothing | **fails** (45 s): boot, `commercial records disagree` |
| X1 | the personal `anchor` is the House's | 2 tests (P): `ct0_personal_renewal_snapshot`, `personal_identity_strings_hold_todays_bytes` | the same | — |
| X2 to X5 | the personal `root_request`, `operations` and `audit`, and the House `economics`, handed the other entry | does not compile: `error[E0308]: mismatched types`, four times | the same | — |

*The identity constants, one of each kind.* Each fails its issuer's identity test and its
issuer's snapshot.

| # | The mutation | What failed |
|---|---|---|
| C1 | a state family, personal operation | 3 tests (P): `ct0_personal_renewal_snapshot`, `personal_identity_strings_hold_todays_bytes`, and `command_reader::tests::command_reader_trade_kinds_answer_only_their_own_party` |
| C2 | a state family, House head | 2 tests (H): `ct0_house_renewal_snapshot`, `house_identity_strings_hold_todays_bytes` |
| C3 | a digest domain, personal predecessor | 2 tests (P): the snapshot and the identity test |
| C4 | a digest domain, House template | 2 tests (H): the same |
| C5 | an action id the twin selects (`Step::Grant` answers the fund action) | 4 tests (2 P, 2 H): both snapshots, both identity tests |
| C6 | a protocol name, Economy's House service constant | 2 tests (H): the snapshot and the identity test |
| C7 | a key prefix, the House operation's root anchor (`roots.rs`) | 2 tests (H): the same |

*A field moved in a record.*

| # | The mutation | What failed |
|---|---|---|
| F1 | the personal binding's `charter` and `line` | 1 test (P): `ct0_personal_renewal_snapshot` |
| F2 | the operation's `offer_digest` and `contract` (`operation_records!`, both issuers) | 2 tests: both snapshots |
| F3 | the head's `current` and `attempted_predecessor` (generic) | 2 tests: both snapshots |

*The hooks' call sites in the one implementation, and the writers' pairing swapped.*

| # | The mutation | Unit tests | The M7 story |
|---|---|---|---|
| G1 | history: the issuer's side of the offer not asked | 2 tests (1 P, 1 H): `an_offer_off_the_issuers_side_of_the_binding_is_refused` | — |
| G2 | chain: a row off the line not refused | 1 test (H): `closed_attempt_…_without_eternal_next_absence` | — |
| G3 | binding: the seat count not asked | nothing | — |
| G4 | writer: the sale's side of the offer not asked | nothing | passes |
| G5 | writer: the House's next economics not asked | nothing | passes |
| G6 | roots: the fresh admission not asked | nothing | passes |
| G7 | audit: the birth not joined | nothing | passes |
| G8 | audit: the inverted check not made | nothing | passes |
| G9 | writer: the accounts not paired | nothing | **fails** (75 s), as R2 |
| W1 | the personal pairing is the House's (the requester's own account, no seller) | nothing | passes |
| W2 | the House pairing is the personal one (a distinct pair) | nothing | **fails** (45 s): the first House draft, `This renewal is unavailable or has changed.` |

*The seal, with `cargo check -p mush-roll`.*

| # | The attempt | The compiler's answer |
|---|---|---|
| S1 | `struct Guild; impl Issuer for Guild {}` | `error[E0277]: the trait bound `Guild: issuer::sealed::Sealed` is not satisfied`, and the same for Economy's `Issuer`, beside E0046 for the items left out |
| S2 | `impl …::renewal_twin::issuer::sealed::Sealed for Guild {}` | `error[E0603]: module `issuer` is private` |
| S3 | a second `impl Issuer for Personal` | `error[E0119]: conflicting implementations` |

Against the brief's list: every `Rules` function answering its neutral value (R1 to R20), the
closed-list entries handed to the other issuer (X1 to X5), a constant of each kind (C1 to C7), a
field moved in a family record, in a macro record and in a generic record (F1 to F3), the
writers' pairing swapped between the issuers (W1, W2), and the seal (S1 to S3).

- **Twenty-four passed the library's tests at first** (every hook that only a writer, a fence or
  a boot audit calls). Ten are closed: the four birth hooks by
  `the_audit_joins_a_renewal_to_its_own_issuers_birth` (R10 to R13), and six fail on the story
  (R1, R2, R16, R20, G9, W2). The offer-side hooks and their call site (R6, R7, G1) failed from the
  first pass, on the scenario this cut added before the runs.
- **Not closed: fourteen**, and none is new. Each that can be made on main's own files was
  (`ct5-muts-base.py`, the slot at base, never synced): the seat check (B14, B15), the birth join
  (B7), the inverted check (B8) and the disabled composition's check (B19) pass main's 367 tests.
  - *Held twice* (R14, R15, G3): the seat count is the grant's template's own validation
    (`entitlement::economics`, `house_purchase::economics`), run by `binding.validate()` before
    the binding's check, and the binding's terms must equal the template's. No binding that
    passes the first can fail the second.
  - *Reached only through the reader's facts* (R4, R5, G4: the offer against the sale; R3, G5:
    the House's own next economics; R8, G6: the fence's admission; W1: the personal seller's
    distinct account). The writer's facts come only from the reader, whose sale facts
    (`EntitlementSaleFacts`, `HouseSaleFacts`) have private fields in `entitlement` and
    `house_catalogue`, which this unit does not touch. On the composed path the market builds
    the offer from the same facts, so no story step has an offer that differs from its sale, a
    House row in the next generation before the grant, a fresh step after the policy changed,
    or a seller who is the renewer.
  - *Reached only through a store with a wrong row* (G7, G8, R19): the boot audit's joins
    refuse rows no writer can write.
  Closing them is a composed test in tacsim (CT.8's rig) or a test constructor for the two sale
  facts (CT.9's files). Owed, below.
- **A cross-issuer failure.** C1, a personal constant, also failed
  `command_reader::tests::command_reader_trade_kinds_answer_only_their_own_party`, which reads
  the personal operation family by its literal: expected, and a sign that no shared body was
  reading one family's name.

**Divergences.**

- **The trait is §5.1's sketch split in two.** The identity strings, the family's records and
  its reader are `Issuer`'s; the authority, payee and stop of the sketch are not in the Roll as
  such (§4.3's table above). Every hook that names a crate-private type is `Rules`'s.
- **Economy's issuer types, Economy's constants.** The Roll's issuers are Economy's `Personal` and
  `House`, and nine of the Roll's identity strings are read from Economy's constants of the same
  value (the identity tests hold each to the literal). Before, each crate held its own copy.
- **Three `#[expect(private_bounds)]`**, on the impl blocks of the writer's two public types and on
  the head's `for_operation`: the methods need `Rules`, which is private to the crate.
- **One `#[expect(clippy::ptr_arg)]`** on `Rules::next_present`: a state family's key is a
  `String` and `ActionTxn::get` takes `&K`.
- **`RenewalStateV1::terminal_commit` is no longer `const`**: it reads the closure through the
  `Closure` trait. No caller is in a const context.
- **The House operation's binding is boxed**, as the personal one was. Not a byte.
- **The family `Transition` lost `fresh` and keeps `select` for tests only**; both were
  `pub(crate)` and the generic code uses `Step`'s. `Transition::step` is new.
- **The House `absent_next` reads the entitlement economics before its own** (the second read's
  order inside one `||`; every read answers `Mismatch`, so no outcome moves; a House row present
  now costs one more read before the refusal). Found by the second read (below).
- **The House history check compares the organisation and scope before the buyer**: pure
  comparisons reordered inside one `||` chain with no fallible call between them.
- **Two test files of CT.0's are edited for visibility only** (`commerce/tests.rs`,
  `commerce/tests/ct0_purchase.rs`, three `pub(crate)`s), and each family's
  `tests/ct0_snapshot.rs` makes its `ct0_offer` `pub(super)` for the rig.
- **The mutations ran on clones**, as in CT.0 to CT.4.

**Not built.**

- The readers, as one implementation (above), and the closed lists (`CommerceFamilies`' fields,
  `RootRequest`, `RootAnchor`, the families' registration in `module.rs`). Deliberate.
- The purchase twins (`entitlement`, `house_purchase`), `paid_resale`, `charter_pending`,
  `conduct`, `rating`, `registry_recall`, `registry_revocation`: untouched (CT.9's or nobody's).
- Tests for the fourteen surviving mutations: a composed test or the sale facts' constructors.
- A public writer face over a generic issuer ("For CT.6").

**Compile time** (§7.4). conclav.dev, six jobs, `CARGO_INCREMENTAL=0`, the crate's `lib.rs`
touched before each run, one uncounted run then five; two rounds, base then tip, while the box
ran other sessions' builds and this unit's stories (load 13 to 20). Medians of five, in seconds:

| | Before, rounds 1 and 2 | After, rounds 1 and 2 |
|---|---|---|
| `cargo check -p mush-roll` | 8.89, 8.90 | 8.96, 9.06 |
| `cargo check -p mush-economy` | 8.83, 9.09 | 8.99, 8.91 |
| `cargo check -p mush-market` | 4.03, 3.92 | 3.93, 3.85 |

The Roll's check is 0.1 s longer, under 2%, about what the two rounds differ by: the generic
owner is type-checked once and monomorphised twice. Economy's and the market's did not move.

**On Linux** (conclav.dev, a throwaway clone of main with this branch's files over it):
`cargo test -p mush-roll -p mush-market -p mush-industry -p mush-economy --no-fail-fast` passes
1,751 tests with none failed and one ignored (the same one as before); `cargo test -p tacsim --lib
ct0_` passes the registration snapshot. No `.snap` file differs after the runs. On both,
`cargo test -p mush-contract-tests` (the census, the panic budget, the restriction lints), clippy
with `-D warnings` over all targets of the two edited crates, CI's M7 memory-size pass and the
docs check with `-D warnings` are clean.

**The yardstick, before and after.** Before is main `23bd1caa6`.

| Root | Before: cloned of lines | Rate | After: cloned of lines | Rate |
|---|---|---|---|---|
| `crates/mush-market/src` | 10,522 of 45,097 | 23.3% | 10,522 of 45,097 | 23.3% |
| `crates/mush-economy/src` | 13,466 of 97,211 | 13.9% | 13,468 of 97,253 | 13.8% |
| `crates/mush-roll/src/commerce` | 16,802 of 91,619 | 18.3% | 11,078 of 90,275 | 12.3% |
| `crates/mush-core/src/action_state/terminal_capacity` | 6,610 of 38,725 | 17.1% | 6,610 of 38,725 | 17.1% |

The Roll lost 5,724 cloned lines, against §7.3's estimate of 7,000 for its renewal twin: what is
left in the two family directories (634 of 1,507 and 637 of 1,415, from 3,530 of 4,441 and
3,525 of 4,280 with `--stems`) is the readers' shared wire and the two `impl Rules` blocks. By
`wc -l` the Roll's `src` went from 99,200 lines to 97,894, with the House's three reader tests,
two scenarios a family, the identity tests and the census more in it. Economy moved by the
accessors.

**Found on the way.**

- **The House's own rules were tested by the copies of the personal scenarios only.** Six of
  them fail now under a mutation that passed before (R6, R7, R10 to R13): the issuer's side of an
  offer, and the audit's birth join and inverted check, for either issuer. They are covered by the
  two scenarios added for both issuers.
- **Fourteen checks have no test that can see them** (the mutations above), all older than this
  unit and the same on main's files: the reader-fed checks of the writer and fence, the boot
  audit's joins over rows no writer writes, and a seat check held twice.
- **Economy's twin had to be public.** `bind` was the one function of its `Issuer` that a public
  trait must not offer; it now takes a key. A second read found that the other store functions
  are now nameable from other crates too (`receipts_mut` among them). They are not a hazard: the
  store's fold is `pub(crate)`, so another crate can reach only a `FoldState` it made itself, and
  the readers return shared references.
- **A second read, not anchored on this one.** A reviewer given the old files and the new, and
  told to substitute each issuer into the twin and explain every difference, found no
  behavioural difference in persisted bytes, identity strings, the error returned or the order of
  side-effecting calls. It named the House `absent_next`'s read order and the House history's
  reordered comparisons (both under Divergences), the boxed binding (no byte), the lost `const`,
  and the widened Economy surface (above).
- No defect in production code was found, and no production behaviour was changed.

**For CT.6.**

- *What the market can name now.* Economy's `renewal_twin` is public (its generic outputs,
  receipts, views and audit at a type parameter), and so is the Roll's (`Issuer`, the records,
  `RenewalStateV1<I>`, `RenewalHeadV1<I>`, `Step`, and the writer types
  `RenewalWriter<I>`, `RegisteredRenewalWriter<I>`, `PreparedRenewal<I>`). The issuer types are
  Economy's `Personal` and `House` for both crates, so the market's trait can be a third subtrait
  over the same two types.
- *What it cannot.* The writer's methods are in impl blocks bound on the Roll's `Rules`, which is
  private to the Roll, so generic market code over `I: mush_roll::…::Issuer` cannot call
  `draft`, `fund`, `terminal` or `close`. Either the market's two families keep calling their
  family's aliases (as now; nothing has to move), or the Roll gives the writer a public face:
  `Rules`' hooks name `Birth`, `RootRequest`, `RootAnchor` and `Record`, which are `pub(crate)`,
  so that face would be a public sealed trait whose methods take only public types (the writer's
  own), implemented for the two issuers by forwarding. Decide before writing the providers.
- *Copy:* the survey sorted into rename, identity and logic, with counts; two traits where the
  hooks name crate-private types; the records behind associated types with one macro text; the
  family's own transition enum over an issuer-free `Step`; `Rig: Rules` and `instantiate!`;
  the identity test an issuer; the mutation runner over a clone, the story runner for what only
  the composed path reaches (`ct5-story-muts.py`: about 4 minutes a binary at six jobs and
  18 minutes a story, four at a time).
- *Watch for:* `I::economics` meant two things (the rig's constructor and the rule's family
  handle) until one was renamed; method calls on an associated type need the trait in scope
  (`Record`), unlike a type parameter's bounds; `macro_rules!` takes a path prefix as one token
  tree (`(holder::)`), as CT.4 found; a `#[cfg(test)]` file under `src/` outside a `tests` dir
  needs `#![cfg(test)]` or the panic budget counts it; a module's outer `///` and inner `//!`
  docs together resolve links in the parent's scope.
- *CT.0b's proof* took 20 minutes for both commands on conclav.dev with the binaries built
  (4 minutes each, eight jobs): the story is 1,113 s.

## CT.6

Not built.

## CT.7

Not built.

## CT.8

Not built.

## CT.9

Not built.
