//! **Doc 17h cut M8e E4/E5: the dispatch's seal, by its pure functions** — the mind sealed
//! over a cored unit ([`seal_held`]), the ROM a lapsed licence falls to ([`seal_rom`]) and the
//! record the act carries ([`assemble`]); the Roll's reads are the excursion harness's
//! (`tests/shard_4/ic_sortie_surface.rs`).
use super::*;
use crate::arena::PartyId;
use crate::arena::core::CoreClass;
use crate::arena::mind::SealEngine;
use crate::arena::mind_seal::Admission;
use crate::arena::profile_seal::{FiledCertificate, MindIdentity, MindOrigin};
use crate::arena::runtime_card::stock_definition;
use crate::arena::test_support::{pose, tank};
use mush_core::{CommitToken, DefinitionDigest};
use sha2::{Digest as _, Sha256};
use std::sync::Arc;

type TestResult = Result<(), Box<dyn std::error::Error>>;
const TV: ObjId = ObjId(40);
const OPERATOR: ObjId = ObjId(10);
const SOURCE: &str = "fn think(v,c) { this.marker = 73; }";

/// A cored hull's unit (the fixture tank's Class I core), governed or not.
fn unit(governed: bool) -> Result<mush_tactics::TacticalUnit, Box<dyn std::error::Error>> {
    let mut unit = tank(TV, PartyId::RED, pose(1500, 1500)).unit;
    if !governed {
        let core = mush_tactics::ComponentId::new("core")?;
        let key = mush_tactics::CapabilityKey {
            id: mush_tactics::CapabilityId::new(crate::definitions::unit::CORE_CAPABILITY)?,
            source: Some(core),
        };
        let schema = unit.capability(&key).ok_or("a core")?.schema().clone();
        let class = crate::arena::core::read_core(&unit).ok_or("a core")?.class;
        let plan = mush_tactics::UnitMutationPlan::new(
            vec![mush_tactics::UnitMutationOp::GrantCapability {
                key,
                rules: mush_core::RulesValue::new(
                    schema,
                    crate::definitions::core::profile_payload(class, false),
                )?,
            }],
            Some(unit.revision()),
            None,
        )?;
        unit = mush_tactics::apply(&unit, &plan, 0)?.0;
    }
    Ok(unit)
}

/// A held scripted mind: filed under licence 7 (a certificate as a Roll holding resolves to),
/// or a personal draft.
fn held(filed: bool) -> Result<HeldMind, Box<dyn std::error::Error>> {
    let card = crate::arena::card::line(&crate::config::ArenaConfig::default().reflex);
    let definition = stock_definition(&[("Line".into(), card)], CoreClass::I)?;
    let digest: [u8; 32] = Sha256::digest(SOURCE.as_bytes()).into();
    let origin = if filed {
        MindOrigin::Filed {
            charter: "1".into(),
            line: "1".into(),
            revision: 1,
            licence: "7".into(),
            certificate: Box::new(FiledCertificate {
                revision_character: ObjId(77),
                revision_mind: 1,
                revision_number: 1,
                revision_digest: DefinitionDigest::from_bytes([1; 32]),
                source_digest: DefinitionDigest::from_bytes(digest),
                rating_job: CommitToken::from_bytes([4; 16]),
                rating_id: DefinitionDigest::from_bytes([2; 32]),
                rating_schema: 1,
                rating_digest: DefinitionDigest::from_bytes([3; 32]),
                compatibility: DefinitionDigest::from_bytes([4; 32]),
                definition: definition.validate()?,
                declared: CoreClass::I,
                minimum: CoreClass::I,
            }),
        }
    } else {
        MindOrigin::Draft {
            character: OPERATOR,
            mind: 1,
            revision: 1,
            digest: DefinitionDigest::from_bytes([1; 32]),
        }
    };
    Ok(HeldMind {
        presets: vec!["Line".into()],
        definition: Some(Arc::new(definition)),
        saved: None,
        identity: MindIdentity {
            mind: if filed { "Surveyor" } else { "Scratch" }.into(),
            origin,
            source_sha256: digest,
        },
        rated: CoreClass::I,
        engine: SealEngine::Scripted,
        source: Some(SOURCE.into()),
    })
}

fn licence() -> Result<LicenceId, Box<dyn std::error::Error>> {
    Ok(LicenceId::new(7)?)
}

