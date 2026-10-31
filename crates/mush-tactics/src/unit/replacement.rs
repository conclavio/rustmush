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

/// **One refit, journalled** (doc 31 cut UD.4a, D-31-15): the hull's
/// definition moved from `before` to `after`, at most one component removed
/// (its state kept for custody) and at most one installed.
#[derive(Clone, Debug, PartialEq)]
pub struct RefitJournal {
    /// Unit identity, unchanged by the refit.
    pub object: ObjId,
    /// Exact preimage revision.
    pub from_revision: u64,
    /// Exactly one greater than the preimage.
    pub to_revision: u64,
    /// Caller-supplied preparation time.
    pub now: i64,
    /// Exact old registered root.
    pub before: DefinitionRef,
    /// Exact new registered root.
    pub after: DefinitionRef,
    /// The component taken out, and its actual state, for custody.
    pub removed: Option<(ComponentId, ComponentState)>,
    /// The component fitted, and the state it went in with.
    pub installed: Option<(ComponentId, ComponentState)>,
}

/// **Refit a hull to another registered root** (doc 31 cut UD.4a, D-31-15):
/// take out at most one component and fit at most one. When both happen they
/// share one mount — the same install point — but not one id: a cannon comes
/// out of the turret and a laser goes in, each under its own variant's
/// component id ("mount-stable", the owner's ruling of 2026-10-30, so the
/// shipped variants keep their ids). Everything else about the two roots must
/// be equal — owner, unit cells, mobility, rules schema, every other
/// component and every capability those do not source — and so must their
/// sections, apart from the section cells in `derived`: the cells a ruleset
/// derives from what is fitted (tacsim's heat pair, added with a heat-bearing
/// weapon). A derived cell present on both sides keeps its current value
/// inside the new bounds; one only the target has starts at its `init`.
///
/// Pure, like [`replace_component`]: no custody, availability, facility or
/// economic authority is established here, and nothing is written.
///
/// # Errors
///
/// [`UnitError::StaleRevision`], an unregistered root, or a
/// [`UnitError::DefinitionShapeMismatch`] naming what the two roots differ in.
#[allow(clippy::too_many_arguments)]
pub fn refit_component(
    registry: &DefinitionRegistry,
    unit: &TacticalUnit,
    target: &DefinitionRef,
    remove: Option<&ComponentId>,
    install: Option<(&ComponentId, &ComponentState)>,
    derived: &[CellId],
    expected_revision: u64,
    now: i64,
) -> Result<(TacticalUnit, RefitJournal), UnitError> {
    if unit.revision != expected_revision {
        return Err(UnitError::StaleRevision {
            expected: expected_revision,
            actual: unit.revision,
        });
    }
    if remove.is_none() && install.is_none() {
        return Err(mismatch("refit removes or installs a component"));
    }
    let before = registered_spec(registry, unit.schema())?;
    let after = registered_spec(registry, target)?;
    before.validate_instance(unit)?;
    if before.definition.key().owner != after.definition.key().owner
        || before.unit_cells != after.unit_cells
        || before.mobility != after.mobility
        || before.rules.schema() != after.rules.schema()
        || unit.rules != before.rules
    {
        return Err(mismatch("refit unchanged unit shape/rules"));
    }
    // Sections: equal but for the derivable section cells.
    let strip = |sections: &[SectionSpec]| -> Vec<SectionSpec> {
        sections
            .iter()
            .map(|s| {
                let mut s = s.clone();
                s.cells.retain(|c| !derived.contains(&c.id));
                s
            })
            .collect()
    };
    if strip(&before.sections) != strip(&after.sections) {
        return Err(mismatch("refit sections"));
    }
    let removed_row = match remove {
        Some(id) => Some(
            before
                .components
                .iter()
                .find(|row| &row.id == id)
                .ok_or_else(|| UnitError::UnknownComponent(id.clone()))?,
        ),
        None => None,
    };
    let installed_row = match install {
        Some((id, _)) => Some(
            after
                .components
                .iter()
                .find(|row| &row.id == id)
                .ok_or_else(|| UnitError::UnknownComponent(id.clone()))?,
        ),
        None => None,
    };
    if let (Some(out), Some(fit)) = (removed_row, installed_row)
        && out.install != fit.install
    {
        return Err(mismatch(
            "refit mount: what is fitted goes where what came out was",
        ));
    }
    let out_id = remove;
    let in_id = install.map(|(id, _)| id);
    // What comes out must be gone from the target, and what goes in absent
    // from the preimage (unless it is the same id at the same mount).
    if out_id
        .is_some_and(|id| in_id != Some(id) && after.components.iter().any(|row| &row.id == id))
        || in_id.is_some_and(|id| out_id != Some(id) && unit.components.contains_key(id))
    {
        return Err(mismatch("refit component ids"));
    }
    let untouched = |row: &&ComponentSpec| Some(&row.id) != out_id && Some(&row.id) != in_id;
    let untouched_cap = |row: &&CapabilityInit| {
        row.key
            .source
            .as_ref()
            .is_none_or(|s| Some(s) != out_id && Some(s) != in_id)
    };
    // A component's definition is scoped to its unit (`<unit>.<component>`),
    // so the same machine gun on two variants is two references. An untouched
    // component matches its counterpart on everything but the reference, and
    // the two definitions must be the same component: equal payload and
    // equal references. Its provenance then follows the hull to the target.
    let same_definition = |a: &DefinitionRef, b: &DefinitionRef| -> bool {
        a == b
            || matches!(
                (registry.inspect_exact(a), registry.inspect_exact(b)),
                (Ok(x), Ok(y)) if x.payload() == y.payload() && x.references() == y.references()
            )
    };
    let kept_before: Vec<&ComponentSpec> = before.components.iter().filter(untouched).collect();
    let kept_after: Vec<&ComponentSpec> = after.components.iter().filter(untouched).collect();
    let mut retarget: Vec<(ComponentId, DefinitionRef)> = Vec::new();
    if kept_before.len() != kept_after.len() {
        return Err(mismatch("refit unrelated components"));
    }
    for (a, b) in kept_before.iter().zip(&kept_after) {
        if a.id != b.id
            || a.install != b.install
            || a.condition != b.condition
            || a.cells != b.cells
            || a.asset != b.asset
            || !same_definition(&a.definition, &b.definition)
        {
            return Err(mismatch("refit unrelated components"));
        }
        if a.definition != b.definition {
            retarget.push((b.id.clone(), b.definition.clone()));
        }
    }
    if before
        .capabilities
        .iter()
        .filter(untouched_cap)
        .ne(after.capabilities.iter().filter(untouched_cap))
    {
        return Err(mismatch("refit unrelated capabilities"));
    }
    let baseline = TacticalUnit::from_spec(&after, unit.object)?;
    if let Some((id, incoming)) = install {
        let expected = ComponentState::capture(&baseline, id)?;
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
            return Err(mismatch("refit incoming shape"));
        }
        if let Some(asset) = incoming.installed.asset
            && (asset == unit.object
                || unit
                    .components
                    .iter()
                    .any(|(other, row)| Some(other) != out_id && row.asset == Some(asset)))
        {
            return Err(mismatch("refit duplicate asset"));
        }
    }
    let removed = match out_id {
        Some(id) => Some((id.clone(), ComponentState::capture(unit, id)?)),
        None => None,
    };
    let mut post = unit.clone();
    post.schema = target.clone();
    post.rules = after.rules.clone();
    if let Some(id) = out_id {
        post.components.remove(id);
        post.cells.retain(
            |key, _| !matches!(key, CellAddr::Component { component, .. } if component == id),
        );
        post.capabilities
            .retain(|key, _| key.source.as_ref() != Some(id));
    }
    if let Some((id, incoming)) = install {
        post.components
            .insert(id.clone(), incoming.installed.clone());
        for (cell, value) in &incoming.cells {
            post.cells.insert(
                CellAddr::Component {
                    component: id.clone(),
                    cell: cell.clone(),
                },
                *value,
            );
        }
        for (cap, rules) in &incoming.capabilities {
            post.capabilities.insert(
                CapabilityKey {
                    id: cap.clone(),
                    source: Some(id.clone()),
                },
                rules.clone(),
            );
        }
    }
    for (id, definition) in retarget {
        if let Some(row) = post.components.get_mut(&id) {
            row.definition = definition;
        }
    }
    // The derived section cells: the target's, the current value kept where
    // both roots carry the cell and it fits the new bounds.
    post.cells
        .retain(|key, _| !matches!(key, CellAddr::Section { cell, .. } if derived.contains(cell)));
    for (key, wanted) in &baseline.cells {
        if let CellAddr::Section { cell, .. } = key
            && derived.contains(cell)
        {
            let value = match unit.cells.get(key) {
                Some(now) if now.current >= wanted.min && now.current <= wanted.max => ScalarCell {
                    current: now.current,
                    ..*wanted
                },
                _ => *wanted,
            };
            post.cells.insert(key.clone(), value);
        }
    }
    post.revision = unit
        .revision
        .checked_add(1)
        .ok_or(UnitError::RevisionOverflow)?;
    after.validate_instance(&post)?;
    let journal = RefitJournal {
        object: unit.object,
        from_revision: unit.revision,
        to_revision: post.revision,
        now,
        before: before.definition,
        after: after.definition,
        removed,
        installed: install.map(|(id, state)| (id.clone(), state.clone())),
    };
    Ok((post, journal))
}
