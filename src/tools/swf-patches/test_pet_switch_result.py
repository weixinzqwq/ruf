"""Regression cases for the resource, player, sample-window and A/B comparison boundaries."""
import json
from pathlib import Path
import tempfile
import unittest

from capture_result import PET, UTIL
from pet_switch_result import compare, MODES, PERF_BUILD, ROOTS, summarize
from pilot_resource import BUILD_ID, UTIL_VERSION, VERSION


def write_rows(path, rows):
    path.write_text("".join(json.dumps(row) + "\n" for row in rows), encoding="utf8")


class PetSwitchResultTest(unittest.TestCase):
    def setUp(self):
        self.work = tempfile.TemporaryDirectory()
        self.addCleanup(self.work.cleanup)
        self.package = Path(self.work.name)
        assets = self.package / "swf-lifecycle"
        assets.mkdir()
        for name in ("newpetpackage-original.swf", f"library_util~{UTIL_VERSION}.swf"):
            (assets / name).write_bytes(b"testdata")

    def run_data(self, mode, changed_window=False, late_hit=False, second_player=False, false_setting=False):
        run = self.package / "runs" / mode
        diag = run / "diagnostics"
        diag.mkdir(parents=True)
        event = {"type": "swf-lifecycle", "build": BUILD_ID, "mode": "original"}
        ready = {**event, "source": "config", "result": "ready", "elapsedMs": 5}
        util = {**event, "source": "network", "resource": UTIL, "result": "dependency-confirmed",
                "version": UTIL_VERSION, "bytes": 8, "elapsedMs": 12}
        hit = {**event, "source": "network", "resource": PET, "result": "hit",
               "version": VERSION, "bytes": 8, "elapsedMs": 25 if late_hit else 15}
        write_rows(diag / "startup-1000-45.jsonl", [ready, util, hit] + ([dict(ready, elapsedMs=28)] if second_player else []))
        fast, profile = MODES[mode]
        deep = profile == "full"
        samples = []
        for i, elapsed in enumerate((10, 20, 30, 40)):
            entry = {"processId": 45, "build": BUILD_ID, "petSwitchBuild": PERF_BUILD, "hostElapsedMs": elapsed,
                     "gotoLeafFastPath": not fast if false_setting else fast, "detailProfile": profile, "gotoProfile": deep,
                     "movieGeneration": 2 if second_player and i >= 2 else 1, "final": i == 3,
                     "subtreeScriptSleep": False, "nativeFrameRateForced": True, "runtimeFrameRateSetting": 50,
                     "optimizationMode": {"sleep": True, "loading": False, "dedup": False},
                     "window": {"physicalWidth": 900 if changed_window and i >= 2 else 960, "physicalHeight": 560, "dpiScale": 1.25},
                     "internalViewport": {"width": 960, "height": 560, "scaleFactor": 1.25}}
            responses = []
            if i == 2:
                for seq in range(1, 9):
                    response = {"root": sorted(ROOTS)[0], "stage": "sockets", "sequence": seq, "totalMs": seq + .25}
                    if deep:
                        construct = {"name": "orphanConstruct", "calls": 2, "exclusiveMs": 5.5,
                                     "counts": {"movieClipConstructVisits": 50, "movieClipConstructLeafHits": 25}}
                        response.update(goto={"enabled": True, "calls": 3, "phases": [construct]},
                                        methods=[{"calls": 1}], orphanCensus={"slots": 1})
                    responses.append(response)
            entry["petDetailResponses"] = {"responses": responses, "responseCount": len(responses), "mode": profile,
                                           "methodBreakdownMeasured": deep, "droppedResponses": 0}
            samples.append(entry)
        write_rows(diag / "performance-10000000-45.jsonl", samples)
        (run / "pet-switch-result.json").write_text('{"game_exit": 0}', encoding="utf8")
        return run

    def test_matched_pair_and_independent_depth(self):
        a, b = self.run_data("A"), self.run_data("B")
        report = compare(a, b)
        self.assertTrue(report["protocol_match"])
        self.assertEqual(report["cohorts"][0]["A"]["stats"], {"n": 8, "median_ms": 4.75, "max_ms": 8.25})
        deep = summarize(self.run_data("deep"), "deep")
        self.assertFalse(deep["failures"])
        self.assertEqual(deep["responses"][0]["deep"]["goto_calls"], 3)
        phases = deep["responses"][0]["deep"]["orphan_phases"]
        self.assertEqual(phases["orphanConstruct"]["leaf_hit_share"], .5)
        self.assertEqual(phases["orphanScripts"], {"status": "no-phase-work-recorded"})

    def test_sample_crosses_gates_resize_or_player_restart(self):
        for mode, arg, reason in (("A", "late_hit", "sample-interval-crosses-resource-load"),
                                  ("B", "changed_window", "sample-interval-crosses-window-change"),
                                  ("deep", "second_player", "resource-unconfirmed-in-this-player")):
            run = self.run_data(mode, **{arg: True})
            report = summarize(run, mode)
            self.assertTrue(report["resource_load_verified"])
            self.assertEqual(report["exclusions"], {reason: 8})
            self.assertEqual(report["eligible_roots"], 0)

    def test_wrong_effective_configuration_blocks_comparison(self):
        a, b = self.run_data("A"), self.run_data("B", false_setting=True)
        report = compare(a, b)
        self.assertFalse(report["protocol_match"])
        self.assertIn("B: unexpected-engine-configuration", report["failures"])


if __name__ == "__main__":
    unittest.main()
