"""Summarize one SWF lifecycle run from its resource-load report and performance samples."""

import argparse
from itertools import groupby
import json
import math
from pathlib import Path
import sys


GC_FIELDS = ("knownMovies", "knownMovieBytes", "libraryCharacters",
             "libraryUploadedBitmaps", "libraryGraphicMeshEntries")
ACTION_FIELDS = ("requested_by_launcher", "ignored_records", "completed_method_sequences",
                 "last_switch_to_cleanup_ms", "reopened_with_cleanup_under_1000_ms",
                 "verified_resource_quick_reopens")
ACTION_NAMES = ("panel-created", "view-switch", "panel-cleanup", "classify-return")
SEQUENCE_FIELDS = ("created", "last_switch", "cleaned", "reopened",
                   "switch_to_cleanup_ms", "resource_hit_before_open_ms")


def selected(record, keys):
    return {key: record[key] for key in keys if key in record}


def resource_summary(report):
    if not isinstance(report, dict) or not isinstance(report.get("resources"), dict):
        raise ValueError("resource-load.json has no resource summary")
    result = selected(report, ("mode", "game_exit", "configured", "resource_load_verified", "failures"))
    result["resources"] = {}
    for key in ("pet/newpetpackage", "library/util"):
        if key in report["resources"]:
            row = report["resources"][key]
            result["resources"][key] = {**selected(row, ("versions", "delivered_bytes")),
                                        "sources": selected(row.get("sources", {}), ("network", "cache"))}
    operations = report.get("operations")
    result["operations"] = None
    if isinstance(operations, dict):
        result["operations"] = {
            **selected(operations, ACTION_FIELDS),
            "counts": selected(operations.get("counts", {}), ACTION_NAMES),
            "sequences_elapsed_ms": [selected(seq, SEQUENCE_FIELDS) for seq in operations.get("sequences_elapsed_ms", [])],
        }
    return result


def window_settings(row):
    def dimensions(field, keys):
        values = selected(row.get(field) or {}, keys)
        return {key: value for key, value in values.items()
                if type(value) in (float, int) and math.isfinite(value) and value > 0}

    return {
        "window": dimensions("window", ("physicalWidth", "physicalHeight", "dpiScale")),
        "viewport": dimensions("internalViewport", ("width", "height", "scaleFactor")),
        **selected(row, ("runtimeFrameRateSetting", "nativeFrameRateForced", "subtreeScriptSleep")),
    }


def window_summary(settings, rows, start):
    # An interval's counters start at the preceding sample. Skip the first sample
    # of each segment so a resize or player restart cannot mix two settings.
    seconds = logical = presented = 0.0
    intervals = 0
    dimensions_recorded = len(settings["window"]) == 3 and len(settings["viewport"]) == 3
    for row in rows[1:] if dimensions_recorded else ():
        data = row.get("performanceInterval") or {}
        if data.get("available") is not True:
            continue
        duration, logical_fps, presented_fps = (data.get(k) for k in ("seconds", "logicalFps", "presentedFps"))
        if not all(type(x) in (int, float) and math.isfinite(x) and x >= 0
                   for x in (duration, logical_fps, presented_fps)) or duration == 0:
            raise ValueError("invalid performance interval counters")
        seconds += duration
        logical += duration * logical_fps
        presented += duration * presented_fps
        intervals += 1
    return {
        **settings, "samples": len(rows), "first_offset_ms": rows[0]["unixMs"] - start,
        "last_offset_ms": rows[-1]["unixMs"] - start, "same_settings_intervals": intervals,
        "measured_seconds": round(seconds, 6),
        "logical_fps": round(logical / seconds, 4) if intervals else None,
        "engine_presented_fps": round(presented / seconds, 4) if intervals else None,
    }


def gc_summary(rows, start):
    snapshots = {}
    for row in rows:
        gc = row.get("gc") or {}
        cycle = gc.get("libraryInventoryCompletedCycle")
        counts = selected(gc, GC_FIELDS)
        if gc.get("libraryInventoryScope") != "post-collection" or type(cycle) is not int or cycle < 1 or not counts:
            continue
        snapshots.setdefault(cycle, {"cycle": cycle, "sample_offset_ms": row["unixMs"] - start, **counts})
    if not snapshots:
        return None
    observed = list(snapshots.values())
    return {"distinct_observed_cycles": len(observed), "first": observed[0],
            "last_distinct": observed[-1] if len(observed) > 1 else None}


def inspect(run):
    logs = list((run / "diagnostics").glob("performance-*.jsonl"))
    if len(logs) != 1:
        raise ValueError(f"expected one performance log, found {len(logs)}")
    report = json.loads((run / "resource-load.json").read_text(encoding="utf-8-sig"))
    rows = []
    with logs[0].open(encoding="utf-8-sig") as stream:
        for line_number, line in enumerate(stream, 1):
            try:
                row = json.loads(line)
            except ValueError as error:
                raise ValueError(f"invalid performance JSON on line {line_number}") from error
            if not isinstance(row, dict) or type(row.get("unixMs")) is not int or not isinstance(row.get("build"), str):
                raise ValueError(f"missing timestamp or build on line {line_number}")
            rows.append(row)
    if not rows:
        raise ValueError("empty performance log")
    if len({row["build"] for row in rows}) != 1 or len({row.get("processId") for row in rows}) != 1:
        raise ValueError("multiple builds or processes in one log")
    if any(a["unixMs"] > b["unixMs"] or a.get("final") is True for a, b in zip(rows, rows[1:])):
        raise ValueError("out-of-order timestamp or records after final sample")
    start = rows[0]["unixMs"]
    generations = []
    for generation, group in groupby(rows, key=lambda row: row.get("movieGeneration")):
        current = list(group)
        if generation is None:
            generations.append({"movie_generation": None, "window_segments": None, "post_collection": None})
            continue
        segments = [window_summary(settings, list(part), start)
                    for settings, part in groupby(current, key=window_settings)]
        generations.append({"movie_generation": generation, "window_segments": segments,
                            "post_collection": gc_summary(current, start)})
    return {
        "run": run.name, "performance_log": logs[0].name, "resource_load_report": resource_summary(report),
        "build": rows[0]["build"], "samples": len(rows), "sample_span_ms": rows[-1]["unixMs"] - start,
        "final_sample": rows[-1].get("final"), "generations": generations,
        "scope": "FPS uses recorded intervals bounded by equal window/viewport settings in one player. "
                 "GC inventories describe the whole movie generation. Operation elapsed times use the startup clock.",
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("run", type=Path, help="one run directory containing resource-load.json and diagnostics/")
    args = parser.parse_args()
    try:
        report = inspect(args.run)
    except (OSError, ValueError) as error:
        sys.exit(str(error))
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