/// **A held filed mind is sealed over the cored rig** (doc 23 S.4a; D-UM-45): the line's facts
/// name the mind, its licence, the licence's own state (`active`), the core and its class and
/// the governed bit; the epoch's id is the rig's; the holder rides beside the seal and the
/// warrant principal stays the operator (D-23-5).
#[test]
fn m8e_a_held_filed_mind_seals_with_its_licence_word() -> TestResult {
    let store = TacsimStore::default();
    let unit = unit(true)?;
    let sortie = SortieId::ic(TV, 500);
    let seal = seal_held(TV, &unit, sortie, &held(true)?, OPERATOR, &store)?;
    check_eq!(seal.sortie, sortie);
    check_eq!(
        seal.principal,
        Some(CharacterRef::Character { id: OPERATOR })
    );
    let sealed = assemble(
        &seal,
        SealLicenceState::Active,
        Some(licence()?),
        Some(Holder::Character(OPERATOR)),
    );
    check_eq!(sealed.facts.mind.as_str(), "Surveyor");
    check_eq!(sealed.facts.licence.as_str(), "7");
    check_eq!(sealed.facts.licence_state, Some(SealLicenceState::Active));
    check_eq!(sealed.facts.core.as_deref(), Some("core"));
    check_eq!(sealed.facts.core_class, Some(CoreClass::I));
    check!(sealed.facts.governed);
    check!(sealed.facts.admissions.is_empty());
    check_eq!(sealed.facts.holder, Some(Holder::Character(OPERATOR)));
    check_eq!(sealed.licence, Some(7));
    check!(!sealed.fell_to_doctrine());
    check_eq!(
        sealed.mark_licence(),
        Some(7),
        "a mind that flies marks its licence"
    );
    check_eq!(
        doctrine_clause(&sealed.facts, "1"),
        None,
        "a mind that flies tells nothing"
    );
    // The act's record round-trips strictly.
    let back: DispatchSeal = serde_json::from_value(serde_json::to_value(&sealed)?)?;
    check_eq!(back, sealed);
    Ok(())
}

/// **A lapsed licence seals Doctrine** (doc 17h cut M8e E4; D-UM-39): the ROM's seal — Standard
/// Doctrine, its digest the ROM seal's own — with the line's `licence_state` `lapsed` and the
/// licence named beside it, so an inspection reads which licence lapsed; the record says the
/// rig fell to doctrine, which the operator alone is told. A revoked licence the same.
#[test]
fn m8e_a_lapsed_licence_seals_doctrine_and_names_the_licence() -> TestResult {
    let store = TacsimStore::default();
    let unit = unit(true)?;
    let sortie = SortieId::ic(TV, 500);
    let rom = seal_rom(TV, &unit, sortie, OPERATOR, &store, false)?;
    for state in [SealLicenceState::Lapsed, SealLicenceState::Revoked] {
        let sealed = assemble(
            &rom,
            state,
            Some(licence()?),
            Some(Holder::Character(OPERATOR)),
        );
        check_eq!(sealed.facts.mind.as_str(), "Standard Doctrine");
        check_eq!(sealed.facts.licence.as_str(), "7");
        check_eq!(sealed.facts.licence_state, Some(state));
        check_eq!(
            sealed.facts.seal_digest.as_str(),
            crate::arena::snapshot::digest_text(&rom.digest()).as_str()
        );
        check!(sealed.fell_to_doctrine());
        check_eq!(
            sealed.mark_licence(),
            None,
            "a seal that fell to the ROM marks nothing"
        );
        check_eq!(
            doctrine_clause(&sealed.facts, "1").as_deref(),
            Some(
                format!(
                    "Licence 7 is {}: rig 1 flies Standard Doctrine sealed aboard.",
                    state.as_str()
                )
                .as_str()
            )
        );
    }
    // The ROM with no licence named is the stock seal's word, and never "fell".
    let stock = assemble(&rom, SealLicenceState::Doctrine, None, None);
    check_eq!(stock.facts.licence.as_str(), "doctrine");
    check!(!stock.fell_to_doctrine());
    check_eq!(doctrine_clause(&stock.facts, "1"), None);
    Ok(())
}

