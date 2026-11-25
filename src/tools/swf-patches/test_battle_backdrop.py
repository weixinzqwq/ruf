"""Exercise each battle entry's actual dispose, transition branches and completion closure."""
from copy import copy

from test_notice_patch import probe_args, run_probe

if __name__ == "__main__":
    args = probe_args(__doc__)
    for owner in ("StartMovie", "StartMovie3v3"):
        trial = copy(args)
        trial.work = args.work / owner
        run_probe(trial, f"aola_backdrop_{owner}", "BattleBackdropProbe", "battle_probe.java",
                  b"", f"{owner} 8 cases passed\n", (owner,))
