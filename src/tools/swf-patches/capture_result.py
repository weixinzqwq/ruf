"""Check resource loads in an isolated run, including the TweenLite dependency during capture."""
import argparse
import json
import re
from collections import Counter
from pathlib import Path

from pilot_resource import BUILD_ID, RESOURCE as PET, UTIL_VERSION, VERSION

UTIL = "library/util"
# File prefix, version observed during official capture and comparison file.
RESOURCES = {
    PET: ("newpetpackage", VERSION, "newpetpackage-original.swf"),
    UTIL: ("library_util", UTIL_VERSION, f"library_util~{UTIL_VERSION}.swf"),
}

def summarize_operations(rows, mode, valid_run):
    counts = dict.fromkeys(("panel-created", "view-switch", "panel-cleanup", "classify-return"), 0)
    sequences = []
    pending = []
    previous_ms = -1
    hit_before_open_ms = None
    last_switch_to_cleanup_ms = None
    ignored = 0
    for entry in rows:
        if entry is None:  # A newly configured player starts a separate observation.
            pending = []
            previous_ms = -1
            continue
        row, current_hit_ms = entry
        name = row.get("operation")
        time_ms = row.get("elapsedMs")
        if (row.get("build") != BUILD_ID or not isinstance(name, str) or name not in counts
                or type(time_ms) is not int or time_ms < 0 or time_ms < previous_ms):
            ignored += 1
            pending = []
            continue
        previous_ms = time_ms
        counts[name] += 1
        if name == "panel-created":
            if len(pending) == 3:
                sequences.append(dict(created=pending[0], last_switch=pending[1], cleaned=pending[2],
                                      reopened=time_ms, switch_to_cleanup_ms=pending[2] - pending[1],
                                      resource_hit_before_open_ms=hit_before_open_ms))
            pending = [time_ms]
            hit_before_open_ms = current_hit_ms if current_hit_ms is not None and current_hit_ms <= time_ms else None
        elif name == "view-switch":
            if len(pending) in (1, 2):
                pending = [pending[0], time_ms]
            elif len(pending) == 3:
                pending = []
        elif name == "panel-cleanup":
            if len(pending) == 2:
                last_switch_to_cleanup_ms = time_ms - pending[1]
                pending.append(time_ms)
            else:
                pending = []
    return dict(requested_by_launcher=mode in ("original", "roundtrip", "lifecycle"), counts=counts,
                ignored_records=ignored, completed_method_sequences=len(sequences),
                last_switch_to_cleanup_ms=last_switch_to_cleanup_ms,
                reopened_with_cleanup_under_1000_ms=sum(seq["switch_to_cleanup_ms"] < 1000 for seq in sequences),
                verified_resource_quick_reopens=sum(seq["resource_hit_before_open_ms"] is not None
                                                    and seq["switch_to_cleanup_ms"] < 1000
                                                    for seq in sequences) if valid_run else 0,
                sequences_elapsed_ms=sequences)


