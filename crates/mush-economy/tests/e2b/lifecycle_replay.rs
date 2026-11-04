//! Doc 43 cut TS.5: a generated history's checkpoint boot equals its whole
//! replay.
//!
//! Random histories of the economy's own staff verbs (mint, pay, hold place,
//! capture and release, title, transfer, assign, unassign, lien and release),
//! with the checkpoint sweep fired at random points and the cadence small, so
//! checkpoints land mid-history and the RAM tail turns over. At the end the
//! store the history wrote is booted twice: as written, which resumes from the
//! newest checkpoint, and with every checkpoint document and the boot manifest
//! removed, which replays the whole journal. Both must equal each other and
//! the live store, snapshot for snapshot.

use std::collections::HashMap;

use mush_core::{GOD, ModuleBootData, ObjId, ObjKind};
use mush_testkit::TestWorld;
use proptest::prelude::*;
use smol_str::SmolStr;

use super::{NS, boot_j3, fold_into, has_error, store, sweep_at};

/// The RAM tail and the checkpoint cadence: small, so a short history crosses
/// both.
const TAIL: usize = 3;
const EVERY: u64 = 4;
const PEOPLE: u32 = 3;
const OBJECTS: u32 = 3;

#[derive(Clone, Debug)]
enum Step {
    Mint {
        to: u32,
        amount: i64,
    },
    Pay {
        from: u32,
        to: u32,
        amount: i64,
    },
    Place {
        account: u32,
        amount: i64,
        /// The beneficiary: only a hold placed for someone can be captured.
        beneficiary: Option<u32>,
    },
    Capture {
        hold: u32,
        to: u32,
        amount: i64,
    },
    ReleaseHold {
        hold: u32,
    },
    Title {
        object: u32,
        owner: u32,
    },
    Transfer {
        object: u32,
        to: u32,
    },
    Assign {
        object: u32,
        to: u32,
    },
    Unassign {
        object: u32,
    },
    Lien {
        object: u32,
        creditor: u32,
        amount: i64,
    },
    ReleaseLien {
        object: u32,
    },
    Sweep,
}

fn step() -> impl Strategy<Value = Step> {
    let person = 0..PEOPLE;
    let object = 0..OBJECTS;
    let amount = prop_oneof![Just(1i64), 1i64..=50, Just(500)];
    prop_oneof![
        (person.clone(), amount.clone()).prop_map(|(to, amount)| Step::Mint { to, amount }),
        (person.clone(), person.clone(), amount.clone()).prop_map(|(from, to, amount)| Step::Pay {
            from,
            to,
            amount
        }),
        (
            person.clone(),
            amount.clone(),
            prop::option::of(person.clone())
        )
            .prop_map(|(account, amount, beneficiary)| Step::Place {
                account,
                amount,
                beneficiary
            }),
        (1u32..6, person.clone(), amount.clone()).prop_map(|(hold, to, amount)| Step::Capture {
            hold,
            to,
            amount
        }),
        (1u32..6).prop_map(|hold| Step::ReleaseHold { hold }),
        (object.clone(), person.clone()).prop_map(|(object, owner)| Step::Title { object, owner }),
        (object.clone(), person.clone()).prop_map(|(object, to)| Step::Transfer { object, to }),
        (object.clone(), person.clone()).prop_map(|(object, to)| Step::Assign { object, to }),
        object.clone().prop_map(|object| Step::Unassign { object }),
        (object.clone(), person.clone(), amount).prop_map(|(object, creditor, amount)| {
            Step::Lien {
                object,
                creditor,
                amount,
            }
        }),
        object.prop_map(|object| Step::ReleaseLien { object }),
        Just(Step::Sweep),
    ]
}

/// The world a history runs in: the instrument, the faucet (account 0), three
/// people with an account each (1..=3) holding 5.00, and three things to title.
struct Cast {
    people: Vec<ObjId>,
    things: Vec<ObjId>,
}

/// The cast's objects, created in a fixed order so a fresh world gives them
/// the same ids: a reboot needs the principals and things the titles name, or
/// it freezes those titles as `principal-gone`.
fn objects(tw: &mut TestWorld) -> Cast {
    let world = tw.world_mut();
    let mut make = |kind: ObjKind, name: String| {
        world
            .create(kind, &name, GOD, Some(mush_core::LIMBO))
            .expect("an object")
    };
    let people = (0..PEOPLE)
        .map(|n| make(ObjKind::Player, format!("Person{n}")))
        .collect();
    let things = (0..OBJECTS)
        .map(|n| make(ObjKind::Thing, format!("thing{n}")))
        .collect();
    Cast { people, things }
}

fn cast(tw: &mut TestWorld) -> Cast {
    assert!(!has_error(&tw.run(GOD, "@instrument/register gold=2")));
    assert!(!has_error(&tw.run(GOD, "@account/system faucet=gold")));
    let c = objects(tw);
    for p in &c.people {
        assert!(!has_error(
            &tw.run(GOD, &format!("@account/open #{}=gold", p.0))
        ));
    }
    // Everyone starts funded, so a place, a pay and a capture can land early.
    for n in 1..=PEOPLE {
        assert!(!has_error(&tw.run(GOD, &format!("@econ/mint 0={n} 500"))));
    }
    c
}

/// Boot `kv` on a fresh world holding the same cast.
fn reboot(kv: Vec<(SmolStr, serde_json::Value)>, c: &Cast) -> TestWorld {
    let mut tw = boot_j3(TAIL, EVERY);
    let again = objects(&mut tw);
    assert_eq!(
        (&again.people, &again.things),
        (&c.people, &c.things),
        "the cast's ids"
    );
    tw.modules_loaded(HashMap::from([(
        SmolStr::new(NS),
        ModuleBootData {
            kv,
            ..ModuleBootData::default()
        },
    )]));
    tw
}

