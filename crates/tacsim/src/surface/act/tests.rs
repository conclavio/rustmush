#![cfg(test)]

use super::*;

/// Every refusal's word is in the table and its message is the table's.
#[test]
fn sm3b_every_refusal_word_has_a_message() {
    // Doc 20 cut SM.11: one sample of every variant, shared with the plan's refusal table.
    let refusals = refusal_samples();
    let mut words: Vec<&str> = refusals.iter().map(Refusal::word).collect();
    for refusal in &refusals {
        assert!(
            REFUSAL_CODES
                .iter()
                .any(|(w, m)| *w == refusal.word() && *m == refusal.code()),
            "{refusal:?}"
        );
        assert!(
            !refusal.line().contains('{'),
            "{refusal:?}: {}",
            refusal.line()
        );
    }
    words.sort_unstable();
    words.dedup();
    assert_eq!(words.len(), REFUSAL_CODES.len(), "one word per line");
    assert_eq!(
        Refusal::Far {
            n: 1,
            m: 20,
            ramp: "stern ramp".to_owned()
        }
        .line(),
        "Rig 1 is about 20 m from the stern ramp; it climbs aboard from beside it."
    );
    assert_eq!(code_of_word("nonsense"), "that rig cannot be read");
    assert_eq!(Refusal::NothingNew.line(), "You learn nothing new here.");
    assert_eq!(Refusal::NothingNew.code(), "you learn nothing new here");
    // Doc 20 cut SM.4b: 24 → 33 words; the drive's lines verbatim. Doc 20 cut SM.7: 33 → 49
    // (mining's sixteen). Doc 20 cut SM.8: 49 → 55 (unloading's six), 55 → 56 at the lead's
    // pre-commit round (item 4: `unload_no_stock` — a named tonnage past the hopper's stock is
    // no longer told as not fitting the hold). Doc 20 cut SM.11: 56 → 63 (the fill plan's
    // seven prepare refusals); 63 → 64 at the lead's pre-commit round (item 1:
    // `unload_markers`, the queued unload's start while the hopper's unload log is full).
    assert_eq!(REFUSAL_CODES.len(), 64);
    assert_eq!(
        Refusal::PlanLive { n: 1, no: 2 }.code(),
        "that rig is working a plan"
    );
    assert_eq!(
        Refusal::UnloadNoFit {
            n: 1,
            q: 9,
            fit: 3,
            ore: "Metal ore".to_owned(),
        }
        .line(),
        "9 t do not fit: rig 1 can unload 3 t of Metal ore now — +tv/1 unload 3, or +tv/1 \
         unload with no number."
    );
    assert_eq!(
        Refusal::UnloadNoStock {
            n: 1,
            fit: 4,
            ore: "ice ore".to_owned(),
        }
        .line(),
        "Rig 1's hopper holds only 4 t of ice ore: +tv/1 unload 4, or +tv/1 unload with no \
         number."
    );
    assert_eq!(
        Refusal::UnloadNoStock {
            n: 1,
            fit: 4,
            ore: String::new(),
        }
        .code(),
        "the hopper holds less than that"
    );
    assert_eq!(
        Refusal::UnloadChanged { n: 2 }.code(),
        "the hopper or the hold's room changed; preview it again"
    );
    assert_eq!(
        Refusal::NoFit {
            n: 1,
            m: 4,
            fit: 3,
            bound: Bound::Hopper,
            fam: String::new(),
        }
        .line(),
        "4 cycles do not fit: the hopper is full after the third — +tv/1 mine 3, or +tv/1 \
         mine with no number."
    );
    assert_eq!(Refusal::Claimed.line(), "That block is under claim.");
    assert_eq!(
        Refusal::Locked.line(),
        "Another rig is working that pocket."
    );
    assert_eq!(
        Refusal::NoWayHome {
            n: 1,
            target: "C117".to_owned(),
            ramp: "stern ramp".to_owned()
        }
        .line(),
        "Rig 1 could not get back to the stern ramp from C117, so it stays where it is."
    );
    assert_eq!(
        Refusal::Winding {
            n: 1,
            target: "field C-NW".to_owned(),
            needed: 33,
            cap: 32,
        }
        .line(),
        "The way to field C-NW winds too much to drive in one leg (32 waypoints at most). \
         Drive to a nearer point first."
    );
    // Doc 20 cut SM.7 (reviewer F7): the shared word speaks of an act, never a drive.
    assert_eq!(
        Refusal::Unplanned.code(),
        "that act was not planned; preview it again"
    );
    assert_eq!(
        Refusal::Moved.code(),
        "that rig has moved since the drive was planned"
    );
    assert_eq!(
        Refusal::LegShort {
            n: 2,
            now: "2".to_owned(),
            need: "3".to_owned(),
            ramp: "stern ramp".to_owned()
        }
        .line(),
        "Rig 2 has 2 % charge and the drive to the stern ramp needs 3 %; it cannot come in \
         on its own."
    );
}

