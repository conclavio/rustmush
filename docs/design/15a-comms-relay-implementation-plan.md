# 15a — Relay backbone implementation plan

**Draft 0.4 — 2026-10-23.** Initially grounded against `origin/main` at
`7c886408c1a1d4d0283b886bdd62fbbe2f8fe7af`.
Parent: [15 — Communications](15-comms-relay-backbone.md).
This is the implementation plan; [15b](15b-comms-relay-cut-notes.md) is the build
and evidence record. R1 is pushed to main. The owner authorized R2 after that
push; R2 is implemented and its dev1 live checks passed. Normal candidate CI is
still required for full acceptance under §5. After the CI repair's dev1 rerun
passed, the owner authorized Gate 3 / R5 and took over monitoring GitHub's long
CI run. R5 is implemented and its real-server gate passed; 15b records the results.
Full acceptance still requires normal candidate CI. The owner authorized Gate 4 /
R6a and automatic continuation to R6b after R6a's real-server gate and push. R6a is
implemented and its dev1 gate passed. R6b, R3 and R4 have now passed their live gates; the remaining authorized train continues with R7.

## 1. Owner rulings and delivery discipline

The owner's 2026-10-23 decisions close doc 15 §10:

| Question | Binding answer |
|---|---|
| Q1 Names | Malapert Exchange; Postmaster Solveig Adebayo-Krishnan; Sovereign, Priority, Standard, Deferred. |
| Q2 World-start coverage | Mercury included. No half at Venus, Titan, Pallas or Vesta. Aitken uses Malapert; Voltaire uses Phobos. |
| Q3 Concord leverage | Informal requests; the Exchange decides and owns the audit trail. |
| Q4 Mail | **All mail is IC and must transit. There is no OOC mail mode or instant-mail alternative.** This replaces both previously offered alternatives. |
| Q5 Congestion board | Mildly dishonest at world start, initially undiscovered; publication of evidence can open the scandal front. |
| Q6 Prices | Standard per message-unit; Priority a multiple; Sovereign flat; Deferred free with an IC-hours wait. Numbers belong to versioned seed data. |
| Q7 First mobile half | A Concord Fleet hull refitted at Luna and sent as the Belt Squadron's relief. None is active at world start. |

OOC **channels** remain outside IC propagation. Staff operations notices and system
notifications are not mail; an application that sends through `mush-mail` uses IC
transport, including staff-authored mail. `/urgent` marks a message; it is neither
an OOC bypass nor permission to buy Priority service.

**One milestone at a time.** Build its real producer, persistence, consumer and
player/operator surface together. Run its integration gate, repair failures, write
the evidence, then record the result and **commit/push that milestone before continuing**. The owner has authorized the remaining R4–R10 train automatically. Do not accumulate dependent implementation while a gate
is failed, skipped or awaiting its live test. Routine implementation choices inside
a milestone can be resolved and recorded without reopening the seven decisions.

The owner subsequently authorized automatic continuation from R6a to R6b, after
R6a's integration gate, commit and push. This changes that stop only; failed gates
still block dependent implementation. The owner subsequently authorized R3 and now the entire remaining R4–R10 train.

Every split milestone below retains its own integration gate. If another split becomes necessary,
each new part must still end in an observable, integrated result. A library, record
schema or mocked consumer alone is not an accepted milestone.

## 2. Starting seams and necessary corrections

These paths are the starting inventory; recheck them at each milestone's base commit.

