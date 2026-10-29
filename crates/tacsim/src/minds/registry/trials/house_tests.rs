//! The launch House minds over the Registry's actual Trial cases (doc 17h M8,
//! the zero-PC story): each case opened and run to its real end exactly as
//! the rating's machine runs it.

use super::*;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const HOUSE: [(&str, &str, &str); 3] = [
    (
        "meridiani_field",
        include_str!("../../../../content/minds/house/meridiani_field.rhai"),
        include_str!("../../../../content/minds/house/meridiani_field.envelope.json"),
    ),
    (
        "coprates_drill",
        include_str!("../../../../content/minds/house/coprates_drill.rhai"),
        include_str!("../../../../content/minds/house/coprates_drill.envelope.json"),
    ),
    (
        "safe_conduct",
        include_str!("../../../../content/minds/house/safe_conduct.rhai"),
        include_str!("../../../../content/minds/house/safe_conduct.envelope.json"),
    ),
];

fn revision(
    source: &str,
    envelope: &str,
) -> Result<mush_roll::Revision, Box<dyn std::error::Error>> {
    let envelope: mush_roll::Envelope = serde_json::from_str(envelope)?;
    Ok(serde_json::from_value(serde_json::json!({
        "format": 1,
        "reference": {"key": {"character": 1, "mind": 1, "number": 1}, "digest": envelope.digest(source)?},
        "source": mush_roll::source_digest(source),
        "envelope": envelope,
        "notes": ""
    }))?)
}

/// Every Trial case of one class and variant, run to its real end.
fn trials(
    policy: &FrozenPolicy,
    subject: &PreparedSubject,
    class: RatingClass,
    variant: u8,
    out: &mut Vec<String>,
) -> Result<Vec<Option<TrialOutcome>>, Box<dyn std::error::Error>> {
    let mut outcomes = Vec::new();
    for layout in [TrialLayout::Duel, TrialLayout::Muster, TrialLayout::Hold] {
        for seed in [1, 2] {
            for swapped in [false, true] {
                let units: u64 = if layout == TrialLayout::Duel { 2 } else { 4 };
                let think_ms = u64::try_from(policy.cfg.minds.classes.row(core(class)).think_ms)?;
                let case = Case {
                    key: mush_roll::commerce::rating::CaseKey::new(
                        class,
                        CaseKind::Trial {
                            variant,
                            layout,
                            seed,
                            swapped,
                        },
                    )?,
                    physics: Trial::physics(policy, layout, core(class))?,
                    maximum_ms: super::super::plan::TRIAL_LIMIT_MS,
                    maximum_thinks: (super::super::plan::TRIAL_LIMIT_MS.div_ceil(think_ms) + 2)
                        * (units / 2),
                };
                let mut trial = Trial::open(policy, subject, &case)?;
                let result = loop {
                    if let Some(result) = trial.advance(u64::from(case.physics.native_ms))?.result {
                        break result;
                    }
                };
                let outcome = result.terminal().map(TrialEnd::outcome);
                let side = |candidate: bool| {
                    trial
                        .arena
                        .units
                        .iter()
                        .filter(|u| trial.candidates.contains(&u.id) == candidate)
                        .map(|u| {
                            format!(
                                "{}:{}‰@({},{})",
                                u.id.0,
                                trial.arena.health_permille(u.id),
                                u.pose.x_cm / 100,
                                u.pose.y_cm / 100
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(" ")
                };
                out.push(format!(
                    "{class:?} v{variant} {layout:?} seed={seed} swapped={swapped}: {outcome:?} end={:?} at_ms={} case={:?} ops_max={} refusals={} | mind [{}] | standard [{}]",
                    trial.arena.ended.map(|e| e.reason),
                    trial.arena.now_ms,
                    result.outcome(),
                    result.meter().max_ops(),
                    trial.refusals,
                    side(true),
                    side(false),
                ));
                outcomes.push(outcome);
            }
        }
    }
    Ok(outcomes)
}

/// Every House mind's every Trial case, printed; with no override, the two
/// combat doctrines (Meridiani, Coprates) must win some Class II trials but
/// not all. Safe Conduct is escort and transit content and is not held to it.
#[test]
#[ignore = "a genuine run of every Trial case (minutes); dispatch or by hand"]
fn house_minds_over_every_trial_case() -> TestResult {
    let policy = super::super::tests::frozen_policy()?;
    let only = std::env::var("HOUSE_ONLY").ok();
    for (name, source, envelope) in HOUSE {
        if only.as_deref().is_some_and(|o| o != name) {
            continue;
        }
        // Hand iteration: HOUSE_SOURCE names a file read in place of the shipped
        // source; HOUSE_CLASS (II|III) and HOUSE_VARIANT narrow the run.
        let owned = std::env::var("HOUSE_SOURCE")
            .ok()
            .map(std::fs::read_to_string)
            .transpose()?;
        let source = owned.as_deref().unwrap_or(source);
        let revision = revision(source, envelope)?;
        let subject = PreparedSubject::prepare(&revision, source, &policy.cfg.arena)?;
        let class_only = std::env::var("HOUSE_CLASS").ok();
        let mut class_ii = (0_usize, 0_usize);
        let variant_only = std::env::var("HOUSE_VARIANT").ok();
        for class in [RatingClass::II, RatingClass::III] {
            if class_only
                .as_deref()
                .is_some_and(|c| c != format!("{class:?}"))
            {
                continue;
            }
            for variant in 0..subject.variants().len() {
                if variant_only
                    .as_deref()
                    .is_some_and(|v| v != variant.to_string())
                {
                    continue;
                }
                let mut out = Vec::new();
                let outcomes = trials(&policy, &subject, class, u8::try_from(variant)?, &mut out)?;
                for line in out {
                    eprintln!("HOUSE {name} {line}");
                }
                let wins = outcomes
                    .iter()
                    .filter(|o| matches!(o, Some(TrialOutcome::Win)))
                    .count();
                eprintln!(
                    "HOUSE_SUMMARY {name} {class:?} v{variant}: {wins}/{} wins",
                    outcomes.len()
                );
                if class == RatingClass::II {
                    class_ii.0 += wins;
                    class_ii.1 += outcomes.len();
                }
            }
        }
        let held = owned.is_none()
            && class_only.is_none()
            && variant_only.is_none()
            && name != "safe_conduct";
        if held {
            check!(
                class_ii.0 > 0 && class_ii.0 < class_ii.1,
                "{name} wins some Class II trials, not all: {class_ii:?}"
            );
        }
    }
    Ok(())
}