/// **The start's `act` words parse** (doc 20 cut SM.4a, D23; the brief's reviewer finding
/// F6): every wire word round-trips through `act_of_word`/`wire_word`; `sound_quiet` is a
/// quiet sounding; `probe` (the retired test kind) and `survey` (the verb's alias only) do
/// not parse; a sounding's quiet flag is spelled, every other kind's ignored.
#[test]
fn sm4a_the_wire_words_parse() {
    // Doc 20 cut SM.7: `mine_stop` is a control, no act (`sm7_the_wire_words_parse`).
    for word in ACT_WIRE_WORDS
        .into_iter()
        .filter(|word| *word != "mine_stop" && !word.starts_with("plan_"))
    {
        let (kind, quiet) = act_of_word(word).unwrap_or_else(|| panic!("{word}"));
        assert_eq!(wire_word(kind, quiet), word);
    }
    assert_eq!(act_of_word("sound_quiet"), Some((ActKind::Sound, true)));
    assert_eq!(act_of_word("sound"), Some((ActKind::Sound, false)));
    assert_eq!(act_of_word("scan"), Some((ActKind::Scan, false)));
    for bad in ["probe", "survey", "quiet", "", "Scan"] {
        assert_eq!(act_of_word(bad), None, "{bad:?}");
    }
    assert_eq!(wire_word(ActKind::Scan, true), "scan");
    let mut sorted = ACT_WIRE_WORDS;
    sorted.sort_unstable();
    assert_eq!(sorted, ACT_WIRE_WORDS, "the Choice is sorted");
    // The start's parameter gate refuses the retired probe with the message naming the six.
    let parameters = |act: &str| {
        json!({ "tv": "900", "act": act, "ordinal": 1 })
            .as_object()
            .cloned()
            .unwrap_or_default()
    };
    let refused = act_param(&parameters("probe")).expect_err("probe is retired");
    assert!(
        refused.to_string().contains(
            "a surface act names launch, recover, dump, scan, sound, sound_quiet, drive, \
             mine, mine_stop, unload, plan_file, plan_clear, plan_strike or plan_end"
        ),
        "{refused}"
    );
    assert_eq!(
        act_param(&parameters("sound_quiet")).ok(),
        Some(StartWord::Act {
            kind: ActKind::Sound,
            quiet: true
        })
    );
}

