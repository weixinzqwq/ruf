"""Overlay the verified battle model selectors on the current optimized battle movie."""
import argparse
import shutil
import struct
import subprocess
from pathlib import Path

from display_patch import make_candidate
from observe_skin_motion import ASC, ENGINE, JAVA, iter_tags
from patch_battle import REFERENCE, ROOT
from mypro_model_recipe import verify_recipe

HERE = Path(__file__).resolve().parent


def compile_bridge(work):
    globals_dir = ENGINE / "core/src/avm2/globals"
    source = work / "MyProBattleModel.as"
    shutil.copyfile(HERE / source.name, source)
    commands = [
        [JAVA, "-classpath", ASC, "macromedia.asc.embedding.ScriptCompiler", "-optimize", "-builtin", "-apiversioning",
         "-outdir", work, "-out", "playerglobal", globals_dir / "Toplevel.as", globals_dir / "globals.as"],
        [JAVA, "-jar", ASC, "-AS3", "-import", work / "playerglobal.abc", source],
    ]
    with (work/"compiler.log").open("wb") as log:
        for command in commands:
            subprocess.run(command,stdout=log,stderr=subprocess.STDOUT,check=True)
    return source.with_suffix(".abc").read_bytes()


def overlay(work,baseline_file,emit=False):
    baseline=baseline_file.read_bytes()
    with (work/"model-patch.log").open("wb") as log:
        subprocess.run([JAVA,"-cp",str(ROOT/"toolchain/ffdec/lib/*"), HERE/"MyProBattlePatch.java",
                        baseline_file,work/"original.abc",work/"patched.abc"],stdout=log,stderr=subprocess.STDOUT,check=True)
    (offset,original,raw),=tuple((offset,payload,raw) for code,offset,payload,raw in iter_tags(baseline[8:]) if code==82)
    start=original.index(0,4)+1
    if original[start:]!=(work/"original.abc").read_bytes():
        raise ValueError("Original optimized battle ABC was not roundtripped verbatim")
    new_abc=original[:start]+(work/"patched.abc").read_bytes()
    model_tag=struct.pack("<HI",(82<<6)|63,len(new_abc))+new_abc
    helper=compile_bridge(work)
    payload=struct.pack("<I",1)+b"aola.tools.MyProBattleModel\0"+helper
    tag=struct.pack("<HI",(82<<6)|63,len(payload))+payload
    body=baseline[8:8+offset]+tag+model_tag+baseline[8+offset+len(raw):]
    movie=baseline[:4]+struct.pack("<I",len(body)+8)+body
    out=work/"swf-battle"
    out.mkdir()
    (out/"mypro-model.swf").write_bytes(movie)
    subprocess.run([JAVA,"-cp",str(ROOT/"toolchain/ffdec/lib/*"), HERE/"MyProBattlePatch.java",
                    "--verify",baseline_file,out/"mypro-model.swf"],check=True)
    verify_recipe(baseline,movie,"battle",emit)
    print(out/"mypro-model.swf",len(movie),flush=True)


def build(work,emit):
    work.mkdir(parents=True,exist_ok=False)
    originals=work/"optimized"
    originals.mkdir()
    _, baseline=make_candidate(REFERENCE,originals,"battle_patch.java",5_377_898)
    baseline_file=work/"baseline.swf"
    baseline_file.write_bytes(baseline)
    overlay(work,baseline_file,emit)


if __name__=="__main__":
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir",required=True,type=Path)
    parser.add_argument("--emit-table",action="store_true")
    args=parser.parse_args()
    build(args.output_dir.resolve(),args.emit_table)
