"""Run the captured battle switch method against local display objects and a synchronous load stub."""
from test_notice_patch import probe_args, run_probe


if __name__ == "__main__":
    run_probe(probe_args(__doc__), "aola_battle_switch_timing", "BattleSwitchProbe", "BattleSwitchProbeAccess.java",
              b"", "switched 1 true\nswitched 1 true\n", frames=65)