| Current implementation | Consequence for this programme |
|---|---|
| `crates/mush-propagation/src/{lib,record,pass,context}.rs`: world-clock delivery tick, bounded envelope store and reaper, boot reconciliation, orders/reports/acknowledgements | Extend the existing delivery owner. Do not create a second message scheduler in mail or comsys. |
| `record.rs`: 512-byte envelope payload, four-hop cap, self-recipient refusal; `pass.rs::settle_arrival`: newest revision supersedes earlier traffic between an issuer and recipient | Explicitly distinguish orders, reports, mail and channel messages. Ordinary messages must not supersede each other. Preserve order behavior and old stored envelopes. Mail-to-self needs a legal local delivery. |
| `pass.rs::step_envelope`: advances by delivery outcome/hop count; it does not implement a strategic per-leg due-time itinerary | R2 must prove actual not-before timestamps, not merely fill in a lag field. Read-count bounds and hop-count bounds are separate. |
| `crates/tacsim/src/sky.rs::Sky::light_lag_s`, `port/voyage.rs`, `port/families.rs`, `mush_sim::CarrierJourney` | Stationary strategic light-time is available. Ship communication must use the actual frozen ticket and journey; do not merge the two travel records or invent a moving ship coordinate. |
| `crates/mush-mail/src/cmds.rs::deliver` immediately creates recipient inbox entries and notices, with a durable-ack gate | Replace send-time delivery with durable dispatch plus arrival-time mailbox insertion. Audit every send/reply/forward path and every inbox/query/notification reader. |
| `crates/mush-mail/src/data.rs`: 64 KiB bodies, 512-byte subjects, at most 32 recipients, mailbox quotas, sender copies | Preserve accepted content sizes. Store content with its owning module and route a bounded reference; do not squeeze a letter into the order payload limit. |
| `crates/mush-comsys/src/{speak,query}.rs`: immediate broadcast, a global history ring/sequence, unread projections, Discord relay output | IC delivery needs per-recipient arrival knowledge, including offline members. A delayed live event with instantly readable history would fail the milestone. |
| `crates/tacsim/src/composition/mod.rs`, `mush_server::boot::build_engine` | Wire every new seam in the shipping composition in the same milestone. Preserve dependency fences through injected adapters. |
| `crates/mush-server/tests/ic_slice_loop.rs`, `crates/mushd/tests/durability_e2e.rs`, `.github/workflows/ci.yml` | Reuse production boot, persistence/restart and real-process Redis testing. In-memory composition tests supplement the live gate; they do not replace it. |

Doc 15's topology means **five nodes, four pairs, eight halves**: four halves at
Malapert and one at each spoke. Older references to "five halves" counted sites.
Seed validators must check pair identity and two distinct halves, not accept the old count.

## 3. Shared contracts to establish early

These are implementation defaults for this plan, distinct from the owner rulings.
Record any necessary revision with its reason and tests before using it.

### 3.1 Ownership, dispatch and durable handoff

- `mush-sim` owns placed relay topology; `mush-propagation` owns routing and delivery;
  mail/comsys own their content and recipient views. TacSim composition supplies sky,
  journey, operator and economy adapters. No propagation dependency on TacSim, mail,
  comsys, economy, tactics or Discord; use narrow injected contracts at existing seams.
- Inventory each new record's owner, compatibility classification, writer, reader,
  durable boundary and boot behavior in the first cut that introduces it. Reuse
  existing kernel mechanisms; this programme does not silently perform all of E7k.
- Use a stable dispatch identity plus recipient identity, a typed message kind and a
  versioned payload reference. Preserve legacy order decoding and revision semantics.
  A transport reference does not grant permission to read the underlying content.
- Cross-owner handoff is a durable, idempotent protocol: the source persists its
  dispatch intent; transport accepts it once; transport persists an arrival receipt;
  the destination applies that receipt once; completion permits bounded cleanup.
  Every boundary must be replayable after a crash. Do not mutate another owner's
  typed state or use an in-memory callback as the only record of delivery.
- Local submission acknowledgement means **queued durably**, never "recipient has
  received it." Delivery/read/failure information learned remotely must return over
  transport before appearing to the sender. Staff diagnostics may inspect truth
  under staff permissions; player projections may not.
- Keep an undelivered payload while any recipient still needs it, even if its sender
  deletes the Sent copy. Reserve bounded pending capacity, reject overload before
  acknowledging acceptance, and reap only after all relevant handoffs complete.
  No unbounded fanout vector, lifetime-only cap or silent message drop.

### 3.2 Time and routing

- Use the world IC clock and generation handling, never a wall-clock sleep per
  message. Freeze each transmitted leg's endpoints, metric inputs and due time.
  A later sky/config edit cannot accelerate that in-flight leg.
