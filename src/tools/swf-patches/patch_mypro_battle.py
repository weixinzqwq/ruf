"""Overlay the verified battle model selectors on the current optimized battle movie."""
import argparse
import re
import shutil
import struct
import subprocess
import tempfile
from pathlib import Path

from display_patch import make_candidate, reviewed_table
from observe_skin_motion import ASC, ENGINE, JAVA, iter_tags
from patch_battle import ROOT
from mypro_model_recipe import RECIPES, model_table, verify_recipe
from mypro_battle_auto import apply_auto
from mypro_battle_background import apply_background
from mypro_battle_effects import apply_effects

HERE = Path(__file__).resolve().parent
PROFILE = RECIPES / "aola_battle_profile.rs"


def active_version():
    return re.search(r'const VERSION: &str = "([0-9]{1,32})";', PROFILE.read_text(encoding="ascii"))[1]


def weekly_profile(version, packed_size, original, baseline, movie):
    base = reviewed_table(original,baseline,"patch_mypro_battle.py",version,name="BASE_EDITS")
    model = model_table(baseline,movie).replace("const EDITS:", "const MODEL_EDITS:")
    return (f'pub(super) const VERSION: &str = "{version}";\n'
            f'pub(super) const PACKED: usize = {packed_size};\n'
            f'pub(super) const FWS: usize = {len(original)};\n' + base + model)


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


def overlay(work,baseline_file):
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
    model_file=work/"model-only.swf"
    model_file.write_bytes(movie)
    subprocess.run([JAVA,"-cp",str(ROOT/"toolchain/ffdec/lib/*"), HERE/"MyProBattlePatch.java",
                    "--verify",baseline_file,model_file],check=True)
    movie=apply_effects(movie,work/"effects")
    movie=apply_background(movie,work/"backdrop")
    movie=apply_auto(movie,work/"auto")
    (out/"mypro-model.swf").write_bytes(movie)
    print(out/"mypro-model.swf",len(movie),flush=True)
    return movie


def build(work,emit=False,source=None,install=False):
    weekly=source is not None
    version=active_version()
    if weekly:
        match=re.fullmatch(r"petfightnew~([0-9]{1,32})\.swf",source.name)
        if not match:
            raise ValueError("Expected official filename petfightnew~<numeric version>.swf")
        version=match[1]
    archive=ROOT/"resources/original"/f"petfightnew~{version}.swf"
    source=(source or archive).resolve(strict=True)
    size=source.stat().st_size
    if not weekly:
        expected=int(re.search(r"const PACKED: usize = (\d+);",PROFILE.read_text(encoding="ascii"))[1])
        if size!=expected:
            raise ValueError("Archived official movie size differs from the active profile")
    work.mkdir(parents=True,exist_ok=False)
    originals=work/"optimized"
    originals.mkdir()
    original, baseline=make_candidate(source,originals,"battle_patch.java",size)
    baseline_file=work/"baseline.swf"
    baseline_file.write_bytes(baseline)
    movie=overlay(work,baseline_file)
    if not weekly and 'include!("aola_mypro_battle.rs")' in PROFILE.read_text(encoding="ascii"):
        verify_recipe(baseline,movie,"battle",emit)
        return
    profile=weekly_profile(version,size,original,baseline,movie)
    if not weekly:
        if profile!=PROFILE.read_text(encoding="ascii"):
            raise ValueError("Battle movie did not reproduce the installed weekly profile")
        return
    (work/PROFILE.name).write_text(profile,encoding="ascii")
    if install:
        if archive.exists() and archive.read_bytes()!=source.read_bytes():
            raise ValueError("A different archived movie already uses this official version")
        if not archive.exists():
            with tempfile.TemporaryDirectory(dir=archive.parent) as staging:
                staged = Path(staging) / source.name
                shutil.copyfile(source,staged)
                # link fails if another installer created the archive during the copy
                archive.hardlink_to(staged)
        with tempfile.TemporaryDirectory(dir=PROFILE.parent) as staging:
            staged = Path(staging) / PROFILE.name
            staged.write_text(profile,encoding="ascii")
            staged.replace(PROFILE)


if __name__=="__main__":
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir",required=True,type=Path)
    parser.add_argument("--emit-table",action="store_true")
    parser.add_argument("--weekly-source",type=Path,help="Generate and verify a local official weekly battle SWF")
    parser.add_argument("--install-weekly",action="store_true",help="Activate the verified profile for subsequent builds")
    args=parser.parse_args()
    if (args.install_weekly and not args.weekly_source) or (args.weekly_source and args.emit_table):
        parser.error("--install-weekly requires --weekly-source; use --emit-table only for the built-in profile")
    build(args.output_dir.resolve(),args.emit_table,args.weekly_source,args.install_weekly)
