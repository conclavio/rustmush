# 26a. Detachment implementation

Started 2026-10-28 against `origin/main` at `62656f43`. Policy comes from
[doc 26](26-detachments.md) and [doc 27 Draft 0.4](27-ic-field-operations-design-and-survey.md),
including the accepted Q-R1–Q-R5 and P-1–P-5. This brief divides the DT stream
into reviewable changes. DT.1a provides the foundation; DT.1b integrates the OOC
host and clients. IC preparation, NPC personnel and durable launches follow.

## Delivery sequence

| Slice | Deliverable | Completion gate |
|---|---|---|
| DT.1a | Neutral numbered roster, leadership selection, versioned orders, recipient history, and an OOC exercise adapter | Deterministic tests cover succession, reconnect flapping, stale delivery, standing orders, and advisory player responses |
| DT.1b | Production OOC host and client integration | Typed commands, numbered-slot controls and roster display use the same revision; session/control hooks feed availability; player notices and NPC/controller delivery run through actual communications and observation boundaries |
| DT.2a | Shared unit authority and mission-role grants | Personal lending and organization scopes; bounded delegation and exclusions; prospective revocation; no charter-owner shortcut |
| DT.2b | Presets, drafts, personnel and preparation | Multiple private plans, current carrier containment, consent/readiness for a revision, contracted NPC crew and exclusive unit reservations |
| DT.3 | Launch and operation history | Preflight then individually durable departures, hull-epoch links, partial failure/retry, returns and unresolved casualties; no rollback of physical departures |

