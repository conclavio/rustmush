#!/usr/bin/env python3
"""Add measurement-only sight instrumentation to an explicitly marked scratch tree.

Create ROOT/.sight-investigation first. Never run on a deployed or Git checkout.
Run the harness serially (TACSIM_FRAME_WORKERS=1). Each ordinary harness report
gets a sibling .sight-probe.json. All sight calls still execute normally; no
cached answer is ever returned. Timing from this instrumented build is invalid.
"""
import argparse
from pathlib import Path

PROBE = r'''
use std::cell::RefCell;
use std::collections::BTreeMap;
use mush_atlas::Eye;
use super::horizon::HorizonSight;

type Endpoint = (u16, u16, i32);
type Key = (usize, Endpoint, Endpoint);
#[derive(Default)]
struct Probe {
    active: bool,
    seen: BTreeMap<Key, HorizonSight>,
    directed: BTreeMap<Key, HorizonSight>,
    directed_repeated: u64,
    directed_changed: u64,
    reversed_changed_sight: u64,
    calls: u64,
    repeated: u64,
    changed: u64,
    outside: u64,
    steps: u64,
    peak_entries: usize,
    cells: u64,
    blocks: u64,
    repeated_cells: u64,
    repeated_blocks: u64,
    callers: BTreeMap<String, (u64, u64)>,
}
thread_local! { static PROBE: RefCell<Probe> = RefCell::new(Probe::default()); }
pub struct Step;
pub fn begin() -> Step {
    PROBE.with_borrow_mut(|p| {
        assert!(!p.active, "nested measurement substep");
        p.active = true;
        p.seen.clear();
        p.directed.clear();
        p.steps += 1;
    });
    Step
}
impl Drop for Step {
    fn drop(&mut self) {
        PROBE.with_borrow_mut(|p| {
            p.peak_entries = p.peak_entries.max(p.seen.len());
            p.seen.clear();
            p.directed.clear();
            p.active = false;
        });
    }
}
#[track_caller]
pub fn record(terrain: usize, from: Eye, to: Eye, answer: &HorizonSight) {
    let caller = std::panic::Location::caller();
    PROBE.with_borrow_mut(|p| {
        if !p.active { p.outside += 1; return; }
        let a = (from.cell.0, from.cell.1, from.height_cm);
        let b = (to.cell.0, to.cell.1, to.height_cm);
        if let Some(previous) = p.directed.insert((terrain, a, b), answer.clone()) {
            p.directed_repeated += 1;
            if previous != *answer { p.directed_changed += 1; }
        }
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        let previous = p.seen.insert((terrain, a, b), answer.clone());
        let repeated = previous.is_some();
        p.calls += 1;
        p.cells += u64::from(answer.cell_probes);
        p.blocks += u64::from(answer.horizon_probes);
        let row = p.callers.entry(format!("{}:{}", caller.file(), caller.line())).or_default();
        row.0 += 1;
        if repeated {
            row.1 += 1;
            p.repeated += 1;
            p.repeated_cells += u64::from(answer.cell_probes);
            p.repeated_blocks += u64::from(answer.horizon_probes);
            if previous.as_ref() != Some(answer) { p.changed += 1; }
            if previous.as_ref().is_some_and(|old| old.sight != answer.sight) { p.reversed_changed_sight += 1; }
        }
    });
}
pub fn take_report() -> serde_json::Value {
    PROBE.with_borrow_mut(|state| {
        let p = std::mem::take(state);
        serde_json::json!({
            "substeps": p.steps, "calls": p.calls, "repeated": p.repeated,
            "directed_repeated": p.directed_repeated,
            "directed_changed_answers": p.directed_changed,
            "canonical_changed_sight": p.reversed_changed_sight,
            "changed_answers": p.changed, "outside_substep_calls": p.outside,
            "peak_unique_keys_per_substep": p.peak_entries,
            "entry_payload_bytes": std::mem::size_of::<(Key, HorizonSight)>(),
            "cell_probes": p.cells, "horizon_probes": p.blocks,
            "repeated_cell_probes": p.repeated_cells,
            "repeated_horizon_probes": p.repeated_blocks,
            "callers_calls_and_repeats": p.callers,
            "note": "Measurement only; all traces executed. Payload size excludes allocator/tree overhead."
        })
    })
}
'''

def replace(root, name, old, new):
    path = root / name
    text = path.read_text(encoding="utf-8")
    assert text.count(old) == 1, (name, "anchor missing or ambiguous")
    path.write_text(text.replace(old, new), encoding="utf-8", newline="\n")

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("root", type=Path)
args = parser.parse_args()
root = args.root.resolve()
assert (root / ".sight-investigation").is_file(), "Scratch marker required"
assert not (root / ".git").exists(), "Refusing to instrument a Git checkout"
assert not (root / "crates/tacsim/src/arena/sight_probe.rs").exists(), "Already instrumented"
(root / "crates/tacsim/src/arena/sight_probe.rs").write_text(PROBE, encoding="utf-8", newline="\n")
with (root / "crates/tacsim/src/arena/mod.rs").open("a", encoding="utf-8", newline="\n") as stream:
    stream.write("\npub mod sight_probe; // SCRATCH MEASUREMENT ONLY\n")
replace(root, "crates/tacsim/src/arena/state.rs",
    "    pub fn sight(&self, from: Eye, to: Eye) -> HorizonSight {\n        self.horizon.sight(self, from, to)\n    }",
    "    #[track_caller]\n    pub fn sight(&self, from: Eye, to: Eye) -> HorizonSight {\n        let answer = self.horizon.sight(self, from, to);\n        super::sight_probe::record(self as *const Self as usize, from, to, &answer);\n        answer\n    }")
replace(root, "crates/tacsim/src/arena/perception.rs",
    "    pub fn sight(&self, from: Eye, to: Eye, counters: &mut PerceptionCounters) -> Sight {",
    "    #[track_caller]\n    pub fn sight(&self, from: Eye, to: Eye, counters: &mut PerceptionCounters) -> Sight {")
replace(root, "crates/tacsim/src/arena/step.rs",
    "    ) -> bool {\n        let mut notices: Vec<Notice> = Vec::new();",
    "    ) -> bool {\n        let _sight_probe = super::sight_probe::begin();\n        let mut notices: Vec<Notice> = Vec::new();")
replace(root, "crates/tacsim/tests/harness.rs",
    'fn write_report(path: &std::path::Path, report: &serde_json::Value) {',
    'fn write_report(path: &std::path::Path, report: &serde_json::Value) {\n    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent).unwrap(); }\n    std::fs::write(path.with_extension("sight-probe.json"), serde_json::to_string_pretty(&tacsim::arena::sight_probe::take_report()).unwrap()).unwrap();')
print("Instrumented scratch tree:", root)
