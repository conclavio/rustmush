//! End-to-end tests: a toy [`Ruleset`] wired through the real [`Engine`] via
//! [`mush_testkit::TestWorld`], exercising the generic command surface
//! (`+sheet`, `@sheet/set`), the web query handler, sheet storage through a
//! `World` attribute, schema composition (the splat seam), and the
//! [`Ruleset`] trait itself.

use std::sync::Arc;

use mush_core::{GOD, GameModule, ModError, ObjId, Registrar};
use mush_proto::OutputEvent;
use mush_ruleset::chargen::{
    BudgetPlan, ChargenFlow, ChargenIssue, ChargenStage, CostRule, FieldSelector, PoolDef,
};
use mush_ruleset::schema::{
    DamageLevelDef, FieldDef, FieldType, SectionLayout, SheetSchema, SheetSchemaBuilder,
    SheetSection, TrackSize,
};
use mush_ruleset::{
    AgentCapabilityProfile, ApprovalState, Ruleset, SheetDoc, SheetError, register_sheet_commands,
};
use mush_testkit::TestWorld;
use smol_str::SmolStr;

// ----------------------------------------------------------------------
// The toy ruleset
// ----------------------------------------------------------------------

const RULESET: &str = "toy";

/// The base toy sheet: three physical attributes, one ability, a bio block
/// with a staff-only field, an open backgrounds map, and a fixed health
/// track.
fn toy_schema() -> SheetSchema {
    SheetSchemaBuilder::new(RULESET, 1)
        .section(
            SheetSection::new("attributes", "Attributes", SectionLayout::grid(1, 3))
                .field(FieldDef::dots("strength", "Strength", 1, 5).category("physical"))
                .field(FieldDef::dots("dexterity", "Dexterity", 1, 5).category("physical"))
                .field(FieldDef::dots("stamina", "Stamina", 1, 5).category("physical")),
        )
        .unwrap()
        .section(
            SheetSection::new("abilities", "Abilities", SectionLayout::List)
                .field(FieldDef::dots("brawl", "Brawl", 0, 5).category("talents")),
        )
        .unwrap()
        .section(
            SheetSection::new("bio", "Bio", SectionLayout::List)
                .field(FieldDef::text("concept", "Concept", 40))
                .field(FieldDef::enumeration(
                    "template",
                    "Template",
                    vec!["mortal".into(), "vampire".into()],
                ))
                .field(FieldDef::dots("secret", "ST Secret", 0, 5).staff_only()),
        )
        .unwrap()
        .section(
            SheetSection::new("backgrounds", "Backgrounds", SectionLayout::Map).field(
                FieldDef::map(
                    "backgrounds",
                    "Backgrounds",
                    FieldType::Dots { min: 0, max: 5 },
                ),
            ),
        )
        .unwrap()
        .section(
            SheetSection::new("health", "Health", SectionLayout::Tracks).field(FieldDef::track(
                "health",
                "Health",
                TrackSize::Fixed(7),
                vec![
                    DamageLevelDef::new("bruised", "Bruised", 0),
                    DamageLevelDef::new("hurt", "Hurt", -1),
                    DamageLevelDef::new("injured", "Injured", -1),
                    DamageLevelDef::new("wounded", "Wounded", -2),
                    DamageLevelDef::new("mauled", "Mauled", -2),
                    DamageLevelDef::new("crippled", "Crippled", -5),
                    DamageLevelDef::new("incap", "Incapacitated", -127),
                ],
            )),
        )
        .unwrap()
        .build()
}

struct ToyRuleset {
    schema: Arc<SheetSchema>,
    chargen: ToyChargen,
}

impl ToyRuleset {
    fn new() -> ToyRuleset {
        let schema = Arc::new(toy_schema());
        ToyRuleset {
            chargen: ToyChargen {
                schema: (*schema).clone(),
            },
            schema,
        }
    }
}

impl GameModule for ToyRuleset {
    fn name(&self) -> &'static str {
        RULESET
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        register_sheet_commands(reg, Arc::clone(&self.schema), None)
    }
}

impl Ruleset for ToyRuleset {
    fn display_name(&self) -> &str {
        "Toy Ruleset"
    }

    fn sheet_schema(&self) -> &SheetSchema {
        &self.schema
    }

    fn chargen(&self) -> Option<&dyn ChargenFlow> {
        Some(&self.chargen)
    }

    fn agent_profile(&self, sheet: &SheetDoc) -> AgentCapabilityProfile {
        let brawl = sheet.dots("brawl").unwrap_or(0);
        AgentCapabilityProfile {
            summary: format!("Brawl {brawl}."),
            stats: serde_json::json!({ "brawl": brawl }),
            tags: if brawl >= 3 {
                vec![SmolStr::new("combat:brawl")]
            } else {
                vec![SmolStr::new("combat:weak")]
            },
            tools: Vec::new(),
        }
    }
}

/// A one-stage chargen flow: 5 physical points, cap 4.
struct ToyChargen {
    schema: SheetSchema,
}

