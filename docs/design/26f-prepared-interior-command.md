# DT.2b2b.2 — Prepared interior authoring

The existing staff `@spatial/space` command now submits the prepared physical
action described in [26d](26d-prepared-physical-transitions.md) and records the
crossings described in [26e](26e-personnel-crossing-capture.md). Defining or
reanchoring an interior no longer publishes its canonical record synchronously.

## Command and acknowledgement

The command retains its staff permission, live-room, `sim.interior` and
member-room checks. The command and planner share the member-room rule, so a
room belonging to another interior still receives the specific refusal. The
actual physical provider and participant revalidate the complete bounded request
and its control, custody, recovery and journey exclusions.

Submission queues only the request. The normal coordinator prepares, rechecks
and durably commits the Sim topology, Core projections and personnel capture
together. There is no synchronous fallback when preparation is unavailable.
Success is the acknowledged receipt: `Spatial update for #<root> committed.`
Its pure projection reads only the retained outcome; it does not look up current
object names or infer whether a historical request created or reanchored a room.
The action, participant and outcome v2 definitions remain byte-identical.

`@spatial` advertises the Authoritative compatibility ceiling because its
`/space` switch submits this action. Other switches retain their existing
implementations. In particular, `/place`, `/unplace`, `/unspace`, mounts,
control authoring, journeys and compound hull construction are not silently
converted into deferred operations by this slice.

## Personnel boundary

An occupied interior reanchor records actual entries and exits even when it
changes no object location. Newly created spaces also capture the actual
unplaced Player/NPC occupants of their room. The capture remains explicitly
`uncertified_legacy_paths`; this command migration alone does not establish
continuous aboard intervals, employment, wages, departure manifests or NPC
eligibility. Uncovered movement and topology paths remain prerequisites.

## Verification approach

Production coordinator tests use the actual `@spatial/space` command in place of
the reanchor test adapter. They cover acknowledgement, exact retry, restart,
stale World and Sim state, clock changes, permissions and same-commit captures.
A new occupied-room case checks that neither the interior nor its success is
published before acknowledgement and that lost-ack restart returns the same
private receipt.

Existing Sim and propagation fixtures opt into an explicit Store-backed test
driver. It acknowledges prior legacy writes before preparing the real composed
action, waits for the exact Store receipt before installation, and hydrates
typed rows from that same Store before module boot. Default `TestWorld::run`
behavior is unchanged. Reserved typed records never pass through legacy
`ModuleOp` writes.

The targeted regression run passes 78 Sim library tests, 155 tests across the
six migrated Sim suites, two command/Store tests, 88 propagation tests, 17 DT
production coordinator tests, 32 production composition tests and 15 hull
tests. The feature inventory and source ownership, panic, typed-state,
restriction, legacy-handler and help contracts also pass.
