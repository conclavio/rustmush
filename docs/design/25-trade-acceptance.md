# Trade acceptance and public traffic boards

Status: **accepted on 2026-10-25**, including the genuine rate-24 passage and second visit.
Started 2026-10-24 Pacific. Baseline: `8d4abdac` on `origin/main`.

## Scope and gates

Close doc 18 section 6's real-server, real-Redis two-session play test and the
public berth/departure-board follow-up in doc 18a C01/C14/P01/P05. Preserve the
rate-24 experience throughout. An accelerated production-composition regression
is additional evidence, never a substitute for the overnight passage.

1. Implement public traffic projections and repair defects found by live play.
2. Run production-composition and focused notice regressions; deploy on dev1.
3. Start a fresh real-Redis world, at world rate 24 and constant sky rate 0.
   Enrol a new pilot, load both hold limits, bunker to the live planner, file the
   outbound sales and home-leg purchases, disconnect and let the queue execute.
4. After actual docking and commerce settlement, reconnect once, verify the
   watch log and balances, walk the Yard, refile the return, and log off.
5. Reconcile conserved money and goods, lineage, capacities, no escalations,
   and restart persistence; update evidence, commit and push `origin/main`, pause.

No manual content apply, world clock change, ship teleport or synthetic arrival
is allowed in the live run. The home departure is initially filed at +72h so
session two can inspect the docked ship; this also gives `fill` a concrete return
leg to reserve fuel and stores for. Session two refiles it at +1h.

## Public boards

`look` in station berth galleries lists current local hulls, registry marks,
berth, title holder, agent of record and clearance status. The authored
`dockmaster` and `harbourmaster` rooms show local hulls and their published
head departure/clearance, plus the origin port's underway departures and ETA.
The existing room roles cover Ceres, Stickney and the later moon stations;
mining pads get no station board. The current room's atlas record is an indexed
lookup; this adds no scan of world rooms, content version bump or persistent
berth authority.

The berth field names the actual gallery. There is no authoritative numbered
cradle allocation in the current spatial model, so the board does not invent
cradle numbers from the hull's original pool index or a transient sorted list.
Registry marks distinguish hulls. A future cradle-reservation feature would own
those numbers and can supply the field.

Only the runnable head departure is advertised. A later return behind cargo
orders is not shown as a current clearance. Paused, held or obsolete queues
advertise no departure; IDLE means no current public clearance, not an empty
private plan. A filed future departure has no claimed arrival until the planner
has frozen a real clearance. Actual departure timestamps use their original
world-clock generation; arrival estimates use the current rate. Boards show no
cargo, prices, crew list, private hold reason or order log. Hidden title-holder
organizations and hidden objects remain redacted as `private registry`.

## Defect found by the first live attempt

The initial ordinary browser session completed enrolment and outfit, filled the
100 t / 120 m3 cargo hold with 20 t concentrate and 80 t ice, and bought 52 t
reaction mass and 19 crew-days after iterating the actual planner. The queued
ship waited for accepted goods to settle, then departed with one burn ticket.
Its actual flight is 1,598,326 IC seconds, about 18h30 at rate 24.

Closing the browser enters a five-minute linkdead grace. `World::is_connected`
stays true in that period, while `has_live_conns` is false. Trade notification
code used only the first flag, so the departure notice was sent without a live
recipient instead of being queued for the watch log. WHO and the persisted
journey/order records confirmed the pilot was absent and the launch completed;
Redis had no launch notice. This attempt is diagnostic evidence, **not the final
accepted two-session run**.

Repair: immediate notices require both session-connected and a live transport.
Trip-level notices queue during grace; the existing surface live-only notice
budget remains live-only. Arrival/waypoint notices use the same deliverability
rule. `ReconnectNotices` drains on revival as well as a full connection, deleting
the queue once. The regression exercises a lost transport, departure during
grace, revival and a second revival without duplicate delivery.

## Live fixture and reproducibility

Dev1 runs an isolated persistent Redis on loopback 6798 with AOF and a separate
world data directory under `/opt/rustmush/dev1/trade-acceptance`. The standard
Redis6788 and dev1 data remain preserved. The gateway's dedicated
`70-trade-acceptance.conf` systemd drop-in selects the acceptance config; dev2
is untouched. The active database is read from that config, never guessed.
Private login credentials stay on the VPS outside version control.

- `scripts/trade-live.mjs`: real TLS HTTP authentication and WebSocket commands;
  input JSON selects `staff` or `pilot` and an explicit command array. `mode`
  `day1` starts the single new-pilot session; `day2` performs the second visit.
  All public replies are saved, with no password or authentication token.
- `scripts/trade-audit.py <label>`: one atomic Redis JSON.MGET snapshot after a
  stable relevant-key census. It checks unchanged rate 24, the closed credit
  ledger, every seeded trade good against current pools plus burned quantities,
  seeded lineage for goods aboard, authored compartment limits, and no economy
  escalations. It never logs in the pilot or writes Redis.