/// **An org-held licence flown by a granted member** (M8c B3): the holder is the organisation
/// with the member acting, on the facts and on the box's open line built from them (the
/// `Holder` wire, `{"org":n,"acting":m}`); the warrant principal stays the member.
#[test]
fn m8e_an_org_held_licence_seals_with_the_org_holder() -> TestResult {
    let store = TacsimStore::default();
    let unit = unit(true)?;
    let sortie = SortieId::ic(TV, 500);
    let seal = seal_held(TV, &unit, sortie, &held(true)?, OPERATOR, &store)?;
    let org = Holder::Organization {
        org: mush_org::OrgId(9),
        acting: OPERATOR,
    };
    let sealed = assemble(&seal, SealLicenceState::Grace, Some(licence()?), Some(org));
    check_eq!(sealed.facts.holder, Some(org));
    let opened = crate::surface::excursion::OpenSortie {
        id: sortie,
        hull: ObjId(20),
        slot: 1,
        site: mush_core::CanonicalId::new("ahuna")?,
        operator: OPERATOR,
        opened_ic: 800,
        abandoned: false,
        abandoned_ic: None,
        unrecorded: false,
        seal: Some(sealed.facts.clone()),
    };
    let line = crate::surface::black_box::BlackBoxLine::mind_sealed(
        &opened,
        TV,
        seal.principal.clone(),
        crate::surface::black_box::Admitted {
            class: Some(crate::definitions::UnitClass::Vehicle),
            host: crate::surface::black_box::EpochHost::Surface,
            count: 1,
        },
    );
    check!(line.check(TV).is_ok());
    let value = serde_json::to_value(&line)?;
    check_eq!(
        value["holder"],
        serde_json::json!({"org": 9, "acting": OPERATOR.0})
    );
    check_eq!(value["licence_state"], serde_json::json!("grace"));
    check_eq!(
        value["principal"],
        serde_json::to_value(CharacterRef::Character { id: OPERATOR })?
    );
    Ok(())
}

/// **The ungoverned rig's draft** (doc 17h cut M8e E5; D-UM-41, D-UM-42): an unfiled personal
/// draft sealed over an ungoverned core carries `unfiled` and `unlicensed` in its admissions,
/// with the `draft` state and no licence (no seal mark); on the box's open line too.
#[test]
fn m8e_an_ungoverned_rig_seals_a_draft_with_its_admissions() -> TestResult {
    let store = TacsimStore::default();
    let unit = unit(false)?;
    let sortie = SortieId::ic(TV, 500);
    let seal = seal_held(TV, &unit, sortie, &held(false)?, OPERATOR, &store)?;
    check!(!seal.governed);
    let sealed = assemble(&seal, SealLicenceState::Draft, None, None);
    check_eq!(
        sealed.facts.admissions,
        vec![Admission::Unfiled, Admission::Unlicensed]
    );
    check_eq!(sealed.facts.licence_state, Some(SealLicenceState::Draft));
    check!(!sealed.facts.governed);
    check_eq!(sealed.licence, None);
    check_eq!(sealed.mark_licence(), None);
    check_eq!(sealed.facts.holder, None);
    Ok(())
}

/// **The `mind=` word** (doc 17h cut M8e E4): `filed:<n>` a licence, `draft:<name>` (either
/// prefix in any case, fix round 1 J7) or a bare name a draft; `filed:` with no number, an empty draft name and zero refuse.
#[test]
fn m8e_the_mind_word() -> TestResult {
    check_eq!(
        MindWord::parse("filed:7"),
        Some(MindWord::Filed(licence()?))
    );
    check_eq!(
        MindWord::parse("FILED:7"),
        Some(MindWord::Filed(licence()?))
    );
    check_eq!(
        MindWord::parse("draft:Scratch"),
        Some(MindWord::Draft("Scratch".into()))
    );
    check_eq!(
        MindWord::parse("DRAFT:Scratch"),
        Some(MindWord::Draft("Scratch".into()))
    );
    check_eq!(
        MindWord::parse("Scratch"),
        Some(MindWord::Draft("Scratch".into()))
    );
    check_eq!(MindWord::parse("filed:"), None);
    check_eq!(MindWord::parse("filed:0"), None);
    check_eq!(MindWord::parse("draft:"), None);
    Ok(())
}

