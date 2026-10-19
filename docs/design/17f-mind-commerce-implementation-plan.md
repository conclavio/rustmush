# 17f — Mind commerce, Registry rights and IC hardware: implementation plan

Implementation continuation, 2026-10-19, on `codex/unit-minds-through-m6`.
M6 is accepted in `17097f5b` with the owner's explicit one-time parity
exception; [17e](17e-field-pilot-acceptance.md) preserves the failed raw comparison
and the passing functional evidence. The current economy checkpoint is IC14
merge `eafe0064caa86411dadb08d4c24ae17488a2bd69`, tree
`2262fd9eb65a765fbbd2ae4d69df8c3ddcd82697`: 49 stages and 1,508 tests pass,
including actual Redis recovery. On 2026-10-18, the owner requested the accepted
through-M6 work land before M7. A fresh fetch confirmed main at `a690412e`, then
a normal fast-forward push advanced it to `6fafaa11dfa6765c0b3a56ef81bc3a2f6e44e4fa`;
an independent remote-ref read verified that result. Executable tracked bytes
are unchanged from the passing IC14/M6 checkpoint; later commits only updated
the five design/acceptance documents. No M7 implementation was in that merge.
[17b §6](17b-unit-minds-implementation-plan.md#6-m6-candidate-and-acceptance-work--2026-10-17)
records the archive and exact provenance. Both implementation prerequisites are
complete. The owner has authorized progress through M7 and progressive pushes.

This note promotes the reviewed isolated design into the programme's tracked
implementation plan. The substantial prepared code remains under active
integration and testing; it is not M7 acceptance or deployment. N35 passed
compile and 215 fast tests (one ignored), then its genuine purchase story aborted
at a nested test-future stack overflow. Its independent server story was
diagnostically cancelled after sustained Registry waiting without an execution
cursor. Recovery of its disconnected runner preserved the original terminal
stage outcomes and separately recorded source verification. N36 was superseded
without execution. N37 failed compilation on a private constant; the narrow
visibility repair allowed N38 to compile.

N38 (`c4f5317d9b5af6464200870f2bd9bb2deed59207`) finished with 392 passing
tests, two failures and one ignored test. The failures were an obsolete
fork-only deletion assertion after retirement support and an unmatched expected
boot-refusal message. Both expensive stories remained unexecuted. All 13 retained
files verify, with unchanged source; archive SHA-256 is
`2a49296d82bf60eb4d29701056bce316d4f11b1ec1358d98c92797d1009d4f2d`.
The source-only F2 renewal capacity prerequisite has independent review and
seven prepared coordinator tests, but no Cargo result yet.

N39 froze 180 reviewed packages at
`2500edb31d3b2e199d3fe4f9b13574d7a9bb147f`. After the unrelated host release
build finished, compilation identified an import and exact-lookup visibility
error; all runtime stages remained unexecuted. Those errors were repaired.

On the next continuation, main was fetched at `3160cdd0` and merged normally
into the implementation branch at `4b6d710f`, then pushed. The candidate includes
the owner's line-tables-only debug profile, Linux CI changes, AQ command-name
protection and P-B's Missing-only Co-op admission rule. Accepted main remains
separate from M7 acceptance. All cumulative preparation through N42 is now
materialized in this implementation checkout for review and progressive commits.
N45 tree `54f804536f00e025abbe1519a4c881fe6cd794f5` was committed and pushed
as `31d19f25` (458 files). This is an implementation checkpoint, not M7 acceptance.

N40 (`e7afe36ab47067da3ec76c08c81c465f6bf9c159`) added the reviewed renewal
capacity, exact linked retry, Economy recovery/participant bridge, Roll renewal
writer and immutable-history audit, strict Market renewal records, Republic
membership guard and upstream reconciliation. Compilation found a missing
re-export of the existing 1,024-byte receipt limit; the limit did not change.
N41 (`3e23a5c7c287dfd358f900621510cd7d31b660e4`) then found missing generic
scan bounds in Market's entitlement recovery. N42
(`225264bae5e652e95538cf3284e58a4539901312`) reached the test fixtures and found
two outdated API calls: the canonical idempotency parser and infallible offer-key
setter. The narrow repairs preserve all production checks and test assertions.
These three runs failed compilation, executed no runtime tests, and retained
their exact unchanged source, logs and verified evidence archives. None is an
acceptance result.

N43 and N44 also failed compilation; repairs corrected a test-only JSON macro,
explicit generic lifetime bounds and an old retirement fixture field. N45
compiles and has passed 554 Core tests, nine Republic-membership tests, two
future-size tests, 20 protocol unit tests, 32 protocol wire/property tests,
14 IPC tests, 14 SSH workshop tests, 146 Economy tests, 174 Roll tests and
eight composition tests. Market has 74 passes and two failures: the public
numeric licence selector was decoded as a persistent string ID, and a negative
fixture broke its own strict funding/contract consistency before the intended
pairing assertion. Workshop has 26 passes and one failure: owned stale retirement
selection returned `missing` instead of the existing `stale` response. Narrow
repairs are prepared; the original failed results remain retained. Later N45
stages then completed: 1,149 tests passed, three failed and one was ignored.
Both genuine stories remained unstarted because earlier required gates failed.
The 18-file evidence archive verifies against unchanged source, SHA-256
`a904086547f38d2f2574bf6342ff0da2fd03e54faf53db4de3d5c94438ce8500`.
Neither genuine story is claimed passed. All 59 web test files / 625 tests pass
against the committed N45 tree.
The first web attempt executed zero tests because Windows sandbox temporary-file
renames were denied; the authorized unsandboxed rerun passed. The actual Rust
protocol exporter then passed both tests and supplied the two removal message
variants. Those generated files are included in N46, whose production web build
and typecheck pass with zero checker errors or warnings. The existing bundle-size
advisory remains report-only. Complete M7 acceptance remains required.

N46 tree `a65939338a12f679ac0576e731108a8045d61056` was committed and pushed
as `bde138fd`. Compilation and 1,165 tests pass; seven fail and one is ignored.
The six Tactics failures share a fixture whose unit roots were not explicitly
selected as current, so real `@unit/create` refused before replacement. One
withdrawal fixture used numeric persistent commerce IDs instead of their
canonical string representation. Both narrow fixture repairs preserve production
checks and assertions. N45's two Market failures and workshop stale-response
failure now pass. Both genuine stories remain unstarted because required gates
failed. The 19-file evidence archive verifies unchanged source, SHA-256
`998e8e5623c68feddbecc4518436f644030ea012c7ccbd84f8430a3c8af5a516`.

The next candidate adds the reviewed optional renewal service: separate immutable
Economy cleanup proof, exact Roll/Market progression, private retained service
intent, holder-bound due processing, and source-free status/request/accept/decline/
retry commands. Automatic processing never pays or retries a closed offer.
Its six Economy, four Market, one reader and genuine child checks still need
execution. The new inactive IC prerequisite adds strict permanent claim history,
ordered capacity replay, actual personal admission reading, and eight structural
tests including same-generation close-clock rollback refusal. It registers no
IC service or custody adapter; the actual four-owner custody, full seal, future
close reservation and host integration remain separate work. These prepared
packages and fixture repairs are not acceptance results or default activation.

N47 tree `25ab15488942154a09f16d20c3060266d9fc9082` stopped at compilation:
the renewal integration added a field to the unrelated purchase registration and
omitted an explicit nested test path. Its two-file terminal archive is verified,
SHA-256 `9ae2ef1470963df1e09e7b2c1a1c99cfecb50614eec84dc911135d3e3c572b21`.
N48 tree `0b949813fba009c16bd1c3b79266ee1262082870` resolved both, then
stopped on an ambiguous redundant String conversion in Market's status output.
Its two-file terminal archive is verified, SHA-256
`eea057e3b5f8afb23ec32f4a9119097da7b62e53f43a7f7617bbfaeea77b749f`.
Both runs executed zero runtime tests; all downstream stages remain unstarted.
The next candidate removes that conversion without changing output bytes.

N49 tree `3dcf5b3e21b5981ef7a1e3286e7fcac980edc3e1` also stopped at
compilation on the two remaining redundant status-output conversions. It ran
zero tests; its verified archive is
`edb8e088f64fd633843bc622f8a577e739b573086093a2ae54a41220f4ef69af`.
N50 tree `f3067fa096c0c94ab3826cd2c47d72b5503fcede` compiled and finished
with 1,184 passing tests, 8 failures and 1 ignored test. Six Tactics
replacement tests now reach the actual provider and expose its test fixture's
missing typed transaction receipt. Two renewal-service tests expose an overlong
capability identifier. Narrow repairs add the test-only typed receipt and retain
the complete domain-separated service digest within Core's 64-byte identifier
limit. Core's 564 tests, Economy's 152 and Roll's 186 pass. Both genuine stories
remain unstarted because required gates failed. All retained evidence verifies
against unchanged source; archive SHA-256 is
`569f7eced01fcca8e8f32f1d39d319529e961b49278fa165bb9edecfe9cc21da`. These failed runs remain evidence,
not acceptance results.

N50 was committed and pushed as `2d9d79fd`. A fresh fetch then found main at
`9cc0dc16`; its CI-only linker/job/resource changes were merged normally and
pushed as `bc817ac9`. The next candidate includes that exact CI update. It also
adds opt-in compatible original-line publication with an actual new Registry
certificate and fee, and personal dependent-owner parent-licence replacement.
The latter preserves original source ancestry, child expiry and independent
paid-customer terms; Open restores owner rights while OpenResale is required for
new sales and renewals. Neither prepared feature has runtime acceptance yet.
The four-owner IC custody service and its fixed future-close capacity are still
in separate preparation; the inactive claim records alone do not enable it.

N51 tree `f64926fae3bc9acc84a91a7b6a6f314ecaa52507` stopped at compilation
because two already-public compatible-update types lacked the parent module
re-exports used by Roll. It executed zero runtime tests. Its two-file archive
verifies unchanged source, SHA-256
`fc043ec96f75d3fb33682713d9df09d5555afe8644403109743127226c6ce716`.
The next candidate fixes those exports and adds the reviewed Core IC capacity
prerequisite: one fixed Roll claim target with Tacsim metadata, immutable Open
fingerprint, reserved complete Close capacity, exact four-owner preparation and
all binds before any apply. Current optional commerce needs 60 participant
requests; IC adds eight, so the declaration ceiling becomes 128. Eight adapters
cover the seven IC/commerce slots plus optional hardware. Same-owner adapters
may share a state slot only with disjoint exact actions; each invocation still
refuses duplicate slots. Ordinary transactions retain their three-image/two-module
limit; only the fully validated IC protocol receives four/four. Ten new tests
and the updated 128/129 boundary are prepared, not passing evidence. Physical
owners, actual seal/event authority and complete integration remain required.

N52 tree `60ba2a4f39fa9a4858407e54ca4ff4c5e0755f15` was frozen and
materialized but superseded before upload or execution. Static review caught a
compatible-update history helper calling mutable transaction reads through an
immutable reference. The narrow repair accepts the existing mutable transaction
and owns the exact operation row across subsequent reads; all identity/proof
checks remain. N52 has no compiler or runtime result. The next runner applies
its combined delta directly over the retained terminal N51 source.

N53 tree `d27ad8f27a00c33c5e876c75d523d8e60e560783` stopped at compilation on two
overlapping mutable transaction borrows in the compatible-update provider. It
ran no tests. Its unchanged-source archive verifies as
`fbfcd14dcc5c0493fd08d1b640cdf653090a5ae7d242bd88a7c4de7dd1477261`. The repair owns the charter and
licence snapshots before reading subsequent rows; identity and eligibility
predicates are unchanged. The next candidate also incorporates upstream main
`41e7259d` (astronomy engine A.1) and adds its sky regression gate.

The next candidate includes the reviewed Sim/Tactics physical IC owners and
their actual Engine adapter tests. The Sim guard rechecks even retained public
transition plans before effects, refuses retirement of the captured domain and
fails closed on missing or crossed indexed custody history. Strict test-only
typed receipts accompany the physical prepared images. These are prerequisites,
not a complete IC service: actual sealed Tacsim runtime/host event authority,
paired Roll roots/claims and complete four-owner boot audit remain required.

N54 tree `2ae22abee09c0fecb49e2e5056043fb1f35a2052` stopped at compilation on three
private `CarrierJourney.root` field accesses in the new Sim boot audit. Its
verified unchanged-source archive is
`d88c64bb5714c90df05fcd6668f741a1c6819a77ca9a3cec82a947545247ec47`; zero tests ran. The next
candidate uses the existing read-only `root()` accessor for the same hull,
base-space and domain-host overlap checks. No admission predicate changes.

N55 tree `67f4627978a2ea8c003d666d4d8e8ed9c6b229eb` passed production checking but
the compile stage failed on a missing `DefinitionValidationContext` import in
the Tactics component-replacement test module. No runtime tests executed. Its
unchanged-source archive is
`ad6e317446ca64fd049d5cbeb5f94e6d117375b0749c253c65e6cd7bf367f75d`. The next candidate adds only
that test import and this evidence note before rerunning all required gates.

Continuation on 2026-10-19: checkpoint N56, tree
`f6dfe46241bd357cd21db5504ff2fbbac096d273`, was committed and pushed as
`b476faee`. The normal merge of main `41e7259d` was pushed as `8eb5c3d4`.
A fresh fetch then found main `c44c0bb6` (astronomy A.2-A.6); it was merged
normally and pushed as `94af1bfb`, tree
`cc86e77afee3983b0abf003871155fd50a8d97e4`. The newer chart, pricing,
route-planning and ephemeris changes are included in the next candidate.
N56's running source remains the earlier exact tree; its results do not
validate the later main merge.

N56 finished with 1,309 passing tests, 11 failures and two ignored tests.
Core's 574 tests, all 45 deliberation tests, the actual BOARD/order and RELEASE
regressions, all seven Tactics replacement checks, the bot arena contract,
Market's 80 tests and Roll's 197 tests pass. Four Sim adapter tests stop at
incomplete test-only participant registration. Five Economy update tests stop
at an initial-filing fixture without a typed transaction write. The snapshot
shape test first stops at obsolete expected format 13 (production is 15).
The ownership census rejects three unnecessary same-owner declarations; its
cross-owner access requirement remains unchanged. The reviewed narrow repairs
are prepared. Both genuine stories remain unstarted because required earlier
gates failed. All 29 retained files verify against unchanged source; archive
SHA-256 is `b2181d4e099760cfbec47d686a6662d7bec7fb29416ccf2c53aca218de982be7`.
This is partial correctness evidence, not M7 acceptance. N56 supplies runtime
evidence for delayed commander fitness and the whole-squad RELEASE notice
correction described below.

The next candidate adds reviewed Core output reservations, personal owner
attestation and compatible updates after attestation, plus explicit leader
record/default and self-charge expiry tests. Output reservations are inert
factory prerequisites, not completed manufacture, delivery or installation.
The adapter registration ceiling becomes nine for the actual four commerce,
three IC, Tactics replacement and Industry slots; ordinary transaction image
and module limits do not change. The owner-attestation paths preserve paid
customer terms, original ancestry and historical update generations. These
new paths still require execution. An isolated snapshot diagnostic will first
check the complete format-15 shape before its hash/count are repinned; merely
changing the version assertion is not acceptance. Lost order watches on resume
and the complete IC runtime remain separate work.

N57 tree `074d5a62ab57220bda39a37949983f0f93dc194a` was an isolated
snapshot diagnostic. It stopped during compilation: an Arc-backed Core test
owner lacked `Clone`, the attestation history reader called a context-specific
receipt API, and its nested JSON fixture exceeded the macro recursion limit.
No runtime test or snapshot diagnostic executed. Its two-file archive verifies
unchanged source, SHA-256
`1547ee6d64bd471afa90cc690fb85e9e154e0a7706380b202392eb6886bbc4b3`.
The next candidate repairs those integration errors before repeating the
snapshot diagnostic; the complete runtime gate remains required.

N58 tree `9f16c2d7e1161e930b791898479623d15d3d7810` compiled the N57
repairs, then stopped on Industry API mismatches: three page-cursor conversions,
two typed replay-write identifiers and two test uses of the acknowledged bundle.
No runtime tests executed. Its unchanged-source archive is
`f30b4af25b175c040f7c157be4295f59c3e6ffca32dcb41255817a4df5a8cd9f`.
The narrow API repair precedes the repeated diagnostic.

N59 tree `8bfe7d5caaa4d9c089151266d524dedf75319b91` compiled and ran
the focused diagnostics: 37 passed, 16 failed, none ignored. Core output
reservation (11), E4 update (6), owner attestation (6), attested compatible
update (6), leader-record/default (1), self-charge expiry (1), state-owner
contract (5) and snapshot shape (1) pass. The format-15 opening fixture retains
the existing 693-path digest: neither the filed-origin variant nor the new
refusal payload is populated by that walk, so this does not claim full format
coverage. No shape hash was changed merely to make the diagnostic pass.

All four Sim tests stopped on the fixture's invalid canonical namespace; all
twelve Industry tests stopped because their cold-boot fixture omitted the real
server's typed-state hydration step. The next candidate fixes those two setup
errors without changing production predicates or weakening assertions. N59's
12-file archive verifies unchanged source, SHA-256
`2a01e0f301f7477fefe841f1a0902e691b2db60b55b38c35ee2cf658bd2f190a`
(15,347 bytes); status SHA-256
`00942606a5c2ed33432d42f5e7527e6bfc37450029bcec51368e67608b25acde`.
This was a focused diagnostic, not the full gate; the genuine purchase and
server stories were not part of its run.

The reviewed L R4 recovery implementation now retains bounded, actual accepted
order attribution in snapshot format 16. It restores the latest squad watch
against the published arena clock, keeps an empty latest-group tombstone to
prevent an older order from returning, and preserves character/persona notice
scope. Existing world-clock watches retain their policy. The record grants no
authority and changes no combat hash, public view or order wire. Thirteen new
native, bridge and actual-boot tests plus independent shape capture ran in N60.
Eleven of the twelve native/bridge tests pass. The 300-stock-hull test stopped
because its fixture constructed overlapping hulls before placement; the actual
boot test stopped because its lobby had not requested the second fill squad.
The next candidate corrects those setups without removing capacity or recovery
assertions. Cross-crash delivery remains subject to the acknowledged snapshot
rollback boundary, not a new exactly-once guarantee.

N60 tree `ba4502a433e00ab0c1c599859789341c0b5e572d` compiles and records
**51 passed, 15 failed, 0 ignored** in its focused diagnostic. Core output (11),
Economy update (6), owner attestation (6), attested update (6), leader record (1),
self-charge expiry (1) and state-owner contract (5) pass. Sim reaches 1/4 and
Industry 3/12; remaining cases stopped on pending bootstrap writes at the real
commit boundary. The fixture repairs fold completed boot effects before the
action, leaving the production dirty-world and legacy-write fences intact.

The format-16 diagnostic deliberately retained the old shape pin. Its actual
walk has **712 paths**, SHA-256
`502a2e44b3572828e4ffbff518604fb8f31975f0210b20376dfe41899ece2d58`.
Independent review found exactly 19 added `state.order_watches` paths; removing
those reproduces all 693 old paths and their exact digest. The next candidate
pins this observed shape; this does not claim coverage of unpopulated variants.

N60's 14-file archive verifies unchanged source, SHA-256
`5c58466c02858dfa56f45a3da0c778fec171659ab4583d0ba6e4e6d9ea71e274`
(22,035 bytes); status SHA-256
`b3e6be45e6ad11151b9fc97c21deedfad93dc1f905f45870f8093cbc27a58061`.
The full runtime gate and both genuine purchase stories were outside this
diagnostic. No M7 acceptance or deployment follows from these counts.

N61 tree `899d3b5e3d518af1a43e54619b12b11b8bd701e6` compiles and records
**1,512 passed, 5 failed, 2 ignored**. All twelve R4 native/bridge recovery
checks pass; both leader presentation checks and the actual bot lapse producer
pass. Sim IC custody (4), Industry (18, including all 12 Origin checks), Core
(585), Economy (158), Roll (204) and Market (80) pass.

Four failures are stale exact version-15 checks after the reviewed format-16
change: two deliberation cases, one conditions snapshot and old-version replay
refusal. The repair pins 16 while retaining all hash/behavior assertions and
explicit predecessor refusal. The fifth failure is the R4 actual-boot fixture:
its legacy partial restart reuses an already-hydrated BulkService. The repair
uses the actual fresh production composition and complete normalized committed
image, preserving the accepted Character/Persona attribution assertions.

N61's 39-file archive verifies unchanged source, SHA-256
`5379e4d53c68007b36e5d21d6d91254fa9e51b91810c24cbdcc4cebb78b4d0af`
(71,634 bytes); status SHA-256
`7439f225585a8ffda867ccd1722e0cac77cc5eb6605e435380edc34a64933721`.
Both genuine purchase stories remained unstarted because prerequisite gates
failed. N61 does not accept M7. N62 includes reviewed exact input reservation,
four-owner IC runtime composition, scoped contract-fact consent and these
fixture repairs. It also imports fetched main `e962b662ec22843f8883c711b4c2e386941aea10`
(A.4b emergency bids and A.7 moons); the implementation branch's merge ancestry
will be recorded at its next checkpoint. These additions still need runtime
evidence; the actual persistent IC region driver remains a named dependency.

N62 (`289bf58dd69031668e7653f9209b77f5507b0265`), N63
(`313ae13434ead458e70f657fd8fbb71d7134e710`) and N64
(`a9911e8c77c723428a90d72a2eae5550828be5d4`) stopped at compilation,
with zero runtime tests and 43/44/44 downstream stages unstarted respectively.
The repairs correct owner registration visibility and fixture API mismatches,
including the existing ModuleCtx/Ctx alias, physical unit query routes and
lossless DomainId conversion. No production predicate or assertion was relaxed.
N63 also adds the opt-in actual Registry recall action, private holder notices
and genuine-story branch; that behavior still needs runtime evidence.
All three terminal archives were hash-verified and reconciled against unchanged
source: N62 `9867bf53ef3753055d8e2afb27a09d42a79cdd1a9c871ebd3b99a4d53786d55b`
(4,245 bytes), N63 `6bdf639d8786c7f40a476a58cfe77c15cbeb04162a7ae31f6c803cff94821195`
(3,471 bytes), and N64 `e4ef0b5b404697a50e4046e13d7c9f42b394cfee7500eaedfd560d7201c44f39`
(5,034 bytes).

N65 (`dbec12c467be59ddd5e9fc65a01a6fe6c70a6871`) compiled and ran 43
stages: 2,075 tests passed, eight failed and two were ignored. Its unchanged-source
44-file archive is SHA-256
`82f2013d8b76822772eae6fb47abb69114eee14f53fa45a3e8791b600fd014ef`
(71,835 bytes). The three bot failures exposed feature-dependent JSON number
spelling in the A.7 lane pack. The first attempted repair incorrectly rounded
four moon coordinates and broke the published digest in N69. N70 restores the
exact upstream values and generator, spelling small exponent values as decimal
text. Decimal equality against upstream and independent canonical hashing retain
published digest `11fb2c9228799918184cc872a01d683e85de91e830104f170b966c7311f23541`;
the real production-loader and bot-composition checks must pass again. No Core
canonical codec or previously published identity is changed. Four recall authority fixtures
and one composition test exposed missing exact definition-validator registration;
that registration is repaired without changing the authority predicates. The IC
runtime stage selected zero tests because its filter used the file name instead
of the Rust module name; N66 corrects the filter and still requires all three.
Both genuine purchase stories remained unstarted. L resume/watch, cold resume,
leader presentation, deliberation, physical custody and OOC compatibility checks
passed on N65; these results do not accept M7.

N66 adds the reviewed term-scoped Registry revocation, public signed-selector
repairs, strict measured-nonconformance artifact codec and the additive format2
IC ownership parser. The latter reserves no new authority: an owned Open may
capture an exact retained charter transfer; old Opens keep their original format.
The full charter producer and measured-failure publication are still separate
implementation work. Actual scoped personal-term lapse facts are queued with
strict private contract consent and committed occurrences. These additions
require their own compilation, runtime and genuine-story evidence.

N66 (`4004a453a89adc0b08a1c6f30e747c5d60e6c310`) stopped at compilation:
nine fixture accesses to private initial-filing fields, with no runtime tests.
The unchanged-source terminal archive is SHA-256
`d1106393d32f24c9a76b4ffc3c962a9a9a3bdf523f732219dbba1e3fda6f3552`
(4,979 bytes, two files). The fixture repairs preserve strict typed decoding
and use checked public serialization, without widening production visibility.
N67 also queues a dependent generation-two source-access correction and the
actual measured-rejection worker/publication path. Rejection remains distinct
from a successful certificate and from infrastructure failure, with an additive
admission marker preserving older quota bytes and capacity promises. Automatic
reconformance, current-certificate selection and resulting recall remain separate
work. No M7 acceptance is claimed.

N67 (`3a9cf2b1f9bca7779e72eb5022da14db881c1657`) stopped at compilation
on two test-only issues: an inferred collection needed its explicit vector type,
and the server's bounded phase logger needed the new Nonconforming enum arm.
Its unchanged-source archive is SHA-256
`7190377e2731451a62c3398e9c026d776f5fd004e80272b6cbb114d6779745ad`
(5,428 bytes, two files); no runtime tests ran. Both narrow repairs preserve
the original assertions and production authority. N68 also includes the reviewed
fixed Industry completion-capacity prerequisite with eight structural tests;
actual factory orders and payments remain separate integration work.

N68 (`a9e9d0cf74024d564eb23d74b79ac24080764ebf`) stopped at compilation
on the new Industry capacity module's private helper scopes and canonical-byte
accessor. Its unchanged-source archive is SHA-256
`87da83926a8c0d2b2fded21d7ea6224bad7425237a8e589b4effb4487f6f4180`
(3,314 bytes, two files); no runtime tests ran. The repair exposes four helpers
only to their enclosing coordinator module and uses the existing byte accessor.
N69 also adds the inactive closed reconformance coordinator profile: exact
Registry service actions, the same 123 future result targets, admission-only
head publication and linear retry-history audit. Five structural coordinator
tests preserve old declaration bytes and cover actual transaction/hydration
boundaries. Registry source access, service submission, certification selection
and automatic recall remain separate producer integration work.

N69 (`8b866937db843ee4e4b94fe16d58869e15d36e0f`) compiled and completed
45 stages: **1,993 passed, 133 failed, 2 ignored**. Its 46-file unchanged-source
archive is SHA-256
`dbaa388b8f537ee2a8963f4500d5a1f5d78022b324b1fe53304b11923a1b68c0`
(83,725 bytes), verified and reconciled. Both genuine purchase stories remained
unstarted after prerequisite failures. The eight Industry-capacity and five
reconformance-capacity tests passed, as did Inventory (103), Economy E4 (340),
Market (80), Economy unit (159), IC runtime composition (3), OOC seal compatibility
(15), twelve L resume/watch tests, two leader presentation tests and the real bot
lapse producer.

One negative Core fixture needs to observe the intended early refusal rather
than later prepare failure; its repair preserves production behavior. The actual
measured rejection worker fails at settlement, with bounded phase instrumentation
queued for diagnosis. Three Roll reader failures exposed missing key-validator
rules for the revocation operation and exact licence/term families: even an absent
sanction could not be read. N71 adds only those canonical key grammars and an
actual registered-family empty-read/insert/readback regression, retaining malformed
key rejection and transaction poisoning. The other 128 failures occur in the
lane-pack-dependent gates affected by the incorrect rounding repair above.

N70 was frozen at `1989f01a31f3c3dbf7d6a807a89bf80797a27f81`, committed in
`79c5097b`, then merged normally with main at `03ce517e` and pushed. It never ran:
the newly identified revocation key bug is included in the superseding N71 input,
which starts from the actual last tested N69 source. N70's queued changes below
are therefore tested as part of N71, without claiming a nonexistent N70 result.
M7 acceptance remains open.

N70 includes reviewed actual immediate personal charter sales: signed seller
consent, actual buyer payment and native title transfer in the same acknowledged
union; immutable ownership history; current-owner attestations and compatible
updates; old-title IC claim completion; and complete cold inverse audit. Seven
native and five Roll tests are queued. Sales with open claims currently refuse;
paid pending transfer, timed settlement and refunds remain implementation work.

N70 also includes reviewed Core factory integration: actual scoped supplier
consent, named buyer acceptance, exact certified die/facility holds, native cash
reservation, bounded real K schedule progress, governed output birth/title and
stable cancellation/refund. Ten actual Engine factory tests and three schedule
replay tests are queued. Production recipe activation, positive Class III and
customer-input acceptance, delivery and Install/Replace remain outstanding.
These additions are not represented as runtime passes.

Fetched main `4465bf06abf924b224cd63667a41c5723211ac3b` adds A.3b's chart panel,
TUI screen and web rendering. Its source is included in N70; the implementation
branch records normal merge ancestry at the checkpoint. The chart-screen and
existing travel/chart checks run with M7. Registry reconformance lifecycle,
automatic measured recall, organization catalogue/payment, pending resale and
Safe Conduct production services still require implementation and acceptance.

The certified Origin producer passes its twelve actual Engine checks in N61:
actual Inventory mint, same-commit certificate and shared civil-month quota,
scoped Org/E4 authority, an independent permanent Industry issuance index and
complete cold reconciliation. Twelve Engine checks cover genuine initialization
followed by an Active restart, real authority/custody boundaries and history
corruption. Initialize cannot issue stock. No importer or hardware producer is
enabled by default. Factory orders, payment/title and consumption are now queued
for N70 validation. Production activation, delivery and installation are still required.


N71 through N75 stopped at compilation and executed zero runtime tests.
Their repairs preserve the existing checks and correct actual owner API,
registration and fixture module boundaries. N76 compiled successfully on exact
tree `2d15c68e499ed1e755da226cef0740e743a7b5f4` (335 reviewed packages), then
completed its runtime roster with unchanged source. Its evidence archive is
`ae10dcb2894067fd5c29732e531187b7810688f489f5fb7305ebfb1fea8ad0f9`.

N76 passed all 244 Roll tests, 80 Market tests, 340 selected Economy E4 tests,
103 Inventory tests, 15 commerce-composition tests and 27 commerce-roster tests.
Its focused L leader, admission, release, deliberation, resume/watch, presentation
and expiry producer checks passed, as did the actual IC custody/composition,
wire, snapshot, shipped-configuration and state-owner contract checks. The Roll
runner itself failed because four exact prefix counts were stale after adding
the ownership-capture tests; those counts are corrected in the successor.

N76 still failed seven Core delivery-capacity tests, five Industry integration
tests, the measured-rejection worker and eleven Economy pending-transfer tests.
Bounded diagnostics isolated the closed delivery guard, scheduled readiness,
terminal quota charge and missing immediate-transfer fixture provider respectively.
Reviewed repairs are included in N78; they require fresh runtime verification.
N77 was frozen but never launched and was superseded by N78 after the last two
N76 findings. No N77 result is claimed.

N78, exact tree `e944b3b8dda208f9178a555093f30f547d512ae3` with 347 reviewed
packages, stopped at compilation and executed zero runtime tests. The compiler
found two Core test API mismatches and a missing serialization implementation
on the House customer read result. The terminal archive verifies unchanged source:
`6be282c15d0419d5346d7975891c9d65b1ebd8ab9d4b6438f3cf8c6169e86c27`.
The narrow fixes are prepared for the next candidate. These are progress
checkpoints, not M7 acceptance or public paid-service activation.

Current main `f3df3205` was merged normally at `31c3e561`; another fetch on
2026-10-19 found no newer main commit. N78 includes that exact main ancestry.

The re-conformance implementation preserves the filed version, uses actual paid
Registry work, and selects a current certificate only after successful completion.
Measured nonconformance publishes permanent evidence and a separate automatic
recall. Its genuine purchase/re-conformance story and the repaired terminal quota
path still require combined execution.

The house catalogue uses actual organization and scope identities and scoped
publishing authority. House policy preserves listings after curator departure;
personal customers pay for their own licence. Reviewed House checkout now connects
actual Market, Economy and Roll owners, with Org treasury recipients and personal
payers. Its disclosed future renewal terms may differ from the initial term.
House renewal and default catalogue activation remain in progress; checkout's
prepared tests have not yet run successfully in the combined candidate.

Reviewed Industry delivery now uses the actual paid manufacture order, original
capacity reservation, real title and custody checks, and Core's prebuilt carry
images and permanent receipt. Core Install/Replace mechanics also preserve exact
ordinary object images with two native owner images. Actual refit consumption
and production activation remain required. Pending charter funding now connects
the native Economy owner, four-target Core completion recipe and actual Roll
service, including last-close/deadline settlement and compound Registry refund.
Its combined runtime and genuine-story checks have not yet passed.
Personal renewal terminal holds are retained through the actual garbage collector
because cold recovery requires their native evidence. New boundary tests for this
retention, full WorkOrderId values and pending-transfer capacity are queued.

Further verification through N81 is retained as **compile-only failure evidence**,
not runtime acceptance. All three immutable archives were downloaded, hash-checked
against every log and the input/source census, and confirmed source-unchanged:

| Run | Exact tree | Terminal result | Evidence archive SHA-256 |
| --- | --- | --- | --- |
| N79 | `aaa731af4cd1a7e0719e4edb6fa3b911aaf447b6` | One House account iterator lifetime error; subsequently repaired with a retained principal binding | `136bbce1fc874ed36c2938aaa78b52860a8c5bbcc21433b8a03e8ec8afaf8aca` |
| N80 | `9297d2e33ed8e4b1f2eb0391e820fde16e125584` | Four Tacsim fixture import/path/helper-shadow errors; subsequently repaired without changing assertions | `5ad9a21de05aae4eef9d2f21caeab42ce8baf54bd3db888d13ee31ac2c08d268` |
| N81 | `233abcbeceed1855f31bdccbc39b60e83cb12c58` | Two House renewal Core fixture visibility/owned-registry clone errors; reviewed repairs await the next run | `eee121da8147f0dcd1e56428d035fccab210530c5f88783102f53aa4f22331b6` |

Reviewed additions reserve seven exact House renewal targets (81,920 bytes) with
personal/House coexistence and independent service closure. The pure Safe Conduct
financial layer now has strict versioned terms, checked minor-unit arithmetic and
same-generation WorldIc term/claim-delay calculations. Its dual price illustration
is conditional; it is not an eligibility proof, quote acceptance, premium, policy,
liability reservation or claim service. The five financial/window tests pass in N82; this is arithmetic evidence, not a bound policy.
Paid-resale IC capture has a separate exact format 3 carrying only paid ownership;
format 2 remains charter ownership and format 1 remains unchanged. This Core
shape and its prepared tests do not substitute for actual resale issuance/history.
The full House renewal owner path has now passed its assigned source reviews and
is integrated for the next runtime candidate. Paid resale and actual Industry
refit remain under implementation/review. None of N79–N81 ran runtime stages.

Latest fetched main `b0d036d324b987ae280dfe9b8d388b5cc71f4133` merged cleanly
as `1457eba89c0f97c4281ccb45566969ded472c7bb`: doc 19 A8a's versioned goods,
parent-frame moon passage and throttle bisection updates remain in the M7 tree.
N82's immutable tree `313987d7848c00e79107d92aa37f534b0d87ee2b` includes that
merge and the reviewed charter PendingTransfer term-window correction. The
correction uses Roll's actual bounded interval rules: issue during Grace is
valid history and never shifts any date or revives current admission. N82
compilation and five underwriting arithmetic/window checks pass; its remaining
runtime gates are still running at this checkpoint. This is not final evidence.

Subsequent reviewed source packages, not included in N82, add:

- The actual House renewal E4/Roll/Market providers, native contract/hold inverse
  checks, historical cleanup and optional service lifecycle, retaining distinct
  captured purchase terms and current renewal terms. Its genuine child uses the
  real paid House purchase and exercises curator departure, retry, service close,
  terminal hold retention, restart and captured IC checkout. Eight Roll, eight
  Market and two composition tests plus the genuine continuation await execution.
- A bounded immutable underwriting product payload binding real Org/scope/account
  identifier domains, adjudicator, complete financial terms, exact hull/zone
  definitions and exact charter/line/filed revision/certificate/role-trial/profile
  references. It requires an explicit recorded-IC-destruction loss rule, excludes
  OOC elimination and journey anomalies, and requires same-transition wreck title
  on payout. This payload still grants no publication, eligibility, money or host
  authority. Five structural tests await execution. The actual CREDIT example
  uses its two-decimal scale: 10,000cr insured value, 100cr ordinary premium,
  conditional 90cr qualified premium and 8,000cr reserved maximum payout.

Underwriting Bind is a separate E4/Roll transition after acknowledged four-owner
IC Open and before a genuine host releases covered departure. Its private host
permit must be retained through Unknown; unavailable host cannot charge. Cover
starts at Bind, never retroactively at Open. Full funded binding, immutable
loss/close evaluation, delayed payout and liability release remain implementation
obligations; the product/math packages do not satisfy those gates.

The L R4 release-notice correction passes in N56: a side lead's fully offered
whole-squad RELEASE also removes the persona commander's watch, preventing a
false expiry announcement. A partial offer or wingman's release preserves the
squad-wide watch. The public BOARD/reply/RELEASE/TTL regression is included;
that regression passes in N56. The later format-16 accepted-watch implementation
repairs resume attribution: N65 passes twelve focused resume/watch tests and the
actual match-boot test with character and persona authors. Watches remain
record-only, outside MindView and combat hashes; empty latest groups prevent an
older held order from regaining its watch.

The L residual audit also confirms and repairs 17b's R1: direct character ORDER
admission now binds `issuer` to the actual occupied hull and applies the same
published wreck predicate as typed cockpit ORDER. The existing actual boarded
lead/wingman regression adds missing/borrowed issuer and egress-wreck refusals,
preserving its valid controls and Persona rules. Its actual boarded regression
passes in N50 and N56. The implementation rechecks commander fitness when a delayed
persona reply lands, tests wrecked/disabled/inert commanders, and replaces five
older manually seated fixtures with actual public BOARD. The new no-commander
snapshot notice requires snapshot format 15; old formats refuse explicitly.
Those additional checks pass in N56, but do not close every historical L residual.

The five real renewal providers and genuine renewal story are integrated, with
new service cleanup/due/status/control work continuing separately. Reviewed
prerequisites add the bounded 32-service registration ceiling (per-invocation
budgets unchanged), a closed Tactics component replacement owner adapter, and
the separately registered sixth unfunded-renewal cleanup action. Personal-author
withdrawal adds current title/readiness fences and permanent cause/root audits;
its genuine child preserves paid Pins and captured sorties. These new source
packages still require their combined runtime gates and are not default service
activation. No existing paid term is truncated to start a renewal early.

Renewal due processing and cleanup/control, owner attestation, compatible
updates and author withdrawal now have implementations and focused evidence;
their genuine combined commerce stories and final acceptance remain required.
Registry recall/revocation and production IC claim/close are implemented with
focused checks; their combined genuine-story and final acceptance gates remain.
Automatic re-conformance, current certificate selection and the scoped house
catalogue reader now have reviewed implementations awaiting their combined
runtime evidence. House renewal now awaits its combined runtime and genuine-story
evidence. Remaining implementation includes paid licence resale and underwriting,
actual governed refit and
production activation. Reviewed checkout and delivery need their combined runtime
and genuine-story acceptance.
The acknowledged Core output reservation and certified Origin producer are
prerequisites, not completed manufacturing or usable installed hardware.

These are M7 obligations. Only integration with an unavailable live IC sortie
or convoy driver is conditional; its production services remain required.
M8's contraband and NPC procurement programme remains separate. Current parallel
work covers paid-licence holder transfer and Industry refit, alongside
underwriting and combined House renewal verification. The absent region caller does not waive its claim, capacity
or custody owner services. Scoped fact consumers, generated client bindings,
operator surfaces and final complete acceptance still follow their actual
implementation. Existing partial passes and source review do not substitute
for those gates.
The detailed cuts below state the complete intended result. Existing inert
components can land with focused evidence; public paid services require their
own complete phase gates. Hardware, renewal/transfer, the house catalogue and
full acceptance are still implementation work. No Windows Cargo build is needed
for the current loop: Linux uses one exclusive build lane with two Cargo jobs.

## 1. Result and boundaries

M7 delivers a usable IC mind market: Registry conformance and rating; private
filing and public listings; titled charters and compatible lines; owner and paid
licences; pin/follow; renewal, Grace and lapse; sealed/open/open-resale forms;
royalties; resale and charter transfer; current holdings in the OOC simulator;
and titled governed cores manufactured from certified dies and fitted by real
refit work orders. The buyer must be able to buy, fly, tune and resell without
source disclosure or an unpaid/duplicated right.

M6 remains the character workflow owner. Saved source, schema, cards, rules,
drafts, checks, proving, bench and transports are extended, not replaced. New
commercial legality belongs to typed state families from birth. Ordinary
workflow JSON is not an exemption for licences, filings or sortie seats.

M8 remains separate: NPC procurement, ungoverning, greys/heirlooms, testimony and
recognition claims, black-box crime detection, seizure/fines and its zero-PC
gate. M7 must provide Registry recall/revocation and governed renewal admission,
but does not quietly enable contraband. Frontier never mounts in a hull or
authors an act; there is no new Command class.

OOC matches, proving and bench consume no IC money or seat and do not advance IC
terms. They may read committed current IC holdings at admission. Running arenas
read frozen seals, never a market, licence table or wall clock. Actual IC sortie
integration is required if that subsystem exists at implementation; otherwise
M7 must ship the real production seal/close service and exercise it through
production composition, explicitly recording the unavailable region boundary.
A test-only seat counter is not that service.

## 2. Decisions carried into implementation

### 2.1 Owner rulings and written default

**Q-UM-17 is an explicit owner ruling for the dependent owner's licence:** loss
of the current derivative charter owner's qualifying immediate-parent grant
blocks that owner's new derivative filings and owner-licence sortie seals
immediately. Existing frozen sorties finish unchanged. That owner's derivative
and its working copies remain source-sealed until qualifying parent rights are
restored; immutable source and lineage are not deleted. Loss includes lapse,
transfer away, revocation and recall as applicable to that qualifying grant and
exact parent revision. Active/Grace rights remain eligible where doc 17 permits
them; Grace ending is lapse, not an extra unpublished extension.

**Paid-customer clarification, explicit user ruling, 2026-10-17:** "Keep existing
paid licences usable through their own term; block new sales and renewals until
parent rights return." Existing paid derivative admission checks that customer's
own term, form and applicable Registry status, not the issuer's current parent
licence. New derivative sales and renewals must recheck the issuer's qualifying
parent rights before commit. Existing sealed sorties finish. Buyers never need
separate ancestor licences. No term, Grace or renewal date is extended by loss
or restoration of parent rights. This clarification preserves the dependent
owner-use/filing/source restriction above; it does not reopen Q17.

Immutable ancestry/royalties, an applicable Registry recall and the requester's
own grant remain separately named checks. There is no universal live-ancestor-
licence test hidden in the word "dependency"; the operation and licence kind
select the exact applicable check (§10).

**Q-UM-18 uses the written default under continuation authorization:** at most
one pending transfer; neither party admits a new seal while pending; settle on
last admitted sortie close or `min(term_end, accepted_sale + 3 IC days)`.
Outstanding sorties retain their old warrant and held seats after settlement.
The buyer receives only the unheld capacity. Term-end settlement delivers Grace
and a buyer-bound renewal; no seller renewal is drafted while pending.
Registry revocation cancels/refunds; recall follows the instrument with notice.
This is not relabelled as an explicit user ruling.

**Coprates Drill, explicit owner ruling, 2026-10-18:** launch standing means
current Republic membership through the existing standing service. No officer
rank or reputation score is required. Check actual admission/commerce authority
and membership loss after an eligible preview, not merely catalogue visibility.

### 2.2 Selected implementation defaults

* Use typed Roll licence authority plus the prepared participant commit in §4.
  Do not introduce title-backed licence objects as a competing holder source.
  Economy title alone owns a charter; the owner's licence derives its holder
  from that title rather than persisting another authoritative owner field.
* Filing and listing are separate. Filing gives a public Roll entry with forms
  `none` and a free, non-transferable owner's licence. `/publish` may file first
  and then list through an explicit combined service; it cannot charge twice.
  Private filing never makes a mind eligible for future NPC procurement.
* Terms are at most 90 IC days; Grace is at most 3. Renewal auto-drafts one fresh
  E4 contract per paid term, not automatic acceptance/debit. Owner renewal is
  attestation without a synthetic zero-price sale, levy or venue cut.
  A buyer may explicitly request a fresh offer after the exact preceding offer
  closes or refunds, including after parent rights return. Retain linked attempts
  and recheck eligibility; no automatic retry loop or history rewrite is allowed.
* An owner's free capacity is `OwnedDispatch`, not the paid offer's seat count:
  it covers the holder's owned hulls dispatched under current authority. Each
  actual `(unit, sortie)` still consumes one bounded open-claim record. Multiple
  owned hulls may dispatch concurrently; storage limits may refuse new work
  before admission but cannot redefine the licence as a one-seat purchased grant.
* A dependent owner's `term_end` never exceeds the qualifying immediate-parent
  grant's `term_end`; its `grace_end` never exceeds either the parent's
  `grace_end` or its own term end plus configured Grace. Apply this at initial
  filing, owner attestation, replacement qualifying grant and charter-holder
  change. If the parent is already in Grace, permitted filing creates a child
  in the remaining bounded Grace interval, not a fresh full-duration Active term;
  record its real issue time and inherited term boundary without fabricating an
  earlier filing or Active interval. No remaining eligible interval means
  refusal. Parent renewal/restoration never
  extends the child automatically: a fresh valid child attestation is required.
* Primary and renewal proceeds: Registry levy (seed 50‰), optional venue,
  parent chain, charter-title owner's remainder. Secondary licence resale uses
  the same disclosed cuts but pays the remainder to the selling licence holder.
  A separately priced charter title sale does not silently acquire mind-use
  royalties. Aggregate parent royalties are capped at 300‰ across depth ≤3,
  not 300‰ independently for every ancestor.
* Compute all non-remainder shares from gross using checked integer arithmetic
  and floor rounding; merge duplicate accounts, omit zero shares, assign the
  whole remainder once. Use E4 Exact shares, not its remainder-relative Permille
  semantics. At most Registry + venue + three ancestors + final recipient are
  distinct payees. Reject overflow, excessive cuts and invalid accounts before
  money/approval/escrow publication.
* Freeze price, payees, royalty terms, title revisions and effect identity in
  consent. A changed required title/payee binding before durable settlement
  refuses/requotes or refunds; it never redirects an accepted payout. After
  durable intent ownership, replay completes those exact images.
* Withdrawal excludes a revision from new offers and follow resolution;
  existing licensed pins and historical seals remain. Only Registry recall
  disables that revision for otherwise lawful new pinned seals. No author-side
  withdrawal is a hidden revocation power.
* A compatible qualifying grant held by the current charter owner to the same
  immediate-parent line can restore that owner's dependency; it does not erase
  exact source ancestry, clear an applicable recall, extend the child's term or
  rewrite an old seal. Traverse immutable ancestry for bounded royalties and
  integrity, not to demand a separate paid licence to every ancestor. A new
  holder's shorter qualifying term re-bounds owner eligibility immediately;
  a longer one does not lengthen the already-attested child term.
* Charter transfer preserves old frozen sorties and applies the same pending
  discipline to affected owner-licence admission. Use a charter-level pending
  record/claim aggregate, not one unbounded transaction rewriting every line.
  Generic title transfer/collateral routes must honor this policy too.
* Preserve genuinely separate pre-filing personal drafts under the OOC draft
  exception. Server-known filed/derived copies retain provenance through copy,
  save, rename, removal and source replacement. Do not infer derivation from
  behavioral similarity or offer a similarity oracle.
* Initial hardware accepts exact quantity-one certified Origin die lots.
  Generic grade/crystallization never creates a die certificate. Governed
  refits use finite content-authored compatible fitted-unit definitions, retain
  unrelated damage/ammo, and never call simulator full `refit_unit` as a repair.

These defaults are selected design choices within the authorized task. A later
departure must be recorded before its implementation and must not be presented
as a previous owner ruling.

### 2.3 Chosen provisional launch tuning

The percentages, 90-day/3-day limits, 50-permille Compact levy, Class III
restriction and Q17 policy come from doc 17. The following **new configurable
launch values are implementation choices**, not discovered prices or owner
rulings. Author them in a versioned commerce-content record with strict units
and bounds; record its digest in quotes/receipts. Missing content refuses
activation rather than silently falling back to free goods.

| Setting | Selected initial value and semantics |
| --- | --- |
| Line-tank catalogue anchor | 100,000cr for `tacsim.unit.line_tank`, a new explicit economic catalogue row. No current tank price exists. The existing 120,000cr Dray prose price and 90,000cr Co-op buyout are unrelated. |
| Finished replacement core references | I 3,000cr, II 8,000cr, III 25,000cr: exactly 3/8/25% of the tank anchor. The reference prices are for the completed core, with refit separately disclosed. |
| Die/integration split | Certified die 90% and paid integration 10% of that core reference. A customer-supplied die is not charged twice. A supplied finished core follows the ordinary asset sale path. |
| Installation service | 100cr per accepted Install/Replace work order, separate from the supplied asset. A failed/cancelled undelivered service refunds under its frozen terms. |
| Registry filing | 100cr for each newly filed revision; publish of an already filed exact revision charges no filing fee again. NPC houses use the same fee. |
| Paid launch house licence | 400cr, sealed, one seat, 90 IC days; named content may choose a different explicit form. Renewal for the same term initially costs the same 400cr, frozen in the accepted licence's renewal terms. A later price edit needs new consent, not a hidden renewal debit. |
| Renewal offer interval | Three IC days (259,200 seconds), configurable from 1 to 259,200. This is the disclosed offer interval, separate from predecessor Grace. Automatic drafting selects the unique actual personal account for the accepted instrument; absent or ambiguous accounts require buyer choice before an offer is created. |
| Paid-seat representation ceiling | `MAX_PAID_SEATS = 1,024` per paid instrument in the prepared M7.1a value codec. This is a selected implementation ceiling, not the launch seat count, an open-claim storage budget or an owner's dispatch limit. Later guarded capacity reservations may refuse new work below it. |
| Venue fee | Direct author listing 0; a house-vendor listing 50 permille. This is additional to the Compact's 50, not the Co-op's 250-permille proceeds share. Parent royalty remains an explicit listing term within the aggregate cap. New offers must explicitly freeze their downstream royalty rate, including zero. For existing v2 OpenResale grants that have no rate field, new versioned resale consent will capture an immutable 0-permille parent edge; old grant/proof bytes stay unchanged and no later listing may add a fee. This selected compatibility rule is preparation, not an implemented producer. |
| Importer allotment | Basin: I 64, II 16, III 4 exact quantity-one dies per WorldIc month, shared by the Ceres and Phobos agents. No boot, connection or per-agent reset. |
| Free content | Standard Doctrine and its six preset pages remain free. An owner's licence has zero-price attestation, not a synthetic sale. |

Currency is the existing `tacsim.credit`, scale 2 (centicredits). Convert major
content prices with checked multiplication. Core-reference percentage math
floors to centicredits and exposes that rule; these initial values divide
exactly. Use checked minor-unit share arithmetic from §2.2. Bound quotas to
1..=1,024 per class/month, venue share to 0..=1,000 with aggregate validation,
and all prices by existing positive `Amount`/quote limits. Config changes cannot
alter an accepted quote, existing renewal consent or an owned replay bundle.

The initial refit family is finite: existing `line_tank` remains the Class II
form; add explicit I and III fitted definitions differing only in the registered
turret `core` binding/capability. All unrelated weapons, sensors, section
consequences, damage and motion remain. Existing laser/thermal/ECM/ECCM lines
are different chassis variants, all II; scout/drone are I, bipeds II, heavy III.
The target and Dray have no existing mount. Do not generalize installation by
matching a label, weaken boot topology, or add owned hardware to OOC pods.

## 3. Authority, records and bounds

### 3.1 Owners and dependencies

`mush-roll` owns typed commercial records and the rights evaluator. It retains
the existing draft/revision/reference owner and takes no market or tacsim edge.
Use economy/org identifiers only where required; game semantics enter through
registered adapters. `tacsim::minds::registry` supplies rating, core/station/floor
admission, public projection, and composition. `mush-economy` owns accounts,
escrow, title and money. `mush-market` owns new entitlement listings, quotes,
capacity and settlement indexes in typed families, alongside its unchanged
legacy commodity market. Core/server own
the composed durable boundary. Tactics owns unit topology; inventory owns lots;
industry owns work-order capacity, schedule and material lifecycle.
Tacsim owns Safe Conduct role content and the bounded typed underwriting
product/quote/policy/exposure/claim families; these enter the same ActionTxn
owner-delegation discipline, not another mutable after-payment store. They use
the existing Economy and Roll-root participants where money/title/retention
needs them, without exceeding the reviewed rights lane's two legacy owners.

Source stays at the existing immutable tacsim source owner. A Roll/market DTO
never contains source simply because a server-side `HeldMind` does. Hidden org
names continue through the established renderer. Authentication AccountId for
the alt prohibition is distinct from economy AccountId for payment.

### 3.2 Typed commercial model

| Record/family | Essential authority and identity |
| --- | --- |
| Charter, line | Titled world object; bounded compatible-schema line identity; owner derived from E4 title; public name/role/house projection |
| Filed revision | Exact saved RevisionRef/source/definition digests; immutable ancestry; line sequence; withdrawal and Registry recall facts |
| Rating receipt | Exact source/envelope, ABI/engine/class-table/scenario manifests; per-profile worst operations/fees and faults; minimum certified class |
| Licence and term | Stable licence ID; paid holder or title-derived owner kind; warrant/scope; pin/follow; class/form; `PaidSeats(n)` versus `OwnedDispatch`; immutable term generation and WorldIc dates; holder-specific immediate-parent binding and bounded dependent dates where applicable |
| Pending transfer | Unique transfer generation; old licence/title revision; seller/buyer; contract; frozen consent digest; deadline; no new admission |
| Seat claim / open seal | `(unit, sortie)`; exact licence/revision/warrant; captured seal digest; open/closed receipt; survives holder change until real close |
| Listing terms | Exact line/form/seat/term/class/transferability offer, venue and frozen economic terms; no execution authority by itself |
| Entitlement quote / market capacity | Market-owned typed listing/quote identity and counters; frozen terms/effect digest, buyer/seller/contract links; bounded query indexes and reserved terminal capacity |
| Commercial operation/receipt | Permanent operation identity, phase/generation, exact effect and economic result; terminal capacity reserved before funding |
| Hardware certificates | Import quota/receipt, exact die, integration output, core asset and installation binding; distinct from a mind charter |
| Cover policy and claim | Versioned product/quote; authenticated exact IC seal/role proof; insured hull/title/value; premium and reserved liability; term/exclusions; immutable mechanical-loss/close identity and claim/wreck-title receipt |
| Inert shell reservation | Stable operation key to reserved ObjId and immutable object shape; durable acknowledgement precedes filing/manufacturing; no title, certificate or execution authority by reservation alone |
| Deployment markers | Explicit initialized-by commit shared by Roll and core shell service; checked permanent allocation/birth/consumption counts distinguish an empty deployment from lost records |

Active/Grace/Lapsed, Recall, Registry revocation, dependency and PendingTransfer
are independent facts. A dominant display refusal must not discard another
fact. New admission evaluates integrity → current principal/title → sanctions →
dependency → term → pending transfer → exact revision → class/station/floor →
IC seat. The captured seal is independent thereafter.

**Explicit user ruling, 2026-10-19:** Registry revocation covers the current
accepted term, including its original Grace. The immutable sanction names the
exact licence and accepted generation; the old generation remains revoked,
including after clock rollback. At or after that term's original Grace end,
a normal fresh term may be issued through current eligibility and consent
checks. There is no automatic extension or restoration of the old term, and
historical sanction evidence remains. Already sealed sorties still finish.

The dependency step names the operation and licence kind. A dependent owner-use
request checks that current owner's immediate-parent grant. Existing paid
customer admission checks their own derivative grant and applicable Registry
status, without consulting the issuer's live parent grant. New sales/renewals
check the issuer's qualifying parent rights before commit under the explicit
2026-10-17 ruling. Neither case infers ancestor purchases from the royalty chain.
Owner dispatch checks current hull title/dispatch authority;
paid seats enforce the purchased count. Both retain old open claims through
holder changes and use the same bounded idempotent close mechanism.

All families have strict codecs, explicit versions, non-reused IDs, checked
counters, bounded payloads/indexes and guarded transitions. Establish numeric
record/page/operation ceilings in the first model cut against actual encoded
worst cases; use existing action/bundle ceilings and refuse before funding.
Do not increase global limits to fit a large sweep. Large line histories and
follow roots use per-revision records/bounded pages, never one unbounded vector.
Receipt identity cannot be discarded by a rolling marker cache. Reserve terminal
receipt/root capacity with every pending obligation; closing/refunding existing
work must remain possible when new-work capacity is exhausted.

New entitlement market rows live under market-owned typed namespaces, not
mirrors in `MarketStore`. Use existing ActionTxn owner delegation and the
market's typed-family precedent (`lot.rs::DrawReceipt`), with a closed commerce
terms adapter. Market-rooted sale actions receive only the delegated Roll
commercial handles and registered participant handles they need; filing is
Roll-rooted. Composition fixes the root, grants and readiness. No client chooses
an owner or participant. Do not borrow the legacy quote allocator, create a
dummy legacy quote or add an after-ack authoritative mirror. Ordinary market
commands/rendering route by representation to the correct owner; commodity
counter/index behavior remains unchanged.

### 3.3 Retention and source rights

Commercial typed records are rights authority. Conservative external Roll roots
are collection authority. Maintain exact roots in the same commit for filed
history, pins/follow eligibility, listings/contracts, pending transfers/renewals,
lineage and open seals. Existing kept match/tape/proving/bench roots remain.
An owner/title change cannot release a still-open seal. Source-byte GC remains
deferred until every source owner is indexed.

Use distinct capabilities for execution, card read, probe, source read, memory,
fork, private derivative filing, derivative listing and charter administration.
Sealed grants execution/card/probe, never source or private memory. Open adds
source/fork/private filing while eligible; open-resale additionally permits a
listed derivative. Source access does not grant another principal's run memory.

Q17 must cover **owned derivative drafts**, not only filed licence lookup.
Persist immutable server-owned lineage and use one shared access predicate at
Read/Copy/Fork/Edit/Save/Check, SSH, web/TUI, trace, EVAL/WATCH, proving/BENCH and
export. Recheck current actor/connection and rights when queued work completes
and before reconnect delivery. Client envelope/source replacement cannot erase
lineage. Already downloaded bytes cannot be revoked; no server path may disclose
fresh bytes after rights loss. Unknown/missing/foreign source refusals remain
indistinguishable.

### 3.4 Committed facts for contracts and fronts

Register one `minds` FactProvider with these exact M7 kinds. They read the
committed commercial records/receipt projection after acknowledgement and after
boot hydration; they never grant a right or invoke a second settlement callback.

| Kind | Exact subject and temporal meaning |
| --- | --- |
| `minds.filed` | A specific immutable filed-revision receipt. True once that filing commits; withdrawal/recall does not undo the historical filing. The public Roll entry may expose its source-free reference. |
| `minds.licence.lapsed` | A specific licence term generation and its committed lapse occurrence. True once that occurrence commits; later renewal is a different generation and does not erase the event. Current admission still derives the exact WorldIc boundary without waiting for a fact-projection sweep. |
| `minds.licence.transferred` | A specific completed licence-transfer receipt and generation. PendingTransfer is not completion. Old open claims and warrants remain independently retained. |

Bound IDs to the exact record/generation and reject malformed IDs, wrong
revisions, unavailable owners and unknown kinds uniformly. Preserve the event's
actual RealUtc audit time in `FactState.at` (or 0 when unknown), never a WorldIc
term timestamp disguised as unix time; IC dates remain in the referenced record.
The event actor is audit attribution, not a principal authorization shortcut.

Public filing facts expose only the existing public Roll projection. Licence
facts are scoped to the holder/title principals and the explicitly authorized
contract/front participants; they must not reveal another holder's instrument
through guessed IDs. Current `FactProvider::resolve(&World, kind, id, revision)`
has no caller/contract context. Add a narrow opt-in scoped authorization seam:
the actual draft/offer/read/settlement paths supply current authenticated
principal and immutable contract/front scope; the `minds` provider refuses
restricted references without it. Validate condition attachment and disclosure
against that scope, and recheck changed parties/authority before offer and use.
Existing providers retain their existing behavior. Do not substitute a
client-supplied actor, an unguessable-looking ID, or a generic source-bearing
fact result for this boundary. The pure engine still receives only authorized
fact truth; the read bridge does not become a mutation owner.

`minds.seized`, `minds.unwarranted` and `minds.claimed` belong to M8 and remain
unsupported in M7. Inventory the three registered M7 kinds, provider namespace,
allowed source-free fields and scope consumers alongside commands/state owners.

## 4. One durable business transition

### 4.1 Prepared state owners and the authoritative receipt

The selected seam is detailed in `target/m7-composed-participant-sketch.md`,
subject to the independent review's corrections adopted here. This is a closed
prepared-state interface, not an arbitrary plugin callback:

1. A registered typed `ParticipantHandle<Request,Output>` is bound to one
   composition, provider and owned module slot. Owner-private preparation returns
   a non-cloneable patch and exact bounded replay images. The calling provider
   gets only prepared result facts. No client supplies writes or owner identity.
2. `PreparedComposedTxn` contains the normal typed transaction and, for rights,
   Economy and Roll retention owner images. Registry completion needs three
   distinct installed slots: Roll retention, Tacsim BulkStore and Tacsim Artifacts.
   The prepared core bound is four registered slots, at most three per invocation
   and at most two distinct module owners. These are fixed composition bounds;
   requests cannot select or expand them.
   New market/rights records are in that same typed transaction and consume no
   additional legacy slot. Hardware has its separate bounded image matrix below.
   A registered read-only preparation context supplies owner state/authority;
   current action providers do not otherwise receive World. No Ctx, outbox,
   live mutation or I/O is available during preparation.
3. Canonicalize the complete union once; reject duplicate physical targets,
   including LedgerChunkSet/JSON aliases, and retain exact equality with the
   authoritative bundle. Preserve dirty-world/module-op and predecessor checks.
4. `StagedAction` transfers one `ComposedInstallation` into `PendingCommit`.
   The writer sees the bundle only. Exact token/hash final acknowledgement is
   the only live installation boundary; ownership acknowledgement is not success.
5. Bind typed installation and all distinct owner slots before the first write.
   Validate composition, installed-slot incarnation, complete pre-state fences
   and generations. Bound installers mutate only their held state/fields and
   cannot fail, serialize, call another owner or queue persistence. The consumed
   erased capsule moves its patch and any immutable `Arc` adapter/config into
   `BoundOwnerInstall<'a>`; the returned installer borrows only the supplied
   owner state/fields, never the destroyed capsule or a temporary `&self`.
   Keep the typed reservation guard outstanding until the last apply returns;
   only then settle it and release notification-only output.
6. Before custody, stale/refused work changes nothing. After an indeterminate
   durable result, freeze; do not replan or undo money locally. Restart first
   replays retained exact images, then hydrates typed/Economy/Roll state and
   audits references. No retained Rust callback or fresh commercial admission
   runs during replay.

Core changes are real scope: `action_state`, participant registration,
`ModuleStateMap` identity/disjoint binding, engine staging/install, coordinator
assembly and world-task pending receipt. Reuse ReplayWrite and store recovery;
no new persistence format is justified merely to carry an in-memory callback.
Factor E4's complete prepared delta/image construction and Roll/Ledger
check/bind/apply helpers. Ledger tail checks happen before any apply; E4 approval,
meta/key/index facts and serialized writes are prepared before any RAM change.
Current `install_commit`, `apply_prepared`, typed `install` (which settles its
guard immediately), and Roll `publish_with` are not already this bridge.
Installed-slot incarnation changes on replacement and clone, not just owner
generation, and is process-local rather than a persisted Rust TypeId.

Every effect-bearing contract uses closed `Accept/Activate/Settle/Cancel/Expire/
Forfeit` phases. Partial acceptance grants no execution right. Zero money still
commits contract/typed phase/root images; it may have no economic transaction ID.
Do not bypass the seam through direct accept saves, empty-refund saves, pending
unwind saves or a settlement-then-licence callback. Wire commands and internal
sweeps before funding these contracts. Ordinary E4 contracts retain their path.

### 4.2 Notifications cannot invoke legacy settlement mutation

Every authority-bearing payment, right, market capacity/quote/index and retention
effect is prepared before submission. New composed effects **do not dispatch
the existing EconomyObserver list after acknowledgement**. That list mutates:
MarketObserver stamps quotes/delivers/settles IC10 proceeds; CanonEvidenceBridge
commits Canon and an Economy marker; CoopObserver persists enrolment changes.
The receipt path's no-write/debug assertion is not an atomicity mechanism.

Emit prepared notification-only facts through the coordinator's durable receipt
or bounded outbox. Persistent derived Canon/inbox projections use explicit
idempotent follow-up actions keyed to the retained committed cause and retried
until acknowledged. They cannot be the action that grants a licence, marks
payment successful, consumes capacity or roots paid content. Reserve bounded
outbox/terminal capacity before funding. Preserve the ordinary observer fan-out
for ordinary commodity and Co-op contracts; identify the new closed effect type
explicitly so it is neither processed twice nor accidentally swallowed.

### 4.3 Durable inert objects precede business activation

Charters and manufactured cores need real ObjIds. Give them a separate durable,
idempotent `ReserveShell` step with operation-key-to-ObjId mapping, immutable
shape, owner/module purpose and bounded reservation capacity. Coordinator
idempotency preflight runs before allocating another identity. Wait for its
durable acknowledgement, then submit filing/manufacturing against that exact
existing shell. Never `World::create` and stage commerce in the same dirty turn.

Implement reservation as a narrow core-owned prepared object creation with
guarded allocator/object/container images and the reservation record in one
durable union. This is not permission for arbitrary World replacement. If an
equivalent acknowledged creation operation exists at M7.0, reuse it; otherwise
this explicit small core addition is part of M7.2. It keeps creation out of the
later two-owner rights transaction and survives a crash between the two steps.

For filing/integration, activation is **derived** from committed typed records
plus the E4 title/certificate. Pre-reserve the required immutable shell shape;
activation does not clear a flag, set an attribute, move an object or change its
World owner after the business receipt. Public rendering checks the typed
authority. Failed/retried work leaves an unusable shell, not a free asset.
Bounded idempotent cleanup checks retained intents/reservations, committed
titles/certificates and live rights before reclaiming an abandoned shell.

Initial installation is explicit. `Initialize` composition registers the typed
families without reserve/filing/rating admission and remains Closed. A God-only
self action proves all actual commerce/rating/shell families and complete E4
filing history empty, with a successful source audit and no commercial roots
or quarantine. It writes both Roll and core shell deployment markers in one
acknowledged transaction; it does not open commerce. Restart in `Active` runs a
synchronous final composition audit after every ordinary loaded hook. Missing
either marker, missing all reservations, absent roots or incomplete historical
E4 proof refuses recovery. Active boot never falls back to initialization.
Permanent shell birth/consumption counts and Roll count/max/next-ID invariants
detect lost families even when every business row has disappeared. The empty
checks run through the actual typed transaction and its revision fence; deleting
staged rows cannot manufacture an empty pre-state.

### 4.4 Hardware's additional prepared images

The rights-phase two-slot use is not a claim that all M7 state fits
Economy+Roll. Use this closed matrix and refuse undeclared owner expansion:

| Transition | One typed transaction | Legacy owner images | Core built-in image |
| --- | --- | --- | --- |
| Reserve charter/output shell | Reservation key and purpose | None | Guarded allocator plus inert object/container creation |
| File / purchase / renewal / resale | Roll commercial + new market records | Economy + Roll retention | None; shell usability is derived |
| Registry completion | Roll rating job/receipt/pages/completion/status | Roll retention + Tacsim BulkStore + Tacsim Artifacts | None |
| Exact-die integration | Inventory/industry/work-order/certificate/payment phase | Economy; another owner only if a real legacy write is proven necessary | None if output activation is derived |
| Core Install/Replace | Installation/certificate/inventory facts | Economy + Tactics | Closed bounded core/object/container exchange |

Do not combine refit with mind selection or sortie checkout: those are separate
business actions and would otherwise add Roll roots/another owner unnoticed.
Hardware's object image includes both moved cores and every affected container's
contents/timestamps/derived index image, merged to one image per physical target.
Tactics includes active lifecycle records and payload digest as well as unit
cells. A core-private field-split binder borrows the object/index fields alongside
disjoint Economy/Tactics slots, completes **all** checks, then applies prebuilt
images without mutable hooks, serialization, fresh lookup or dirty writes.

Keep the existing legacy single-`CustodyEffect` arity limit unchanged. A new
closed prepared Install/Replace exchange is not two sequential legacy moves
hidden in one provider. Its bounded multiple moves are validated and installed
as one object-image set. A later unsupported container/index operation refuses
before custody rather than falling back to `World::move_to` after payment.

Final guards cover all economic facts used, not only transaction allocator and
ledger tail. Roll reference installation advances the same generation as M6
prune: prepared prune then new root refuses stale prune; prune first refuses the
old root preparation. CommitPending prevents interleaving while awaiting receipt.
Malformed/unreadable retained rights at boot block collection and unprovable
rights, without exposing which foreign owner/source caused the refusal.

These bindings are a trusted owner implementation contract, not a safety proof
from returning `()`. Compile and fault-test the erased ownership/lifetime seam
before E4 business adaptation: two heterogeneous patches, alias/missing/foreign/
cloned slots, second bind failure after first bind, guard lifetime, drop and
panic. A bind failure changes nothing; an unexpected apply panic freezes/poisons
the owner and requires recovery rather than pretending rollback occurred.

## 5. Ordered implementation cuts

Each cut ends with a reviewable diff, focused correctness gates and an evidence
entry. A model or inert adapter may land before its command; no partial paid
service is exposed. The cuts below are deliberately smaller than the original
M7 row.

| Acceptance phase | Cuts | Required exit evidence before enabling the next authority |
| --- | --- | --- |
| A: prerequisites and commit substrate | M7.0–2 | Accepted M6 and fresh economy checkpoint; strict owner/readiness inventory; real memory/Redis bind/replay/failure matrix; ordinary contracts/observers unchanged; durable inert reservation proved. No paid mind entry point yet. |
| B: certified ownership and use | M7.3–5 | Deterministic per-card rating/promotion; fee/title/owner-right/root atomicity; source ACL including derivatives and delayed work; real IC claim/close service and unchanged OOC seats/money/time. Enable private filing/use only after this phase. |
| C: commercial rights and catalogue | M7.6–8 | Typed market and mixed legacy compatibility; exact payouts; all transfer/renewal/parent-loss races; scoped committed facts; named house source/filing/eligibility and actual Safe Conduct role/underwriting services. Individual entries may activate after their gates; absent IC-host live acceptance alone follows the explicit host condition, while missing role/services remains incomplete M7. |
| D: hardware | M7.9–11 | Certified bounded supply and real manufacture; titled-core sale; all-owner object/Tactics exchange matrix; current Class III permission; preserved hull state and no simulator ownership dependency. No live refit before this phase passes. |
| E: full acceptance | M7.12 | All required stories, catalogue dependencies, compatibility and operational gates on one exact clean revision; retained artifacts and truthful deployment record. |

Pure content/adapters may be developed in parallel after A, but paid service
activation follows the phase gates. A partial phase is a checkpoint, not M7
acceptance. Do not defer an explicit M7 requirement merely to keep one diff small.

### M7.0 — Accept the prerequisites and freeze shared interfaces

Record M6 acceptance separately. Fetch/reconcile current main and record the
merged SHA, actual in-flight trade owner interfaces and checkpoint results in
17d §7. Review E4/title/issuer/lease/market hooks and the M6 source/holdings seam.
Run the checkpoint commands in §7; no arbitrary quiet period or requirement to
finish all IC13 work substitutes for that evidence.

Track IC10's 4,096 inventory-marker deployment blocker and its other documented
delivery/boot residuals as existing trade constraints. IC12 did not repair them.
Current main's P-B supersedes IC12's funds-depletion enrolment gate:
only Missing required Treasury/Bids dependencies close Co-op enrolment; normal
insufficient funds are reported/paged without blocking enrolment. Sky remains
reported/paged and ungated. Preserve that current trade policy in M7 composition
and regression checks; do not restore the superseded Pending-funds rule.
Do not copy the marker scheme or silently make this cut unrelated trade cleanup.
If those paths are required by the chosen deployment, activation waits for their
tracked repair. Coordinate `contract_live`, market APIs and tacsim composition;
preserve IC13 buyout/end-lease RetryShortly/Retrying behavior and inflight orders.

### M7.1 — Typed records, access evaluator and retention reservations

Implement Roll's §3 records in `mush-roll`, market-owned entitlement families in
`mush-market`, and game adapters in tacsim; Roll retains no market edge.
Add pure guarded transition plans, bounded IDs/indexes, capacity reservation,
readiness/quarantine and derived term/source admission. Persist derivative draft
lineage through the existing workflow without permitting client edits to it.
Make `PaidSeats`/`OwnedDispatch` and the holder-specific immediate-parent binding
distinct typed variants. Validate dependent term and Grace bounds at every plan
that creates, renews, restores or changes the owner-derived grant. Implement
§3.4's event references, scoped fact bridge and exact M7 kind inventory; no
unscoped licence lookup may be exposed through the generic Fact condition.
Register the fixed action-root/delegation matrix, separately named typed market
quote/listing counters and bounded seller/buyer/contract indexes. Add strict
versioned §2.3 commerce content and reject invalid shares/quotas/price units.

Gate: model/codec bounds; independent status combinations; exact deadline edges;
depth/cycle/parent recall; no source oracle; complete boot cross-references; full
capacity still permits close/refund; both stale-root/prune orderings;
initial/renewed child term never outruns parent, parent-Grace filing and changed
qualifying-holder bounds; owner capacity distinct from paid seats; foreign and
missing restricted fact references indistinguishable; market owner/delegation
refusal and no legacy counter consumption. No payable
command is enabled by this cut.

First filing additionally requires explicit deployment setup/restart as §4.3
describes. Real Engine tests must prove both markers share one commit, prepare
and dropped custody publish neither, setup remains Closed, retained acknowledged
images recover after a lost installation, and either missing marker or repeated
initialization refuses. No test-only readiness setter proves this boundary.

### M7.2 — Core composed commit and owner adapters

Implement §4 first with one minimal typed operation plus real E4 and Roll root
participants. Preserve typed-only providers through a zero-participant wrapper.
Add slot incarnation and bind-all-before-apply; no fallible publication loop.
Use real memory and Redis authoritative stores.

Split this cut into reviewable commits: (a) compiler-checked erased core binding
and zero-participant compatibility; (b) Ledger/Economy/Roll prepared images and
all closed effect phases, including zero-money terminals and sweeps;
(c) notification-only new-effect routing plus durable derived-projection retry;
(d) the narrow ReserveShell lifecycle. Do not enable effect-bearing funding
before all phases are wired. No arbitrary new callback or owner count is hidden
behind a generic `FnOnce`.

Gate: the complete fault matrix in §8, zero-money phases, exact equality and
collision checks, all-or-none binding, lost-install freeze, replay past expiry,
no observer persistence and unchanged ordinary action/economy paths. Use the real
observer composition to prove a new settlement emits no legacy module ops,
World dirtiness, delivery callback or second grant; ordinary IC10 commodity
settlement and Co-op lease termination still project exactly once. Kill between
shell reservation acknowledgement and business submit, and retry/cleanup across
restart without allocating another shell or granting title prematurely. Independent
review must follow this cut before business services depend on it. Fact reads
remain false/unknown before receipt, become exact after publication, and match
fresh boot hydration; lost acknowledgements/replay cannot duplicate an event or
expose a fact ahead of the authoritative state.

### M7.3 — Registry conformance, rating and re-rating

Reuse M6's isolated bounded worker for a separately identified fixed Registry
scenario manifest `combat.v1`. Compile current ABI; validate exact schema, ≤8 presets,
profiles/rules, declared class, station vocabulary, floor and memory limits;
exercise armed and unarmed roles. Measure actual operations plus kernel fees
and faults for every declared profile/preset; bind source/envelope, engine,
class table, scenario/chassis/ground/seeds and result digest into a certified
receipt. A user `/check`, win rate, source declaration or fixture hint is not a
Registry rating. Non-budget correctness faults refuse certification; ordinary
tactical refusals (cycling weapon, vanished contact) are reported outcomes,
not a failure merely because a valid request could not land.

Reserve exact terminal keys before admission. The durable admission CommitToken
keys the receipt, status and ordinal pages; the mandatory RatingRef carries that
job key, the measured certificate-content RatingId and the full immutable receipt
digest. Pages bind job, rating and result. Pending retention roots belong to the
actual queued/running job and are exchanged atomically for its exact final rating
root on completion. Failure/cancellation removes only the pending root; the
auditable job remains. A retry of one job accepts only identical complete bytes.
A new attempt gets a new job, preserving prior results and recall status even
when the measured certificate content is identical. Neither a planned job ID nor
an arbitrary structurally valid receipt constitutes certification.

The initial versioned manifest is concrete:

* Freeze the exact class table, floor, ABI/engine identity, source/card/rule and
  ground/chassis digests, 2,000ms startup and 600,000ms maximum duration. Use the
  existing Proving Ground v5 and Duel/Muster/Hold layouts, seeds 1 and 2 and both
  side orientations; run Duel 1v1, Muster 2v2, Hold 2v2 against pinned Standard/Line.
  Retain the current explicit proving chassis mapping: line_tank at I/II and
  heavy_mk1 at III, with the candidate's exact tested class schedule; freeze the
  applicable unit/weapon/sensor definitions rather than inferring them from role.
  This is 12 whole trials per distinct canonical card/profile, with real memory,
  schedule, sample lease and station landing.
* Exercise every unique preset/card and authored profile, not merely the first
  profile that the existing six-picture check executes. Map aliases to identical
  measured canonical cards. At most 8 presets plus the class's 2/3/4 profiles bound
  the set. Each card gets Empty/Hostile450/HurtTwo on armed and unarmed public
  pictures and a deterministic multi-think order/lock-loss/miss/hurt/dry/profile
  transition sequence. Promote those fixture definitions into production
  content; do not import `tests/` code.
* Add capped public-picture probes with the class's full 12/16/24 contact rows,
  allowed odds rows, rule slots and memory limits. A 2v2 match cannot measure
  the maximum-row case. Include the shipped ford/motor route and disabled-motion
  boundary; use existing public observations only. Unarmed mobile drone-style
  probes remain unarmed, and no absent target/Dray core mount is invented.
* Use a small fixed-manifest factory over the existing pure scenario/custody/tape
  builders. The current owner-selected PROVE factory's 10..100-game request
  bounds and mutable user choice are not Registry authority. Keep Registry jobs
  separately identified, bounded and scheduled fairly alongside live work.

The manifest is sampled conformance, not a theorem about all future branches.
Runtime class limits remain hard. Worst observed ops plus fees determine the
minimum complete passing candidate class; wins remain reported quality. The
bounded maximum is 12 cards x 12 full trials x 3 candidate classes = 432 trials
plus explicitly capped probes. Emit progress/cancel results without source or
freshly altered operator settings changing an in-flight receipt.

The measured minimum may exceed the author's declaration, as doc 17 requires.
Keep the saved source/envelope immutable: add an explicit certified execution
requirement to filed custody and the shared seal adapter. Do not silently rewrite
`CardDefinition.declared_class` or pass an arbitrary rated hint. The current M6
`seal.rs` equality between declared and rated class is a known integration change.
Try permitted core classes from the declared minimum upward against the same
manifest, requiring a passing exact candidate; the receipt authenticates any
promotion. Hardware refusal still precedes optional class caps, and exact final
card/rule capacity is checked at the class actually sealed.

On a Rhai/ABI/class/scenario compatibility change, queue bounded re-conformance
for live revisions. New admissions require a current valid certification;
Registry recalls failures with private holder notice. Existing frozen sorties
remain intact. Gate: repeat/cross-host deterministic identity; class-boundary
promotion; unknown/stale/tampered receipts; cancellation/restart; worker resource
isolation; no fee or live-cache pollution on failure.

### M7.4 — Atomic filing, charter and owner's licence

Register the Registry issuer and fee/levy sink. File one exact saved revision
only after certified conformance, canonical duplicate check, lineage authority,
capacity and payer/title authorization. Use a durably acknowledged M7.2 shell
with bounded idempotent cleanup; derived activation, title, fee, typed records, owner
licence, references and receipt share the composed commit. An inert reservation
has no execution or commercial authority. Do not call `issue_title` and then
fallibly save a licence. Do not mutate the shell's World fields to activate it.

Support compatible line updates, explicit new majors, pin/follow resolution,
withdraw and Registry recall; followers choose the newest eligible revision
filed before the seal. Cross-charter normalized duplicates refuse generically
without naming a charter; clean-room similar behavior remains legal. An ABI
change cannot make another owner's retained identity disappear from duplicate
or receipt checks.

Use a separate canonical duplicate identity: normalized retained source SHA plus
exact schema, presets, profiles, programs/rules and declared class, excluding only
the runtime `mind_abi` identity. Exact revision and rating identities still bind
that ABI. Persist the duplicate identity in filed and index rows; boot derives
it again from the actual source-audited revision. Keep compatible-line schema
identity separate from the Registry execution-compatibility manifest. ABI-only
refiling collides; changed source/schema/program inputs change duplicate identity.

Gate: file/retry/reboot exactly once; failed fee/title/conformance leaves no
active right; filed-unlisted owner use; public forms `none`; parent authority
lost after rating but before file; line compatibility; old pins/seals on update;
withdraw versus recall; hidden-org renderer; no procurement eligibility.

### M7.5 — Shared holdings, source ACL and real seal/close service

Extend `match_/holdings`, the shared workshop access predicate and sealed origin
with exact committed filed licence/rating facts. Selection may show prospective
state; checkout resolves/rechecks once and captures immutable custody. A stale
selection is not authority. IC seal atomically claims `(unit, sortie)` and roots;
close is an idempotent production operation and never clears someone else's
claim. OOC checkout reads holdings with no claim, payment or licence-time write.
For `OwnedDispatch`, authorize each actual owned hull and dispatcher rather than
subtracting from the paid launch offer's one seat. Concurrent owned hulls each
get an exact open claim. A charter transfer changes new owner admission, not
old claim identity or warrant; global bounded capacity refuses before sealing.

Loss at a new seal follows the established Doctrine fallback/refusal policy,
with the reason only to the unit's own principals. Claim and close remain valid
after lapse, transfer, revocation, recall or source-right loss. Current author
access may be narrower than historical replay execution; separate the two.

Gate: sealed holder flies and tunes/probes with denied source/memory through
all transports; Q17 owned-draft and queued/reconnect matrix; class/floor/station
recheck; old seal/hash unchanged; OOC journal/seat/time invariants; duplicate,
cross-unit and stale closes; restart with open claims; two or more owned hulls
dispatch under the free owner's licence while an otherwise equivalent one-seat
paid grant refuses its second concurrent claim. Include wrong-hull/title and
charter-transfer races and no capacity loss on an idempotent close. Integrate real region
sorties if available. If the fresh checkpoint proves the region driver absent,
deliver the production IC claim/close action provider and real composition
story now, name the exact missing driver/call site, and make attaching it an
explicit dependent integration task. Do not claim that OOC seats exercised IC
execution or mark the unavailable region end-to-end story as passed.

### M7.6 — Typed market routing, primary sale and the launch catalogue

Enable M7.1's separately scoped entitlement families/terms adapter, with actor
seller distinct from optional venue. Existing commodity `SaleTerms` and
`MAX_PROCEEDS_CUTS=1` remain unchanged. Bind dynamic Roll listings through a
bounded versioned catalogue seam; `CatalogSource::pack` is currently static and
cannot simply read live Roll state. Preserve existing catalogue ceilings. Use
the typed listing/quote/capacity indexes throughout acceptance and settlement;
no legacy MarketStore quote, counter or delivery marker is part of this path.

Expose sealed/open/open-resale and transferability, exact licence capacity/class/
term, rating and role. Purchase drafts an E4 escrow contract whose effect goes
through §4; the current after-payment `DeliveryProvider` cannot be the right's
authority. Approvals and queueing return pending, never granted. Payment does
not edit a loadout. Add the launch house vendor through the same actor/issuer/
fee/levy path as a player, without a privileged free grant.

Deliver the four named house entries in bounded subcuts through the same
principal/fee/title/licence path as player publication:

1. Stickney Yards Doctrine Library publishes the existing free Line/Sentry/
   Sniper/Skirmisher/Hunter/Rearguard teaching pages, keeping ROM access free.
2. Meridiani Field Series gets a real house org/account/vendor and staff-authored
   sealed combat source/presets with Registry receipts; no fixture identity or
   free privileged grant. Use §2.3's launch licence terms unless content explicitly
   differs. Existing Stickney/Tycho capital is not authority to debit for another
   house; create and fund the actual scoped principals through normal services.
3. Territorial Force Coprates Drill gets a real filed combat mind and a
   Republic-standing AvailabilityProvider with the same current transaction
   check. Standing preview alone does not authorize settlement.
4. Tycho Mutual Safe Conduct includes real `Escort` convoy-defence and `Transit`
   haul-under-escort profiles, lawful explicit ground-role hull/core binding and
   a deterministic role scenario. A cargo carrier needs actual capacity/custody
   content; the unarmed drone is only a public-picture probe. Exercise public
   commander objectives/plots, route completion and cargo conservation, not a
   renamed combat preset. Doc 17 permits this regional ground role; it does not
   require a Dray autopilot. Existing ship departure requires a pilot aboard and
   has no mind station. Do not relax that contract, expose another unit's private
   pose, or repurpose ground `goto` as an IC ship command.

   Add real production underwriting services: versioned product and exact quote,
   policy, reserved insurer exposure, mechanical-loss claim and expiry/close.
   Discount eligibility binds an exact certified Safe Conduct role and trusted
   IC seal proof, never a name, purchased licence alone, caller boolean or OOC
   seal. Premium, coverage/exposure, roots and receipt share §4's prepared commit;
   deterministic accepted losses pay once after the explicit delay/exclusions,
   including the paid wreck's title transfer to Tycho. Selected provisional
   configurable tuning is a 10‰ base premium on bounded agreed insured value,
   100‰ reduction of that premium, 800‰ covered-value payout and 1 IC-day delay:
   10,000cr insured value quotes 100cr ordinarily and 90cr with eligible cover.
   These are new launch choices, not existing policy/prices; reserve real insurer
   liability before accepting a premium. Existing RecoveryConfig cooldown/loaner
   coverage is not this product, and doc 18's later underwriting is not proof it
   already exists. Follow the concrete service/claim design in the Tycho note.

   The real-IC-host condition in doc 17's M7 gate applies here too. If the fresh
   checkpoint confirms no persistent IC sortie host, M7 still delivers and tests
   the actual role source/certification and production underwriting services.
   Authenticated composition tests exercise their same trusted mechanical proof
   interfaces. Public activation without such a host/proof refuses with no
   charge; quote/readback names the unavailable IC activation. Actual player
   convoy departure, live loss/claim and return acceptance remains an explicitly
   conditional integration, not a passed test or a requirement to build a new
   persistent host for M7. New carried shapes/stations, if genuinely needed by
   an available host, receive the declared ABI/frame/hash/inventory review.

This does not waive the named Safe Conduct role or priced-cover services. It
separates those required M7 implementations from the already-written host
condition. Later unnamed role doctrines and autonomous interplanetary ship
operation are not silently added. An actual conflict in the product contract is
reported precisely, not converted into fictional hauling or an advertised
discount with no real quote/premium/claim implementation.

Gate: actual second-account purchase, balanced §2 split and exactly one licence;
wrong/missing account, org scope and unknown identity; self/alt refusal; stale
venue/title/terms; tiny amounts/max chain/duplicate recipients; no source in
quotes/events/errors; author versus venue payout; ordinary IC10 proceeds intact.
Add mixed legacy/new sales in one process across sweeps/restart, plus all four
real catalogue reads and purchases. Safe Conduct's gate must demonstrate the
actual eligible cover quote difference and its real role behavior, not only a
label or static card. Prove counterfeit/OOC proof refusal, no-host zero-charge,
premium/exposure conservation, duplicate claim/reboot, excluded loss and wreck
title; distinguish service-composition evidence from the conditional live convoy
story. Coprates must refuse the ineligible principal at commit.

### M7.7 — Renewal, lapse, resale, charter title and PendingTransfer

Implement paid renewal and free owner attestation with one contract/receipt per
term generation. Use WorldIc for terms/Grace/deadlines and RealUtc only for
ordinary quote TTL. At exact boundaries: derive sanctions/dependency/term,
process close/refund/settlement, then admit a new seal. A deadline is a positive
settlement trigger, not `Before(deadline)` that becomes false and refunds.

Implement licence and charter resale, non-transferability and account checks
through every relevant generic E4 title/contract route, including lien/collateral
restrictions. Apply §2 Q18 and preserve held seats; charter ownership changes
derive the new free-license holder without duplicate owner authority. A pending
title sale blocks owner-licence admission across its lines through one bounded
charter pending record/aggregate. Never clear a lease on `LeaseEnd::Retrying`.
Owner attestation and charter-holder change re-bind the qualifying parent grant
and enforce §2.2's term/Grace cap; an absent/shorter parent cannot inherit a
fresh 90-day owner term. Publish §3.4's generation-specific lapse and completed
transfer facts only with their authoritative transition. A renewed licence does
not erase the earlier lapse event, and PendingTransfer is never a transfer fact.

Gate: immediate/no-claim sale; two seats with one open claim; last close and
deadline racing; term-end buyer Grace; duplicate close/settle; no seller renewal;
revocation refund and recall carried; holder/source revocation at committed
transfer; generic title bypass; stale renewal versus resale; frozen old warrants;
terminal completion at capacity and after restart.
Add dependent-parent Grace/term boundaries, restoration and shorter/longer
replacement grants, plus exact scoped fact values before/after receipt and
restart, no pending/failed-transfer fact, and no guessed-licence oracle through
contract conditions, readback or front consumers.

### M7.8 — Open forks, dependency and parent-chain settlement

Expose server-authorized fork/private-file/list operations using the persisted
lineage from M7.1. Open permits private filing; open-resale permits listing.
Every derivative settlement carries the bounded immutable royalty chain.
Renewal/reacquisition can restore current eligibility, not rewrite ancestry.
Keep four checks distinct: immutable ancestry/integrity and royalties; applicable
exact-revision Registry action; the requester's own grant/form; and the current
charter owner's qualifying immediate-parent grant for owner use/filing. A paid
customer of Child need not buy Parent. If that customer forks Child, their new
owner's licence depends on their own immediate-parent Child grant. Do not infer
automatic recall of every descendant from an ancestor recall.

Implement the explicit 2026-10-17 downstream ruling from §10: existing paid
licences remain usable under their own term and Registry restrictions; issuer
parent-right loss blocks new sales and renewals, with the qualifying grant
rechecked before commit. Do not add that issuer dependency to an existing paid
customer's new-seal or source/form checks. No date is extended on restoration.
At each applicable file/attestation/restore/charter-transfer transition, the
owner's child term and Grace are bounded by that holder's qualifying grant.
Restoration never silently extends a child or rewrites an existing sortie.

Gate: all Q17 loss causes for dependent owner use through depth three; source
sealing across owned-copy/replacement; old seals finish; initial and renewed
term caps, parent-Grace filing, restoration with expired child term, changed
qualifying grant and charter holder; applicable recall cannot be laundered;
private open fork not listed; open-resale exact full-chain money; same-account
charter/licence transfer refusal. Separately test Ada's dependent owner use,
Bo's existing paid Child grant, fresh Child issuance/renewal and Bo's own fork
under the explicit downstream ruling; no case charges Bo for ancestor licences.

### M7.9 — Certified die inputs, integration orders and titled cores

Add Basin importer Origin receipts and a persistent bounded WorldIc-month
allotment using §2.3's 64/16/4 launch quantities. Create the actual chartered
importer principal, Luna depot and Ceres/Phobos agent records; both agents debit
the same importer/month/class counter. Public labels are `die.c1|c2|c3` on
`tacsim.electronics`; numeric Grade
alone is never authority. Exact quantity-one certified lots are the initial
input. Register the separate `core_integration` facility charter and Stickney
`core_integrate@1` recipe with exact die/certificate, facility/policy/capacity,
paid service, requester and the durably acknowledged reserved output identity.
Use the explicit die/integration price split and actual scoped payer/payee
accounts; a read-only listing is not authority to mint or consume a die.

Extend industry exact-input/output semantics deliberately: current commodity
recipe lines and Grade(0) outputs cannot certify or title a core. Reserve exact
input, complete one durable manufacturing certificate/consumption/title/work-order
transition, and release once on cancellation/refusal. Reuse M7.2's separate
inert-shell reservation rather than allocating during completion. No active
core exists until its certificate and title commit; abandoned shells are bounded
and non-authorizing. The work-order typed transition and Economy participant
must be in one exact union; current commodity Grade(0) output/delivery callbacks
are neither a certified core nor an atomic substitute.

Gate: quota/month rollover/replay; forged Origin, caller-chosen grade and generic
crystallization refused; actual material conservation; repeated completion/
restart creates one output and payment; facility/charter drift; cancelled exact
input reuse; no untitled/double-titled output; capacity full after reservation.

### M7.10 — M32 component installation and hardware commit extension

Add typed `UnitMutationOp::InstallComponent` with expected unit revision and a
registered compatible fitted-unit definition. Start with §2.3's finite
line-tank I/II/III family; existing II stays canonical and the two new definitions
change only the allowed turret core binding. Pure scratch application replaces
only the designated core component/cells/capability, updates exact schema ref,
preserves all unrelated state, and validates exact post-topology before one
revision/journal increment. Empty mount and replacement are explicit; no second
core or rogue capability. Keep strict boot shape validation.

Add a tactics prepared owner image and a core-owned prepared object/custody image
where installation physically moves the replacement/displaced core. This is a
**second reviewed extension** of §4, not a claim that the initial E4+Roll pair
already publishes World objects. The hardware action can use Economy+Tactics
participants alongside typed inventory/industry/certificates, with built-in
object images bound before any apply. Tactics preparation must include its
active lifecycle projection and payload digest, not only `UnitStore.units`.
Canonicalize the incoming/outgoing core and all affected containers into one
guarded post-image each, including locations, contents, timestamps and derived
indexes. Bind these core-owned fields alongside the disjoint module slots using
the private World field-split seam. Resolve carry/destination/authority checks
and exact object revisions before custody; after receipt install prebuilt images
without mutable hooks or a fallible `World::move`. Preserve the existing legacy
single-custody-effect restriction. This new closed prepared Install/Replace
operation permits its bounded exchange, including old-core return, only because
the entire set binds before any apply. It is not two moves hidden in one mutable
provider. Finish this core seam before enabling refits; do not leave dirty
post-ack writes for a later flush. Mind selection/checkout is a separate action.

Require authority over hull and core, actual custody, certificate/title health,
unencumbered permitted title, facility charter and non-deployed/non-voyaging
state. A typed reverse installation index prevents one asset in two hulls.
Return displaced core intact to authorized custody; installation is not repair
of unrelated systems. Refuse M8 ungoverned outcomes.

Gate: install/replace/reboot; topology and capability forgery; stale unit/object/
facility; double install; deployed and port-under-way refusal; preserved damage,
ammo, heat and mobility; old-core return; destroy/scrub reconciliation; failure
between every prepared hardware leg; exact write collision; no half-installed
asset after lost receipt/restart.

### M7.11 — Restricted core sales, prices and operator surfaces

Register `tacsim.licence.core3_end_user` Entitlement and validate the current
purchasing principal/scope at actual Class III transfer, including resale and
service completion. `Legality::Restricted` and shop availability are labels/
previews, not the transaction guard. Unknown/wrong/revoked entitlement refuses
before charge; an org grant is scoped to the org. Revocation does not rewrite a
previously installed paid core; later seal rules are separate.

Author §2.3's new 100,000cr line-tank catalogue baseline, the 3,000/8,000/25,000cr
finished-core references and separate 100cr fitting quote. No existing Dray
price is used as a tank anchor. Validate the 3/8/25 percent relationship and
checked integer rounding in content tests. Ship importer,
facility, recipe and market readback explaining certified provenance. Expose
mind filing/listing/licence/renewal/resale/fork and hardware order flows through
existing command and typed request services. No text interpolation bypass, actor
field supplied by a client, unbounded source reply or alternative source oracle.

The shared service inventory must include `@mind/file <owned>@<rev>`,
`@mind/publish <owned>@<rev> ...`, current holdings/licence readback, pin/follow,
fork, withdraw, renewal, licence resale and Registry recall/revoke. Purchases
and title trades use the normal market/contract confirmation path; hardware
uses normal industry/refit orders. Select the remaining short spellings with
the existing command grammar in this cut, then expose the same typed actions
in web/TUI without requiring clients to synthesize command text. Reuse
connection/actor freshness and exact draft EditBase where source is involved;
commercial operations bind their own record/terms generations.

Gate: end-user right revoked between quote/accept/settle; direct title/service
bypass; scope/issuer failures; prices and lawful rounding; real hardware story;
command/help/type inventories; bounded narrow/wide rendering and typed stale-edit
errors. OOC pods and no-IC-posting assertions remain unchanged.

### M7.12 — Full author/buyer/hardware acceptance and release record

Run §9 on one exact clean revision with real composition/transports and retained
artifacts. Close compatibility, boot/migration, source visibility, retention and
performance reports. Update docs 17/17b/17d, help and inventories from observed
results; do not mark M7 complete on pure owner tests alone. Any unavailable IC
region integration is named, with its production service and substitute gate
evidence; it cannot be recorded as a passed real region story. The four-house
catalogue, actual Safe Conduct role and underwriting services must pass M7.6;
only the live host-dependent flight/convoy/loss story follows doc 17's explicit
IC-host condition. The resolved §10 downstream paid-derivative ruling must have
its chosen behavior tested before full M7 acceptance.
M8 remains blocked on its own IC-sortie/evidence prerequisites.

## 6. Files and compatibility discipline

Expected owners: `mush-core/{action_state,action,engine,module,ledger}` and a
participant module; `mush-server/{action_coordinator,world_task,persist,modules}`;
`mush-economy/{api,contract,contract_live,engine,module}`; market's separately
scoped entitlement terms/admission; new Roll commercial modules; tacsim Registry,
access, composition/content and existing holdings/workshop/transport services;
tactics unit/module; inventory exact-lot and industry work-order adapters.
Prefer no edits to port co-op behavior; any actual shared seam is coordinated.

Retain M6 source ABI, canonical card and runtime behavior unless a real carried
shape changes. Certified rating/provenance or new seal fields require one explicit
format/hash review with old reader rejection and resume fixtures. Bump the next
actual versions, not a number copied from these notes. No silent fixture repins,
declared/rated rewrite, observer-rule change, new private arena read or resource
budget raise. Update module roster, dependencies, ownership/delegation, mutation
writers, command/help/typed-client, state-family and persistence inventories.
Include the three M7 fact kinds and their scoped consumers; keep M8 kinds
unsupported. At documentation promotion, align doc 17's stale D-UM-57 row and
older M7-gate pending/default wording with the explicit owner-use Q17 ruling
and separately recorded downstream answer. Do not alter those production docs
as part of this preparation note.

## 7. Economy checkpoint evidence required before implementation

Run against the accepted M6 tree plus freshly reconciled main; record complete
commands, SHA, host and counts, including disabled/ignored cases and reasons:

```text
cargo test -p mush-economy --test e4b2 --test e4c --test e3a --test e3b
cargo test -p mush-market --test ic7 --test ic8 --test ic10 --test e7r2
cargo test -p tacsim --test coop --test commerce --test sale --test orders --test ports --test voyage
cargo test -p mush-server --test market_issuer --test ic_slice_loop --test boot_parity
```

IC12 moved `ic_slice_loop` from tacsim to mush-server and now boots the
production root at IC rate 24 before its controlled acceleration; do not retain
the deleted tacsim test target or substitute the older hand-built fixture. Its
new five-story target is memory-backed evidence, not the still-owed Redis smoke.

Also run current lease/end-lease RetryShortly/Retrying/reboot tests, registered
issuer/account/title and composition inventories, inventory retention refusal,
and actual merged IC13 buyout/re-lease/inflight-order cases when present. Verify
money+title atomicity, lost-title refusal, every contract state after reboot and
journal-tail eviction, approval not acceptance, refunds/expiry/idempotence,
market delivery/quote/capacity and IC10's unchanged commodity split. These command
names are a starting inventory, not permission to ignore renamed/current tests.
The completed IC14 checkpoint above satisfies this prerequisite. Historical
preparation notes alone do not supply passing evidence; repeat affected checks
when integration changes their inputs.

## 8. Mandatory composed-commit fault matrix

Run with real memory and Redis authoritative persistence; after each injected
failure, compare both live state and a fresh boot, plus actual public holdings.

| Injection point | Required outcome |
| --- | --- |
| Validation, source/rating, account authority, encoding, capacity or participant prepare | No money/title/right/root mutation; inert reservation at most, bounded and unusable |
| Old generation, slot replacement, foreign composition or second-owner bind failure | No owner installed; pre-custody refusal or post-receipt integrity freeze as applicable |
| Erased patch consumed, cloned owner slot, first owner bound then second bind fails | Returned installers own patches/config and borrow only valid owner fields; all guards release without partial apply; no generation-only alias acceptance |
| Duplicate key, Ledger/JSON collision, oversized union, legacy dirt/predecessor | No submission; no normalization/deduplication hides a conflict |
| Writer send before custody / definitive refused intent | Complete abort; ordinary retry follows durable idempotency preflight |
| Owned intent, process kill before any write / after each replay write | Restart completes exact retained bundle once; no fresh price/rights/clock evaluation |
| Final receipt lost, wrong token/hash, install capsule dropped | No success; integrity freeze/recovery; no second charge or locally inferred commit |
| Crash after RAM install but before player notification | Durable retry returns original receipt; no duplicate notice-dependent grant |
| Duplicate/conflicting operation key, delayed former callback | Exact retry returns same outcome; different request refuses; old callback grants nothing |
| Expiry/parent/title change around ownership boundary | Before ownership revalidate/refuse; after ownership replay exact images then process a separate new transition |
| Full capacity after accepted funding | Reserved terminal records permit settle/refund/close; no stranded escrow or seat |
| Prune then retain / retain then prune / unreadable retained boot state | Stale operation refuses; unknown roots cannot be reclaimed; generic privacy-preserving refusal |
| Zero consideration, zero refund, no journal append | Contract/rights/pending/root phase still atomic and replayable |
| Shell acknowledgement lost / crash between shell ack and file or manufacturing submit | Same operation resolves the same shell; no second allocation, title or active certificate; cleanup cannot delete an intent-owned shell |
| New-effect notifications with real observers; derived projection retry after restart | No legacy persistence/delivery runs inside receipt; paid rights already committed; idempotent derived projection retry cannot grant twice; ordinary IC10/Co-op observers unchanged |
| Replacement shares an old/new holder or container; later hardware bind fails | Canonical one-image-per-object exchange with complete containers/indexes; no first move, missing old core, dirty write or partial Tactics lifecycle update |

Assertions compare account balances/holds, journal sequence/key, title, contract
phase, licence/term/holder, pending state, operation receipt, seat claims and
revision/source roots. A callback counter, an observer mock or a final balance
alone is insufficient. Hardware repeats this matrix for exact lot consumption,
output certificate/title, custody, complete object/container/index state, unit
state/revision/lifecycle and work-order result. Compare hydrated RAM with live
indexes/counters, not only serialized documents, after every retained replay.

## 9. Final acceptance stories and gates

1. **Commercial author story:** real character A files and publishes a sealed
   mind; character B on another account buys IC, sees it in current sim holdings,
   selects/seals/releases and flies it. B reads cards and permitted trace, never
   source/memory through line, typed web/TUI/SSH, export or bench. The OOC run
   posts no IC transaction, takes no seat and changes no licence date. B resells;
   the old holding is unavailable at the next sim seal, with one balanced split.
2. **Private/derivative story:** filed-unlisted owner use has no offered form;
   ordinary open fork files privately but cannot list; open-resale derivative
   lists and pays the exact bounded parent chain. Parent lapse/transfer/revoke/
   applicable recall seals the dependent owner's source and blocks their new
   file/owner-use seal, while an already-running sortie and hash sequence stay
   unchanged. Restoration preserves ancestry and rechecks bounded child term/
   Grace without silently extending it. An existing paid customer's new sorties
   remain available through their own lawful term; new sales/renewals refuse
   until the issuer's qualifying parent rights return. Recheck parent loss
   between quote/approval and commit, preserve independent Registry restrictions
   and unchanged dates, and never require that customer to buy ancestors.
3. **Seat/term story:** two-seat licence, one live IC claim; sale goes pending;
   both parties refused new admission; last-close and deadline variants settle
   once. Remaining claim still consumes capacity, old warrant survives, duplicate
   close is harmless; term-end buyer gets Grace/renewal. Revocation refunds,
   recall travels, and lapse fallback is visible only to own principals. A free
   owner's parallel dispatches use `OwnedDispatch`, not that purchased seat
   counter; title transfer preserves old open claims and changes new admission.
4. **Hardware story:** real certified importer allotment → exact die custody →
   Stickney integration → titled core sale → refit → restart → actual core read.
   Class III permission is tested at transfer, replacement retains damage/ammo
   elsewhere, displaced core remains owned, and a second install cannot reuse it.
   Existing OOC pods show no owned-hardware dependency.
5. **Authority/adversarial story:** same-account licence and charter transfers,
   unscoped org acts, missing account oracle, forged receipt/grade/lineage,
   generic-title bypass, stale callbacks, cross-character source selectors,
   malformed retained records, queue saturation and cancellation all fail with
   their required no-mutation/privacy outcome.
6. **Launch catalogue story:** read and acquire the four named house entries
   through real owner/catalogue services. Free Doctrine stays free; paid houses
   pay filing/levy and obey budgets. Coprates refuses an ineligible principal
   even after an eligible preview. Safe Conduct demonstrates its real role and
   priced cover discount through the actual services and fixed role scenario;
   a renamed Standard card or static marketing sentence is insufficient. Include
   actual composition premium/exposure/claim/wreck-title records, and zero-charge
   refusal without trusted IC-host proof. Record live convoy/loss/return as
   conditional when its real host is absent; never label the service rig a live
   IC journey.
7. **Fact-interface story:** contracts/fronts observe exact committed filing,
   lapse-generation and transfer-receipt facts with proper scope; before receipt,
   after failed transition and PendingTransfer do not claim a completed event.
   Restart gives the same facts, renewal does not erase historical lapse, and a
   foreign caller cannot distinguish unknown from private licence references.

Every story includes restart and durable artifact inspection. Use configured
real public services, not direct fixture grants, for its successful path. For
unimplemented IC regions use the same production claim/close APIs the region
will call, never simulated OOC seats. Prove unlisted lines are excluded from
procurement eligibility without implementing M8 procurement.

Run workspace tests, relevant integration/ignored acceptance targets, formatting,
strict Clippy/restriction inventories, strict rustdoc, web checks and external
bot contracts. Preserve existing core/arena determinism and observer batteries;
run cross-host deterministic cases for changed execution/receipt codecs. Benchmark
Registry work alongside live arenas and report bounded queue/fairness/memory,
peak operations/fees and latency distributions with exact hardware/config.
The owner's report-only timing ruling remains: timing misses are reported and
explained, not silently made correctness gates. Resource ceilings, operation
caps, data isolation and deterministic replay are still hard requirements.

## 10. Paid-derivative ruling and remaining integration facts

Q17 is ruled, Q18 has its authorized written default, §2.3 selects provisional
launch tuning, M7.3 fixes the combat manifest, and M7.9–11 choose the finite
hardware catalogue. These are no longer generic open questions. Ordinary M7
implementation remains authorized after the existing M6/checkpoint gates.

**Explicit user ruling, 2026-10-17:** "Keep existing paid licences usable through
their own term; block new sales and renewals until parent rights return."
Ada holds Parent open-resale, files Child and sells Bo a paid Child licence.
If Ada, or Child's current charter owner/issuer, loses the qualifying Parent
grant, the resulting checks are:

| Operation | Required behavior |
| --- | --- |
| Ada's dependent owner-use/new filing | Q17 still blocks new owner-licence seals and filings; dependent owned source stays sealed until qualifying rights return. |
| Bo's already-paid Child grant | New admission uses Bo's own term/form/seat/warrant and applicable Registry status. Ada's missing parent grant is not an additional use predicate. |
| New Child sale or renewal | Refuse until the issuer's qualifying parent rights return; recheck them in the guarded preparation/commit path, including loss after quote or approval. |
| Existing sealed sorties | Finish unchanged for both owner and customer; no mutable licence lookup enters the running arena. |
| Restoration | Re-enables otherwise eligible fresh commerce; it neither extends Bo's term nor renews Ada's bounded owner term automatically. |
| Bo forks Child under a lawful open grant | Bo's new owner-use dependency is Bo's own immediate-parent Child grant. Bo does not purchase Parent or other ancestors separately. |

These are independent predicates. Applicable recall/revocation and the holder's
own term/form still apply; parent loss is not an automatic Registry recall of
Child. Preserve immutable ancestry and royalty payouts. After durable intent
ownership, replay completes the exact accepted images rather than applying a
new eligibility decision. Source/privacy projections and delayed responses use
the same operation-specific rights evaluator; they must not accidentally revoke
Bo's lawful existing paid form because Ada lost Parent.
See `target/m7-parent-dependency-interpretation.md` for the historical question,
evidence and exact resolved matrix. No policy answer is pending for this edge.

The genuinely unresolved facts are checked at M7.0 and recorded against code:

| Fact to establish | Required disposition |
| --- | --- |
| Actual merged E4/market/IC13 interfaces and trade deployment constraints | Fresh fetch/reconcile plus the exact checkpoint; adapt the scoped provider/phase boundaries without changing ordinary commodity/Co-op behavior. A known deployment blocker remains visible until its owning repair passes. |
| IC region sortie driver and production call sites | Integrate current driver with the new claim/close service if present. If absent, finish the production service/composition gates and name the precise missing driver as a dependent integration, never a passed live-region story. |
| Safe Conduct role and insurer service | Implement the actual ground Escort/Transit source/content and deterministic role scenario, plus production quote/premium/policy/exposure/claim/close services in M7.6. If no IC host supplies trusted seal/loss proof, public activation refuses without charge; production-composition gates still run and live convoy/loss acceptance remains conditional under the existing doc17 M7 clause. No Dray autopilot or unrelated persistent-host build is implied. |

No new ruling is needed for the prepared-object reservation/exchange: it is
necessary implementation work selected in §4. Its exact safe API must pass the
compiler/failure gates. Do not transform a feasibility failure into two mutable
calls or an after-payment callback. Likewise, no launch balance is asserted as
historically authored: changeable defaults are plainly labelled and all accepted
terms remain immutable. Resolve routine details autonomously. No new policy
request is created by these notes;
report an actual newly discovered contradiction rather than inventing one.

## 11. Acceptance record template

For each cut record: source and fetched-main SHA; exact scoped files; new or
changed contracts/formats; commands and passing/failing/skipped counts; memory
and Redis fault cases; cross-host comparisons where applicable; retained story
artifacts; independent review findings/dispositions; performance report; known
residual owner and deployment status. Mark M7 accepted only when all applicable
stories and hard gates pass, every explicit deferral is truthful, and the ledger
describes implemented behavior rather than this plan.
