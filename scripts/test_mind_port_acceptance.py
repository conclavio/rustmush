"""Synthetic validation fixtures only; these never count as pilot acceptance."""
import contextlib
import copy
import hashlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import mind_port_acceptance as acceptance


class ComparisonTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="m6-comparison-test-")
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        (self.directory / "author").mkdir()
        (self.directory / "external").mkdir()
        source = acceptance.read(acceptance.MANIFEST)["source_sha256"]
        context = {
            "scenario": "muster", "scenario_content_sha256": "1" * 64,
            "ground": "test-ground", "ground_sha256": "2" * 64,
            "config_sha256": "3" * 64, "chassis": "test-chassis",
            "startup_ms": 2000, "max_duration_ms": 600000, "native_substep_ms": 250,
            "candidate": {"class": "II", "profile": "Hold", "source_sha256": source},
            "opponent": {"class": "I", "profile": "Line", "source_sha256": "4" * 64},
            "pair_seeds": list(range(15)),
        }
        metadata = {"revision": "test-revision", "run_id": "test-run",
                    "started_unix_ms": 2, "finished_unix_ms": 3}
        self.write("acceptance-run.json", {**metadata, "started_unix_ms": 1})
        self.proving = {"context": context, "summary": [
            "Won 15; lost 15; drew 0; timed out 0.",
            *[f"Seed {seed}: normal Win; swapped Some(Loss)." for seed in range(15)],
        ]}
        proof = self.write("author/proving-manifest.json", self.proving)
        self.story = {**metadata, "proving_manifest_sha256": hashlib.sha256(proof).hexdigest(),
                      "first_seal": {"class": "II", "requested": {"source_sha256": source}},
                      "checks": [{"armed": armed, "fault": None, "station_refusals": [],
                                  "class_violations": [], "operations": 20, "fees": 4}
                                 for armed in [False, True] for _ in range(3)]}
        self.write("author/author-story.json", self.story)
        sides = []
        matches = []
        for side, other in [("red", "blue"), ("blue", "red")]:
            outcomes = [side] * 6 + [other] * 5 + ["draw"] * 2 + ["timeout"] * 2
            sides.append({"side": side, "samples": 15, "wins": 6, "losses": 5,
                          "draws": 2, "timeouts": 2})
            matches.extend({"side": side, "match": index, "outcome": outcome,
                            "ground_sha256": context["ground_sha256"],
                            "opponent": context["opponent"], "ledger": {"digest": "5" * 64}}
                           for index, outcome in enumerate(outcomes, 1))
        self.external = {**copy.deepcopy(context), **metadata, "per_side": 1, "samples": 30,
                         "native_substep_max_ms": 250, "sides": sides, "matches": matches,
                         "wins": 12, "losses": 10, "draws": 4, "timeouts": 4}
        self.write("external/external-interval.json", self.external)

    def write(self, relative, value):
        encoded = json.dumps(value, indent=2).encode("utf-8")
        (self.directory / relative).write_bytes(encoded)
        return encoded

    def compare(self):
        with patch.object(acceptance, "clean_revision", return_value="test-revision") as revision:
            with contextlib.redirect_stdout(io.StringIO()):
                acceptance.compare(self.directory)
            revision.assert_called_once_with("test-revision")
        return acceptance.read(self.directory / "comparison.json")

    def test_counts_all_actual_outcomes_on_both_sides(self):
        result = self.compare()
        self.assertEqual(result["fresh_external_bot"]["samples"], 30)
        self.assertEqual(result["fresh_external_bot"]["timeouts"], 4)
        self.assertFalse(result["intervals_do_not_overlap"])

    def test_rejects_consistent_totals_that_disagree_with_actual_records(self):
        self.external["sides"][0]["wins"] -= 1
        self.external["sides"][0]["losses"] += 1
        self.external["wins"] -= 1
        self.external["losses"] += 1
        self.write("external/external-interval.json", self.external)
        with self.assertRaisesRegex(ValueError, "actual match outcomes"):
            self.compare()

    def test_rejects_unknown_actual_end_instead_of_omitting_it(self):
        self.external["matches"][0]["outcome"] = "abort"
        self.write("external/external-interval.json", self.external)
        with self.assertRaisesRegex(ValueError, "unclassified actual match outcome"):
            self.compare()

    def test_rejects_missing_actual_outcome(self):
        del self.external["matches"][0]["outcome"]
        self.write("external/external-interval.json", self.external)
        with self.assertRaises(KeyError):
            self.compare()

    def test_rejects_replaced_proving_artifact_even_when_context_matches(self):
        self.proving["different_retained_artifact"] = True
        self.write("author/proving-manifest.json", self.proving)
        with self.assertRaisesRegex(ValueError, "exact result retained"):
            self.compare()


class RevisionTests(unittest.TestCase):
    def test_clean_same_revision_is_accepted(self):
        with patch.object(acceptance.subprocess, "check_output", side_effect=["old\n", ""]):
            self.assertEqual(acceptance.clean_revision("old"), "old")

    def test_head_change_refuses_later_command(self):
        with patch.object(acceptance.subprocess, "check_output", return_value="new\n"):
            with self.assertRaisesRegex(ValueError, "revision changed"):
                acceptance.clean_revision("old")

    def test_uncommitted_edit_refuses_later_command(self):
        with patch.object(acceptance.subprocess, "check_output", side_effect=["old\n", " M pilot.rs\n"]):
            with self.assertRaisesRegex(ValueError, "clean tree throughout"):
                acceptance.clean_revision("old")


if __name__ == "__main__":
    unittest.main()