DT.4 subset delegation is deferred by the owner (2026-10-30). DT.5 remains
the communications follow-on, integrated with the existing IC transport and ECM
path; see [its scope below](#dt5-communications-integration).
The DT.2a authority and retained-custody policy is specified in
[26b](26b-unit-authority-and-grants.md).
The preparation slices and their durable admission boundaries are specified in
[26c](26c-detachment-preparation.md).
H1/H2 and W-clock keep their existing owners and may proceed independently where
doc 27's dependencies permit.

## DT.1a ownership and authority

`mush-tactics::detachment` owns the common model. It does not depend on simulator
teams, pod counts, match lifecycle, synthetic personas or the arena's fixed order
timeout. `tacsim` supplies game objectives through an OOC adapter. The common
records reference existing unit and character identities; constructing a roster
neither creates physical assets nor acquires control of them.

Roster identity and revision identify the accepted membership/priority. A separate
command term changes on succession, leaving slots stable. The first available
player in slot order leads, even if their command authorization is insufficient;
that missing authority refuses orders instead of silently choosing another player.
An explicitly nominated, available and authorized NPC character is the fallback.
An automated unit is never a human commander. No candidate means no leader.

Availability is supplied by the host that owns character capability, station
control and unit condition. Linkdead retains leadership through the host's actual
grace deadline; it does not create a separate hard-coded 300-second timer.
Incapacity, disablement or released control take effect immediately. A returning
earlier player must remain eligible through a short hysteresis; the initial
selection is immediate. DT.1a uses a configurable policy with a three-second
default, an engineering tuning value rather than a new world ruling. Time is
explicit input, with backward updates refused.
The host reconciles at session-grace and hysteresis deadlines as well as before
admission. The model exposes the next required refresh and refuses new admission
or delivery once that deadline arrives until availability is reconciled again.

The model consumes explicit command authorization at admission and delivery. It
does not implement the DT.2 grant resolver or manufacture grants from leadership,
ownership of a carrier, or presence aboard. Mission grants must cover each target
unit and the accepted mission/roster. Delayed work must be revalidated against the
current roster revision, leader and command term, including revocation.

Orders carry a generic objective, issuer, sequence, roster revision and command
term. No expiry means a standing order. An explicit deadline belongs to the host's
chosen clock; the library owns no wall clock or campaign multiplier. Delivery is
separate from acknowledgement, attempt and completion. A player's acknowledgement
is a report, never a control intent. Previously delivered standing orders survive
succession; a late old-term message cannot replace them. Per-recipient state is
bounded; a later durable operation history belongs to DT.3's owner.
The OOC exercise uses one monotonic millisecond domain for both leadership timers
and explicit order deadlines. Production integration must translate real session
deadlines and chosen IC order expiry into its host clock, or supply distinct typed
clock inputs. This slice does not implement the campaign/tactical clock adapter.

The roster projection omits health/disconnect reasons. It is a projection for an
authorized roster reader, not a public sensor or instant IC broadcast. DT.1b must
deliver changes over the recipient's available communication path and must not
turn private availability facts into casualty reports.

## DT.1b live OOC integration

`+sortie/practice` explicitly enables the seated player's practice group. The OOC
adapter groups each side's stable callsign order into groups of at most six; it
does not truncate larger sides or convert a departed player into an automated
roster member. Practice has no real NPC personnel to nominate as commander.
`+sortie/roster` prints the current slots and versioned command prefills. This
opt-in is deliberately confined to OOC practice and grants no IC asset rights.

All clients submit the same ordinary line commands:

```
ORDER DT:<id>:<revision>:<term> <slot|ALL> <objective>
WILCO DT:<id>:<revision> <sequence>
NEGATIVE DT:<id>:<revision> <sequence>
```

Objectives reuse the cockpit vocabulary: HOLD, ADVANCE, ENGAGE, WITHDRAW, SCREEN,
SUPPORT and RELEASE (cancellation). No deadline means a standing objective.
The world adds a member-only `detachment` object to the existing cockpit frame;
query, push, STATUS and VIEW share this projection. Decimal-string counters avoid
JavaScript precision loss. Client decoders reject malformed or inconsistent
prefills. Web and SSH pixel F1–F6 address slots 1–6; ALL has its own visible
control/text prefill. SSH text mode retains its screen navigation and displays
the complete command text in the roster. Acknowledgements never emit helm/fire
intents. Neither a successful offer nor WILCO claims execution or completion.

Session hooks, station changes, own cockpit frames and a coarse tick feed live
availability. The session owner keeps `is_connected` true through its configured
linkdead grace; the host does not invent a second deadline. Commands re-evaluate
availability before admission, and failed reconciliation closes admission and
delivery until a successful refresh. The three-second return hysteresis uses a
monotonic elapsed clock. Own wreck/disabled observations can end leadership;
the roster does not disclose private incapacity/disconnect causes.

Controller recipients first receive a monotonic authority fence, then an order
carrying the detachment, roster revision, leadership term and sequence. The arena
checks those fields along with the actual helm epoch and machine controller.
Target hints are re-keyed from the issuer's admitted unit into the recipient's
own classified contacts. Legacy side/wingman and persona commands cannot overwrite
an opted-in automated recipient. An unconfirmed queue offer remains pending;
queue acceptance records delivery separately from an execution outcome.

The OOC host is ephemeral. A live restart from a compatible format-18 snapshot
clears practice detachment objectives and authority; the player must explicitly
re-enable practice, which mints a fresh roster revision. Exact snapshot round
trips preserve arena fences for replay; the live restart boundary resets them.
The added authority state/input events bump snapshot format to 18 and the replay
engine stamp to `tacsim-engine-dt1b-v1`. Mind API 6 and cockpit frame 8 expose an
opaque order identity, with stock doctrine version 3 comparing that identity
instead of treating a standing deadline as an order id. This lets a new standing
objective replace an old unreachable/ignored objective in both native and scripted
minds. Legacy orders retain their existing deadline identity. Current content
envelopes and source pins follow the new ABI; historical identity controls remain
explicitly historical.

Deploying this cut over format 17 is a separate boundary: the existing OOC resume
policy refuses the older snapshot, restores units from their check-out snapshots
and records `aborted(restart)`. It does not migrate an active match to format 18.
The new replay engine stamp likewise refuses a recording from an older engine
for re-simulation instead of treating it as a current recording.

Saved API-5 source and envelopes remain immutable archival material and may enter
the existing retained Registry re-conformance path; they gain no new execution
authority. Fresh authoring saves require API 6. The shipped House content is
version 3 on the current ABI; this does not rewrite already filed revisions.
Existing IC execution still handles an ABI mismatch through its established
ROM/drop path, retaining the old seal and memory and journalling `MindDropped`
with cause `abi`. This slice adds no IC detachment persistence or restart policy.

## Following integration boundaries

The opt-in detachment path replaces AI-only side/wingman authority for its
automated recipients. `match_::cockpit::command_of` remains the legacy practice
path for other units. Existing `deliberation::Order` has a mandatory arena TTL;
the separate detachment lane carries optional expiry and never renews a fake TTL.
NPC/controller execution must use its own observation and station admission;
delivery of a target hint cannot grant a contact or bypass those checks.
The admitted order records its issuer's assigned unit as the hint's scope.
Reusing an objective after succession therefore cannot retain the old leader's
private contact scope. Player and NPC completion reports are trusted observations
in this layer; a future command endpoint must authenticate the reporter before
recording them. Acknowledging an order never invents an attempt or completion.

**Owner correction, 2026-10-29:** DT.2 resolves NPC eligibility from current
containment in the carrier, checked at preparation and launch (FO-16/Q-R2).
Neither a last-port manifest nor a recorded embarkation event/interval is required.
Crew are hired at port under contract records referencing real NPCs, subject to
injury and contracted mission scope (Q-R5). They board through ordinary movement.
Any later wages accrue from the contract, not aboard-time evidence; numbers are
content tuning. Assignment never generates a person from a software profile.

Follow [doc 26c's current delivery sequence](26c-detachment-preparation.md#current-delivery-sequence):
DT.2b3 hiring contract, ordinary boarding, DT.2b4 activation of the six slots
against current hull contents, then DT.3 launch. Docs 26g–26j are superseded and
must not be extended; their merged code remains in place for now. No movement
coverage programme or adoption of the inert v2 provider is a DT prerequisite.

DT.3 separates organizer, actual launch principal, field commander and operator.
The first three are not necessarily the same person. Roster creation or disbanding
does not open/close a hull epoch, recall a field unit, repair damage, or resolve a
casualty. Licensed unit minds need their actual production host; neither the OOC
adapter nor an NPC character's decision driver provides that authorization.

DT.3b supplies physical bay-to-surface departures and mobile ramp returns for
accepted operators already at real consoles with current helm leases. It retains
per-unit flight history and actual hull epochs across restart. DT.3c connects hired
NPCs to their assigned physical consoles and the existing control-handoff host.
The IC field command/observer driver and mining integration remain subsequent work;
these cuts do not claim the complete NPC mining story.

DT.3d connects the common succession model to accepted IC operations. Its bounded
checkpoint lives in the existing operation record, retaining command terms and
reconnect timers across restart while excluding cached authorization. Sim supplies
the actual K6 lease through an owner-attributed read; Org supplies current command
rights. Preparation reads both owners again and retains their typed preimages.
The flight tick and explicit `+sortie/refresh` reconcile current field placement,
physical consoles, unit condition, employment and actual session state. Advancing
the clock alone does not write another operation record. `+sortie/field` shows
only the last saved leadership identity and term, not private availability causes.

DT.3e connects fenced IC `ORDER`, `WILCO` and `NEGATIVE` commands to the existing
Propagation transport. The common bounded order checkpoint lives beside the
operation's succession checkpoint. One exact-action Propagation participant
enqueues real envelopes in the source action's commit; the existing delivery
tick supplies actual routing, serialization, service admission and interference.
No second transport scheduler or authoritative World copy of the operation is
introduced. Mail/channel reconciliation remains responsible only for its own
external source references. Field envelopes use existing global queue limits and
the 30 IC day retention window, then are compacted without erasing the source's
last confirmed delivered objective.

Each issue and delivery freshly reconciles actual field station/control,
employment, sessions and unit condition. Org command rights are read through
the authenticated owner service and fenced by actual permission revision keys;
becoming leader grants no missing asset right. All selected units pass before
any outbound message is committed. Delivery must match the exact source slot,
sequence, actors, revision and command term. Obsolete, missing or expired
transport cannot replace a standing delivery. Explicit optional expiry uses IC
seconds; no arena TTL is renewed to simulate an IC standing order.

The recipient projection shows only its own delivered messages. The issuer
projection uses actual outgoing transport references and arrived return packets,
never private recipient acknowledgements or inferred attempts/completion.
Repeated replies and pending/standing history survive restart; current session
and control facts still determine whether a saved pending message is admissible.
Player acknowledgements remain advisory and move no unit. ENGAGE carries only a
hint scoped to the issuer's own observation, without creating a contact share.

DT.3f connects current-leader mission-role grants, as described in
[26b](26b-unit-authority-and-grants.md#accepted-mission-roles). The accepted roster
digest and activation identify the role; current field facts select its eligible
holder, while Org retains the real finite lending lineage. Role declarations
use the existing issue/revoke coordinator path and are not personal rights.

At DT.3f, NPC execution was still required before claiming the IC command flow
complete. DT.3g supplies its physical surface host and DT.3h supplies the actual
delivered-order driver. The OOC adapter alone does not prove those IC paths.

DT.3g connects physical mining operators to the existing surface action host.
The actual accepted operator, current unit console, K6 helm holder, employment,
unit condition and OperateCrew permission admit each new action. The stock rental
rigs have real, initially unoccupied helm consoles in their authored cabs; adding
a missing console does not assign crew or take control.

For the existing leased-rig mining host, a player operates their own lease. A
hired Mining-duty NPC works its employer's actual leased rig on the accepted
carrier. The character retains its own chart and action identity; the real
employer/lessee supplies mining licence and tenure rights independently of field
leadership. Renting, hiring, organizing or holding command conveys no missing
unit permission. No NPC account, artificial lease or wage ledger is introduced.

The DT flight retains the hull epoch. Surface work carries its exact operation,
flight and operator as a causal link and never opens a second epoch. Bay charge
ends at actual departure. Surface recovery uses ordinary placement; the existing
flight observer confirms the return and closes its epoch. Loaded returns use
the existing queued unload after that confirmation. A returned flight cannot
restart field work through the surface host; its next departure uses DT launch.
Station/control loss or disabled controls prevent further physical work, while
the actual admitted work can still settle to a refusal. Individual drive, scan,
sound, mine, recover, dump and unload actions are supported; the carrier's
multi-trip teleoperation plans retain their existing separate admission.
Their idle recall also remains on the teleoperation path; an idle physical slot
does not pilot itself home.

DT.3h adds a bounded NPC decision pass to the existing surface tick, sharing
its actual-principal submission budget. It discovers accepted operators through
the existing flight index and executes only orders delivered for that exact
flight/site. Old messages without flight coordinates remain advisory. No new
movement scheduler, personnel ledger or licensed-mind substitute is introduced.

The physical Mining crew supports HOLD, ADVANCE, WITHDRAW and finite MINE
[family] objectives. MINE uses the operator's own scan/sounding coverage, chooses
among at most eight nearby known ore pockets, admits one real extraction cycle,
then recovers and unloads. A failed receipt or no reachable known ore produces a
real NEGATIVE reply. A replacement/expiry stops the old mining run after its
current cycle; already admitted motion still settles physically. Unsupported
combat/support objectives return NEGATIVE. Player control remains advisory.

One causal link on the existing rig record identifies the delivered sequence,
accepted flight and actual act ordinal. Source admission stamps Attempted in the
same Tacsim transaction as real work. Matching actual drive/recovery or ore
unload receipts alone stamp Completed. WILCO is neither authority nor execution.
Stale receipts cannot complete a replacement order; the source history survives
restart. Replies still use real Propagation envelopes, and the issuer view does
not expose private recipient work as instantaneous telemetry.

DT.3g validation covers all 27 production preparation/launch/field stories,
including a real leased-rig NPC mining and unload flow, permission loss at final
take, station loss without a reading or charge spend, charge settlement for
movement already completed, and no implicit recall of idle physical slots.
The 183 surface library tests, 26 legacy mining tests, 30 excursion tests and
27 source-policy tests passed. Strict Clippy and the unchanged six-package M7
memory gate passed, including all targets for the memory checks.

DT.3h validation covers 31 production preparation/launch/field stories, including
a two-member player-leader/hired-NPC mining trip across restart, refusal before
message delivery, replacement stopping, unsupported/expired tasks, and actual
ADVANCE/WITHDRAW movement completion. The 183 surface library tests, 26 legacy
mining tests, 30 excursion tests and 27 source-policy tests passed. Strict Clippy
and the unchanged six-package M7 all-target memory gate passed. This proves the
physical player/hired-NPC mining milestone; licensed autonomous-mind field hosts,
H1/H2/H3 remain separate workstreams. DT.4 subset delegation is deferred;
DT.5 follows through the existing communications/ECM path.

DT.3i ships persistent mining/recovery crew at Ceres's Slack Line through the
existing NPC and content-latch owners. Ilya Mendes offers subordinate service;
Sana Okonkwo offers commander-capable terms. The crew role authors only standing
consent on those actual people. Hiring still uses its one source contract write;
boot and content updates preserve the hired person's identity, location and
memories rather than replenishing a pool. The new pack defers while a kept world
still has an older Ceres dependency. Three content tests, 13 production hiring
stories, all 31 DT stories, 24 station tests, 27 source policies, strict Clippy and
the unchanged M7 gate passed. Normal rental equipment still needs an explicit
owner-issued operating/deployment/command loan; a rental record alone grants none.

DT.3j supplies the normal rental counter's separate operational lending agreement.
The real lessor NPC issues all four unit grants in one ordinary Org source action,
conditional on the exact accepted economic lease. The lessee can lend crew use
onward to their hired NPC through that actual source grant. Preview grants nothing;
lease termination or loss of any lending root before final take rejects the whole
agreement. Revoked loans stay revoked on retry. No new source family, movement
ledger or NPC account is introduced. The mining stories now use shipped Ilya
Mendes and actual counter loans instead of staff-authored offers or staff
operational grants. Normal player station boarding and disembarkation remain
the next usability cut. Validation: 35 production DT stories, 24 Org tests,
2 Economy authority tests, 49 rental/refit stories and 27 source-policy checks
passed, together with strict Clippy, formatting and the unchanged M7 memory gate.

DT.3k completes the normal physical player/hired-NPC mining flow. A captain
boards their own rented rig with `+tv/<n> board`, personally chooses focus and
requests its actual K6 control lease. Returned crews leave through `+tv/<n> leave`
and `+sortie/unseat <npc>`. These use the ordinary Sim interior and billet writers.
Voluntary control release uses the existing handoff state machine's vacant target
through an owner-only source action; physical exit waits for that committed
release. An open ramp flight must settle or be explicitly cancelled first.
Rental return and charter repossession check current local room contents before
moving a rig to the shed. There is no new personnel history or movement provider.

The production mining story now uses two actual stock rentals, real lessor loans,
shipped Ilya Mendes, ordinary player boarding/focus/handoff, and an accepted Mining
roster. Actual radio delivery starts NPC mining; a restart preserves the running
job, the real unload completes it, both rigs physically return, both operators
release control and exit, and coordination closes. Staff-created leader units,
staff-issued operational grants and staff-authored NPC offers are absent from
that story. Site/time/geology boot fixtures still supply the deterministic test
world; the established voyage host owns sailing to and from the site.

Validation on current `origin/main` at `a1ddefaa`: 38 production DT stories,
55 station/proxy integration tests, 49 rental/refit stories and 27 source-policy
checks passed. Strict Clippy, formatting and the unchanged six-crate M7 all-target
memory gate passed as well.

This closes the DT.1–DT.3 physical player/hired-NPC mining milestone. Licensed
autonomous-mind field hosting and H1/H2/H3 retain their separate briefs. DT.4
attachments are deferred; DT.5 dedicated communications remains the follow-on
described below. No deferred locomotion work is a prerequisite for taking these
crews mining.

## DT.5 communications integration

Owner direction, 2026-10-30: defer DT.4 attachments/subset command delegation;
retain DT.5 on top of the communications work that integrates ECM. This is an
implementation scope, not a claim that the delivery and ECM integration is complete.

Provide a dedicated detachment communications surface for member coordination,
orders and replies. Reuse the existing IC channel/propagation transport from
[doc 15](15-comms-relay-backbone.md) and the existing DT order/response path.
Keep their distinct message semantics: a chat line does not become an order,
and channel membership does not grant command authority. Do not build another
relay, queue, jamming model or personnel ledger for this feature.

Delivery remains per recipient, subject to the real communication path's lag,
ECM/jamming, outages and routing. Acknowledgements must travel back; issuing an
order or locally generating a reply does not prove its remote arrival. Members
see only delivered messages and knowledge available to them. Lack of a reply
must not reveal hidden positions, damage, connectivity causes or casualties.

Acceptance uses a real detachment and the gameplay communications/ECM integration:

- Members exchange detachment text over actual IC delivery, with different arrival
  times where their communication paths differ; outsiders receive no messages.
- Jamming one recipient delays its traffic without stopping reachable members.
  That unit retains its last delivered standing order. A jammed reply path leaves
  the issuer without an acknowledgement until the reply actually arrives.
- Restoring communication delivers valid pending traffic under the existing
  transport's expiry and supersession rules. Order revisions and command terms
  still prevent obsolete orders from replacing the current valid order.
- Restart during delayed traffic preserves pending delivery without duplicate
  visible messages. The player-facing surface distinguishes local submission
  from received messages and acknowledgements without exposing hidden telemetry.

DT.4 delegation and a broader electronic-warfare redesign are not prerequisites.
If gameplay ECM is not yet connected to this delivery path, the required work is
that narrow shared integration, with evidence from actual deliveries rather than
a separate detachment-only jam flag.

## DT.1a verification

Validated locally on 2026-10-28:

- `cargo test -p mush-tactics -p tacsim --lib detachment::`: 21 tests passed,
  including missed grace deadlines, reconnect hysteresis, permission revocation,
  revision/term/sequence fences, slot reassignment, explicit completion and
  successor contact scoping.
- The complete `mush-tactics` library suite passed during implementation.
- `cargo clippy -p mush-tactics -p tacsim --lib -- -D warnings` passed.
- Formatting and `git diff --check` passed; independent review findings were
  resolved before the final targeted test run.

These are library and OOC adapter checks. No production `+sortie` flow, physical
launch, NPC hiring, persistent operation recovery or live client acceptance is
claimed by DT.1a.

## DT.1b verification

Validated locally on Windows on 2026-10-28:

- The full TacSim library suite passed: 1,905 tests, four ignored.
- Integration shard 4 passed all 488 tests, including the format-18 snapshot
  shape. Shard 6 passed 240 tests; its three stale frame-schema assertions were
  corrected and each passed on rerun. Both production detachment tests passed:
  advisory player acknowledgements, actual automated delivery, stale fences and
  legacy-order refusal.
- All 150 mind getters passed their keyframe parity check. The four-row
  deterministic corpus, 30 native/Rhai whole-match plays and 45 actual worker-pool
  matches passed. The corpus's stopping times and independent float-probe bitmask
  stayed unchanged. Linux reproduction belongs to the PR's CI checks.
- The arena and four mind harness cohorts completed without integrity failures.
  Their deterministic comparison changed exactly five fields: four final hashes
  and the encoded ledger byte count. The declared update and retained performance
  provenance are recorded in [17b §7](17b-unit-minds-implementation-plan.md#7-dt1b-declared-encoding-re-bless--2026-10-28).
  These debug runs do not establish performance acceptance.
- The web type check passed with no warnings; 104 focused decoder and cockpit
  component tests passed. SSH detachment decoder and slot-key tests passed.
- All five production command-registration contract tests passed.
- `cargo clippy -p tacsim -p mush-net-ssh --lib -- -D warnings` passed.
- Formatting and `git diff --check` passed; the final independent review found
  no blocking authority, delivery or replay issue.

IC authority, personnel, preparation and durable operation history remain the
following slices; these results cover the production OOC practice integration.
