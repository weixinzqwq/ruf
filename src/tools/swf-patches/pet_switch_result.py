"""Measure individual pet detail root calls and compare two original-backpack runs."""
import argparse
from collections import Counter
import json
import math
from pathlib import Path
import statistics
import sys
from tempfile import NamedTemporaryFile

from capture_result import summarize as summarize_resources
from pilot_resource import BUILD_ID


PERF_BUILD = "pet-switch-leaf-v1"
MODES = {"A": (False, "summary"), "B": (True, "summary"), "deep": (True, "full")}
ROOTS = {"PetDataService/onViewSinglePet", "PetDetailDataMap/tryDispatcherCurrentDetailedCachedEvent"}
TARGET = 8


def sample_settings(row):
    window = row["window"]
    viewport = row["internalViewport"]
    settings = {"width": window["physicalWidth"], "height": window["physicalHeight"], "dpi": window["dpiScale"],
                "viewport_width": viewport["width"], "viewport_height": viewport["height"],
                "scale": viewport["scaleFactor"], "fps": row["runtimeFrameRateSetting"]}
    if not all(type(value) in (int, float) and math.isfinite(value) and value > 0 for value in settings.values()):
        raise ValueError("Missing or invalid viewport, frame rate or DPI")
    return settings


def orphan_phases(goto):
    phases = {p["name"]: p for p in goto["phases"]}
    observed = {}
    for name, counter in (("orphanConstruct", "movieClipConstruct"), ("orphanScripts", "movieClipScript")):
        phase = phases.get(name)
        if phase is None:
            observed[name] = {"status": "no-phase-work-recorded"}
            continue
        visits, hits = (phase["counts"][counter + suffix] for suffix in ("Visits", "LeafHits"))
        ms, calls = phase["exclusiveMs"], phase["calls"]
        if (type(visits) is not int or type(hits) is not int or not 0 <= hits <= visits
                or type(calls) is not int or calls < 0 or type(ms) not in (float, int) or not math.isfinite(ms) or ms < 0):
            raise ValueError(f"Invalid deep counters for {name}")
        observed[name] = {"status": "recorded", "calls": calls, "exclusive_ms": ms,
                          "movie_clip_visits": visits, "leaf_hits": hits, "leaf_hit_share": hits / visits if visits else None}
    return observed


