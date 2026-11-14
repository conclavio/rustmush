# 47. The close-out — cut notes

One section a unit of [doc 47](47-close-out.md) §6. A unit's session replaces its own "Not built."
line with what it built, under these headings: **Survey**, **Built**, **Tests**, **Divergences**,
**Not built**. Each session edits only its own section, so two units' notes never touch the same
lines.

## CO.1

Not built.

## CO.2

Not built.

## CO.3

Not built.

## CO.4

Not built.

## CO.5

Not built.

## CO.6

Not built.

## CO.7

Not built.

## CO.8

**Survey.** A hull whose launch runs late passes an interdicted waypoint unmet. Three ticks take part. The port's
tick (`port::voyage::launch_step`) departs her with leg times frozen from the plan's own `depart_ic` (doc 18 lead
ruling L3), so a late launch is back-dated and her first boundary is already due. Sim's journey tick crosses one
boundary a fire (R13). At an interdicted boundary it calls the carrier hook (`TacsimCarrierHooks::boundary`,
`composition/sim_routes.rs`), then `raise_interception`, which raises an interception only into a domain that
is already there at the waypoint; with none it logs "presents into NO domain". The space tick
(`promote_due_interdictions`) opens the bubble only where that interception stands. The waypoint's domain is made
lazily, by the hook alone, and since doc 35 cut SF.R14 (`e6f573c45`) only when the boundary is crossed within
3,600 IC s of its due time. A later crossing made no domain, so nothing was raised. The cursor moves past a
boundary once, so nothing was raised later either: she flew her second leg unmet.

Reproduced on conclav.dev with probes on each tick, on MW.10's story with its `fire_quietly` deleted, at
3,600:1 (one real second is one IC hour). The launch lands 1,464 IC s past the boundary. When the journey tick
fires after the port's tick in that slice, the crossing is 1,464 s late and the bubble opens. When the port's
tick is last in its slice, the crossing comes one slice later, 5,064 s late, and the space tick then reads no
domain for 30 s.

In play at 24:1 the order in a slice does not matter; the crossing being an IC hour (150 real seconds) late
does. For a launch, on the shipped Ceres–Stickney lane (leg 0 is 326,136 IC s), the launch arm must run about
3 h 49 min of real time after `depart_ic`. That happens after an outage or restart between the burn landing
and the launch, or when staff release a held launch (`launch_hold`: pilot ashore, lease ended, ticket unreadable)
that much later. She is then past Kirkwood straight away, and neither the customs picket nor the interdictor
stops her.

Only the first hull ever to reach the waypoint could pass: once any hull has opened its domain, Sim raises into it
at any lateness, and every later hull is stopped however late she crosses. So the gate was never a rule the game
kept (PR #586 ruled the same lazy domain a production fault for a restart). It came in the same commit that
interdicted the shipped Kirkwood leg, and the clock-jumping trade-run and docking stories met no picket only
through it.

**Built.** The hook opens the waypoint's domain however late the boundary is crossed: the 3,600 IC s clause is
removed. The crew gate (a connected player aboard, or a standing link-loss record) is unchanged.

**Tests.** `domain_world::voyage::a_launch_that_runs_late_still_meets_the_picket`: the burn lands and the port's
tick first runs two IC hours past leg 0's frozen end. The picket's bubble opens within three seconds and her
journey waits `Promoted` at cursor 1. On the code before it fails at "the picket's bubble opens on her", in every
slice order (the crossing is more than an IC hour late in each). Stories moved to a lane whose Kirkwood leg is
not interdicted, because they step the clock past Kirkwood to reach Stickney and the picket is not their subject:
`sfr16_client_docking_game_...`, `sfr24_one_docking_kinematics_...` and `sfr17_hole_closeout_ii_...`'s docking rig
(`lane_without_the_kirkwood_interdiction`, `domain_world/voyage.rs`); every story of `domain_economy/orders.rs`
and `domain_economy/sale.rs` (their `lane_variant`; `orders.rs`'s `stocked()` now seeds it, as `sale.rs`'s
did). With the fix and without the move, 3 `domain_world` and 29 `domain_economy` stories fail, at "the
tactical bubble is not secure", "GOD never submits from the port tick" and "the 395's epoch closed at Stickney".

**Divergences.** None from doc 47. `orders.rs` was composed on the shipped game content (`Roster::bare`) and now
boots a seeded lane, as `sale.rs` does; `Roster::bare` is removed.

**Not built.** A first-ever crossing caught up at boot (`journey::advance_due(Source::Boot)`) still raises
nothing, because the boot pass never calls the hook (R9). The thorough fix is to make the domains of interdicted
waypoints before any hull can reach them, at lane apply or at launch. That would also make the crew gate moot, so
a hull that sails with nobody linked and no link-loss record would be held at Kirkwood for good: MW.13's open
owner call. It waits on that call.

## CO.9

Not built.

## CO.10

Not built.

## CO.11

Not built.

## CO.12

Not built.
