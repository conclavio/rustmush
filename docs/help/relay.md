# Relay topology (staff)

R1 installs the first Luna–Ceres relay pair. It does not route messages yet;
mail, channels, orders and reports keep their existing delivery behavior.

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
The 8 units/IC-second capacity is catalog metadata until R2 defines byte accounting.

Installed hardware cannot be carried, teleported, destroyed or have its referenced
behavior removed through ordinary commands. If the registry is corrupt, missing
beside surviving bindings, or contradicts its objects/operator, startup stops with
a diagnostic naming `relay:registry`. Restore the matching document and objects
from a consistent backup; do not delete the record to force a second seed.
