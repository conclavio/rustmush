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

Not built.

## CT.2

Not built.

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
