#![cfg(test)]

use super::*;

const ORGANIZER: ObjId = ObjId(10);
const PILOT: ObjId = ObjId(11);
const OTHER: ObjId = ObjId(12);
const CARRIER: ObjId = ObjId(20);
const UNIT: ObjId = ObjId(30);

fn slot(number: u16, unit: Option<ObjId>, controller: PlannedController) -> SlotPlan {
    SlotPlan {
        slot: Slot::new(number).expect("slot"),
        unit,
        controller,
        role: None,
        mind: None,
    }
}
fn plan() -> Plan {
    Plan {
        carrier: CARRIER,
        slots: vec![slot(
            1,
            Some(UNIT),
            PlannedController::Player { character: PILOT },
        )],
        npc_commander: None,
        financial_terms: None,
    }
}
fn book_with_draft() -> (PlanningBook, PlanId) {
    let change = PlanningBook::new(ORGANIZER)
        .expect("book")
        .apply(ORGANIZER, &PlanningOperation::CreateDraft { plan: plan() })
        .expect("draft");
    (change.book, change.affected)
}
fn declare(
    book: &PlanningBook,
    id: PlanId,
    actor: ObjId,
    state: ConsentState,
) -> Result<PlanningChange, PlanningError> {
    let draft = book.drafts.get(&id).expect("draft");
    book.apply(
        actor,
        &PlanningOperation::Declare {
            id,
            expected_revision: draft.revision,
            expected_hash: draft.plan_hash.clone(),
            state,
        },
    )
}
fn ready(book: PlanningBook, id: PlanId) -> PlanningBook {
    let accepted = declare(&book, id, PILOT, ConsentState::Accepted)
        .expect("accept")
        .book;
    declare(&accepted, id, PILOT, ConsentState::Ready)
        .expect("ready")
        .book
}

#[test]
fn presets_are_private_and_other_actors_can_only_declare_their_own_player_assignment() {
    let empty = PlanningBook::new(ORGANIZER).expect("book");
    let create = PlanningOperation::CreatePreset {
        name: "Patrol".into(),
        plan: plan(),
    };
    assert_eq!(empty.apply(OTHER, &create), Err(PlanningError::Forbidden));
    let preset = empty.apply(ORGANIZER, &create).expect("preset");
    assert_eq!(
        preset.book.apply(
            OTHER,
            &PlanningOperation::LoadPreset {
                id: preset.affected,
                expected_revision: 1
            }
        ),
        Err(PlanningError::Forbidden)
    );
    let (book, id) = book_with_draft();
    assert!(book.drafts[&id].visible_to(ORGANIZER));
    assert!(book.drafts[&id].visible_to(PILOT));
    assert!(!book.drafts[&id].visible_to(OTHER));
    assert_eq!(
        declare(&book, id, OTHER, ConsentState::Accepted),
        Err(PlanningError::Forbidden)
    );
    assert_eq!(
        declare(&book, id, ORGANIZER, ConsentState::Accepted),
        Err(PlanningError::Forbidden)
    );
    assert!(declare(&book, id, PILOT, ConsentState::Accepted).is_ok());
    assert_eq!(
        book.apply(
            PILOT,
            &PlanningOperation::DiscardDraft {
                id,
                expected_revision: 1
            }
        ),
        Err(PlanningError::Forbidden)
    );
    assert_eq!(
        book.apply(
            PILOT,
            &PlanningOperation::SaveDraftAsPreset {
                draft: id,
                expected_revision: 1,
                name: "Not mine".into()
            }
        ),
        Err(PlanningError::Forbidden)
    );
}

