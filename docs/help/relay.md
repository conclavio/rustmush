# Relay service and topology

The Luna star carries IC mail, IC channel copies, orders, contact reports and returning acknowledgements.
Malapert is the hub of four separate pairs: Phobos, Ceres, Kirkwood and the Mercury stub.
Aitken Highport uses Malapert; Stickney and Voltaire use Phobos. Titan, Pallas,
Venus and Vesta have no halves. Spoke-to-spoke traffic consumes both links.
Conventional legs wait for their physical travel time. A disabled pair uses the
conventional route when positions are known; an unknown endpoint waits until it
can be located or the message expires. All mail travels IC; OOC channels remain immediate.


`+relay` shows public links, private links you may discover, and your class; `relay.board` is the matching
web query. `+relay/receipts` shows timestamps for your retained, arrived traffic.
Leaving a channel removes access to its receipts as well as its history. Private
link stamps also require current discovery rights.

The v1 policy has four classes. Sovereign and Priority wait for actual capacity.
Standard additionally waits 60 IC seconds from queue admission. Deferred waits at
least 7,200 IC seconds and transmits in the first 3,600 seconds of each 86,400-second
IC day. Old eligible work gains a scheduling rank every 300 IC seconds; smaller
eligible copies may pass a head that cannot fit the remaining capacity. The pair
shares one eight-unit bucket across directions; it never accumulates idle credit.
Delivery is checked on the existing coarse cadence, so these are earliest times.

New Exchange traffic defaults to free Deferred. Short queued copies can buy Standard at
1 cr per unit or Priority at 5 cr per unit. One unit is each started 128 content
bytes (including the subject for mail), up to 1024 bytes. Each recipient copy has
its own quote and charge. A route across multiple Exchange pairs pays once; each
pair still consumes physical capacity. Existing accepted traffic keeps its terms.

```
+relay/outbox
+relay/quote <envelope>=standard,<payer account>
+relay/quote <envelope>=priority,<payer account>
+relay/buy <quote>
+relay/purchases
+relay/cancel <quote>
```

Quotes freeze class, recipient, content units, tariff and payer for 600 IC seconds.
Buying reserves real funds; the account's normal debit and approval rules apply.
The first Exchange transmission settles the whole fee. An unsent reservation
expires after one IC day from quoting or can be cancelled for full release.
Consumed service has no refund. To upgrade unused Standard, cancel its reservation
and quote Priority. Repeating a purchase does not reserve or charge again.
The Exchange's initial capital comes from an existing finite clearing balance;
paid quotes wait until that transfer lands on the economy cadence.

## Availability, private links and official traffic

New pending data is limited to 32 recipient copies per actor. Explicitly represented
organization traffic also shares a 32-copy organization allowance across its officials.
Switching identity does not reset the actor allowance. Mail and IC channels each have
48-copy producer limits and share 64 external-data slots. All data together may use
112 of the 128 pending slots; the remaining 16 are reserved for returning mail receipts
and order acknowledgements. Unresolved source intents count before handoff and after
restart; they are not a second free queue. Mail retains its original sender and
organization allowance through the returning receipt, until the round trip completes.
Old accepted work is retained on upgrade.
A refusal identifies the allowance and creates no letter/channel line; retry once
pending work settles. A full physical queue can still refuse control traffic.

Private pairs start closed. Both endpoints need positive use rights; buying service,
paying for manufacture or receiving a class grant does not grant carriage. Link
knowledge and carriage are separate. Operators can grant either or both rights:

```
+relay/access <pair>/#<character>=discover,use
+relay/access <pair>/org:<organization>=use
+relay/access <pair>/#<character>=none
+relay/network <pair>=public
+relay/network <pair>=private
```

Access changes require `relay.manage` and knowledge of the pair. Opening a private
pair to public carriage also requires `relay.publish`. Private identifiers without
discovery rights produce the same reply as unknown identifiers. `relay.discover`,
`relay.use`, `relay.inspect`, `relay.manage` and `relay.publish` are independent
scoped capabilities. Staff retain their administrative authority. The active owning
organization and its legally titled units have ownership authority; ordinary human
members do not inherit that authority. Existing manufactured pairs need explicit
grants or public carriage when upgrading from the earlier permissive behavior.

```
+relay/represent org:<organization>
+relay/represent personal
+relay/represent
```

