"""Summarize one MyPro v1.0.1 session using only the native diagnostic records."""
import argparse
import csv
import json
import math
import re
from collections import Counter
from datetime import datetime, timedelta, timezone
from pathlib import Path


RECORD = re.compile(
    r"^(\S+)\s+\w+\s+(?:MyPro::(?:app|memory)|ruffle_core::player):\s+"
    r"(BATTLE_SCOPE(?:_RECLAIM|_ALIVE_TOP|_TAGGED_TOP)?|BATTLE_MEM|BATTLE_FOLLOWUP_MEM|MOVIE_RECLAIM(?:_SEC)?|"
    r"HEAP_CENSUS|HEAP_CREATED|HEAP_STAGE)\s+(.*)$"
)
ANSI = re.compile(r"\x1b\[[0-9;]*m")
NUMERIC = re.compile(r"(\w+)=(-?\d+(?:\.\d+)?)\b")
GC_SUMS = ("calls", "rounds", "gc_ms", "mark_ms", "sweep_ms", "finalize_ms",
           "demoted", "resurrected", "freed")


def timestamp(value):
    return datetime.fromisoformat(value.replace("Z", "+00:00"))


def fields(message):
    return {k: float(v) if "." in v else int(v) for k, v in NUMERIC.findall(message)}


def which_battle(battles, start, end):
    if start is None:
        return None
    for battle in battles:
        if battle["intervalStartUtc"] <= start and (battle["endUtc"] is None or end <= battle["endUtc"]):
            return battle["id"]
    return None


def battle_from_settlement(message, now, previous_end):
    if message.split()[:2] != ["end", "protocol_settled"]:
        return None
    duration = re.search(r"(?:^|\s)battle_seconds=(\d+\.\d)(?:\s|$)", message)
    if not duration:
        return None
    seconds = float(duration[1])
    if not seconds or not math.isfinite(seconds):
        return None
    # The native duration displays tenths. Leave one display unit on either
    # side of the estimate for overlap checks and use the later edge for sums.
    margin = timedelta(milliseconds=100)
    try:
        estimate = timestamp(now) - timedelta(seconds=seconds)
        earliest = estimate - margin
    except OverflowError:
        return None
    if previous_end is not None and earliest < previous_end:
        return None
    return {"startUtc": estimate.isoformat(timespec="microseconds"),
            "startSource": "protocol-duration", "startReason": "end protocol_settled battle_seconds",
            "reportedBattleSeconds": seconds, "durationResolutionSeconds": 0.1,
            "intervalStartUtc": (estimate + margin).isoformat(timespec="microseconds"),
            "endUtc": now, "endReason": message}


def summarize_movie_events(rows):
    totals = {}
    for key in ("demoted", "resurrected", "freed", "rounds", "mark_ms", "sweep_ms", "finalize_ms", "reclaim_ms"):
        totals[key] = sum(row[key] for row in rows) if rows and all(key in row for row in rows) else None
    complete_timings = totals["mark_ms"] is not None and totals["sweep_ms"] is not None
    return {"events": len(rows) or None, **totals,
            "peakMarkMs": max(row["mark_ms"] for row in rows) if totals["mark_ms"] is not None else None,
            "peakSweepMs": max(row["sweep_ms"] for row in rows) if totals["sweep_ms"] is not None else None,
            "markPlusSweepMs": totals["mark_ms"] + totals["sweep_ms"] if complete_timings else None,
            "peakMarkPlusSweepMs": max(row["mark_ms"] + row["sweep_ms"] for row in rows) if complete_timings else None}