/// **Tenure reads the block, not the family** (doc 20 cut SM.7; the brief's 3.3 item 5,
/// reviewer F5, decisions R5 and R20) — `authored_of` over a synthetic `SiteDef` and
/// `tenure_verdict` over hand-built `ClaimRecord`s, with **industry's own
/// `extraction_authorized`** as the authorizer: on a bare `TestWorld` with no licence
/// instrument named, its tenure half passing reads `Err(NoLicenceInstrument)` (the licence half
/// is the world tests'), so the holder rule here is industry's, not a model of it. Commons with a
/// claim → Ok (no claim read); `(Held, None)` → open → Ok; `(Held, Some)` with no claim →
/// `Claimed`; open with a live claim of another → `Claimed`; open with a term-expired unlapsed
/// claim → Ok; open with a lapsed claim → Ok; open with an uncomparable term → `Claimed`; the
/// holder of a `Suspended` claim → `Claimed`; the holder of an uncontested in-term claim → Ok.
#[test]
fn sm7_mine_tenure_reads_the_block_not_the_family() {
    use crate::port::def::{BlockDef, SiteDef, SiteTenureDef, Tenure, Zone};
    use mush_core::{ClockRef, ClockStamp};
    use mush_industry::{
        Accessibility, BandId, ClaimId, ClaimRecord, ClaimSubject, ContestState, CurvePoint,
        Exclusivity, ExtractionRefusal, OreDeposit, OreDepositId, Transferability, YieldCurve,
    };
    use mush_org::PrincipalId;
    let cid = |text: &str| CanonicalId::new(text).unwrap_or_else(|_| panic!("{text}"));
    let block = |tenure: Option<Tenure>, holder: Option<&str>| BlockDef {
        tenure,
        holder: holder.map(cid),
        ..BlockDef::default()
    };
    let site = SiteDef {
        name: "Test Flank".to_owned(),
        issuer: cid("freeport_registry"),
        zone: Zone::Core,
        tenure: SiteTenureDef {
            default: Tenure::Open,
        },
        blocks: [
            ("A".to_owned(), block(Some(Tenure::Commons), None)),
            ("F".to_owned(), block(Some(Tenure::Held), Some("occator"))),
            ("G".to_owned(), block(Some(Tenure::Held), None)),
        ]
        .into_iter()
        .collect(),
        papers: std::collections::BTreeMap::new(),
        yard: None,
    };
    assert_eq!(authored_of(&site, 'A'), Authored::Commons);
    assert_eq!(authored_of(&site, 'F'), Authored::Held);
    assert_eq!(
        authored_of(&site, 'G'),
        Authored::Open,
        "a holder-less held block"
    );
    assert_eq!(authored_of(&site, 'C'), Authored::Open, "the default");
    let stamp = |at: i64, generation: u64| ClockStamp {
        clock: ClockRef::WorldIc { generation },
        at,
    };
    let now = stamp(1_000, 1);
    let me = ObjId(42);
    let other = ObjId(43);
    let body = OreDeposit::new(
        OreDepositId(7),
        ObjId(7),
        cid("metal_ore"),
        mush_inventory::Grade::new(0).unwrap_or_else(|_| panic!("a grade")),
        Accessibility::new(100).unwrap_or_else(|_| panic!("accessible")),
        BandId::new("surface").unwrap_or_else(|_| panic!("a band")),
        BandId::new("surface").unwrap_or_else(|_| panic!("a band")),
        YieldCurve::new(vec![CurvePoint {
            cumulative: 0,
            per_cycle: 1,
        }])
        .unwrap_or_else(|_| panic!("a curve")),
        mush_inventory::EndpointRef(VaultId(9)),
        40,
    )
    .unwrap_or_else(|_| panic!("a deposit"));
    let claim = |id: u64, holder: ObjId, term: ClockStamp| {
        ClaimRecord::new(
            ClaimId(id),
            ClaimSubject::Deposit {
                deposit: OreDepositId(7),
            },
            PrincipalId::Object(ObjId(1)),
            PrincipalId::Object(holder),
            ObjId(900),
            Exclusivity::Exclusive,
            1,
            term,
            Transferability::Transferable,
        )
        .unwrap_or_else(|_| panic!("a claim"))
    };
    let world = mush_testkit::TestWorld::new();
    let authorizes = |claim: &ClaimRecord| {
        matches!(
            mush_industry::extraction_authorized(
                world.world(),
                me,
                &PrincipalId::Object(me),
                &body,
                body.initial_reserve(),
                std::slice::from_ref(claim),
                None,
                &now,
            ),
            Ok(_) | Err(ExtractionRefusal::NoLicenceInstrument)
        )
    };
    let verdict = |authored: Authored, claims: &[ClaimRecord]| {
        tenure_verdict(authored, claims, &now, &authorizes)
    };
    let live_other = claim(1, other, stamp(5_000, 1));
    assert_eq!(
        verdict(Authored::Commons, std::slice::from_ref(&live_other)),
        Ok(())
    );
    assert_eq!(verdict(Authored::Open, &[]), Ok(()));
    assert_eq!(verdict(Authored::Held, &[]), Err(TenureBar::Claimed));
    assert_eq!(
        verdict(Authored::Open, std::slice::from_ref(&live_other)),
        Err(TenureBar::Claimed)
    );
    let expired = claim(2, other, stamp(500, 1));
    assert!(!claim_live(&expired, &now));
    assert_eq!(
        verdict(Authored::Open, &[expired]),
        Ok(()),
        "term past, not lapsed"
    );
    let lapsed = claim(3, other, stamp(5_000, 1))
        .lapsed(stamp(900, 1))
        .unwrap_or_else(|_| panic!("lapses"));
    assert_eq!(verdict(Authored::Open, &[lapsed]), Ok(()));
    let uncomparable = claim(4, other, stamp(5_000, 2));
    assert!(
        claim_live(&uncomparable, &now),
        "a claim we cannot date bars"
    );
    assert_eq!(
        verdict(Authored::Open, &[uncomparable]),
        Err(TenureBar::Claimed)
    );
    let suspended = claim(5, me, stamp(5_000, 1))
        .with_contest_state(ContestState::Suspended {
            by: PrincipalId::Object(ObjId(1)),
        })
        .unwrap_or_else(|_| panic!("suspends"));
    assert_eq!(
        verdict(Authored::Open, &[suspended]),
        Err(TenureBar::Claimed),
        "a suspended claim bars its holder too"
    );
    let mine = claim(6, me, stamp(5_000, 1));
    assert_eq!(verdict(Authored::Open, std::slice::from_ref(&mine)), Ok(()));
    assert_eq!(
        verdict(Authored::Held, &[mine]),
        Ok(()),
        "a holder's own claim"
    );
}

