//! Core alone constructs this bounded two-core exchange. Containers shared by
//! both moves have one merged postimage; no mutation hook runs after custody.
use super::*;
use crate::core_delivery::CarryCheck;
use crate::reserved_shell::ShellObjectStorage;

pub(crate) struct ExchangeObjects {
    before: BTreeMap<ObjId, Obj>,
    after: BTreeMap<ObjId, Obj>,
    ancestors: BTreeMap<ObjId, Obj>,
    carry: Vec<CarryCheck>,
    fence: Box<dyn crate::core_delivery::DeliveryFence>,
    pub(crate) participants: [DefinitionRef; 2],
    pub(crate) action: DefinitionRef,
    pub(crate) commit: CommitToken,
    writes: Vec<ReplayWrite>,
}
impl std::fmt::Debug for ExchangeObjects {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExchangeObjects")
            .field("objects", &self.before.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}
impl ExchangeObjects {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare(
        world: &World,
        lens: crate::action_state::TypedStateLens<'_>,
        actor: ObjId,
        facts: &ExchangeFacts,
        hooks: Arc<[crate::module::HookReg]>,
        fence: Box<dyn crate::core_delivery::DeliveryFence>,
        participants: [DefinitionRef; 2],
        action: DefinitionRef,
        commit: CommitToken,
    ) -> Result<Self, ParticipantError> {
        facts.validate()?;
        let mut moves = vec![(facts.incoming, facts.source, facts.hull)];
        if let Some(outgoing) = facts.outgoing {
            moves.push((outgoing, facts.hull, facts.return_to.ok_or_else(refused)?));
        }
        let moving: BTreeSet<_> = moves.iter().map(|(id, _, _)| *id).collect();
        let containers: BTreeSet<_> = moves.iter().flat_map(|(_, a, b)| [*a, *b]).collect();
        if !moving.is_disjoint(&containers) || moving.len() != moves.len() {
            return Err(refused());
        }
        let get = |id| world.get(id).cloned().ok_or_else(refused);
        let actor_row = get(actor)?;
        if actor_row.kind != ObjKind::Player || actor_row.flags.contains(Flags::GOING) {
            return Err(refused());
        }
        let mut before = BTreeMap::new();
        for id in moving.iter().chain(&containers) {
            let object = get(*id)?;
            if object.flags.contains(Flags::GOING)
                || (moving.contains(id)
                    && (object.kind != ObjKind::Thing
                        || !object.contents.is_empty()
                        || !object.exits.is_empty()))
                || (containers.contains(id)
                    && !matches!(
                        object.kind,
                        ObjKind::Thing | ObjKind::Room | ObjKind::Player
                    ))
                || (*id == facts.hull && object.kind != ObjKind::Thing)
            {
                return Err(refused());
            }
            before.insert(*id, object);
        }
        if before.len() > MAX_EXCHANGE_OBJECTS {
            return Err(ParticipantError::ImageLimit);
        }
        let mut ancestors = BTreeMap::from([(actor, actor_row)]);
        for container in &containers {
            let mut at = Some(*container);
            let mut seen = BTreeSet::new();
            while let Some(id) = at {
                if seen.len() >= crate::MAX_CONTAINMENT_DEPTH
                    || !seen.insert(id)
                    || moving.contains(&id)
                {
                    return Err(refused());
                }
                let object = get(id)?;
                if object.flags.contains(Flags::GOING) {
                    return Err(refused());
                }
                if let Some(parent) = object.location {
                    let holder = get(parent)?;
                    if holder.contents.iter().filter(|child| **child == id).count() != 1
                        || holder.exits.contains(&id)
                    {
                        return Err(refused());
                    }
                }
                at = object.location;
                ancestors.insert(id, object);
            }
            // The moved core is a leaf, one deeper than this unchanged chain.
            if seen.len() >= crate::MAX_CONTAINMENT_DEPTH {
                return Err(refused());
            }
        }
        let mut after = before.clone();
        let mut carry = Vec::new();
        let attempt = Arc::new(ExchangeCarryAttempt::capture(
            facts,
            &action,
            commit,
            &participants,
        )?);
        for (object, source, destination) in moves {
            let item = before.get(&object).ok_or_else(refused)?;
            let from = before.get(&source).ok_or_else(refused)?;
            let to = before.get(&destination).ok_or_else(refused)?;
            if source == destination
                || item.location != Some(source)
                || from.contents.iter().filter(|id| **id == object).count() != 1
                || from.exits.contains(&object)
                || to.contents.contains(&object)
                || to.exits.contains(&object)
            {
                return Err(refused());
            }
            world
                .can_move_to(object, destination)
                .map_err(|_| refused())?;
            carry.push(CarryCheck::capture_exchange(
                Arc::clone(&hooks),
                world,
                lens,
                actor,
                object,
                destination,
                Arc::clone(&attempt),
            )?);
            let item = after.get_mut(&object).ok_or_else(refused)?;
            item.location = Some(destination);
            item.modified = world.now();
            let from = after.get_mut(&source).ok_or_else(refused)?;
            from.contents.retain(|id| *id != object);
            from.modified = world.now();
            let to = after.get_mut(&destination).ok_or_else(refused)?;
            to.contents.push(object);
            to.modified = world.now();
        }
        let writes: Vec<_> = after
            .values()
            .cloned()
            .map(|object| ReplayWrite::ObjectSet { object })
            .collect();
        canonical_json_bytes_with_limit(
            &serde_json::to_value(&writes).map_err(|_| ParticipantError::Encoding)?,
            IMAGE_BYTES,
        )
        .map_err(|_| ParticipantError::ImageLimit)?;
        let image = Self {
            before,
            after,
            ancestors,
            carry,
            fence,
            participants,
            action,
            commit,
            writes,
        };
        image.recheck(world, lens)?;
        Ok(image)
    }
    pub(crate) fn digests(&self) -> Result<Vec<ExchangeObjectDigest>, ParticipantError> {
        self.before
            .iter()
            .map(|(id, before)| {
                Ok(ExchangeObjectDigest {
                    object: *id,
                    before: object_digest(before)?,
                    after: object_digest(self.after.get(id).ok_or_else(refused)?)?,
                })
            })
            .collect()
    }
    pub(crate) fn writes(&self) -> &[ReplayWrite] {
        &self.writes
    }
    pub(crate) fn recheck(
        &self,
        world: &World,
        lens: crate::action_state::TypedStateLens<'_>,
    ) -> Result<(), ParticipantError> {
        self.fence.recheck(world)?;
        self.local(world)?;
        for carry in &self.carry {
            carry.recheck(world, lens)?;
        }
        self.local(world)
    }
    pub(crate) fn local(&self, world: &World) -> Result<(), ParticipantError> {
        self.fence.local(world)?;
        if self
            .before
            .values()
            .chain(self.ancestors.values())
            .any(|o| world.get(o.id) != Some(o))
        {
            return Err(refused());
        }
        Ok(())
    }
    pub(crate) fn bind(
        self,
        storage: ShellObjectStorage<'_>,
    ) -> Result<BoundExchangeObjects<'_>, ParticipantError> {
        let ShellObjectStorage { objects, .. } = storage;
        if self
            .before
            .values()
            .chain(self.ancestors.values())
            .any(|o| objects.get(o.id.0 as usize).and_then(Option::as_ref) != Some(o))
        {
            return Err(refused());
        }
        Ok(BoundExchangeObjects {
            objects,
            after: self.after,
        })
    }
}
fn object_digest(object: &Obj) -> Result<DefinitionDigest, ParticipantError> {
    let value = serde_json::to_value(object).map_err(|_| ParticipantError::Encoding)?;
    DefinitionDigest::for_canonical_value_with_limit(
        "core.component_exchange.object.v1",
        &value,
        IMAGE_BYTES,
    )
    .map_err(|_| ParticipantError::ImageLimit)
}
pub(crate) struct BoundExchangeObjects<'a> {
    objects: &'a mut Vec<Option<Obj>>,
    after: BTreeMap<ObjId, Obj>,
}
impl BoundExchangeObjects<'_> {
    pub(crate) fn apply(self) {
        for (id, object) in self.after {
            self.objects[id.0 as usize] = Some(object);
        }
    }
}