def summarize_log(path, launch_utc=None):
    battles, gc, snapshots, maintenance = [], [], [], []
    movie_events = []
    previous_snapshot = current_snapshot = None
    previous_end = launch_utc
    unmatched_ends = 0
    if not path.exists():
        return {"available": False, "battles": [], "heapSnapshots": [], "maintenance": []}
    with path.open(encoding="utf-8-sig", errors="replace") as log:
        for line in log:
            match = RECORD.match(ANSI.sub("", line.rstrip()))
            if not match:
                continue
            raw_time, kind, message = match.groups()
            # Convert all native timestamps to the same precision and offset before comparisons.
            now = timestamp(raw_time).astimezone(timezone.utc).isoformat(timespec="microseconds")
            if kind == "BATTLE_SCOPE":
                if message.startswith("start "):
                    if battles and battles[-1]["endUtc"] is None:
                        battles[-1].update(endUtc=now, endReason="next-start-without-end")
                    battles.append({"id": len(battles) + 1, "startUtc": now, "intervalStartUtc": now,
                                    "startSource": "native-start", "startReason": message,
                                    "endUtc": None, "endReason": None})
                elif message.startswith("end "):
                    if battles and battles[-1]["endUtc"] is None:
                        battles[-1].update(endUtc=now, endReason=message)
                    else:
                        inferred = battle_from_settlement(message, now, previous_end)
                        if inferred is None:
                            unmatched_ends += 1
                        else:
                            battles.append({"id": len(battles) + 1, **inferred})
                    previous_end = timestamp(now)
            elif kind == "MOVIE_RECLAIM_SEC":
                gc.append((now, fields(message)))
            elif kind == "MOVIE_RECLAIM":
                movie_events.append((now, fields(message)))
            elif kind == "HEAP_CENSUS":
                current_snapshot = {"utc": now, "sinceUtc": previous_snapshot,
                                    "census": fields(message), "reportedCreated": None}
                snapshots.append(current_snapshot)
                previous_snapshot = now
            elif kind == "HEAP_CREATED":
                # A filtered or partial log can start in the middle of a census.
                if current_snapshot is None or current_snapshot["reportedCreated"] is not None:
                    current_snapshot = {"utc": now, "sinceUtc": None, "census": None}
                    snapshots.append(current_snapshot)
                listed = message.removeprefix("created=")
                current_snapshot["reportedCreated"] = listed
                parsed = {}
                for item in listed.split(" | "):
                    name, separator, count = item.rpartition("=")
                    if separator and name and count.isdecimal():
                        parsed[name] = int(count)
                current_snapshot["listedClassCounts"] = parsed or None
            elif kind == "HEAP_STAGE":
                maintenance.append({"utc": now, "kind": kind, "values": {
                    k: v for k, v in fields(message).items() if k in ("on_stage", "orphans")}})
            elif kind in ("BATTLE_SCOPE_ALIVE_TOP", "BATTLE_SCOPE_TAGGED_TOP"):
                maintenance.append({"utc": now, "kind": kind, "reported": message})
            else:
                maintenance.append({"utc": now, "kind": kind, "values": fields(message)})

    movie_source = path.name if movie_events else None
    supplement = path.with_name("movie-reclaim.log")
    if not movie_events and supplement.is_file():
        with supplement.open(encoding="utf-8-sig", errors="replace") as extra:
            for line in extra:
                match = RECORD.match(ANSI.sub("", line.rstrip()))
                if match and match[2] == "MOVIE_RECLAIM":
                    now = timestamp(match[1]).astimezone(timezone.utc).isoformat(timespec="microseconds")
                    movie_events.append((now, fields(match[3])))
        if movie_events:
            movie_source = supplement.name

    for snap in snapshots:
        snap["wholeIntervalBattle"] = which_battle(battles, snap["sinceUtc"], snap["utc"])
    battle_gc_pairs = {}
    for (start, previous), (end, current) in zip(gc, gc[1:]):
        battle_id = which_battle(battles, start, end)
        if battle_id is not None:
            battle_gc_pairs.setdefault(battle_id, []).append((previous, current))
    battle_movie_events = {}
    for now, values in movie_events:
        battle_id = which_battle(battles, now, now)
        if battle_id is not None:
            battle_movie_events.setdefault(battle_id, []).append(values)
    for battle in battles:
        battle["movieReclaim"] = summarize_movie_events(battle_movie_events.get(battle["id"], []))
        pairs = battle_gc_pairs.get(battle["id"], [])
        rows = [current for _, current in pairs]
        counts = {key: sum(r[key] for r in rows) if rows and all(key in r for r in rows) else None
                  for key in GC_SUMS}
        counts["movieReclaimFreed"] = counts.pop("freed")
        readings = [pairs[0][0], *rows] if pairs else []
        live = [r["live_gc_objects"] for r in readings if "live_gc_objects" in r]
        drops = [max(0, earlier["live_gc_objects"] - later["live_gc_objects"]) for earlier, later in pairs
                 if "live_gc_objects" in earlier and "live_gc_objects" in later]
        battle["gc"] = {"completeNativeIntervals": len(rows), **counts,
                        "liveObjectsFirstLast": [live[0], live[-1]] if live else None,
                        "sampledLiveNodeDrops": sum(drops) if drops else None, "sampledLiveNodePairs": len(drops),
                        "allGcNodesFreed": None}
        intervals = [snap for snap in snapshots if snap["wholeIntervalBattle"] == battle["id"]]
        reported = [snap for snap in intervals if snap.get("listedClassCounts")]
        created = Counter()
        for snap in reported:
            created.update(snap["listedClassCounts"])
        battle["displayCreations"] = {"completeCensusIntervals": len(intervals),
                                      "intervalsWithClassCounts": len(reported),
                                      "reportedClassTotals": dict(created.most_common()) or None}
    return {"available": True, "battles": battles, "unmatchedEndMarkers": unmatched_ends,
            "movieReclaimSource": movie_source,
            "nativeGcIntervalsReported": len(gc), "heapSnapshots": snapshots, "maintenance": maintenance}