impl ToyChargen {
    fn plan() -> BudgetPlan {
        BudgetPlan::new()
            .pool(PoolDef::new("physical", "Physical", 5))
            .rule(
                CostRule::new("physical", FieldSelector::Category("physical".into()), 1, 1)
                    .with_cap(4),
            )
    }
}

impl ChargenFlow for ToyChargen {
    fn stages(&self) -> Vec<ChargenStage> {
        vec![
            ChargenStage::new("attrs", "Attributes", "Spend 5 physical points.")
                .with_budget(Self::plan()),
        ]
    }

    fn validate_stage(&self, sheet: &SheetDoc, stage: &SmolStr) -> Vec<ChargenIssue> {
        self.stages()
            .into_iter()
            .find(|s| &s.id == stage)
            .map(|s| s.assess(&self.schema, sheet))
            .unwrap_or_default()
    }

    fn finalize(&self, sheet: &mut SheetDoc) -> Result<(), ChargenIssue> {
        let issues = self.validate_stage(sheet, &SmolStr::new("attrs"));
        if let Some(first) = issues.into_iter().next() {
            return Err(first);
        }
        // Derived: willpower = stamina + 3 (toy rule).
        let wp = sheet.number("stamina").unwrap_or(1) + 3;
        sheet.set_number("willpower", wp);
        Ok(())
    }
}

// ----------------------------------------------------------------------
// Helpers
// ----------------------------------------------------------------------

fn world() -> (TestWorld, ObjId) {
    let mut tw = TestWorld::with_modules(vec![Box::new(ToyRuleset::new())]);
    let bob = tw.spawn_player("Bob");
    (tw, bob)
}

fn saw(events: &[(mush_core::Audience, OutputEvent)], actor: ObjId, needle: &str) -> bool {
    events.iter().any(|(aud, ev)| {
        matches!(aud, mush_core::Audience::Actor(a) if *a == actor)
            && mush_testkit::text_of(ev).contains(needle)
    })
}

// ----------------------------------------------------------------------
// Tests
// ----------------------------------------------------------------------

#[test]
fn sheet_namespace_is_claimed() {
    let (tw, _bob) = world();
    assert_eq!(
        tw.registry().attr_ns_owner("sheet").map(SmolStr::as_str),
        Some(RULESET)
    );
}

#[test]
fn staff_set_creates_and_validates() {
    let (mut tw, bob) = world();

    // No sheet yet.
    let ev = tw.run(bob, "+sheet");
    assert!(saw(&ev, bob, "no sheet"));

    // Staff sets a scalar (creating the sheet).
    let ev = tw.run(GOD, "@sheet/set Bob/strength=3");
    assert!(saw(&ev, GOD, "Set Bob/strength = 3"));

    // Out-of-range is rejected, nothing changes.
    let ev = tw.run(GOD, "@sheet/set Bob/strength=99");
    assert!(saw(&ev, GOD, "out of range"));

    // Enum membership enforced.
    let ev = tw.run(GOD, "@sheet/set Bob/template=werewolf");
    assert!(saw(&ev, GOD, "not one of"));

    // A map entry.
    let ev = tw.run(GOD, "@sheet/set Bob/backgrounds:Resources=2");
    assert!(saw(&ev, GOD, "backgrounds:Resources = 2"));

    // Bob's own view shows the dots and hides the staff-only field.
    let ev = tw.run(bob, "+sheet");
    assert!(saw(&ev, bob, "***..")); // strength 3
    assert!(saw(&ev, bob, "Resources"));
    assert!(!saw(&ev, bob, "Secret")); // staff_only hidden from the player
}

#[test]
fn staff_only_field_gate() {
    let (mut tw, bob) = world();
    tw.run(GOD, "@sheet/set Bob/strength=2");

    // A player cannot run the staff editor at all.
    let ev = tw.run(bob, "@sheet/set Bob/strength=5");
    assert!(saw(&ev, bob, "Permission denied."));

    // Staff can set a staff-only field, and the staff view shows it.
    let ev = tw.run(GOD, "@sheet/set Bob/secret=4");
    assert!(saw(&ev, GOD, "Set Bob/secret = 4"));
    let ev = tw.run(GOD, "@sheet Bob");
    assert!(saw(&ev, GOD, "Secret")); // staff view includes staff-only
}

#[test]
fn store_round_trips_through_world_attr() {
    let (mut tw, bob) = world();
    let store = mush_ruleset::SheetStore::new(Arc::new(toy_schema()));

    let mut doc = SheetDoc::new_default(store.schema());
    doc.set_number("strength", 4);
    doc.set_text("concept", "brooding detective");
    store.put(tw.world_mut(), bob, &doc).unwrap();

    // Read back identical.
    let read = store.get(tw.world(), bob).unwrap();
    assert_eq!(read, doc);
    assert_eq!(read.dots("strength"), Some(4));

    // Invalid put is rejected and leaves storage untouched.
    let mut bad = read.clone();
    bad.set_number("strength", 42);
    assert!(store.put(tw.world_mut(), bob, &bad).is_err());
    assert_eq!(
        store.get(tw.world(), bob).unwrap().dots("strength"),
        Some(4)
    );
}

