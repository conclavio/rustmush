//! Atomic retention publication for volatile lobby choices.
use super::lobby::{Centre, SimError};
use super::{MatchBook, MatchId, MatchState};
use crate::arena::mind::MindSeal;
use crate::arena::profile_seal::MindOrigin;
use mush_core::{Ctx, World};
use mush_roll::{PreparedChange, ReferenceKey, ReferenceKind, RollError, RollStore, Selection};
use mush_roll::{RevisionKey, RevisionRef};

/// One preparation captures every pin the prospective centre owns. Follow has
/// no extra root: the current saved head remains owner-retained until checkout.
pub(crate) fn prepare(w: &World, centre: &Centre) -> Result<Option<PreparedChange>, RollError> {
    let mut replacements = Vec::new();
    let choices = matches!(centre.state, MatchState::Selection | MatchState::Boarding);
    for slot in centre.slots.iter().filter(|_| choices) {
        if let Some(saved) = slot.mind.as_ref().and_then(|choice| choice.saved.as_ref()) {
            if saved.character != slot.pilot {
                return Err(RollError::Missing);
            }
            if let Selection::Pin { revision } = saved.selection {
                if revision.key.character != saved.character || revision.key.mind != saved.mind {
                    return Err(RollError::Missing);
                }
                replacements.push((
                    ReferenceKey {
                        owner: crate::store::NS.into(),
                        kind: ReferenceKind::Selection,
                        record: format!("slot/{}", slot.pilot.0),
                    },
                    vec![revision],
                ));
            }
        }
    }
    for fill in centre.fill_minds.iter().filter(|_| choices) {
        if let Some(saved) = fill.choice.saved.as_ref()
            && let Selection::Pin { revision } = saved.selection
        {
            if revision.key.character != saved.character || revision.key.mind != saved.mind {
                return Err(RollError::Missing);
            }
            replacements.push((
                ReferenceKey {
                    owner: crate::store::NS.into(),
                    kind: ReferenceKind::Selection,
                    record: format!("fill/{}", fill.team.as_str()),
                },
                vec![revision],
            ));
        }
    }
    let Some(owner) = w.module_state::<RollStore>() else {
        return if replacements.is_empty() {
            Ok(None)
        } else {
            Err(RollError::Missing)
        };
    };
    let releases = owner
        .reference_keys(crate::store::NS, ReferenceKind::Selection)
        .into_iter()
        .filter(|key| !replacements.iter().any(|(next, _)| next == key))
        .collect::<Vec<_>>();
    if replacements.is_empty() && releases.is_empty() {
        return Ok(None);
    }
    owner
        .prepare_reference_batch(crate::store::NS, replacements, releases)
        .map(Some)
}

/// Final fallible step after full layout admission. Both root replacement and
/// centre assignment are published synchronously; no validation follows writes.
pub(crate) fn publish_centre(ctx: &mut Ctx<'_>, next: Centre) -> Result<(), SimError> {
    if ctx.w.module_state::<MatchBook>().is_none() {
        return Err(SimError::Unavailable);
    }
    let prepared = prepare(ctx.w, &next).map_err(|error| SimError::MindAdmission {
        reason: error.to_string(),
    })?;
    if let Some(prepared) = prepared {
        validate_publication(ctx).map_err(|error| SimError::MindAdmission {
            reason: error.to_string(),
        })?;
        let (roll, book) = ctx
            .w
            .module_state_pair_mut::<RollStore, MatchBook>()
            .ok_or(SimError::Unavailable)?;
        let mut operations = Vec::new();
        roll.publish_with(prepared, |writes| operations.extend_from_slice(writes))
            .map_err(|error| SimError::MindAdmission {
                reason: error.to_string(),
            })?;
        *book.centre_mut() = next;
        for operation in operations {
            ctx.persist(operation);
        }
    } else {
        *ctx.w
            .module_state_mut::<MatchBook>()
            .ok_or(SimError::Unavailable)?
            .centre_mut() = next;
    }
    Ok(())
}