An official needs `relay.speak` at the organization's root scope; a grant confined to
a child compartment cannot claim the whole institution. Membership, account debit
authority and the payer do not confer speaking authority. The actor, represented
organization and authorizing scope are recorded separately. Selection applies to
new orders, reports, mail and IC channel lines, and may qualify them for that
organization's subscription. It never changes the sender or who pays. Queued work
keeps its original representation after selection changes or restart. Authority and
private use are rechecked before the next relay hop; revocation selects conventional
fallback where possible. A physical leg already in flight keeps its frozen timing.
If a selected identity loses authority, new sends refuse until you select personal
or regain authority. Recipients are checked as themselves or their titled owner;
a human's selected sending identity does not silently act as a receiving mailbox.

## Subscriptions, treaties and arrears

A v1 subscription covers short outgoing copies for its named IC principal:
Standard is 25 cr per IC day, Priority 100 cr, and Sovereign 500 cr per IC day
under a current Exchange treaty. Capacity, declared Standard delay, both endpoints'
restrictions and the 1024-byte short-message limit still apply. Your account pays;
it does not change who sent a message. Personal memberships do not make personal
mail an organization's traffic. An organization's titled unit speaks for that
organization while that institution is active; an ordinary character speaks personally
unless explicitly authorized and selected as described below.

```
+relay/subscribe #<your character>=priority,<payer account>
+relay/subscribe org:<organization>=standard,<payer account>
+relay/enroll <subscription quote>
+relay/subscriptions
+relay/cure <subscription>
+relay/unsubscribe <subscription>
```

A subscription quote lasts 600 IC seconds. Enrollment creates a real recurring
obligation, subject to the payer's actual debit authority. The first collection is
due at enrollment and runs on the economy's existing cadence; service begins only
when an installment is actually paid. Later installments fall due one full IC day
apart. Collection is bounded; the economy's existing downtime-forgiveness policy
still applies. Each started period has the quoted flat price, with no prorated
refund on cancellation or treaty expiry. The obligation and its real transactions
remain inspectable through the economy commands.

One unpaid installment throttles service; two suspend it. A later successful bill
does not repay older missed installments. `+relay/cure <id>` states the exact debt
and prints a confirmation command with its counters. That confirmation pays the
actual debt, refuses stale terms, and survives retries/restart without charging
again. Cancellation stops future bills and grants no further class, but existing
arrears remain payable. Cure cannot undo cancellation, expire an independent
account review, clear `operator.*` sanctions, or remove permanent anathema.

An organization enrolls through a representative with `economy.account.post` on
its root compartment; payer account authority is checked separately. Sovereign
recognition is pinned to the existing government institutions: Concord Orbital
Access, Republic Strategic Reserve, Freeport Registry and Titan Concession Company.
A faction affiliation or a similar organization name is not eligibility.

An Exchange operator with `relay.manage` may offer
`+relay/treaty org:<id>=<IC days>,<reason>`. Offers last 2–365 IC days and are immutable until expiry. The
recognized institution's authorized representative accepts through a quoted
Sovereign subscription. At least one full billing period must remain at enrollment.
`+relay/treaty org:<id>` lets the representative or Exchange operator read the offer.
Expiry removes Sovereign eligibility immediately and the economy refuses any
collection at or after the agreed endpoint. A new term requires a new agreement;
separate operator sanctions remain available during a treaty.

The service retains at most 128 subscription/quote rows, eight current quotes per
issuer and 32 treaty counterparties. Expired quotes and cancelled debt-free records
may be retired for room. Active obligations and outstanding debt are never evicted.
Notices are queued through the existing economic-notice inbox, not instant mail.
Routing reads current billing status directly; a delayed notice cannot grant service.

Scoped operators may still give explicitly noncommercial courtesy service with
`+relay/grant luna_ceres/#character=priority` (or `sovereign`, `standard`, `deferred`),
under `relay.manage`. On Exchange links, Deferred removes the override; independent operators default
to Standard and remove their override by granting Standard. These grants are audited and
never represented as purchases. Purchased terms override courtesy class for that
copy. Service restrictions at both endpoints are rechecked before transmission;
changing the payer cannot evade a receiver's denial.

`+relay/status luna_ceres` shows your service restrictions and reasons. An operator
with `relay.manage` can set, for example:

```
+relay/policy luna_ceres/#123=throttled,operator.incident,Investigation pending
+relay/policy luna_ceres/org:22=suspended,operator.sanction,Contract violation
+relay/policy luna_ceres/#123=served,operator.incident,Investigation cleared
```

A policy belongs to the pair's operator and applies to every link that operator
runs. The strongest sender/receiver restriction wins: Served keeps the purchased or courtesy
class; Throttled demotes once (Sovereign → Priority → Standard → Deferred);
Suspended uses conventional transport or waits for a known route. Independent
operators' links remain eligible. Both characters and a titled unit's legal owner
are checked. Ordinary membership, database ownership and payment do not authorize
representing an organization or bypass a receiving endpoint's restriction.

