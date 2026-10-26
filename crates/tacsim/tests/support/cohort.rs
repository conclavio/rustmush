//! **The cost lane's world-free pieces** (doc 20 cut SM.12c-lane; doc 20 §11's Tick row, §13's
//! SM.12 row; doc 20a §5.3): the percentile helpers and the windows, the act-kind classifier's
//! pure core, the write-gate table [`WRITE_GATES`] with each ceiling's [`Basis`], the census table
//! [`CENSUS`], the leak guard's core [`unclassified`], the lever table the lane-diff pin reads,
//! the report's serde shape and its writer, and a copy of `harness.rs`'s `report_path`.
//!
//! Nothing here reads a world, so the module needs no state-slot row: `tests/cost_lane.rs` owns
//! every world reach and supplies the two facts only a world knows — the rig record's act in
//! flight (a finish's kind) and the driver's label (a claim's word, a normal-lane command's).

#![allow(dead_code)]

use serde::Serialize;
use serde_json::Value;

// ---------------------------------------------------------------------------
// Percentiles and windows
// ---------------------------------------------------------------------------

/// **The nearest-rank percentile** of an ascending sample, per mille: index `⌈n × p / 1000⌉ − 1`,
/// saturating at both ends; `0` for an empty sample.
#[must_use]
pub fn pct(sorted: &[u64], per_mille: u32) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let n = sorted.len();
    let rank = (n.saturating_mul(per_mille as usize)).div_ceil(1_000);
    let at = rank.saturating_sub(1).min(n - 1);
    sorted.get(at).copied().unwrap_or(0)
}

/// The sample sorted ascending.
#[must_use]
pub fn sorted(samples: &[u64]) -> Vec<u64> {
    let mut out = samples.to_vec();
    out.sort_unstable();
    out
}

/// The median (nearest rank).
#[must_use]
pub fn p50(samples: &[u64]) -> u64 {
    pct(&sorted(samples), 500)
}

/// The 99th percentile (nearest rank).
#[must_use]
pub fn p99(samples: &[u64]) -> u64 {
    pct(&sorted(samples), 990)
}

/// The largest sample, `0` for none.
#[must_use]
pub fn max(samples: &[u64]) -> u64 {
    samples.iter().copied().max().unwrap_or(0)
}

/// **The p99 of each of `n` equal consecutive windows** of `samples` (the last takes the
/// remainder) — reported beside the whole span's figure, never the gate's statistic.
#[must_use]
pub fn windows(samples: &[u64], n: usize) -> Vec<u64> {
    if n == 0 || samples.is_empty() {
        return Vec::new();
    }
    let size = samples.len().div_ceil(n).max(1);
    samples.chunks(size).map(p99).collect()
}

// ---------------------------------------------------------------------------
// The gate table
// ---------------------------------------------------------------------------

/// **Where a ceiling comes from** (doc 20 §11's Tick row; the lead's pin-vs-target rule).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Basis {
    /// A completion within §11's "≤ 8 writes a completion": the ceiling is 8.
    DocCompletion,
    /// Any other act within §11's "≤ 3 for any other act": the ceiling is 3.
    DocOther,
    /// A figure an earlier cut measured and handed forward ("do not trim"), its test still
    /// pinning it: `handed` is that cut's figure, the ceiling the largest this lane constructs or
    /// the handed one where the handed variant is rig-only.
    Pinned {
        /// The cut that handed it.
        cut: &'static str,
        /// The test that pins it.
        test: &'static str,
        /// The figure it handed.
        handed: usize,
    },
    /// SM.12c's own constructed maximum over its target, for the owner (O-20).
    Measured,
}

impl Basis {
    /// The basis as the report prints it.
    #[must_use]
    pub fn word(self) -> String {
        match self {
            Basis::DocCompletion => "doc-completion".to_owned(),
            Basis::DocOther => "doc-other".to_owned(),
            Basis::Pinned { cut, test, handed } => format!("pinned({cut} {handed} by {test})"),
            Basis::Measured => "measured".to_owned(),
        }
    }
}

/// **Which lane a kind's writes go by**: a bundle's staged ops (an authoritative landing) or a
/// normal-lane handler's module ops (a synchronous command, a tick's legacy write).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lane {
    /// An authoritative bundle: its `ops.len()` (SM.8's `land_counting` rule).
    Bundle,
    /// A normal-lane write: the module ops it folded.
    Normal,
}

impl Lane {
    /// The lane as the report prints it.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Lane::Bundle => "bundle",
            Lane::Normal => "normal",
        }
    }
}

