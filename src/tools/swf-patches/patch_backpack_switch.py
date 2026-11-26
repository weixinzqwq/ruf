"""Rebuild the two reviewed backpack movies and check the embedded edit tables."""
import argparse
import os
from pathlib import Path
import shutil
import subprocess

from display_patch import make_candidate, reviewed_table
from observe_skin_motion import iter_tags
from pilot_resource import REFERENCE, ROOT, VERSION
from roundtrip import JAVA


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, required=True, help="Fresh working directory")
    parser.add_argument("--emit-table", action="store_true", help="Update the Rust edit tables")
    args = parser.parse_args()
    work = args.output_dir.resolve()
    work.mkdir(parents=True, exist_ok=False)
    classes = work / "classes"
    classes.mkdir()
    lib = str(ROOT / "toolchain/ffdec/lib/*")
    scripts = Path(__file__).resolve().parent
    java_classes = ("PetSwitchPatch", "PetListSwitchPatch", "PetTabSwitchPatch", "PetActiveTabSwitchPatch")
    subprocess.run([JAVA.with_name("javac.exe"), "-encoding", "UTF-8", "-cp", lib, "-d", classes,
                    *(scripts / f"{name}.java" for name in java_classes)], check=True)
    patch = [JAVA, "-cp", os.pathsep.join((str(classes), lib)), "PetSwitchPatch"]

    bundle = work / "swf-backpack"
    bundle.mkdir()
    tables = []
    for label, version, source, size in (
        ("supplied", "official unversioned sample", ROOT / "newpetpackage.swf", 1_524_996),
        ("archived", VERSION, REFERENCE, 1_525_161),
    ):
        case = work / label
        case.mkdir()
        original, candidate = make_candidate(source, case, None, size, java_command=patch)
        before = list(iter_tags(original[8:]))
        after = list(iter_tags(candidate[8:]))
        if [tag[0] for tag in before] != [tag[0] for tag in after]:
            raise ValueError(f"{label}: SWF tag order changed")
        changed = [a[0] for a, b in zip(before, after) if a[3] != b[3]]
        first_tag = 8 + before[0][1]
        if changed != [82] or original[:4] != candidate[:4] or original[8:first_tag] != candidate[8:first_tag]:
            raise ValueError(f"{label}: expected only the DoABC tag to change")
        tables.append(reviewed_table(original, candidate, Path(__file__).name, version,
                                     name=label.upper() + "_EDITS"))
        shutil.copyfile(source, bundle / f"{label}.swf")
        (bundle / f"{label}-optimized.swf").write_bytes(candidate)
        print(f"{label}: {len(before)} tags, 1 DoABC changed, FWS {len(original)} -> {len(candidate)}")
    table = "\n".join(tables)
    path = ROOT / "src/optimization/engine/frontend-utils/src/backends/navigator/aola_backpack_edits.rs"
    if args.emit_table:
        path.write_bytes(table.encode("utf-8"))
    elif table != path.read_text(encoding="utf-8"):
        raise ValueError("Regenerated backpack recipes differ from the compiled tables")
    print(bundle)


if __name__ == "__main__":
    main()
