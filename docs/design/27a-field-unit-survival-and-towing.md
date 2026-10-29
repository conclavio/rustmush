# Doc 27 (`H1`–`H3`) — IC Field Combat, Component Survival, Universal Towing, Salvage Title & Workshop Restoration

The accepted design rules live in [doc 27](27-ic-field-operations-design-and-survey.md) (`FO-1`–`FO-4`, `FO-10`, `FO-12`–`FO-14`, `Q-H1`–`Q-H12`, `Q-S1`–`Q-S15`, `Q-F3`–`Q-F6`, `Q-R1`, `Q-R3`, `P-2`–`P-5`). This brief records the neutral `mush-tactics` foundation (`mush_tactics::field_ops`) and the `tacsim` ruleset integration (`tacsim::field_ops`) across `H1-A`, `H1-B`, `H1-C`, `H2-A`, `H2-B`, `H2-C`, and `H3`.

| Milestone | Deliverable | Module |
|---|---|---|
| `H1-A` | Section-localized component survival, derived operational states (`operational`, `immobile`, `unarmed`, `disabled`, `wreck`, `irrecoverable`), IC-custody-legal condition mutation plans, hull-epoch close gating (`Q-H9`), and versioned authority handoff (`§4.4`, `FO-10`) | `mush_tactics::field_ops::condition` |
| `H1-B` / `H1-C` | Frontier site engagement promotion/demotion, motion-clock handoff (`rate 24` surface <-> `1:1` combat, `Q-R3`), reinforcement, withdrawal, carrier leave-behind confirmation (`Q-H10`), re-attacking disabled units (`Q-H8`), link-loss safe posture (`Q-H5`, `P-2`), crew survival / guaranteed NPC extraction (`FO-12`, `Q-H12`, `Q-F4`, `P-5`), and observer-bounded post-battle digests (`Q-H6`) | `mush_tactics::field_ops::engagement` |
| `H2-A` | Universal tow coupling (`<= 2×` tug mass cap), tonnage-differential speed scaling (`Standard` & `RecoveryOptimized` curves per `FO-13`, `Q-S6`), field inspection, exclusive physical attachment (`Q-S9`), coupled movement, carrier bay capacity checks (`Q-S8`), and unhealed bay recovery (`FO-1`, `FO-3`) | `mush_tactics::field_ops::recovery` |
| `H2-B` / `H2-C` | Ground tenure pinned at attach (`Q-S3`), lawful/unlawful recovery entitlement (`FO-4`, `Q-S1`), private authoritative recovery records, evidence-gated unlawful salvage actions (`Q-S4`), post-close black-box access rules (`P-3`), public salvage claims with a 48-available-real-hour contest window (`Q-S2`, `Q-S13`–`Q-S15`, `Q-F3`), `Q-R1` bare-prior-title rejection, and pending-title custody gates (`Q-S14`, `Q-F5`) | `mush_tactics::field_ops::salvage` |
| `H3` | Component stripping with parent-hull provenance & double-strip/double-claim prevention (`Q-S11`, `Q-F5`, `Q-F6`), hull scrapping, in-place section/component repair (including pending-custody repair-at-risk per `Q-S14`), section reconstruction, destroyed/stripped component replacement (`FO-2`, `Q-S12`), component rearming, and facility restoration cost quoting (`WorkshopFacilityTariff`, `WorkshopCostQuote`) | `mush_tactics::field_ops::workshop` |
| `Sortie & Arena Loop` | Carrier personnel manifests with embarkation origin tracking (`FO-16`, `Q-R2`), port NPC crew pools & daily IC wage settlement (`Q-R5`, `Q-H11`), explicit unit expense payer policies (`Q-A4`), organizer-launched & no-player NPC-commanded field sorties with partial-launch persistence (`FO-6`–`FO-9`, `FO-15`, `Q-D3`–`Q-D6`, `Q-F1`, `Q-F2`, `P-1`), plus `tacsim` live `Arena` reconciliation, surface `BlockTenure` ground-tenure mapping, and active-combat mining pause (`Q-H1`) | `mush_tactics::field_ops::sortie`, `tacsim::field_ops` |
| `W-clock` | Unplanned-outage detection (`>= 120 s` vs `< 120 s` silent re-anchor, `Q-R4`), 5-minute (`300 s`) recovery countdown freeze (`FO-11`, `Q-H11`), reconnect banner & milestone broadcasts (`5m`, `3m`, `1m`, `30s`, `10s`, `0s`), in-flight unit state quarantine (`P-4`), and automatic 48-hour salvage claim deadline extension (`Q-F3`, `Q-F4`) bridged with `mush_core::clock::WorldTimeline` | `mush_tactics::field_ops::clock`, `tacsim::field_ops` |
| `Live Coupled Towing & Interdiction` | Coupled surface drive motion (`attach_site_tow`, `step_coupled_tow_motion`, `coupled_drive_rates_for_tow`, `step_coupled_surface_tow_drive_leg`), mid-tow combat interdiction (`interdict_active_tow`) severing the towline and promoting both tug & casualty to `1:1` combat (`H1-B` + `H2-A`, `Q-S7`), emergency tow drop (`emergency_drop_tow`), and tug/casualty damage synchronization (`apply_field_damage_with_recovery_sync`) | `mush_tactics::field_ops::recovery`, `mush_tactics::field_ops::engagement`, `tacsim::field_ops` |
| `Black-Box Forensics & Contraband Provenance` | Tamper-evident per-hull black-box flight logs (`BlackBoxFlightEntry`, `append_black_box_entry`), physical-custody forensic extraction (`extract_black_box_forensics` -> `BlackBoxForensicReport` -> `SalvageEvidenceKind::BlackBoxForensics`, `FO-4`, `Q-S4`, `P-3`), unlawful field hot-stripping (`StripAuthorization::UnlawfulFieldHotStrip`), and contraband component provenance gating at legitimate workshops (`ProvenanceLegalityStatus`, `ContrabandReason`, `WorkshopError::ContrabandComponentRefused`, `Q-F5`, `Q-F6`) | `mush_tactics::field_ops::salvage`, `mush_tactics::field_ops::workshop`, `tacsim::field_ops` |
| `Field Engineering & Resupply Tenders` | Registered mobile recovery rigs & resupply tenders (`FieldTenderState`, `FieldEngineeringBook`), field component unjamming (`field_unjam_component`), damaged section field stabilization (`field_stabilize_damaged_section`), field ammo rearming & charge transfer (`field_resupply_ammo_and_charge`), and strict enforcement of the `H3` workshop boundary (`FieldEngineeringError::RequiresWorkshopFacility` for destroyed sections, destroyed/stripped components, and irrecoverable wrecks) | `mush_tactics::field_ops::engineering`, `tacsim::field_ops` |