def summarize(run, mode, game_exit=0, reference_dir=None):
    logs = list((run / "diagnostics").glob("startup-*.jsonl"))
    failures = set()
    ready = 0
    player_configured = False
    target_hit_ms = None
    confirmed_players = []
    util_confirmed_in_player = False
    expected = (PET, UTIL)
    sources = {key: Counter() for key in expected}
    versions = {key: set() for key in expected}
    sizes = {key: set() for key in expected}
    references = {}
    operations = []
    reference_dir = reference_dir or run.parent.parent / "swf-lifecycle"
    if game_exit != 0:
        failures.add("game-exited-with-error")
    if len(logs) != 1:
        failures.add("expected-one-startup-log")
    else:
        with logs[0].open(encoding="utf-8-sig") as stream:
            for line in stream:
                try:
                    row = json.loads(line)
                except ValueError:
                    failures.add("incomplete-startup-log")
                    continue
                if not isinstance(row, dict):
                    continue
                if row.get("type") == "swf-operation":
                    operations.append((row, target_hit_ms))
                    continue
                if row.get("type") != "swf-lifecycle":
                    continue
                result = row.get("result")
                source = row.get("source")
                if source == "config":
                    operations.append(None)
                    util_confirmed_in_player = False
                    player_configured = False
                    target_hit_ms = None
                if row.get("mode") != mode or row.get("build") != BUILD_ID:
                    failures.add("unexpected-experiment")
                    continue
                if source == "config":
                    if result == "ready":
                        ready += 1
                        player_configured = True
                    elif result in ("conflicting-modes", "invalid-configuration", "missing-data-directory"):
                        failures.add(result)
                    else:
                        failures.add("unexpected-configuration")
                    continue
                key = row.get("resource")
                if not isinstance(key, str) or key not in expected:
                    failures.add("unexpected-resource")
                    continue
                expected_result = "captured" if mode == "capture" else "hit"
                if mode != "capture" and key == UTIL:
                    expected_result = "dependency-confirmed"
                if result != expected_result or source not in ("cache", "network"):
                    allowed = {"version-mismatch", "source-mismatch", "unexpected-response", "capture-failed",
                               "read-failed", "streaming-official", "dependency-unconfirmed", "dependency-mismatch",
                               "unexpected-identity", "unsupported-request", "fetch-failed"}
                    reason = result if isinstance(result, str) and result in allowed else "unexpected-event"
                    failures.add(f"{key}: {reason}")
                    continue
                version = row.get("version")
                size = row.get("bytes")
                if not isinstance(version, str) or not re.fullmatch("[0-9]{1,24}", version) or type(size) is not int or size < 8:
                    failures.add(f"{key}: invalid-resource-record")
                    continue
                prefix, reference_version, _ = RESOURCES[key]
                if mode == "capture":
                    swf = run / "swf-capture" / f"{prefix}~{version}.swf"
                    if not swf.is_file() or swf.stat().st_size != size:
                        failures.add(f"{key}: missing-or-incomplete-capture")
                else:
                    if version != reference_version:
                        failures.add(f"{key}: unexpected-version")
                    name = RESOURCES[UTIL][2] if key == UTIL else f"newpetpackage-{mode}.swf"
                    bundled = reference_dir / name
                    if not bundled.is_file() or bundled.stat().st_size != size:
                        failures.add(f"{key}: delivered-size-differs-from-package")
                if mode != "capture":
                    if key == UTIL:
                        util_confirmed_in_player = True
                    elif not util_confirmed_in_player:
                        failures.add("pet/newpetpackage: dependency-unconfirmed")
                if key == PET and result == "hit" and player_configured and target_hit_ms is None:
                    elapsed = row.get("elapsedMs")
                    if type(elapsed) is int and elapsed >= 0:
                        target_hit_ms = elapsed
                        if util_confirmed_in_player:
                            confirmed_players.append({"generation": ready, "hit_elapsed_ms": elapsed})
                versions[key].add(version)
                sizes[key].add(size)
                sources[key][source] += 1
    if not ready:
        failures.add("configuration-unconfirmed")
    for key in expected:
        if len(versions[key]) != 1:
            failures.add(f"{key}: expected-one-observed-version")
        if mode != "capture":
            continue
        if not sources[key]["network"]:
            failures.add(f"{key}: no-network-response-in-this-run")
        prefix, reference_version, filename = RESOURCES[key]
        swf = run / "swf-capture" / f"{prefix}~{reference_version}.swf"
        reference = reference_dir / filename
        if len(versions[key]) != 1:
            references[key] = "unconfirmed"
        elif reference_version not in versions[key]:
            references[key] = "different-version"
        elif not swf.is_file() or not reference.is_file():
            references[key] = "comparison-file-missing"
        else:
            references[key] = "byte-identical" if swf.read_bytes() == reference.read_bytes() else "different-bytes"
    return dict(mode=mode, game_exit=game_exit, configured=bool(ready), configured_players=ready,
                confirmed_players=confirmed_players,
                resources={k: dict(versions=sorted(versions[k]), delivered_bytes=sorted(sizes[k]),
                                   sources=dict(sources[k])) for k in expected},
                archived_reference_comparison=references, failures=sorted(failures),
                operations=summarize_operations(operations, mode, not failures),
                resource_load_verified=not failures)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("run", type=Path)
    parser.add_argument("mode", choices=["capture", "original", "roundtrip", "lifecycle"])
    parser.add_argument("--game-exit", type=int, default=0)
    parser.add_argument("--reference-dir", type=Path, help="Packaged swf-lifecycle directory if run was moved")
    args = parser.parse_args()
    report = summarize(args.run, args.mode, args.game_exit, args.reference_dir)
    result = args.run / "resource-load.json"
    result.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(result)
    print("Resource load verified." if report["resource_load_verified"] else
          "Invalid experiment: " + ", ".join(report["failures"]))
    if report["operations"]["requested_by_launcher"]:
        operations = report["operations"]
        print("Recorded open/switch/cleanup/reopen sequences:", operations["completed_method_sequences"])
        print("Sequences with last switch to cleanup under 1000 ms:", operations["reopened_with_cleanup_under_1000_ms"])
        print("Quick reopens after the resource hit in the same player:", operations["verified_resource_quick_reopens"])
    for resource, status in report["archived_reference_comparison"].items():
        print(f"Archived {resource}: {status}")
    raise SystemExit(0 if report["resource_load_verified"] else 2)


if __name__ == "__main__":
    main()