/// **The start's nine words parse** (doc 20 cut SM.7; the brief's 3.4): `mine` is an act (the
/// `Mine` kind), `mine_stop` a [`StartWord::MineStop`]; every other word's [`start_word`] is its
/// [`act_of_word`]; the Choice is sorted and holds nine; the cycle and stop keys form.
#[test]
fn sm7_the_wire_words_parse() {
    // Doc 20 cut SM.8: nine → ten with `unload`; doc 20 cut SM.11: ten → fourteen with the
    // plan's four controls.
    assert_eq!(ACT_WIRE_WORDS.len(), 14);
    let mut sorted = ACT_WIRE_WORDS;
    sorted.sort_unstable();
    assert_eq!(sorted, ACT_WIRE_WORDS);
    assert_eq!(act_of_word("mine"), Some((ActKind::Mine, false)));
    assert_eq!(wire_word(ActKind::Mine, false), "mine");
    assert_eq!(act_of_word("mine_stop"), None, "no act");
    assert_eq!(start_word("mine_stop"), Some(StartWord::MineStop));
    for word in ACT_WIRE_WORDS {
        let parsed = start_word(word).unwrap_or_else(|| panic!("{word}"));
        match parsed {
            StartWord::Act { kind, quiet } => {
                assert_eq!(act_of_word(word), Some((kind, quiet)));
            }
            StartWord::MineStop => assert_eq!(word, "mine_stop"),
            StartWord::PlanFile => assert_eq!(word, "plan_file"),
            StartWord::PlanClear => assert_eq!(word, "plan_clear"),
            StartWord::PlanStrike => assert_eq!(word, "plan_strike"),
            StartWord::PlanEnd => assert_eq!(word, "plan_end"),
        }
    }
    for word in ["plan_file", "plan_clear", "plan_strike", "plan_end"] {
        assert_eq!(act_of_word(word), None, "{word} is no act");
    }
    assert_eq!(start_word("drill"), None, "the alias is the verb's only");
    assert_eq!(
        cycle_key(ObjId(900), 3, 2).map(|k| k.as_str().to_owned()),
        Some("tacsim.surface/900/3/cycle/2".to_owned())
    );
    assert_eq!(
        stop_key(ObjId(900), 3).map(|k| k.as_str().to_owned()),
        Some("tacsim.surface/900/3/stop".to_owned())
    );
    let parameters = |cycle: Value| {
        json!({ "tv": "900", "ordinal": 1, "cycle": cycle })
            .as_object()
            .cloned()
            .unwrap_or_default()
    };
    assert_eq!(cycle_param(&parameters(json!(1))).ok(), Some(Some(1)));
    assert_eq!(cycle_param(&parameters(json!(8))).ok(), Some(Some(8)));
    for bad in [json!(0), json!(9), json!("1"), json!(-1)] {
        assert!(cycle_param(&parameters(bad.clone())).is_err(), "{bad}");
    }
    let none = json!({ "tv": "900", "ordinal": 1 })
        .as_object()
        .cloned()
        .unwrap_or_default();
    assert_eq!(cycle_param(&none).ok(), Some(None));
}

