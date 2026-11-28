"""Exercise the verified FightPanel bindings and disposal guard on an offline AVM2 player."""
import os
import re
import subprocess
from datetime import datetime

from test_notice_patch import probe_args, run_probe


def transitions(raw):
    plain = re.sub(r'\x1b\[[0-9;]*m', '', raw)
    rows = []
    for stamp, event, rest in re.findall(r'^(\S+)\s+INFO aola_swf_operation: operation="(battle-[a-z-]+)"(.*)$', plain, re.M):
        fields = {key: value.strip('"') for key, value in re.findall(r'(\w+)=("[^"]*"|\d+)', rest)}
        rows.append((datetime.fromisoformat(stamp.replace("Z", "+00:00")), event, fields))
    return rows


def verify_transitions(raw, policy):
    rows = transitions(raw)
    expected = ["battle-enter", "battle-exit", "battle-enter",
                "battle-enter", "battle-enter", "battle-exit", "battle-exit"]
    lifecycle = [event for _, event, _ in rows if not event.endswith("-gc")]
    assert lifecycle == expected, rows
    maintenance = [(stamp, event, fields) for stamp, event, fields in rows if event.endswith("-gc")]
    if policy == "off":
        assert not maintenance, maintenance
        return
    # The first exit has only one frame before the next fight cancels pending
    # work. The last fight (entry 4) always has time to receive both stages.
    main_ids = [fields["battleId"] for _, event, fields in maintenance if event == "battle-main-gc"]
    allowed_main_ids = (["4"], ["1", "4"]) if policy == "current" else ([],)
    assert main_ids in allowed_main_ids, maintenance
    (followup_time, followup), = [(stamp, fields) for stamp, event, fields in maintenance
                                  if event == "battle-followup-gc"]
    assert followup["battleId"] == "4" and followup["mode"] == "slice", maintenance
    for _, event, fields in maintenance:
        for key in ("registeredMovies", "registeredMoviesAfter", "nodesBefore", "nodesAfter", "dropped",
                    "freed", "cyclesCompleted", "weakEntriesPruned", "gcUs", "maintenanceUs"):
            assert int(fields[key]) >= 0, fields
        if event == "battle-main-gc":
            assert fields["mode"] == "incremental-drain" and fields["cyclesCompleted"] == "1", fields
    reference = "battle-main-gc" if policy == "current" else "battle-exit"
    last = max(stamp for stamp, event, _ in rows if event == reference)
    assert (followup_time - last).total_seconds() >= 2.19, rows


if __name__ == "__main__":
    args = probe_args(__doc__)
    os.environ["AOLA_SWF_OPERATIONS"] = "1"
    os.environ["AOLA_LONG_RUN_MAINTENANCE"] = "0"
    os.environ["RUST_LOG"] = "aola_swf_operation=info"
    run_probe(args, "aola_battle_gc", "BattleGcProbe", "BattleGcProbeAccess.java", b"",
              "standalone managers\nnormal then early 1\ntail threw after managers\n"
              "replacement entered 3\nreplacement exited 4\n", frames=8)
    verify_transitions((args.work / "runner.log").read_text(encoding="utf-8"), "off")

    # Use actual host frame boundaries and elapsed time for the two strategies.
    fixture = args.work / "tests/swfs/aola_battle_gc/test.toml"
    fixture.write_text("num_ticks = 100\nsleep_to_meet_frame_rate = true\n", encoding="utf-8")
    for policy, maintenance, cleanup in (("off", "0", "1"), ("legacy", "1", "0"), ("current", "1", "1")):
        env = {**os.environ, "AOLA_LONG_RUN_MAINTENANCE": maintenance, "AOLA_BATTLE_CLEANUP": cleanup, "AOQI_GC_SLICE_MS": "2",
               "RUFFLE_TEST_OPTS": "--compile-mode use-precompiled"}
        result = subprocess.run([args.runner.resolve(strict=True), "aola_battle_gc", "--exact"],
                                cwd=args.work, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=90)
        (args.work / f"timed-{policy}.log").write_bytes(result.stdout)
        if result.returncode or b"1 passed" not in result.stdout:
            raise RuntimeError(result.stdout.decode("utf-8", errors="replace"))
        verify_transitions(result.stdout.decode("utf-8"), policy)
