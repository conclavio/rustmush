//! **The NPC ability residual probe** (H2c; `docs/design/11-h-implementation-plan.md`
//! §3.2 H2c). Every NPC ability a ruleset grounding offers either maps to a
//! projected legal action (`Ruleset::projected_ability`, executed by the action
//! coordinator) or stays on the legacy `invoke_ability` path — the
//! **residual**. This probe lists the residual for the grounding and pins it,
//! so it can only shrink: a new grounding ability that appears unlisted fails
//! here until it is either mapped or deliberately pinned.
//!
//! 16b cut R-b: the two game rulesets this used to run over are gone, so the
//! probe runs over the `toy` test ruleset's grounding. The platform seam is
//! identical — `RulesetAgent<R>` over a `Ruleset` whose `projected_ability`
//! maps exactly one shape of one ability and leaves the rest legacy.

use std::sync::Arc;

use mush_core::{GOD, ObjId};
use mush_npc::{AgentGrounding, NpcModule};
use mush_server::modules::RulesetAgent;
use mush_testkit::toy_ruleset::ToyRuleset;
use mush_testkit::{TestWorld, text_of};

/// The pinned residual: `<ruleset>:<ability>` for every ability that still
/// reaches `invoke_ability` from the NPC completion with a plain input.
const RESIDUAL: &[&str] = &[];

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
    let npc = ObjId(digits.parse().expect("the create line names a numeric id"));
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

    // The toy: `toy_roll` without Willpower maps to `toy_roll`; with Willpower
    // it maps to `toy_willpower_roll`, leaving zero residual.
    let grounding: Arc<dyn AgentGrounding> = Arc::new(RulesetAgent::new(ToyRuleset::new()));
    let mut tw = TestWorld::with_modules(vec![
        Box::new(mush_jobs::JobsModule::new()),
        Box::new(ToyRuleset::new()),
        Box::new(NpcModule::with_grounding(Arc::clone(&grounding))),
    ]);
    let npc = create_agent(&mut tw, "alpha");
    residual.extend(residual_of("toy", &tw, grounding.as_ref(), npc));
    let plain = grounding
        .projected_ability(
            tw.world(),
            npc,
            "toy_roll",
            &serde_json::json!({ "pool_expr": "strength" }),
        )
        .expect("a plain toy_roll maps to the coordinator");
    assert_eq!(plain.action.key().id.as_str(), "toy_roll");
    assert_eq!(plain.parameters["pool"], "strength");
    assert_eq!(plain.parameters["ignore_wounds"], false);
    let projected = grounding
        .projected_ability(
            tw.world(),
            npc,
            "toy_roll",
            &serde_json::json!({ "pool_expr": "strength", "willpower": true }),
        )
        .expect("a Willpower-backed toy_roll maps to the coordinator");
    assert_eq!(projected.action.key().id.as_str(), "toy_willpower_roll");
    assert_eq!(projected.parameters["pool"], "strength");
    assert_eq!(projected.parameters["ignore_wounds"], false);

    residual.sort();
    let pinned: Vec<String> = RESIDUAL.iter().map(|s| (*s).to_owned()).collect();
    assert_eq!(
        residual, pinned,
        "the NPC ability residual changed: map the new ability through \
         `Ruleset::projected_ability`, or pin it here deliberately"
    );
}
