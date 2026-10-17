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
            "Rounds fired 10; candidate rounds left 20; rules fired 10, held 2; faults 0.",
            *[f"Seed {seed}: normal Win; swapped Some(Loss)." for seed in range(15)],
        ]}
        envelope = acceptance.read(acceptance.ENVELOPE)
        definition = "6" * 64
        cards = {"selected":[{"name":"Hold", "key":{"index":0}}], "definition":{
            "schema":envelope["schema"], "profiles":[{
                "name":"Hold", "values":envelope["profiles"]["Hold"],
                "rules":envelope["rules"].get("Hold", [])}]}}
        seal = {"class":"II", "engine":"scripted", "source_sha256":source,
                "cards":cards, "card_digest":"7" * 64,
                "requested":{"mind":"FieldPilot", "source_sha256":source,
                             "origin":{"digest":definition}}}
        opponent = {"class":"I", "source_sha256":context["opponent"]["source_sha256"],
                    "requested":{"mind":"Standard"}, "cards":{
                        "selected":[{"name":"Line", "key":{"index":0}}],
                        "definition":{"profiles":[{"name":"Line"}]}}}
        self.frozen = {"context":context, "games":[{
            "seed":index // 2, "swapped":index % 2 == 1,
            "seals":[copy.deepcopy(seal), copy.deepcopy(opponent)]} for index in range(30)]}
        self.proving["key"] = {"account":"test-account", "run":1, "part":"result"}
        self.result = {"context":context, "report":{"games":[{
            "seed":index // 2, "swapped":index % 2 == 1,
            "outcome":"win" if index % 2 == 0 else "loss", "faults":0,
        } for index in range(30)]}}
        result_bytes = self.write("author/proving-result.json", self.result)
        self.proving["artifact"] = self.descriptor(result_bytes)
        self.start = {"key":{**self.proving["key"], "part":"start"}}
        self.story = {**metadata, "first_definition":definition, "first_seal":seal,
                      "proving_result_artifact":self.proving["artifact"],
                      "checks": [{"armed": armed, "fault": None, "station_refusals": [],
                                  "class_violations": [], "operations": 20, "fees": 4}
                                 for armed in [False, True] for _ in range(3)]}
        self.update_frozen()
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

    @staticmethod
    def descriptor(encoded):
        return {"bytes":len(encoded), "sha256":list(hashlib.sha256(encoded).digest())}

    def update_frozen(self):
        encoded = self.write("author/proving-start.json", self.frozen)
        self.start["artifact"] = self.descriptor(encoded)
        self.write("author/proving-start-manifest.json", self.start)
        self.story["proving_start_artifact"] = self.start["artifact"]
        self.update_proving()

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
        self.assertTrue(result["field_pilot_rate_in_external_wilson_95"])
        self.assertTrue(result["acceptance_passed"])
        self.assertEqual(result["proving_faults"]["all_minds"], 0)
        self.assertIsNone(result["proving_faults"]["candidate"])

    def update_proving(self):
        proof = self.write("author/proving-manifest.json", self.proving)
        self.story["proving_manifest_sha256"] = hashlib.sha256(proof).hexdigest()
        self.write("author/author-story.json", self.story)

    def update_result(self):
        encoded = self.write("author/proving-result.json", self.result)
        self.proving["artifact"] = self.descriptor(encoded)
        self.story["proving_result_artifact"] = self.proving["artifact"]
        self.update_proving()

    def set_embedded_wins(self, wins):
        outcomes = ["Win"] * wins + ["Loss"] * (30 - wins)
        self.proving["summary"] = [
            f"Won {wins}; lost {30 - wins}; drew 0; timed out 0.",
            "Rounds fired 10; candidate rounds left 20; rules fired 10, held 2; faults 0.",
            *[f"Seed {seed}: normal {outcomes[2 * seed]}; swapped Some({outcomes[2 * seed + 1]})."
              for seed in range(15)],
        ]
        for index, game in enumerate(self.result["report"]["games"]):
            game["outcome"] = outcomes[index].lower()
        self.update_result()

    def test_overlapping_intervals_do_not_replace_the_point_rate_gate(self):
        # Both upper and lower misses can have overlapping Wilson intervals.
        # They must still fail doc 17's port-point-rate-in-bot-interval gate.
        for wins in [21, 3]:
            with self.subTest(wins=wins):
                self.set_embedded_wins(wins)
                with self.assertRaisesRegex(ValueError, "point win rate is outside"):
                    self.compare()
                result = acceptance.read(self.directory / "comparison.json")
                self.assertFalse(result["intervals_do_not_overlap"])
                self.assertFalse(result["field_pilot_rate_in_external_wilson_95"])
                self.assertFalse(result["acceptance_passed"])
                self.assertEqual(result["embedded_rhai"]["wins"], wins)
                self.assertEqual(result["fresh_external_bot"]["timeouts"], 4)
                self.assertEqual(result["context"], self.proving["context"])
                self.assertEqual(result["interpretation"], "Intervals overlap; no difference called")

    def test_separated_intervals_preserve_the_failed_complete_report(self):
        self.set_embedded_wins(30)
        with self.assertRaisesRegex(ValueError, "point win rate is outside"):
            self.compare()
        result = acceptance.read(self.directory / "comparison.json")
        self.assertTrue(result["intervals_do_not_overlap"])
        self.assertFalse(result["acceptance_passed"])
        self.assertEqual(result["embedded_rhai"]["samples"], 30)
        self.assertEqual(result["interpretation"], "95% intervals do not overlap")

    def test_actual_proving_faults_refuse_even_when_the_point_rate_passes(self):
        self.proving["summary"][1] = (
            "Rounds fired 10; candidate rounds left 20; rules fired 10, held 2; faults 3.")
        self.result["report"]["games"][7]["faults"] = 3
        self.update_result()
        with self.assertRaisesRegex(ValueError, "3 mind faults across both sides"):
            self.compare()
        result = acceptance.read(self.directory / "comparison.json")
        self.assertTrue(result["field_pilot_rate_in_external_wilson_95"])
        self.assertFalse(result["acceptance_passed"])
        self.assertEqual(result["proving_faults"]["all_minds"], 3)
        self.assertIsNone(result["proving_faults"]["candidate"])
        self.assertIsNone(result["proving_faults"]["opponent"])

    def test_missing_or_duplicate_actual_fault_totals_are_not_assumed_zero(self):
        row = self.proving["summary"].pop(1)
        self.update_proving()
        with self.assertRaisesRegex(ValueError, "fault total is required"):
            self.compare()
        self.proving["summary"].extend([row, row])
        self.update_proving()
        with self.assertRaisesRegex(ValueError, "fault total is required"):
            self.compare()

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

    def test_rejects_changed_result_bytes_even_when_summary_matches(self):
        self.write("author/proving-result.json", {"context":self.proving["context"], "changed":True})
        with self.assertRaisesRegex(ValueError, "proving-result.json bytes differ"):
            self.compare()

    def test_rejects_missing_or_incomplete_actual_proving_games(self):
        for report in [{}, {"games":[]}, {"games":self.result["report"]["games"][:-1]}]:
            with self.subTest(report=report):
                self.result["report"] = report
                self.update_result()
                with self.assertRaisesRegex(ValueError, "thirty actual retained proving game reports"):
                    self.compare()

    def test_rejects_actual_game_seed_or_swapped_order_mismatch(self):
        original = copy.deepcopy(self.result)
        for key, value in [("seed", 999), ("swapped", False)]:
            with self.subTest(key=key):
                self.result = copy.deepcopy(original)
                self.result["report"]["games"][7][key] = value
                self.update_result()
                with self.assertRaisesRegex(ValueError, "game seed/side order differs"):
                    self.compare()

    def test_rejects_actual_outcome_that_disagrees_with_summary(self):
        self.result["report"]["games"][7]["outcome"] = "win"
        self.update_result()
        with self.assertRaisesRegex(ValueError, "actual proving outcomes disagree"):
            self.compare()

    def test_rejects_missing_actual_proving_outcome(self):
        del self.result["report"]["games"][7]["outcome"]
        self.update_result()
        with self.assertRaisesRegex(ValueError, "game outcome is missing"):
            self.compare()

    def test_rejects_actual_faults_that_disagree_with_summary(self):
        self.result["report"]["games"][7]["faults"] = 1
        self.update_result()
        with self.assertRaisesRegex(ValueError, "actual proving fault counts disagree"):
            self.compare()

    def test_rejects_actual_wrong_revision_with_unchanged_requested_context(self):
        self.frozen["games"][7]["seals"][0]["requested"]["origin"]["digest"] = "8" * 64
        self.update_frozen()
        with self.assertRaisesRegex(ValueError, "first exact sealed revision/card/rules"):
            self.compare()

    def test_rejects_actual_changed_card_with_same_source_and_profile_name(self):
        self.frozen["games"][7]["seals"][0]["cards"]["definition"]["profiles"][0]["values"]["band_m"] = 400
        self.update_frozen()
        with self.assertRaisesRegex(ValueError, "first exact sealed revision/card/rules"):
            self.compare()

    def test_rejects_actual_different_opponent_profile(self):
        self.frozen["games"][7]["seals"][1]["cards"]["selected"][0]["name"] = "Sniper"
        self.update_frozen()
        with self.assertRaisesRegex(ValueError, "actual sealed selection is not Line"):
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
