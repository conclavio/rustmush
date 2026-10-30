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

DT.4 subset delegation and DT.5 dedicated communications remain later cuts.
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
per-unit flight history and actual hull epochs across restart. Automatic hired-NPC
seating/control acquisition and the IC field command/observer driver remain the
next integration work; this cut does not claim the complete NPC mining story.

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