def summarize(run, mode, game_exit=0):
    resources = summarize_resources(run, "original", game_exit)
    hits = {entry["generation"]: entry["hit_elapsed_ms"] for entry in resources["confirmed_players"]}
    failures = list(resources["failures"])
    fast_path, detail = MODES[mode]
    deep = detail == "full"
    rows = []
    logs = list((run / "diagnostics").glob("performance-*.jsonl"))
    startup = list((run / "diagnostics").glob("startup-*.jsonl"))
    if len(logs) != 1 or len(startup) != 1:
        raise ValueError("Expected one performance log and one startup log in this run")
    pid = int(startup[0].stem.rsplit("-", 1)[1])
    if logs[0].stem.rsplit("-", 1)[1] != str(pid):
        raise ValueError("Resource and performance logs refer to different processes")
    with logs[0].open(encoding="utf-8-sig") as stream:
        for line in stream:
            rows.append(json.loads(line))
    if not rows or rows[-1].get("final") is not True:
        failures.append("missing-final-sample")
    samples = []
    generations = set()
    previous = None
    previous_elapsed = -1
    previous_sequence = {}
    dropped = 0
    for index, row in enumerate(rows):
        if row["processId"] != pid or row["build"] != BUILD_ID or row["petSwitchBuild"] != PERF_BUILD:
            raise ValueError("Unexpected process or build")
        generation, elapsed = row["movieGeneration"], row["hostElapsedMs"]
        if (type(generation) is not int or generation < 1 or type(elapsed) is not int or elapsed < previous_elapsed
                or (previous and generation < previous[0]) or (index < len(rows) - 1 and row.get("final"))):
            raise ValueError("Invalid player generation or sample order")
        generations.add(generation)
        details = row["petDetailResponses"]
        if (row["gotoLeafFastPath"] is not fast_path or row["detailProfile"] != detail or row["gotoProfile"] is not deep
                or row["subtreeScriptSleep"] is not False or row["nativeFrameRateForced"] is not True
                or row["runtimeFrameRateSetting"] != 50 or details["mode"] != detail
                or details["methodBreakdownMeasured"] is not deep
                or row["optimizationMode"] != {"sleep": True, "loading": False, "dedup": False}):
            failures.append("unexpected-engine-configuration")
        settings = sample_settings(row)
        misses = details["droppedResponses"]
        responses = details["responses"]
        if type(misses) is not int or misses < 0 or details["responseCount"] != len(responses) + misses:
            raise ValueError("Inconsistent response count")
        dropped += misses
        for entry in responses:
            root, stage, seq, ms = (entry[k] for k in ("root", "stage", "sequence", "totalMs"))
            if (root not in ROOTS or not isinstance(stage, str) or not stage or type(seq) is not int
                    or seq <= previous_sequence.get(generation, 0)
                    or type(ms) not in (int, float) or not math.isfinite(ms) or ms < 0):
                raise ValueError("Invalid pet response")
            previous_sequence[generation] = seq
            result = dict(generation=generation, sequence=seq, root=root, stage=stage, total_ms=ms, settings=settings)
            if deep:
                census = entry.get("orphanCensus")
                if not isinstance(census, dict) or not census or not entry["goto"]["enabled"] or not entry["methods"]:
                    failures.append("missing-deep-measurements")
                else:
                    result["deep"] = {"method_rows": len(entry["methods"]), "goto_calls": entry["goto"]["calls"],
                                      "orphan_phases": orphan_phases(entry["goto"]),
                                      "orphan_census": census}
            elif any(k in entry and entry[k] is not None for k in ("orphanCensus", "goto", "methods")):
                failures.append("deep-breakdown-in-summary-mode")
            reason = None
            if generation not in hits or not resources["resource_load_verified"]:
                reason = "resource-unconfirmed-in-this-player"
            elif not previous or previous[0] != generation or previous[1] <= hits[generation]:
                reason = "sample-interval-crosses-resource-load"
            elif previous[2] != settings:
                reason = "sample-interval-crosses-window-change"
            result["excluded"] = reason
            samples.append(result)
        previous = (generation, elapsed, settings)
        previous_elapsed = elapsed
    if dropped:
        failures.append("responses-dropped")
    if sorted(generations) != list(range(1, resources["configured_players"] + 1)):
        failures.append("player-generation-coverage-mismatch")
    included = [entry for entry in samples if not entry["excluded"]]
    if not included:
        failures.append("no-confirmed-timed-root")
    if len({entry["generation"] for entry in included}) > 1:
        failures.append("responses-span-multiple-players")
    return {"mode": mode, "build": PERF_BUILD, "resource_build": BUILD_ID, "game_exit": game_exit,
            "resource_load_verified": resources["resource_load_verified"], "resources": resources["resources"],
            "samples": len(rows), "dropped_responses": dropped, "eligible_roots": len(included),
            "exclusions": dict(Counter(entry["excluded"] for entry in samples if entry["excluded"])),
            "failures": sorted(set(failures)), "responses": samples,
            "scope": "Each duration covers the outer detail method. Deep leaf hit shares use movie clip visits, "
                     "which may revisit the same object. Full diagnostic costs include profiling overhead."}


def metrics(values):
    return {"n": len(values), "median_ms": statistics.median(values), "max_ms": max(values)}