/// **An implied launch settles at the launch half, never at the due instant** (doc 20 cut
/// SM.4a, F1, D22): with a battery's figures and a charge under capacity in the bay,
/// `spent_after_launch` leaves `charge_at(c, started + 300) − cost` out of the bay stamped
/// at the due instant — strictly less than `charge_at(c, due) − cost`, which SM.3b's
/// `spent` would credit — and equal to `charge_for_act` at the start less the cost.
#[test]
fn sm4a_spent_after_launch_credits_the_bay_only_for_the_launch_half() {
    let battery = crate::definitions::tools::tools_table()
        .unwrap_or_else(|e| panic!("{e}"))
        .values()
        .find_map(ToolDef::battery)
        .copied()
        .unwrap_or_else(|| panic!("a battery row"));
    let charge = Charge {
        value_cu: 20_000,
        at_ic: 1_000,
        in_bay: true,
    };
    let act = Act {
        ordinal: 1,
        kind: ActKind::Scan,
        operator: ObjId(42),
        hull: ObjId(500),
        slot: 1,
        site: CanonicalId::new("ahuna_port").unwrap_or_else(|_| panic!("an id")),
        started_ic: 1_000,
        due_ic: 1_900,
        reserved_cu: 1_000,
        params: ActParams::None {},
        by: None,
        seal: None,
    };
    let settled = spent_after_launch(&charge, &act, &battery).unwrap_or_else(|e| panic!("{e:?}"));
    let launched = charge_at(&charge, 1_300, &battery);
    assert_eq!(
        settled,
        Charge {
            value_cu: launched - 1_000,
            at_ic: 1_900,
            in_bay: false
        }
    );
    assert!(settled.value_cu < charge_at(&charge, 1_900, &battery) - 1_000);
    assert_eq!(
        settled.value_cu,
        charge_for_act(&charge, act.started_ic, true, &battery) - act.reserved_cu
    );
    let ruinous = Act {
        reserved_cu: launched + 1,
        ..act
    };
    assert!(spent_after_launch(&charge, &ruinous, &battery).is_err());
}

/// **An implied launch refused `Moved` leaves the bay at the launch half, spending nothing**
/// (the SM.4a fixer round's finding F1): the charge is `charge_at(c, started + 300)` out of
/// the bay at the due instant — never the bay's credit for the whole act, never less the
/// act's cost; without the battery's row the stored value stands (no credit), still out of
/// the bay; a charge already out of the bay is unchanged.
#[test]
fn sm4a_a_moved_implied_launch_leaves_the_bay_at_the_launch_half() {
    let battery = crate::definitions::tools::tools_table()
        .unwrap_or_else(|e| panic!("{e}"))
        .values()
        .find_map(ToolDef::battery)
        .copied()
        .unwrap_or_else(|| panic!("a battery row"));
    let charge = Charge {
        value_cu: 20_000,
        at_ic: 1_000,
        in_bay: true,
    };
    let act = Act {
        ordinal: 1,
        kind: ActKind::Sound,
        operator: ObjId(42),
        hull: ObjId(500),
        slot: 1,
        site: CanonicalId::new("ahuna_port").unwrap_or_else(|_| panic!("an id")),
        started_ic: 1_000,
        due_ic: 2_500,
        reserved_cu: 2_000,
        params: ActParams::None {},
        by: None,
        seal: None,
    };
    let left = left_after_launch(&charge, &act, Some(&battery));
    assert_eq!(
        left,
        Charge {
            value_cu: charge_at(&charge, 1_300, &battery),
            at_ic: 2_500,
            in_bay: false
        }
    );
    assert!(
        left.value_cu > charge.value_cu,
        "the launch half's credit shows"
    );
    assert!(left.value_cu < charge_at(&charge, 2_500, &battery));
    assert_eq!(
        left_after_launch(&charge, &act, None),
        Charge {
            value_cu: 20_000,
            at_ic: 2_500,
            in_bay: false
        }
    );
    let out = Charge {
        in_bay: false,
        ..charge
    };
    assert_eq!(left_after_launch(&out, &act, Some(&battery)), out);
}