- Reach an eligible node conventionally, cross live pairs instantly, and reach the
  recipient conventionally. Queue and service time are separate from physical lag.
  Carry causal entry/exit stamps, route version and stable tie-breaks.
- R2 includes the stationary part of the R5 metric adapter so its first real route
  has real strategic light-time. R5 then adds ship progress and endpoint changes.
  Unknown positions wait with a bounded expiry; they never turn into zero lag.
- Choose the nearest reachable eligible ingress/egress with deterministic ties;
  walk the permitted pair graph under explicit node, edge and hop limits. Distinguish
  an unreachable route from a work-budget continuation. Persist scan/queue progress.
- Queued traffic revalidates policy and route before transmitting its next leg.
  Completed legs are never replayed. A lost next hop uses a valid conventional
  fallback or store-and-forward; a half's destruction is not a teleport or an ACK.
- R5 freezes a conventional leg using the ship's current progress on its frozen
  passage legs and their endpoint bodies. At arrival, revalidate the addressed
  character's receiving endpoint; movement can require a new bounded forwarding leg,
  but cannot retroactively deliver a message to a departed place.
- Keep tactical metric units and astronomical kilometres distinct. Quantize positive
  physical lag upward to the existing supported IC-clock resolution; a pair's own
  transit is zero. Document cadence rounding rather than promise subsecond delivery
  on a one-second coarse tick. Pausing or changing world rate preserves IC due times.
- Bound work on successful and unsuccessful lookups and keep hidden-node failures
  indistinguishable in player output. Test lookup/read budgets; a plain cap on the
  final route length is insufficient. Legitimate physical arrival differences remain.

### 3.3 Mail, channel history and bridges

- All compose/quick/reply/reply-all/forward and programmatic mail paths submit through
  transport. Existing mail is migrated as already delivered; do not resend or erase
  historical messages. This compatibility treatment is not an OOC mode.
- A sender can read their own queued content. A recipient gets no inbox entry,
  subject, unread increment, search/thread exposure or `MailNotice` before arrival.
  Query/UI hydration and reconnect must enforce the same boundary as live output.
- Recipient locks, ignore policy, destruction and mailbox capacity are checked at
  acceptance/arrival as applicable. A refusal discovered at the destination returns
  as an IC failure notice, not an immediate remote-state oracle. Offline characters
  receive at their persisted in-world location. An unplaced address waits.
- Mail-to-self still uses dispatch/arrival settlement, with a local physical route.
  Preserve mailbox limits and define pending-slot reservation/release so concurrent
  arrivals cannot overfill a mailbox or strand quota after expiration.
- Short eligible mail can use a relay; large mail uses the conventional path under
  a versioned size threshold. The whole letter becomes readable only on completion.
  Preserve the existing 64 KiB content limit; initial implementation needs no new
  attachment format, unbounded fragmentation or generic bulk-transfer service.
- IC channel recipients are eligible members at dispatch, with bounded persisted
  fanout and arrival authorization checks. New membership does not grant instant
  access to undelivered historical traffic. A leave/rejoin must not skip transit.
- Track received messages/holes, not just a highest global sequence: later messages
  may arrive before earlier ones. Recall, backlog, unread counts, notifications and
  reconnect expose only messages that this recipient has received. Do not let gaps
  reveal the count of hidden traffic.
- A linked Discord speaker uses their character's in-world endpoint. Outbound IC
  content uses the authorized observer's delivered stream. A shared Discord channel
  cannot represent viewers with different delivery knowledge: its IC mirror must
  remain disabled unless all viewers are represented by one explicitly authorized
  in-world endpoint. Per-observer scene/DM delivery remains usable. OOC bridges keep
  their existing behavior. External-provider retries must not duplicate world delivery.

### 3.4 Capacity, classes, money and disclosure

- Introduce bounded FIFO link capacity with R2; R3 extends that same queue with
  classes. Define message-units from actual content bytes and freeze the accounting
  rule/version, never charge only the tiny payload reference.