/// The staff command a step runs (refusals are part of the history too).
fn command(c: &Cast, step: &Step) -> Option<String> {
    let acct = |n: u32| n + 1;
    let person = |n: u32| c.people[n as usize].0;
    let thing = |n: u32| c.things[n as usize].0;
    Some(match *step {
        Step::Mint { to, amount } => format!("@econ/mint 0={} {amount}", acct(to)),
        Step::Pay { from, to, amount } => format!("@econ/pay {}={} {amount}", acct(from), acct(to)),
        Step::Place {
            account,
            amount,
            beneficiary,
        } => match beneficiary {
            Some(b) => format!("@hold/place {}={amount} for=#{}", acct(account), person(b)),
            None => format!("@hold/place {}={amount}", acct(account)),
        },
        Step::Capture { hold, to, amount } => format!("@hold/capture {hold}={} {amount}", acct(to)),
        Step::ReleaseHold { hold } => format!("@hold/release {hold}"),
        Step::Title { object, owner } => {
            format!("@asset/title #{}=#{}", thing(object), person(owner))
        }
        Step::Transfer { object, to } => {
            format!("@asset/transfer #{}=#{}", thing(object), person(to))
        }
        Step::Assign { object, to } => format!("@asset/assign #{}=#{}", thing(object), person(to)),
        Step::Unassign { object } => format!("@asset/unassign #{}", thing(object)),
        Step::Lien {
            object,
            creditor,
            amount,
        } => format!(
            "@asset/lien #{}=#{} {amount} gold",
            thing(object),
            person(creditor)
        ),
        Step::ReleaseLien { object } => format!("@asset/release #{}=1", thing(object)),
        Step::Sweep => return None,
    })
}

/// What a history reached: the verbs that succeeded, and whether its boot
/// resumed from a checkpoint.
#[derive(Default)]
struct Reach {
    succeeded: std::collections::BTreeSet<String>,
    resumed: bool,
}

/// Run `history`, then boot what it wrote both ways and compare.
fn run(history: &[Step]) -> Result<Reach, String> {
    let mut reach = Reach::default();
    let mut tw = boot_j3(TAIL, EVERY);
    let c = cast(&mut tw);
    let mut now = 100;
    for step in history {
        match command(&c, step) {
            Some(line) => {
                if !has_error(&tw.run(GOD, &line)) {
                    let verb = line.split_whitespace().next().unwrap_or_default();
                    reach.succeeded.insert(verb.to_owned());
                }
            }
            None => {
                now += 60;
                sweep_at(&mut tw, now);
            }
        }
    }
    let live = store(&tw).snapshot().clone();
    let kv = fold_into(Vec::new(), &mut tw);
    let checkpointed = kv.iter().any(|(k, _)| k.starts_with("checkpoint:"));

    let resumed = reboot(kv.clone(), &c);
    let whole_kv: Vec<(SmolStr, serde_json::Value)> = kv
        .into_iter()
        .filter(|(k, _)| !k.starts_with("checkpoint:") && k != mush_core::BOOT_MANIFEST_KEY)
        .collect();
    let whole = reboot(whole_kv, &c);

    let (r, w) = (store(&resumed), store(&whole));
    if r.is_frozen() || w.is_frozen() {
        return Err(format!(
            "a boot froze: resumed {}, whole {}",
            r.is_frozen(),
            w.is_frozen()
        ));
    }
    if checkpointed && r.boot_replay().and_then(|s| s.from_checkpoint).is_none() {
        return Err("a checkpoint was written but the boot did not resume from it".into());
    }
    if w.boot_replay().and_then(|s| s.from_checkpoint).is_some() {
        return Err("the whole boot resumed from a checkpoint".into());
    }
    if r.snapshot() != w.snapshot() {
        return Err("the checkpoint boot's snapshot differs from the whole replay's".into());
    }
    if *r.snapshot() != live {
        return Err("the booted snapshot differs from the live one".into());
    }
    if r.journal_len() != w.journal_len() {
        return Err(format!(
            "journal lengths {} and {}",
            r.journal_len(),
            w.journal_len()
        ));
    }
    reach.resumed = r.boot_replay().and_then(|s| s.from_checkpoint).is_some();
    Ok(reach)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

    #[test]
    fn a_checkpoint_boot_equals_the_whole_replay(history in prop::collection::vec(step(), 1..40)) {
        if let Err(e) = run(&history) {
            prop_assert!(false, "{}", e);
        }
    }
}

/// The histories are not vacuous: across a fixed sample, every verb succeeds
/// at least once and some boots resume from a checkpoint.
#[test]
fn the_histories_reach_every_verb_and_a_checkpoint() {
    use proptest::strategy::ValueTree;
    let mut runner = proptest::test_runner::TestRunner::deterministic();
    let strategy = prop::collection::vec(step(), 1..40);
    let mut succeeded = std::collections::BTreeSet::new();
    let mut resumed = 0;
    for _ in 0..48 {
        let h = strategy.new_tree(&mut runner).expect("draws").current();
        let reach = run(&h).expect("the property holds");
        succeeded.extend(reach.succeeded);
        resumed += usize::from(reach.resumed);
    }
    for verb in [
        "@econ/mint",
        "@econ/pay",
        "@hold/place",
        "@hold/capture",
        "@hold/release",
        "@asset/title",
        "@asset/transfer",
        "@asset/assign",
        "@asset/unassign",
        "@asset/lien",
        "@asset/release",
    ] {
        assert!(
            succeeded.contains(verb),
            "{verb} never succeeded: {succeeded:?}"
        );
    }
    assert!(resumed > 0, "no boot resumed from a checkpoint");
}
