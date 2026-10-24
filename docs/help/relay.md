# Relay service and topology

The Luna–Ceres pair carries IC mail, IC channel copies, orders, contact reports and returning acknowledgements.
Conventional legs wait for their physical travel time. A disabled pair uses the
conventional route when positions are known; an unknown endpoint waits until it
can be located or the message expires. All mail travels IC; OOC channels remain immediate.


`+relay` shows the public service board and your class; `relay.board` is the matching
web query. `+relay/receipts` shows timestamps for your retained, arrived traffic.
Leaving a channel removes access to its receipts as well as its history.

The v1 policy has four classes. Sovereign and Priority wait for actual capacity.
Standard additionally waits 60 IC seconds from queue admission. Deferred waits at
least 7,200 IC seconds and transmits in the first 3,600 seconds of each 86,400-second
IC day. Old eligible work gains a scheduling rank every 300 IC seconds; smaller
eligible copies may pass a head that cannot fit the remaining capacity. The pair
shares one eight-unit bucket across directions; it never accumulates idle credit.
Delivery is checked on the existing coarse cadence, so these are earliest times.

Class grants currently come from the operator; no purchase or charge exists yet.
A scoped operator can use `+relay/grant luna_ceres/#character=priority` (or
`sovereign`, `standard`, `deferred`). This needs `relay.manage` on the Exchange's
organization scope. Grants affect future pair tickets; issued tickets survive
restart with their class and eligibility time. The receiver's class is not used
for the sender's message.

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
is Degraded; a destroyed half makes it Dead. R1 has no destruction or replacement
command. Power and staffing are explicit operator settings, not a crew simulation.
Repeated settings make no change. Successful mutations persist before confirmation.

The seed pack is `tacsim.relay.luna_ceres`. Reapplying it preserves IDs and settings.
Its Malapert Exchange endpoints occupy the Aitken service spine on Luna and the
Ceres maintenance room; coverage is the explicit room list shown by `/show`.
The pair shares 8 units/IC-second in both directions, with one unit per started
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