- R3's seed policy defines byte units, rate/burst limits, short-message threshold,
  declared waiting time and the Deferred off-peak window. Standard cannot bypass
  physical capacity because its published board claims a shorter wait.
- Each link transmission consumes capacity once; a shared hub scheduling queue must
  not charge the same hub traversal twice. Conservatively charge capacity per routed
  recipient copy unless a proven multicast implementation deduplicates it.
- Effective service is checked for the principal legitimately represented at each
  endpoint, under that link operator's policy. Acting for an organization requires
  existing scoped authority; a private login account is not an IC principal.
  Denial at a receiving endpoint is not bypassed by choosing a different payer.
- Default throttling ladder: Sovereign → Priority → Standard → Deferred, with Deferred
  staying Deferred. Suspension removes that operator's backbone service, not
  conventional physics or an independently owned private link.
- Before R8, class entitlement is explicitly operator-granted and no payment is
  claimed. R8 replaces grants for purchased products with real economy entitlements.
  No placeholder debit, invented balance or "billing integrated later" paid service.
- R8 freezes price/entitlement terms at acceptance, uses existing accounts, holds,
  obligations and ledger receipts, and settles each purchased service once. An
  unsent cancelled/expired paid dispatch releases its reservation; consumed service
  follows the published tariff's refund terms. Restart cannot charge twice.
- Public congestion is distinct from private actual load. Only authorized operator
  access sees traffic that genuinely traversed that operator. Operator inspection,
  discovery and publication are separate permissioned acts; generic player panels
  never expose payloads, secret taps or true queue load.
- Staff state changes and commercial changes need attributed durable audit records.
  No new promise of encryption: content protection is a later T15 extension.

## 4. Milestone sequence and integration stops

The sequence preserves the original R identifiers. R6, R8 and R9 are split because
their halves have independently testable outcomes. **All thirteen rows are gates.**

| Gate | Scope | Observable result at the stop |
|---|---|---|
| 1 — R1 | Registry and first seed slice | A real server boots, inspects and reconciles one installed pair. |
| 2 — R2 | Routed delivery; stationary metric from R5 | Existing orders/reports traverse that pair or its timed fallback. |
| 3 — R5 | Ships underway | The same traffic reaches a real travelling hull with journey-derived lag. |
| 4 — R6a | All mail over transport | A normal letter queues, survives restart and reaches its recipient later. |
| 5 — R6b | IC channels and observer-safe bridges | Two members receive the same line at their own arrival times. |
| 6 — R3 | Classes, congestion and capacity | Real traffic demonstrates the public-board delay and Priority advantage. |
| 7 — R4 | Operator service policy | An audited suspension causes real traffic to take fallback. |
| 8 — R7 | Physical loss and consequences | Damage kills a half, orphans its partner and affects live traffic. |
| 9 — R8a | Exchange and per-message sales | A player buys expedited transport with conserved money. |
| 10 — R8b | Subscriptions, treaty and arrears | A real obligation changes service state and recovers after payment. |
| 11 — R9a | Manufacture, ship, install | Conserved industrial output becomes a working private pair. |
| 12 — R9b | A mobile half | An installed half moves with a real hull and extends coverage. |
| 13 — R10 | Full world-start pack and campaign acceptance | The complete Luna star and its political/economic story work together. |

### Gate 1 — R1: registry, composition and a small real seed

Build pair/half/node identity, installed bindings, coverage, capacity, operator and
Live/Degraded/Dead state. Powered/staffed availability is initially an explicit
operator-controlled input; crew simulation is not invented as a dependency.
Preserve the existing monolithic-record/boot-reconciliation approach unless the cut
documents a required change. Provide bounded, permissioned diagnostics and the
minimum mutation surface needed to test install/disable/repair; do not expose secrets.

Bring forward R10's smallest real content: Malapert's operator identity and a
Luna–Ceres pair bound to real places, through the production pack pipeline. A test
that constructs a registry without booting that content is not enough.