#[test]
fn multiple_plans_can_reference_the_same_unit_without_reserving_it() {
    let mut book = PlanningBook::new(ORGANIZER).expect("book");
    for name in ["Patrol", "Recovery"] {
        book = book
            .apply(
                ORGANIZER,
                &PlanningOperation::CreatePreset {
                    name: name.into(),
                    plan: plan(),
                },
            )
            .expect("same unit")
            .book;
    }
    for id in [PlanId(1), PlanId(2)] {
        book = book
            .apply(
                ORGANIZER,
                &PlanningOperation::LoadPreset {
                    id,
                    expected_revision: 1,
                },
            )
            .expect("load")
            .book;
    }
    assert_eq!(book.presets.len(), 2);
    assert_eq!(book.drafts.len(), 2);
    assert!(
        book.drafts
            .values()
            .all(|draft| draft.plan.slots[0].unit == Some(UNIT) && draft.consents.is_empty())
    );
    assert!(
        PlanningBook::new(OTHER)
            .expect("other book")
            .apply(OTHER, &PlanningOperation::CreateDraft { plan: plan() })
            .is_ok()
    );
}

#[test]
fn load_copy_and_explicit_save_are_independent_snapshots() {
    let first = PlanningBook::new(ORGANIZER)
        .expect("book")
        .apply(
            ORGANIZER,
            &PlanningOperation::CreatePreset {
                name: "Original".into(),
                plan: plan(),
            },
        )
        .expect("preset");
    let original_id = first.affected;
    let copied = first
        .book
        .apply(
            ORGANIZER,
            &PlanningOperation::CopyPreset {
                id: original_id,
                expected_revision: 1,
                name: "Copy".into(),
            },
        )
        .expect("copy");
    let copied_id = copied.affected;
    let loaded = copied
        .book
        .apply(
            ORGANIZER,
            &PlanningOperation::LoadPreset {
                id: original_id,
                expected_revision: 1,
            },
        )
        .expect("load");
    let draft_id = loaded.affected;
    let mut revised = plan();
    revised.slots[0].unit = Some(ObjId(31));
    let edited = loaded
        .book
        .apply(
            ORGANIZER,
            &PlanningOperation::EditDraft {
                id: draft_id,
                expected_revision: 1,
                plan: revised.clone(),
            },
        )
        .expect("edit");
    assert_eq!(edited.book.presets[&original_id].plan, plan());
    assert_eq!(edited.book.presets[&copied_id].plan, plan());
    let saved = edited
        .book
        .apply(
            ORGANIZER,
            &PlanningOperation::SaveDraftAsPreset {
                draft: draft_id,
                expected_revision: 2,
                name: "Edited mission".into(),
            },
        )
        .expect("save");
    assert_eq!(saved.book.presets[&saved.affected].plan, revised);
    assert_eq!(saved.book.presets[&original_id].plan, plan());
    let updated = saved
        .book
        .apply(
            ORGANIZER,
            &PlanningOperation::UpdatePreset {
                id: original_id,
                expected_revision: 1,
                name: "New original".into(),
                plan: Plan::empty(CARRIER),
            },
        )
        .expect("update");
    assert_eq!(updated.book.drafts[&draft_id].plan, revised);
    assert_eq!(updated.book.presets[&copied_id].plan, plan());
}

#[test]
fn ready_requires_self_acceptance_and_does_not_change_material_revision() {
    let (book, id) = book_with_draft();
    assert_eq!(
        declare(&book, id, PILOT, ConsentState::Ready),
        Err(PlanningError::AcceptanceRequired)
    );
    let ready = ready(book.clone(), id);
    assert_eq!(ready.drafts[&id].revision, book.drafts[&id].revision);
    assert_eq!(ready.drafts[&id].plan_hash, book.drafts[&id].plan_hash);
    assert_eq!(ready.revision, book.revision + 2);
    assert_eq!(
        ready.drafts[&id].consents[&PILOT].state,
        ConsentState::Ready
    );
    assert_eq!(
        declare(&ready, id, PILOT, ConsentState::Ready),
        Err(PlanningError::NoChange)
    );
    let declined = declare(&ready, id, PILOT, ConsentState::Declined)
        .expect("decline")
        .book;
    assert_eq!(
        declare(&declined, id, PILOT, ConsentState::Ready),
        Err(PlanningError::AcceptanceRequired)
    );
}