#[test]
fn web_query_emits_schema_and_filters_staff_fields() {
    let (mut tw, bob) = world();
    tw.run(GOD, "@sheet/set Bob/strength=3");
    tw.run(GOD, "@sheet/set Bob/secret=5");

    let (_, handler) = tw
        .registry()
        .query_handler("toy.sheet")
        .expect("query registered");

    // Bob queries his own sheet: staff-only field stripped.
    let ev = handler
        .query(tw.world(), bob, &serde_json::json!({ "target": null }))
        .unwrap();
    let OutputEvent::Panel { feed, data, .. } = &ev else {
        panic!("expected a Panel frame, got {ev:?}");
    };
    assert_eq!(feed.as_str(), "toy.sheet");
    assert!(data.get("schema").is_some(), "schema description present");
    assert_eq!(data["data"]["strength"], serde_json::json!(3));
    assert!(
        data["data"].get("secret").is_none(),
        "staff field stripped for player"
    );

    // Staff sees the staff-only field.
    let ev = handler
        .query(tw.world(), GOD, &serde_json::json!({ "target": "Bob" }))
        .unwrap();
    let OutputEvent::Panel { data, .. } = &ev else {
        panic!("expected a Panel frame");
    };
    assert_eq!(data["data"]["secret"], serde_json::json!(5));
}

#[test]
fn schema_composes_for_a_splat() {
    // The splat seam: extend the base without editing it.
    let base = toy_schema();
    let vampire = SheetSchemaBuilder::from_schema(&base)
        .set_version(2)
        // A new Disciplines map section...
        .section(
            SheetSection::new("disciplines", "Disciplines", SectionLayout::Map).field(
                FieldDef::map(
                    "disciplines",
                    "Disciplines",
                    FieldType::Dots { min: 0, max: 5 },
                ),
            ),
        )
        .unwrap()
        // ...a Blood Pool track...
        .section(
            SheetSection::new("blood", "Blood", SectionLayout::Tracks).field(FieldDef::track(
                "blood_pool",
                "Blood Pool",
                TrackSize::Fixed(10),
                vec![],
            )),
        )
        .unwrap()
        // ...and a Generation background onto the existing bio section.
        .extend_section("bio", [FieldDef::number("generation", "Generation", 3, 13)])
        .unwrap()
        .build();

    // Base untouched; extension has both old and new.
    assert!(base.section("disciplines").is_none());
    assert_eq!(vampire.version, 2);
    assert!(vampire.section("attributes").is_some());
    assert!(vampire.section("disciplines").is_some());
    assert!(vampire.field("blood_pool").is_some());
    assert!(vampire.field("generation").is_some());
    // A default sheet for the composed schema is valid.
    assert!(mush_ruleset::validate(&vampire, &vampire.default_data()).is_ok());
}

#[test]
fn ruleset_trait_surface() {
    let toy = ToyRuleset::new();
    assert_eq!(toy.id().as_str(), "toy");
    assert_eq!(toy.display_name(), "Toy Ruleset");
    assert_eq!(toy.sheet_schema().version, 1);
    assert!(toy.chargen().is_some());

    // Default migrate: current version passes, older is refused.
    let data = serde_json::json!({ "strength": 2 });
    assert!(toy.migrate_sheet(1, data.clone()).is_ok());
    assert!(matches!(
        toy.migrate_sheet(0, data),
        Err(SheetError::Msg(_))
    ));

    // agent_profile grounds a sheet.
    let mut doc = SheetDoc::new_default(toy.sheet_schema());
    doc.set_number("brawl", 4);
    let profile = toy.agent_profile(&doc);
    assert!(profile.tags.iter().any(|t| t == "combat:brawl"));

    // invoke_ability default rejects unknown abilities.
    assert!(
        toy.invoke_ability(GOD, "wod_roll", serde_json::json!({}))
            .is_err()
    );
}

#[test]
fn chargen_flow_via_framework() {
    use mush_ruleset::chargen::{ChargenProgress, finalize, try_advance};

    let toy = ToyRuleset::new();
    let flow = toy.chargen().unwrap();
    let mut doc = SheetDoc::new_default(toy.sheet_schema());
    let mut progress = ChargenProgress::start();

    // Underspent → refused.
    assert!(try_advance(flow, &doc, &mut progress).is_err());

    // Spend exactly 5 physical points.
    doc.set_number("strength", 4);
    doc.set_number("dexterity", 2);
    doc.set_number("stamina", 2);
    assert!(try_advance(flow, &doc, &mut progress).is_ok());
    assert!(progress.complete);

    // Finalize computes the derived willpower (stamina 2 + 3).
    finalize(flow, &mut doc).unwrap();
    assert_eq!(doc.number("willpower"), Some(5));
    assert_eq!(doc.approved, ApprovalState::Unapproved);
}
