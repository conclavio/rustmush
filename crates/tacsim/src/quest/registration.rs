//! Keep definition construction outside the large port registration frame.
use super::{
    model::{QuestIndex, QuestIndexKey, QuestKey, QuestRecord},
    store,
};
use crate::port::families::{PortFamilies, family_descriptor};
use mush_core::{
    DefinitionDescriptor, DefinitionId, DefinitionRef, ModError, Registrar, StateFamilyHandle,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

type Families = (
    StateFamilyHandle<QuestKey, QuestRecord>,
    StateFamilyHandle<QuestIndexKey, QuestIndex>,
);
pub(crate) fn families(reg: &mut Registrar<'_>) -> Result<Families, ModError> {
    let descriptor = family_descriptor(store::FAMILY, "tacsim.quest_key.v1", "tacsim.quest")?
        .with_limits(256, store::MAX_RECORD_BYTES)
        .map_err(|e| ModError::msg(e.to_string()))?;
    let records = reg.state_family::<QuestKey, QuestRecord, _, _, _, _>(
        descriptor,
        |key: &QuestKey| {
            key.check()?;
            Ok(key.key_text())
        },
        |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
        |value: &QuestRecord| value.check(),
        |value: &QuestRecord| serde_json::to_value(value).map_err(|e| e.to_string()),
    )?;
    let descriptor = family_descriptor(
        store::INDEX_FAMILY,
        "tacsim.quest_index_key.v1",
        "tacsim.quest_index",
    )?
    .with_limits(256, store::MAX_INDEX_BYTES)
    .map_err(|e| ModError::msg(e.to_string()))?;
    let index = reg.state_family::<QuestIndexKey, QuestIndex, _, _, _, _>(
        descriptor,
        QuestIndexKey::key_text,
        |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
        |value: &QuestIndex| value.check(),
        |value: &QuestIndex| serde_json::to_value(value).map_err(|e| e.to_string()),
    )?;
    Ok((records, index))
}

pub(crate) fn action(
    outcomes: &BTreeMap<String, DefinitionRef>,
) -> Result<DefinitionDescriptor, ModError> {
    let outcome = outcomes
        .get(store::OUTCOME)
        .cloned()
        .ok_or_else(|| ModError::msg("quest outcome missing"))?;
    DefinitionDescriptor::new(
        mush_core::DefinitionKind::new("action").map_err(|e| ModError::msg(e.to_string()))?,
        DefinitionId::new(store::ACTION).map_err(|e| ModError::msg(e.to_string()))?,
        1,
        json!({"subject":"self_actor","targets":0,"family":store::FAMILY}),
    )
    .map(|d| d.as_current().with_references(vec![outcome]))
    .map_err(|e| ModError::msg(e.to_string()))
}

pub(crate) fn provider(
    reg: &mut Registrar<'_>,
    actions: &BTreeMap<String, DefinitionRef>,
    outcomes: &BTreeMap<String, DefinitionRef>,
    families: &PortFamilies,
) -> Result<Option<DefinitionRef>, ModError> {
    let action = actions.get(store::ACTION).cloned();
    if let Some(action) = &action {
        let outcome = outcomes
            .get(store::OUTCOME)
            .cloned()
            .ok_or_else(|| ModError::msg("quest outcome missing"))?;
        reg.action_provider(
            action.clone(),
            store::Provider {
                action: action.clone(),
                outcome,
                records: families.quests.clone(),
                index: families.quest_index.clone(),
                passengers: families.passengers.clone(),
                passenger_index: families.passenger_index.clone(),
                survey: families.quest_survey_evidence.clone(),
                commissions: families.surveys.clone(),
            },
        )?;
    }
    Ok(action)
}