/// The keys of both actions form.
#[test]
fn sm3b_the_act_and_done_keys_form() {
    assert_eq!(
        act_key(ObjId(900), 3).map(|k| k.as_str().to_owned()),
        Some("tacsim.surface/900/3/act".to_owned())
    );
    assert_eq!(
        done_key(ObjId(900), 3).map(|k| k.as_str().to_owned()),
        Some("tacsim.surface/900/3/done".to_owned())
    );
    // Doc 20 cut SM.8 (decision R15): the queued unload's start has its own key.
    assert_eq!(
        queued_key(ObjId(900), 3).map(|k| k.as_str().to_owned()),
        Some("tacsim.surface/900/3/queued".to_owned())
    );
    // Doc 20 cut SM.11 (the brief's 3.6, D9, D17): the plan's keys.
    let text = |key: Option<IdempotencyKeyText>| key.map(|k| k.as_str().to_owned());
    assert_eq!(
        text(plan_file_key(ObjId(900), 2)),
        Some("tacsim.surface/900/plan/2/file".to_owned())
    );
    assert_eq!(
        text(plan_clear_key(ObjId(900), 2, 7)),
        Some("tacsim.surface/900/plan/2/clear/7".to_owned())
    );
    assert_eq!(
        text(plan_strike_key(ObjId(900), 2, 11)),
        Some("tacsim.surface/900/plan/2/strike/11".to_owned())
    );
    assert_eq!(
        text(plan_end_key(ObjId(900), 2)),
        Some("tacsim.surface/900/plan/2/end".to_owned())
    );
    assert_eq!(
        text(plan_step_key(ObjId(900), 12, 3, "drive")),
        Some("tacsim.surface/900/12/plan/3/drive".to_owned())
    );
    assert_eq!(
        text(idle_key(ObjId(900), 12)),
        Some("tacsim.surface/900/12/idle".to_owned())
    );
    assert_ne!(
        text(plan_clear_key(ObjId(900), 2, 7)),
        text(plan_strike_key(ObjId(900), 2, 7)),
        "a clear and a strike never share a key"
    );
}
/// **A plan's filing leaves the slot once it landed or the rig moved on** (doc 20 cut SM.11;
/// the brief's 3.8, the reviewer's F11): `prune_mine_orders` keeps a filing while
/// `plans < no` and the record's ordinal is the one it was planned at; drops it once the filing
/// landed (`plans ≥ no`) or the ordinal moved; a rig with no record keeps a first filing
/// planned at ordinal 0 and drops one planned at another.
#[test]
fn sm11_the_prune_drops_a_landed_or_overtaken_filing() {
    let mut tw = mush_testkit::TestWorld::with_modules(vec![Box::new(
        crate::TacsimRuleset::with_config(crate::fixture::config()),
    )]);
    let w = &mut tw.engine_mut().world;
    let filing = |no: u32, ordinal: u32| {
        ActOrder::Plan(Box::new(PlanOrder {
            actor: ObjId(42),
            ordinal,
            no,
            list: "D".to_owned(),
            family: None,
            compiled: crate::surface::plan::Compiled {
                scheme: crate::surface::address::Scheme::SHIPPED,
                list: vec!["D".to_owned()],
                family: OreFamily::Metals,
                steps: vec![crate::surface::plan::PlanStep::Block { letter: 'D' }],
                est_secs: 1,
                step_secs: vec![1],
                far: Vec::new(),
            },
        }))
    };
    let record = |tv: u32, ordinal: u32, plans: u32| {
        let mut record = RigRecord::fresh_unread(ObjId(tv));
        record.ordinal = ordinal;
        record.plans = plans;
        record
    };
    let mut run = |orders: Vec<(u32, ActOrder)>, records: &[RigRecord]| {
        if let Some(slot) = w.module_state_mut::<ActOrders>() {
            slot.0 = orders.into_iter().map(|(tv, o)| (ObjId(tv), o)).collect();
        }
        let dropped = prune_mine_orders(w, records);
        let kept: Vec<u32> = w
            .module_state::<ActOrders>()
            .map(|slot| slot.0.keys().map(|tv| tv.0).collect())
            .unwrap_or_default();
        (dropped, kept)
    };
    assert_eq!(
        run(vec![(1, filing(1, 3))], &[record(1, 3, 0)]),
        (0, vec![1]),
        "not yet landed"
    );
    assert_eq!(
        run(vec![(1, filing(1, 3))], &[record(1, 3, 1)]),
        (1, Vec::new()),
        "the filing landed: plans ≥ no"
    );
    assert_eq!(
        run(vec![(1, filing(2, 3))], &[record(1, 4, 1)]),
        (1, Vec::new()),
        "the rig's orders moved on"
    );
    assert_eq!(
        run(vec![(1, filing(1, 0)), (2, filing(1, 2))], &[]),
        (1, vec![1]),
        "no record: a first filing at ordinal 0 stays, another goes"
    );
}