---

## 1. `H1-A` — Field-Unit Component Survival & Operational State (`mush_tactics::field_ops::condition`)

### 1.1 Section-Localized Component Survival (`FO-1`, `FO-2`, `Q-H8`)
- **Component condition vocabulary (`ComponentCondition`)**: `Intact`, `Damaged`, `Jammed`, `Destroyed`.
- **Section condition vocabulary (`SectionCondition`)**: `Intact`, `Damaged`, `Destroyed`, `Wreck`, `Irrecoverable`.
- **Localized destruction**: An installed component becomes `ComponentCondition::Destroyed` if and only if:
  1. Its stored component condition is `Destroyed`, or
  2. Its component-local health cell (`condition`) reaches its floor (`<= min`), or
  3. Its host structural section (`InstallPoint::Section` or `InstallPoint::Slot`) is structurally destroyed (`SectionCondition::Destroyed` or `SectionCondition::Irrecoverable`).
- Components installed in surviving sections (`Intact`, `Damaged`, or non-obliterated core `Wreck` sections) retain their actual `Intact`, `Damaged`, or `Jammed` condition and their remaining ammunition/charge cells.

### 1.2 Operational State Derivation & Hull Epoch Disposition (`Q-H7`, `Q-H9`)
`assess_tactical_unit(unit, anatomy)` derives `FieldUnitAssessment` purely from the `TacticalUnit` and its `UnitAnatomyProfile`:
- `OperationalState::Operational`: retains power and command control, can move under its own power, and (for armed hulls) has at least one operational weapon with ammunition.
- `OperationalState::Immobile`: retains power and command control, but locomotion is lost (either a `required_locomotion_sections` entry such as `engine` is destroyed, or functional `locomotion_sections` falls below `min_locomotion_sections`, such as losing both tracks or both legs).
- `OperationalState::Unarmed`: retains power, command control, and mobility, but all weapons on an armed hull are destroyed, jammed, or depleted.
- `OperationalState::Disabled`: inoperable under its own control or power (e.g. cockpit/control section such as `head` destroyed while core/limbs survive, power plant offline, or both immobile and disarmed), yet remains a physical, towable unit with surviving structure and components.
- `OperationalState::Wreck`: primary core section (`hull` or `torso`) reduced to `0` internals (`Wreck`), immobilized and inoperable on the field, while remaining a physical, towable wreck whose surviving non-destroyed sections retain their components.
- `OperationalState::Irrecoverable`: catastrophic total loss (`Irrecoverable` section condition or all sections destroyed); leaves no towable unit.