**Integration gate:** fresh and existing-store boot; inspect both installed halves;
disable and re-enable one; restart; identical pair identity/coverage. Duplicate
halves, contradictory bindings and corrupt records are refused or held with an
actionable diagnostic, never silently reseeded. Verify bounded inspection and no
traffic regression. **Stop after recording this evidence.**

**Result (2026-10-23):** implemented at `916e8fee`; 262 targeted tests passed,
plus real dev1 upgrade/fresh boot, TLS WebSocket and SSH commands, confirmed-write
SIGKILL recovery, and four bad-record boot refusals. The pair is restored to Live.
See [15b](15b-comms-relay-cut-notes.md) for transcripts, deployed binary hashes and
the normal-CI gap and subsequent CI correction. The owner then authorized R2.

### Gate 2 — R2: one complete routed delivery

Add durable itinerary/due-time state, bounded graph lookup, the stationary strategic
metric adapter, minimal FIFO capacity, conventional fallback and route diagnostics.
Keep the existing order/report/ACK producers and consumers connected. Establish the
typed dispatch-kind boundary now so R6 cannot inherit order supersession.
Prove the existing four-hop envelope limit accommodates the counted itinerary;
separate link-hop and conventional-leg limits explicitly where required.

**Integration gate:** on the production server, issue a legitimate order/report
between endpoints served by the seeded pair; show no recipient effect before the
computed due time and exactly one effect afterward. Return the ACK over transport.
Disable the pair and repeat over a measured conventional fallback. Kill/restart
while queued and while a conventional leg is in flight; replay neither loses nor
duplicates delivery. Exercise a cycle, missing route and full queue. **Stop.**

### Gate 3 — R5: communication during a real passage

Wire the actual port ticket, frozen passage legs, hull containment and journey
progress through the metric adapter. Preserve docking/arrival transitions and
expose an honest player-facing lag estimate. Do not substitute a staff-teleported
test object for a ship that left through the existing departure workflow.

**Integration gate:** depart a Dray through the real trade path; exchange traffic
shore→ship and ship→shore at departure, mid-leg, a leg boundary and docking.
Verify nearest-end selection, recipient movement/forwarding, missing ticket
handling, a world-rate change and restart mid-passage. OOC channel messages remain
immediate; no remote ACK arrives early. **Stop.**

### Gate 4 — R6a: all mail is IC

Implement the durable source/transport/destination handshake and arrival-only inbox
visibility from §3. Existing mail commands keep their purpose, with truthful queued
and transit status; no `/ooc` or parallel instant send path. Update the line help,
mail queries, TUI/web mail presentation and notification integration in this cut.

**Integration gate:** two actual characters send, reply, reply-all and forward
between Ceres and an uncovered destination, then to a character aboard a moving ship.
Test self-mail and a large letter too. Before arrival, probe every recipient read
surface and reconnect; none exposes the letter. Kill between source acceptance,
transport acceptance, arrival receipt and mailbox insertion; each letter arrives
once, with one unread increment. Test offline recipient, full mailbox, changed lock,
destroyed recipient, transport saturation, expired payload and delayed failure reply.
Upgrade a store with existing mail; old inboxes remain readable and are not resent.
Run real SSH and web mail clients against the server. **Stop.**

**Result (2026-10-23):** live gate passed, including upgrade, four durable crash
boundaries, all send paths, full-sized/offline mail, arrival refusals, saturation,
30-IC-day expiry and a real Dray exchange. The gate found and repaired the missing
public `@lock/mail` switch. [15b](15b-comms-relay-cut-notes.md) records candidate
provenance and results. The owner authorized automatic continuation to R6b after
this push; normal GitHub CI remains owner-monitored.

### Gate 5 — R6b: IC channels and bridge boundaries

Route only IC channels through the same transport, with bounded fanout and durable
per-member receipt knowledge. Change recall, panel/backlog, mentions, unread and
reconnect together. Preserve join/speak/see permissions and ordinary OOC comsys.
Audit all `Audience::Relay`/Discord paths; apply §3.3's observer rule.