/// **A plan's start names its plan and step** (doc 20 cut SM.11; the brief's 3.6): the
/// parameter rule, word by word.
#[test]
fn sm11_a_plan_start_names_its_plan_and_step() {
    let params = |value: Value| value.as_object().cloned().unwrap_or_default();
    let act = |word: &str| start_word(word).unwrap_or_else(|| panic!("{word}"));
    let ok = |word: &str, value: Value| by_params(act(word), &params(value)).is_ok();
    // A plan's step: drive, scan, sound, mine, recover and plan_end, with plan and step.
    for word in ["drive", "scan", "sound", "mine", "recover", "plan_end"] {
        assert!(
            ok(word, json!({ "by": "plan", "plan": 1, "step": 0 })),
            "{word}"
        );
        assert!(
            !ok(word, json!({ "by": "plan", "plan": 1 })),
            "{word}: no step"
        );
        assert!(
            !ok(word, json!({ "by": "plan", "step": 0 })),
            "{word}: no plan"
        );
    }
    for word in ["launch", "dump", "unload", "mine_stop", "plan_file"] {
        assert!(
            !ok(word, json!({ "by": "plan", "plan": 1, "step": 0 })),
            "{word}"
        );
    }
    // The idle return: a recover only, with neither.
    assert!(ok("recover", json!({ "by": "idle" })));
    assert!(!ok("recover", json!({ "by": "idle", "plan": 1 })));
    assert!(!ok("scan", json!({ "by": "idle" })));
    // The controls.
    assert!(ok("plan_file", json!({})));
    assert!(!ok("plan_file", json!({ "plan": 1 })));
    assert!(ok("plan_clear", json!({ "plan": 1 })));
    assert!(ok("plan_strike", json!({ "plan": 1 })));
    assert!(!ok("plan_clear", json!({})));
    assert!(!ok("plan_end", json!({})), "an end is the plan's");
    // A hand start carries none; malformed values refuse.
    assert!(ok("scan", json!({})));
    assert!(!ok("scan", json!({ "plan": 1 })));
    assert!(!ok("drive", json!({ "by": "hand" })));
    assert!(!ok("drive", json!({ "by": "plan", "plan": 0, "step": 0 })));
    assert!(!ok("drive", json!({ "by": "plan", "plan": 1, "step": 65 })));
}