**Hull Epoch Disposition (`Q-H9`)**:
- Recoverable field states (`Operational`, `Immobile`, `Unarmed`, `Disabled`, `Wreck`) map to `FieldEpochDisposition::RemainOpen`. `validate_close_outcome` (and `tacsim::field_ops::validate_ic_close_for_unit`) refuses closing a recoverable unit's hull epoch as `IcCloseKind::Destroyed`; its epoch remains open across disablement, abandonment, and towing until carrier/hangar recovery closes it as `IcCloseKind::RecoveredToBase { base }`.
- Only `OperationalState::Irrecoverable` maps to `FieldEpochDisposition::CloseDestroyed` and permits `IcCloseKind::Destroyed`.

### 1.3 IC Condition Plan & Versioned Authority Handoff (`§4.4`, `FO-10`)
- **`ic_condition_plan(before, after, anatomy, cause)`**:
  - Produces a `UnitMutationPlan` with `expected_revision = Some(before.revision())` using **only** the `mush_tactics::ic_custody`-accepted vocabulary (`SetCell`, `SetSectionCondition`, `SetComponentCondition` — never `RevokeCapability` or `GrantCapability`).
  - Strips OOC arena blanket `"wreck"` stamps from sections whose `internals` cell is still positive, ensuring surviving sections and their hosted components remain `intact` or `damaged`.
- **`FieldCustodyLedger` & `FieldCustodyRecord`**:
  - Tracks `authority_generation` (`>= 1`), `FieldAuthorityKind` (`Hangar`, `CarrierBay`, `FieldSurface`, `CombatInstance`), `operational_state`, `epoch_disposition`, `unit_revision`, `sortie_id`, and retained cargo (`cargo_mass_kg`, `cargo_volume_l`).
  - Fences concurrent/stale writers via `expected_generation`, refuses `unit_revision` regression, prevents `Irrecoverable` units from entering new combat instances, and explicitly blocks OOC snapshot restoration (`refuse_ooc_snapshot_restore`) on IC-custodied units.

---

## 2. `H2-A` — Universal Tow Coupling & Durable Recovery State (`mush_tactics::field_ops::recovery`)

### 2.1 Universal `2×` Tonnage Cap & Speed Scaling (`FO-13`, `Q-S6`)
- **Mass basis (`UnitMassProfile`)**: `total_mass_kg = dry_mass_kg + cargo_mass_kg`, plus `cargo_volume_l` and dimensional envelope (`width_cm`, `height_cm`).
- **Inclusive `2×` ceiling**: Any operational mobile unit (`tug`) can tow one towable casualty (`Immobile`, `Unarmed`, `Disabled`, or `Wreck`) provided `casualty_mass_kg <= tug_mass_kg * 2` (`MAX_TOW_MASS_RATIO_PERMILLE = 2_000`). Targets exceeding `2×` tug mass are refused with `RecoveryError::ExceedsTonnageCap`.
- **Deterministic speed scaling (`tow_speed_factor_permille` / `scaled_tow_speed_cms`)**:
  - Let `ratio_permille = (casualty_mass_kg * 1_000) / tug_mass_kg` in `0..=2_000`.
  - `TowRigProfile::Standard` (all ordinary mobile units):
    $$\text{factor}_{\text{permille}} = 850 - \left\lfloor \frac{300 \times \text{ratio}_{\text{permille}}}{1000} \right\rfloor$$
    yielding `70%` base speed at `0.5×` tug mass, `55%` at `1.0×` equal mass, and a `25%` crawl at the `2.0×` ceiling.
  - `TowRigProfile::RecoveryOptimized` (dedicated recovery vehicles/rigs):
    $$\text{factor}_{\text{permille}} = 950 - \left\lfloor \frac{225 \times \text{ratio}_{\text{permille}}}{1000} \right\rfloor$$
    obeying the same universal `2×` tonnage ceiling while retaining higher speed under load (`83.8%` at `0.5×`, `72.5%` at `1.0×`, `50%` at `2.0×`).

### 2.2 Physical Recovery Stages, Exclusivity & Carrier Bay Loading (`Q-S5`–`Q-S9`, `Q-S11`)
- **Inspect (`inspect_field_unit` -> `FieldInspectionReport`)**: Reports the target's `FieldUnitAssessment` (operational state, intact/degraded/destroyed components), `UnitMassProfile` (dry mass + retained cargo), and `TowEvaluation` (mass ratio, coupled speed, or refusal reason for a prospective tug).
- **Exclusive Attach (`RecoveryBook::attach`)**:
  - Requires same `site`, physical distance `<= max_attach_distance_cm`, mobile operational tug, towable casualty (`!Operational && !Irrecoverable`), and `casualty_mass <= 2 * tug_mass`.
  - Enforces exclusive first-completed physical attachment (`Q-S9`): at most one active (`Attached` or `Towing`) operation per tug or casualty; a towed unit cannot act as a tug.