/// One kind's write gate.
#[derive(Clone, Copy, Debug)]
pub struct Gate {
    /// The act kind (`kind_of`'s word).
    pub kind: &'static str,
    /// The most writes one act of the kind may make.
    pub ceiling: usize,
    /// Where the ceiling comes from.
    pub basis: Basis,
    /// Which lane its writes go by.
    pub lane: Lane,
}

/// A completion at §11's target.
const fn done(kind: &'static str) -> Gate {
    Gate {
        kind,
        ceiling: 8,
        basis: Basis::DocCompletion,
        lane: Lane::Bundle,
    }
}

/// Any other bundle act at §11's target.
const fn act(kind: &'static str) -> Gate {
    Gate {
        kind,
        ceiling: 3,
        basis: Basis::DocOther,
        lane: Lane::Bundle,
    }
}

/// A normal-lane act at §11's target.
const fn normal(kind: &'static str) -> Gate {
    Gate {
        kind,
        ceiling: 3,
        basis: Basis::DocOther,
        lane: Lane::Normal,
    }
}

/// A kind over its target with no earlier handoff: pinned at SM.12c's constructed maximum
/// (O-20).
const fn measured(kind: &'static str, ceiling: usize, lane: Lane) -> Gate {
    Gate {
        kind,
        ceiling,
        basis: Basis::Measured,
        lane,
    }
}

/// **Every mining-layer act kind's write gate** (doc 20 §11's Tick row: "≤ 8 writes a completion,
/// ≤ 3 for any other act"; doc 20a §5.3.3; the lead's pin-vs-target rule). A kind whose
/// constructed maximum meets its target carries `DocCompletion`/`DocOther` and the target; a
/// figure an earlier cut handed forward is `Pinned` with that cut's test; any other kind over its
/// target is `Measured` at its exact constructed maximum, never trimmed (O-20). The census tests
/// (`sm12c_every_act_kind_counts_its_writes_*`) construct every kind's structural worst case on
/// the shipped lane and the month re-checks every landing against this table.
///
/// **A change that adds a write moves a ceiling here and must say why; a count observed above a
/// ceiling is a missing census case, never a reason to raise the const alone.**
pub const WRITE_GATES: &[Gate] = &[
    // The surface starts (the operator's authoritative submission; doc 20 §8).
    act("start.drive"),
    act("start.dump"),
    act("start.launch"),
    act("start.mine"),
    act("start.mine_stop"),
    act("start.plan_clear"),
    act("start.plan_end"),
    act("start.plan_file"),
    act("start.plan_strike"),
    act("start.recover"),
    act("start.scan"),
    act("start.sound"),
    act("start.sound_quiet"),
    act("start.unload"),
    // The plan's own steps (SM.11), the idle return and the queued unload (SM.8).
    act("start.drive.plan"),
    act("start.mine.plan"),
    act("start.recover.plan"),
    act("start.sound.plan"),
    act("start.recover.idle"),
    act("start.unload.queued"),
    // The completions (the surface tick's finish, `tacsim_surface_done`).
    done("done.drive"),
    done("done.dump"),
    done("done.launch"),
    done("done.recover"),
    done("done.scan"),
    done("done.sound"),
    done("done.cycle"),
    // SM.8 handed 19 (two goods with evictions); the structural maximum is four: a hopper holds
    // by mass, so a rig that works a pocket of each family carries all four ores, and the finish
    // pushes one ring entry a good, each evicting up to eight — four goods into four fresh hold
    // pools with 32 evictions, constructed 63 (the review's correctness/F4; O-20).
    Gate {
        kind: "done.unload",
        ceiling: 63,
        basis: Basis::Pinned {
            cut: "SM.8",
            test: "sm8_the_unload_finish_counts_its_writes",
            handed: 19,
        },
        lane: Lane::Bundle,
    },
    // The chart and the preference document. A forget writes one chart row a site its target
    // names: a body's forget at a principal with a row at every Ceres site writes five (the
    // review's conformance/F6), over §11's 3 (O-20).
    measured("forget", 5, Lane::Bundle),
    normal("tvpref"),
    // Tenure (doc 20 §9; SM.10): every claim act is one `industry_claim_tenure`.
    measured("claim.stake", 9, Lane::Bundle),
    measured("claim.renew", 5, Lane::Bundle),
    measured("claim.contest", 6, Lane::Bundle),
    measured("claim.release", 6, Lane::Bundle),
    measured("claim.unpaid", 6, Lane::Bundle),
    measured("claim.lapse", 6, Lane::Bundle),
    measured("claim.seed", 7, Lane::Bundle),
    // The site clerk (SM.6, SM.12b).
    measured("site.open", 5, Lane::Bundle),
    measured("site.worked", 6, Lane::Bundle),
    normal("site.house_licence"),
    // The licence and the lease (synchronous normal-lane commands).
    measured("licence.buy", 14, Lane::Normal),
    measured("lease.take", 19, Lane::Normal),
    measured("lease.return", 15, Lane::Normal),
    // The mill (SM.12a) and the dispatcher (SM.12b).
    Gate {
        kind: "mill.batch",
        ceiling: 6,
        basis: Basis::Pinned {
            cut: "SM.12a",
            test: "sm12a_the_mill_counts_its_writes",
            handed: 6,
        },
        lane: Lane::Bundle,
    },
    normal("mill.restock"),
    Gate {
        kind: "dispatch.deliver",
        ceiling: 8,
        basis: Basis::Pinned {
            cut: "SM.12b",
            test: "sm12b_a_delivery_is_one_receipted_bundle",
            handed: 8,
        },
        lane: Lane::Bundle,
    },
    act("dispatch.survey"),
];