/// **The pending sentence** (fix round 1, J4; doc 17 D-UM-40): it names the mind.
#[test]
fn m8e_the_pending_sentence_names_the_mind() {
    assert_eq!(
        MIND_PENDING.replace("{mind}", "Surveyor"),
        "Surveyor is changing hands; it is not sealed aboard until the transfer settles."
    );
}

/// **The state rule for a paid view or an owner row** (fix round 1, J2): Active and Grace seal
/// the mind, Lapsed and (a paid view's) revoked fall to the ROM, and a funded transfer is
/// pending whatever the phase; an owner row (no revoked bit) follows the same rule.
#[test]
fn m8e_the_state_rule_for_paid_and_owner_rows() {
    use mush_roll::commerce::command_reader::OwnerPhase;
    let active = OwnerPhase::Active { days_left: 3 };
    let grace = OwnerPhase::Grace { days_left: 1 };
    assert_eq!(state_of(active, false, false), SealLicenceState::Active);
    assert_eq!(state_of(grace, false, false), SealLicenceState::Grace);
    assert_eq!(
        state_of(OwnerPhase::Lapsed, false, false),
        SealLicenceState::Lapsed
    );
    assert_eq!(state_of(active, true, false), SealLicenceState::Revoked);
    for phase in [active, grace, OwnerPhase::Lapsed] {
        assert_eq!(state_of(phase, false, true), SealLicenceState::Pending);
    }
}

/// **A held mind on a seized rig** (doc 17h cut M8d2, "impounded in place"; the dispatch's
/// impounded branch): the licence still resolves, but the rig flies the ROM sealed as no core —
/// no core, Class 0, governed, no floor lifted — recorded under `doctrine` with no licence, no
/// holder and no seal mark, while the core stays fitted. The same rig unimpounded seals the
/// mind; a governed rig's ROM is unchanged by the flag's absence.
#[test]
fn m8d2_a_held_mind_on_a_seized_rig_seals_the_rom() -> TestResult {
    let store = TacsimStore::default();
    let rig = unit(false)?;
    let governed_rig = unit(true)?;
    let sortie = SortieId::ic(TV, 500);
    let mind = seal_held(TV, &rig, sortie, &held(true)?, OPERATOR, &store)?;
    check_eq!(mind.class, CoreClass::I, "unimpounded, the mind flies");
    let rom = seal_rom(TV, &rig, sortie, OPERATOR, &store, true)?;
    check_eq!(
        (rom.core.clone(), rom.core_class, rom.class, rom.governed),
        (None, CoreClass::Zero, CoreClass::Zero, true)
    );
    check!(!rom.lifts_floor());
    check_eq!(rom.engine, SealEngine::Native);
    let sealed = assemble(&rom, SealLicenceState::Doctrine, None, None);
    check_eq!(sealed.facts.licence_state, Some(SealLicenceState::Doctrine));
    check_eq!(sealed.licence, None);
    check_eq!(sealed.mark_licence(), None);
    check_eq!(sealed.facts.holder, None);
    check!(crate::arena::core::read_core(&rig).is_some_and(|hw| !hw.governed));
    // A governed rig, not impounded: its ROM seals over its fitted core as before.
    let governed = seal_rom(TV, &governed_rig, sortie, OPERATOR, &store, false)?;
    check!(governed.core.is_some() && governed.governed);
    Ok(())
}

/// **The owner row is read before the paid view** (doc 17h cut M8g's finding): a refused owner
/// read poisons the whole preparation, and the paid view refuses every owner licence, so the
/// paid view is asked only when no owner row names the licence. An owner row answers with no
/// paid read made; a paid licence (no owner row) is read through its view; neither is missing.
#[test]
fn m8g_the_owner_row_is_read_before_the_paid_view() -> TestResult {
    let order = |owner: Option<u8>, paid: Option<u8>| {
        let mut asked = Vec::new();
        let found = owner_then_paid(
            &mut asked,
            |asked| {
                asked.push("owner");
                owner
            },
            |asked| {
                asked.push("paid");
                paid
            },
        );
        (found, asked)
    };
    check_eq!(order(Some(1), Some(2)), (Some(1), vec!["owner"]));
    check_eq!(order(None, Some(2)), (Some(2), vec!["owner", "paid"]));
    check_eq!(order(None, None), (None, vec!["owner", "paid"]));
    Ok(())
}