- **Coupled Movement & Detach (`RecoveryBook::advance_tow`, `RecoveryBook::detach`)**:
  - Advances both tug and casualty pose together on the same site, enforcing `step_distance_cm <= coupled_speed_cms * dt_secs` and accumulating `distance_towed_cm`.
  - Detaching leaves the casualty at its current `FieldPose` in its exact damaged state.
- **Carrier Bay Load (`RecoveryBook::load_into_bay`)**:
  - Checks `BayCapacitySpec` (`max_units`, `max_width_cm`, `max_height_cm`, `max_mass_kg` including retained cargo mass, and `max_volume_l` including retained cargo volume, per `Q-S8`).
  - Verifies the casualty's `TacticalUnit::revision()` matches `casualty_unit_revision` (refusing any silent healing/reset) and returns `LoadedCasualtyOutcome` with `FieldCloseOutcome::RecoveredToBase { base: carrier }` (`IcCloseKind::RecoveredToBase`).

---

## 3. `H1-B` & `H1-C` — Field Combat Entry, Motion-Clock Handoff, Crew Survival & Observer Projection (`mush_tactics::field_ops::engagement`)

### 3.1 Promotion, Demotion & Motion-Clock Discontinuity (`Q-H1`–`Q-H4`, `Q-R3`, `Q-S9`, `Q-S10`)
- **Clock Rates (`Q-R3`)**: Unengaged frontier site surface operations advance on `IC_SURFACE_CLOCK_RATE = 24` (`24` IC seconds per real second); active combat (`FieldSiteEngagementBook::is_engaged()`) runs on `COMBAT_WALL_CLOCK_RATE = 1` (`1:1` wall time).
- **Promotion (`promote_to_engagement`)**: Triggered by `FieldPromotionTrigger::{ExplicitHostileAction, ArmedPerimeterCrossing, MutualEngagementOrder, RecoveryInterdiction}`. Hands off every unit on the site from `FieldAuthorityKind::FieldSurface` to `FieldAuthorityKind::CombatInstance` at its exact current `FieldPose` (`MotionClockHandoff`) without rewinding or duplicating cumulative distance.
- **Reinforcement & Withdrawal (`reinforce_engagement`, `withdraw_unit`, `Q-H3`)**: Reinforcements enter at a validated site entry pose; mobile units (or towed casualties) withdraw to `WithdrawalDestination::{PerimeterSurface, CarrierBay, Hangar}`.
- **Demotion Settlement (`demote_engagement`, `Q-H4`, `Q-S10`)**: Blocked (`EngagementError::DemotionBlocked`) while `DemotionSettlementGate` has any non-zero `pending_impacts`, `active_hostile_contacts`, or `pending_boundary_handoffs`. Once settled, transitions remaining units back to `FieldAuthorityKind::FieldSurface`, leaving `Immobile`, `Disabled`, and `Wreck` units as persistent field casualties with `FieldEpochDisposition::RemainOpen`.
- **Carrier Leave-Behind Confirmation (`depart_carrier`, `Q-H10`)**: Refuses carrier departure when sortie units remain outside the carrier bay unless `confirm_leave_behind == true`; when confirmed, leaves those units in the field without deleting them, releasing title, or closing their hull epochs.

### 3.2 Link-Loss Posture, Crew Survival & Observer Digest (`Q-H5`, `Q-H6`, `Q-H12`, `Q-F4`, `P-2`, `P-5`, `FO-12`)
- **Link-Loss Handover (`resolve_link_loss_posture`)**: Remains `PlayerControlled` while `disconnected_elapsed_ms < linkdead_grace_ms + reclaim_min_ms`; thereafter switches to `SealedMindFallback` if a mind is sealed on the hull, or `SafeHoldPosture` (no fire, no motion, `P-2`) when no mind is sealed.
- **Crew Survival & Guaranteed NPC Extraction (`CrewSurvivalBook`)**:
  - Enforces **zero player permadeath** (`FO-12`, `Q-H12`): unit defeat transitions occupants to `AboardUnit { injured }` or `StrandedInField`, never death.
  - `request_npc_extraction` (`Q-F4`, `P-5`): triggered strictly on the stranded character's explicit request (never an automatic timeout extracting an unwilling pilot), refused **only** while an unexpired `FriendlyRescueUnderway` is active (`EngagementError::FriendlyRescueActive`), paused/extended across world outage freezes (`pause_extractions_for_outage` / `resume_extractions_after_outage`), and delivers the character to the nearest eligible known safe port (or fallback `"phobos"`) while leaving the hull, equipment, and cargo in the field with an open epoch.