def summarize_csv(path):
    if not path.exists():
        return {"available": False}
    values = {key: [] for key in ("seconds", "fps", "render_ms", "live_gc_objects", "full_collections",
                                  "last_pause_ms", "private_mb")}
    rows = 0
    with path.open(encoding="utf-8-sig", newline="") as handle:
        for row in csv.DictReader(handle):
            rows += 1
            for key, series in values.items():
                try:
                    value = float(row[key])
                except (KeyError, TypeError, ValueError):
                    continue
                if math.isfinite(value):
                    series.append(value)
    return {"available": True, "rows": rows,
            "sessionFields": {key: {"first": series[0], "last": series[-1], "min": min(series),
                                    "max": max(series), "samples": len(series)} if series else None
                              for key, series in values.items()}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("session", type=Path, help="single run directory containing telemetry.log and mem.csv")
    parser.add_argument("--output", type=Path, help="write JSON to this file (default stdout)")
    args = parser.parse_args()
    if not args.session.is_dir():
        parser.error("session must be an existing run directory")
    session_path = args.session / "session.json"
    launch_utc = None
    if session_path.is_file():
        try:
            launch = json.loads(session_path.read_text(encoding="utf-8-sig"))["launchUtc"]
            parsed_launch = timestamp(launch)
            if parsed_launch.tzinfo is not None:
                launch_utc = parsed_launch
        except (ValueError, KeyError, TypeError, AttributeError):
            pass  # The native telemetry is still usable with incomplete launch metadata.
    report = {
        "telemetry": summarize_log(args.session / "telemetry.log", launch_utc),
        "memoryCsv": summarize_csv(args.session / "mem.csv"),
        "scope": {
            "battles": "Prefer an explicit native start. An otherwise unpaired end protocol_settled may give the start estimate from its positive battle_seconds, reported to tenths. For inferred starts, intervalStartUtc excludes the first 100 ms of uncertainty. Overlapping estimates and unsupported end markers are not used to create battles.",
            "gc": "Sum only native MOVIE_RECLAIM_SEC intervals with both report times inside one observed battle. sampledLiveNodeDrops adds observed decreases between adjacent live_gc_objects readings; allocations and releases between readings are unobserved. An open battle ends at available evidence. Movie reclaim freed and CSV full_collections have different scopes. Total freed GC nodes is unknown.",
            "movieReclaim": "MOVIE_RECLAIM events are assigned by their report timestamp. freed counts movie registry entries. Event timings are separate from MOVIE_RECLAIM_SEC and cannot be added to it. Null means no usable observation. A same-run movie-reclaim.log supplement is read only if telemetry.log has no MOVIE_RECLAIM events.",
            "created": "HEAP_CREATED lists up to 14 classes per census interval in the tested build. Missing reports and unlisted classes have no assigned count. A null wholeIntervalBattle means outside a single battle or unknown initial boundary. No orphan construct-frame traversal count is available.",
            "csv": "Session-wide native readings. CSV seconds are relative; telemetry uses UTC. Rows arrive about once per second; fps and render_ms can repeat until their separate update (about ten seconds in the offline probe). No per-battle assignment or equation between process memory and GC releases.",
        },
    }
    text = json.dumps(report, indent=2, ensure_ascii=False)
    if args.output:
        args.output.write_text(text + "\n", encoding="utf-8")
    else:
        print(text)


if __name__ == "__main__":
    main()