**Integration gate:** one covered member and one uncovered/underway member receive
the same line at different legitimate times. No early recall, unread count or mention.
Send a second message over a faster route and prove receiving it does not expose the
first. Test offline catch-up, leave/rejoin, revocation, source destruction and restart
during fanout. Exercise Discord ingress/egress via the real bridge with a controlled
provider adapter, and the supported live integration when credentials are available;
an unavailable live bridge leaves this acceptance item open. OOC chat still works.
**Stop.**

**Result (2026-10-23):** implemented. Dev1 passed 68 authenticated TLS WebSocket
checks and five SSH checks, including upgrade, restart during fanout, overtaking,
offline/uncovered delivery, membership boundaries, arrival revocation, real
underway-hull traffic and controlled source-object loss. The real bridge passed
controlled-provider ingress/egress and retry checks, including reply modals.
Live Discord remains an open acceptance item: dev1 has no enabled bot or relay
mapping. Normal CI remains owner-monitored. [15b](15b-comms-relay-cut-notes.md)
records provenance and limits. The owner subsequently authorized R3.

### Gate 6 — R3: classes and a deliberately dishonest board

Extend R2's queue with real capacity accounting, class entitlement, public declared
delay, private actual load, timestamps, Deferred windows and bounded fair draining.
Publish versioned tuning values in the developing seed pack; Q6's currency prices
remain for R8. Add the permissioned evidence/publication path for the congestion
scandal, using the existing knowledge/canon/front mechanisms rather than a raw-data
dump. Bring forward the relevant R10 front content here.

**Integration gate:** saturate the real link with mail/channel/order traffic.
Sovereign/Priority obey actual load; Standard observes the extra declared delay;
Deferred is free-class traffic and waits for its configured window. No class exceeds
capacity, no retry double-consumes it, a busy head cannot starve unrelated eligible
work, and hub transit is accounted consistently. Ordinary players can compare their
own timestamps but cannot query actual load. An authorized publication opens the
scandal front once. Restart with all four classes queued. **Stop.**

**R3 status (2026-10-23):** implemented on runtime candidate `97b4106b`.
All 52 Linux boot/relay/passage tests, targeted lint, fresh Redis boot/reboot and
upgraded dev1 WebSocket/SSH gates passed. All four queued classes and publication
survived actual child SIGKILLs; 13 mixed producer copies drained within the shared
capacity. [15b](15b-comms-relay-cut-notes.md) records the evidence. Normal CI is
owner-monitored. **R3 gate passed; remaining R4–R10 implementation is authorized.**

### Gate 7 — R4: service state and operator authority

Add Served/Throttled/Suspended by legitimate IC principal, scoped operator authority,
reasoned audit events and player-safe status. Use existing organization permissions
now; a temporary global staff-only table must not become the permanent ownership
model. Apply changes to not-yet-transmitted hops; preserve already-consumed service.

**Integration gate:** authorized operator throttles and suspends an actual sender
and receiver in separate cases. Traffic demotes or falls back, including traffic
already queued; neither the receiver nor a payer alias bypasses denial. Restore
service and test another operator's independent link. Refuse unauthorized changes.
Restart across a policy change; the reason and routing result agree. **Stop.**

**Result (2026-10-23):** runtime `c5a393b1` passed 55 Linux production stories,
419 local owner tests, 23 contracts, and the repaired real Redis/process-death
regression. Dev1 passed fresh/upgrade/restart, 85 WS commands, six SSH checks,
sender/receiver restrictions, scoped authority and a controlled independent-link
fixture. The seed was restored with zero pending traffic. Evidence is in 15b;
commit/push precedes automatic continuation to R7. Normal CI remains owner-monitored.

### Gate 8 — R7: physical loss and political consequences

Bind installed relay nodes to checked structural unit definitions and the actual
TacSim damage/custody pipeline. At the cut's preflight, identify the live writer that
commits damage back to the persistent node; the retired legacy fire lane is not an
integration seam. Include any required narrow adapter in this cut. Destroying a half
durably kills its pair and orphans its counterpart.
One causal event drives canon, the Exchange's anathema, faction/front effects and the
available insurance-policy consumer. Missing consumers are a named part of this cut,
not proof that emitting an unused event completed it. Add the bounded `attention`
pool capability; seed no attention-seeking actor by default. Test with controlled
content using the normal faction APIs.

