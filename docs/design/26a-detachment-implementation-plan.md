# 26a. Detachment implementation

Started 2026-10-28 against `origin/main` at `62656f43`. Policy comes from
[doc 26](26-detachments.md) and [doc 27 Draft 0.4](27-ic-field-operations-design-and-survey.md),
including the accepted Q-R1–Q-R5 and P-1–P-5. This brief divides the DT stream
into reviewable changes. DT.1a starts the implementation; DT.1 is not yet complete.

## Delivery sequence

| Slice | Deliverable | Completion gate |
|---|---|---|
| DT.1a | Neutral numbered roster, leadership selection, versioned orders, recipient history, and an OOC exercise adapter | Deterministic tests cover succession, reconnect flapping, stale delivery, standing orders, and advisory player responses |
| DT.1b | Production OOC host and client integration | Typed commands, numbered-slot controls and roster display use the same revision; session/control hooks feed availability; player notices and NPC/controller delivery run through actual communications and observation boundaries |
| DT.2a | Shared unit authority and mission-role grants | Personal lending and organization scopes; bounded delegation and exclusions; prospective revocation; no charter-owner shortcut |
| DT.2b | Presets, drafts, personnel and preparation | Multiple private plans, actual embarkation events, consent/readiness for a revision, contracted NPC crew and exclusive unit reservations |
| DT.3 | Launch and operation history | Preflight then individually durable departures, hull-epoch links, partial failure/retry, returns and unresolved casualties; no rollback of physical departures |

DT.4 subset delegation and DT.5 dedicated communications remain later cuts.
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

## Following integration boundaries

DT.1b must replace the OOC host's use of AI-only side/wingman authority for this
new detachment path. `match_::cockpit::command_of` remains the existing practice
command path until that integration is tested. Existing `deliberation::Order`
has a mandatory arena TTL and cannot represent the common standing-order record.
NPC/controller execution must use its own observation and station admission;
delivery of a target hint cannot grant a contact or bypass those checks.
The admitted order records its issuer's assigned unit as the hint's scope.
Reusing an objective after succession therefore cannot retain the old leader's
private contact scope. Player and NPC completion reports are trusted observations
in this layer; a future command endpoint must authenticate the reporter before
recording them. Acknowledging an order never invents an attempt or completion.

DT.2 resolves NPC identity from physical presence plus a recorded embarkation
event, including field rescue/transfer/recovered crew (Q-R2). Older last-port-only
phrasing is superseded. Crew are hired from bounded port pools, paid per IC day,
subject to injury and contracted mission scope (Q-R5); numbers are content tuning.
Assignment never generates a person from a software profile.

DT.3 separates organizer, actual launch principal, field commander and operator.
The first three are not necessarily the same person. Roster creation or disbanding
does not open/close a hull epoch, recall a field unit, repair damage, or resolve a
casualty. Licensed unit minds need their actual production host; neither the OOC
adapter nor an NPC character's decision driver provides that authorization.

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
