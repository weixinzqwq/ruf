"""Build two optional tools-mode backpacks on top of the reviewed performance patches."""
import argparse
import os
from pathlib import Path
import shutil
import struct
import subprocess

from display_patch import make_candidate
from observe_skin_motion import ASC, ENGINE, JAVA, iter_tags
from pilot_resource import REFERENCE, ROOT
from mypro_lineup_backpack import apply_lineup
from mypro_model_recipe import RECIPES, model_table


def build(work,emit=False):
    work.mkdir(parents=True, exist_ok=False)
    classes = work / "classes"
    classes.mkdir()
    bundle = work / "swf-backpack"
    bundle.mkdir()
    scripts = Path(__file__).resolve().parent
    lib = str(ROOT / "toolchain/ffdec/lib/*")
    names = ["MyProBackpackPatch", "MyProBackpackVisualPatch", "PetListSwitchPatch", "PetTabSwitchPatch",
             "PetActiveTabSwitchPatch", "PetSwitchPatch"]
    subprocess.run([JAVA.with_name("javac.exe"), "-encoding", "UTF-8", "-cp", lib, "-d", classes,
                    *(scripts / (name+".java") for name in names)],check=True)
    globals_dir = ENGINE / "core/src/avm2/globals"
    subprocess.run([JAVA, "-classpath", ASC, "macromedia.asc.embedding.ScriptCompiler", "-optimize",
                    "-builtin", "-apiversioning", "-outdir", work, "-out", "playerglobal",
                    globals_dir/"Toplevel.as", globals_dir/"globals.as"], check=True)
    helper = Path(shutil.copyfile(scripts / "MyProBackpackMorph.as", work / "MyProBackpackMorph.as"))
    subprocess.run([JAVA, "-jar", ASC, "-AS3", "-import", work / "playerglobal.abc", helper], check=True)
    payload = struct.pack("<I", 1) + b"aola/tools/MyProBackpackMorph\0" + helper.with_suffix(".abc").read_bytes()
    helper_tag = struct.pack("<HI",(82 << 6) | 63,len(payload)) + payload
    java = [JAVA, "-cp", os.pathsep.join((str(classes),lib)), names[0]]
    tables = {}
    for label, source, size in (("supplied", ROOT/"newpetpackage.swf", 1524996), ("archived", REFERENCE, 1525161)):
        case = work / label
        case.mkdir()
        orig, patched = make_candidate(source, case, None, size, java_command=java)
        before = list(iter_tags(orig[8:]))
        tags = list(iter_tags(patched[8:]))
        assert [t[0] for t in before] == [t[0] for t in tags]
        assert [a[0] for a,b in zip(before,tags) if a[3]!=b[3]] == [82]
        offset, = [pos for kind,pos,_,_ in tags if kind==82]
        body = patched[8:8+offset] + helper_tag + patched[8+offset:]
        result = patched[:4] + struct.pack("<I",8+len(body)) + body
        candidate = bundle / (label+"-mypro.swf")
        result = apply_lineup(result,case/"lineup")
        candidate.write_bytes(result)
        subprocess.run([*java,"--verify",candidate],check=True)
        base_dir = case/"baseline"
        base_dir.mkdir()
        _, baseline = make_candidate(source,base_dir,None,size,java_command=[*java[:-1],"PetSwitchPatch"])
        tables[RECIPES/f"aola_mypro_{label}.rs"] = model_table(baseline,result)
        print(label, len(result))
    for path,table in tables.items():
        if table != path.read_text(encoding="ascii"):
            if not emit:
                raise ValueError(f"Backpack overlay differs from the reviewed {path.stem} profile")
            path.write_text(table,encoding="ascii")
    return bundle


if __name__ == "__main__":
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--output-dir",type=Path,required=True)
    ap.add_argument("--emit-table",action="store_true")
    args=ap.parse_args()
    build(args.output_dir.resolve(),args.emit_table)