/// **Where each exact pin is asserted** (SM.8's T6 style): every `Measured` gate's ceiling — and
/// `done.unload`'s constructed maximum — is asserted equal to the constructed maximum by the
/// census test named here, the one whose script builds that kind's largest variant.
pub const MEASURED_AT: &[(&str, &str)] = &[
    (
        "done.unload",
        "sm12c_every_act_kind_counts_its_writes_on_the_surface",
    ),
    (
        "forget",
        "sm12c_every_act_kind_counts_its_writes_on_the_surface",
    ),
    (
        "licence.buy",
        "sm12c_every_act_kind_counts_its_writes_on_the_surface",
    ),
    (
        "lease.take",
        "sm12c_every_act_kind_counts_its_writes_on_the_surface",
    ),
    (
        "lease.return",
        "sm12c_every_act_kind_counts_its_writes_in_tenure",
    ),
    (
        "claim.stake",
        "sm12c_every_act_kind_counts_its_writes_in_tenure",
    ),
    (
        "claim.contest",
        "sm12c_every_act_kind_counts_its_writes_in_tenure",
    ),
    (
        "claim.seed",
        "sm12c_every_act_kind_counts_its_writes_in_tenure",
    ),
    (
        "claim.lapse",
        "sm12c_every_act_kind_counts_its_writes_in_tenure",
    ),
    (
        "claim.release",
        "sm12c_every_act_kind_counts_its_writes_in_tenure",
    ),
    (
        "claim.renew",
        "sm12c_every_act_kind_counts_its_writes_in_tenure",
    ),
    (
        "claim.unpaid",
        "sm12c_every_act_kind_counts_its_writes_in_tenure",
    ),
    (
        "site.open",
        "sm12c_every_act_kind_counts_its_writes_in_tenure",
    ),
    (
        "site.worked",
        "sm12c_every_act_kind_counts_its_writes_in_tenure",
    ),
];

/// The gate of `kind`, if it is gated.
#[must_use]
pub fn gate(kind: &str) -> Option<&'static Gate> {
    WRITE_GATES.iter().find(|gate| gate.kind == kind)
}

