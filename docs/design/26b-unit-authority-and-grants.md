# 26b. Shared unit authority and grants

DT.2a implements the authority boundary accepted in doc 27 Q-A1–Q-A4 and Q-F2.
It follows [26a](26a-detachment-implementation-plan.md) and the typed-family and
coordinator rules in [09b](09b-k8-compatibility-rules.md). This brief records the
revocation and emergency policy before the custody adapters use it.

## Ownership and persistence

Economy remains authoritative for title, lifecycle and economic scope. Org owns
operational grants in a new strictly decoded typed family. Existing Org records
continue to own membership, roles, direct capability grants and policy denies;
the new grant registry is never copied into `OrgStore` as a second authority.
Issue and revoke use the common coordinator. A command stages bounded input and
submits an action; only an acknowledged result publishes success.

Personal owners can lend directly to a character or an organization. A grant to
an organization names its receiving compartment, independently of the lender's
title scope. Carrier ownership, berth presence, `AssetAssignment` and a generic
lease marker establish no operational rights. Future contractual rights must
arrive through their actual owner's explicitly scoped evidence.

Separate capabilities cover operate/crew, deploy, command assigned AI units,
recover/load, maintain/refit and transfer/abandon title. Onward delegation is
separate from exercising each capability. Carrier bay services and departure
authority remain separate. No grant creates a budget, insurance coverage or a
personal liability guarantee.

## Exact roots and bounded delegation

Every grant carries one capability and an exact title, organization grant, role
template or parent unit-grant source. A generic organization Allow decision is
not proof of delegation: it may come from policy or a nondelegable source. Root
validation checks the exact source and current role membership, scope, term, limits
and delegable status; applicable explicit policy denies still win. Direct Org
grants retain Org's existing ability to authorize a named nonmember; role-template
roots require active membership and the current role revision.

Personal title roots pin the journal sequence and effect index of the last
`Created` or `TitleTransferred` event for that unit. Routine effects leave this
identity unchanged. Free transfers, title leaving and returning, and reissues
replace it. Economy checkpoint format 4 carries this derived index. An older
checkpoint triggers the existing complete-journal fetch/replay fallback; a
partial loaded tail must never manufacture lineage from an asset revision.

Children may narrow capability, unit scope, validity, exclusions and onward
authority. Every admission re-resolves the live lineage; a matching revision
alone cannot establish unexpired authority. Revocation and expiry invalidate
dependent grants without rewriting every descendant. The initial engineering
bounds are eight grant rows in a proof, 128 visited proof nodes, 256 explicit
units/exceptions per scope and 1,024 retained grants in the durable registry.
Encoded size is bounded independently. Capacity refuses before mutation; revoked
identities are retained and never reused.

An organization wildcard follows currently controlled eligible units. Current
title and genuine incoming operational grants can establish control; physical
custody cannot. A child cannot remove an ancestor's exclusion. Exclusions on a
recipient's overlapping wildcard grants take precedence over wildcard allows;
an independently explicit unit grant remains subject to its lineage and policy
denies. Newly acquired units enter a wildcard only while current rights cover
the requested capability and delegation.

## Accepted mission roles

A current-leader grant is nondelegable and command-only. It binds the mission,
detachment, accepted roster revision and digest, and a finite set of accepted
units. The actual mission owner supplies the current eligible player or human
NPC and command term. Succession changes the role holder without minting rights;
a changed accepted roster requires fresh grants. A software mind is not a human
principal.

DT.3f connects the accepted IC owner through Org's existing authenticated read
boundary. Tacsim supplies a freshly derived eligibility selection from the
actual accepted roster, live reservations, field flights, station/K6 control,
employment, unit condition and sessions. Org accepts this request only from the
registered Tacsim requester, then resolves its own actual grant registry and
current owner/loan/policy facts. The final common roster uses that answer; a
role declaration cannot elect its own holder or displace a higher-priority
eligible player. No mission or grant authority is mirrored into World.

A prospective declaration can be issued before a leader takes the field. It
proves the lender's full finite command/delegation lineage at creation, but
supplies no active mission, eligible person or current right. Ordinary grants
retain their existing validation. At exercise, role resolution still requires
the exact accepted binding, finite accepted units, active mission and current
human leader/term. `+sortie/command-grants` prints per-unit declarations from the
actual accepted activation for its authorized roster readers; the unit owner or
authorized lender issues them through the existing Org coordinator action.

