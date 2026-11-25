"""Run the active skill tab's original and candidate detail dispatch in both listener orders."""
from argparse import ArgumentParser, Namespace
from pathlib import Path
from observe_skin_motion import INTERFACES, abc_tags
from test_notice_patch import run_probe

if __name__ == "__main__":
    parser = ArgumentParser(description=__doc__)
    for key in ("original", "candidate", "work", "runner"):
        parser.add_argument("--" + key, required=True, type=Path)
    args = parser.parse_args()
    interface, = abc_tags(INTERFACES.read_bytes(), lambda name: name == b"mmo/interfaces/IDispose")
    for key, active in (("original", ("1,2", "2,2")), ("candidate", ("2", "2"))):
        expected = f"panel-first {active[0]} 2,3,3,4\npage-first {active[1]} 2,3,4,4\n"
        trial = Namespace(candidate=getattr(args, key), runner=args.runner, work=args.work / key)
        run_probe(trial, "aola_active_skill_tab", "PetActiveTabProbe", "PetActiveTabProbeAccess.java", interface, expected)