- **Observer-Bounded Post-Battle Digest (`FieldObservationBook`, `Q-H6`)**: Scopes `ObservedCombatFact` entries strictly to the `observer` (`DirectLineOfSight`, `DeliveredReport`, `FieldInspection`, `RecoveredBlackBox`), preventing any omniscient public battle replay or hidden component leak.

---

## 4. `H2-B` & `H2-C` — Recovery Entitlement, Unlawful Salvage Evidence & 48-Hour Salvage Claims (`mush_tactics::field_ops::salvage`)

### 4.1 Ground Tenure & Lawful vs Unlawful Entitlement (`FO-4`, `Q-S1`, `Q-S3`, `Q-S4`)
- **Pinned Tenure (`GroundTenure`, `Q-S3`)**: Captured at the casualty's position when physical attachment begins (`Unclaimed` or `Owned { landholder, claim_id }`) and persisted across continuous recovery.
- **Independent Entitlements (`evaluate_recovery_entitlement`, `Q-S1`)**:
  - `LawfulSelfOrAgent`: unit title holder or authorized agent (lawful on both unclaimed and owned ground without needing landholder permission).
  - `LawfulLandholderOrAgent`: recognized landholder or authorized agent on `Owned` ground (lawful without needing unit-owner permission).
  - `LawfulOpenGround`: any capable recoverer on `Unclaimed` ground.
  - `UnlawfulOnOwnedGround`: third party on `Owned` ground lacking both owner and landholder authority (`FO-4`, `Q-S4`). Physically permitted to attach, tow, and load (`Q-S5`), recorded privately in `AuthoritativeRecoveryRecord`.
- **Evidence Gate & Post-Close Black-Box Access (`Q-S4`, `P-3`)**:
  - Inspecting a third party's `AuthoritativeRecoveryRecord` or contesting an unlawful recovery requires a matching `SalvageEvidenceRecord` (`DirectWitness`, `SensorLog`, `CustodyInspection`, or `RecoveredBlackBox`).
  - `read_recovered_black_box` (`P-3`): requires physical/custody access to the bay holding the recovered hull (`has_physical_bay_access == true`); a distant prior owner cannot telepathically read the black box upon epoch close.

### 4.2 Salvage Title Claims, Outage Pause & Contest Adjudication (`Q-S2`, `Q-S13`–`Q-S15`, `Q-F3`, `Q-F5`, `Q-F6`, `Q-R1`)
- **Self-Recovery (`resolve_hull_recovery_custody`)**: Immediately returns `CustodyTitleStatus::ResolvedTitle { owner }` with no claim or contest window (`Q-S1`, `Q-S14`).
- **Non-Owner Recovery**: Opens a `SalvageClaim` with `CONTEST_WINDOW_AVAILABLE_REAL_SECS = 172_800` (`48` available real hours, `Q-S15`), paused and extended across global world outage freezes via `pause_claims_for_outage` and `resume_claims_after_outage` (`Q-F3`, `P-4`).
- **Contest Grounds (`submit_contest`, `adjudicate_claim`, `Q-S13`, `Q-R1`)**:
  - `ContestGround::BarePriorTitle`: deterministically rejected (`valid_to_vacate = None`) per explicit owner ruling `Q-R1` — bare prior title alone never defeats a lawful salvage claim.
  - `ContestGround::UnlawfulRecovery { evidence }`, `DefectiveProcedure`, and `SuperiorSpecificRight`: vacate the claim (`SalvageClaimState::Vacated`) and restore title to the winning challenger without reimbursing `documented_repairs_at_risk` (`Q-S14`).
  - Uncontested claims (or claims with only rejected contests) settle at `now_real_s >= deadline_real_s` as `SalvageClaimState::AwardedToClaimant`, transferring clean legal title to the claimant.
- **Pending-Title Custody Gate (`check_custody_action_allowed`, `Q-S14`, `Q-F5`)**: Permits `Stabilize` and `DocumentedRepairAtRisk` while title is pending, and refuses `DeploySortie`, `SellOrTransfer`, `IssueUseGrant`, `StripComponent`, and `ScrapHull` unless explicit owner permission is present.

---

## 5. `H3` — Component Stripping, Scrapping & Workshop Restoration (`mush_tactics::field_ops::workshop`)