**Integration gate:** inflict real damage on a seeded node; queued traffic uses
fallback and the distant counterpart remains orphaned across reboot. The consequence
chain fires once, including service denial and a meaningful insurance response where
cover exists. Replaying destruction cannot repeat penalties or mint attention.
Repair does not resurrect a destroyed entangled pair. Also exercise disablement
without destruction and destruction during a pending leg. **Stop.**

### Gate 9 — R8a: the Exchange sells a message-unit service

Add the Exchange's real funded accounts and the narrow E8 product needed here.
Implement Standard purchases and Priority upgrades with authorized payer selection,
versioned unit/price quotes, hold/settlement/release and an auditable receipt.
Keep Deferred usable without a paid account. No broad marketplace rewrite is needed.

**Integration gate:** a player buys delivery through the normal command/UI, a real
ledger entry pays the Exchange, and the message uses the purchased service.
Compare content sizes and multiple recipients against the published unit rule.
Insufficient funds grants no paid entitlement; duplicate submissions/restart do not
double-charge; expiry before service releases the hold. Kill on each side of the
money/transport handoff and reconcile the actual store. Money is conserved. **Stop.**

### Gate 10 — R8b: subscriptions, Sovereign treaty and arrears

Use existing obligations/contracts and the IC cadence for subscriptions and treaty
eligibility. Implement the specified arrears → Throttled → Suspended policy, with
quoted terms, notices, cure and recovery. Establish source-specific restrictions:
paying an invoice clears arrears, never an incident sanction or anathema.

**Integration gate:** enroll real person/org principals, run the billing boundary,
miss payments, observe actual delivery changes, pay and recover. Test treaty expiry,
two concurrent restriction reasons, clock changes and restart around settlement.
Repeated cadence fires cannot bill twice or clear the wrong suspension. **Stop.**

### Gate 11 — R9a: manufacture and deploy a private pair

A real gated foundry recipe consumes finite inputs and authorized know-how; its
completed work order yields a unique pair with two accountable cargo halves.
Use existing industry, inventory, title/custody and journey mechanics. Shipping
and installation establish the link; creating output never installs it remotely.
Private pairs have their own operator policy and lower seeded bandwidth.

**Integration gate:** submit and complete a real work order, ship a half on a real
carrier, install the pair at distinct places, and deliver a letter without Luna.
Verify conservation, title/installation authority, interruption, stolen/destroyed
cargo and restart at manufacture/install boundaries. Consume the discovery event
in the anti-proliferation front once. No free pair mint or staff-only recipe path
counts as the finished product. **Stop.**

### Gate 12 — R9b: move a half with a hull

Implement the minimal checked hull capability and installation/refit path, moving
coverage and removal/destruction lifecycle. This is not the player unit-builder
programme. Exercise the accepted Concord Fleet relief-hull story through the front
and the real refit/launch consumer; keep it dormant in the world-start pack.

**Integration gate:** refit, launch and sail the hull; real onboard and eligible
nearby traffic uses its half, while out-of-coverage traffic does not. Restart in
passage, transfer/disembark a recipient, disable the installation, and destroy the
carrier in a controlled test. The link dies consistently and canon records the
first launch once. **Stop.**

### Gate 13 — R10: complete the Luna star

Finish the five-node/four-pair/eight-half pack: Malapert, Phobos, Ceres, Kirkwood and
the Mercury stub; the Exchange, Postmaster, coverage, published tariffs, mildly false
board and the two original fronts. Keep Titan/Pallas/Venus/Vesta without halves;
keep private/mobile pairs absent at start. Reuse identities/content introduced by
earlier cuts. This milestone completes content, not previously deferred wiring.