fn validate_publication(ctx: &Ctx<'_>) -> Result<(), RollError> {
    if let Some(module) = &ctx.module
        && (module != crate::store::NS
            || ctx.reg.persist_delegation(module, mush_roll::NS).is_none())
    {
        return Err(RollError::Invalid {
            field: "selection".into(),
            message: "persistence delegation unavailable".into(),
        });
    }
    Ok(())
}

pub(crate) fn publish(ctx: &mut Ctx<'_>, prepared: PreparedChange) -> Result<(), RollError> {
    validate_publication(ctx)?;
    let mut operations = Vec::new();
    ctx.w
        .module_state_mut::<RollStore>()
        .ok_or(RollError::Missing)?
        .publish_with(prepared, |writes| operations.extend_from_slice(writes))?;
    for operation in operations {
        ctx.persist(operation);
    }
    Ok(())
}

/// Exact revisions include requested provenance when the effective execution is
/// the ROM. One revision used by several hulls consumes one bounded target.
pub(crate) fn revisions<'a>(seals: impl IntoIterator<Item = &'a MindSeal>) -> Vec<RevisionRef> {
    let mut revisions = std::collections::BTreeSet::new();
    for seal in seals {
        for origin in [&seal.origin, &seal.requested.origin] {
            let reference = match origin {
                MindOrigin::Draft {
                    character,
                    mind,
                    revision,
                    digest,
                } => Some(RevisionRef {
                    key: RevisionKey {
                        character: *character,
                        mind: *mind,
                        number: *revision,
                    },
                    digest: *digest,
                }),
                MindOrigin::Filed { certificate, .. } => Some(RevisionRef {
                    key: RevisionKey {
                        character: certificate.revision_character,
                        mind: certificate.revision_mind,
                        number: certificate.revision_number,
                    },
                    digest: certificate.revision_digest,
                }),
                MindOrigin::Doctrine { .. } => None,
            };
            if let Some(reference) = reference {
                revisions.insert(reference);
            }
        }
    }
    revisions.into_iter().collect()
}

fn match_key(kind: ReferenceKind, id: MatchId) -> ReferenceKey {
    ReferenceKey {
        owner: crate::store::NS.into(),
        kind,
        record: format!("match/{}", id.0),
    }
}

/// Preparation checks ownership, source presence and persistence authority before
/// any Boarding/marker write. MatchRecord roots last as long as the durable result
/// document (the existing result store does not delete those on UI-ring eviction).
pub(crate) fn prepare_checkout<'a>(
    ctx: &Ctx<'_>,
    id: MatchId,
    seals: impl IntoIterator<Item = &'a MindSeal>,
) -> Result<Option<PreparedChange>, RollError> {
    let revisions = revisions(seals);
    if revisions.is_empty() {
        return Ok(None);
    }
    validate_publication(ctx)?;
    let owner = ctx
        .w
        .module_state::<RollStore>()
        .ok_or(RollError::Missing)?;
    owner
        .prepare_reference_batch(
            crate::store::NS,
            vec![
                (match_key(ReferenceKind::OpenSeal, id), revisions.clone()),
                (match_key(ReferenceKind::MatchRecord, id), revisions),
            ],
            vec![],
        )
        .map(Some)
}

