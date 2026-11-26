"""Run each source's real tab listener, visibility methods and completion callback in AVM2."""
from argparse import ArgumentParser, Namespace
from pathlib import Path
from observe_skin_motion import INTERFACES, abc_tags
from test_notice_patch import run_probe

PAGES = ("PetStudyInfo", "PetDetailAbility", "PetItem", "GodCard", "SimpleViewController")

if __name__ == "__main__":
    parser = ArgumentParser(description=__doc__)
    for flag in ("candidate", "original", "work", "runner"):
        parser.add_argument("--" + flag, required=True, type=Path)
    args = parser.parse_args()
    for mode, source in (("original", args.original), ("candidate", args.candidate)):
        rows = []
        for page in PAGES:
            optimized = mode == "candidate" and page != "SimpleViewController"
            counts = "0 0 0/4 0/5 -1/6 1 4 5 5 6" if optimized else "2 1 0/4 0/5 -1/6 1 6 8 9 9"
            rows.append(f"{page} {counts}\n")
        rows.append(f"pair {3 if mode == 'candidate' else 4} 4 2 6/2 2\ndisposed remaining 0\n")
        case = Namespace(work=args.work / mode, runner=args.runner, candidate=source)
        interface, = abc_tags(INTERFACES.read_bytes(), lambda name: name == b"mmo/interfaces/IDispose")
        run_probe(case, "pet_tab_switch", "PetTabSwitchProbe", "PetTabProbeAccess.java",
                  interface, "".join(rows), frames=121)