Final admission retains both halves: Tacsim's source guard and actual owner
preimage reads fence the accepted activation and current eligibility; an Org
mission-lineage coordinate re-resolves the selected actual grant, its ancestors,
current title lineage, expiry, exclusions and policy for the actor/unit. That
coordinate alone never establishes leadership. Revoked or transferred backing
refuses even if the actor has acquired an independent permission through another
lineage. The ephemeral OOC practice host supplies none of these IC facts.

## Prospective revocation and retained custody

Revocation denies new reservations, deployments, station acquisitions, grants and
mission orders. It does not eject an occupant, clear an already delivered order,
erase an executed act, transfer title or remotely disable a unit. Physical hosts
continue to own their existing control and action records.

Every grant records either no retained-return consent or `ReturnOnly` consent.
A personal title root may authorize the latter. A child may preserve or narrow
its parent's policy. An organization root can grant it through an exact,
delegable recover/load source; an operate or command grant alone cannot add it.
An organization may provide a separate recover/load grant for preparation.

The policy alone is not an executable recovery token. The actual custody host
must record the admitted unit, human operator, open hull epoch, return origin
and recovery berth together with the applicable policy. Only current matching
custody can use this retained authority for safe stop, disengagement, direct
return to that origin or recovery into that berth. A different unit, operator,
epoch or destination fails closed. It cannot authorize a new expedition,
station acquisition, refit, title operation, or a new offensive order.

Self-defense is not inferred from a client flag or the word “emergency.” This
slice grants no firing exception after revocation. H1 must supply an explicit
rule using host-observed attack and purpose evidence before such an exception
can be admitted. Stopping, disengaging and returning remain the defined safe
actions. DT.3 owns the actual durable custody/epoch producer; DT.2a supplies no
synthetic live custody claim.

## Verification boundary

The production command is `@unitgrant`: `/issue` handles a personal loan or an
explicit parent grant; `/json` accepts an explicit organization source and scope;
`/revoke` is limited to the original grantor. `/check` reports only the caller's
current allowance, and `/list` pages only that caller's issued/directly received
records. Errors disclose no other character's authority or hidden asset facts.

For example, an organization wildcard draft names all fields explicitly. Replace
these illustrative IDs with the actual organization, compartment, recipient and
delegable Org grant. If the source expires, supply a no-later `valid_until`.

```json
{
  "grantee": { "Character": 42 },
  "capability": "deploy",
  "scope": {
    "OrganizationControlled": {
      "organization": 1,
      "scope": { "Exact": 2 },
      "excluded": []
    }
  },
  "valid_until": null,
  "delegable": false,
  "recovery": "none",
  "parent": { "OrganizationGrant": { "organization": 1, "grant": 3 } }
}
```

Acceptance covers personal lending, exact organization roots, incoming loans and
future-unit wildcards, exclusions, bounded multi-hop delegation, parent expiry
and revocation, free title transfers, mission binding/term checks, strict decode,
coordinator acknowledgement, retry and restart. Missing external facts refuse.
Preparation, personnel and physical departures remain DT.2b and DT.3.

The DT.2a verification run passed 263 Org/Economy library tests, 108 Economy
integration tests, 51 existing Org integration tests, and three production
coordinator tests covering acknowledgement loss, restart, stale inputs, changed
title and expiry at final admission. The production command compatibility test,
help/handler contracts, state ownership and typed-reader checks also passed.
Clippy passed for Org and Economy across all targets, and for the affected Tacsim
and server targets; workspace formatting and whitespace checks passed.

## Conditional equipment lending at Occator

The actual lessor's existing NPC signatory has four explicit delegable Org
capability roots in the lessor's title scope. A normal lessee can separately
request the equipment agreement at the counter with `+tv/<n>/permissions go`.
The signatory submits one bounded four-grant Org issue action, whose sole
registry post-image records all four loans or none. OperateCrew and
CommandAssignedAi permit onward delegation; Deploy and RecoverLoad do not.
The offer costs nothing extra and conveys neither title nor MaintainRefit.

Each root names its real organization grant and the exact economic contract,
tenant, unit and accepted consent hash. Economy checks the current journal-backed
organization title, active accepted contract and its real lease encumbrance.
That is a condition on an explicit operational loan, never permission inferred
from a generic rental. Resolver children and mission-role proofs retain the same
condition. Final action validation rechecks the whole agreement, including a
later capability losing its lending root. New loans never revive an existing
revoked agreement. Existing merged v2 movement providers remain unused.

The public owner reader may inspect a proposal for a named lender. This is an
advisory subject selection, not a change to its immutable authenticated caller or
authority. The actual NPC's ordinary authoritative submission and Org's private
Prepare path re-resolve its source rights before any grant commits. The source
provider still owns the single durable registry write; no new family or movement
observer is introduced.