/// **The census tests and the kinds each must construct** (doc 20a §5.3.4; the brief's L4.4): a
/// test's constructed kinds are the kinds its own script landed — by its own pilots, under its
/// own labels, or the clerks' and the dispatcher's landings the script set up and waited for.
/// The union over the rows is exactly [`WRITE_GATES`]' kinds (both ways, pinned by
/// `sm12c_every_gated_kind_has_a_census_and_every_mining_id_a_kind`).
pub const CENSUS: &[(&str, &[&str])] = &[
    (
        "sm12c_every_act_kind_counts_its_writes_on_the_surface",
        &[
            "start.launch",
            "done.launch",
            "start.scan",
            "done.scan",
            "start.sound",
            "start.sound_quiet",
            "done.sound",
            "start.drive",
            "done.drive",
            "start.mine",
            "done.cycle",
            "start.mine_stop",
            "start.recover",
            "done.recover",
            "start.unload.queued",
            "start.unload",
            "done.unload",
            "start.dump",
            "done.dump",
            "forget",
            "tvpref",
            "licence.buy",
            "lease.take",
        ],
    ),
    (
        "sm12c_every_act_kind_counts_its_writes_in_a_plan",
        &[
            "start.launch",
            "done.launch",
            "start.scan",
            "done.scan",
            "start.plan_file",
            "start.plan_clear",
            "start.plan_strike",
            "start.plan_end",
            "start.drive.plan",
            "start.sound.plan",
            "start.mine.plan",
            "start.recover.plan",
            "start.recover.idle",
            "start.unload.queued",
            "done.drive",
            "done.sound",
            "done.cycle",
            "done.recover",
            "done.unload",
            "licence.buy",
            "lease.take",
        ],
    ),
    (
        "sm12c_every_act_kind_counts_its_writes_in_tenure",
        &[
            "licence.buy",
            "lease.take",
            "lease.return",
            "start.launch",
            "done.launch",
            "start.scan",
            "done.scan",
            "start.sound",
            "start.sound_quiet",
            "done.sound",
            "start.drive",
            "done.drive",
            "start.mine",
            "done.cycle",
            "start.dump",
            "done.dump",
            "start.recover.idle",
            "done.recover",
            "claim.stake",
            "claim.release",
            "claim.renew",
            "claim.contest",
            "claim.unpaid",
            "claim.lapse",
            "claim.seed",
            "site.open",
            "site.worked",
        ],
    ),
    (
        "sm12c_every_act_kind_counts_its_writes_at_the_port",
        &[
            "mill.batch",
            "mill.restock",
            "dispatch.survey",
            "dispatch.deliver",
            "site.house_licence",
            "start.plan_file",
        ],
    ),
];

// ---------------------------------------------------------------------------
// The classifier's pure core
// ---------------------------------------------------------------------------

/// The player's stake, renew and contest receipt, formatted inline at `port/claims.rs:2130-2135`
/// (`tacsim.claim/{actor}/{contract}`; a release `tacsim.claim/{actor}/release/{site}/{block}/
/// {epoch}`). No constant names it; matched as written there, after the clerk's three prefixes —
/// which read `tacsim.claim.` and so are never prefixed by it.
const PLAYER_CLAIM_PREFIX: &str = "tacsim.claim/";

/// The labels a player's claim receipt is classified by.
pub const CLAIM_LABELS: [&str; 3] = ["stake", "renew", "contest"];

/// **The act kind of one landing** (doc 20a §5.3.3; the brief's L4.1), pure over the action id,
/// its parameters, the rig record's act in flight (`done_kind`, a finish's `ActKind::word`) and
/// the driver's label. Every mining-layer action id is mapped explicitly; every other id is
/// `other.<id>`; an unmapped `industry_*` id is `industry.<suffix>` and a player's claim landed
/// with no label `claim.unlabelled` — neither gated nor `other.*`, so the leak guard fails on them
/// loudly.
#[must_use]
pub fn kind_of(
    action: &str,
    params: &Value,
    done_kind: Option<&str>,
    label: Option<&'static str>,
) -> String {
    use tacsim::port::site::{
        CLAIM_LAPSE_KEY_PREFIX, CLAIM_SEED_KEY_PREFIX, CLAIM_UNPAID_KEY_PREFIX, SITE_WORK_ACTION,
    };
    use tacsim::surface::act::{SURFACE_ACT_ACTION, SURFACE_DONE_ACTION};
    let word = |key: &str| params.get(key).and_then(Value::as_str).unwrap_or_default();
    match action {
        SURFACE_ACT_ACTION => {
            let act = word("act");
            if act == "plan_end" {
                return "start.plan_end".to_owned();
            }
            if word("how") == "queued" {
                return "start.unload.queued".to_owned();
            }
            match word("by") {
                "plan" => format!("start.{act}.plan"),
                "idle" => format!("start.{act}.idle"),
                _ => format!("start.{act}"),
            }
        }
        SURFACE_DONE_ACTION => {
            if params.get("cycle").is_some_and(|cycle| !cycle.is_null()) {
                "done.cycle".to_owned()
            } else {
                format!("done.{}", done_kind.unwrap_or("unknown"))
            }
        }
        tacsim::surface::chart::CHART_FORGET_ACTION => "forget".to_owned(),
        SITE_WORK_ACTION => "site.worked".to_owned(),
        mush_industry::ACTION_CLAIM_TENURE => {
            let receipt = word("receipt");
            if receipt.starts_with(CLAIM_UNPAID_KEY_PREFIX) {
                "claim.unpaid".to_owned()
            } else if receipt.starts_with(CLAIM_LAPSE_KEY_PREFIX) {
                "claim.lapse".to_owned()
            } else if receipt.starts_with(CLAIM_SEED_KEY_PREFIX) {
                "claim.seed".to_owned()
            } else if let Some(rest) = receipt.strip_prefix(PLAYER_CLAIM_PREFIX) {
                if rest
                    .split('/')
                    .nth(1)
                    .is_some_and(|second| second == "release")
                {
                    "claim.release".to_owned()
                } else {
                    match label {
                        Some(word) if CLAIM_LABELS.contains(&word) => format!("claim.{word}"),
                        _ => "claim.unlabelled".to_owned(),
                    }
                }
            } else {
                "claim.unlabelled".to_owned()
            }
        }
        mush_industry::provider::ACTION_DEPOSIT_OPEN => "site.open".to_owned(),
        tacsim::port::mill::MILL_ACTION => "mill.batch".to_owned(),
        tacsim::port::yard::DISPATCH_ACTION => format!("dispatch.{}", word("act")),
        other => match other.strip_prefix("industry_") {
            Some(suffix) => format!("industry.{suffix}"),
            None => format!("other.{other}"),
        },
    }
}

