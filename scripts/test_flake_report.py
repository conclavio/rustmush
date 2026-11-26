#!/usr/bin/env python3
"""The flake report's rule, on synthetic reports (doc 51 unit FL.2).

    python3 -B scripts/test_flake_report.py

No network and no `gh`: the CI sources are exercised through the same
`Seen` records the downloader yields.
"""

import datetime
import json
import pathlib
import tempfile
import unittest

import flake_report as report

UTC = datetime.timezone.utc
NOW = datetime.datetime(2026, 11, 23, 12, 0, tzinfo=UTC)
BINARY = "mushd::sim_lenses"


def junit(cases):
    """cases: [(binary, name, outcome)], outcome one of pass, fail, retry."""
    inner = {
        "pass": "",
        "fail": '<failure message="boom" type="test failure">boom</failure>',
        "retry": '<flakyFailure message="boom" type="test failure">boom</flakyFailure>',
    }
    body = "".join(
        f'<testsuite name="{binary}"><testcase name="{name}" classname="{binary}" '
        f'timestamp="2026-11-23T01:30:00.000+00:00" time="1.0">{inner[outcome]}</testcase></testsuite>'
        for binary, name, outcome in cases
    )
    return f'<?xml version="1.0"?><testsuites name="x" timestamp="2026-11-23T01:30:00.000+00:00">{body}</testsuites>'


def ci(name, outcome, head, branch, run="1"):
    when = NOW - datetime.timedelta(days=1)
    return report.Seen((BINARY, name), outcome, head, branch, when, f"{run} a1 rest-mushd", f"https://example.invalid/{run}", "ci")


def judged(seen):
    tally = report.Tally()
    for one in seen:
        tally.add(one)
    return report.judge(tally)


def row(**fields):
    base = {"binary": BINARY, "class": "clock", "row": 13, "listed": "2026-11-23", "expires": "2026-12-07"}
    base.update(fields)
    return base


class Reading(unittest.TestCase):
    def test_a_report_gives_each_testcase_its_outcome(self):
        data = junit([(BINARY, "a", "pass"), (BINARY, "b", "fail"), (BINARY, "c", "retry")]).encode()
        got = {test[1]: outcome for test, outcome, _ in report.read_junit(data, "x")}
        self.assertEqual(got, {"a": "pass", "b": "fail", "c": "retry"})

    def test_a_file_that_declares_entities_is_not_read(self):
        data = b'<?xml version="1.0"?><!DOCTYPE x [<!ENTITY a "b">]><testsuites/>'
        self.assertEqual(list(report.read_junit(data, "x")), [])

    def test_an_artifact_belongs_to_the_attempt_whose_job_was_running(self):
        jobs = [
            {"name": "test (bot)", "run_attempt": 1, "started_at": "2026-11-22T18:57:00Z", "completed_at": "2026-11-22T19:07:20Z"},
            {"name": "test (bot)", "run_attempt": 2, "started_at": "2026-11-22T21:50:00Z", "completed_at": "2026-11-22T22:03:40Z"},
        ]
        first = {"created_at": "2026-11-22T19:07:16Z"}
        second = {"created_at": "2026-11-22T22:03:33Z"}
        self.assertEqual(report.attempt_of(first, "bot", jobs), 1)
        self.assertEqual(report.attempt_of(second, "bot", jobs), 2)
        self.assertEqual(report.shard_of("test (rest-mushd)"), "rest-mushd")


class TheRule(unittest.TestCase):
    def test_a_pass_on_retry_is_a_flake(self):
        facts = judged([ci("t", "retry", "h1", "main")])
        self.assertTrue(facts[(BINARY, "t")]["firm"])
        self.assertEqual(facts[(BINARY, "t")]["why"], ["retry x1"])

    def test_a_failure_and_a_pass_at_one_head_is_a_flake(self):
        facts = judged([ci("t", "fail", "h1", "b", "1"), ci("t", "pass", "h1", "b", "2")])
        self.assertEqual(facts[(BINARY, "t")]["why"], ["same head x1"])
        self.assertTrue(facts[(BINARY, "t")]["firm"])

    def test_a_pass_at_another_head_proves_nothing(self):
        facts = judged([ci("t", "fail", "h1", "b"), ci("t", "pass", "h2", "b")])
        self.assertEqual(facts[(BINARY, "t")]["why"], [])

    def test_two_branches_alone_is_a_lead_to_read_not_a_finding(self):
        facts = judged([ci("t", "fail", "h1", "one"), ci("t", "fail", "h2", "two")])
        self.assertEqual(facts[(BINARY, "t")]["why"], ["2 branches (to read)"])
        self.assertFalse(facts[(BINARY, "t")]["firm"])

    def test_a_test_that_only_passed_is_not_in_the_facts(self):
        self.assertEqual(judged([ci("t", "pass", "h1", "main")]), {})