- **Component Stripping (`WorkshopBook::strip_component`, `Q-S11`, `Q-F5`, `Q-F6`)**:
  - Enforces `StripAuthorization::{BayOrWorkshop, Field}` (refusing pending-claim stripping without owner permission per `Q-F5` and unlawful field stripping on owned ground per `Q-F6`).
  - Detaches surviving (`Intact`, `Damaged`, or `Jammed`) components into a `StrippedComponentRecord` preserving component definition, condition, cells (such as remaining ammo), asset pointer, and `ComponentProvenance { parent_hull, parent_schema, host_section, stripped_by, stripped_at_ic, field_stripped }`.
  - Marks the hull slot `"stripped"`, zeroes its cells to `min`, clears its asset pointer, and registers the stripped component on `SalvageClaimBook` so the remaining chassis cannot double-strip or double-claim the removed part (`Q-F6`).
- **Hull Scrapping (`WorkshopBook::scrap_hull`, `Q-S11`, `Q-F5`)**:
  - Requires resolved title or explicit owner permission (`Q-F5`), computes deterministic `ScrapYieldReport` (`structural_scrap_kg` + `dismantled_salvageable_components`), and transitions the hull to `OperationalState::Irrecoverable`.
- **Workshop Repair, Section Reconstruction & Component Replacement (`FO-2`, `Q-S12`, `Q-S14`)**:
  - `repair_section` & `repair_component`: repair `Damaged`/`Jammed` sections or components in place without resetting untouched sections, components, or ammo (`Q-S12`), logging an `AtRiskRepairEntry` when performed under `PendingSalvageClaim` (`Q-S14`).
  - `reconstruct_section`: rebuilds a `Destroyed` or `Wreck` section shell to `Intact` while keeping components slagged when that section was destroyed explicitly `"destroyed"` until replaced (`FO-2`, `Q-S12`).
  - `replace_component_in_slot`: installs a compatible `ReplacementPartSpec` (manufactured or built via `ReplacementPartSpec::from_stripped_record`) into a `"destroyed"` or `"stripped"` slot whose host section is functional, recomputing `FieldUnitAssessment` (e.g. restoring a headshot `Disabled` biped or disarmed `Unarmed` unit back to `Operational`).
  - `rearm_component` & `quote_full_restoration`: replenishes component ammunition/charge cells (`WorkshopOperationKind::RearmComponent`) without altering unrelated structure or components, and computes deterministic facility restoration quotes (`WorkshopFacilityTariff` -> `WorkshopCostQuote`) across structural repair, section reconstruction, component repair, slot replacement, and rearming.

---

## 6. Sortie Orchestration, Crew Pools, Personnel Manifests & `tacsim` Arena/Surface Bridges (`mush_tactics::field_ops::sortie`, `tacsim::field_ops`)

- **Carrier Personnel Manifest & Embarkation Provenance (`CarrierPersonnelManifest`, `EmbarkationOrigin`, `FO-16`, `Q-R2`)**:
  - Tracks every player and hired NPC embarked aboard a carrier or expedition base with explicit `EmbarkationOrigin` (`LastPortDeparture { port_id }`, `ShipToShipTransfer { from_vessel }`, `FieldRescue { site, rescued_from_unit }`, `RecoveredHullCrew { hull }`).
  - Validates that a pilot or NPC commander is physically embarked aboard the carrier before boarding or launching a sortie slot (`Q-R2`).
- **Port NPC Crew Pools & Daily Wage Settlement (`PortCrewPool`, `NpcCrewContract`, `Q-R5`, `Q-H11`)**:
  - Manages hired human NPC pilots, gunners, and detachment commanders by home port and employer, accruing daily IC wages (`IC_DAY_SECONDS = 86_400`) via `settle_npc_wages` while pausing wage accrual during world outage freezes (`Q-H11`).
- **Unit Expense Payer Policy (`UnitExpensePolicy`, `UnitExpenseCategory`, `Q-A4`)**:
  - Resolves explicit payer principals for `Ammunition`, `Recovery`, and `Repair` expenses on borrowed, leased, or org-owned hulls (`default_payer` = unit owner unless overridden per expense category).
