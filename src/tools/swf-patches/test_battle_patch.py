"""Verify the real Wanyu helper's completion, null callback, and normal service path in AVM2."""
from test_notice_patch import probe_args, run_probe

if __name__ == "__main__":
    run_probe(probe_args(__doc__), "aola_wanyu", "BattleProbe", "battle_probe.java", b"",
              "blocked 1 0 2\noriginal 2 2 4\n")