The JSON audit is a phase snapshot; the existing story tests additionally check
capacity at every write and replay boundaries inside settlement. Do not claim
that phase snapshots alone prove every intermediate live write.

## Evidence and remaining acceptance

The board implementation passed all 11 tests in `mush-server`'s `ic_slice_loop`
target, including unchanged flight, queue, refusal and relay/fleet cases. The
additional linkdead regression passed separately (51.58 s). The final focused
suites passed **101 tests**: 34 orders, 26 plans, 16 port-tick and 25 voyage tests.
Strict TacSim Clippy and targeted Rust formatting passed.

The repaired release is deployed on dev1. Seven changed source files matched the
local worktree by normalized SHA-256 at build time. Final formatting only moved
the `mod traffic` declaration into alphabetical order; the exact post-build
formatting diff is retained in the evidence directory. Server binary:
`f0987369700226d00ac5e841b367d0a8dd1cb7e15cb9dc020e9022716682a91c`.
The fresh accepted attempt uses database **6798/1** and `data-v2`; the first
failed attempt remains frozen in database 0 and its original data directory.
Its evidence and credentials are preserved under `evidence/attempt-1` on dev1.

The new pilot completed session one and disconnected at **2026-10-25 02:42:43
UTC**. The Salt Moth departed at **02:43:13 UTC**, inside linkdead grace. Redis
now holds both the wait-for-goods notice and the clearance notice, without any
pilot reconnect. The hold contains 20 t concentrate and 80 t ice, filling both
100 t and 120 m3 limits. Exactly one ticket consumed 52 t reaction mass and
19 crew-days. The audit reconciles all eight trade goods, all 26 credit accounts,
and the original one-shot capital issue; there are no economy escalations.

A real gateway SIGHUP replaced the server child mid-passage while preserving the
gateway. Before/after Redis snapshots agree on the route and departure, ticket,
all credit accounts and inventory pools, offline watch log and rate-24 timeline.
Fresh authoritative health passed. The first readiness probe incorrectly treated
health's gateway uptime as child uptime; verification instead uses the changed
child PID and fresh world timestamp. Dev2's gateway remains PID 539467.

The public office board correctly showed the underway ship, destination and ETA;
the origin berth gallery no longer listed it. Actual docking occurred at
**2026-10-25 21:13:10 UTC / 14:13:10 Pacific**. Queued sales and purchases completed
by 21:18:23 UTC without the pilot online.

### Second visit: accepted

The pilot returned at **22:01:42 UTC / 15:01 Pacific**. All eleven queued lines
arrived exactly once: the goods wait, clearance, waypoint, docking, both sales,
four purchases and the original return schedule. A harness assertion expected
`docked at Stickney Yards`, while the actual status is `docked Stickney Yards`.
It stopped after that read-only status command. The recovery at 22:02:20 verified
no repeated watch heading or lines, then completed the visit; no mutation was
replayed. This is two acceptance visits, with one harness reconnect during the
second visit, not a claim of exactly two physical connections.

The pilot walked the berth gallery, harbourmaster and Service Counter, read both
public boards, replaced order 8 with a return departure at +1h, boarded again,
read the real home-leg planner and logged off. Final state:

- Two sales: 9,600 credits trading margin; 2,400 credits Co-op share; 340 credits
  available and none held. Charter remains in force.
- Cargo: 49 t crated yard fittings and 17 t alloy, 66/100 t and 117.8/120 m3.
  The 55 t alloy request correctly bought only 17 t because cash was limiting.
- Supplies: 50 t fuel and 19 crew-days. The home planner requires 49 t and 19
  crew-days for this load; the filed departure is 23:02:21 UTC, order 9.
- All eight trade goods and the closed credit ledger reconcile, with unchanged
  one-shot capitalization, seeded lineage, one outbound burn ticket and no
  refused/disputed settlement or economy escalation.
- The old watch log drained. A single **new** return-schedule notice was queued
  after the second logoff; the queue is not incorrectly required to stay empty
  after new events occur. No old line was replayed.

The real test used the previously built release throughout, preserving its
clock and lane. Main advanced meanwhile through SM.12b (`f171b36c`); the trade
changes were reapplied without conflict, and all twelve production-composition stories passed again (462.25 s) against
that integrated source. Strict TacSim Clippy and formatting passed. The 67-file
integration manifest matches the sources tested on dev1. No second
rate-24 passage or claim that the overnight run included SM.12b is implied.
The original standard dev1 world and first diagnostic attempt remain preserved.

The unrelated `sim_rematch` CI repair (`e5599e5e`) passed its GitHub target.
Follow-up failures in arena-story telemetry and panic/relaunch are tracked in
doc 24, separately from the live trade result.

Evidence directory: [trade-acceptance-2026-10-24](evidence/trade-acceptance-2026-10-24/).
Private credentials and full Redis snapshots remain on the VPS.