- **Sortie Launch, Partial-Launch Persistence & Leadership Succession (`ActiveFieldSortie`, `FO-6`–`FO-9`, `FO-15`, `Q-D3`–`Q-D6`, `Q-F1`, `Q-F2`, `P-1`)**:
  - Supports both player-led and organizer-launched no-player NPC-commanded sorties (`FO-15`, `Q-F1`), issuing bounded `MissionLeaderRoleGrant`s (`Q-F2`) when leadership transfers to a human player or embarked human NPC successor (`P-1`).
  - Enforces partial-launch persistence (`Q-D6`): slots that pass bay/title/fitness checks deploy into `FieldSiteEngagementBook` and remain in the field even if another slot is blocked (`Blocked { reason }`), allowing blocked slots to be retried or cancelled (`cancel_unlaunched_slot`) without rolling back already-launched units.
  - `sync_detachment_availability_from_field` and `reconcile_leadership_from_field` project live unit operational state (`FieldSiteEngagementBook`) and crew fitness (`CrewSurvivalBook`) into `DetachmentRoster::reconcile`.
- **`tacsim::field_ops` Live Arena & Surface Bridges**:
  - `reconcile_arena_to_field_engagement`: reconciles `Arena::returned_units()` and combat `Notice`s into `FieldSiteEngagementBook`, `FieldCustodyLedger`, `CrewSurvivalBook`, and observer-bounded `FieldObservationBook` using `Arena` contact visibility (`Q-H6`, `Q-H8`, `Q-H12`).
  - `block_letter_at_site_pose` & `ground_tenure_at_site_pose`: maps a casualty's `FieldPose` (`x_cm`, `y_cm`) to its `10 m` surface cell and `4×4` frontier block letter (`'A'..='P'`) and resolves live `tacsim::surface::claims::BlockTenure` records into `GroundTenure` (`FO-4`, `Q-S1`, `Q-S3`).
  - `can_run_surface_mining_for_unit`: enforces `Q-H1` by refusing/pausing surface mining on a site while `site_book.is_engaged()` is true or when the unit is not `Operational` on `FieldSurface`.

---

## 7. `W-clock` — Unplanned-Outage Freeze, 5-Minute Recovery Countdown & Deadline Extension (`mush_tactics::field_ops::clock`, `tacsim::field_ops`)

- **Outage Threshold & Silent Re-anchor (`OutageFreezeConfig`, `FieldClockCoordinator::resolve_startup_gap`, `Q-R4`)**:
  - Sub-threshold restart gaps (`< 120` real seconds) return `FieldOutageResolution::SilentReanchor`, re-anchoring `(real_anchor, ic_anchor)` from the last durable checkpoint with zero countdown and zero unseen IC catch-up.
  - Gaps meeting or exceeding `DEFAULT_OUTAGE_FREEZE_THRESHOLD_REAL_S = 120` enter `FieldOutageResolution::CountdownFrozen(ActiveFieldFreeze)` for `DEFAULT_OUTAGE_COUNTDOWN_REAL_S = 300` (`5` real minutes, `FO-11`, `Q-R4`).
- **Reconnect Banner, Milestone Broadcasts & In-Flight Quarantine (`P-4`)**:
  - `reconnect_banner(now_real_unix)` produces `OutageReconnectBanner` with exact `remaining_real_secs` and `frozen_ic_unix`.
  - `poll_due_broadcast_milestones(now_real_unix)` emits `OutageBroadcastMilestone::{FiveMinutes, ThreeMinutes, OneMinute, ThirtySeconds, TenSeconds, Unfrozen}` at most once per countdown.
  - `quarantine_unit` / `clear_quarantine` holds units with corrupted/ambiguous mid-outage state in `QuarantinedUnitRecord` without mutating, deleting, or rolling back healthy units (`P-4`).
- **Unfreeze & Salvage Deadline Extension (`complete_unfreeze_and_adjust_claims`, `Q-F3`, `Q-F4`)**:
  - Accumulates `outage_paused_real_secs = unfreeze_real_unix - frozen_real_unix` (downtime + countdown) and extends every pending 48-hour salvage claim deadline on `SalvageClaimBook` (`AdjustedSalvageClaimDeadline`).
  - `tacsim::field_ops::{resolve_world_outage_with_field_clock, complete_world_outage_unfreeze_with_field_clock}` synchronize `FieldClockCoordinator` directly with `mush_core::clock::WorldTimeline`.

---

## 8. Live Coupled Surface Drive & Mid-Tow Combat Interdiction (`mush_tactics::field_ops::recovery`, `mush_tactics::field_ops::engagement`, `tacsim::field_ops`)

- **Coupled Surface Motion (`attach_site_tow`, `step_coupled_tow_motion`)**:
  - Validates same-site non-combat presence for both tug and casualty on `FieldSiteEngagementBook` and advances both `RecoveryOperation::{tug_pose, casualty_pose}` and `FieldSiteEngagementBook` unit poses in lockstep within `coupled_speed_cms * dt_secs`.
  - `EngagementError::UnitCoupledToActiveTow` refuses independent withdrawal or site demotion while a unit is coupled to an active tow (`Q-S7`).
