//! Authored local-detail quests. Source bindings never rewrite live progress.

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_content::{Appliers, EntryApplier, Pack, PackError, RawEntry};
use mush_core::{Attr, AttrFlags, CanonicalId, Ctx, Flags, GOD, ObjId, ObjKind, World};
use serde::Deserialize;

use super::model::{Objective, Prerequisite, QuestChoice, decode_definitions};
use super::{Checkpoint, QuestDef};

const ATTR: &str = super::DEFINITION_ATTR;

pub fn register_appliers(appliers: &mut Appliers) -> Result<(), PackError> {
    let kind = CanonicalId::new("quest").map_err(|error| PackError::EntryDecode {
        kind: "quest".into(),
        detail: error.to_string(),
    })?;
    appliers.register(crate::NS, Arc::new(QuestApplier { kind }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceQuest {
    npc: CanonicalId,
    #[serde(default)]
    npc_pack: Option<CanonicalId>,
    #[serde(default)]
    prerequisite: Prerequisite,
    #[serde(default)]
    choice: Option<QuestChoice>,
    title: String,
    introduction: String,
    destination: String,
    checkpoints: Vec<SourceCheckpoint>,
    completion: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceCheckpoint {
    id: String,
    title: String,
    room_pack: CanonicalId,
    room_entry: CanonicalId,
    #[serde(default)]
    detail: String,
    instructions: String,
    #[serde(default)]
    objective: SourceObjective,
}

#[derive(Default, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum SourceObjective {
    #[default]
    Observation,
    CoopEnrolment,
    SurveyCommission {},
    Trade {
        origin: String,
        destination: String,
        commodity: String,
        quantity: u64,
    },
    Passenger {
        npc_pack: CanonicalId,
        npc: CanonicalId,
    },
    Contact {
        npc_pack: CanonicalId,
        npc: CanonicalId,
    },
    PaidPassage {
        npc_pack: CanonicalId,
        npc: CanonicalId,
        origin: String,
        destination: String,
    },
    Survey {
        mode: String,
    },
}

fn external_object(
    w: &World,
    pack: &Pack,
    dependency: &CanonicalId,
    entry: &CanonicalId,
    kind: &str,
) -> Result<ObjId, String> {
    let requirement = pack
        .manifest()
        .requires()
        .iter()
        .find(|r| &r.id == dependency)
        .ok_or("quest binding must be an explicit pack dependency")?;
    let content = w
        .module_state::<mush_content::ContentState>()
        .ok_or("content state is unavailable")?;
    let latch = content
        .latches
        .applied(dependency)
        .filter(|latch| {
            !content.latches.is_held(dependency)
                && !latch.is_retired()
                && latch.anomaly().is_none()
                && latch.version() == requirement.version
                && requirement
                    .digest
                    .is_none_or(|digest| digest == latch.digest())
        })
        .ok_or("quest binding has no matching active dependency latch")?;
    let applied_entry = latch.entries().get(entry).or_else(|| {
        if kind != "npc" {
            return None;
        }
        if let Some(station) = dependency.as_str().strip_prefix("neighborhood.") {
            let cast_id = CanonicalId::new(format!("tacsim.cast.{station}")).ok()?;
            return content
                .latches
                .applied(&cast_id)
                .filter(|l| {
                    !content.latches.is_held(&cast_id) && !l.is_retired() && l.anomaly().is_none()
                })?
                .entries()
                .get(entry);
        }
        if let Some(station) = dependency.as_str().strip_prefix("tacsim.cast.") {
            let nb_id = CanonicalId::new(format!("neighborhood.{station}")).ok()?;
            return content
                .latches
                .applied(&nb_id)
                .filter(|l| {
                    !content.latches.is_held(&nb_id) && !l.is_retired() && l.anomaly().is_none()
                })?
                .entries()
                .get(entry);
        }
        None
    });
    applied_entry
        .filter(|entry| entry.kind().as_str() == kind)
        .and_then(mush_content::AppliedEntry::object)
        .filter(|id| {
            w.get(*id).is_some_and(|o| {
                !o.flags.contains(Flags::GOING)
                    && ((kind == "room" && o.kind == ObjKind::Room)
                        || (kind == "npc" && o.kind == ObjKind::Npc))
            })
        })
        .ok_or_else(|| format!("quest binding {dependency}/{entry} is not a live {kind}"))
}

fn quest_profile(w: &World, npc: ObjId) -> Result<Attr, String> {
    let previous = w
        .get(npc)
        .and_then(|o| o.attrs.get(mush_npc::PROFILE_ATTR))
        .ok_or("quest giver has no profile")?;
    let mut profile: mush_npc::AgentProfile = serde_json::from_value(
        previous
            .as_json()
            .cloned()
            .ok_or("quest giver profile is not JSON")?,
    )
    .map_err(|_| "quest giver profile is invalid")?;
    profile.validate().map_err(|e| e.to_string())?;
    if !profile.interfaces.iter().any(|id| id == "quest") {
        profile.interfaces.push("quest".into());
    }
    if !profile.command_refs.iter().any(|id| id == "+quest") {
        profile.command_refs.push("+quest".into());
    }
    profile.validate().map_err(|e| e.to_string())?;
    let mut attr = Attr::json(
        serde_json::to_value(profile).map_err(|e| e.to_string())?,
        previous.owner,
    );
    attr.flags = previous.flags;
    Ok(attr)
}

fn role_catalog(w: &World, npc: ObjId) -> Result<Vec<QuestDef>, String> {
    let Some(old) = w.get(npc).and_then(|o| o.attrs.get(ATTR)) else {
        return Ok(Vec::new());
    };
    if old.owner != GOD {
        return Err("existing quest catalog is not source-owned".into());
    }
    decode_definitions(old.as_json().ok_or("existing quest catalog is not JSON")?)
}

fn catalog_attr(mut definitions: Vec<QuestDef>) -> Result<Attr, String> {
    definitions.sort_by(|a, b| a.key.cmp(&b.key));
    let value = if definitions.len() == 1 {
        serde_json::to_value(&definitions[0])
    } else {
        serde_json::to_value(&definitions)
    }
    .map_err(|error| error.to_string())?;
    decode_definitions(&value)?;
    let mut attr = Attr::json(value, GOD);
    attr.flags = AttrFlags::WIZARD_ONLY | AttrFlags::HIDDEN | AttrFlags::NO_INHERIT;
    Ok(attr)
}

fn merge_definition(catalog: &mut Vec<QuestDef>, definition: QuestDef) {
    catalog.retain(|old| old.key != definition.key);
    catalog.push(definition);
}

fn source_npc(w: &World, pack: &Pack, source: &SourceQuest) -> Result<Option<ObjId>, String> {
    if let Some(dependency) = &source.npc_pack {
        return external_object(w, pack, dependency, &source.npc, "npc").map(Some);
    }
    if !pack
        .entry(&source.npc)
        .is_some_and(|entry| entry.kind().as_str() == "npc")
    {
        return Err("npc must name an NPC entry in this pack".into());
    }
    // A new same-pack NPC does not exist until the earlier NPC phase. An update
    // already has an identity and must account for its existing catalog too.
    let existing = w
        .module_state::<mush_content::ContentState>()
        .and_then(|content| content.latches.applied(pack.manifest().id()))
        .and_then(|latch| latch.entries().get(&source.npc))
        .and_then(mush_content::AppliedEntry::object);
    if existing.is_some_and(|npc| {
        !w.get(npc).is_some_and(|object| {
            object.kind == ObjKind::Npc && !object.flags.contains(Flags::GOING)
        })
    }) {
        return Err("quest source NPC is no longer live".into());
    }
    Ok(existing)
}

fn check_source_binding(
    w: &World,
    pack: &Pack,
    id: &CanonicalId,
    npc: Option<ObjId>,
) -> Result<(), String> {
    let old = w
        .module_state::<mush_content::ContentState>()
        .and_then(|content| content.latches.applied(pack.manifest().id()))
        .and_then(|latch| latch.entries().get(id))
        .and_then(mush_content::AppliedEntry::object);
    if old.is_some() && old != npc {
        return Err(format!("{id}: a quest role cannot move to another NPC"));
    }
    Ok(())
}

fn checkpoint(w: &World, pack: &Pack, source: SourceCheckpoint) -> Result<Checkpoint, String> {
    let requirement = pack
        .manifest()
        .requires()
        .iter()
        .find(|r| r.id == source.room_pack)
        .ok_or("quest room_pack must be an explicit pack dependency")?;
    let content = w
        .module_state::<mush_content::ContentState>()
        .ok_or("content state is unavailable")?;
    let latch = content
        .latches
        .applied(&source.room_pack)
        .filter(|latch| {
            !content.latches.is_held(&source.room_pack)
                && !latch.is_retired()
                && latch.anomaly().is_none()
                && latch.version() == requirement.version
                && requirement
                    .digest
                    .is_none_or(|digest| digest == latch.digest())
        })
        .ok_or("quest room_pack has no matching active dependency latch")?;
    let room = latch
        .entries()
        .get(&source.room_entry)
        .filter(|entry| entry.kind().as_str() == "room")
        .and_then(mush_content::AppliedEntry::object)
        .filter(|room| {
            w.get(*room)
                .is_some_and(|o| o.kind == ObjKind::Room && !o.flags.contains(Flags::GOING))
        })
        .ok_or("quest checkpoint is not a live authored room")?;
    let detail_key = format!(
        "atlas:detail:{}",
        mush_core::fold_detail_keyword(&source.detail)
    );
    let detail = w
        .get(room)
        .and_then(|o| o.attrs.get(&detail_key))
        .filter(|a| {
            a.owner == GOD
                && !a
                    .flags
                    .intersects(AttrFlags::WIZARD_ONLY | AttrFlags::SELF_ONLY | AttrFlags::HIDDEN)
        })
        .and_then(Attr::as_text)
        .filter(|text| {
            !text.trim().is_empty()
                && text.len() <= super::model::MAX_DETAIL_BYTES
                && !text.contains('\0')
        });
    let objective = match source.objective {
        SourceObjective::Observation => Objective::Observation,
        SourceObjective::CoopEnrolment => Objective::CoopEnrolment,
        SourceObjective::SurveyCommission {} => Objective::SurveyCommission,
        SourceObjective::Trade {
            origin,
            destination,
            commodity,
            quantity,
        } => Objective::Trade {
            origin,
            destination,
            commodity,
            quantity,
        },
        SourceObjective::Passenger { npc_pack, npc } => Objective::Passenger {
            npc: external_object(w, pack, &npc_pack, &npc, "npc")?,
        },
        SourceObjective::Contact { npc_pack, npc } => Objective::Contact {
            npc: external_object(w, pack, &npc_pack, &npc, "npc")?,
        },
        SourceObjective::PaidPassage {
            npc_pack,
            npc,
            origin,
            destination,
        } => {
            let npc = external_object(w, pack, &npc_pack, &npc, "npc")?;
            let traveller = crate::port::passenger::traveller(w, npc)
                .ok_or("paid passage objective must name an authored traveller")?;
            if traveller.origin != origin || traveller.destination != destination {
                return Err(
                    "paid passage objective must match the traveller's authored route".into(),
                );
            }
            Objective::PaidPassage {
                npc,
                origin,
                destination,
            }
        }
        SourceObjective::Survey { mode } => Objective::Survey { mode },
    };
    if matches!(objective, Objective::Observation) && detail.is_none() {
        return Err("quest checkpoint must name an existing public room detail".into());
    }
    Ok(Checkpoint {
        id: source.id,
        title: source.title,
        room_pack: source.room_pack.to_string(),
        room_entry: source.room_entry.to_string(),
        room,
        detail: source.detail,
        instructions: source.instructions,
        objective,
    })
}

fn definition(
    w: &World,
    pack: &Pack,
    id: &CanonicalId,
    entry: &RawEntry,
) -> Result<(CanonicalId, QuestDef), String> {
    let source: SourceQuest = entry.decode().map_err(|error| format!("{id}: {error}"))?;
    let npc = source_npc(w, pack, &source)?;
    let npc_entry = source.npc.clone();
    let npc_pack = source.npc_pack.clone();
    check_source_binding(w, pack, id, npc)?;
    if let Some(npc) = npc.filter(|_| npc_pack.is_some()) {
        quest_profile(w, npc)?;
    }
    let def = source_definition(w, pack, id, source)?;
    let mut catalog = npc
        .map(|npc| role_catalog(w, npc))
        .transpose()?
        .unwrap_or_default();
    // Account for the entire incoming pack before any entry mutates. Checking
    // entries one by one would admit two additions when only one slot remains.
    for (other_id, other) in pack
        .entries()
        .iter()
        .filter(|(_, e)| e.kind().as_str() == "quest")
    {
        let other: SourceQuest = other
            .decode()
            .map_err(|error| format!("{other_id}: {error}"))?;
        let other_npc = source_npc(w, pack, &other)?;
        if (other.npc == npc_entry && other.npc_pack == npc_pack)
            || (npc.is_some() && other_npc == npc)
        {
            check_source_binding(w, pack, other_id, other_npc)?;
            merge_definition(&mut catalog, source_definition(w, pack, other_id, other)?);
        }
    }
    merge_definition(&mut catalog, def.clone());
    catalog_attr(catalog)?;
    Ok((npc_entry, def))
}

fn source_definition(
    w: &World,
    pack: &Pack,
    id: &CanonicalId,
    source: SourceQuest,
) -> Result<QuestDef, String> {
    let checkpoints = source
        .checkpoints
        .into_iter()
        .map(|source| checkpoint(w, pack, source))
        .collect::<Result<Vec<_>, _>>()?;
    let def = QuestDef {
        schema: 1,
        key: format!("{}/{id}", pack.manifest().id()),
        title: source.title,
        introduction: source.introduction,
        destination: source.destination,
        checkpoints,
        completion: source.completion,
        prerequisite: source.prerequisite,
        choice: source.choice,
    };
    def.check()?;
    Ok(def)
}

struct QuestApplier {
    kind: CanonicalId,
}

impl QuestApplier {
    fn bind(
        &self,
        ctx: &mut Ctx<'_>,
        pack: &Pack,
        id: &CanonicalId,
        entry: &RawEntry,
        placed: &BTreeMap<CanonicalId, ObjId>,
        existing: Option<ObjId>,
    ) -> Result<Option<ObjId>, String> {
        let (npc_entry, def) = definition(ctx.w, pack, id, entry)?;
        let source: SourceQuest = entry.decode().map_err(|error| format!("{id}: {error}"))?;
        let npc = source
            .npc_pack
            .as_ref()
            .map(|dependency| external_object(ctx.w, pack, dependency, &npc_entry, "npc"))
            .transpose()?
            .or_else(|| placed.get(&npc_entry).copied())
            .filter(|npc| {
                ctx.w
                    .get(*npc)
                    .is_some_and(|o| o.kind == ObjKind::Npc && !o.flags.contains(Flags::GOING))
            })
            .ok_or_else(|| format!("{id}: NPC {npc_entry} has not been created"))?;
        if existing.is_some_and(|old| old != npc) {
            return Err(format!("{id}: a quest role cannot move to another NPC"));
        }
        let mut catalog = role_catalog(ctx.w, npc)?;
        merge_definition(&mut catalog, def);
        let attr = catalog_attr(catalog)?;
        // Extend only the source-owned capability list. Preserve this person's
        // personality, affiliation, location, live memories and other services.
        if source.npc_pack.is_some() {
            let profile_attr = quest_profile(ctx.w, npc)?;
            ctx.w
                .set_attr(npc, mush_npc::PROFILE_ATTR, profile_attr)
                .map_err(|e| e.to_string())?;
        }
        ctx.w.set_attr(npc, ATTR, attr).map_err(|e| e.to_string())?;
        // Accepted rows retain their own frozen definitions. Reapplying source
        // changes neither those rows nor the NPC's location, memory or receipts.
        Ok(Some(npc))
    }
}

impl EntryApplier for QuestApplier {
    fn kinds(&self) -> Vec<CanonicalId> {
        vec![self.kind.clone()]
    }
    fn phase(&self) -> u8 {
        mush_npc::content::PHASE + 1
    }
    fn check(
        &self,
        ctx: &Ctx<'_>,
        pack: &Pack,
        id: &CanonicalId,
        entry: &RawEntry,
    ) -> Result<(), String> {
        definition(ctx.w, pack, id, entry).map(|_| ())
    }
    fn create(
        &self,
        ctx: &mut Ctx<'_>,
        pack: &Pack,
        id: &CanonicalId,
        entry: &RawEntry,
        placed: &BTreeMap<CanonicalId, ObjId>,
    ) -> Result<Option<ObjId>, String> {
        self.bind(ctx, pack, id, entry, placed, None)
    }
    fn update(
        &self,
        ctx: &mut Ctx<'_>,
        pack: &Pack,
        id: &CanonicalId,
        entry: &RawEntry,
        existing: Option<ObjId>,
        placed: &BTreeMap<CanonicalId, ObjId>,
    ) -> Result<Option<ObjId>, String> {
        self.bind(ctx, pack, id, entry, placed, existing)
    }
    fn retire_check(
        &self,
        ctx: &Ctx<'_>,
        _pack: &CanonicalId,
        id: &CanonicalId,
        existing: Option<ObjId>,
        _placed: &BTreeMap<CanonicalId, ObjId>,
    ) -> Result<(), String> {
        if existing.is_some_and(|npc| ctx.w.get(npc).is_some()) {
            return Err(format!(
                "{id}: a live quest role cannot be removed through a content pack"
            ));
        }
        Ok(())
    }
    fn retire(
        &self,
        _ctx: &mut Ctx<'_>,
        _pack: &CanonicalId,
        _id: &CanonicalId,
        _existing: Option<ObjId>,
    ) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_content::{Applied, AppliedEntry, ContentState};
    use mush_core::{ClockRef, ClockStamp, DefinitionDigest};

    const ROLE: &str = "quest.leila_second_reading";
    const NPC: &str = "npc.leila_mensah_kovacs";
    const ROOM_PACK: &str = "neighborhood.ceres_freeport";
    const ROOM: &str = "room.c07_exchange";
    fn id(text: &str) -> CanonicalId {
        CanonicalId::new(text).unwrap()
    }

    fn fixture(w: &mut World) -> (Pack, ObjId, ObjId) {
        let room = w.create(ObjKind::Room, "Exchange Hall", GOD, None).unwrap();
        w.set_attr(
            room,
            "atlas:detail:sample_tap",
            Attr::text("Occator water. A cup on a chain.", GOD),
        )
        .unwrap();
        let npc = w.create(ObjKind::Npc, "Leila", GOD, Some(room)).unwrap();
        let mut content = ContentState::default();
        content.latches.set(
            Applied::new(
                id(ROOM_PACK),
                4,
                DefinitionDigest::from_bytes([3; 32]),
                ClockStamp {
                    clock: ClockRef::WorldIc { generation: 1 },
                    at: 0,
                },
                BTreeMap::from([(
                    id(ROOM),
                    AppliedEntry::new(
                        id("room"),
                        DefinitionDigest::from_bytes([4; 32]),
                        Some(room),
                    ),
                )]),
                false,
                None,
            )
            .unwrap(),
        );
        w.module_state_insert(content);
        (
            crate::port::passenger::content::packs().unwrap().remove(0),
            room,
            npc,
        )
    }
    fn read(w: &World, pack: &Pack) -> Result<(CanonicalId, QuestDef), String> {
        definition(w, pack, &id(ROLE), pack.entry(&id(ROLE)).unwrap())
    }

    #[test]
    fn authored_quest_resolves_the_existing_public_detail() {
        let mut w = World::new_minimal();
        let (pack, room, _) = fixture(&mut w);
        let (npc, def) = read(&w, &pack).unwrap();
        assert_eq!(npc, id(NPC));
        assert_eq!(
            def.key,
            "tacsim.passengers.stickney/quest.leila_second_reading"
        );
        assert_eq!(def.title, "A second reading");
        assert_eq!(def.checkpoints.len(), 1);
        assert_eq!(def.checkpoints[0].room, room);
        assert_eq!(def.checkpoints[0].detail, "sample tap");
    }

    #[test]
    fn contact_objectives_resolve_exact_npc_dependencies_without_a_passenger_role() {
        let mut w = World::new_minimal();
        let (_, room, npc) = fixture(&mut w);
        let mut latch = serde_json::to_value(
            w.module_state::<ContentState>()
                .unwrap()
                .latches
                .applied(&id(ROOM_PACK))
                .unwrap(),
        )
        .unwrap();
        latch["entries"]["npc.contact"] = serde_json::to_value(AppliedEntry::new(
            id("npc"),
            DefinitionDigest::from_bytes([5; 32]),
            Some(npc),
        ))
        .unwrap();
        w.module_state_mut::<ContentState>().unwrap().latches =
            mush_content::Latches::load(&[(format!("applied:{ROOM_PACK}").into(), latch)]);
        let mut value: serde_json::Value =
            serde_json::from_str(crate::port::passenger::content::JSON).unwrap();
        value["entries"][ROLE]["def"]["checkpoints"][0]["objective"] = serde_json::json!({
            "kind": "contact", "npc_pack": ROOM_PACK, "npc": "npc.contact"
        });
        value["entries"][ROLE]["def"]["checkpoints"][0]["detail"] = "".into();
        let pack = mush_content::parse_pack(&value.to_string()).unwrap();
        let (_, definition) = read(&w, &pack).unwrap();
        assert_eq!(
            definition.checkpoints[0].objective,
            Objective::Contact { npc }
        );
        assert_eq!(definition.checkpoints[0].room, room);
        for target in ["room.c07_exchange", "npc.missing"] {
            let mut invalid = value.clone();
            invalid["entries"][ROLE]["def"]["checkpoints"][0]["objective"]["npc"] = target.into();
            assert!(read(&w, &mush_content::parse_pack(&invalid.to_string()).unwrap()).is_err());
        }
        value["entries"][ROLE]["def"]["checkpoints"][0]["objective"]["npc_pack"] =
            "undeclared.station".into();
        assert!(read(&w, &mush_content::parse_pack(&value.to_string()).unwrap()).is_err());
    }

    #[test]
    fn survey_commission_source_keeps_board_selection_open() {
        let mut w = World::new_minimal();
        let (_, room, _) = fixture(&mut w);
        let mut value: serde_json::Value =
            serde_json::from_str(crate::port::passenger::content::JSON).unwrap();
        value["entries"][ROLE]["def"]["checkpoints"][0]["objective"] =
            serde_json::json!({"kind": "survey_commission"});
        value["entries"][ROLE]["def"]["checkpoints"][0]["detail"] = "".into();
        let (_, definition) =
            read(&w, &mush_content::parse_pack(&value.to_string()).unwrap()).unwrap();
        assert_eq!(
            definition.checkpoints[0].objective,
            Objective::SurveyCommission
        );
        assert_eq!(definition.checkpoints[0].room, room);
        value["entries"][ROLE]["def"]["checkpoints"][0]["objective"]["offer"] = "ahuna-a".into();
        assert!(read(&w, &mush_content::parse_pack(&value.to_string()).unwrap()).is_err());
    }

    #[test]
    fn paid_passage_source_requires_an_authored_traveller_with_the_exact_route() {
        let mut w = World::new_minimal();
        let (_, npc, _) = multiple_external_fixture(&mut w);
        let mut value: serde_json::Value =
            serde_json::from_str(crate::port::passenger::content::JSON).unwrap();
        value["entries"][ROLE]["def"]["checkpoints"][0]["objective"] = serde_json::json!({
            "kind": "paid_passage", "npc_pack": ROOM_PACK, "npc": NPC,
            "origin": "stickney", "destination": "ceres"
        });
        let parse =
            |value: &serde_json::Value| mush_content::parse_pack(&value.to_string()).unwrap();
        assert!(
            read(&w, &parse(&value)).is_err(),
            "a generic contact is not a passenger"
        );
        let traveller = crate::port::passenger::TravellerDef {
            schema: 1,
            key: "test/passenger.contact".into(),
            origin: "stickney".into(),
            destination: "ceres".into(),
            opening_fare_minor: 100,
            max_fare_minor: 200,
            funding_minor: 1000,
        };
        w.set_attr(
            npc,
            crate::port::passenger::TRAVELLER_ATTR,
            Attr::json(serde_json::json!(traveller), GOD),
        )
        .unwrap();
        let (_, definition) = read(&w, &parse(&value)).unwrap();
        assert_eq!(
            definition.checkpoints[0].objective,
            Objective::PaidPassage {
                npc,
                origin: "stickney".into(),
                destination: "ceres".into()
            }
        );
        value["entries"][ROLE]["def"]["checkpoints"][0]["objective"]["destination"] =
            "vesta".into();
        assert!(read(&w, &parse(&value)).is_err());
    }

    #[test]
    fn checkpoint_requires_a_healthy_exact_dependency_and_public_live_detail() {
        #[derive(Debug)]
        enum Change {
            Old,
            New,
            Retired,
            Anomaly,
            WrongKind,
            MissingRoom,
            Held,
            Private(AttrFlags),
            MissingDetail,
            OversizedDetail,
            ForgedDetail,
        }
        for change in [
            Change::Old,
            Change::New,
            Change::Retired,
            Change::Anomaly,
            Change::WrongKind,
            Change::MissingRoom,
            Change::Held,
            Change::Private(AttrFlags::WIZARD_ONLY),
            Change::Private(AttrFlags::HIDDEN),
            Change::Private(AttrFlags::SELF_ONLY),
            Change::MissingDetail,
            Change::OversizedDetail,
            Change::ForgedDetail,
        ] {
            let mut w = World::new_minimal();
            let (pack, room, _) = fixture(&mut w);
            let content = w.module_state::<ContentState>().unwrap();
            let mut latch = serde_json::to_value(content.latches.all().next().unwrap()).unwrap();
            match change {
                Change::Old => latch["version"] = 3.into(),
                Change::New => latch["version"] = 5.into(),
                Change::Retired => latch["retired"] = true.into(),
                Change::Anomaly => latch["anomaly"] = "incomplete".into(),
                Change::WrongKind => latch["entries"][ROOM]["kind"] = "npc".into(),
                Change::MissingRoom => latch["entries"][ROOM]["object"] = 99999.into(),
                Change::Held => latch["version"] = 0.into(),
                Change::Private(flags) => {
                    let mut attr = Attr::text("Not public.", GOD);
                    attr.flags = flags;
                    w.set_attr(room, "atlas:detail:sample_tap", attr).unwrap();
                }
                Change::MissingDetail => {
                    w.remove_attr(room, "atlas:detail:sample_tap").unwrap();
                }
                Change::OversizedDetail => {
                    w.set_attr(
                        room,
                        "atlas:detail:sample_tap",
                        Attr::text("x".repeat(super::super::model::MAX_DETAIL_BYTES + 1), GOD),
                    )
                    .unwrap();
                }
                Change::ForgedDetail => {
                    w.set_attr(
                        room,
                        "atlas:detail:sample_tap",
                        Attr::text("An unowned claim.", room),
                    )
                    .unwrap();
                }
            }
            w.module_state_mut::<ContentState>().unwrap().latches =
                mush_content::Latches::load(&[(format!("applied:{ROOM_PACK}").into(), latch)]);
            assert!(read(&w, &pack).is_err(), "{change:?}");
        }
    }

    #[test]
    fn malformed_sources_cannot_rebind_or_forge_derived_room_identity() {
        #[derive(Debug)]
        enum Change {
            MissingNpc,
            UndeclaredRoom,
            Digest,
            ForgedRoom,
            ForgedKey,
        }
        for change in [
            Change::MissingNpc,
            Change::UndeclaredRoom,
            Change::Digest,
            Change::ForgedRoom,
            Change::ForgedKey,
        ] {
            let mut w = World::new_minimal();
            fixture(&mut w);
            let mut value: serde_json::Value =
                serde_json::from_str(crate::port::passenger::content::JSON).unwrap();
            match change {
                Change::MissingNpc => value["entries"][ROLE]["def"]["npc"] = "npc.missing".into(),
                Change::UndeclaredRoom => {
                    value["entries"][ROLE]["def"]["checkpoints"][0]["room_pack"] =
                        "another_station".into()
                }
                Change::Digest => {
                    value["manifest"]["requires"][0]["digest"] = "ff".repeat(32).into()
                }
                Change::ForgedRoom => {
                    value["entries"][ROLE]["def"]["checkpoints"][0]["room"] = 1.into()
                }
                Change::ForgedKey => value["entries"][ROLE]["def"]["key"] = "borrowed/quest".into(),
            }
            let pack = mush_content::parse_pack(&value.to_string()).unwrap();
            assert!(read(&w, &pack).is_err(), "{change:?}");
        }
    }

    #[test]
    fn source_update_keeps_identity_location_and_memory_and_refuses_rebinding() {
        let (mut w, reg) = mush_testkit::boot_world();
        let (pack, room, npc) = fixture(&mut w);
        let mut out = mush_core::Outbox::new();
        let mut ctx = Ctx {
            w: &mut w,
            actor: GOD,
            switches: Default::default(),
            args: Default::default(),
            out: &mut out,
            reg: &reg,
            module: Some(mush_content::NS.into()),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };
        let applier = QuestApplier { kind: id("quest") };
        let placed = BTreeMap::from([(id(NPC), npc)]);
        let role = id(ROLE);
        let entry = pack.entry(&role).unwrap();
        assert_eq!(
            applier
                .create(&mut ctx, &pack, &role, entry, &placed)
                .unwrap(),
            Some(npc)
        );
        let attr = ctx.w.attr(npc, ATTR).unwrap();
        assert_eq!(attr.owner, GOD);
        assert!(
            attr.flags
                .contains(AttrFlags::HIDDEN | AttrFlags::WIZARD_ONLY | AttrFlags::NO_INHERIT)
        );
        let elsewhere = ctx.w.create(ObjKind::Room, "Elsewhere", GOD, None).unwrap();
        ctx.w.move_to(npc, elsewhere).unwrap();
        ctx.w
            .set_attr(
                npc,
                mush_npc::SUMMARY_ATTR,
                Attr::text("We met aboard the ship.", GOD),
            )
            .unwrap();
        let mut value: serde_json::Value =
            serde_json::from_str(crate::port::passenger::content::JSON).unwrap();
        value["entries"][ROLE]["def"]["introduction"] =
            "Revised invitation for future acceptances.".into();
        let revised = mush_content::parse_pack(&value.to_string()).unwrap();
        applier
            .update(
                &mut ctx,
                &revised,
                &role,
                revised.entry(&role).unwrap(),
                Some(npc),
                &placed,
            )
            .unwrap();
        assert_eq!(ctx.w.get(npc).unwrap().location, Some(elsewhere));
        assert_eq!(
            ctx.w.attr(npc, mush_npc::SUMMARY_ATTR).unwrap().as_text(),
            Some("We met aboard the ship.")
        );
        let stored: QuestDef =
            serde_json::from_value(ctx.w.attr(npc, ATTR).unwrap().as_json().unwrap().clone())
                .unwrap();
        assert_eq!(
            stored.introduction,
            "Revised invitation for future acceptances."
        );
        assert!(
            applier
                .retire_check(&ctx, pack.manifest().id(), &role, Some(npc), &placed)
                .is_err()
        );
        let stranger = ctx
            .w
            .create(ObjKind::Npc, "Someone else", GOD, Some(room))
            .unwrap();
        assert!(
            applier
                .update(
                    &mut ctx,
                    &pack,
                    &role,
                    entry,
                    Some(npc),
                    &BTreeMap::from([(id(NPC), stranger)])
                )
                .is_err()
        );
        assert!(ctx.w.attr(stranger, ATTR).is_none());
        let mut other_role = stored;
        other_role.key = "another_pack/quest.another".into();
        ctx.w
            .set_attr(
                npc,
                ATTR,
                Attr::json(serde_json::to_value(&other_role).unwrap(), GOD),
            )
            .unwrap();
        applier
            .create(&mut ctx, &pack, &role, entry, &placed)
            .unwrap();
        let catalog = role_catalog(ctx.w, npc).unwrap();
        assert_eq!(catalog.len(), 2);
        assert_eq!(
            catalog
                .iter()
                .find(|definition| definition.key == other_role.key),
            Some(&other_role)
        );
    }

    #[test]
    fn external_giver_keeps_staff_profile_and_only_adds_quest_capabilities() {
        let mut w = World::new_minimal();
        let staff = w.create(ObjKind::Player, "Staff", GOD, None).unwrap();
        let npc = w.create(ObjKind::Npc, "Existing giver", GOD, None).unwrap();
        let profile = mush_npc::AgentProfile {
            persona: "A patient clerk with a dry sense of humor.".into(),
            background: "A history written by staff.".into(),
            goals: vec!["Keep the queue moving.".into()],
            public_faction: Some(mush_faction::FactionId(42)),
            interfaces: vec!["help".into(), "coop".into()],
            command_refs: vec!["+coop".into()],
            fallback_lines: vec!["Please take a number.".into()],
            ..Default::default()
        };
        let mut original = Attr::json(serde_json::to_value(&profile).unwrap(), staff);
        original.flags = AttrFlags::WIZARD_ONLY | AttrFlags::NO_INHERIT;
        w.set_attr(npc, mush_npc::PROFILE_ATTR, original.clone())
            .unwrap();

        let merged = quest_profile(&w, npc).unwrap();
        assert_eq!(merged.owner, staff);
        assert_eq!(merged.flags, original.flags);
        let actual: mush_npc::AgentProfile =
            serde_json::from_value(merged.as_json().unwrap().clone()).unwrap();
        let mut expected = profile;
        expected.interfaces.push("quest".into());
        expected.command_refs.push("+quest".into());
        assert_eq!(actual, expected);
        assert_eq!(w.attr(npc, mush_npc::PROFILE_ATTR).unwrap(), &original);

        w.set_attr(npc, mush_npc::PROFILE_ATTR, merged.clone())
            .unwrap();
        assert_eq!(quest_profile(&w, npc).unwrap(), merged);
    }

    #[test]
    fn external_giver_capability_capacity_fails_without_mutating_profile() {
        for interfaces_full in [true, false] {
            let mut w = World::new_minimal();
            let npc = w.create(ObjKind::Npc, "Existing giver", GOD, None).unwrap();
            let mut profile = mush_npc::AgentProfile::default();
            if interfaces_full {
                profile.interfaces = (0..mush_npc::MAX_PROFILE_INTERFACES)
                    .map(|n| format!("service{n}").into())
                    .collect();
            } else {
                profile.command_refs = (0..mush_npc::MAX_PROFILE_COMMAND_REFS)
                    .map(|n| format!("+command{n}").into())
                    .collect();
            }
            profile.validate().unwrap();
            let original = Attr::json(serde_json::to_value(profile).unwrap(), GOD);
            w.set_attr(npc, mush_npc::PROFILE_ATTR, original.clone())
                .unwrap();
            let error = quest_profile(&w, npc).unwrap_err();
            assert!(error.contains(if interfaces_full {
                "too many interfaces"
            } else {
                "too many command references"
            }));
            assert_eq!(w.attr(npc, mush_npc::PROFILE_ATTR).unwrap(), &original);
        }
    }

    #[test]
    fn external_role_conflict_refuses_the_whole_pack_before_any_profile_write() {
        #[derive(Debug)]
        enum Conflict {
            DuplicateKeys,
            WrongOwner,
            NotJson,
            InvalidDefinition,
        }
        for conflict in [
            Conflict::DuplicateKeys,
            Conflict::WrongOwner,
            Conflict::NotJson,
            Conflict::InvalidDefinition,
        ] {
            let (mut w, reg) = mush_testkit::boot_world();
            let (original_pack, room, first) = fixture(&mut w);
            let (_, mut previous_def) = read(&w, &original_pack).unwrap();
            let second = w
                .create(ObjKind::Npc, "Another giver", GOD, Some(room))
                .unwrap();
            let profile = Attr::json(
                serde_json::to_value(mush_npc::AgentProfile::default()).unwrap(),
                GOD,
            );
            for npc in [first, second] {
                w.set_attr(npc, mush_npc::PROFILE_ATTR, profile.clone())
                    .unwrap();
            }
            let mut latch = serde_json::to_value(
                w.module_state::<ContentState>()
                    .unwrap()
                    .latches
                    .applied(&id(ROOM_PACK))
                    .unwrap(),
            )
            .unwrap();
            for (entry, npc) in [("npc.first", first), ("npc.second", second)] {
                latch["entries"][entry] = serde_json::to_value(AppliedEntry::new(
                    id("npc"),
                    DefinitionDigest::from_bytes([5; 32]),
                    Some(npc),
                ))
                .unwrap();
            }
            w.module_state_mut::<ContentState>().unwrap().latches =
                mush_content::Latches::load(&[(format!("applied:{ROOM_PACK}").into(), latch)]);

            let mut source: serde_json::Value =
                serde_json::from_str(crate::port::passenger::content::JSON).unwrap();
            source["manifest"]["id"] = "test.external_quests".into();
            source["manifest"]["requires"] = serde_json::json!([{ "id": ROOM_PACK, "version": 4 }]);
            let mut first_entry = source["entries"][ROLE].clone();
            first_entry["def"]["npc_pack"] = ROOM_PACK.into();
            first_entry["def"]["npc"] = "npc.first".into();
            let mut second_entry = first_entry.clone();
            second_entry["def"]["npc"] = "npc.second".into();
            source["entries"] = serde_json::json!({
                "quest.a_first": first_entry,
                "quest.z_conflict": second_entry,
            });
            let pack = mush_content::parse_pack(&source.to_string()).unwrap();
            previous_def.key = "test.external_quests/quest.z_conflict".into();
            let previous = match conflict {
                Conflict::DuplicateKeys => {
                    Attr::json(serde_json::json!([previous_def, previous_def]), GOD)
                }
                Conflict::WrongOwner => {
                    Attr::json(serde_json::to_value(previous_def).unwrap(), second)
                }
                Conflict::NotJson => Attr::text("not a quest definition", GOD),
                Conflict::InvalidDefinition => {
                    previous_def.checkpoints.clear();
                    Attr::json(serde_json::to_value(previous_def).unwrap(), GOD)
                }
            };
            w.set_attr(second, ATTR, previous.clone()).unwrap();
            let mut latches = w.module_state::<ContentState>().unwrap().latches.clone();
            let mut appliers = Appliers::new();
            register_appliers(&mut appliers).unwrap();
            let mut out = mush_core::Outbox::new();
            let mut ctx = Ctx {
                w: &mut w,
                actor: GOD,
                switches: Default::default(),
                args: Default::default(),
                out: &mut out,
                reg: &reg,
                module: Some(mush_content::NS.into()),
                typed_state: mush_core::ModuleActionState::unavailable(),
            };
            let first_id = id("quest.a_first");
            QuestApplier { kind: id("quest") }
                .check(&ctx, &pack, &first_id, pack.entry(&first_id).unwrap())
                .unwrap();
            let result = mush_content::apply(&mut ctx, &mut latches, &appliers, &pack);
            assert!(
                matches!(&result, Err(mush_content::ApplyError::EntryRefused { entry, .. })
                if entry == "quest.z_conflict"),
                "{conflict:?}: {result:?}"
            );
            assert!(latches.applied(pack.manifest().id()).is_none());
            assert!(ctx.w.attr(first, ATTR).is_none());
            assert_eq!(ctx.w.attr(second, ATTR).unwrap(), &previous);
            for npc in [first, second] {
                assert_eq!(ctx.w.attr(npc, mush_npc::PROFILE_ATTR).unwrap(), &profile);
            }
        }
    }

    fn multiple_external_fixture(w: &mut World) -> (Pack, ObjId, QuestDef) {
        let (original, _, npc) = fixture(w);
        let (_, legacy) = read(w, &original).unwrap();
        w.set_attr(
            npc,
            mush_npc::PROFILE_ATTR,
            Attr::json(
                serde_json::to_value(mush_npc::AgentProfile::default()).unwrap(),
                GOD,
            ),
        )
        .unwrap();
        let mut latch = serde_json::to_value(
            w.module_state::<ContentState>()
                .unwrap()
                .latches
                .applied(&id(ROOM_PACK))
                .unwrap(),
        )
        .unwrap();
        latch["entries"][NPC] = serde_json::to_value(AppliedEntry::new(
            id("npc"),
            DefinitionDigest::from_bytes([5; 32]),
            Some(npc),
        ))
        .unwrap();
        w.module_state_mut::<ContentState>().unwrap().latches =
            mush_content::Latches::load(&[(format!("applied:{ROOM_PACK}").into(), latch)]);
        let mut source: serde_json::Value =
            serde_json::from_str(crate::port::passenger::content::JSON).unwrap();
        source["manifest"]["id"] = "test.more_quests".into();
        source["manifest"]["requires"] = serde_json::json!([{ "id": ROOM_PACK, "version": 4 }]);
        let mut entry = source["entries"][ROLE].clone();
        entry["def"]["npc_pack"] = ROOM_PACK.into();
        source["entries"] = serde_json::json!({ "quest.a_first": entry, "quest.b_second": entry });
        (
            mush_content::parse_pack(&source.to_string()).unwrap(),
            npc,
            legacy,
        )
    }

    #[test]
    fn multiple_sources_preserve_legacy_definition_and_update_only_their_own_key() {
        let (mut w, reg) = mush_testkit::boot_world();
        let (pack, npc, legacy) = multiple_external_fixture(&mut w);
        // An old save still carries its single object, without a migration.
        w.set_attr(
            npc,
            ATTR,
            Attr::json(serde_json::to_value(&legacy).unwrap(), GOD),
        )
        .unwrap();
        w.set_attr(
            npc,
            mush_npc::SUMMARY_ATTR,
            Attr::text("Our completed passage.", GOD),
        )
        .unwrap();
        assert_eq!(role_catalog(&w, npc).unwrap(), vec![legacy.clone()]);
        let location = w.get(npc).unwrap().location;
        let mut out = mush_core::Outbox::new();
        let mut ctx = Ctx {
            w: &mut w,
            actor: GOD,
            switches: Default::default(),
            args: Default::default(),
            out: &mut out,
            reg: &reg,
            module: Some(mush_content::NS.into()),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };
        let applier = QuestApplier { kind: id("quest") };
        for (entry_id, entry) in pack.entries() {
            applier.check(&ctx, &pack, entry_id, entry).unwrap();
            applier
                .create(&mut ctx, &pack, entry_id, entry, &BTreeMap::new())
                .unwrap();
        }
        let before = role_catalog(ctx.w, npc).unwrap();
        assert_eq!(before.len(), 3);
        assert!(before.contains(&legacy));
        assert!(before.windows(2).all(|pair| pair[0].key < pair[1].key));
        let first = id("quest.a_first");
        let mut updated = pack.entry(&first).unwrap().def_value();
        updated["introduction"] = "An updated invitation for later acceptances.".into();
        let updated = RawEntry::new(id("quest"), updated).unwrap();
        applier
            .update(
                &mut ctx,
                &pack,
                &first,
                &updated,
                Some(npc),
                &BTreeMap::new(),
            )
            .unwrap();
        let after = role_catalog(ctx.w, npc).unwrap();
        assert_eq!(after.len(), 3);
        for previous in &before {
            let current = after.iter().find(|def| def.key == previous.key).unwrap();
            if previous.key == "test.more_quests/quest.a_first" {
                assert_eq!(
                    current.introduction,
                    "An updated invitation for later acceptances."
                );
            } else {
                assert_eq!(current, previous);
            }
        }
        assert_eq!(ctx.w.get(npc).unwrap().location, location);
        assert_eq!(
            ctx.w.attr(npc, mush_npc::SUMMARY_ATTR).unwrap().as_text(),
            Some("Our completed passage.")
        );
    }

    #[test]
    fn whole_pack_capacity_counts_all_new_definitions_for_one_giver() {
        let (mut w, reg) = mush_testkit::boot_world();
        let (pack, npc, legacy) = multiple_external_fixture(&mut w);
        let existing: Vec<_> = (0..super::super::model::MAX_QUESTS_PER_NPC - 1)
            .map(|n| {
                let mut def = legacy.clone();
                def.key = format!("other_pack/quest.existing{n}");
                def
            })
            .collect();
        let original = catalog_attr(existing).unwrap();
        w.set_attr(npc, ATTR, original.clone()).unwrap();
        let profile = w.attr(npc, mush_npc::PROFILE_ATTR).unwrap().clone();
        let mut latches = w.module_state::<ContentState>().unwrap().latches.clone();
        let mut appliers = Appliers::new();
        register_appliers(&mut appliers).unwrap();
        let mut out = mush_core::Outbox::new();
        let mut ctx = Ctx {
            w: &mut w,
            actor: GOD,
            switches: Default::default(),
            args: Default::default(),
            out: &mut out,
            reg: &reg,
            module: Some(mush_content::NS.into()),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };
        let result = mush_content::apply(&mut ctx, &mut latches, &appliers, &pack);
        assert!(
            matches!(result, Err(mush_content::ApplyError::EntryRefused { .. })),
            "{result:?}"
        );
        assert!(latches.applied(pack.manifest().id()).is_none());
        assert_eq!(ctx.w.attr(npc, ATTR).unwrap(), &original);
        assert_eq!(ctx.w.attr(npc, mush_npc::PROFILE_ATTR).unwrap(), &profile);
    }
}
