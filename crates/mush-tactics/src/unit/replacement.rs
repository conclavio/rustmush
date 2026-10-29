//! Pure replacement between exact registered unit shapes. This prepares data;
//! it grants no installation, ownership, facility or economic authority. There
//! is deliberately no corresponding ordinary `UnitMutationOp`/World write.

use super::*;

/// The real state removed from, or proposed for, one component. A capture is
/// not a custody proof: a composed installer must prove the incoming asset is
/// available and publish its removal and the displaced asset together.
#[derive(Clone, Debug, PartialEq)]
pub struct ComponentState {
    installed: InstalledComponent,
    cells: BTreeMap<CellId, ScalarCell>,
    capabilities: BTreeMap<CapabilityId, RulesValue>,
}

impl ComponentState {
    /// Capture every current cell and sourced capability, including absence,
    /// without resetting condition, scalar values or economic identity.
    pub fn capture(unit: &TacticalUnit, component: &ComponentId) -> Result<Self, UnitError> {
        unit.validate()?;
        let installed = unit
            .components
            .get(component)
            .cloned()
            .ok_or_else(|| UnitError::UnknownComponent(component.clone()))?;
        let cells = unit
            .cells
            .iter()
            .filter_map(|(address, value)| match address {
                CellAddr::Component {
                    component: source,
                    cell,
                } if source == component => Some((cell.clone(), *value)),
                _ => None,
            })
            .collect();
        let capabilities = unit
            .capabilities
            .iter()
            .filter(|(key, _)| key.source.as_ref() == Some(component))
            .map(|(key, value)| (key.id.clone(), value.clone()))
            .collect();
        Ok(Self {
            installed,
            cells,
            capabilities,
        })
    }

    /// Exact component provenance, install point, condition and asset pointer.
    #[must_use]
    pub const fn installed(&self) -> &InstalledComponent {
        &self.installed
    }
    /// Current component-local scalar state.
    #[must_use]
    pub fn cells(&self) -> &BTreeMap<CellId, ScalarCell> {
        &self.cells
    }
    /// All current capabilities sourced by this component.
    #[must_use]
    pub fn capabilities(&self) -> &BTreeMap<CapabilityId, RulesValue> {
        &self.capabilities
    }
}

/// One pure structural transition. Kept separate from the ordinary damage
/// journal so existing World mutation dispatch cannot acquire a fitting verb.
#[derive(Clone, Debug, PartialEq)]
pub struct ReplacementJournal {
    /// Unit identity, unchanged by replacement.
    pub object: ObjId,
    /// Exact preimage revision.
    pub from_revision: u64,
    /// Exactly one greater than the preimage.
    pub to_revision: u64,
    /// Caller-supplied preparation time; no mobility normalization occurs.
    pub now: i64,
    /// Designated component instance, at the unchanged mount.
    pub component: ComponentId,
    /// Exact old registered root.
    pub before: DefinitionRef,
    /// Exact new registered root.
    pub after: DefinitionRef,
    /// Actual displaced state, retained for later custody; `None` when the
    /// component is fitted into a mount the preimage did not have (doc 17h cut
    /// M8b: a core fitted into a coreless chassis).
    pub displaced: Option<ComponentState>,
    /// Exact inserted state.
    pub inserted: ComponentState,
}

fn mismatch(field: &'static str) -> UnitError {
    UnitError::DefinitionShapeMismatch { field }
}

/// Decode the canonical body from the frozen registry itself, not a caller's
/// same-labelled approximation. `inspect_exact` establishes structural identity
/// only; the eventual owner must separately require current mechanical readiness.
pub fn registered_spec(
    registry: &DefinitionRegistry,
    reference: &DefinitionRef,
) -> Result<UnitSpec, UnitError> {
    if reference.key().kind.as_str() != "unit" {
        return Err(mismatch("replacement unit kind"));
    }
    let entry = registry
        .inspect_exact(reference)
        .map_err(|error| UnitError::DefinitionDescriptor(error.to_string()))?;
    let body: UnitSpecBody = serde_json::from_value(entry.payload().clone())
        .map_err(|error| UnitError::DefinitionDescriptor(error.to_string()))?;
    let spec = body.with_definition(reference.clone());
    let descriptor = spec.descriptor()?;
    // Typed decoding must not erase an unknown field or normalize the payload.
    if descriptor.payload() != entry.payload() || descriptor.references() != entry.references() {
        return Err(mismatch("replacement registered body"));
    }
    TacticalUnit::from_spec(&spec, ObjId(0))?;
    Ok(spec)
}