#[test]
fn every_material_edit_invalidates_prior_acceptance_and_readiness() {
    let (book, id) = book_with_draft();
    let book = ready(book, id);
    for edit in 0..8 {
        let mut changed = plan();
        match edit {
            0 => changed.carrier = ObjId(21),
            1 => changed.npc_commander = Some(ObjId(40)),
            2 => changed.financial_terms = Some(PlanHash::new("a".repeat(64)).expect("hash")),
            3 => changed.slots[0].unit = Some(ObjId(31)),
            4 => changed.slots[0].controller = PlannedController::Player { character: OTHER },
            5 => changed.slots[0].slot = Slot::new(2).expect("slot"),
            6 => changed.slots[0].role = Some("Scout".into()),
            _ => {
                changed.slots[0].mind = Some(MindChoice {
                    licence: 2,
                    profiles: BTreeMap::from([("posture".into(), "defensive".into())]),
                })
            }
        }
        let updated = book
            .apply(
                ORGANIZER,
                &PlanningOperation::EditDraft {
                    id,
                    expected_revision: 1,
                    plan: changed,
                },
            )
            .expect("edit")
            .book;
        assert!(updated.drafts[&id].consents.is_empty(), "edit {edit}");
        assert_eq!(updated.drafts[&id].revision, 2);
        assert_ne!(updated.drafts[&id].plan_hash, book.drafts[&id].plan_hash);
    }
    assert_eq!(
        book.apply(
            ORGANIZER,
            &PlanningOperation::EditDraft {
                id,
                expected_revision: 1,
                plan: plan()
            }
        ),
        Err(PlanningError::NoChange)
    );
    assert_eq!(book.drafts[&id].consents[&PILOT].state, ConsentState::Ready);
}

#[test]
fn stale_revision_or_hash_cannot_reconfirm_a_changed_assignment() {
    let (book, id) = book_with_draft();
    let old_hash = book.drafts[&id].plan_hash.clone();
    let mut changed = plan();
    changed.slots[0].unit = Some(ObjId(31));
    let updated = book
        .apply(
            ORGANIZER,
            &PlanningOperation::EditDraft {
                id,
                expected_revision: 1,
                plan: changed,
            },
        )
        .expect("edit")
        .book;
    for revision in [1, 2] {
        assert_eq!(
            updated.apply(
                PILOT,
                &PlanningOperation::Declare {
                    id,
                    expected_revision: revision,
                    expected_hash: old_hash.clone(),
                    state: ConsentState::Accepted
                }
            ),
            Err(PlanningError::Stale)
        );
    }
    assert_eq!(
        updated.apply(
            ORGANIZER,
            &PlanningOperation::EditDraft {
                id,
                expected_revision: 1,
                plan: plan()
            }
        ),
        Err(PlanningError::Stale)
    );
    assert_eq!(
        updated.apply(
            ORGANIZER,
            &PlanningOperation::SaveDraftAsPreset {
                draft: id,
                expected_revision: 1,
                name: "Stale".into()
            }
        ),
        Err(PlanningError::Stale)
    );
}

#[test]
fn incomplete_preferences_do_not_invent_an_operator() {
    let empty = PlanningBook::new(ORGANIZER)
        .expect("book")
        .apply(
            ORGANIZER,
            &PlanningOperation::CreateDraft {
                plan: Plan::empty(CARRIER),
            },
        )
        .expect("empty draft");
    let mut unresolved = plan();
    unresolved.slots[0].unit = None;
    unresolved
        .slots
        .push(slot(2, Some(ObjId(999_999)), PlannedController::Unassigned));
    let edited = empty
        .book
        .apply(
            ORGANIZER,
            &PlanningOperation::EditDraft {
                id: empty.affected,
                expected_revision: 1,
                plan: unresolved,
            },
        )
        .expect("unresolved references");
    assert_eq!(
        declare(&edited.book, edited.affected, PILOT, ConsentState::Accepted),
        Err(PlanningError::UnresolvedAssignment)
    );
    assert!(declare(&edited.book, edited.affected, PILOT, ConsentState::Declined).is_ok());
    let mut npc = plan();
    npc.slots[0].controller = PlannedController::Npc { character: PILOT };
    let updated = edited
        .book
        .apply(
            ORGANIZER,
            &PlanningOperation::EditDraft {
                id: edited.affected,
                expected_revision: 2,
                plan: npc,
            },
        )
        .expect("NPC preference");
    assert_eq!(
        declare(
            &updated.book,
            updated.affected,
            PILOT,
            ConsentState::Accepted
        ),
        Err(PlanningError::Forbidden)
    );
}