/// **An unload's start names how it was planned** (doc 20 cut SM.8; decision R15): `how` reads
/// the three words and nothing else; `qty` reads whole tonnes `1..=99`; `unload` is an act
/// word (the `Unload` kind) and its wire word round-trips.
#[test]
fn sm8_an_unload_start_names_how() {
    let parameters = |extra: Value| {
        let mut map = json!({ "tv": "900", "act": "unload", "ordinal": 4 })
            .as_object()
            .cloned()
            .unwrap_or_default();
        if let Some(more) = extra.as_object() {
            map.extend(more.clone());
        }
        map
    };
    assert_eq!(act_of_word("unload"), Some((ActKind::Unload, false)));
    assert_eq!(wire_word(ActKind::Unload, false), "unload");
    for (word, how) in [
        ("queued", UnloadHow::Queued),
        ("whole", UnloadHow::Whole),
        ("part", UnloadHow::Part),
    ] {
        assert_eq!(how_of(&parameters(json!({ "how": word }))), Some(how));
    }
    for bad in [json!("all"), json!(1), json!("Whole")] {
        assert_eq!(how_of(&parameters(json!({ "how": bad }))), None, "{bad}");
    }
    assert_eq!(how_of(&parameters(json!({}))), None);
    assert_eq!(qty_of(&parameters(json!({ "qty": 3 }))), Some(3));
    assert_eq!(qty_of(&parameters(json!({ "qty": 99 }))), Some(99));
    for bad in [json!(0), json!(100), json!("3"), json!(-1)] {
        assert_eq!(qty_of(&parameters(json!({ "qty": bad }))), None, "{bad}");
    }
}

/// **The unload weighs with the plan's table** (doc 20 cut SM.8; decision R16): the goods table
/// a hold plan carries (`plan_goods` of the Dray-P's `stowage_plan`) is the compiled-in table
/// the layer's store loads (`goods_table`, `TacsimStore::goods`), so the room `unload_room`
/// computes through `hold_unreserved_with(plan_goods(..))` equals `hold_unreserved`'s on the
/// shipped tables, and the preview, the offer, the prepare and the finish weigh alike. (No test
/// seam sets the store's table apart from the compiled one, so the unequal-table refusal is
/// asserted by construction only.)
#[test]
fn sm8_the_unload_weighs_with_the_plans_table() {
    let pack = crate::pack::load_pack(crate::definitions::content::DRAY_P_JSON)
        .unwrap_or_else(|e| panic!("{e:?}"));
    let id =
        CanonicalId::new(crate::definitions::content::DRAY_P).unwrap_or_else(|_| panic!("an id"));
    let logistics = match pack.entry(&id) {
        Some(crate::pack::PackEntry::Unit(spec)) => spec
            .logistics
            .clone()
            .unwrap_or_else(|| panic!("logistics")),
        _ => panic!("the Dray-P unit"),
    };
    let plan = crate::port::market::stowage_plan(&logistics).unwrap_or_else(|| panic!("a plan"));
    let table = crate::port::market::plan_goods(&plan).unwrap_or_else(|| panic!("a table"));
    let store = crate::definitions::goods::goods_table().unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(*table, store, "the plan's table is the store's");
    let mut other = plan.clone();
    other.goods = "00".repeat(32);
    assert!(
        crate::port::market::plan_goods(&other).is_none(),
        "another digest reads dark"
    );
    let ore = CanonicalId::new("metal_ore").unwrap_or_else(|_| panic!("an id"));
    assert!(
        crate::port::capacity::compartment_for(
            table,
            &crate::port::market::plan_compartments(&plan),
            &ore
        )
        .is_some(),
        "the Dray-P hold has one ore compartment"
    );
}

/// **A load line names its goods as a list** (the lead's pre-commit round, item 5): one good
/// reads as itself (byte-identical to the old join), two join with " and ", three or more
/// with ", " and a last " and " — so goods v3's comma-bearing labels never read as more goods.
#[test]
fn sm8_a_load_line_names_its_goods_as_a_list() {
    let words =
        |items: &[&str]| -> Vec<String> { items.iter().map(|item| (*item).to_owned()).collect() };
    assert_eq!(and_list(&[]), "");
    assert_eq!(
        and_list(&words(&["8 t of ice ore, run-of-mine"])),
        "8 t of ice ore, run-of-mine"
    );
    assert_eq!(
        and_list(&words(&[
            "8 t of ice ore, run-of-mine",
            "4 t of metal ore, run-of-mine"
        ])),
        "8 t of ice ore, run-of-mine and 4 t of metal ore, run-of-mine"
    );
    assert_eq!(
        and_list(&words(&["1 t of a", "2 t of b", "3 t of c"])),
        "1 t of a, 2 t of b and 3 t of c"
    );
}
