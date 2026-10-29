# Doc 27 (`H1-A` / `H2-A`) — Field-Unit Component Survival & Universal Towing Foundation

The accepted design rules live in [doc 27](27-ic-field-operations-design-and-survey.md) (`FO-1`–`FO-3`, `FO-10`, `FO-13`, `Q-H7`–`Q-H9`, `Q-S5`–`Q-S9`, `Q-S11`, §4.1, §4.4, §5). This brief records the neutral `mush-tactics` foundation (`mush_tactics::field_ops`) and the `tacsim` ruleset integration (`tacsim::field_ops`) for `H1-A` and `H2-A`.

| Milestone | Deliverable | Scope boundary |
|---|---|---|
| `H1-A` | Section-localized component survival, derived operational states (`operational`, `immobile`, `unarmed`, `disabled`, `wreck`, `irrecoverable`), IC-custody-legal condition mutation plans, hull-epoch close gating (`Q-H9`), and versioned authority handoff (`§4.4`, `FO-10`) | Preserves surviving component/section/cell state across defeat; component stripping, scrapping, and workshop repair remain `H3` (`FO-14`, `Q-S11`, `Q-S12`) |
| `H2-A` | Universal tow coupling (`<= 2×` tug mass cap), tonnage-differential speed scaling, recovery-optimized rig curve (`FO-13`, `Q-S6`), field inspection, exclusive physical attachment (`Q-S9`), coupled movement, carrier bay capacity checks (`Q-S8`), and unhealed bay recovery (`FO-1`, `FO-3`) | Carrier bay load preserves damaged `TacticalUnit` state and retained cargo without automatic repair |

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