#[test]
fn material_hash_is_canonical_and_excludes_record_identity_and_declarations() {
    let mut left = plan();
    left.slots[0].mind = Some(MindChoice {
        licence: 4,
        profiles: [("z".into(), "last".into()), ("a".into(), "first".into())]
            .into_iter()
            .collect(),
    });
    let mut right = plan();
    right.slots[0].mind = Some(MindChoice {
        licence: 4,
        profiles: [("a".into(), "first".into()), ("z".into(), "last".into())]
            .into_iter()
            .collect(),
    });
    assert_eq!(left.material_hash(), right.material_hash());
    let decoded: Plan =
        serde_json::from_str(&serde_json::to_string(&left).expect("encode")).expect("decode");
    assert_eq!(decoded.material_hash(), left.material_hash());
    let (book, id) = book_with_draft();
    let original_hash = book.drafts[&id].plan_hash.clone();
    assert_eq!(ready(book, id).drafts[&id].plan_hash, original_hash);
}

#[test]
fn decode_rejects_missing_nullable_fields_unknown_data_duplicate_keys_and_bad_hashes() {
    let value = serde_json::to_value(plan()).expect("encode");
    for key in ["npc_commander", "financial_terms"] {
        let mut broken = value.clone();
        broken.as_object_mut().expect("object").remove(key);
        assert!(
            serde_json::from_value::<Plan>(broken).is_err(),
            "missing {key}"
        );
    }
    for key in ["unit", "role", "mind"] {
        let mut broken = value.clone();
        broken["slots"][0]
            .as_object_mut()
            .expect("slot")
            .remove(key);
        assert!(
            serde_json::from_value::<Plan>(broken).is_err(),
            "missing {key}"
        );
    }
    let mut broken = value.clone();
    broken["slots"][0]["controller"]["unexpected"] = true.into();
    assert!(serde_json::from_value::<Plan>(broken).is_err());
    assert!(
        serde_json::from_str::<MindChoice>(
            r#"{"licence":1,"profiles":{"key":"first","key":"second"}}"#
        )
        .is_err()
    );
    for text in ["a".repeat(63), "A".repeat(64), "g".repeat(64)] {
        assert!(serde_json::from_value::<PlanHash>(serde_json::json!(text)).is_err());
    }
    let (book, id) = book_with_draft();
    let mut value = serde_json::to_value(book).expect("book");
    value["drafts"][id.0.to_string()]["plan_hash"] = "0".repeat(64).into();
    assert!(serde_json::from_value::<PlanningBook>(value).is_err());
}

#[test]
fn strict_slot_and_book_bounds_reject_conflicting_or_oversized_records() {
    let mut duplicate_unit = plan();
    duplicate_unit
        .slots
        .push(slot(2, Some(UNIT), PlannedController::Automated));
    assert!(duplicate_unit.validate().is_err());
    let mut duplicate_person = plan();
    duplicate_person.slots.push(slot(
        2,
        Some(ObjId(31)),
        PlannedController::Player { character: PILOT },
    ));
    assert!(duplicate_person.validate().is_err());
    let mut outside = plan();
    outside.slots[0].slot = Slot::new(7).expect("nonzero");
    assert!(outside.validate().is_err());
    let mut book = PlanningBook::new(ORGANIZER).expect("book");
    for n in 0..MAX_PRESETS {
        book = book
            .apply(
                ORGANIZER,
                &PlanningOperation::CreatePreset {
                    name: format!("Plan {n}"),
                    plan: plan(),
                },
            )
            .expect("fits")
            .book;
    }
    let original = book.clone();
    assert_eq!(
        book.apply(
            ORGANIZER,
            &PlanningOperation::CreatePreset {
                name: "Ninth".into(),
                plan: plan()
            }
        ),
        Err(PlanningError::Capacity)
    );
    assert_eq!(book, original);
    for _ in 0..MAX_DRAFTS {
        book = book
            .apply(ORGANIZER, &PlanningOperation::CreateDraft { plan: plan() })
            .expect("fits")
            .book;
    }
    assert_eq!(
        book.apply(ORGANIZER, &PlanningOperation::CreateDraft { plan: plan() }),
        Err(PlanningError::Capacity)
    );
    assert_eq!(
        serde_json::from_slice::<PlanningBook>(&serde_json::to_vec(&book).expect("encode"))
            .expect("roundtrip"),
        book
    );
}