Each source is independent; Served clears only the named source. Manual sources
must begin `operator.`. Changes require a reason, are durably audited, and affect
queued traffic before its next relay hop. Already consumed service and frozen
conventional legs are preserved. Up to 256 active source/principal/operator
restrictions are retained; clearing a source frees its slot. Only your own reasons
are exposed in player status; actual load remains operator-only.

`+relay/load luna_ceres`, `/audit` and `/discover` need `relay.inspect`.
Discovery files timestamp evidence in the observer's private `+intel` ledger.
`+relay/publish luna_ceres` separately needs `relay.publish` and retained evidence;
publication writes canon and opens the congestion-board scandal front once.
Ordinary players cannot query actual queue load. Wizard staff can administer these
commands. The service ledger retains 512 receipts, 128 audit events, 256 class
grants and 64 evidence records; receipt and audit history roll over at their caps.

`@relay` and `@relay/check` inspect the bounded registry and validate its object
bindings. `@relay/show luna_ceres` shows both half IDs, installed objects, host
places, coverage and availability. These commands require SIM_ADMIN or Royalty.

Availability controls take a half ID:

```
@relay/disable luna_ceres.luna
@relay/enable luna_ceres.luna
@relay/power luna_ceres.ceres=off
@relay/power luna_ceres.ceres=on
@relay/staff luna_ceres.ceres=off
@relay/staff luna_ceres.ceres=on
```

A pair is Live when both halves are enabled, powered and staffed. Otherwise it
is Degraded; a destroyed half makes it permanently Dead and orphans its partner.
Repairing the station does not recreate the pair. Power and staffing are explicit
operator settings, not a crew simulation.
Repeated settings make no change. Successful mutations persist before confirmation.

The seed pack is `tacsim.relay.luna_ceres`. Its current version supplies the five-node
Luna star, with four pairs and eight halves. Malapert occupies the Exchange compound;
coverage is the explicit room list shown by `/show`. Existing worlds take upgrades
through the content plan/apply commands, preserving pair identities and settings.
Each public pair shares 8 units/IC-second in both directions, with one unit per started
128 payload bytes. Its class queue admits at most 64 pending messages; total pending
transport is capped at 128. Overload is refused before acceptance. A message larger
than a link's one-second capacity uses conventional delivery.

`@prop/route <envelope id>` shows the staff-only itinerary, frozen due time, FIFO
ticket and route-work counts. Players receive an order only after arrival, and its
issuer learns delivery only when the receiver's `+orders/ack <id>` returns.
Link repairs and sky changes cannot accelerate an already transmitted leg.

Installed hardware cannot be carried, teleported, destroyed or have its referenced
behavior removed through ordinary commands. If the registry is corrupt, missing
beside surviving bindings, or contradicts its objects/operator, startup stops with
a diagnostic naming `relay:registry`. Restore the matching document and objects
from a consistent backup; do not delete the record to force a second seed.

## Structural loss and anathema

Installed relay nodes are checked station units. Damage to power structure disables
service independently of the operator power switch; structural repair restores this
availability. Core destruction permanently kills installed halves and leaves their
distant partners orphaned. Repairing a station does not recreate its entanglement.
Queued work takes conventional fallback; an already-flying leg keeps its deadline.

An attributed destruction produces a public canon incident and faction condemnation.
The Exchange permanently denies the perpetrator service, including reception, and
Tycho voids their recovery cover and insured-loss cooldown waiver. `+relay/status
<pair>` shows your sanction; `@recovery/status` shows its insurance effect where the
recovery provider is enabled. `+relay/policy ...=served,operator...` cannot clear it.
Unattributed custody damage is recorded without inventing an attacker.

Staff may opt a controlled faction into notoriety using `@faction/resource
<faction>=attention/0/<reason>`. Attention is a rating from 0 to 1000; no faction
receives it unless the pool exists. A new attributed loss grants one attention to
affiliated factions with that pool, reduces an existing legitimacy rating by one,
and advances clocks watching `relay.node_destroyed`. Replaying the loss grants
nothing. No attention-seeking faction is seeded.


## Building a private pair

`+relaypair` locates the lunar entanglement foundry and its input/output vault.
Running it requires the foundry organization's authority and `vault.withdraw`
on that vault's scope. `/stock` issues the world's eight original relay blanks
once. Instrument boards must be bought from real stock or shipped in.

