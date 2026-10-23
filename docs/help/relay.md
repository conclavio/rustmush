# Relay topology (staff)

The Luna–Ceres pair carries orders, contact reports and returning acknowledgements.
Conventional legs wait for their physical travel time. A disabled pair uses the
conventional route when positions are known; an unknown endpoint waits until it
can be located or the message expires. Mail and channels are not connected yet.

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
128 payload bytes. Its FIFO admits at most 64 pending messages; total pending
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