**Integration gate:** fresh and upgraded worlds tell the same story without duplicate
assets or capitalization. Two players use mail and IC channels across the full star,
an uncovered destination and a trade voyage; observe class differences, arrears and
a controlled outage; restart midway; restore service. Test both all-in-one and split
server deployments. Verify startup readiness/help and run a measured bounded-load
soak against realistic concurrent traffic. The final scenario reruns the programme;
it is not its first integration test. **Stop for programme acceptance.**

## 5. Mandatory gate evidence

Every milestone must provide all applicable layers below on the same candidate
commit. Unit tests alone, a compiled executable or green CI with skipped tests is
not acceptance.

1. **Focused contracts:** deterministic routing/time math, permission failures,
   queue/record bounds, legacy decode and the cut's failure paths.
2. **Production composition:** boot through `mush_server::boot::build_engine`, load
   the actual growing seed pack, drive normal commands, advance the IC clock and
   observe the real consumer. Use the production persistence/commit drivers. Do not
   patch private state to manufacture a successful ending.
3. **Persistence and real process:** run a candidate server against an isolated
   Redis 8 test database; connect actual clients; exercise the milestone's happy path
   and outage/restart path. Kill at a meaningful durable boundary. Test both fresh
   and upgraded state when schemas change. Never target production data for this gate.
4. **Surface parity:** line output and queries agree; test affected SSH/TUI/web and
   Discord paths in the milestone that changes them. Existing ordinary chat, tactical
   orders and trade travel remain useful regression probes.
5. **Operational evidence:** queue depth/oldest age, records visited, per-fire work,
   delivered/expired/refused counts and relevant timing are bounded and inspectable.
   Establish measured baselines in R2 and repeat affected measurements; do not invent
   a performance target by silently increasing limits until a test passes.
6. **Build/CI:** format, targeted tests and lint; run the repository's applicable
   integration/contract gates and normal CI on the candidate. Add every required
   Redis-gated test to the actual CI job/inventory so absence of `REDIS_URL` cannot
   become a silent pass. An unavailable required environment is **not run / gate
   incomplete**, not accepted with integration deferred.

Reuse a small cumulative relay story suite rather than copy a large harness for
each cut. R1 established `crates/mush-server/tests/relay_registry.rs` for production
boot, command and persistence contracts, and `scripts/relay-r1-live.mjs` for real
TLS WebSocket acceptance against the split gateway/server. Its VPS fault and SSH
procedures are recorded in 15b. The Rust target is part of the normal workspace
test discovery; the live script is an explicit manual gate, not a Redis-gated test
that silently passes without a server. R2 must add its routed-delivery stories and
automated real-process fault tests to the applicable CI inventory. Keep test
support separate from shipping paths and deterministic clock advancement separate
from real-process durability proof.

Each milestone's evidence record must name:

- base and candidate commits; source/data/schema changes; implemented producer and consumer;
- exact commands, environment, pass/fail/skip counts and live transcript/log paths;
- seed version, IC rate, route/due-time expectations and observed arrival/ledger results;
- restart/fault boundaries exercised, upgrade result and measured bounds;
- defects found and resolved, any uncompleted criterion, and **accepted / not accepted**;
- the milestone result, commit/push and the owner's standing authorization to continue.

Acceptance means every required criterion is green. A material scope change or
waived criterion must be recorded as an explicit owner decision, not described as
completed work.

## 6. Scope boundaries and first action

This programme does not add travel gates, full uplink occlusion, entanglement-stock
depletion/resupply, encryption/forgery, a generic attachment system, a complete
multicrew simulator or all of E8. Existing OOC channels remain available. A missing
small adapter needed to complete a milestone belongs in that milestone; an unrelated
large dependency requires a revised, separately gated slice before implementation
continues.

The initial action was R1 only: registry, minimal Luna–Ceres seed, diagnostics,
real Redis/client testing and an owner review. The owner subsequently authorized
the R1 push and R2 implementation, then Gate 3 / R5 after the corrective dev1
rerun. The next authorization covers R6a and automatic continuation to R6b after
R6a's live gate and push. The owner subsequently authorized R3. **The remaining R4–R10 train is authorized, committing/pushing after each successful milestone gate.** The owner is
monitoring GitHub CI; every live milestone gate still must pass before proceeding.