class TheSoak(unittest.TestCase):
    def soak(self, passes):
        self.dir = tempfile.TemporaryDirectory()
        root = pathlib.Path(self.dir.name)
        (root / "soak.json").write_text(json.dumps({"head": "abc", "branch": "main"}), encoding="utf-8")
        for at, cases in enumerate(passes):
            cell = root / f"cell-{at % 2 + 1}"
            cell.mkdir(exist_ok=True)
            (cell / f"pass-{at}-mushd.xml").write_text(junit(cases), encoding="utf-8")
        return judged(report.from_dir(root))

    def tearDown(self):
        self.dir.cleanup()

    def test_failed_in_some_passes_and_passed_in_others_is_a_flake_with_its_rate(self):
        facts = self.soak([[(BINARY, "t", "fail")], [(BINARY, "t", "pass")], [(BINARY, "t", "pass")]])
        fact = facts[(BINARY, "t")]
        self.assertEqual((fact["soak"], fact["firm"], fact["broken"]), ((1, 3), True, False))

    def test_failed_in_every_pass_is_broken_not_flaky(self):
        facts = self.soak([[(BINARY, "t", "fail")], [(BINARY, "t", "fail")]])
        fact = facts[(BINARY, "t")]
        self.assertEqual((fact["soak"], fact["firm"], fact["broken"]), ((2, 2), False, True))

    def test_one_failed_pass_alone_says_nothing_yet(self):
        facts = self.soak([[(BINARY, "t", "fail")]])
        fact = facts[(BINARY, "t")]
        self.assertEqual((fact["firm"], fact["broken"]), (False, False))


class TheRecord(unittest.TestCase):
    RECORD = [
        ("7", "| 7 | `ic_slice_loop::ic12_part_one_a_leg_by_hand` | ... |"),
        ("17", "| 17 | `ic_buyout_dormancy::sm2_a_week_of_hops_trips_no_recall` | ... |"),
    ]

    def test_a_test_is_found_by_its_own_name(self):
        test = (BINARY, "ic_slice_loop::ic12_part_one_a_leg_by_hand")
        self.assertEqual(report.record_row(self.RECORD, test), "7")
        self.assertIsNone(report.record_row(self.RECORD, (BINARY, "a_module::nobody_wrote_down")))

    def test_a_story_run_at_named_phases_is_found_by_the_story(self):
        for phase in report.PHASE_TESTS:
            test = (BINARY, f"ic_buyout_dormancy::sm2_a_week_of_hops_trips_no_recall::{phase}")
            self.assertEqual(report.record_row(self.RECORD, test), "17")
        # A phase's word alone names no story.
        self.assertIsNone(report.record_row(self.RECORD, (BINARY, "mid_month")))


class TheList(unittest.TestCase):
    def test_a_row_covers_a_test_a_module_or_a_binary(self):
        rows = [
            row(test="observer_rule::quiet"),
            row(module="sim_reflex", **{"class": "unclassed"}),
            row(binary="mush-worldgen::plan", whole_binary=True),
        ]
        self.assertIs(report.row_for(rows, (BINARY, "observer_rule::quiet")), rows[0])
        self.assertIsNone(report.row_for(rows, (BINARY, "observer_rule::quiet_too")))
        self.assertIs(report.row_for(rows, (BINARY, "sim_reflex::m3")), rows[1])
        self.assertIsNone(report.row_for(rows, (BINARY, "sim_reflexes::m3")))
        self.assertIs(report.row_for(rows, ("mush-worldgen::plan", "anything")), rows[2])

    def test_a_row_holds_through_its_last_day(self):
        one = row(test="t", expires="2026-11-23")
        self.assertFalse(report.expired(one, datetime.date(2026, 11, 23)))
        self.assertTrue(report.expired(one, datetime.date(2026, 11, 24)))

    def render(self, seen, rows):
        return report.render(judged(seen), rows, [], ["a test"], NOW)

    def test_check_names_the_unlisted_the_expired_and_spares_the_lead(self):
        seen = [
            ci("listed", "retry", "h1", "main"),
            ci("unlisted", "retry", "h1", "main"),
            ci("old", "retry", "h1", "main"),
            ci("lead", "fail", "h1", "one"),
            ci("lead", "fail", "h2", "two"),
        ]
        rows = [row(test="listed"), row(test="old", expires="2026-11-01")]
        lines, unlisted, over, broken, stale = self.render(seen, rows)
        self.assertEqual(unlisted, [(BINARY, "unlisted")])
        self.assertEqual(over, [(BINARY, "old")])
        self.assertEqual(broken, [])
        self.assertEqual([r["test"] for r in stale], ["old"])
        text = "\n".join(lines)
        self.assertIn("**NOT LISTED**", text)
        self.assertIn("**EXPIRED 2026-11-01**", text)
        self.assertIn("2 branches (to read)", text)

    def test_the_shipped_list_parses_and_every_row_has_a_binary_and_a_date(self):
        # The list may be empty (doc 51 unit FL.10 emptied it): it must parse.
        rows = report.read_known(report.KNOWN)
        self.assertIsInstance(rows, list)
        for one in rows:
            self.assertIn("binary", one)
            datetime.date.fromisoformat(one["expires"])


if __name__ == "__main__":
    unittest.main()
