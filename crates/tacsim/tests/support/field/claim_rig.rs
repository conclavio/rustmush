//! **The Registry's claim channel over a field world** (doc 23 cut S.4b-T): `driving`'s keyed
//! rig rebooted under a mind commerce with the Registry inspection and the quirk claim composed,
//! the ported twin of `excursions.rs`'s `reboot_with_inspection` (`domain_field`), so `@mind/claim`
//! runs through its production command path beside a running IC field. Every domain root that
//! needs it declares this module by path, like the other `*_rig` modules.
#![allow(dead_code, unused_imports)]

use std::sync::Arc;

use mush_core::{AuthoritativeSubmit, GOD, ObjId, PerceivedRef};
use mush_org::{PrincipalId, SystemPrincipalId};
use mush_proto::OutputEvent;
use tacsim::TacsimConfig;

use crate::driving_rig::{CommerceFactory, Rig, land_composed, reboot};

/// A staff command that must not be refused.
pub(crate) fn god(rig: &mut Rig, line: &str) {
    let events = rig.tw.run(GOD, line);
    let refused: Vec<String> = events
        .iter()
        .filter_map(|(_, event)| match event {
            OutputEvent::Sys(text)
                if text
                    .spans()
                    .iter()
                    .any(|span| span.tag.as_deref() == Some("error")) =>
            {
                Some(text.to_plain_text())
            }
            _ => None,
        })
        .collect();
    assert!(refused.is_empty(), "{line}: {refused:?}");
}

/// **The Registry inspection and its quirk claim composed** (doc 17h cuts M8a, M8g G3), as
/// `excursions.rs` composes them: a `Registry` org with a `review` scope, `operator` granted
/// `roll.registry.inspect` on it and a system account for its filings, by GOD's commands in the
/// running world; then the Roll's acknowledged setup (a reboot under `Initialize`, GOD's one
/// `initialize_commerce`) and the reboot under `Active` with the inspection, its fine, the
/// principal revocation and the claim. Every later reboot keeps the commerce.
pub(crate) fn with_claims(rig: Rig, operator: ObjId, reconnect: &[ObjId]) -> Rig {
    use mush_core::mechanics::{
        DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
    };
    use mush_roll::commerce::{
        filing::InitialFilingTerms, module::CommerceMode,
        registry_inspection::RegistryInspectionPolicy,
    };
    use tacsim::composition::mind_commerce::MindCommerce;

    let mut rig = rig;
    for line in [
        "@org/create Registry",
        "@org/scope create Registry=review",
        "@account/system registry.fixture=tacsim.credit",
    ] {
        god(&mut rig, line);
    }
    let (org, scope) = {
        let state = rig
            .tw
            .world()
            .module_state::<mush_org::OrgStore>()
            .expect("the Org owner")
            .state();
        let org = state
            .orgs()
            .find(|o| o.name.as_str() == "Registry")
            .expect("the Registry org")
            .id;
        let scope = state
            .scopes_in(org)
            .find(|s| s.name.as_str() == "review")
            .expect("the review scope")
            .id;
        (org, scope)
    };
    god(
        &mut rig,
        &format!(
            "@org/grant #{}=Registry roll.registry.inspect scope=#{}",
            operator.0, scope.0
        ),
    );
    let issuer = SystemPrincipalId::new("registry.fixture").expect("a system principal");
    let account = rig
        .tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .expect("economy")
        .snapshot()
        .accounts()
        .find(|a| a.owner == PrincipalId::System(issuer.clone()))
        .expect("the Registry's account")
        .id;
    let policy = RegistryInspectionPolicy::new(issuer.clone(), org, scope, vec![operator])
        .expect("the inspection policy");
    let cfg = rig.cfg.clone();
    let commerce = move |mode: CommerceMode| -> MindCommerce {
        let layer =
            tacsim::TacsimRuleset::with_game_content(cfg.clone(), true).with_test_world(true);
        let commerce = MindCommerce::prepare(
            &layer,
            mode,
            InitialFilingTerms::new(90 * 86_400, 3 * 86_400).expect("the filing terms"),
            mush_economy::registry_filing::InitialFilingPolicy::new(
                issuer.clone(),
                account,
                mush_economy::Amount::from_minor(0),
                mush_economy::CatalogRef {
                    ruleset: "roll".into(),
                    code: "mind_charter".into(),
                },
            )
            .expect("the filing policy"),
        )
        .expect("the commerce prepares");
        if mode == CommerceMode::Active {
            commerce
                .with_registry_inspection(policy.clone())
                .expect("the inspection composes")
                .with_registry_fines(account)
                .expect("the fine composes")
                .with_registry_principal_revocation(90)
                .expect("the principal revocation composes")
                .with_registry_claims()
                .expect("the claim composes")
        } else {
            commerce
        }
    };
    let commerce = Arc::new(commerce);
    let initialize: CommerceFactory = {
        let commerce = commerce.clone();
        Arc::new(move || commerce(CommerceMode::Initialize))
    };
    rig.commerce = Some(initialize);
    let mut rig = reboot(rig, reconnect);
    let setup = DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new("roll").expect("an owner"),
            DefinitionDescriptor::new(
                DefinitionKind::new("action").expect("a kind"),
                DefinitionId::new("initialize_commerce").expect("an id"),
                1,
                serde_json::json!({"format":1,"explicit_setup":true}),
            )
            .expect("the setup descriptor"),
        )
        .expect("the setup reference");
    // The composed path: the prepare initializes the commerce shells through its context.
    land_composed(
        &mut rig,
        &AuthoritativeSubmit {
            actor: GOD,
            action: setup,
            subject: PerceivedRef::SelfActor,
            targets: Vec::new(),
            parameters: serde_json::json!({}),
            key: mush_core::IdempotencyKeyText::parse_canonical("roll.initialize.0")
                .expect("a key"),
        },
    )
    .expect("the commerce initializes");
    let active: CommerceFactory = Arc::new(move || commerce(CommerceMode::Active));
    rig.commerce = Some(active);
    reboot(rig, reconnect)
}
