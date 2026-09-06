//! **The NPC ability residual probe** (H2c; `docs/design/11-h-implementation-plan.md`
//! §3.2 H2c). Every NPC ability a production ruleset grounding offers either
//! maps to a projected legal action (`Ruleset::projected_ability`, executed by
//! the action coordinator) or stays on the legacy `invoke_ability` path — the
//! **residual**. This probe lists the residual for every production grounding
//! and pins it, so it can only shrink: a new grounding ability that appears
//! unlisted fails here until it is either mapped or deliberately pinned.

use std::sync::Arc;

use mush_core::{GOD, ObjId};
use mush_npc::{AgentGrounding, NpcModule};
use mush_server::modules::RulesetAgent;
use mush_testkit::{TestWorld, text_of};

/// The pinned residual: `<ruleset>:<ability>` for every ability that still
/// reaches `invoke_ability` from the NPC completion with a plain input.
const RESIDUAL: &[&str] = &["shadowrun:sr_roll", "wod:wod_roll"];

fn create_agent(tw: &mut TestWorld, template: &str) -> ObjId {
    let created: String = tw
        .run(GOD, "@agent/create Bob")
        .iter()
        .map(|(_, e)| text_of(e))
        .collect::<Vec<_>>()
        .join("\n");
    let at = created.find("(#").expect("the create line names the id");
    let digits: String = created[at + 2..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    let npc = ObjId(digits.parse().unwrap());
    let sheet: String = tw
        .run(GOD, &format!("@agent/sheet Bob={template}"))
        .iter()
        .map(|(_, e)| text_of(e))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(sheet.contains("sheet"), "{sheet}");
    npc
}

/// Every ability the grounding offers `npc`, with the plain input the model
/// would send, and whether a projected action maps it.
fn residual_of(
    label: &str,
    tw: &TestWorld,
    grounding: &dyn AgentGrounding,
    npc: ObjId,
) -> Vec<String> {
    let profile = grounding
        .profile(tw.world(), npc)
        .expect("the agent has a capability profile");
    assert!(
        !profile.tools.is_empty(),
        "{label}: the grounding offers no ability"
    );
    profile
        .tools
        .iter()
        .filter(|tool| {
            grounding
                .projected_ability(
                    tw.world(),
                    npc,
                    &tool.name,
                    &serde_json::json!({ "pool_expr": "strength" }),
                )
                .is_none()
        })
        .map(|tool| format!("{label}:{}", tool.name))
        .collect()
}

#[test]
fn h2c_the_residual_is_pinned_and_can_only_shrink() {
    let mut residual = Vec::new();

    // WoD: `wod_roll` without Willpower is legacy; with Willpower it is the
    // coordinator's `willpower_roll`.
    let wod_grounding: Arc<dyn AgentGrounding> =
        Arc::new(RulesetAgent::new(ruleset_wod::WodRuleset::new()));
    let mut tw = TestWorld::with_modules(vec![
        Box::new(mush_jobs::JobsModule::new()),
        Box::new(ruleset_wod::WodRuleset::new()),
        Box::new(NpcModule::with_grounding(Arc::clone(&wod_grounding))),
    ]);
    let npc = create_agent(&mut tw, "mortal");
    residual.extend(residual_of("wod", &tw, wod_grounding.as_ref(), npc));
    let projected = wod_grounding
        .projected_ability(
            tw.world(),
            npc,
            "wod_roll",
            &serde_json::json!({ "pool_expr": "strength", "willpower": true }),
        )
        .expect("a Willpower-backed wod_roll maps to the coordinator");
    assert_eq!(projected.action.key().id.as_str(), "willpower_roll");
    assert_eq!(projected.parameters["pool"], "strength");
    assert_eq!(projected.parameters["ignore_wounds"], false);

    // Shadowrun: `sr_roll` is wholly legacy.
    let sr_grounding: Arc<dyn AgentGrounding> =
        Arc::new(RulesetAgent::new(ruleset_shadowrun::ShadowrunRuleset::new()));
    let mut tw = TestWorld::with_modules(vec![
        Box::new(mush_jobs::JobsModule::new()),
        Box::new(ruleset_shadowrun::ShadowrunRuleset::new()),
        Box::new(NpcModule::with_grounding(Arc::clone(&sr_grounding))),
    ]);
    let npc = create_agent(&mut tw, "runner");
    residual.extend(residual_of("shadowrun", &tw, sr_grounding.as_ref(), npc));

    residual.sort();
    let pinned: Vec<String> = RESIDUAL.iter().map(|s| (*s).to_owned()).collect();
    assert_eq!(
        residual, pinned,
        "the NPC ability residual changed: map the new ability through \
         `Ruleset::projected_ability`, or pin it here deliberately"
    );
}
