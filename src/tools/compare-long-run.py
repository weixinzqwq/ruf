"""Summarize long-run windows from the first host-clock sample of the final player."""
import argparse
import json
from pathlib import Path


def read_run(path):
    with path.open(encoding="utf-8-sig") as lines:
        rows = [json.loads(line) for line in lines if line.strip()]
    known_builds = ("aola-long-run-v1", "aola-weak-keys-battle-gc-v1")
    if not rows or rows[-1].get("longRunBuild") not in known_builds or not rows[-1]["final"]:
        raise ValueError(f"Expected a completed long-run build log: {path}")
    for previous, current in zip(rows, rows[1:]):
        if current["hostElapsedMs"] < previous["hostElapsedMs"] or current["movieGeneration"] < previous["movieGeneration"]:
            raise ValueError(f"Out-of-order host clock or player generation: {path}")
    generation = rows[-1]["movieGeneration"]
    rows = [row for row in rows if row["movieGeneration"] == generation]
    if len(rows) < 2:
        raise ValueError(f"No game sampling intervals: {path}")
    return rows


def window_summary(rows):
    if not rows:
        return None
    intervals = [r["performanceInterval"] for r in rows]
    duration = sum(i["seconds"] for i in intervals)
    logical_frames = sum(i["logicalFps"] * i["seconds"] for i in intervals)
    histogram = intervals[0]["hostPresentGaps"]
    bounds = histogram["upperBoundsMs"]
    counts = [0] * (len(bounds) + 1)
    for i in intervals:
        gaps = i["hostPresentGaps"]
        if gaps["upperBoundsMs"] != bounds or len(gaps["counts"]) != len(counts):
            raise ValueError("Incompatible present interval buckets")
        counts = [a + b for a, b in zip(counts, gaps["counts"])]
    n = sum(counts)
    slow = sum(counts[bounds.index(50) + 1:])
    def upper(percent):
        if not n:
            return None
        cumulative = 0
        for bound, count in zip(bounds, counts):
            cumulative += count
            if 100 * cumulative >= percent * n:
                return bound
        return f">{bounds[-1]}"
    last = rows[-1]
    gpu = last["gpu"].get("resources") or {}
    memory = last.get("processMemory") or {}
    return dict(sampleSeconds=round(duration, 2), windows=sorted({tuple(r["window"][k] for k in
                ("physicalWidth","physicalHeight","dpiScale")) for r in rows}), presentIntervals=n,
                presentedFps=round(sum(i["presentedFps"]*i["seconds"] for i in intervals)/duration, 2),
                logicalFps=round(logical_frames / duration, 2),
                averageFrameMs=round(sum(i["frameCpuMs"] for i in intervals) / logical_frames, 2) if logical_frames else None,
                gcCpuMs=round(sum(i["gcCpuMs"] for i in intervals), 2),
                gcSlices=sum(i["gcSlices"] for i in intervals),
                p95UpperMs=upper(95), p99UpperMs=upper(99),
                hostPresentGaps={"upperBoundsMs": bounds, "counts": counts},
                intervalsOver50ms=slow,
                intervalsOver50msPer1000=round(1000 * slow / n, 2) if n else None,
                temporaryPoolHits=sum(i["temporaryPoolHits"] for i in intervals),
                temporaryPoolMisses=sum(i["temporaryPoolMisses"] for i in intervals),
                smallUploadSubmissionsSkipped=sum(i["renderCounts"]["smallUploadSubmissionsSkipped"] for i in intervals),
                directSubmissions=sum(i["renderCounts"]["directSubmissions"] for i in intervals),
                privateBytes=memory.get("privateBytes"), workingSet=memory.get("workingSet"),
                knownMovieBytes=last["gc"]["knownMovieBytes"], completedGcCycles=last["gc"]["completedCycles"],
                poolIdleBytes=gpu.get("temporaryPoolIdleAllocationBytes"), audioDefinitions=last.get("audioDefinitions"),
                battleMaintenance=last.get("battleMaintenance"))


def summarize(path):
    rows = read_run(path)
    first = rows[0]["hostElapsedMs"]
    seconds = (rows[-1]["hostElapsedMs"] - first) / 1000
    def select(start, end):
        return window_summary([r for r in rows if r["performanceInterval"]["available"] and
                               start < (r["hostElapsedMs"] - first) / 1000 <= end])
    # Assign intervals by their end timestamp. Include only reached milestones.
    windows = {f"minute{minute}": select(60*minute-120, 60*minute)
               for minute in (2, 10, 30, 60) if seconds >= 60*minute}
    windows["last2minutes"] = select(max(0, seconds-120), seconds)
    return dict(log=str(path.resolve()), seconds=round(seconds, 2), movieGeneration=rows[-1]["movieGeneration"],
                settings=rows[-1]["optimizationMode"], targetFps=rows[-1]["runtimeFrameRateSetting"],
                windows=windows)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("control", type=Path)
    parser.add_argument("optimized", type=Path)
    args = parser.parse_args()
    a, b = summarize(args.control), summarize(args.optimized)
    if a["settings"].get("longRunMaintenance") is not False or b["settings"].get("longRunMaintenance") is not True:
        parser.error("Expected original-maintenance log first, optimized log second")
    equal_settings = a["targetFps"] == b["targetFps"] and {**a["settings"], "longRunMaintenance":True} == b["settings"]
    print(json.dumps(dict(control=a, optimized=b, matchingEngineSettings=equal_settings,
                          scope="Whole samples are assigned by interval end on the host clock, relative to the first sample of the final player. Check window, map, order and battle count before comparing."),
                     ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
