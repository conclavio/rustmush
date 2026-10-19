//! Real ephemeral card/program admission for a fixed Registry case. This is a
//! private construction step before the arena can run; failure discards that
//! owned case. It never claims a saved/paid seal or rewrites the declaration.
use super::{
    identity,
    meter::{Binding, MeteredRunner},
    subject::{PreparedSubject, Variant},
};
use crate::arena::{
    Arena,
    core::CoreClass,
    mind::SealEngine,
    profile_seal::SealedCards,
    runtime::{channels_for, prepare_profile_runtime, weapons_for},
};
use mush_core::ObjId;
use mush_roll::commerce::rating::{CaseBinding, RatingError};
use std::{collections::BTreeMap, sync::Arc};

pub(super) struct Installed {
    pub(super) meter: Arc<MeteredRunner>,
    pub(super) binding: CaseBinding,
}
/// Only fixed case construction calls this, with its new private arena and
/// ROMs captured before custody. There is no public API accepting a live arena.
pub(super) fn install(
    arena: &mut Arena,
    subject: &PreparedSubject,
    variant: &Variant,
    initial: u8,
    class: CoreClass,
    roms: &BTreeMap<ObjId, SealedCards>,
    maximum_thinks: u64,
) -> Result<Installed, RatingError> {
    if class == CoreClass::Zero
        || class.ordinal() < subject.subject().declared().ordinal()
        || roms.is_empty()
        || roms.len() > 2
        || usize::from(initial) >= variant.definition.profiles.len()
    {
        return Err(RatingError::Invalid);
    }
    let profile = arena.classes.row(class);
    let definition = variant
        .definition
        .validate()
        .map_err(|_| RatingError::Invalid)?;
    let mut bindings = BTreeMap::new();
    let mut program_identities = Vec::new();
    for (id, rom) in roms {
        let own = arena.unit(*id).ok_or(RatingError::Incomplete)?;
        if own.seal.core_class < class || own.seal.closed {
            return Err(RatingError::Invalid);
        }
        let weapons = weapons_for(own);
        let channels = channels_for(&weapons);
        let names = variant
            .definition
            .profiles
            .iter()
            .map(|p| p.name.clone())
            .collect::<Vec<_>>();
        let cards = SealedCards::prepare(
            variant.definition.clone(),
            &names,
            &profile,
            &weapons,
            &channels,
            arena.odds_floor_permille,
        )
        .map_err(|_| RatingError::Invalid)?;
        let mut runtime = prepare_profile_runtime(
            &mut arena.cards,
            *id,
            &cards,
            rom,
            &arena.classes,
            class,
            &weapons,
            &channels,
            arena.odds_floor_permille,
        )
        .map_err(|_| RatingError::Invalid)?;
        // Preserve canonical profile numbering; selecting a different opening
        // does not rotate names or drop the other saved programs.
        if initial != 0 {
            let ids = weapons.iter().map(|w| w.id.clone()).collect::<Vec<_>>();
            let next = runtime
                .prepare_switch(
                    &arena.cards,
                    *id,
                    initial,
                    0,
                    &profile,
                    &ids,
                    &channels,
                    arena.odds_floor_permille,
                )
                .map_err(|_| RatingError::Invalid)?;
            runtime
                .commit(&mut arena.conditions, next)
                .map_err(|_| RatingError::Invalid)?;
            // Construction has no prior answer and is not a live switch. The
            // first actual profile command still receives the ordinary cooldown.
            runtime.generation = 0;
            runtime.switched_ms = None;
        }
        runtime
            .program
            .install(&mut arena.conditions, *id, profile.slots)
            .map_err(|_| RatingError::Invalid)?;
        let mut unit_bindings = Vec::new();
        for (index, program) in cards.programs.iter().enumerate() {
            let key = cards
                .selected
                .key(u8::try_from(index).map_err(|_| RatingError::Bound)?)
                .ok_or(RatingError::Incomplete)?;
            let card = arena.cards.resolve(key).map_err(|_| RatingError::Invalid)?;
            unit_bindings.push(Binding::capture(&card, program)?);
            program_identities.push((*id, key, program.clone()));
        }
        let own = arena.unit_mut(*id).ok_or(RatingError::Incomplete)?;
        own.seal.class = class;
        own.seal.engine = SealEngine::Scripted;
        own.seal.doctrine = false;
        own.seal.profiles = runtime.profiles.clone();
        own.seal.weapons = weapons;
        own.seal.channels = channels;
        own.seal.rom = runtime.rom.key();
        own.card_profile = crate::arena::card::ActiveProfile {
            index: initial,
            switched_ms: None,
        };
        arena.profiles.insert(*id, runtime);
        let state = arena.minds.get_mut(id).ok_or(RatingError::Incomplete)?;
        if state.pending.is_some() || state.think_seq != 0 {
            return Err(RatingError::Invalid);
        }
        state.memory = b"{}".to_vec();
        state.strikes = 0;
        state.dropped = None;
        bindings.insert(*id, unit_bindings);
    }
    let binding = CaseBinding::new(
        subject.subject().revision().source(),
        definition,
        identity("tacsim.registry.case.cards.v1", &bindings, 512 * 1024)?,
        identity(
            "tacsim.registry.case.programs.v1",
            &program_identities,
            512 * 1024,
        )?,
    );
    let meter = Arc::new(MeteredRunner::new(
        subject.runner(),
        profile,
        bindings,
        maximum_thinks,
    )?);
    for id in roms.keys() {
        arena.runners.insert(*id, meter.clone());
        // The baseline ROM custody scheduled Class 0's first due time. This
        // construction has no collected/pending answer; bind the real tested
        // class's ordinary stagger grid before any private execution begins.
        arena.due.retain(|kind| {
            !matches!(kind,
            crate::arena::state::DueKind::MindThink { unit, .. } if unit == id)
        });
        arena.adopt_mind(*id, None);
    }
    Ok(Installed { meter, binding })
}