/// The action-id prefixes of the mining layer: none of them may ever land as `other.*`.
pub const MINING_IDS: [&str; 6] = [
    "tacsim_surface",
    "tacsim_chart",
    "tacsim_site",
    "tacsim_dispatch",
    "tacsim_port_mill",
    "industry_",
];

/// **The leak guard's core** (the brief's L4.2), pure over `(kind, action id)` pairs: every
/// offender — a kind neither gated nor `other.*` (`claim.unlabelled`, `industry.<x>`, a typo, a
/// new id), or a mining-layer id landed as `other.*` — named once.
#[must_use]
pub fn unclassified<'a>(kinds: impl IntoIterator<Item = (&'a str, &'a str)>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (kind, action) in kinds {
        let leak = if kind.starts_with("other.") {
            MINING_IDS.iter().any(|prefix| action.starts_with(prefix))
        } else {
            gate(kind).is_none()
        };
        let line = format!("{kind} ({action})");
        if leak && !out.contains(&line) {
            out.push(line);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// The levers the cohort's lane edits (C7 reads them)
// ---------------------------------------------------------------------------

/// **The JSON paths the cohort lane may differ from the shipped lane at** (O-21; the brief's
/// L9.2 and L11.1), each a `/`-joined path into the lane document with `*` matching any one
/// segment. `manifest/digest` always differs (the variant is re-signed). The coarse filter only:
/// C7 also asserts the diff equals the exact paths found by identity in the shipped lane
/// (`cost_lane.rs`'s `lever_paths_by_identity`), both ways.
pub const LEVER_PATHS: &[&str] = &[
    "manifest/digest",
    // The Co-op's programme: the hull pool and its names; the Trust's capital beside it.
    "entries/ceres_port/def/programme/pool",
    "entries/ceres_port/def/programme/hull_names",
    "entries/ceres_port/def/programme/hull_names/*",
    "entries/ceres_port/def/orgs/*/capital_major",
    "entries/ceres_port/def/orgs/*/raises",
    "entries/ceres_port/def/orgs/*/raises/*",
    // Occator's rig pool.
    "entries/ceres_port/def/vehicles/pool",
    // The ore bids' intake and drip, the stock rows, the mills.
    "entries/ceres_port/def/vendors/*/bids/*/intake",
    "entries/ceres_port/def/vendors/*/bids/*/refresh/per_ic_day",
    "entries/ceres_port/def/vendors/*/bids/*/refresh/cap",
    "entries/ceres_port/def/vendors/*/stock/*/qty",
    "entries/ceres_port/def/vendors/*/mill/*/per_ic_day_t",
    "entries/ceres_port/def/vendors/*/mill/*/batch_t",
];

/// Whether `path` matches one of [`LEVER_PATHS`].
#[must_use]
pub fn is_lever(path: &str) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    LEVER_PATHS.iter().any(|lever| {
        let want: Vec<&str> = lever.split('/').collect();
        want.len() == parts.len()
            && want
                .iter()
                .zip(&parts)
                .all(|(want, part)| *want == "*" || want == part)
    })
}

/// **Every path at which two JSON documents differ** (pure; objects by key, arrays by index, an
/// array of a different length differing at the array itself), in document order.
#[must_use]
pub fn json_diff(a: &Value, b: &Value) -> Vec<String> {
    let mut out = Vec::new();
    diff_into(a, b, "", &mut out);
    out
}