/// Prepare one replacement from exact registry bodies, preserving every live
/// field outside the designated component. Target roots may change the opaque
/// unit rules, but only from their exact registered preimage: modified rules
/// refuse rather than being reset. A finite ruleset adapter must prove the
/// allowed semantic rule change and incoming health/governance.
///
/// The result is still scratch data. No economic authority, incoming-asset
/// availability, deployment state or durable installation is established here.
pub fn replace_component(
    registry: &DefinitionRegistry,
    unit: &TacticalUnit,
    target: &DefinitionRef,
    component: &ComponentId,
    incoming: &ComponentState,
    expected_revision: u64,
    now: i64,
) -> Result<(TacticalUnit, ReplacementJournal), UnitError> {
    if unit.revision != expected_revision {
        return Err(UnitError::StaleRevision {
            expected: expected_revision,
            actual: unit.revision,
        });
    }
    let before = registered_spec(registry, unit.schema())?;
    let after = registered_spec(registry, target)?;
    before.validate_instance(unit)?;
    if before.definition.key().owner != after.definition.key().owner
        || before.sections != after.sections
        || before.unit_cells != after.unit_cells
        || before.mobility != after.mobility
        || before.rules.schema() != after.rules.schema()
        || unit.rules != before.rules
    {
        return Err(mismatch("replacement unchanged unit shape/rules"));
    }
    // Doc 17h cut M8b (G2): the preimage may lack the component altogether —
    // a fitting into the coreless base of a finite family. Everything else must
    // still be the target's exactly.
    let old = before.components.iter().find(|row| &row.id == component);
    let new = after
        .components
        .iter()
        .find(|row| &row.id == component)
        .ok_or_else(|| UnitError::UnknownComponent(component.clone()))?;
    if old.is_some_and(|old| old.install != new.install)
        || (old.is_none() && unit.components.contains_key(component))
        || before
            .components
            .iter()
            .filter(|row| &row.id != component)
            .ne(after.components.iter().filter(|row| &row.id != component))
        || before
            .capabilities
            .iter()
            .filter(|row| row.key.source.as_ref() != Some(component))
            .ne(after
                .capabilities
                .iter()
                .filter(|row| row.key.source.as_ref() != Some(component)))
    {
        return Err(mismatch("replacement unrelated components/mount"));
    }
    let baseline = TacticalUnit::from_spec(&after, unit.object)?;
    let expected = ComponentState::capture(&baseline, component)?;
    if incoming.installed.definition != expected.installed.definition
        || incoming.installed.install != expected.installed.install
        || incoming.cells.len() != expected.cells.len()
        || incoming.cells.iter().any(|(key, cell)| {
            expected
                .cells
                .get(key)
                .is_none_or(|wanted| cell.min != wanted.min || cell.max != wanted.max)
        })
        || incoming.capabilities.iter().any(|(key, rules)| {
            expected
                .capabilities
                .get(key)
                .is_none_or(|wanted| rules.schema() != wanted.schema())
        })
    {
        return Err(mismatch("replacement incoming shape"));
    }
    if let Some(asset) = incoming.installed.asset
        && (asset == unit.object || unit.components.values().any(|row| row.asset == Some(asset)))
    {
        return Err(mismatch("replacement duplicate asset"));
    }
    let displaced = match old {
        Some(_) => Some(ComponentState::capture(unit, component)?),
        None => None,
    };
    let mut post = unit.clone();
    post.schema = target.clone();
    post.rules = after.rules.clone();
    post.components
        .insert(component.clone(), incoming.installed.clone());
    post.cells.retain(|key, _| !matches!(key, CellAddr::Component { component: source, .. } if source == component));
    for (cell, value) in &incoming.cells {
        post.cells.insert(
            CellAddr::Component {
                component: component.clone(),
                cell: cell.clone(),
            },
            *value,
        );
    }
    post.capabilities
        .retain(|key, _| key.source.as_ref() != Some(component));
    for (id, rules) in &incoming.capabilities {
        post.capabilities.insert(
            CapabilityKey {
                id: id.clone(),
                source: Some(component.clone()),
            },
            rules.clone(),
        );
    }
    post.revision = unit
        .revision
        .checked_add(1)
        .ok_or(UnitError::RevisionOverflow)?;
    after.validate_instance(&post)?;
    let journal = ReplacementJournal {
        object: unit.object,
        from_revision: unit.revision,
        to_revision: post.revision,
        now,
        component: component.clone(),
        before: before.definition,
        after: after.definition,
        displaced,
        inserted: incoming.clone(),
    };
    Ok((post, journal))
}