- **Mid-Tow Combat Interdiction & Emergency Drop (`interdict_active_tow`, `emergency_drop_tow`, `H1-B` + `H2-A`)**:
  - Hostile contact during an active tow severs the towline (`RecoveryStage::Released` at the current coupled pose) and promotes the site into `1:1` combat (`promote_to_engagement`).
  - `apply_field_damage_with_recovery_sync` and `RecoveryBook::sync_casualty_damage` propagate field combat damage onto coupled operations: disabling the tug releases the casualty in place, while obliterating the casualty (`Irrecoverable`) aborts the tow (`RecoveryStage::AbortedIrrecoverable`).
- **Surface Drive Rate Scaling (`coupled_drive_rates_for_tow`, `step_coupled_surface_tow_drive_leg`)**:
  - Bridges `RecoveryOperation` with `tacsim::surface::drive::{DriveRates, secs_of_cost, cu_of_cost}`, scaling `max_speed_cms` to `op.coupled_speed_cms()` and scaling `cu_per_nominal_cell` proportionally to combined mass `(tug_mass_kg + casualty_mass_kg) / tug_mass_kg`.

---

## 9. Black-Box Forensics & Contraband Component Provenance (`mush_tactics::field_ops::salvage`, `mush_tactics::field_ops::workshop`, `tacsim::field_ops`)

- **Tamper-Evident Black-Box Flight Log (`BlackBoxFlightEntry`, `BlackBoxForensicReport`, `FO-4`, `Q-H6`, `Q-H9`, `Q-S4`, `P-3`)**:
  - `SalvageClaimBook::append_black_box_entry` (and `tacsim::field_ops::{record_surface_black_box_line_to_forensics, record_arena_notices_to_black_box}`) records combat hits, section destruction, disablement, and sortie seal/close events onto each hull's black box.
  - `extract_black_box_forensics` requires physical custody/bay access (`P-3`) and produces a `BlackBoxForensicReport` that converts directly via `into_salvage_evidence()` into `SalvageEvidenceKind::BlackBoxForensics` to vacate unlawful third-party salvage claims (`Q-S4`).
- **Unlawful Hot-Stripping & Contraband Provenance (`StripAuthorization::UnlawfulFieldHotStrip`, `ProvenanceLegalityStatus`, `ContrabandReason`, `Q-F5`, `Q-F6`)**:
  - Field hot-stripping on owned land without owner/landholder authority opens a contestable part salvage claim (`SalvageSubject::StrippedComponent`).
  - When a hull or part salvage claim is vacated, `mark_stripped_component_contraband_on_vacated_claim` marks all associated stripped components `ProvenanceLegalityStatus::Contraband { rightful_owner, reason }`.
  - Legitimate workshops (`allow_contraband == false`) refuse installing contraband parts with `WorkshopError::ContrabandComponentRefused` until cleared by the rightful owner (`clear_contraband_provenance`).

---

## 10. Field Engineering, Mobile Recovery Rigs & Resupply Tenders (`mush_tactics::field_ops::engineering`, `tacsim::field_ops`)

- **Mobile Field Tenders (`FieldTenderState`, `FieldEngineeringBook`)**:
  - Tracks operational field tenders / recovery rigs on a site with finite `spare_parts_kg`, `ammo_supply_rounds`, `charge_reserve_cu`, and `max_service_distance_cm`.
  - Refuses field engineering while either tender or target is in active combat (`TenderInActiveCombat` / `TargetInActiveCombat`) or while the target is coupled to an active tow (`TargetCoupledToActiveTow`).
- **Field Repairs vs. `H3` Workshop Boundary**:
  - `field_unjam_component` (`field_unjam_tacsim_unit`): clears a `Jammed` component back to `Intact` or `Damaged` in the field using spare parts (`DEFAULT_UNJAM_SPARE_PARTS_KG`), while refusing `Destroyed` components or components hosted in a `Destroyed` section (`FieldEngineeringError::RequiresWorkshopFacility`).
  - `field_stabilize_damaged_section` (`field_stabilize_tacsim_unit_section`): patches structural internals on a `Damaged` section (`DEFAULT_SPARE_PARTS_KG_PER_STRUCTURAL_POINT`), while refusing `Destroyed`, `Wreck`, or `Irrecoverable` sections (`FieldEngineeringError::RequiresWorkshopFacility`).
  - `field_resupply_ammo_and_charge` (`field_resupply_tacsim_unit`): transfers ammunition rounds and power charge (`CU`) from the tender's reserves, computing deterministic credit billing via `UnitExpensePolicy::payer_for(UnitExpenseCategory::Ammunition)`.