/// Reconcile removals after an infallible lifecycle transition, and after boot
/// has decided which Running matches survived. Failure keeps conservative roots;
/// it never permits pruning a revision still used by a match or selection.
pub(crate) fn reconcile(ctx: &mut Ctx<'_>) {
    let next = ctx
        .w
        .module_state::<MatchBook>()
        .map(|book| book.centre().clone())
        .unwrap_or_default();
    let result = prepare(ctx.w, &next).and_then(|prepared| match prepared {
        Some(prepared) => publish(ctx, prepared),
        None => Ok(()),
    });
    if let Err(error) = result {
        tracing::error!(%error, "tacsim: mind selection retention reconciliation refused; roots kept");
    }
    // Bound each owner transaction even if a damaged prior boot left many roots.
    loop {
        let Some(owner) = ctx.w.module_state::<RollStore>() else {
            return;
        };
        let releases = owner
            .reference_keys(crate::store::NS, ReferenceKind::OpenSeal)
            .into_iter()
            .filter(|key| {
                let Some(id) = key
                    .record
                    .strip_prefix("match/")
                    .and_then(|word| word.parse::<u64>().ok())
                else {
                    return false;
                };
                !ctx.w
                    .module_state::<MatchBook>()
                    .and_then(|book| book.get(MatchId(id)))
                    .is_some_and(|record| record.state.is_live())
            })
            .take(mush_roll::MAX_REFERENCE_CHANGES)
            .collect::<Vec<_>>();
        if releases.is_empty() {
            return;
        }
        let result = owner
            .prepare_reference_batch(crate::store::NS, vec![], releases)
            .and_then(|prepared| publish(ctx, prepared));
        if let Err(error) = result {
            tracing::error!(%error, "tacsim: closed mind seal retention reconciliation refused; roots kept");
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::{Engine, ObjId, Outbox};

    struct SourceOwner;
    impl mush_core::GameModule for SourceOwner {
        fn name(&self) -> &'static str {
            crate::store::NS
        }
        fn register(
            &mut self,
            reg: &mut mush_core::Registrar<'_>,
        ) -> Result<(), mush_core::ModError> {
            reg.state(crate::TacsimStore::default());
            reg.state(MatchBook::new());
            Ok(())
        }
    }
    fn world() -> Engine {
        Engine::with_modules(
            World::new_minimal(),
            mush_core::Registry::new(),
            vec![
                Box::new(SourceOwner),
                Box::new(mush_roll::RollModule::with_writers([crate::store::NS])),
            ],
        )
        .unwrap()
    }
    fn with_ctx<T>(engine: &mut Engine, actor: ObjId, f: impl FnOnce(&mut Ctx<'_>) -> T) -> T {
        let mut out = Outbox::new();
        f(&mut Ctx {
            w: &mut engine.world,
            actor,
            switches: Default::default(),
            args: Default::default(),
            out: &mut out,
            reg: &engine.registry,
            module: Some(crate::store::NS.into()),
            typed_state: mush_core::ModuleActionState::unavailable(),
        })
    }
    fn draft(source: &str) -> mush_roll::WorkingCopy {
        mush_roll::WorkingCopy {
            source: source.into(),
            envelope: mush_roll::Envelope {
                schema: serde_json::json!({"sections": []}),
                presets: serde_json::json!({}),
                profiles: serde_json::json!({"Default": {}}),
                rules: serde_json::json!({}),
                declared_class: "II".into(),
                mind_abi: crate::minds::MIND_ABI.into(),
            },
        }
    }
    fn save(engine: &mut Engine, actor: ObjId, name: &str, source: &str) -> RevisionRef {
        let roll = engine.world.module_state_mut::<RollStore>().unwrap();
        let change = match roll.mind(actor, name) {
            Ok(mind) => roll.prepare_edit(actor, name, mind.base(), draft(source)),
            Err(_) => roll.prepare_create(actor, name, draft(source)),
        }
        .unwrap();
        roll.publish_with(change, |_| {}).unwrap();
        let roll = engine.world.module_state::<RollStore>().unwrap();
        let prepared = roll
            .prepare_save(
                actor,
                name,
                roll.mind(actor, name).unwrap().base(),
                "",
                None,
                &crate::minds::authoring::RevisionRules,
                engine.world.module_state::<crate::TacsimStore>().unwrap(),
            )
            .unwrap();
        with_ctx(engine, actor, |ctx| {
            crate::store::publish_mind_revision(ctx, prepared)
        })
        .unwrap()
    }
    fn sealed(held: &super::super::holdings::HeldMind, entry: &str) -> MindSeal {
        use crate::arena::core::CoreClass;
        let catalog = crate::definitions::Catalog::build_fielded(&[], "tacsim").unwrap();
        let key = mush_core::CanonicalId::new(entry).unwrap();
        let cells = crate::definitions::to_tactical_unit(
            &catalog.specs[&key],
            &catalog.bindings[&key],
            ObjId(7),
        )
        .unwrap();
        let terms = super::super::seal::SealTerms {
            authored: held.definition.clone(),
            selected_profiles: vec!["Default".into()],
            identity: held.identity.clone(),
            rated: held.rated,
            engine_for_minds: held.engine,
            class_cap: CoreClass::III,
            match_id: MatchId(9),
            ..super::super::seal::SealTerms::fixture()
        };
        let posture = mush_sim::data::passive_rules_value(serde_json::json!({})).unwrap();
        crate::match_::custody_unit(
            &catalog,
            cells,
            ObjId(7),
            crate::arena::state::PartyId::RED,
            mush_sim::data::Pose::at(1_500, 1_500, 0, posture),
            0,
            crate::arena::state::Controller::Mind,
            1,
            &terms,
        )
        .unwrap()
        .seal
    }
    #[test]
    fn m6_saved_class_ii_uses_real_line_tank_and_exact_revision_roots() {
        use crate::arena::core::CoreClass;
        let actor = ObjId(10);
        let mut engine = world();
        let revision = save(&mut engine, actor, "hunter", "fn think(view, card) {} ");
        let held = super::super::holdings::choose(&engine.world, actor, "draft:hunter", Some("1"))
            .unwrap();
        assert!(
            super::super::holdings::choose(&engine.world, ObjId(11), "draft:hunter", Some("1"))
                .is_err()
        );
        let seal = sealed(&held, crate::definitions::content::LINE_TANK);
        assert_eq!(
            (seal.core_class, seal.class),
            (CoreClass::II, CoreClass::II)
        );
        assert_eq!(seal.engine, crate::arena::mind::SealEngine::Scripted);
        assert_eq!(seal.requested, held.identity);
        assert_eq!(revisions([&seal]), vec![revision]);
        assert_eq!(
            seal.cards.definition.profiles[0].values.len(),
            0,
            "parameter-free authored card stays authored"
        );
        let prepared = with_ctx(&mut engine, actor, |ctx| {
            prepare_checkout(ctx, MatchId(9), [&seal])
        })
        .unwrap()
        .unwrap();
        assert!(
            engine
                .world
                .module_state::<RollStore>()
                .unwrap()
                .reference_keys(crate::store::NS, ReferenceKind::OpenSeal)
                .is_empty()
        );
        with_ctx(&mut engine, actor, |ctx| publish(ctx, prepared)).unwrap();
        let owner = engine.world.module_state::<RollStore>().unwrap();
        assert_eq!(
            owner.reference_keys(crate::store::NS, ReferenceKind::OpenSeal),
            vec![match_key(ReferenceKind::OpenSeal, MatchId(9))]
        );
        assert_eq!(
            owner.reference_keys(crate::store::NS, ReferenceKind::MatchRecord),
            vec![match_key(ReferenceKind::MatchRecord, MatchId(9))]
        );
        // No live match survives this fixture's boot/teardown; only its durable
        // result identity remains a retention root, just as after a real finish.
        with_ctx(&mut engine, actor, reconcile);
        let owner = engine.world.module_state::<RollStore>().unwrap();
        assert!(
            owner
                .reference_keys(crate::store::NS, ReferenceKind::OpenSeal)
                .is_empty()
        );
        assert_eq!(
            owner
                .reference_keys(crate::store::NS, ReferenceKind::MatchRecord)
                .len(),
            1
        );

        let rom = sealed(&held, crate::definitions::content::TARGET_MK1);
        assert_eq!(rom.class, CoreClass::Zero);
        assert_eq!(rom.engine, crate::arena::mind::SealEngine::Native);
        assert_eq!(rom.cards, rom.rom);
        assert_eq!(
            rom.requested, held.identity,
            "no-core ROM retains requested revision provenance"
        );
        assert_eq!(revisions([&rom]), vec![revision]);
        assert!(rom.cards.definition.profiles[0].values.len() == 24);
    }
    #[test]
    fn m6_saved_pin_survives_head_change_and_rename_while_follow_resolves_once() {
        let actor = ObjId(10);
        let mut engine = world();
        let first = save(&mut engine, actor, "hunter", "fn think(view, card) {} ");
        let pinned =
            super::super::holdings::choose(&engine.world, actor, "draft:hunter", Some("1"))
                .unwrap();
        let followed =
            super::super::holdings::choose(&engine.world, actor, "draft:hunter", None).unwrap();
        let frozen = sealed(&followed, crate::definitions::content::LINE_TANK);
        let second = save(
            &mut engine,
            actor,
            "hunter",
            "fn think(view, card) { this.counter = 1; }",
        );
        assert_ne!(first, second);
        let owner = engine.world.module_state_mut::<RollStore>().unwrap();
        let change = owner
            .prepare_rename(
                actor,
                "hunter",
                owner.mind(actor, "hunter").unwrap().base(),
                "renamed",
            )
            .unwrap();
        owner.publish_with(change, |_| {}).unwrap();
        let pin = super::super::holdings::resolve_choice(
            &engine.world,
            actor,
            "old display",
            pinned.saved.as_ref(),
        )
        .unwrap();
        let follow = super::super::holdings::resolve_choice(
            &engine.world,
            actor,
            "old display",
            followed.saved.as_ref(),
        )
        .unwrap();
        assert_eq!(
            revisions([&sealed(&pin, crate::definitions::content::LINE_TANK)]),
            vec![first]
        );
        assert_eq!(
            revisions([&sealed(&follow, crate::definitions::content::LINE_TANK)]),
            vec![second]
        );
        assert_eq!(
            revisions([&frozen]),
            vec![first],
            "already sealed flight does not follow a later head"
        );
    }
    #[test]
    fn m6_stale_retention_prepare_publishes_no_partial_roots_and_boot_releases_selection() {
        let actor = ObjId(10);
        let mut engine = world();
        let revision = save(&mut engine, actor, "hunter", "fn think(view, card) {} ");
        let held =
            super::super::holdings::choose(&engine.world, actor, "draft:hunter", None).unwrap();
        let seal = sealed(&held, crate::definitions::content::LINE_TANK);
        let stale = with_ctx(&mut engine, actor, |ctx| {
            prepare_checkout(ctx, MatchId(9), [&seal])
        })
        .unwrap()
        .unwrap();
        let owner = engine.world.module_state_mut::<RollStore>().unwrap();
        let selection = ReferenceKey {
            owner: crate::store::NS.into(),
            kind: ReferenceKind::Selection,
            record: "slot/10".into(),
        };
        let changed = owner
            .prepare_reference(crate::store::NS, selection, vec![revision])
            .unwrap();
        owner.publish_with(changed, |_| {}).unwrap();
        assert!(with_ctx(&mut engine, actor, |ctx| publish(ctx, stale)).is_err());
        let owner = engine.world.module_state::<RollStore>().unwrap();
        assert!(
            owner
                .reference_keys(crate::store::NS, ReferenceKind::OpenSeal)
                .is_empty()
        );
        assert!(
            owner
                .reference_keys(crate::store::NS, ReferenceKind::MatchRecord)
                .is_empty()
        );
        with_ctx(&mut engine, actor, reconcile);
        assert!(
            engine
                .world
                .module_state::<RollStore>()
                .unwrap()
                .reference_keys(crate::store::NS, ReferenceKind::Selection)
                .is_empty()
        );
    }
}
