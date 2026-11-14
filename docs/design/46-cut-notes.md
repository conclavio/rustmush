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

Not built.

## CT.4

Not built.

## CT.5

Not built.

## CT.6

Not built.

## CT.7

Not built.

## CT.8

Not built.

## CT.9

Not built.
