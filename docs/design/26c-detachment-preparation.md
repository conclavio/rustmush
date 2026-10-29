# DT.2b — Detachment preparation

The accepted rules remain [doc 27](27-ic-field-operations-design-and-survey.md).
This implementation brief separates durable planning from the physical and
economic facts needed to activate a mission.

**Owner correction, 2026-10-29:** current containment in the carrier is the
personnel eligibility rule, checked at preparation and launch. Embarkation
intervals, departure manifests and complete movement coverage are not prerequisites.
Docs 26g–26j are superseded and must not be extended. Their merged code stays in
place for now; the closed provider is inert on the live command path. A later
cleanup cut may delete the v2 provider if nothing adopts it.

| Slice | Deliverable | Admission boundary |
|---|---|---|
| DT.2b1 | Private presets, independent drafts, invitations and revision-bound player declarations | No reservations, employment contracts, mission-role grants or launches |
| DT.2b2a | Prepared structural movement foundation | Core object images and the canonical Sim placement patch commit together; no personnel eligibility yet |
| DT.2b2b.1 | Immutable personnel crossing capture | Same-commit physical evidence is explicitly uncertified while legacy paths remain; no intervals or eligibility |
| DT.2b2b.2 | Prepared staff interior authoring | `@spatial/space` commits its real topology and crossing capture together; no synchronous fallback |
| DT.2b2b.3a–3d | Superseded movement work in docs 26g–26j; do not extend | Retain merged code for now; no live-path adoption or DT dependency |
| DT.2b2b.3e+ | Withdrawn movement-coverage, interval and departure-manifest programme | No further coverage work required for NPC eligibility or contracts |
| DT.2b3 | Hire a real NPC at port under a contract record | Existing character, accepted terms and mission scope; later wages accrue from the contract, not aboard-time evidence |
| DT.2b4 | Preparation and activation of the six slots | Resolve who is currently in the hull; live authority, consent, licences, exclusive reservations and accepted mission-role grants |
| DT.3 | Per-unit launch and operation history | Recheck current containment, then launch the assigned people and units; durable departures, partial failure/retry, hull epochs, custody and returns |

## Current delivery sequence

1. **DT.2b3 — hire:** at port, record the accepted contract referencing a real NPC.
   Hiring does not create a synthetic pilot or teleport the character. Contract
   terms define any future wage accrual; wage settlement can follow later.
2. **Ordinary boarding:** the NPC walks into the ship's interior through existing
   movement and accompanies the ship. No new movement slice or boarding-history
   record is needed.
3. **DT.2b4 — activate:** resolve the plan's six slots against real units and NPCs
   currently contained in the selected carrier. Read current room/interior
   containment through the existing physical model. Contract scope, availability,
   permissions, consent and reservations remain separate checks. Saved plans and
   crossing records do not substitute for current presence.
4. **DT.3 — launch:** recheck containment at launch so an NPC who left after
   preparation cannot launch from a stale roster. Move the assigned people and
   units into their actual field roles; they need not remain aboard to serve in
   the launched detachment. Returning, rescued or transferred NPCs qualify on
   current containment without historical boarding evidence.

Durable hiring, activation and launch changes use the coordinator under K8.
That does not require ordinary walking or every post-move observer to migrate
to the closed movement protocol. Cleanup of unused movement infrastructure is
a separate later cut, not an activation prerequisite.
Preserve the completed movement work for reconsideration once the core surface
and mining flow is playable; it is deferred engineering, not rejected as incorrect.

## DT.2b1 ownership and records

The neutral `mush-tactics::detachment::planning` model defines material plans,
presets, drafts and declarations. Tacsim owns their durable typed families and
submits changes through the common coordinator. World state holds registered
handles and bounded pending command input; it holds no authoritative planning
mirror. A missing acknowledgement cannot install a prepared change. Restart
hydrates only committed typed records and exact retries return their receipt.

Each character owns a bounded private book containing at most eight presets
and eight drafts. Both use monotone local identities, which are never reused.
There are at most six ordered command slots per plan. Independent drafts may
prefer the same unit: planning does not reserve anything. Loading a preset copies
its preferences into a new draft. Editing that draft cannot modify its source
preset or another draft. Saving a draft as a preset is another explicit copy.