fn diff_into(a: &Value, b: &Value, at: &str, out: &mut Vec<String>) {
    let join = |key: &str| {
        if at.is_empty() {
            key.to_owned()
        } else {
            format!("{at}/{key}")
        }
    };
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            let mut keys: Vec<&String> = x.keys().chain(y.keys()).collect();
            keys.sort();
            keys.dedup();
            for key in keys {
                match (x.get(key), y.get(key)) {
                    (Some(l), Some(r)) => diff_into(l, r, &join(key), out),
                    _ => out.push(join(key)),
                }
            }
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
            for (n, (l, r)) in x.iter().zip(y).enumerate() {
                diff_into(l, r, &join(&n.to_string()), out);
            }
        }
        _ if a == b => {}
        _ => out.push(at.to_owned()),
    }
}

// ---------------------------------------------------------------------------
// The report
// ---------------------------------------------------------------------------

/// **`cost-lane.json`** (the brief's L3.7): one object, written once an IC day and before any
/// end-of-run assertion. Every value is the month's own JSON; the top-level keys are fixed.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Report {
    /// The report's schema version.
    pub schema: u32,
    /// TVs, hulls, sites, IC days, the levers.
    pub cohort: Value,
    /// The surface fires' figures.
    pub surface: Value,
    /// The port fires' figures, total and by arm.
    pub port: Value,
    /// The other ticks' share of each second.
    pub other_ticks: Value,
    /// One row per landed act kind.
    pub writes: Value,
    /// Landing times per kind.
    pub landings: Value,
    /// The site-ground LRU and the warms.
    pub churn: Value,
    /// The planner's figures.
    pub planning: Value,
    /// The feeds' frames.
    pub frames: Value,
    /// The hopper rings.
    pub rings: Value,
    /// The applied-marker mirror.
    pub mirror: Value,
    /// The Ceres clerk's acts by step per IC day.
    pub clerk: Value,
    /// Loops, tonnes, batches, deliveries, backlogs, stuck pilots.
    pub activity: Value,
    /// Claims by kind, skips by kind.
    pub claims: Value,
    /// The three conditional decisions' inputs.
    pub decisions: Value,
    /// One row per IC day.
    pub days: Value,
    /// Inline breaches, raised after the final write.
    pub breaches: Value,
    /// Why the month stopped early, if it did.
    pub stopped: Value,
    /// The lane's own wall time, seconds.
    pub wall_s: Value,
}

/// **Where `TACSIM_COST_JSON` lands** (`harness.rs`'s `report_path`, copied): `cargo test` runs an
/// integration test with the package directory as its working directory, so a relative path is
/// resolved against the workspace root, never `crates/tacsim/`.
#[must_use]
pub fn report_path(given: &str) -> std::path::PathBuf {
    let path = std::path::PathBuf::from(given);
    if path.is_absolute() {
        return path;
    }
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .map(|root| root.join(&path))
        .unwrap_or(path)
}

/// Write `report` as pretty JSON to `path`.
///
/// # Panics
/// When it does not serialize or write (the lane's own report is never silently lost).
pub fn write_report(path: &std::path::Path, report: &Report) {
    let text = serde_json::to_string_pretty(report).expect("the report serializes");
    std::fs::write(path, text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
}

// ---------------------------------------------------------------------------
// Pure tests of the pieces above (run in every build through cost_lane's pure test)
// ---------------------------------------------------------------------------

/// The pure pieces' own checks, called by `cost_lane.rs`'s coverage test.
pub fn self_check() {
    assert_eq!(pct(&[], 990), 0);
    assert_eq!(pct(&[5], 990), 5);
    let ten: Vec<u64> = (1..=10).collect();
    assert_eq!(pct(&ten, 500), 5);
    assert_eq!(pct(&ten, 990), 10);
    assert_eq!(pct(&ten, 100), 1);
    assert_eq!(windows(&ten, 5), vec![2, 4, 6, 8, 10]);
    assert!(is_lever("entries/ceres_port/def/vendors/0/bids/2/intake"));
    assert!(!is_lever(
        "entries/ceres_port/def/vendors/0/bids/2/price_major"
    ));
    assert_eq!(
        json_diff(
            &serde_json::json!({"a": 1, "b": [1, 2]}),
            &serde_json::json!({"a": 2, "b": [1, 3], "c": 0})
        ),
        vec!["a".to_owned(), "b/1".to_owned(), "c".to_owned()]
    );
}