Use ordinary industry commands. Substitute the numbers shown by `+relaypair`
and your own funded credit account:

```
@wo/draft tacsim.relay.manufacture/private_pair/1=<facility> 1 <account> <vault> <vault>
@wo/submit <work-order>
@wo/show <work-order>
@wo/deliver <work-order>
@inv/lots <vault> relay_pair
+relaypair/claim <work-order>=<output-lot>
+relaypair/collect <work-order>
```

Each pair consumes one relay blank, two instrument boards and 120 hourly IC
production occurrences. Delivery and the industry's budget settlement must finish
before claiming the output. Claim consumes that exact industrial lot once;
collection gives its owner two uniquely titled machines. Repeating collection or
restarting never replaces missing, stolen or destroyed machinery.

Each half weighs five tonnes and occupies one checked vehicle-bay slot. A Dray-P
can carry two. Stand at the dock gallery with the actual carrier and cargo, then
use `+relaypair/load #half`. Sail through the ordinary ship workflow and use
`+relaypair/unload #half` at the destination gallery. Cargo cannot be loaded or
unloaded during passage or while clearance is filed. Ordinary carry and teleport
commands cannot bypass this workflow; `+tv` includes cargo occupying the bay.

At each destination, supply an owned mounting object in the same room as the
unloaded half. `+relaypair/install #half=#mount org:<operator-id>` requires cargo
title and authority to represent an independent operator. The two stations must
occupy distinct places. The first installation survives restart but provides no
link; the second activates the pair at one unit per IC second, compared with the
Exchange's eight. Each station covers its own room and uses its operator's policy.
Its power and structural loss follow the ordinary installed-station rules.

`+relaypair/discover #half` requires physical inspection of a completed private
installation. It files intelligence and advances the Concord's anti-proliferation
front once per pair. Repeated inspections and restarts do not advance it again.


## Concord Fleet mobile programme

`+relayfleet` shows **A Half Afloat**, the dormant eight-segment relief programme.
Its Halcyon capital hull begins without a working relay. Fleet command is a real
`@asset/assign` relationship; treasury and hold access use normal organization grants.

- `+relayfleet/fund` authorizes six actual 10,000-credit payments from the finite
  Fleet treasury to the Lunar refit yard. The first is due immediately; subsequent
  payments are at least 30 IC days apart. Insufficient funds pause the programme.
- `+relayfleet/consent yes` records Postmaster Solveig Adebayo-Krishnan's decision
  through an authorized Exchange representative.
- `+relayfleet/refit wo#<16-hex>` installs both titled halves of a completed
  foundry order while the assigned commander and idle cruiser are at Luna. The
  physical pair and real refit provide the eighth segment, in any producer order.
- With all eight segments complete, the half serves the cruiser and traffic within
  300,000 km along its checked journey. `+ship/plan`, `/board`, `/undock`, and `/leave`
  use the ordinary paid supplies and real passage. First-launch canon follows the
  first actual departure, once.
- `+relayfleet/remove` at the Lunar yard disconnects the pair and returns both
  manufactured halves to the dock. Refit reuses the same halves and cradles, without
  repeating the front or first launch. Lost hardware cannot be remounted to revive it.

Carrier power damage interrupts service. Destroying the carrier kills its mobile
half and orphans its mate, with the same consequences as destroying a fixed node.

## The Exchange and its original fronts

Postmaster Solveig Adebayo-Krishnan works at the Malapert compound. The public
counter displays the Exchange charter; the relay hall is staff restricted.
`+clocks` shows their **Named service case** and **Fleet relief relay programme**
clocks; staff can inspect **Terms of Service** and **A Half Afloat** with `@front`. Fresh worlds start
both clocks at zero, without a commissioned private or mobile pair.

Terms of Service records the first named principal's real arrears or authorized
incident case: two segments for a throttle and four for a confirmed suspension.
A direct suspension can fill the clock immediately. The same principal must own
both stages; unrelated debts never accumulate into their case. Repeated notices
and restarts do not advance it. Paying arrears restores service according to the
subscription rules; it does not erase the historical case or another sanction.

Staff upgrading an older world first apply its pending station and lane packs
through `@atlas/pack plan` / `@atlas/pack apply`. Restart to seed the missing
Exchange geography, then explicitly apply `tacsim.relay.luna_ceres` version 2 and
restart. Existing halves, purchases, balances and damage survive; a conflicting
partial expansion refuses and requires repair. Startup logs report the full star
ready; `@relay` shows the topology, and `+relay` shows the board and tariffs.