#[test]
fn deleted_ids_are_not_reused_and_counter_overflow_is_transactional() {
    let (book, id) = book_with_draft();
    let removed = book
        .apply(
            ORGANIZER,
            &PlanningOperation::DiscardDraft {
                id,
                expected_revision: 1,
            },
        )
        .expect("discard")
        .book;
    let next = removed
        .apply(ORGANIZER, &PlanningOperation::CreateDraft { plan: plan() })
        .expect("new");
    assert!(next.affected > id);
    for mode in 0..2 {
        let mut exhausted = PlanningBook::new(ORGANIZER).expect("book");
        if mode == 0 {
            exhausted.next_id = u64::MAX;
        } else {
            exhausted.revision = u64::MAX;
        }
        let before = exhausted.clone();
        assert_eq!(
            exhausted.apply(ORGANIZER, &PlanningOperation::CreateDraft { plan: plan() }),
            Err(PlanningError::Capacity)
        );
        assert_eq!(exhausted, before);
    }
    let mut exhausted = book;
    exhausted.drafts.get_mut(&id).expect("draft").revision = u64::MAX;
    let mut changed = plan();
    changed.carrier = ObjId(21);
    assert_eq!(
        exhausted.apply(
            ORGANIZER,
            &PlanningOperation::EditDraft {
                id,
                expected_revision: u64::MAX,
                plan: changed
            }
        ),
        Err(PlanningError::Capacity)
    );
}

#[test]
fn total_book_byte_capacity_refuses_before_returning_a_proposed_change() {
    let mut large = Plan::empty(CARRIER);
    for number in 1..=6 {
        let mut selection = slot(
            number,
            Some(ObjId(100 + u32::from(number))),
            PlannedController::Automated,
        );
        selection.mind = Some(MindChoice {
            licence: 1,
            profiles: (0..8)
                .map(|n| (format!("{n}{}", "k".repeat(63)), "v".repeat(128)))
                .collect(),
        });
        selection.role = Some("r".repeat(64));
        large.slots.push(selection);
    }
    large.validate().expect("one plan fits");
    let mut book = PlanningBook::new(ORGANIZER).expect("book");
    for n in 0..MAX_PRESETS {
        book = book
            .apply(
                ORGANIZER,
                &PlanningOperation::CreatePreset {
                    name: format!("Large {n}"),
                    plan: large.clone(),
                },
            )
            .expect("presets fit")
            .book;
    }
    let mut byte_refusal = false;
    for _ in 0..MAX_DRAFTS {
        match book.apply(
            ORGANIZER,
            &PlanningOperation::CreateDraft {
                plan: large.clone(),
            },
        ) {
            Ok(change) => book = change.book,
            Err(PlanningError::Capacity) => {
                byte_refusal = true;
                break;
            }
            Err(error) => panic!("unexpected refusal: {error}"),
        }
    }
    assert!(byte_refusal);
    assert!(book.drafts.len() < MAX_DRAFTS);
    assert!(serde_json::to_vec(&book).expect("encode").len() <= MAX_PLANNING_BOOK_BYTES);
    book.validate().expect("last accepted book remains valid");
}
