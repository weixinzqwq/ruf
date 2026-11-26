"""Exercise the release start/control hooks across AVM2 domains and load all ten packaged skins."""
import argparse
import os
import shutil
import struct
import subprocess
from pathlib import Path

from mypro_game_ui import ASC, ENGINE, HERE, JAVA, compile_globals
from observe_skin_motion import iter_tags, unpack_swf


def make_probe(work, fixture, cls, source_movie, helper_name, dest):
    source=Path(shutil.copyfile(HERE/"harness"/f"{cls}.as",work/f"{cls}.as"))
    subprocess.run([JAVA,"-jar",ASC,"-AS3","-import",work/"playerglobal.abc","-swf",f"{cls},100,100,30",source],check=True)
    helper,=(p for k,_,p,_ in iter_tags(unpack_swf(source_movie.read_bytes()))
             if k==82 and p[4:].startswith(f"aola.tools.{helper_name}\0".encode()))
    helper=helper.replace(b"flash.external",b"probe.external")
    game=unpack_swf(source.with_suffix(".swf").read_bytes())
    kind,pos,payload,raw=next(t for t in iter_tags(game) if t[0] in (72,82))
    def tag(p):
        return struct.pack("<HI",(82<<6)|63,len(p))+p
    probe=raw if kind==82 else tag(bytes(4)+b"probe\0"+payload)
    body=game[:pos]+probe+tag(helper)+game[pos+len(raw):]
    (fixture/dest).write_bytes(b"FWS\x12"+struct.pack("<I",len(body)+8)+body)


def test(bundle, work, runner):
    fixture=work/"tests/swfs/mypro_game_ui"
    fixture.mkdir(parents=True)
    compile_globals(work)
    make_probe(work,fixture,"MyProGameUiProbe",bundle/"start-ui.swf","MyProGameUi","test.swf")
    make_probe(work,fixture,"MyProGameUiControlProbe",bundle/"control-ui.swf","MyProGameUiDomain","control.swf")
    for i in range(1,11):
        shutil.copyfile(bundle/f"skin{i}.swf",fixture/f"skin{i}.swf")
    expected="""empty -1 unavailable
child only false 0
settings requested activityres/activity240927/controlbarskinsetting/controlbarskinsetting_code mmo.activity240927.controlbarskinsetting.panel.CBSS_MainPl
summoner requested activityres/activity250516/newzhaohuanshi/newzhaohuanshi_code mmo.activity250516.newzhaohuanshi.panel.NZHS_MainPl
unknown blocked
battle WaitingBattleEnd blocked
after Applied 10
native Applied 0
"""
    (fixture/"output.txt").write_text(expected+"".join(f"leaf {i} true true\n" for i in range(1,11)),encoding="ascii")
    (fixture/"test.toml").write_text("num_frames = 100\n",encoding="ascii")
    result=subprocess.run([runner,"mypro_game_ui","--exact"],cwd=work,env={**os.environ,"RUFFLE_TEST_OPTS":"--compile-mode use-precompiled"},
                          capture_output=True,timeout=90)
    log=result.stdout+result.stderr
    (work/"runner.log").write_bytes(log)
    print(log.decode(errors="replace"))
    if result.returncode or b"1 passed" not in log: raise SystemExit(1)


if __name__=="__main__":
    parser=argparse.ArgumentParser(description=__doc__)
    for key in ["bundle","work","runner"]:
        parser.add_argument("--"+key,type=Path,required=True)
    args=parser.parse_args()
    test(args.bundle.resolve(),args.work.resolve(),args.runner.resolve())