def compare(a, b):
    groups = [summarize(path, mode, read_exit(path)) for path, mode in ((a, "A"), (b, "B"))]
    failures = [f"{group['mode']}: {failure}" for group in groups for failure in group["failures"]]
    streams = [[r for r in group["responses"] if not r["excluded"]] for group in groups]
    def key(entry):
        return (entry["root"], entry["stage"], tuple(sorted(entry["settings"].items())))
    if any(len(stream) < TARGET for stream in streams):
        failures.append("fewer-than-eight-confirmed-roots")
    if [key(r) for r in streams[0]] != [key(r) for r in streams[1]]:
        failures.append("different-root-stage-or-window-sequence")
    cohorts = []
    for identity in sorted({key(r) for stream in streams for r in stream}):
        root, stage, settings = identity
        result = {"root": root, "stage": stage, "settings": dict(settings)}
        for name, stream in zip(("A", "B"), streams):
            rows = [r for r in stream if key(r) == identity]
            result[name] = {"calls": [{"sequence": r["sequence"], "ms": r["total_ms"]} for r in rows],
                            "stats": metrics([r["total_ms"] for r in rows]) if rows else None}
        cohorts.append(result)
    return {"A": a.name, "B": b.name, "build": PERF_BUILD, "eligible_roots": [len(s) for s in streams],
            "protocol_match": not failures, "failures": failures, "cohorts": cohorts,
            "scope": "Counts are observed outer root calls. The operator supplies the eight switches, "
                     "same map, selected pets, order and pacing. Root wall times exclude network wait before dispatch."}


def read_exit(path):
    # Retain the process result recorded by the launcher while rechecking the raw evidence.
    status = json.loads((path / "pet-switch-result.json").read_text(encoding="utf8"))["game_exit"]
    if type(status) is not int:
        raise ValueError("Missing launcher process result")
    return status


def choose_run(mode):
    directory = Path(__file__).resolve().parent / "runs"
    runs = sorted(p for p in directory.glob(f"{mode}-*") if (p / "pet-switch-result.json").is_file())
    if len(runs) == 1:
        return runs[0]
    if not runs:
        raise ValueError(f"No completed {mode} runs")
    for index, run in enumerate(runs, 1):
        print(f"{index}. {run.name}")
    index = int(input(f"Select {mode} run: "))
    if not 1 <= index <= len(runs):
        raise ValueError("Invalid run number")
    return runs[index - 1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    run = commands.add_parser("run")
    run.add_argument("directory", type=Path)
    run.add_argument("mode", choices=MODES)
    run.add_argument("--game-exit", type=int, required=True)
    pair = commands.add_parser("compare")
    pair.add_argument("a", nargs="?", type=Path)
    pair.add_argument("b", nargs="?", type=Path)
    pair.add_argument("--save-dir", type=Path)
    args = parser.parse_args()
    if args.command == "run":
        try:
            report = summarize(args.directory, args.mode, args.game_exit)
        except (ValueError, TypeError, KeyError, OSError) as error:
            report = {"mode": args.mode, "game_exit": args.game_exit, "eligible_roots": 0,
                      "failures": [f"invalid-evidence: {error}"]}
        path = args.directory / "pet-switch-result.json"
        path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf8")
        print(f"{report['eligible_roots']} pet detail calls in confirmed resource and window intervals. Files: {path}")
    else:
        if bool(args.a) != bool(args.b):
            parser.error("Supply both the A run directory and the B run directory")
        a, b = (args.a, args.b) if args.a else (choose_run("A"), choose_run("B"))
        report = compare(a, b)
        print(json.dumps(report, indent=2))
        if args.save_dir:
            args.save_dir.mkdir(exist_ok=True, parents=True)
            with NamedTemporaryFile(mode="w", encoding="utf8", prefix="pet-switch-", suffix=".json",
                                    delete=False, dir=args.save_dir) as stream:
                json.dump(report, stream, indent=2)
                stream.write("\n")
            print(f"Comparison saved to {stream.name}")
    if report["failures"]:
        print("Review: " + ", ".join(report["failures"]), file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (ValueError, TypeError, KeyError, OSError, EOFError) as error:
        sys.exit(f"Invalid pet switch run: {error}")
