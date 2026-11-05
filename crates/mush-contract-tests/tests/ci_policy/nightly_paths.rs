//! **Every non-PR job names the paths that should dispatch it** — a CI policy gate
//! (doc 43 rule 2.3 #15).
//!
//! Pull-request CI never runs `.github/workflows/ci.yml`'s nightly-, weekly- and
//! dispatch-tier jobs, so a PR that moves their pins learns of it the next morning
//! (the nightly of 2026-11-05 broke on #475's grace windows). Rule #15 has a PR that
//! touches one of their paths dispatch them and get them green before it is marked
//! ready. `.github/nightly-paths.txt` names the paths, and `scripts/nightly_advice.py`
//! prints the jobs a change should dispatch; the `lint` cell runs it on every PR.
//!
//! An unchecked map rots. This gate refuses:
//! - a job id the map names that ci.yml does not have;
//! - a non-PR job with no line (except the report job, which tests nothing);
//! - a tier that disagrees with the job's `if:`;
//! - a path that no longer exists;
//! - a workflow whose lint cell no longer runs the advice.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use mush_contract_tests::scan;

/// The jobs that run on neither a push nor a PR but test nothing: they need no paths.
const NOT_A_TEST: &[&str] = &["nightly-report"];

/// The tier a job's `if:` puts it in, or `None` for a job that runs on pull requests.
fn tier_of(condition: &str) -> Option<&'static str> {
    if condition.contains("pull_request") {
        return None;
    }
    if condition.contains("'17 4 * * 0'") {
        Some("weekly")
    } else if condition.contains("schedule") {
        Some("nightly")
    } else if condition.contains("workflow_dispatch") {
        Some("dispatch")
    } else {
        None
    }
}

/// ci.yml's jobs: id -> its job-level `if:`, if it has one.
fn workflow_jobs(workflow: &str) -> BTreeMap<String, Option<String>> {
    let mut jobs = BTreeMap::new();
    let mut in_jobs = false;
    let mut current: Option<String> = None;
    for line in workflow.lines() {
        if line == "jobs:" {
            in_jobs = true;
            continue;
        }
        if !in_jobs {
            continue;
        }
        if let Some(id) = line
            .strip_prefix("  ")
            .filter(|rest| !rest.starts_with(' ') && !rest.starts_with('#'))
            .and_then(|rest| rest.strip_suffix(':'))
        {
            jobs.insert(id.to_owned(), None);
            current = Some(id.to_owned());
            continue;
        }
        if let (Some(id), Some(condition)) = (&current, line.strip_prefix("    if: ")) {
            jobs.insert(id.clone(), Some(condition.trim().to_owned()));
        }
    }
    jobs
}

/// The map's lines: (job id, tier, path).
fn map_lines(text: &str) -> Vec<(String, String, String)> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            assert_eq!(
                parts.len(),
                3,
                "nightly-paths line `{line}` is not `<job id> <tier> <path>`"
            );
            (
                parts[0].to_owned(),
                parts[1].to_owned(),
                parts[2].to_owned(),
            )
        })
        .collect()
}

#[test]
fn every_non_pr_job_names_its_paths_at_its_tier() {
    let root = scan::repo_root();
    let workflow_path = root.join(".github/workflows/ci.yml");
    let workflow = fs::read_to_string(&workflow_path)
        .unwrap_or_else(|err| panic!("{}: {err}", workflow_path.display()));
    let map_path = root.join(".github/nightly-paths.txt");
    let map =
        fs::read_to_string(&map_path).unwrap_or_else(|err| panic!("{}: {err}", map_path.display()));

    let jobs = workflow_jobs(&workflow);
    let non_pr: BTreeMap<&str, &str> = jobs
        .iter()
        .filter(|(id, _)| !NOT_A_TEST.contains(&id.as_str()))
        .filter_map(|(id, condition)| {
            condition
                .as_deref()
                .and_then(tier_of)
                .map(|tier| (id.as_str(), tier))
        })
        .collect();
    assert!(
        non_pr.len() >= 10,
        "found only {} non-PR jobs in ci.yml; the job scan no longer reads the file",
        non_pr.len()
    );

    let mut mapped: BTreeSet<String> = BTreeSet::new();
    for (job, tier, path) in map_lines(&map) {
        let Some(expected) = non_pr.get(job.as_str()) else {
            panic!(
                "{} names `{job}`, which is not a nightly, weekly or dispatch job of {}",
                map_path.display(),
                workflow_path.display()
            );
        };
        assert_eq!(
            tier.as_str(),
            *expected,
            "{}: `{job}` is listed as {tier} but its `if:` makes it {expected}",
            map_path.display()
        );
        assert!(
            root.join(&path).exists(),
            "{}: `{job}`'s path `{path}` does not exist; a moved file takes its line with it",
            map_path.display()
        );
        mapped.insert(job);
    }
    let unmapped: Vec<&&str> = non_pr.keys().filter(|id| !mapped.contains(**id)).collect();
    assert!(
        unmapped.is_empty(),
        "these ci.yml jobs never run on a pull request and {} names no path for them, so no \
         PR is ever told to dispatch them (rule 2.3 #15): {unmapped:?}",
        map_path.display()
    );
}

#[test]
fn the_lint_cell_prints_the_dispatch_advice() {
    let root = scan::repo_root();
    let workflow_path = root.join(".github/workflows/ci.yml");
    let workflow = fs::read_to_string(&workflow_path)
        .unwrap_or_else(|err| panic!("{}: {err}", workflow_path.display()));
    assert!(
        workflow.contains("scripts/nightly_advice.py"),
        "{} no longer runs scripts/nightly_advice.py on pull requests (rule 2.3 #15)",
        workflow_path.display()
    );
    assert!(
        root.join("scripts/nightly_advice.py").is_file(),
        "scripts/nightly_advice.py is gone"
    );
}

#[test]
fn the_tier_reading_matches_the_three_schedules() {
    assert_eq!(
        tier_of("github.event_name == 'workflow_dispatch' || github.event_name == 'schedule'"),
        Some("nightly")
    );
    assert_eq!(
        tier_of(
            "github.event_name == 'workflow_dispatch' || github.event.schedule == '17 4 * * 0'"
        ),
        Some("weekly")
    );
    assert_eq!(
        tier_of("github.event_name == 'workflow_dispatch'"),
        Some("dispatch")
    );
    assert_eq!(tier_of("github.event_name == 'pull_request'"), None);
    assert_eq!(tier_of("matrix.shard == 'lint'"), None);
}