Only the owner reads their presets or edits their book. A draft is visible to its
organizer and currently assigned players. A bounded invitation index provides
that lookup without a global scan. Changes to a draft and its invitation indices
commit together; capacity exhaustion refuses the whole change. An invited
player cannot edit the organizer's plan or declare consent for another player.
Declining frees that player's invitation-index entry. They may still read the
assigned draft by its known reference and explicitly accept again; a material
edit clears declarations and can issue a fresh invitation. Both operations
remain subject to index capacity, so rejection cannot leave a partial edit.
An organizer need not occupy slot one, lead the detachment, or own the carrier.

The material plan includes the carrier, ordered slots, preferred unit, human or
automated controller selection, role, selected mind/profile preferences,
explicit NPC commander and optional financial-terms digest. Missing references
remain unresolved preferences. Mind and financial selections in this slice do
not establish a licence, employment agreement, debit authority or actual host.
All durable fields are strictly decoded, with explicit nulls for optional facts,
independent encoded-size limits and bounded collections.

## Consent and readiness

A declaration binds the authenticated player, their actual selected assignment,
draft revision and canonical material-plan hash. A player accepts before marking
ready. They may decline their own assignment. Material edits clear declarations
and advance the draft revision. A no-op edit refuses without changing either.
Later automatic command succession changes the leadership term, not this stable
roster revision, and therefore does not silently invalidate consent.

Ready additionally requires the player personally at the assigned unit's actual
helm console, with the current billet and focus, and both player and unit
physically aboard the selected carrier. Remote focus and port-dispatcher
exceptions do not count as presence. The real Sim control-lease witness guards
the action through final commit; a physical preimage also revalidates placement,
focus and station changes. The record is a declaration for that plan, not a
reusable permission or proof that the crew remains seated after committing it.
Activation and launch must recheck current facts, including after restart.

## Preparation reports

Reports read current physical ancestry and the authenticated viewer's operational
permissions through the registered Org reader. They disclose no hidden object
names or another principal's authority. Other principals' permissions remain
unresolved until their own owner-attributed admission. A physical station
observation is labelled separately from the actual lease check on Ready.

DT.2b1's existing reports identify unresolved crew employment, applicable
mind/licence admission and reservation/activation work. Any legacy report that
requires embarkation history must be corrected during DT.2b4 integration: current
containment establishes physical eligibility. There is no synthetic personnel
record, automatic NPC creation, teleport, fabricated contract or launch operation
in DT.2b1. This requirements correction does not claim those runtime reports have
already changed.

Player commands are documented in [detachment preparation help](../help/detachments.md).
The retained physical transaction foundation is specified
in [26d](26d-prepared-physical-transitions.md).
Uncertified crossing capture and its compatibility boundary are specified in
[26e](26e-personnel-crossing-capture.md).
The first migrated command is specified in [26f](26f-prepared-interior-command.md).
The historical movement work is recorded in [26g](26g-locomotion-admission.md),
[26h](26h-locomotion-dispatch.md), [26i](26i-locomotion-transition.md) and
[26j](26j-movement-completion.md). All four are superseded and must not be extended.

## DT.2b1 verification

Validated locally on 2026-10-28:

- Twelve neutral-model tests cover strict codecs, independent copies, monotone
  identities, bounds, assignment-specific declarations and material revisions.
- Twelve Tacsim tests cover command fences, transactional invitation capacity,
  decline/reaccept behavior, nested physical presence, actual station facts and
  privacy-equivalent missing/inaccessible references.
- Three tests run the production server composition and ordinary coordinator:
  private planning and consent survive restart and lost acknowledgements; stale
  queued input cannot select a replacement request; readiness requires the real
  helm lease and refuses a physical departure before final commit. Live unit
  grants and revocation change the preparation report.
- The existing three unit-authority coordinator tests still pass. Production
  composition admits 35 owner-read services and 38 grants, within its 64/64 caps.
- The twelve motor regression probes reproduce the DT.1b API-6 replay fixtures
  on Windows, including the retained identity-only compatibility control.
- Clippy passes with warnings denied for the changed libraries and integration
  targets. Source-policy scans cover panic budgets and state-slot ownership.
- The complete Tacsim library suite passes: 1,917 tests passed, four ignored.
  Thirty-two server composition tests, two market privacy regressions and 26
  help/registration/typed-state/restriction contract tests also pass.
